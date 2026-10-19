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
//!   message is the first thing the resumed Claude reads. It starts in the session's directory, else its
//!   project's, as a tab would. Receipt `sent`, or `blocked` when Claude stops at a prompt before reading
//!   it (folder trust, login), which also reports the session `asking`. A tab opened later attaches to
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
CREATE TABLE IF NOT EXISTS session_tell_delivered (
 id TEXT PRIMARY KEY REFERENCES session_tells(id) ON DELETE CASCADE,
 receipt TEXT NOT NULL
);
";

/// Whether an executable name is Claude Code's. The native installer ships `claude` / `claude.exe`; an npm
/// install runs under node, which the PTY tree cannot tell apart from any other node process, so it is not
/// recognised and such a session gets the shell-safety refusal rather than a guess.
fn is_claude_executable(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "claude" || name == "claude.exe"
}

/// Whether a running Claude is at its prompt, from its last reported state. Every turn starts with a
/// prompt-submit hook that reports `working`, and the state is dropped when the terminal exits, so a Claude
/// that has reported nothing since its terminal started has not begun a turn: it is waiting for input.
fn ready_for_input(state: Option<&str>) -> bool {
    matches!(state, None | Some("waiting") | Some("idle"))
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

/// One write, so the paste and its Enter are queued together or not at all.
fn type_in(app: &AppCtx, target: &str, text: &str) -> Result<(), String> {
    app.pty().write(target, &format!("{}\r", paste_bytes(text)))
}

/// The receipt a message already got, so a retry with the same ID reports it instead of delivering again.
fn delivered_receipt(app: &AppCtx, id: &str) -> Result<Option<&'static str>, String> {
    let delivered: Option<String> = app
        .db()
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT receipt FROM session_tell_delivered WHERE id=?1", [id], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(delivered.map(|receipt| if receipt == "blocked" { "blocked" } else { "sent" }))
}

fn mark_delivered(app: &AppCtx, ids: &[String], receipt: &str) {
    if let Ok(conn) = app.db().conn.lock() {
        for id in ids {
            let _ = conn.execute(
                "INSERT OR IGNORE INTO session_tell_delivered(id,receipt) VALUES (?1,?2)",
                params![id, receipt],
            );
        }
    }
}

fn enqueue(app: &AppCtx, target: &str, ids: &[String]) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let conn = app.db().conn.lock().map_err(|_| "The database is unavailable")?;
    for id in ids {
        conn.execute(
            "INSERT OR IGNORE INTO session_tell_pending(id,target_id,queued_at) VALUES (?1,?2,?3)",
            params![id, target, now],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn is_pending(app: &AppCtx, id: &str) -> Result<bool, String> {
    Ok(app
        .db()
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT 1 FROM session_tell_pending WHERE id=?1", [id], |_| Ok(()))
        .optional()
        .map_err(|e| e.to_string())?
        .is_some())
}

/// The directory a woken session starts in: its own, else its project's, as when a tab opens it. Most
/// sessions store no directory of their own, and the PTY falls back to the home directory, where Claude
/// finds no conversation to resume and stops at its folder-trust prompt.
fn wake_cwd(own: Option<String>, project_root: Option<String>) -> Option<String> {
    own.filter(|d| !d.trim().is_empty()).or(project_root)
}

/// Start a stopped Claude terminal session from the backend, resuming its conversation with `wire` as the
/// launch prompt. The terminal starts at a default size; the first tab to attach resizes it. Returns the
/// receipt: `sent` once Claude has read the message, `blocked` when Claude is up but stopped short of it.
fn wake(app: &AppCtx, target: &str, wire: &str) -> Result<&'static str, String> {
    use crate::models::SessionKind;
    let (session, cwd, extra_args, permission_mode) = {
        let conn = app.db().conn.lock().map_err(|_| "The database is unavailable")?;
        let session = crate::db::repo::get_session(&conn, target)?.ok_or("The target session no longer exists")?;
        let cwd = wake_cwd(session.cwd.clone(), crate::db::repo::get_project_root(&conn, &session.project_id)?);
        let stored = crate::db::repo::get_permission_mode(&conn, target)?;
        let mode = crate::agent::permission_catalog::effective(&conn, SessionKind::Claude, stored.as_deref())?;
        let args = crate::db::repo::get_agent_args(&conn, target)?;
        let args = crate::agent::inject::merge_permission_flag(SessionKind::Claude, mode.as_deref(), args.as_deref());
        (session, cwd, args, mode)
    };
    let resume_id = crate::agent::resume::checked_resume_id(SessionKind::Claude, session.agent_session_id.clone())?
        .ok_or("The target has no saved Claude conversation to resume. Open it once, then send again")?;
    crate::agent::inject::set_resume_prompt(target, wire);
    let spawned = app.pty().spawn(
        app.clone(),
        target.to_string(),
        SessionKind::Claude,
        session.shell.clone(),
        cwd,
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
    // A launch consumes the prompt. Whatever is left was not delivered by any launch.
    let unclaimed = crate::agent::inject::clear_resume_prompt(target);
    let spawned = spawned?;
    if spawned.attached {
        // A tab opened the session at the same moment. If its launch took the message, it is delivered.
        return if unclaimed {
            Err("The target terminal started while the message was being sent. Send again".into())
        } else {
            Ok("sent")
        };
    }
    // The placeholder subscriber owns the terminal's size, and no real client ever matches it: drop it so the
    // ownership is released and the first tab to open the session sizes the terminal itself.
    app.pty().detach(app, target, spawned.sub_id, "vtell");
    let launch = spawned.launch.ok_or("The target terminal has no Claude launch command")?;
    app.pty().write(target, &format!("{launch}\r"))?;
    if confirm_woken(app, target)? {
        return Ok("sent");
    }
    // Claude is up but has not submitted its launch prompt: a folder-trust or login prompt, or a resume that
    // is still loading. The message is in its launch command and is read once that clears, so it counts as
    // delivered, and a retry must not type it in again. Report the session as asking, so a sweep leaves it
    // alone and a later message queues instead of being typed into the prompt.
    if ensure_claude_in_terminal(app, target).is_err() {
        return Err("Claude exited in the woken terminal before reading the message. Open the session to see why".into());
    }
    let asking = crate::pty::StatusSignal::State {
        state: crate::pty::AgentState::Asking,
        silent: false,
        authoritative: false,
    };
    crate::agent::status_watch::record(target, crate::pty::AgentState::Asking);
    app.emit(&crate::pty::StatusSignal::event_name(target), asking);
    Ok("blocked")
}

/// How long a wake waits for the resumed Claude to take its message.
const WAKE_CONFIRM: std::time::Duration = std::time::Duration::from_secs(30);

/// Wait until the woken Claude has read the message. Typing the launch command only proves the shell got it:
/// Claude can still exit at once (a launch argument it rejects, a missing binary), leaving a bare shell, or
/// stop at a prompt before its first turn. A resumed Claude submits its launch prompt as its first turn, and
/// that turn's prompt-submit hook reports the session working, so that is the receipt: `true`. `false` when
/// the wait runs out with the terminal still open; an error when it closes.
fn confirm_woken(app: &AppCtx, target: &str) -> Result<bool, String> {
    let deadline = std::time::Instant::now() + WAKE_CONFIRM;
    while std::time::Instant::now() < deadline {
        if !app.pty().is_running(target) {
            return Err("The woken terminal closed before Claude read the message".into());
        }
        if app.pty().cached_agent_state(target).as_deref() == Some("working") {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Ok(false)
}

/// Deliver an already-recorded `session_tells` message to a terminal session.
pub fn deliver(app: &AppCtx, target: &str, wire: &str, id: &str) -> Result<&'static str, String> {
    // A retry after an uncertain receipt: report what already happened rather than deliver again.
    if let Some(receipt) = delivered_receipt(app, id)? {
        return Ok(receipt);
    }
    let ids = [id.to_string()];
    if !app.pty().is_running(target) {
        // Also covers a message queued while the terminal ran: it goes out with the wake, and only once.
        let receipt = wake(app, target, wire)?;
        if let Ok(conn) = app.db().conn.lock() {
            let _ = conn.execute("DELETE FROM session_tell_pending WHERE id=?1", [id]);
        }
        mark_delivered(app, &ids, receipt);
        return Ok(receipt);
    }
    ensure_claude_in_terminal(app, target)?;
    let state = app.pty().cached_agent_state(target);
    if !ready_for_input(state.as_deref()) {
        // A retry of a message still waiting for its turn to end is reported again rather than re-queued.
        if !is_pending(app, id)? {
            enqueue(app, target, &ids)?;
        }
        return Ok(waiting_receipt(state.as_deref()));
    }
    // Claude is at its prompt: type in everything still waiting for it, oldest first, with this message. A
    // message queued while it was busy is otherwise held until its next turn ends, and a session that went
    // idle before the message was queued sends no further idle notice to deliver it.
    let waiting = take_pending(app, target);
    let earlier: Vec<String> = waiting.iter().map(|(queued, _)| queued.clone()).filter(|queued| queued != id).collect();
    let mut wires: Vec<String> = waiting.into_iter().filter(|(queued, _)| queued != id).map(|(_, w)| w).collect();
    wires.push(wire.to_string());
    if let Err(error) = type_in(app, target, &wires.join("\n\n")) {
        let _ = enqueue(app, target, &earlier);
        return Err(error);
    }
    let mut all = earlier;
    all.push(id.to_string());
    mark_delivered(app, &all, "sent");
    Ok("sent")
}

/// Remove and return a session's waiting messages as (id, wire), oldest first.
fn take_pending(app: &AppCtx, target: &str) -> Vec<(String, String)> {
    let Ok(mut conn) = app.db().conn.lock() else {
        return Vec::new();
    };
    let Ok(tx) = conn.transaction() else {
        return Vec::new();
    };
    let wires: Vec<(String, String)> = tx
        .prepare(
            "SELECT p.id, t.wire FROM session_tell_pending p JOIN session_tells t ON t.id=p.id
             WHERE p.target_id=?1 ORDER BY p.queued_at, p.id",
        )
        .and_then(|mut stmt| {
            stmt.query_map([target], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
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
            let taken = take_pending(app, target);
            if taken.is_empty() {
                return None;
            }
            let (ids, wires): (Vec<String>, Vec<String>) = taken.into_iter().unzip();
            mark_delivered(app, &ids, "sent");
            Some(stop_response(&wires))
        }
        // Idle without a Stop means the turn was interrupted; nothing is mid-turn, so type them in.
        "idle" => {
            if ensure_claude_in_terminal(app, target).is_err() {
                return None;
            }
            let taken = take_pending(app, target);
            if !taken.is_empty() {
                let app = app.clone();
                let target = target.to_string();
                std::thread::spawn(move || {
                    let (ids, wires): (Vec<String>, Vec<String>) = taken.into_iter().unzip();
                    // A write that did not go through leaves the messages waiting for the next chance.
                    match type_in(&app, &target, &wires.join("\n\n")) {
                        Ok(()) => mark_delivered(&app, &ids, "sent"),
                        Err(_) => {
                            let _ = enqueue(&app, &target, &ids);
                        }
                    }
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

    fn app_with_messages(ids: &[&str]) -> AppCtx {
        let dir = std::env::temp_dir().join(format!("vlx-tell-tui-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Db::open(&dir.join("test.db")).unwrap();
        let app = AppCtx::Headless(std::sync::Arc::new(crate::host::HeadlessHost::new(dir, db)));
        let conn = app.db().conn.lock().unwrap();
        conn.execute("INSERT INTO projects(id,name,root_path,created_at) VALUES ('p','Project','/tmp',0)", []).unwrap();
        conn.execute("INSERT INTO groups(id,project_id,name,created_at) VALUES ('g','p','Group',0)", []).unwrap();
        conn.execute(
            "INSERT INTO sessions(id,project_id,group_id,name,kind,engine,cwd,created_at) VALUES ('t','p','g','T','claude','tui','/tmp',0)",
            [],
        )
        .unwrap();
        for id in ids {
            conn.execute(
                "INSERT INTO session_tells(id,sender_id,target_id,fingerprint,wire,origin) VALUES (?1,'s','t','f',?2,'{}')",
                params![id, format!("wire {id}")],
            )
            .unwrap();
        }
        drop(conn);
        app
    }

    #[test]
    fn waiting_messages_come_out_in_order_and_a_failed_paste_puts_them_back() {
        let app = app_with_messages(&["msg-a", "msg-b"]);
        enqueue(&app, "t", &["msg-a".into()]).unwrap();
        enqueue(&app, "t", &["msg-b".into()]).unwrap();
        let taken = take_pending(&app, "t");
        assert_eq!(taken.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["msg-a", "msg-b"]);
        assert_eq!(taken[0].1, "wire msg-a");
        assert!(take_pending(&app, "t").is_empty(), "taking removes them");

        // What the idle path does when its write fails: the messages wait for the next chance.
        let (ids, _): (Vec<String>, Vec<String>) = taken.into_iter().unzip();
        enqueue(&app, "t", &ids).unwrap();
        assert!(is_pending(&app, "msg-a").unwrap() && is_pending(&app, "msg-b").unwrap());
    }

    #[test]
    fn a_delivered_message_answers_its_retry_with_the_original_receipt() {
        let app = app_with_messages(&["msg-a"]);
        assert_eq!(delivered_receipt(&app, "msg-a").unwrap(), None);
        mark_delivered(&app, &["msg-a".into()], "sent");
        assert_eq!(delivered_receipt(&app, "msg-a").unwrap(), Some("sent"));
        // Recording it twice is harmless.
        mark_delivered(&app, &["msg-a".into()], "sent");
        assert_eq!(delivered_receipt(&app, "msg-a").unwrap(), Some("sent"));
    }

    #[test]
    fn a_wake_stopped_at_a_prompt_answers_its_retry_as_blocked() {
        let app = app_with_messages(&["msg-a"]);
        mark_delivered(&app, &["msg-a".into()], "blocked");
        assert_eq!(delivered_receipt(&app, "msg-a").unwrap(), Some("blocked"));
    }

    #[test]
    fn a_woken_session_starts_in_its_own_directory_else_its_projects() {
        let root = Some("/work/project".to_string());
        assert_eq!(wake_cwd(Some("/work/own".into()), root.clone()).as_deref(), Some("/work/own"));
        assert_eq!(wake_cwd(None, root.clone()).as_deref(), Some("/work/project"));
        assert_eq!(wake_cwd(Some("  ".into()), root).as_deref(), Some("/work/project"));
        assert_eq!(wake_cwd(None, None), None);
    }

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
    fn a_claude_that_has_not_started_a_turn_is_ready_for_input() {
        assert!(ready_for_input(None));
        assert!(ready_for_input(Some("waiting")));
        assert!(ready_for_input(Some("idle")));
        assert!(!ready_for_input(Some("working")));
        assert!(!ready_for_input(Some("asking")));
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
