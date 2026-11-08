//! Verified native Kiro text history. No CLI execution, source writes, or inferred tool schemas.
//! Provenance: T7 round-1/2 structure audits (SQLite conversations_v2 and paired JSONL v1).

pub(crate) mod snapshot;
pub(crate) mod parse;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq)]
pub struct KiroMessage {
    pub role: &'static str,
    pub text: String,
    pub timestamp: Option<String>,
    pub native_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Context {
    pub model: Option<String>,
    pub limit: Option<u64>,
    pub percent: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub cwd: PathBuf,
    pub title: String,
    pub updated_at: i64,
    pub context: Context,
    // Metadata can remain useful even when an event requires an unverified parser.
    pub messages: Result<Vec<KiroMessage>, String>,
    pub revision: String,
}

pub(crate) struct Roots {
    pub sessions: PathBuf,
    pub database: Option<PathBuf>,
}
impl Roots {
    pub(crate) fn native() -> Result<Self, String> {
        let home = super::kiro::kiro_home().ok_or("Kiro home directory is unavailable")?;
        // Only the macOS SQLite location has native evidence. Other platforms use the documented file store.
        let database = if cfg!(target_os = "macos") {
            crate::host::home_dir().map(|h| h.join("Library/Application Support/kiro-cli/data.sqlite3"))
        } else { None };
        Ok(Self { sessions: home.join("sessions/cli"), database })
    }
}

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 200 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a.canonicalize().unwrap_or_else(|_| a.to_path_buf()) == b.canonicalize().unwrap_or_else(|_| b.to_path_buf())
}

fn revision(parts: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    for part in parts { hash.update((part.len() as u64).to_le_bytes()); hash.update(part); }
    format!("kiro:sha256:{:x}", hash.finalize())
}

fn json(bytes: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(bytes).map_err(|_| "Kiro history is damaged: invalid JSON".into())
}

fn read_pair(roots: &Roots, id: &str) -> Result<Session, String> {
    let (meta, log) = snapshot::pair(&roots.sessions.join(format!("{id}.json")), &roots.sessions.join(format!("{id}.jsonl")))?;
    let mut session = parse::file(id, &json(&meta)?, &log)?;
    session.revision = revision(&[b"jsonl-v1", roots.sessions.to_string_lossy().as_bytes(), id.as_bytes(), &meta, &log]);
    Ok(session)
}

fn sqlite(roots: &Roots, wanted: Option<&str>) -> Result<Vec<(Option<String>, Result<Session, String>)>, String> {
    let Some(path) = roots.database.as_deref() else { return Ok(Vec::new()); };
    if !path.try_exists().map_err(|_| "Cannot inspect Kiro history database".to_string())? { return Ok(Vec::new()); }
    let db = snapshot::database(path)?;
    let exists: bool = db.conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='conversations_v2')", [], |r| r.get(0))
        .map_err(|_| "Cannot inspect Kiro history schema".to_string())?;
    if !exists { return Err("Kiro history database schema is not supported".into()); }
    let mut statement = db.conn.prepare("SELECT key, conversation_id, value, created_at, updated_at FROM conversations_v2 WHERE (?1 IS NULL OR conversation_id = ?1)")
        .map_err(|_| "Kiro history database schema is not supported".to_string())?;
    let rows = statement.query_map([wanted], |r| Ok((r.get::<_, String>(1).ok(),
        (|| Ok::<_, rusqlite::Error>((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?)))())))
        .map_err(|_| "Cannot read Kiro history database".to_string())?;
    let mut sessions = Vec::new();
    for row in rows {
        let (row_id, row) = row.map_err(|_| "Cannot read Kiro history row".to_string())?;
        sessions.push((row_id, (|| {
            let (cwd, id, raw, created, updated) = row.map_err(|_| "Kiro history row has invalid field types".to_string())?;
            let mut session = parse::sqlite(&id, &cwd, created, updated, &json(raw.as_bytes())?)?;
            session.revision = revision(&[b"sqlite-v2", path.to_string_lossy().as_bytes(), cwd.as_bytes(), id.as_bytes(), raw.as_bytes(), &created.to_le_bytes(), &updated.to_le_bytes()]);
            Ok(session)
        })()));
    }
    // The old key/value table is not an independently verified history format.
    let legacy: bool = db.conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='conversations')", [], |r| r.get(0)).unwrap_or(false);
    if wanted.is_none() && legacy {
        let count: i64 = db.conn.query_row("SELECT COUNT(*) FROM conversations", [], |r| r.get(0)).map_err(|_| "Cannot inspect legacy Kiro history".to_string())?;
        if count > 0 { sessions.push((None, Err("Legacy Kiro history records are not supported".into()))); }
    }
    Ok(sessions)
}

fn combine(mut candidates: Vec<Session>) -> Result<Session, String> {
    let mut found = candidates.pop().ok_or("Kiro history was not found for this session")?;
    let mut revisions = vec![found.revision.clone()];
    for candidate in candidates {
        if candidate.id != found.id || !same_dir(&candidate.cwd, &found.cwd) {
            return Err("Kiro history has conflicting session or project identities".into());
        }
        // Native message IDs differ between formats; they are not used as aliases across stores.
        let equal_text = match (&candidate.messages, &found.messages) {
            (Ok(a), Ok(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a,b)| a.role == b.role && a.text == b.text && a.timestamp == b.timestamp),
            _ => false,
        };
        if !equal_text || candidate.context != found.context {
            return Err("Kiro history has conflicting sources; no source was selected".into());
        }
        revisions.push(candidate.revision.clone());
        if candidate.updated_at > found.updated_at { found = candidate; }
    }
    revisions.sort();
    found.revision = revision(&revisions.iter().map(|r| r.as_bytes()).collect::<Vec<_>>());
    Ok(found)
}

pub fn read(agent_session_id: &str) -> Result<Session, String> {
    read_from(&Roots::native()?, agent_session_id)
}

pub(crate) fn read_from(roots: &Roots, id: &str) -> Result<Session, String> {
    if !valid_id(id) { return Err("Invalid Kiro session ID".into()); }
    let mut candidates = Vec::new();
    let meta = roots.sessions.join(format!("{id}.json"));
    let log = roots.sessions.join(format!("{id}.jsonl"));
    if meta.try_exists().map_err(|_| "Cannot inspect Kiro history metadata".to_string())?
        || log.try_exists().map_err(|_| "Cannot inspect Kiro history events".to_string())? {
        candidates.push(read_pair(roots, id)?);
    }
    for (_, result) in sqlite(roots, Some(id))? { candidates.push(result?); }
    combine(candidates)
}

pub(crate) fn discover(roots: &Roots, directory: &Path) -> (Vec<Session>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut candidates: BTreeMap<String, Vec<Session>> = BTreeMap::new();
    let mut invalid = HashSet::new();
    let mut ids = HashSet::new();
    match std::fs::read_dir(&roots.sessions) {
        Ok(entries) => for entry in entries {
            let Ok(entry) = entry else { warnings.push("Cannot inspect a Kiro history entry".into()); continue; };
            let path = entry.path();
            if matches!(path.extension().and_then(|s| s.to_str()), Some("json" | "jsonl")) {
                if let Some(id) = path.file_stem().and_then(|s| s.to_str()) { ids.insert(id.to_string()); }
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
        Err(_) => warnings.push("Cannot read Kiro history directory".into()),
    }
    for id in ids {
        let parsed = if valid_id(&id) { read_pair(roots, &id) } else { Err("Invalid Kiro history filename".into()) };
        match parsed {
            Ok(session) => { candidates.entry(id).or_default().push(session); },
            Err(e) => { invalid.insert(id); warnings.push(e); },
        }
    }
    match sqlite(roots, None) {
        Ok(rows) => for (row_id, row) in rows {
            match row {
                Ok(session) => { candidates.entry(session.id.clone()).or_default().push(session); },
                Err(e) => { if let Some(id) = row_id { invalid.insert(id); } warnings.push(e); },
            }
        },
        Err(e) => warnings.push(e),
    }
    let mut sessions = Vec::new();
    for (id, sources) in candidates {
        if invalid.contains(&id) { continue; }
        match combine(sources) {
            Ok(session) if same_dir(&session.cwd, directory) => {
                if let Err(e) = &session.messages { warnings.push(e.clone()); }
                sessions.push(session);
            },
            Ok(_) => {},
            Err(e) => warnings.push(e),
        }
    }
    warnings.sort(); warnings.dedup();
    (sessions, warnings)
}

#[cfg(test)]
pub(crate) mod tests;
