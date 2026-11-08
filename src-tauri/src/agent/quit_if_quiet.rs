//! `--quit-if-quiet`: a second launch of the app hands this argument to the running instance, which exits
//! without the quit dialog only when every session is quiet by `vstop`'s rules, and otherwise stays up.
//! Either way it writes `quit-if-quiet.json` in the app data directory, so the caller can read the outcome.

use std::path::Path;

use serde_json::{json, Value};

use crate::{host::AppCtx, models::SessionKind, pty::AgentState};

/// The argument a second launch carries.
pub const ARG: &str = "--quit-if-quiet";
/// The result file, in the app data directory.
pub const RESULT_FILE: &str = "quit-if-quiet.json";

/// Whether a launch's arguments (program name first) ask for the quiet quit.
pub fn requested(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a == ARG)
}

/// One session with a running process, as the quiet decision sees it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Candidate {
    pub session_id: String,
    pub name: String,
    pub kind: SessionKind,
    /// A native chat conversation, with no PTY.
    pub chat: bool,
    /// The last state the session reported.
    pub state: Option<AgentState>,
    /// A Bash `run_in_background` was started in it (see `stop::mark_background_shell`).
    pub marked: bool,
    /// A shell is found below its Claude, for a marked waiting session.
    pub shell_found: bool,
    /// A chat conversation has a turn in progress.
    pub turn_in_progress: bool,
}

/// A session that keeps the app up, and why.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Busy {
    pub session_id: String,
    pub name: String,
    pub why: String,
}

/// The sessions that are not quiet. A plain shell terminal or a browser tab never blocks. A chat is busy
/// only mid-turn. Any other session is judged by `vstop`'s rules without `--force`. The candidate's state
/// is already the decision from `prompt_ready::quit_state`, so a Claude with no state here may still be
/// starting: no saved conversation and under 10 minutes up, or a saved one under 15 s up.
pub(crate) fn busy_sessions(candidates: &[Candidate]) -> Vec<Busy> {
    candidates
        .iter()
        .filter_map(|c| {
            if matches!(c.kind, SessionKind::Terminal | SessionKind::Browser) {
                return None;
            }
            let why = if c.chat {
                c.turn_in_progress.then_some("a conversation turn is in progress")
            } else {
                super::stop::refusal(c.state, c.marked, c.shell_found)
            }?;
            Some(Busy { session_id: c.session_id.clone(), name: c.name.clone(), why: why.to_string() })
        })
        .collect()
}

/// Every session with a running process. Chat sessions count when their agent process is alive.
fn candidates(app: &AppCtx) -> Result<Vec<Candidate>, String> {
    let sessions = {
        let conn = app.db().conn.lock().map_err(|_| "The database is unavailable")?;
        crate::db::repo::list_all_sessions(&conn)?
    };
    let states = super::status_watch::snapshot().1;
    let mut out = Vec::new();
    for session in sessions {
        let chat = session.engine == "chat";
        let alive = if chat { app.chat().is_alive(&session.id) } else { app.pty().is_running(&session.id) };
        if !alive {
            continue;
        }
        let reported = states.iter().find(|row| row.session_id == session.id).map(|row| row.state);
        // A resumed Claude that never reported counts as at its prompt (see `prompt_ready`).
        let state = if chat {
            reported
        } else {
            super::prompt_ready::quit_state(
                session.kind,
                session.agent_session_id.as_deref().is_some_and(|id| !id.trim().is_empty()),
                app.pty().uptime(&session.id),
                reported,
            )
        };
        let shell_found = super::stop::shell_below_claude(app, &session.id, chat, state);
        out.push(Candidate {
            marked: super::stop::is_marked(&session.id),
            turn_in_progress: chat && app.chat().turn_in_progress(&session.id),
            session_id: session.id,
            name: session.name,
            kind: session.kind,
            chat,
            state,
            shell_found,
        });
    }
    Ok(out)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The result file's content: session names and reasons only, never text from a session.
pub(crate) fn result_json(at: i64, result: &str, busy: &[Busy]) -> Value {
    json!({
        "at": at,
        "result": result,
        "busy": busy
            .iter()
            .map(|b| json!({"sessionId": b.session_id, "name": b.name, "why": b.why}))
            .collect::<Vec<_>>(),
    })
}

/// Rewrite the result file and record the same fields as a diagnostics event.
pub(crate) fn write_result(data_dir: &Path, result: &str, busy: &[Busy]) {
    let value = result_json(now_ms(), result, busy);
    if let Err(e) = std::fs::create_dir_all(data_dir)
        .and_then(|_| std::fs::write(data_dir.join(RESULT_FILE), value.to_string()))
    {
        crate::diagnostics::record("WARN", "quit_if_quiet_write", json!({"error": e.to_string()}));
    }
    crate::diagnostics::record("INFO", "quit_if_quiet", value);
}

/// Handle the request in the running instance: decide, write the result, and return whether the app should
/// exit. A decision that can't be made (the session list can't be read) leaves the app up and says so.
pub fn handle(app: &AppCtx, data_dir: &Path) -> bool {
    match candidates(app) {
        Ok(candidates) => {
            let busy = busy_sessions(&candidates);
            if busy.is_empty() {
                write_result(data_dir, "quitting", &[]);
                true
            } else {
                write_result(data_dir, "busy", &busy);
                false
            }
        }
        Err(e) => {
            let busy = [Busy { session_id: String::new(), name: String::new(), why: format!("could not list sessions: {e}") }];
            write_result(data_dir, "busy", &busy);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).into()).collect()
    }

    fn claude(id: &str, state: Option<AgentState>) -> Candidate {
        Candidate {
            session_id: id.into(),
            name: format!("name-{id}"),
            kind: SessionKind::Claude,
            chat: false,
            state,
            marked: false,
            shell_found: false,
            turn_in_progress: false,
        }
    }

    fn busy_ids(candidates: &[Candidate]) -> Vec<String> {
        busy_sessions(candidates).into_iter().map(|b| b.session_id).collect()
    }

    #[test]
    fn the_argument_is_found_anywhere_after_the_program_name() {
        assert!(requested(&args(&["velaterm", "--quit-if-quiet"])));
        assert!(requested(&args(&["velaterm", "--other", "--quit-if-quiet"])));
        assert!(!requested(&args(&["velaterm"])));
        assert!(!requested(&args(&["velaterm", "--quit-if-quiet-ish", "/work"])));
        assert!(!requested(&args(&["--quit-if-quiet"])), "the program name is not an argument");
    }

    #[test]
    fn all_waiting_sessions_are_quiet() {
        let all = [claude("a", Some(AgentState::Waiting)), claude("b", Some(AgentState::Waiting))];
        assert!(busy_sessions(&all).is_empty());
        assert!(busy_sessions(&[]).is_empty(), "no sessions at all is quiet");
    }

    #[test]
    fn a_working_asking_or_background_session_is_busy() {
        for state in [AgentState::Working, AgentState::Asking, AgentState::Background] {
            let all = [claude("quiet", Some(AgentState::Waiting)), claude("x", Some(state))];
            assert_eq!(busy_ids(&all), vec!["x"], "{state:?}");
        }
        let busy = busy_sessions(&[claude("x", Some(AgentState::Working))]);
        assert_eq!(busy[0].name, "name-x");
        assert_eq!(busy[0].why, "it is working");
    }

    #[test]
    fn a_marked_bash_shell_blocks_only_while_a_shell_is_found() {
        let mut c = claude("x", Some(AgentState::Waiting));
        c.marked = true;
        assert!(busy_sessions(&[c.clone()]).is_empty(), "a stale mark");
        c.shell_found = true;
        let busy = busy_sessions(&[c]);
        assert_eq!(busy.len(), 1);
        assert!(busy[0].why.contains("run_in_background"));
    }

    #[test]
    fn a_claude_with_no_state_is_busy_but_a_plain_terminal_is_not() {
        assert_eq!(busy_ids(&[claude("x", None)]), vec!["x"]);
        let mut shell = claude("t", None);
        shell.kind = SessionKind::Terminal;
        assert!(busy_sessions(&[shell]).is_empty());
        let mut page = claude("b", None);
        page.kind = SessionKind::Browser;
        assert!(busy_sessions(&[page]).is_empty());
    }

    #[test]
    fn a_chat_is_busy_only_mid_turn() {
        let mut chat = claude("c", None);
        chat.chat = true;
        assert!(busy_sessions(&[chat.clone()]).is_empty(), "idle, with no state reported");
        chat.turn_in_progress = true;
        let busy = busy_sessions(&[chat]);
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0].why, "a conversation turn is in progress");
    }

    #[test]
    fn the_result_file_holds_names_and_reasons_only() {
        let busy = busy_sessions(&[claude("x", Some(AgentState::Asking))]);
        let value = result_json(42, "busy", &busy);
        assert_eq!(
            value,
            json!({"at": 42, "result": "busy", "busy": [{
                "sessionId": "x", "name": "name-x", "why": "it is waiting on a permission prompt or a question"
            }]})
        );
        assert_eq!(result_json(7, "quitting", &[]), json!({"at": 7, "result": "quitting", "busy": []}));
    }

    #[test]
    fn a_result_is_written_and_rewritten_in_the_data_directory() {
        let dir = std::env::temp_dir().join(format!("velaterm-qiq-{}", std::process::id())).join("nested");
        write_result(&dir, "not-running", &[]);
        let read = || serde_json::from_str::<Value>(&std::fs::read_to_string(dir.join(RESULT_FILE)).unwrap()).unwrap();
        assert_eq!(read()["result"], json!("not-running"));
        write_result(&dir, "quitting", &[]);
        assert_eq!(read()["result"], json!("quitting"));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
