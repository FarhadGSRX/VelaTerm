//! Durable notification publication from host-owned task state and a bounded reply excerpt.
//! The phone's revocable publisher capability authorizes one destination; connection secrets stay local.

pub(crate) mod preview;

use crate::{host::AppCtx, web::{public_relay, share_policy::ShareScope}};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::{mpsc, Mutex, OnceLock}, time::{Duration, SystemTime, UNIX_EPOCH}};

const ORIGIN: &str = "https://velaterm.com";
pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS mobile_push_subscription (
    id TEXT PRIMARY KEY, publisher_token TEXT NOT NULL, authority TEXT, bound_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS mobile_push_outbox (
    event_id TEXT NOT NULL, subscription_id TEXT NOT NULL REFERENCES mobile_push_subscription(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL, state TEXT NOT NULL, created_at INTEGER NOT NULL,
    next_attempt_at INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(event_id, subscription_id)
);
"#;

#[derive(Clone, Serialize, Deserialize)]
struct Authority { grant_id: String, account_id: String }
struct Signal { id: String, session: String, state: String, at: i64, body: String }
static WORKERS: OnceLock<Mutex<HashMap<PathBuf, mpsc::SyncSender<Signal>>>> = OnceLock::new();
fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }
fn now_ms() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64 }
fn valid_id(value: &str) -> bool { !value.is_empty() && value.len() <= 128 && value.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') }
fn uuid(value: &str) -> bool { uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value) }

pub fn init(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    for column in ["title", "body"] {
        let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('mobile_push_outbox') WHERE name=?1)", [column], |r| r.get(0)).map_err(|e| e.to_string())?;
        if !exists { conn.execute(&format!("ALTER TABLE mobile_push_outbox ADD COLUMN {column} TEXT NOT NULL DEFAULT ''"), []).map_err(|e| e.to_string())?; }
    }
    Ok(())
}

/// The same host-owned excerpt is used by the foreground fallback and the durable push outbox.
pub fn notification_preview(app: &AppCtx, session: &str) -> Result<preview::Preview, String> {
    let state = crate::session_state::snapshot().get(session).and_then(|record| record.agent_state.clone()).unwrap_or_else(|| "waiting".into());
    preview::for_event(app, session, &state, now_ms())
}

/// The scope comes from the authenticated WebSocket, never from client-supplied JSON.
pub fn bind(app: &AppCtx, scope: Option<&ShareScope>, args: &Value) -> Result<Value, String> {
    let object = args.as_object().ok_or("Invalid notification subscription")?;
    if object.keys().any(|key| !matches!(key.as_str(), "subscriptionId" | "publisherToken")) {
        return Err("Unsupported notification subscription fields".into());
    }
    let id = args["subscriptionId"].as_str().filter(|id| uuid(id)).ok_or("Invalid subscription ID")?;
    let token = args["publisherToken"].as_str().filter(|token| token.len() == 43 && valid_id(token)).ok_or("Invalid publisher credential")?;
    let authority = match scope {
        Some(scope) => {
            let (grant_id, account_id) = scope.push_authority.as_ref().ok_or("Share authorization is unavailable")?;
            if public_relay::share_scope_for(app, grant_id, account_id).is_none() { return Err("Share is no longer available".into()); }
            Some(Authority { grant_id: grant_id.clone(), account_id: account_id.clone() })
        }
        None => None,
    };
    let (status, verified) = post(id, token, "verify", &json!({}))?;
    if status != 200 || verified["active"] != true { return Err("Notification subscription is unavailable".into()); }
    match &authority {
        Some(authority) if verified["grantId"] == authority.grant_id && verified["accountId"] == authority.account_id => {},
        None if verified["grantId"].is_null() => {},
        _ => return Err("Notification subscription belongs to a different connection scope".into()),
    }
    {
        let conn = app.db().conn.lock().map_err(|_| "Database unavailable")?;
        let count: i64 = conn.query_row("SELECT count(*) FROM mobile_push_subscription", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM mobile_push_subscription WHERE id=?1)", [id], |r| r.get(0)).map_err(|e| e.to_string())?;
        if count >= 1000 && !exists { return Err("Too many notification subscriptions".into()); }
        let authority = authority.map(|value| serde_json::to_string(&value)).transpose().map_err(|_| "Invalid notification authority")?;
        conn.execute("INSERT INTO mobile_push_subscription(id,publisher_token,authority,bound_at) VALUES (?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET publisher_token=excluded.publisher_token,authority=excluded.authority,bound_at=excluded.bound_at",
            params![id, token, authority, now()]).map_err(|e| e.to_string())?;
    }
    start(app)?;
    Ok(json!({"active":true,"protocol":1}))
}

fn post(id: &str, token: &str, action: &str, body: &Value) -> Result<(u16, Value), String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).redirects(0).build();
    let result = agent.post(&format!("{ORIGIN}/api/mobile-push/subscriptions/{id}/{action}"))
        .set("Authorization", &format!("Bearer {token}")).set("Content-Type", "application/json")
        .send_string(&body.to_string());
    match result {
        Ok(response) => {
            use std::io::Read;
            let status = response.status();
            let mut bytes = Vec::new();
            response.into_reader().take(16385).read_to_end(&mut bytes).map_err(|_| "Invalid notification response")?;
            if bytes.len() > 16384 { return Err("Invalid notification response".into()); }
            let body = serde_json::from_slice(&bytes).map_err(|_| "Invalid notification response")?;
            Ok((status, body))
        },
        Err(ureq::Error::Status(status, _)) => Ok((status, Value::Null)),
        Err(_) => Err("Notification relay is unavailable".into()),
    }
}

pub fn start(app: &AppCtx) -> Result<(), String> {
    let path = app.data_dir()?;
    let mut workers = WORKERS.get_or_init(|| Mutex::new(HashMap::new())).lock().map_err(|_| "Notification worker unavailable")?;
    if workers.contains_key(&path) { return Ok(()); }
    let (sender, receiver) = mpsc::sync_channel(1024);
    let app = app.clone();
    std::thread::Builder::new().name("mobile-push".into()).spawn(move || loop {
        match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(signal) => { if persist(&app, signal).is_err() { crate::diagnostic_warn!("mobile push event persistence failed"); } }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if drain(&app).is_err() { crate::diagnostic_warn!("mobile push outbox processing failed"); }
    }).map_err(|_| "Cannot start notification worker")?;
    workers.insert(path, sender);
    Ok(())
}

/// Called only for the same new, non-silent events that raise the authoritative unread marker.
pub fn observe(app: &AppCtx, session: &str, state: &str, body: Option<&str>) {
    if !valid_id(session) || !matches!(state, "waiting" | "asking") { return; }
    let Some(workers) = WORKERS.get() else { return; };
    let Ok(path) = app.data_dir() else { return; };
    let Ok(workers) = workers.lock() else { return; };
    if let Some(sender) = workers.get(&path) {
        let event = Signal { id: uuid::Uuid::new_v4().to_string(), session: session.into(), state: state.into(), at: now_ms(), body: preview::plain(body.unwrap_or(""), 240) };
        if sender.try_send(event).is_err() { crate::diagnostic_warn!("mobile push event queue is full"); }
    }
}

fn covers(app: &AppCtx, authority: Option<&str>, session: &str) -> bool {
    match authority {
        None => {
            let Ok(conn) = app.db().conn.lock() else { return false; };
            crate::db::repo::get_session(&conn, session).ok().flatten().is_some()
        }
        Some(raw) => serde_json::from_str::<Authority>(raw).ok()
            .and_then(|a| public_relay::share_scope_for(app, &a.grant_id, &a.account_id))
            .is_some_and(|scope| scope.covers_session(app, session)),
    }
}

fn persist(app: &AppCtx, signal: Signal) -> Result<(), String> {
    let subscriptions = {
        let conn = app.db().conn.lock().map_err(|_| "Database unavailable")?;
        let mut stmt = conn.prepare("SELECT id,authority FROM mobile_push_subscription").map_err(|e| e.to_string())?;
        let values = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))
            .map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        values
    };
    let mut content = None;
    for (id, authority) in subscriptions {
        if !covers(app, authority.as_deref(), &signal.session) { continue; }
        let content = content.get_or_insert_with(|| {
            let mut value = preview::for_event(app, &signal.session, &signal.state, signal.at).unwrap_or_default();
            if value.body.is_empty() { value.body = signal.body.clone(); }
            value
        });
        let conn = app.db().conn.lock().map_err(|_| "Database unavailable")?;
        conn.execute("DELETE FROM mobile_push_outbox WHERE created_at<?1", [now() - 3600]).map_err(|e| e.to_string())?;
        let count: i64 = conn.query_row("SELECT count(*) FROM mobile_push_outbox WHERE subscription_id=?1", [&id], |r| r.get(0)).map_err(|e| e.to_string())?;
        if count >= 1000 { crate::diagnostic_warn!("mobile push destination queue is full"); continue; }
        conn.execute("INSERT OR IGNORE INTO mobile_push_outbox(event_id,subscription_id,session_id,state,created_at,next_attempt_at,title,body) VALUES (?1,?2,?3,?4,?5,?5,?6,?7)",
            params![signal.id, id, signal.session, signal.state, now(), content.title, content.body]).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn drain(app: &AppCtx) -> Result<(), String> {
    for _ in 0..8 {
        let item = {
            let conn = app.db().conn.lock().map_err(|_| "Database unavailable")?;
            conn.query_row("SELECT e.event_id,e.subscription_id,e.session_id,e.state,e.created_at,e.attempts,s.publisher_token,s.authority,e.title,e.body FROM mobile_push_outbox e JOIN mobile_push_subscription s ON s.id=e.subscription_id WHERE next_attempt_at<=?1 ORDER BY next_attempt_at LIMIT 1", [now()], |r| Ok((
                r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?, r.get::<_, u32>(5)?, r.get::<_, String>(6)?, r.get::<_, Option<String>>(7)?, r.get::<_, String>(8)?, r.get::<_, String>(9)?
            ))).optional().map_err(|e| e.to_string())?
        };
        let Some((event, subscription, session, state, created, attempts, token, authority, title, body)) = item else { break; };
        let eligible = created >= now() - 3600 && attempts < 10 && covers(app, authority.as_deref(), &session);
        let status = if eligible { post(&subscription, &token, "events", &json!({"eventId":event,"sessionId":session,"state":state,"title":title,"body":body})).map(|r| r.0).unwrap_or(0) } else { 204 };
        let conn = app.db().conn.lock().map_err(|_| "Database unavailable")?;
        if status == 401 || status == 404 {
            conn.execute("DELETE FROM mobile_push_subscription WHERE id=?1", [&subscription]).map_err(|e| e.to_string())?;
        } else if (200..300).contains(&status) || (400..500).contains(&status) && status != 429 {
            conn.execute("DELETE FROM mobile_push_outbox WHERE event_id=?1 AND subscription_id=?2", params![event, subscription]).map_err(|e| e.to_string())?;
        } else {
            let delay = (5_i64 << attempts.min(8)).min(900);
            conn.execute("UPDATE mobile_push_outbox SET attempts=attempts+1,next_attempt_at=?1 WHERE event_id=?2 AND subscription_id=?3", params![now() + delay, event, subscription]).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_outbox_migration_is_idempotent_and_keeps_pending_events() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute("INSERT INTO mobile_push_subscription VALUES ('sub','token',NULL,1)", []).unwrap();
        conn.execute("INSERT INTO mobile_push_outbox VALUES ('event','sub','session','waiting',1,1,0)", []).unwrap();
        init(&conn).unwrap(); init(&conn).unwrap();
        let row: (String, String, String) = conn.query_row("SELECT event_id,title,body FROM mobile_push_outbox", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(row, ("event".into(), "".into(), "".into()));
    }
    #[test]
    fn outbox_is_scoped_deduplicated_and_survives_reopening() {
        let dir = std::env::temp_dir().join(format!("mobile-push-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.db");
        let db = crate::db::Db::open(&path).unwrap();
        let session = {
            let conn = db.conn.lock().unwrap();
            let project = crate::db::repo::create_virtual_project(&conn, "Private project").unwrap();
            let session = crate::db::repo::create_session(&conn, &project.id, None, "Private title", crate::models::SessionKind::Claude, None, None, None, None, None).unwrap();
            conn.execute("INSERT INTO mobile_push_subscription VALUES ('full','capability',NULL,?1)", [now()]).unwrap();
            conn.execute("INSERT INTO mobile_push_subscription VALUES ('revoked','capability',?1,?2)", params![r#"{"grant_id":"missing","account_id":"missing"}"#, now()]).unwrap();
            session.id
        };
        let app = AppCtx::Headless(std::sync::Arc::new(crate::host::HeadlessHost::new(dir.clone(), db)));
        for _ in 0..2 { persist(&app, Signal { id: "event".into(), session: session.clone(), state: "waiting".into(), at: now_ms(), body: "Current result".into() }).unwrap(); }
        drop(app);
        let db = crate::db::Db::open(&path).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            let count: i64 = conn.query_row("SELECT count(*) FROM mobile_push_outbox", [], |r| r.get(0)).unwrap();
            assert_eq!(count, 1);
            let subscription: String = conn.query_row("SELECT subscription_id FROM mobile_push_outbox", [], |r| r.get(0)).unwrap();
            assert_eq!(subscription, "full");
            let mut stmt = conn.prepare("PRAGMA table_info(mobile_push_outbox)").unwrap();
            let columns = stmt.query_map([], |r| r.get::<_, String>(1)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
            assert!(!columns.iter().any(|name| matches!(name.as_str(), "url" | "password" | "private_key")));
            let content: (String, String) = conn.query_row("SELECT title,body FROM mobile_push_outbox", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            assert_eq!(content, ("Private title".into(), "Current result".into()));
            conn.execute("DELETE FROM mobile_push_subscription WHERE id='full'", []).unwrap();
            assert_eq!(conn.query_row("SELECT count(*) FROM mobile_push_outbox", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        }
        drop(db); std::fs::remove_dir_all(dir).unwrap();
    }
}
