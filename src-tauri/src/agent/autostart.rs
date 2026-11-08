//! Autostart: when VelaTerm launches, Claude terminal sessions linked to a file preset that carries an
//! `autostart` message are resumed in the background with that message as the launch prompt, through the same
//! path `vtell` uses to wake a stopped session (`tell_tui::wake`). It runs once per process start.

use std::time::Duration;

use rusqlite::Connection;
use serde_json::json;

use crate::{host::AppCtx, models::SessionKind};

/// Time for the hook server, the preset import and the local services to come up.
const SETTLE: Duration = Duration::from_secs(5);
/// Time between one start and the next.
const SPACING: Duration = Duration::from_secs(2);

/// What the selection needs to know about one session.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    pub id: String,
    pub name: String,
    pub kind: SessionKind,
    pub engine: String,
    pub archived: bool,
    pub created_at: i64,
    pub agent_session_id: Option<String>,
    /// The linked file preset's `autostart` message, `None` when unlinked or the preset has none.
    pub autostart: Option<String>,
    pub running: bool,
}

/// A session to start, or to report as skipped.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pick {
    pub id: String,
    pub name: String,
    pub message: String,
    /// Why it can't be resumed, when it is eligible but has no saved conversation.
    pub skip: Option<&'static str>,
}

/// The sessions to start, oldest first. Not archived, a Claude terminal, linked to a file preset with a
/// non-empty `autostart`, and no PTY running. One without a saved conversation is kept, marked skipped.
pub(crate) fn select(rows: Vec<Row>) -> Vec<Pick> {
    let mut rows: Vec<Row> = rows
        .into_iter()
        .filter(|r| !r.archived && r.kind == SessionKind::Claude && r.engine == "tui" && !r.running)
        .filter(|r| r.autostart.as_deref().is_some_and(|m| !m.trim().is_empty()))
        .collect();
    rows.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
    rows.into_iter()
        .map(|r| Pick {
            skip: r
                .agent_session_id
                .as_deref()
                .map_or(true, |id| id.trim().is_empty())
                .then_some("no saved conversation"),
            message: r.autostart.unwrap_or_default(),
            id: r.id,
            name: r.name,
        })
        .collect()
}

/// Every session linked to a file preset, with that preset's `autostart`. `running` is filled by the caller.
fn load(conn: &Connection, running: impl Fn(&str) -> bool) -> Result<Vec<Row>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.name, s.kind, s.engine, s.archived_at IS NOT NULL, s.created_at, s.agent_session_id, p.autostart
               FROM sessions s JOIN agent_presets p ON p.id = s.agent_preset_id
              WHERE p.source = 'file' AND s.kind = p.base_kind AND p.autostart IS NOT NULL",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                name: r.get(1)?,
                kind: SessionKind::from_db(&r.get::<_, String>(2)?),
                engine: r.get(3)?,
                archived: r.get(4)?,
                created_at: r.get(5)?,
                agent_session_id: r.get(6)?,
                autostart: r.get(7)?,
                running: false,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|r| Row { running: running(&r.id), ..r }).collect())
}

fn record(session_id: &str, outcome: &str) {
    crate::diagnostics::record("INFO", "autostart", json!({"sessionId": session_id, "outcome": outcome}));
}

/// Start the autostart sessions once, on a thread of its own.
pub fn start(app: AppCtx) {
    let _ = std::thread::Builder::new().name("autostart".into()).spawn(move || {
        std::thread::sleep(SETTLE);
        run(&app);
    });
}

fn run(app: &AppCtx) {
    let rows = {
        let Ok(conn) = app.db().conn.lock() else { return };
        match load(&conn, |id| app.pty().is_running(id)) {
            Ok(rows) => rows,
            Err(e) => {
                crate::diagnostics::record("WARN", "autostart", json!({"outcome": format!("error: {e}")}));
                return;
            }
        }
    };
    for (n, pick) in select(rows).into_iter().enumerate() {
        if n > 0 {
            std::thread::sleep(SPACING);
        }
        if let Some(reason) = pick.skip {
            record(&pick.id, &format!("skipped: {reason}"));
            continue;
        }
        // A session a tab opened meanwhile is already up.
        if app.pty().is_running(&pick.id) {
            record(&pick.id, "skipped: already running");
            continue;
        }
        match super::tell_tui::wake(app, &pick.id, &pick.message) {
            Ok("sent") => record(&pick.id, "started"),
            Ok(_) => record(&pick.id, "blocked"),
            Err(e) => record(&pick.id, &format!("error: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, created_at: i64) -> Row {
        Row {
            id: id.into(),
            name: id.into(),
            kind: SessionKind::Claude,
            engine: "tui".into(),
            archived: false,
            created_at,
            agent_session_id: Some("conv".into()),
            autostart: Some("go".into()),
            running: false,
        }
    }

    fn ids(picks: &[Pick]) -> Vec<&str> {
        picks.iter().map(|p| p.id.as_str()).collect()
    }

    #[test]
    fn only_an_eligible_session_is_picked() {
        let mut archived = row("archived", 1);
        archived.archived = true;
        let mut unlinked = row("unlinked", 1);
        unlinked.autostart = None;
        let mut blank = row("blank", 1);
        blank.autostart = Some("  ".into());
        let mut running = row("running", 1);
        running.running = true;
        let mut codex = row("codex", 1);
        codex.kind = SessionKind::Codex;
        let mut chat = row("chat", 1);
        chat.engine = "chat".into();
        let picks = select(vec![archived, unlinked, blank, running, codex, chat, row("ok", 1)]);
        assert_eq!(ids(&picks), vec!["ok"]);
        assert_eq!(picks[0].message, "go");
        assert_eq!(picks[0].skip, None);
    }

    #[test]
    fn a_session_with_no_saved_conversation_is_kept_as_skipped() {
        let mut none = row("none", 1);
        none.agent_session_id = None;
        let mut empty = row("empty", 2);
        empty.agent_session_id = Some(" ".into());
        let picks = select(vec![none, empty]);
        assert_eq!(ids(&picks), vec!["none", "empty"]);
        assert!(picks.iter().all(|p| p.skip == Some("no saved conversation")));
    }

    #[test]
    fn sessions_start_oldest_first() {
        let picks = select(vec![row("c", 30), row("b", 20), row("a2", 10), row("a1", 10)]);
        assert_eq!(ids(&picks), vec!["a1", "a2", "b", "c"]);
    }

    #[test]
    fn load_reads_linked_sessions_with_their_autostart() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::schema::SCHEMA).unwrap();
        conn.execute("INSERT INTO projects (id, name, root_path, created_at) VALUES ('p', 'p', '/w', 0)", []).unwrap();
        for (id, source, autostart) in [("a", "file", Some("go")), ("b", "file", None), ("c", "", Some("go"))] {
            conn.execute(
                "INSERT INTO agent_presets (id, name, base_kind, sort_order, created_at, source, autostart)
                 VALUES (?1, ?1, 'claude', 0, 0, NULLIF(?2, ''), ?3)",
                rusqlite::params![id, source, autostart],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, project_id, name, kind, agent_preset_id, agent_session_id, created_at)
                 VALUES (?1, 'p', ?1, 'claude', ?1, 'conv', 5)",
                [id],
            )
            .unwrap();
        }
        let rows = load(&conn, |id| id == "a").unwrap();
        assert_eq!(rows.len(), 1, "only a file preset with an autostart");
        assert_eq!((rows[0].id.as_str(), rows[0].running), ("a", true));
        assert!(select(rows).is_empty(), "it is running");
    }
}
