//! Claude's native OAuth controls own PKCE, credential storage and account refresh.

use super::{active, emit_extras, finish, now_ms, AuthState, ChatProcess};
use crate::{
    agent::chat::{engine, protocol},
    host::AppCtx,
    models::SessionKind,
};
use serde_json::{json, Value};
use std::{
    process::Stdio,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

/// Retain configuration sources that may select a different credential store or authentication method.
pub(in crate::agent::chat::engine) fn settings_args(args: &[String]) -> Vec<String> {
    let mut selected = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if matches!(arg.as_str(), "--settings" | "--setting-sources") {
            selected.push(arg.clone());
            if let Some(value) = args.next() {
                selected.push(value.clone());
            }
        } else if arg.starts_with("--settings=") || arg.starts_with("--setting-sources=") {
            selected.push(arg.clone());
        }
    }
    selected
}

fn idle(proc: &ChatProcess) -> Result<(), String> {
    if !proc.alive.load(Ordering::Relaxed) {
        return Err("Claude has stopped. Please try again.".into());
    }
    let turn = proc.turn.lock().unwrap();
    if turn.running
        || !turn.active_tasks.is_empty()
        || !turn.background_tasks.is_empty()
        || !proc.permissions.lock().unwrap().is_empty()
    {
        return Err("Wait for Claude's current turn and background tasks to finish before changing accounts.".into());
    }
    Ok(())
}

pub(in crate::agent::chat::engine) fn start(
    app: &AppCtx,
    session_id: &str,
    proc: &Arc<ChatProcess>,
) -> Result<(), String> {
    if active(proc) {
        return Ok(());
    }
    idle(proc)?;
    let id = proc.request_id("claude_auth_start");
    let mut state = AuthState::new("starting");
    state.attempt = id.clone();
    proc.extras.lock().unwrap().auth = Some(state);
    emit_extras(app, session_id, proc);
    crate::diagnostics::record(
        "INFO",
        "claude_account",
        json!({"method":"program","sessionId":session_id,"status":"started"}),
    );
    if proc
        .write(&protocol::control_request(
            &id,
            json!({"subtype":"claude_authenticate","loginWithClaudeAi":true}),
        ))
        .is_err()
    {
        finish(app, session_id, proc, "failed");
        return Err("Could not start Claude sign-in.".into());
    }
    let (app, session_id, proc) = (app.clone(), session_id.to_string(), proc.clone());
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let state = proc.extras.lock().unwrap().auth.clone();
        let Some(state) = state.filter(|s| s.attempt == id && s.active()) else {
            break;
        };
        let limit = match state.status {
            "canceling" => 10_000,
            "submitting" => 120_000,
            _ if state.callback_url.is_none() => 30_000,
            _ => 15 * 60_000,
        };
        if !proc.alive.load(Ordering::Relaxed) || now_ms().saturating_sub(state.started_at) >= limit
        {
            let _action = proc.action.lock().unwrap();
            if proc
                .extras
                .lock()
                .unwrap()
                .auth
                .as_ref()
                .is_some_and(|s| s.attempt == id && s.active())
            {
                finish(&app, &session_id, &proc, "failed");
                release(&app, &session_id, &proc);
            }
            break;
        }
    });
    Ok(())
}

// Never accept an arbitrary link from a provider error, and never send cancellation off-host.
fn authorization_urls(value: &Value) -> Option<(String, String, String)> {
    let manual = url::Url::parse(value["manualUrl"].as_str()?).ok()?;
    let automatic = url::Url::parse(value["automaticUrl"].as_str()?).ok()?;
    for url in [&manual, &automatic] {
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || !matches!(
                url.host_str()?,
                "claude.ai" | "platform.claude.com" | "console.anthropic.com"
            )
            || url.path() != "/oauth/authorize"
        {
            return None;
        }
    }
    let parameter = |url: &url::Url, name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    let state = parameter(&manual, "state")?;
    if state.is_empty() || parameter(&automatic, "state")? != state {
        return None;
    }
    let callback = url::Url::parse(&parameter(&automatic, "redirect_uri")?).ok()?;
    if callback.scheme() != "http"
        || callback.host_str() != Some("127.0.0.1")
        || !callback.username().is_empty()
        || callback.password().is_some()
        || callback.port().is_none_or(|port| port == 0)
        || callback.path() != "/callback"
        || callback.query().is_some()
        || callback.fragment().is_some()
    {
        return None;
    }
    Some((manual.to_string(), callback.to_string(), state))
}

pub(in crate::agent::chat::engine) fn response(
    app: &AppCtx,
    session_id: &str,
    proc: &Arc<ChatProcess>,
    id: &str,
    kind: Option<&str>,
    value: &Value,
    error: Option<&str>,
) -> bool {
    if proc.kind != SessionKind::Claude
        || !matches!(
            kind,
            Some("claude_auth_start" | "claude_auth_wait" | "claude_auth_submit")
        )
    {
        return false;
    }
    let _action = proc.action.lock().unwrap();
    let state = proc.extras.lock().unwrap().auth.clone();
    let Some(state) = state.filter(|s| {
        s.active()
            && (s.attempt == id
                || s.completion_id.as_deref() == Some(id)
                || s.submission_id.as_deref() == Some(id))
    }) else {
        return true;
    };
    if kind == Some("claude_auth_start") {
        if let Some((manual, callback, oauth_state)) =
            authorization_urls(value).filter(|_| error.is_none())
        {
            let wait = proc.request_id("claude_auth_wait");
            let mut next = state.clone();
            next.status = if state.status == "canceling" {
                "canceling"
            } else {
                "pending"
            };
            next.verification_url = Some(manual);
            next.callback_url = Some(callback);
            next.oauth_state = Some(oauth_state);
            next.completion_id = Some(wait.clone());
            proc.extras.lock().unwrap().auth = Some(next.clone());
            if proc
                .write(&protocol::control_request(
                    &wait,
                    json!({"subtype":"claude_oauth_wait_for_completion"}),
                ))
                .is_err()
            {
                finish(app, session_id, proc, "failed");
                release(app, session_id, proc);
            } else {
                emit_extras(app, session_id, proc);
                if next.status == "canceling" {
                    cancel_callback(app, session_id, proc, &next);
                }
            }
        } else {
            // Older CLIs reject these controls. Never display a raw provider response containing auth data.
            finish(app, session_id, proc, "failed");
            release(app, session_id, proc);
        }
    } else {
        let status = if error.is_none() && value["account"].is_object() {
            "success"
        } else if state.status == "canceling" {
            "canceled"
        } else {
            "failed"
        };
        finish(app, session_id, proc, status);
        engine::release_if_idle(app, session_id, proc);
    }
    true
}

pub(in crate::agent::chat::engine) fn submit(
    app: &AppCtx,
    session_id: &str,
    proc: &Arc<ChatProcess>,
    input: &str,
) -> Result<(), String> {
    let state = proc
        .extras
        .lock()
        .unwrap()
        .auth
        .clone()
        .filter(|s| s.status == "pending")
        .ok_or("Claude is not waiting for an authorization code.")?;
    let (code, oauth_state) = input
        .trim()
        .split_once('#')
        .ok_or("claude_auth_invalid_code")?;
    if code.is_empty()
        || code.len() > 4096
        || code.chars().any(char::is_whitespace)
        || Some(oauth_state) != state.oauth_state.as_deref()
    {
        return Err("claude_auth_invalid_code".into());
    }
    let id = proc.request_id("claude_auth_submit");
    let mut next = state;
    next.status = "submitting";
    next.started_at = now_ms();
    next.verification_url = None;
    next.submission_id = Some(id.clone());
    proc.extras.lock().unwrap().auth = Some(next);
    emit_extras(app, session_id, proc);
    if proc
        .write(&protocol::control_request(
            &id,
            json!({"subtype":"claude_oauth_callback","authorizationCode":code,"state":oauth_state}),
        ))
        .is_err()
    {
        finish(app, session_id, proc, "failed");
        release(app, session_id, proc);
        return Err("Could not submit the Claude authorization code.".into());
    }
    Ok(())
}

pub(in crate::agent::chat::engine) fn cancel(
    app: &AppCtx,
    session_id: &str,
    proc: &Arc<ChatProcess>,
) -> Result<(), String> {
    let mut state = match proc.extras.lock().unwrap().auth.clone() {
        Some(state) if matches!(state.status, "starting" | "pending") => state,
        Some(state) if state.status == "submitting" => {
            return Err("Wait for Claude to finish verifying the authorization code.".into())
        }
        _ => return Ok(()),
    };
    state.status = "canceling";
    state.started_at = now_ms();
    proc.extras.lock().unwrap().auth = Some(state.clone());
    emit_extras(app, session_id, proc);
    if state.callback_url.is_some() {
        cancel_callback(app, session_id, proc, &state);
    }
    Ok(())
}

fn cancel_callback(app: &AppCtx, session_id: &str, proc: &Arc<ChatProcess>, state: &AuthState) {
    let (Some(callback), Some(oauth_state)) = (&state.callback_url, &state.oauth_state) else {
        return;
    };
    let Ok(mut url) = url::Url::parse(callback) else {
        return;
    };
    url.query_pairs_mut()
        .append_pair("error", "access_denied")
        .append_pair("state", oauth_state);
    let (app, session_id, proc, attempt) = (
        app.clone(),
        session_id.to_string(),
        proc.clone(),
        state.attempt.clone(),
    );
    std::thread::spawn(move || {
        // The native callback rejects the pending OAuth promise, closes its listener and answers wait.
        let sent = ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .redirects(0)
            .timeout(Duration::from_secs(5))
            .build()
            .get(url.as_str())
            .call();
        if matches!(sent, Err(ureq::Error::Transport(_))) {
            let _action = proc.action.lock().unwrap();
            if proc
                .extras
                .lock()
                .unwrap()
                .auth
                .as_ref()
                .is_some_and(|s| s.attempt == attempt && s.status == "canceling")
            {
                finish(&app, &session_id, &proc, "failed");
                release(&app, &session_id, &proc);
            }
        }
    });
}

pub(in crate::agent::chat::engine) fn logout(
    app: &AppCtx,
    session_id: &str,
    proc: &Arc<ChatProcess>,
) -> Result<(), String> {
    idle(proc)?;
    if active(proc) {
        return Err("Finish or cancel the current account operation before signing out.".into());
    }
    proc.extras.lock().unwrap().auth = Some(AuthState::new("signingOut"));
    emit_extras(app, session_id, proc);
    let mut command = crate::host::command(&proc.bin);
    command
        .args(&proc.auth_settings_args)
        .args(["auth", "logout"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(cwd) = &proc.cwd {
        command.current_dir(cwd);
    }
    for key in crate::pty::manager::AGENT_HARNESS_MARKERS {
        command.env_remove(key);
    }
    crate::agent::executable::prepare_command(&mut command, &proc.bin);
    let result = (|| {
        let mut child = command.spawn().map_err(|_| ())?;
        for _ in 0..300 {
            match child.try_wait() {
                Ok(Some(status)) => return if status.success() { Ok(()) } else { Err(()) },
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        Err(())
    })();
    finish(
        app,
        session_id,
        proc,
        if result.is_ok() {
            "signedOut"
        } else {
            "logoutFailed"
        },
    );
    // The old process caches credentials. Its replacement resumes the same native conversation.
    release(app, session_id, proc);
    result.map_err(|_| "claude_auth_logout_failed".into())
}

fn release(app: &AppCtx, session_id: &str, proc: &Arc<ChatProcess>) {
    proc.auth_restart.store(true, Ordering::Relaxed);
    engine::release_process(app, session_id, proc);
}

pub(in crate::agent::chat::engine) fn restore(previous: &ChatProcess, proc: &ChatProcess) -> bool {
    if previous.kind != SessionKind::Claude || !previous.auth_restart.load(Ordering::Relaxed) {
        return false;
    }
    proc.timeline
        .lock()
        .unwrap()
        .replace_all(previous.timeline.lock().unwrap().rows.clone());
    *proc.extras.lock().unwrap() = previous.extras.lock().unwrap().clone();
    *proc.agent_session_id.lock().unwrap() = previous.agent_session_id.lock().unwrap().clone();
    *proc.user_targets.lock().unwrap() = previous.user_targets.lock().unwrap().clone();
    proc.turn.lock().unwrap().waiting = previous.turn.lock().unwrap().waiting.clone();
    proc.compactions.store(
        previous.compactions.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
    true
}

pub(in crate::agent::chat::engine) fn exited(
    app: &AppCtx,
    session_id: &str,
    proc: &Arc<ChatProcess>,
) {
    if proc.kind == SessionKind::Claude && active(proc) {
        let _action = proc.action.lock().unwrap();
        if active(proc) {
            finish(app, session_id, proc, "failed");
        }
    }
}

pub(in crate::agent::chat::engine) fn observe(
    app: &AppCtx,
    session_id: &str,
    proc: &Arc<ChatProcess>,
    line: &str,
) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if !value["parent_tool_use_id"].is_null() {
        return;
    }
    let required = match value["type"].as_str() {
        Some("assistant") => value["error"] == "authentication_failed",
        Some("system") if value["subtype"] == "api_error" => {
            value["error"]["status"] == 401 || value["error"]["type"] == "authentication_error"
        }
        Some("result") if value["is_error"] == true => {
            value["errors"].as_array().is_some_and(|errors| {
                errors.iter().any(|error| {
                    super::is_auth_error(error)
                        || error.as_str().is_some_and(|message| {
                            let message = message.to_lowercase();
                            message.contains("authentication_error")
                                || message.contains("invalid api key")
                                || message.contains("please run /login")
                        })
                })
            })
        }
        _ => false,
    };
    if required {
        super::require(
            app,
            session_id,
            proc,
            &json!({"message":"authentication required"}),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logout_retains_settings_sources_without_passing_model_flags() {
        let args: Vec<String> = [
            "--model",
            "model",
            "--settings",
            "custom.json",
            "--setting-sources=user,project",
            "--effort",
            "high",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        assert_eq!(
            settings_args(&args),
            vec![
                "--settings",
                "custom.json",
                "--setting-sources=user,project"
            ]
        );
    }

    #[test]
    fn authorization_destinations_reject_remote_callbacks_and_mismatched_state() {
        let manual = "https://claude.ai/oauth/authorize?state=expected";
        let valid = "https://claude.ai/oauth/authorize?state=expected&redirect_uri=http%3A%2F%2F127.0.0.1%3A43217%2Fcallback";
        assert!(authorization_urls(&json!({"manualUrl":manual,"automaticUrl":valid})).is_some());
        for invalid in [
            valid.replace("127.0.0.1", "192.168.1.1"),
            valid.replace("expected", "other"),
            valid.replace("claude.ai", "claude.ai.example.org"),
            valid.replace("43217%2Fcallback", "43217%2Fother"),
            valid.replace("http%3A", "https%3A"),
        ] {
            assert!(
                authorization_urls(&json!({"manualUrl":manual,"automaticUrl":invalid})).is_none()
            );
        }
    }
}
