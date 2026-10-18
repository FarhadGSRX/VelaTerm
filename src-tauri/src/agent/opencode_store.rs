//! Read-only access to OpenCode's own conversation store.
//!
//! OpenCode keeps sessions, messages, and message parts in one SQLite database rather than in flat files.
//! The chat engine reads it to replay a conversation before its server is up, the read-only session view
//! reads it for a terminal-driven session, and the Markdown export reads it for a finished conversation.
//! Nothing here writes: OpenCode owns the file, and this module only mirrors what it recorded.
//!
//! Every row stores its JSON body without the identifying columns, so the columns are folded back into the
//! value on the way out. Callers then see the same `Session`, `Message`, and `Part` shapes the HTTP API
//! returns, which is what lets one converter serve both the store and the live server.

use std::path::PathBuf;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

/// One message with its parts, in recording order.
#[derive(Clone, Debug)]
pub struct OpencodeMessage {
    pub info: Value,
    pub parts: Vec<Value>,
}

/// Where OpenCode keeps its data: `$XDG_DATA_HOME/opencode`, else `~/.local/share/opencode` — the same
/// answer on every platform, because OpenCode's own path resolution falls back to that directory when no
/// XDG variable is set.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        let base = PathBuf::from(xdg);
        if !base.as_os_str().is_empty() {
            return Some(base.join("opencode"));
        }
    }
    crate::host::home_dir().map(|home| home.join(".local").join("share").join("opencode"))
}

pub fn db_path() -> Option<PathBuf> {
    if let Some(path)=std::env::var_os("OPENCODE_DB").filter(|p|!p.is_empty()) { return Some(PathBuf::from(path)); }
    data_dir().map(|dir| dir.join("opencode.db"))
}

fn open() -> Result<Connection, String> {
    let path = db_path().ok_or("OpenCode's data directory could not be located")?;
    if !path.exists() {
        return Err("OpenCode has not recorded any conversation on this machine".to_string());
    }
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Failed to open OpenCode's store: {e}"))?;
    // OpenCode writes to the same file while it runs; wait briefly rather than fail on a busy lock.
    conn.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|e| format!("Failed to open OpenCode's store: {e}"))?;
    conn.execute_batch("PRAGMA cache_size=-8192; PRAGMA query_only=ON; BEGIN;").map_err(|e|e.to_string())?;
    Ok(conn)
}

fn parse_json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

pub(crate) fn has_table(conn: &Connection, name: &str) -> bool {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",[name],|row|row.get(0)).unwrap_or(false)
}

fn session_table(conn: &Connection, id: &str) -> Result<&'static str, String> {
    for table in ["session_v2","session"] {
        if !has_table(conn,table) {continue;}
        let exists: bool=conn.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id=?1)"),[id],|r|r.get(0))
            .map_err(|e|format!("Failed to read OpenCode's store: {e}"))?;
        if exists {return Ok(table);}
    }
    Err(format!("OpenCode has no session {id}"))
}

/// A native-store lookup covers both generations without invoking a different installed CLI.
pub fn session_exists(id: &str) -> Result<bool, String> {
    let conn=open()?;
    match session_table(&conn,id) {
        Ok(_)=>Ok(true),
        Err(error) if error.starts_with("OpenCode has no session ")=>Ok(false),
        Err(error)=>Err(error),
    }
}

/// The native model and variant used when OpenCode resumes its terminal interface.
pub fn session_model(session_id: &str) -> Result<Option<Value>, String> {
    use rusqlite::OptionalExtension;
    let conn = open()?;
    let table=session_table(&conn,session_id)?;
    let raw: Option<(Option<u64>,Option<String>)> = conn.query_row(
        &format!("SELECT octet_length(model),CASE WHEN octet_length(model)<=65536 THEN model ELSE NULL END FROM {table} WHERE id = ?1"),
        [session_id], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional().map_err(|e| format!("Failed to read OpenCode session model: {e}"))?;
    if raw.as_ref().is_some_and(|(size,_)|size.is_some_and(|size|size>65536)) {return Err("unsupported_record: OpenCode model metadata exceeds its byte budget".into());}
    Ok(raw.and_then(|(_,raw)|raw).map(|text| parse_json(&text)))
}

/// Every message of a session with its parts, oldest first.
///
/// Messages that OpenCode has reverted are left out, the way its own client hides them: a revert is
/// staged on the session and cleaned up at the next prompt, and until then the rows still exist.
pub fn messages(session_id: &str) -> Result<Vec<OpencodeMessage>, String> {
    let mut remaining=super::history_store::limits::PAGE_BYTES;
    messages_bounded(session_id,&mut remaining)
}

pub(crate) fn messages_bounded(session_id:&str,remaining:&mut usize) -> Result<Vec<OpencodeMessage>,String> {
    let conn = open()?;
    // Pin one read snapshot before the preflight so an append cannot grow a selected column between
    // counting its original bytes and materializing a small compatibility response.
    conn.execute_batch("BEGIN DEFERRED").map_err(|e|e.to_string())?;
    messages_from(&conn,session_id,remaining)
}

fn messages_from(conn: &Connection, session_id: &str, remaining: &mut usize) -> Result<Vec<OpencodeMessage>, String> {
    let table=session_table(conn,session_id)?;
    let revert = {
        let mut statement = conn
            .prepare(&format!("SELECT octet_length(revert),CASE WHEN octet_length(revert)<=65536 THEN revert ELSE NULL END FROM {table} WHERE id = ?1"))
            .map_err(|e| format!("Failed to read OpenCode's store: {e}"))?;
        let mut rows = statement
            .query([session_id])
            .map_err(|e| format!("Failed to read OpenCode's store: {e}"))?;
        match rows.next().map_err(|e| format!("Failed to read OpenCode's store: {e}"))? {
            Some(row) => {
                if row.get::<_,Option<u64>>(0).map_err(|e|e.to_string())?.is_some_and(|size|size>65536) {
                    return Err("unsupported_record: OpenCode revert metadata exceeds its byte budget".into());
                }
                row
                .get::<_, Option<String>>(1)
                .map_err(|e| format!("Failed to read OpenCode's store: {e}"))?
                .map(|text| parse_json(&text))
            },
            None => return Err(format!("OpenCode has no session {session_id}")),
        }
    };
    let mut out = if has_table(conn,"session_message") && (table=="session_v2" || !has_table(conn,"message")) {
        // The same aggregate budget as the v1 tables, counted before any message body is read.
        let bytes:u64=conn.query_row("SELECT coalesce(sum(octet_length(data)+octet_length(id)+octet_length(type)+128),0) FROM session_message WHERE session_id=?1",
            [session_id],|row|row.get(0)).map_err(|e|e.to_string())?;
        if bytes>*remaining as u64 {return Err(super::history_store::compat::UPGRADE.into());}
        let mut statement=conn.prepare("SELECT id,type,data FROM session_message WHERE session_id=?1 ORDER BY seq,id")
            .map_err(|e|format!("Failed to read OpenCode's store: {e}"))?;
        let rows=statement.query_map([session_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))
            .map_err(|e|format!("Failed to read OpenCode's store: {e}"))?;
        let mut out=Vec::new();
        for row in rows {
            let (id,kind,data)=row.map_err(|e|format!("Failed to read OpenCode's store: {e}"))?;
            let mut native=parse_json(&data);
            if !native.is_object(){return Err("OpenCode's store contains an invalid message".into());}
            native["id"]=Value::String(id);
            native["type"]=Value::String(kind);
            out.extend(crate::agent::chat::opencode_v2::messages(session_id,&native));
        }
        *remaining-=bytes as usize;
        out
    } else if has_table(conn,"message") {
        let (out,bytes)=read_messages_bounded(conn,session_id,*remaining)?;
        *remaining-=bytes;
        out
    } else {Vec::new()};
    if let Some(cut) = revert.as_ref().and_then(|value| reverted_from(value)) {
        if let Some(index) = out.iter().position(|m| m.info.get("id").and_then(Value::as_str) == Some(cut)) {
            out.truncate(index);
        }
    }
    Ok(out)
}

/// The message a staged revert cut the conversation at, when the whole message was reverted.
///
/// A revert that names a part keeps the message and drops only later parts; OpenCode's own client shows
/// such a message in full, so it is kept here too.
pub fn reverted_from(revert: &Value) -> Option<&str> {
    if revert.get("partID").and_then(Value::as_str).is_some() {
        return None;
    }
    revert.get("messageID").and_then(Value::as_str)
}

#[cfg(test)]
fn read_messages(conn: &Connection, session_id: &str) -> Result<Vec<OpencodeMessage>, String> {
    read_messages_bounded(conn,session_id,super::history_store::limits::PAGE_BYTES).map(|(messages,_)|messages)
}

fn read_messages_bounded(conn:&Connection,session_id:&str,budget:usize) -> Result<(Vec<OpencodeMessage>,usize),String> {
    // SQLite's octet_length(column) inspects the record header without loading the TEXT/BLOB body.
    // This compatibility reader has one aggregate budget, enforced before either SELECT materializes JSON.
    let bytes:u64=conn.query_row("SELECT
        coalesce((SELECT sum(octet_length(data)+octet_length(id)+128) FROM message WHERE session_id=?1),0)+
        coalesce((SELECT sum(octet_length(data)+octet_length(id)+octet_length(message_id)+128) FROM part WHERE session_id=?1),0)",
        [session_id],|row|row.get(0)).map_err(|e|e.to_string())?;
    if bytes>budget as u64 {return Err(super::history_store::compat::UPGRADE.into());}
    let mut statement = conn
        .prepare("SELECT id, data FROM message WHERE session_id = ?1 ORDER BY time_created, id")
        .map_err(|e| format!("Failed to read OpenCode's store: {e}"))?;
    let mut rows = statement
        .query([session_id])
        .map_err(|e| format!("Failed to read OpenCode's store: {e}"))?;
    let mut messages: Vec<OpencodeMessage> = Vec::new();
    while let Some(row) = rows.next().map_err(|e| format!("Failed to read OpenCode's store: {e}"))? {
        let id: String = row
            .get(0)
            .map_err(|e| format!("Failed to read OpenCode's store: {e}"))?;
        let data: String = row
            .get(1)
            .map_err(|e| format!("Failed to read OpenCode's store: {e}"))?;
        let mut info = parse_json(&data);
        if let Value::Object(map) = &mut info {
            map.insert("id".into(), Value::String(id));
            map.insert("sessionID".into(), Value::String(session_id.to_string()));
        }
        messages.push(OpencodeMessage { info, parts: Vec::new() });
    }
    let mut statement = conn
        .prepare("SELECT id, message_id, data FROM part WHERE session_id = ?1 ORDER BY time_created, id")
        .map_err(|e| format!("Failed to read OpenCode's store: {e}"))?;
    let mut rows = statement
        .query([session_id])
        .map_err(|e| format!("Failed to read OpenCode's store: {e}"))?;
    let index: std::collections::HashMap<String, usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(i, m)| m.info.get("id").and_then(Value::as_str).map(|id| (id.to_string(), i)))
        .collect();
    while let Some(row) = rows.next().map_err(|e| format!("Failed to read OpenCode's store: {e}"))? {
        let id: String = row
            .get(0)
            .map_err(|e| format!("Failed to read OpenCode's store: {e}"))?;
        let message_id: String = row
            .get(1)
            .map_err(|e| format!("Failed to read OpenCode's store: {e}"))?;
        let data: String = row
            .get(2)
            .map_err(|e| format!("Failed to read OpenCode's store: {e}"))?;
        let Some(&slot) = index.get(&message_id) else { continue };
        let mut part = parse_json(&data);
        if let Value::Object(map) = &mut part {
            map.insert("id".into(), Value::String(id));
            map.insert("messageID".into(), Value::String(message_id));
            map.insert("sessionID".into(), Value::String(session_id.to_string()));
        }
        messages[slot].parts.push(part);
    }
    Ok((messages,bytes as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_message_revert_names_its_cut() {
        assert_eq!(reverted_from(&serde_json::json!({"messageID":"msg_1"})), Some("msg_1"));
        assert_eq!(
            reverted_from(&serde_json::json!({"messageID":"msg_1","partID":"prt_2"})),
            None
        );
        assert_eq!(reverted_from(&Value::Null), None);
    }

    #[test]
    fn data_dir_follows_xdg_then_home() {
        // Whatever the machine has, the answer ends in OpenCode's own directory name.
        let dir = data_dir();
        assert!(dir.is_none_or(|d| d.ends_with("opencode")));
    }

    #[test]
    fn compatibility_budget_rejects_large_native_parts_before_materializing_them() {
        let conn=Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT);
            CREATE TABLE part(id TEXT,message_id TEXT,session_id TEXT,time_created INTEGER,data TEXT);
            INSERT INTO message VALUES('m','selected',1,'{\"role\":\"assistant\"}');
            INSERT INTO part VALUES('p','m','selected',1,CAST(zeroblob(40*1024*1024) AS TEXT));").unwrap();
        assert_eq!(read_messages(&conn,"selected").unwrap_err(),super::super::history_store::compat::UPGRADE);
        assert!(read_messages(&conn,"other").unwrap().is_empty());
    }

    #[test]
    fn v2_store_reads_sequence_and_revert_without_changing_native_rows() {
        let conn=Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE session_v2(id TEXT,revert TEXT); CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,seq INTEGER,data TEXT);
            INSERT INTO session_v2 VALUES ('ses_native',NULL);").unwrap();
        for (id,kind,seq,data) in [("msg_z","user",1,serde_json::json!({"text":"first"})),
            ("msg_a","assistant",2,serde_json::json!({"content":[{"type":"text","text":"reply"}],"time":{"completed":200}})),
            ("msg_b","user",3,serde_json::json!({"text":"reverted"}))] {
            conn.execute("INSERT INTO session_message VALUES (?1,'ses_native',?2,?3,?4)",rusqlite::params![id,kind,seq,data.to_string()]).unwrap();
        }
        let before:i64=conn.query_row("SELECT total_changes()",[],|r|r.get(0)).unwrap();
        let rows=messages_from(&conn,"ses_native",&mut usize::MAX.clone()).unwrap();
        assert_eq!(rows.iter().map(|m|m.info["id"].as_str().unwrap()).collect::<Vec<_>>(),["msg_z","msg_a","msg_b"]);
        assert_eq!(conn.query_row::<i64,_,_>("SELECT total_changes()",[],|r|r.get(0)).unwrap(),before);
        conn.execute("UPDATE session_v2 SET revert=?1",[serde_json::json!({"messageID":"msg_b"}).to_string()]).unwrap();
        assert_eq!(messages_from(&conn,"ses_native",&mut usize::MAX.clone()).unwrap().len(),2);
        conn.execute_batch("CREATE TABLE session(id TEXT,revert TEXT); CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT);
            CREATE TABLE part(id TEXT,message_id TEXT,session_id TEXT,time_created INTEGER,data TEXT);
            INSERT INTO session VALUES ('ses_v1',NULL);
            INSERT INTO session_message VALUES ('msg_setting','ses_v1','model-switched',1,'{}');").unwrap();
        assert!(messages_from(&conn,"ses_v1",&mut usize::MAX.clone()).unwrap().is_empty());
    }
}
