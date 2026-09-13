//! Memory ownership and names are snapshots, independent of the live project tree.
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn init(conn: &Connection) -> Result<(), String> {
    let migrated: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('memory_entries') WHERE name='session_id')", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    if migrated {
        return Ok(());
    }
    conn.execute_batch("SAVEPOINT memory_hierarchy;")
        .map_err(|e| e.to_string())?;
    let result = migrate(conn);
    if result.is_err() {
        let _ = conn.execute_batch("ROLLBACK TO memory_hierarchy; RELEASE memory_hierarchy;");
    } else {
        conn.execute_batch("RELEASE memory_hierarchy;")
            .map_err(|e| e.to_string())?;
    }
    result
}

fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("ALTER TABLE memory_entries ADD COLUMN session_id TEXT NOT NULL DEFAULT '';
        CREATE TABLE memory_sessions(id TEXT PRIMARY KEY, name TEXT NOT NULL, project_id TEXT NOT NULL, project_name TEXT NOT NULL);
        CREATE INDEX memory_entries_session ON memory_entries(session_id);
        UPDATE memory_entries SET session_id=CASE
          WHEN json_array_length(sources)=0 THEN ''
          WHEN (SELECT count(DISTINCT s.session_id) FROM memory_sources s JOIN json_each(memory_entries.sources) j ON j.value=s.id)=1
          THEN (SELECT s.session_id FROM memory_sources s JOIN json_each(memory_entries.sources) j ON j.value=s.id LIMIT 1)
          ELSE '__legacy__' END;
        INSERT INTO memory_sessions VALUES('__legacy__','','__legacy__','');
        -- A control-character prefix cannot occur in validated titles. Stage old keys first
        -- so a title containing JSON syntax cannot collide with another row's new key.
        UPDATE memory_entries SET title_key=char(1)||title_key;
        UPDATE memory_entries SET title_key=json_array(session_id,substr(title_key,2));") .map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT session_id,session_name FROM memory_sources ORDER BY created_at,id")
        .map_err(|e| e.to_string())?;
    let sources = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for (id, name) in sources {
        snapshot(conn, &id, &name)?;
    }
    Ok(())
}

pub fn snapshot(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    let live_tables: bool = conn.query_row("SELECT count(*)=2 FROM sqlite_master WHERE type='table' AND name IN ('projects','sessions')", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    let project: Option<(String, String)> = if live_tables {
        conn.query_row(
            "SELECT p.id,p.name FROM sessions s JOIN projects p ON p.id=s.project_id WHERE s.id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
    } else {
        None
    };
    let (project_id, project_name) = project.unwrap_or_default();
    conn.execute("INSERT OR IGNORE INTO memory_sessions(id,name,project_id,project_name) VALUES(?1,?2,?3,COALESCE((SELECT project_name FROM memory_sessions WHERE project_id=?3 LIMIT 1),?4))", params![id,name,project_id,project_name]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Rename a session's snapshot after the live session was renamed from the knowledge tree. Rows
/// unrelated to the renamed session are untouched.
pub fn rename_snapshot(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE memory_sessions SET name=?2 WHERE id=?1",
        params![id, name],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Rename a project's snapshot for every session that recorded it, matching the live project rename.
pub fn rename_project_snapshot(conn: &Connection, project_id: &str, name: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE memory_sessions SET project_name=?2 WHERE project_id=?1",
        params![project_id, name],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Re-point a session snapshot at another project after the live session moved there.
pub fn move_snapshot(
    conn: &Connection,
    id: &str,
    project_id: &str,
    project_name: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE memory_sessions SET project_id=?2,project_name=?3 WHERE id=?1",
        params![id, project_id, project_name],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn tree(
    conn: &Connection,
    filter: &str,
    values: &[rusqlite::types::Value],
) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT session_id,count(*) FROM memory_entries WHERE {filter} GROUP BY session_id"
        ))
        .map_err(|e| e.to_string())?;
    let counts = stmt
        .query_map(rusqlite::params_from_iter(values.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut projects: BTreeMap<String, Value> = BTreeMap::new();
    for (id, count) in counts {
        let metadata: Option<(String, String, String)> = conn
            .query_row(
                "SELECT name,project_id,project_name FROM memory_sessions WHERE id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let (name, project_id, project_name) = metadata.unwrap_or_default();
        let kind = if id.is_empty() {
            "manual"
        } else if id == "__legacy__" {
            "legacy"
        } else {
            "session"
        };
        let project_id = if id.is_empty() {
            "__manual__".to_string()
        } else if project_id.is_empty() {
            "__unknown__".to_string()
        } else {
            project_id
        };
        let project = projects.entry(project_id.clone()).or_insert_with(|| json!({"id":project_id,"name":project_name,"kind":if kind=="manual" || kind=="legacy" {kind} else if project_id=="__unknown__" {"unknown"} else {"project"},"count":0,"sessions":[]}));
        project["count"] = json!(project["count"].as_i64().unwrap_or(0) + count);
        project["sessions"].as_array_mut().unwrap().push(json!({"id":if id.is_empty(){"__manual__"}else{&id},"name":if name.is_empty() && kind=="session" {&id}else{&name},"kind":kind,"count":count}));
    }
    let mut result: Vec<_> = projects.into_values().collect();
    result.sort_by_key(|p| {
        (
            p["kind"].as_str() != Some("project"),
            p["name"].as_str().unwrap_or("").to_lowercase(),
            p["id"].as_str().unwrap_or("").to_string(),
        )
    });
    for project in &mut result {
        project["sessions"]
            .as_array_mut()
            .unwrap()
            .sort_by_key(|s| {
                (
                    s["name"].as_str().unwrap_or("").to_lowercase(),
                    s["id"].as_str().unwrap_or("").to_string(),
                )
            });
    }
    Ok(result)
}

pub fn location(conn: &Connection, session_id: &str) -> Result<Value, String> {
    let row: Option<(String, String, String)> = conn
        .query_row(
            "SELECT name,project_id,project_name FROM memory_sessions WHERE id=?1",
            [session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let (name, project_id, project_name) = row.unwrap_or_default();
    let kind = if session_id.is_empty() {
        "manual"
    } else if session_id == "__legacy__" {
        "legacy"
    } else {
        "session"
    };
    Ok(
        json!({"sessionId":if session_id.is_empty(){"__manual__"}else{session_id},"sessionName":name,"projectId":if kind=="manual"{"__manual__"}else if project_id.is_empty(){"__unknown__"}else{&project_id},"projectName":project_name,"kind":kind}),
    )
}
