//! Per-session private flag: a private session is never indexed for search, and no other session may read
//! its conversation through `vrefer` or `vrefer --ask`.
//!
//! The flag comes from the session's file preset (`private: true` in presets.json), see
//! `agent/preset_file.rs`. It lives in its own table, so deleting a session removes its flag by cascade.

use std::collections::HashSet;

use rusqlite::{params, Connection};

use crate::db::table_exists;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS session_private (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    set_at INTEGER NOT NULL
);";

/// Whether the session is private. A missing table (a degraded or very old database) means not private.
pub fn is_private(conn: &Connection, session_id: &str) -> bool {
    table_exists(conn, "session_private")
        && conn
            .query_row("SELECT 1 FROM session_private WHERE session_id = ?1", [session_id], |_| Ok(()))
            .is_ok()
}

/// Every private session id.
pub fn ids(conn: &Connection) -> HashSet<String> {
    if !table_exists(conn, "session_private") {
        return HashSet::new();
    }
    let Ok(mut stmt) = conn.prepare("SELECT session_id FROM session_private") else {
        return HashSet::new();
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
        return HashSet::new();
    };
    rows.filter_map(Result::ok).collect()
}

/// Make the row match `private`: set it (keeping the original `set_at`) or clear it.
pub fn set(conn: &Connection, session_id: &str, private: bool) -> Result<(), String> {
    if private {
        conn.execute(
            "INSERT OR IGNORE INTO session_private(session_id, set_at) VALUES (?1, ?2)",
            params![session_id, now_secs()],
        )
    } else {
        conn.execute("DELETE FROM session_private WHERE session_id = ?1", [session_id])
    }
    .map(|_| ())
    .map_err(|e| format!("Failed to update the session's private flag: {e}"))
}

/// The refusal for a read of a private session by another session, or None when the read is allowed.
/// `requester` is the asking session's id; a session may always read itself.
pub fn refusal(conn: &Connection, requester: &str, target_id: &str, target_name: &str) -> Option<String> {
    (requester != target_id && is_private(conn, target_id))
        .then(|| format!("{target_name} is private, so another session can't read its conversation"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(crate::db::schema::SCHEMA).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute("INSERT INTO projects (id, name, root_path, created_at) VALUES ('p','p','/p',0)", []).unwrap();
        for id in ["a", "b"] {
            conn.execute("INSERT INTO sessions (id, project_id, name, kind, created_at) VALUES (?1,'p',?1,'claude',0)", [id])
                .unwrap();
        }
        conn
    }

    #[test]
    fn set_and_clear_follow_the_flag_and_cascade_on_delete() {
        let conn = db();
        assert!(!is_private(&conn, "a"));
        set(&conn, "a", true).unwrap();
        set(&conn, "a", true).unwrap();
        assert!(is_private(&conn, "a") && !is_private(&conn, "b"));
        assert_eq!(ids(&conn), HashSet::from(["a".to_string()]));
        set(&conn, "a", false).unwrap();
        assert!(ids(&conn).is_empty());
        set(&conn, "b", true).unwrap();
        conn.execute("DELETE FROM sessions WHERE id = 'b'", []).unwrap();
        assert!(ids(&conn).is_empty());
    }

    #[test]
    fn another_session_is_refused_and_the_session_itself_is_not() {
        let conn = db();
        set(&conn, "a", true).unwrap();
        let msg = refusal(&conn, "b", "a", "alpha").unwrap();
        assert_eq!(msg, "alpha is private, so another session can't read its conversation");
        assert_eq!(refusal(&conn, "a", "a", "alpha"), None);
        assert_eq!(refusal(&conn, "a", "b", "beta"), None, "a session that is not private is readable");
    }
}
