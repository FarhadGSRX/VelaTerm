//! `vtell` delivery to Claude sessions running in a terminal (engine `tui`).
//!
//! A native chat session takes a message through its stream-json pipe. A terminal session has only its
//! keyboard, so a message reaches it one of two ways:
//!
//! - **Idle** (`waiting`/`idle`): the message is typed in as one bracketed paste followed by Enter, the way
//!   the mobile view sends a draft into a live Claude TUI. Receipt `sent`.
//! - **Busy** (`working`, or `asking` on a permission prompt): the message waits in `session_tell_pending`.
//!   When the turn ends, Claude's Stop hook asks VelaTerm whether to stop; the hook response blocks the stop
//!   with the pending messages as its reason, so Claude reads them and carries on without anything being
//!   typed under the user's cursor. Receipts `queued` and `blocked`. A turn that ends without a Stop
//!   (interrupted with Esc) keeps its messages until Claude's idle notification, which pastes them.
//!
//! - **Stopped** (no PTY): the session is woken. VelaTerm starts its terminal itself and resumes the saved
//!   Claude conversation with the message as the launch prompt (`claude --resume <id> "<message>"`), so the
//!   message is the first thing the resumed Claude reads. Receipt `sent`. A tab opened later attaches to
//!   that process. A session with no saved conversation to resume is refused.
//!
//! Typing into a terminal is only safe while Claude owns it. A PTY whose Claude has exited still has a live
//! shell, where pasted text would run as commands, so every paste first confirms a Claude process is in the
//! PTY's process tree.

use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use crate::host::AppCtx;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS session_tell_pending (
 id TEXT PRIMARY KEY REFERENCES session_tells(id) ON DELETE CASCADE,
 target_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 queued_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS session_tell_pending_target ON session_tell_pending(target_id, queued_at);
";

/// Whether an executable name is Claude Code's. The native installer ships `claude` / `claude.exe`; an npm
/// install runs under node, which the PTY tree cannot tell apart from any other node process, so it is not
/// recognised and such a session gets the shell-safety refusal rather than a guess.
fn is_claude_executable(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "claude" || name == "claude.exe"
}

/// The receipt for a message that cannot be typed in right now, from the target's last reported state.
fn waiting_receipt(state: Option<&str>) -> &'static str {
    if state == Some("asking") {
        "blocked"
    } else {
        "queued"
    }
}

/// Text as one bracketed paste. Control characters other than tab and newline are dropped so the message
/// cannot end the paste early or reach the terminal as a keystroke.
fn paste_bytes(text: &str) -> String {
    let clean: String = text
        .chars()
        .filter(|&c| c == '\t' || c == '\n' || !(c.is_control()))
        .collect();
    format!("\x1b[200~{clean}\x1b[201~")
}

/// Fail unless Claude itself is running in the target's terminal.
fn ensure_claude_in_terminal(app: &AppCtx, target: &str) -> Result<(), String> {
    if !app.pty().is_running(target) {
        return Err("The target terminal is not running".into());
    }
    let pid = app.pty().cwd_pid(target).ok_or("The target terminal has no process yet")?;
    match crate::procstat::subtree_has_process(pid, is_claude_executable) {
        Some(true) => Ok(()),
        _ => Err("Claude is not running in the target terminal, so the message would reach its shell. \
                  Start Claude there, then send again"
            .into()),
    }
}

fn type_in(app: &AppCtx, target: &str, text: &str) -> Result<(), String> {
    app.pty().write(target, &paste_bytes(text))?;
    app.pty().write(target, "\r")
}

/// Start a stopped Claude terminal session from the backend, resuming its conversation with `wire` as the
/// launch prompt. The terminal starts at a default size; the first tab to attach resizes it.
fn wake(app: &AppCtx, target: &str, wire: &str) -> Result<(), String> {
    use crate::models::SessionKind;
    let (session, extra_args, permission_mode) = {
        let conn = app.db().conn.lock().map_err(|_| "The database is unavailable")?;
        let session = crate::db::repo::get_session(&conn, target)?.ok_or("The target session no longer exists")?;
        let stored = crate::db::repo::get_permission_mode(&conn, target)?;
        let mode = crate::agent::permission_catalog::effective(&conn, SessionKind::Claude, stored.as_deref())?;
        let args = crate::db::repo::get_agent_args(&conn, target)?;
        let args = crate::agent::inject::merge_permission_flag(SessionKind::Claude, mode.as_deref(), args.as_deref());
        (session, args, mode)
    };
    let resume_id = crate::agent::resume::checked_resume_id(SessionKind::Claude, session.agent_session_id.clone())?
        .ok_or("The target has no saved Claude conversation to resume. Open it once, then send again")?;
    crate::agent::inject::set_resume_prompt(target, wire);
    let spawned = app.pty().spawn(
        app.clone(),
        target.to_string(),
        SessionKind::Claude,
        session.shell.clone(),
        session.cwd.clone(),
        None,
        120,
        32,
        app.hooks().endpoint(),
        Some(resume_id),
        false,
        None,
        extra_args,
        permission_mode,
        None,
        "vtell",
        false,
        Box::new(|_: &[u8]| true),
    );
    // The launch consumes the prompt; a spawn that failed or attached instead must not leave it behind.
    crate::agent::inject::clear_resume_prompt(target);
    let spawned = spawned?;
    if spawned.attached {
        return Err("The target terminal started while the message was being sent. Send again".into());
    }
    let launch = spawned.launch.ok_or("The target terminal has no Claude launch command")?;
    app.pty().write(target, &format!("{launch}\r"))
}

/// Deliver an already-recorded `session_tells` message to a terminal session.
pub fn deliver(app: &AppCtx, target: &str, wire: &str, id: &str) -> Result<&'static str, String> {
    if !app.pty().is_running(target) {
        wake(app, target, wire)?;
        return Ok("sent");
    }
    ensure_claude_in_terminal(app, target)?;
    let state = app.pty().cached_agent_state(target);
    let already_waiting = {
        let conn = app.db().conn.lock().unwrap();
        conn.query_row("SELECT 1 FROM session_tell_pending WHERE id=?1", [id], |_| Ok(()))
            .optional()
            .map_err(|e| e.to_string())?
            .is_some()
    };
    if already_waiting {
        // A retry of a message still waiting for its turn to end: report it again rather than re-queue it.
        return Ok(waiting_receipt(state.as_deref()));
    }
    match state.as_deref() {
        Some("waiting") | Some("idle") => {
            type_in(app, target, wire)?;
            Ok("sent")
        }
        other => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            app.db()
                .conn
                .lock()
                .unwrap()
                .execute(
                    "INSERT OR IGNORE INTO session_tell_pending(id,target_id,queued_at) VALUES (?1,?2,?3)",
                    params![id, target, now],
                )
                .map_err(|e| e.to_string())?;
            Ok(waiting_receipt(other))
        }
    }
}

/// Remove and return a session's waiting messages, oldest first.
fn take_pending(app: &AppCtx, target: &str) -> Vec<String> {
    let Ok(mut conn) = app.db().conn.lock() else {
        return Vec::new();
    };
    let Ok(tx) = conn.transaction() else {
        return Vec::new();
    };
    let wires: Vec<String> = tx
        .prepare(
            "SELECT t.wire FROM session_tell_pending p JOIN session_tells t ON t.id=p.id
             WHERE p.target_id=?1 ORDER BY p.queued_at, p.id",
        )
        .and_then(|mut stmt| {
            stmt.query_map([target], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();
    if wires.is_empty() {
        return wires;
    }
    if tx.execute("DELETE FROM session_tell_pending WHERE target_id=?1", [target]).is_err()
        || tx.commit().is_err()
    {
        return Vec::new();
    }
    wires
}

/// Whether a hook body is Claude Code's Stop event.
fn is_claude_stop(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("hook_event_name").and_then(Value::as_str).map(|e| e == "Stop"))
        .unwrap_or(false)
}

/// The Stop-hook response that hands Claude its waiting messages instead of letting the turn end.
fn stop_response(wires: &[String]) -> String {
    let reason = format!(
        "Other VelaTerm sessions sent you messages while you were working. Read them and respond:\n\n{}",
        wires.join("\n\n")
    );
    json!({ "decision": "block", "reason": reason }).to_string()
}

/// Called for every Claude hook a terminal session posts. Returns a body for the hook response when the
/// event delivers waiting messages; the caller answers with an empty 200 otherwise.
pub fn on_hook(app: &AppCtx, target: &str, event: &str, body: &str) -> Option<String> {
    match event {
        "waiting" if is_claude_stop(body) => {
            let wires = take_pending(app, target);
            (!wires.is_empty()).then(|| stop_response(&wires))
        }
        // Idle without a Stop means the turn was interrupted; nothing is mid-turn, so type them in.
        "idle" => {
            if ensure_claude_in_terminal(app, target).is_err() {
                return None;
            }
            let wires = take_pending(app, target);
            if !wires.is_empty() {
                let app = app.clone();
                let target = target.to_string();
                std::thread::spawn(move || {
                    let _ = type_in(&app, &target, &wires.join("\n\n"));
                });
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_claudes_own_executable_counts_as_claude() {
        assert!(is_claude_executable("claude"));
        assert!(is_claude_executable("Claude.exe"));
        assert!(!is_claude_executable("bash.exe"));
        assert!(!is_claude_executable("pwsh.exe"));
        assert!(!is_claude_executable("node.exe"));
        assert!(!is_claude_executable("claude-helper.exe"));
    }

    #[test]
    fn a_busy_target_queues_and_a_prompting_one_is_blocked() {
        assert_eq!(waiting_receipt(Some("working")), "queued");
        assert_eq!(waiting_receipt(None), "queued");
        assert_eq!(waiting_receipt(Some("asking")), "blocked");
    }

    #[test]
    fn a_paste_cannot_end_early_or_press_keys() {
        let out = paste_bytes("line one\n\tline two\x1b[201~\rrm -rf /\x03");
        assert_eq!(out, "\x1b[200~line one\n\tline two[201~rm -rf /\x1b[201~");
    }

    #[test]
    fn only_claudes_stop_event_carries_messages() {
        assert!(is_claude_stop(r#"{"hook_event_name":"Stop","stop_hook_active":false}"#));
        assert!(!is_claude_stop(r#"{"hook_event_name":"SubagentStop"}"#));
        assert!(!is_claude_stop("not json"));

        let body: Value = serde_json::from_str(&stop_response(&["first".into(), "second".into()])).unwrap();
        assert_eq!(body["decision"], "block");
        let reason = body["reason"].as_str().unwrap();
        assert!(reason.contains("first\n\nsecond"), "got: {reason}");
    }
}
