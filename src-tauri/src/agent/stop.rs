//! `vstop`: stop another session's process the way the sidebar's Kill does, so a captain can park an idle
//! session and free its memory. The session itself stays, and `vtell` or a click resumes it.

use std::io::Read;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{command_core as core, host::AppCtx, pty::AgentState};

/// Kill source for a `vstop`, distinct from every client's own so each one closes its view of the session.
const STOP_SOURCE: &str = "vstop";

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub session_id: String,
    pub target: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub force: bool,
}

/// Sessions whose Claude has asked for a Bash command with `run_in_background`. The hooks report such a
/// session as `waiting` once its turn ends, so the mark is the only sign that shell work may be running.
/// In memory only: a restart of VelaTerm also restarts the terminals the marks describe.
fn marked() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static MARKED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    MARKED.get_or_init(Default::default)
}

pub fn mark_background_shell(session_id: &str) {
    marked().lock().unwrap().insert(session_id.to_string());
}

/// Called when the session's terminal exits or is killed, so a restart starts clean.
pub fn clear_background_shell(session_id: &str) {
    marked().lock().unwrap().remove(session_id);
}

pub fn clear_all_background_shells() {
    marked().lock().unwrap().clear();
}

fn is_marked(session_id: &str) -> bool {
    marked().lock().unwrap().contains(session_id)
}

/// Whether a Claude hook body is a `PreToolUse` for the Bash tool with `run_in_background` true.
pub(crate) fn starts_background_shell(body: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return false };
    let event = v.get("hook_event_name").or_else(|| v.get("hookEventName")).and_then(Value::as_str);
    event == Some("PreToolUse")
        && v.get("tool_name").or_else(|| v.get("toolName")).and_then(Value::as_str) == Some("Bash")
        && v.get("tool_input").and_then(|input| input.get("run_in_background")).and_then(Value::as_bool) == Some(true)
}

/// A POSIX shell, with or without `.exe`. Not `cmd`, `pwsh` or `powershell`: Windows MCP launchers use those.
fn is_posix_shell(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    matches!(name, "bash" | "sh" | "zsh" | "dash" | "fish")
}

fn is_claude(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "claude" || name == "claude.exe"
}

/// Why a session must not be stopped without `--force`, or `None` when it may be.
///
/// A working or background session would lose its work. An asking one is waiting on fg, and stopping it
/// throws the question away. A session that never reported a state is a plain shell or still starting,
/// and neither is idle in the sense the sweep means. A waiting session that started Bash background work
/// (`marked`) is refused while a shell is still found below its Claude (`shell_found`).
fn refusal(state: Option<AgentState>, marked: bool, shell_found: bool) -> Option<&'static str> {
    match state {
        Some(AgentState::Waiting) if marked && shell_found => {
            Some("it has background shell work still running (Bash run_in_background)")
        }
        Some(AgentState::Waiting) => None,
        Some(AgentState::Working) => Some("it is working"),
        Some(AgentState::Background) => Some("its background work is still running"),
        Some(AgentState::Asking) => Some("it is waiting on a permission prompt or a question"),
        None => Some("it has not reported a state (a plain shell, or still starting)"),
    }
}

/// Look for a shell below Claude only when the mark says one may exist and nothing else already refuses.
/// A mark with no shell behind it is stale, so it is cleared.
fn shell_below_claude(app: &AppCtx, target: &str, chat: bool, state: Option<AgentState>) -> bool {
    if chat || state != Some(AgentState::Waiting) || !is_marked(target) {
        return false;
    }
    let found = app
        .pty()
        .cwd_pid(target)
        .and_then(|pid| crate::procstat::below_process_has(pid, is_claude, is_posix_shell))
        .unwrap_or(false);
    if !found {
        clear_background_shell(target);
    }
    found
}
fn state_name(state: Option<AgentState>) -> Value {
    state.map(|s| json!(s)).unwrap_or(Value::Null)
}

pub fn stop(app: &AppCtx, req: &Request) -> Result<Value, String> {
    let target = super::tell::resolve(app, &req.target)?;
    if target == req.session_id {
        return Err("A session cannot stop itself".into());
    }
    super::tell::session(app, &req.session_id)?;
    let session = super::tell::session(app, &target)?;
    let state = super::status_watch::snapshot()
        .1
        .into_iter()
        .find(|row| row.session_id == target)
        .map(|row| row.state);
    let chat = session.engine == "chat";
    let alive = if chat { app.chat().is_alive(&target) } else { app.pty().is_running(&target) };
    let mut out = json!({"sessionId": target, "name": session.name, "before": state_name(state)});
    if !alive {
        // A sweep re-running over parked sessions clears any marker left from before.
        crate::session_state::set_unread(app, &target, false);
        out["result"] = json!("already stopped");
        return Ok(out);
    }
    if !req.force {
        let shell_found = shell_below_claude(app, &target, chat, state);
        if let Some(why) = refusal(state, is_marked(&target), shell_found) {
            return Err(format!("Not stopping {}: {why}. Pass --force to stop it anyway", session.name));
        }
    }
    out["pid"] = if chat { Value::Null } else { json!(app.pty().cwd_pid(&target)) };
    if chat {
        core::chat_stop(app, &target)?;
    } else {
        app.pty().kill(&target, STOP_SOURCE, crate::pty::session::KillReason::Close)?;
    }
    // Clear the unread marker too. A parked session that flashes "unread reply" invites a click, and the
    // click starts the very process the stop just ended.
    crate::session_state::set_unread(app, &target, false);
    crate::diagnostics::record(
        "INFO",
        "session_stop",
        json!({"sessionId":target,"by":req.session_id,"before":state_name(state),"force":req.force,"reason":req.reason}),
    );
    out["result"] = json!("stopped");
    Ok(out)
}

pub fn handle(app: AppCtx, mut request: crate::diagnostics::HttpRequest, token: String) {
    let started = Instant::now();
    let request_id = request.id().to_owned();
    let authorized = request
        .headers()
        .iter()
        .any(|h| h.field.equiv("Authorization") && h.value.as_str() == format!("Bearer {token}"));
    let mut body = String::new();
    let result = if !authorized {
        Err((401, "Authentication required".to_owned()))
    } else if request.method() != &tiny_http::Method::Post {
        Err((405, "Use POST".to_owned()))
    } else if request.as_reader().take(65537).read_to_string(&mut body).is_err() || body.len() > 65536 {
        Err((413, "Request too large".to_owned()))
    } else {
        serde_json::from_str::<Request>(&body)
            .map_err(|e| e.to_string())
            .and_then(|req| stop(&app, &req))
            .map_err(|e| (400, e))
    };
    let (status, value) = match result {
        Ok(result) => (200, json!({"result":result})),
        Err((status, error)) => (status, json!({"error":error})),
    };
    let _ = request.respond(
        tiny_http::Response::from_string(value.to_string())
            .with_status_code(status)
            .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap()),
    );
    crate::diagnostics::record(
        if status == 200 { "INFO" } else { "WARN" },
        "session_stop_request",
        json!({"requestId":request_id,"statusCode":status,"durationMs":started.elapsed().as_millis() as u64}),
    );
}

const USAGE: &str = "usage: vstop <session> [--reason \"<text>\"] [--force] [--json]
Stop a session's process the way the sidebar's Kill does. The session stays in the tree, and vtell
or a click resumes it in the same conversation.
Targets accept a session ID, an ID prefix of at least 8 characters, or an unambiguous name.
Refuses, unless --force, a session that is working, running background work (including a Bash
run_in_background command still running), waiting on a permission prompt or question, or has not
reported a state; and always refuses the calling session.
Stopping also clears the session's unread marker.
--reason is recorded in VelaTerm's diagnostics log.";

/// Parsed command line: the request and whether to print JSON. `None` asks for the usage text.
fn parse_args(args: &[String]) -> Result<Option<(Request, bool)>, String> {
    let mut req = Request::default();
    let mut json = false;
    let mut target = None;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--help" | "-h" => return Ok(None),
            "--force" => req.force = true,
            "--json" => json = true,
            "--reason" => {
                i += 1;
                req.reason = Some(args.get(i).ok_or("--reason needs a value")?.clone());
            }
            _ if arg.starts_with("--reason=") => req.reason = Some(arg["--reason=".len()..].to_owned()),
            _ if arg.starts_with('-') => return Err(format!("Unknown option {arg}")),
            _ if target.is_none() => target = Some(arg.to_owned()),
            _ => return Err("Specify one target session".into()),
        }
        i += 1;
    }
    req.target = target.ok_or("Specify a target session")?;
    Ok(Some((req, json)))
}

pub fn run_cli(args: &[String]) -> ! {
    let mut json = false;
    let result = (|| -> Result<Value, String> {
        let Some((mut req, as_json)) = parse_args(&args[args.len().min(2)..])? else {
            println!("{USAGE}");
            std::process::exit(0);
        };
        json = as_json;
        req.session_id = std::env::var("VLX_SESSION_ID").map_err(|_| "Run inside a VelaTerm session")?;
        super::tell::post_local("stop", &serde_json::to_value(req).unwrap())
    })();
    match result {
        Ok(value) if json => {
            println!("{value}");
            std::process::exit(0);
        }
        Ok(value) => {
            let text = |key: &str| value[key].as_str().unwrap_or("-").to_owned();
            let pid = value["pid"].as_u64().map(|p| format!(", pid {p}")).unwrap_or_default();
            println!("{} ({}): {}, was {}{pid}", text("name"), text("sessionId"), text("result"), text("before"));
            std::process::exit(0);
        }
        Err(error) => {
            if json {
                println!("{}", json!({"error": error}));
            } else {
                eprintln!("vstop: {error}");
            }
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).into()).collect()
    }

    #[test]
    fn only_a_waiting_session_stops_without_force() {
        assert_eq!(refusal(Some(AgentState::Waiting), false, false), None);
        for state in [Some(AgentState::Working), Some(AgentState::Background), Some(AgentState::Asking), None] {
            assert!(refusal(state, false, false).is_some(), "{state:?} must need --force");
        }
    }

    #[test]
    fn a_waiting_session_is_refused_only_with_a_mark_and_a_shell() {
        assert_eq!(refusal(Some(AgentState::Waiting), true, false), None);
        assert_eq!(refusal(Some(AgentState::Waiting), false, true), None);
        let why = refusal(Some(AgentState::Waiting), true, true).unwrap();
        assert!(why.contains("Bash run_in_background"));
    }

    #[test]
    fn hook_bodies_mark_only_background_bash() {
        let bash = |input: &str| format!(r#"{{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{input}}}"#);
        assert!(starts_background_shell(&bash(r#"{"command":"sleep 99","run_in_background":true}"#)));
        assert!(!starts_background_shell(&bash(r#"{"command":"ls"}"#)));
        assert!(!starts_background_shell(&bash(r#"{"command":"ls","run_in_background":false}"#)));
        assert!(!starts_background_shell(r#"{"hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"run_in_background":true}}"#));
        assert!(!starts_background_shell(r#"{"hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"run_in_background":true}}"#));
        assert!(!starts_background_shell("not json"));
    }

    #[test]
    fn only_posix_shells_count() {
        for name in ["bash", "bash.exe", "SH", "zsh", "dash", "fish.exe"] {
            assert!(is_posix_shell(name), "{name}");
        }
        for name in ["cmd.exe", "pwsh.exe", "powershell", "node", "claude"] {
            assert!(!is_posix_shell(name), "{name}");
        }
    }

    #[test]
    fn the_mark_is_set_and_cleared_per_session() {
        mark_background_shell("mark-test");
        assert!(is_marked("mark-test"));
        clear_background_shell("mark-test");
        assert!(!is_marked("mark-test"));
    }

    #[test]
    fn parse_reads_target_reason_force_and_json() {
        let (req, json) = parse_args(&args(&["Mesh-Lead", "--reason", "idle 75 min", "--force", "--json"]))
            .unwrap()
            .unwrap();
        assert_eq!(req.target, "Mesh-Lead");
        assert_eq!(req.reason.as_deref(), Some("idle 75 min"));
        assert!(req.force && json);
        let (req, json) = parse_args(&args(&["--reason=parked", "12345678"])).unwrap().unwrap();
        assert_eq!((req.target.as_str(), req.reason.as_deref(), req.force, json), ("12345678", Some("parked"), false, false));
        assert!(parse_args(&args(&["--help"])).unwrap().is_none());
        for values in [vec![], vec!["a", "b"], vec!["a", "--reason"], vec!["a", "--kill"]] {
            assert!(parse_args(&args(&values)).is_err(), "{values:?}");
        }
    }

    #[test]
    fn request_rejects_unknown_fields() {
        assert!(serde_json::from_value::<Request>(json!({"sessionId":"s","target":"t","force":true})).is_ok());
        assert!(serde_json::from_value::<Request>(json!({"sessionId":"s","target":"t","keepAlive":true})).is_err());
    }
}
