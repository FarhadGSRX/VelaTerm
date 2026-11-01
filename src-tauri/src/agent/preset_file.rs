//! Presets kept in step with a generated file, `~/.config/velaterm/presets.json`, while VelaTerm runs.
//!
//! A preset made in VelaTerm is copied onto a session once, when the session is created. A file preset is
//! followed instead: every launch of a linked session (a click, a vtell wake, a web client, the chat view)
//! first takes the preset's current arguments, permission mode and settings file. Editing the file
//! therefore changes the next resume, with no restart.
//!
//! Any agent on the machine can write the file, so it can't raise a session's permissions, choose its
//! executable, or set the flags VelaTerm owns. Every change it makes is appended to `presets-import.log`
//! beside it, one JSON object per line: visible, not asked.
//!
//! The file:
//!
//! ```json
//! {"version": 1, "presets": [{"id": "f-co/velaterm-lead", "name": "velaterm-lead", "kind": "claude",
//!   "cwd": "C:/Users/me/Code/VelaTerm", "args": ["--model", "opus"], "permissionMode": "auto",
//!   "settings": "C:/Users/me/.config/velaterm/settings/velaterm-lead.json"}]}
//! ```

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};

use crate::host::{AppCtx, PRESETS_CHANGED};
use crate::models::SessionKind;

/// `agent_presets.source` for a preset that comes from the file.
pub const SOURCE: &str = "file";
const VERSION: u64 = 1;
const POLL: Duration = Duration::from_secs(2);
/// Permission modes the file may choose. `bypassPermissions` is left out on purpose.
const PERMISSION_MODES: &[&str] = &["default", "acceptEdits", "plan", "auto"];
/// Flags VelaTerm sets itself, or that would raise permissions or detach a session from its conversation.
const RESERVED_FLAGS: &[&str] = &[
    "--settings",
    "--setting-sources",
    "--permission-mode",
    "--dangerously-skip-permissions",
    "--allow-dangerously-skip-permissions",
    "--resume",
    "-r",
    "--continue",
    "-c",
    "--fork-session",
    "--session-id",
];

fn config_dir() -> Option<PathBuf> {
    crate::host::home_dir().map(|home| home.join(".config").join("velaterm"))
}

/// The presets file VelaTerm follows.
pub fn file_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("presets.json"))
}

fn log_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("presets-import.log"))
}

fn now() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
}

/// One preset in the shape `agent_presets` stores it.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    id: String,
    name: String,
    args: Option<String>,
    permission_mode: Option<String>,
    cwd: Option<String>,
    settings: Option<String>,
}

fn map_entry(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: r.get(0)?,
        name: r.get(1)?,
        args: r.get(2)?,
        permission_mode: r.get(3)?,
        cwd: r.get(4)?,
        settings: r.get(5)?,
    })
}

const ENTRY_COLUMNS: &str = "p.id, p.name, p.agent_args, p.permission_mode, p.cwd, p.settings_path";

// ── Reading the file ──

fn parse_doc(text: &str) -> Result<Vec<Value>, String> {
    let doc: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("not valid JSON: {e}"))?;
    match doc.get("version").and_then(Value::as_u64) {
        Some(VERSION) => {}
        other => {
            let found = other.map_or_else(|| "none".to_string(), |v| v.to_string());
            return Err(format!("version must be {VERSION}, found {found}"));
        }
    }
    doc.get("presets")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| "presets must be a list".to_string())
}

/// A valid preset and the keys it carried that VelaTerm ignores, or the preset's id (when it has one)
/// and why it was rejected.
fn parse_entry(value: &Value) -> Result<(Entry, Vec<&'static str>), (Option<String>, String)> {
    let obj = value.as_object().ok_or((None, "a preset must be an object".to_string()))?;
    let id = obj
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or((None, "a preset needs an id".to_string()))?
        .to_string();
    let fail = |reason: String| (Some(id.clone()), reason);
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| fail("a preset needs a name".into()))?
        .to_string();
    match obj.get("kind") {
        None | Some(Value::Null) => {}
        Some(Value::String(kind)) if kind == "claude" => {}
        Some(other) => return Err(fail(format!("kind {other} is not supported; only \"claude\" is"))),
    }
    let args = match obj.get("args") {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => {
            let mut words = Vec::with_capacity(items.len());
            for item in items {
                let arg = item.as_str().ok_or_else(|| fail("args must be strings".into()))?;
                check_arg(arg).map_err(fail)?;
                words.push(quote(arg));
            }
            (!words.is_empty()).then(|| words.join(" "))
        }
        Some(_) => return Err(fail("args must be a list of strings".into())),
    };
    let permission_mode = match obj.get("permissionMode") {
        None | Some(Value::Null) => None,
        Some(Value::String(mode)) if mode == "bypassPermissions" => {
            return Err(fail("permissionMode bypassPermissions can't come from the file".into()));
        }
        Some(Value::String(mode)) if PERMISSION_MODES.contains(&mode.as_str()) => Some(mode.clone()),
        Some(other) => {
            return Err(fail(format!("permissionMode {other} is not one of {}", PERMISSION_MODES.join(", "))));
        }
    };
    let cwd = path_field(obj, "cwd").map_err(fail)?;
    let settings = path_field(obj, "settings").map_err(fail)?;
    let ignored = if obj.contains_key("execPath") { vec!["execPath"] } else { Vec::new() };
    Ok((Entry { id, name, args, permission_mode, cwd, settings }, ignored))
}

/// An argument VelaTerm can quote the same way for every shell it launches in, and that leaves alone the
/// flags VelaTerm owns.
fn check_arg(arg: &str) -> Result<(), String> {
    if arg.is_empty() {
        return Err("an argument is empty".into());
    }
    let flag = arg.split_once('=').map_or(arg, |(flag, _)| flag);
    if RESERVED_FLAGS.contains(&flag) {
        return Err(format!(
            "{flag} can't come from the file: VelaTerm sets it itself, or it changes permissions or the conversation"
        ));
    }
    if arg.contains('\\') {
        return Err(format!("argument {arg:?} contains a backslash; use forward slashes"));
    }
    if let Some(c) = arg.chars().find(|c| matches!(c, '"' | '`' | '$' | '%' | '!') || c.is_control()) {
        return Err(format!("argument {arg:?} contains {c:?}, which can't be quoted the same way for every shell"));
    }
    if arg.contains("  ") {
        return Err(format!("argument {arg:?} contains a double space"));
    }
    Ok(())
}

/// Bare when every character is plain, otherwise double-quoted. `check_arg` has already ruled out what
/// would still be special inside double quotes in a POSIX shell, fish, PowerShell or cmd.
fn quote(arg: &str) -> String {
    if arg.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '=' | '+')) {
        arg.to_string()
    } else {
        format!("\"{arg}\"")
    }
}

fn path_field(obj: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(path)) if path.trim().is_empty() => Ok(None),
        Some(Value::String(path)) if Path::new(path).is_absolute() => Ok(Some(path.clone())),
        Some(Value::String(path)) => Err(format!("{key} {path:?} is not an absolute path")),
        Some(_) => Err(format!("{key} must be a path")),
    }
}

// ── Importing ──

fn stored(conn: &Connection) -> Result<HashMap<String, Entry>, String> {
    let sql = format!("SELECT {ENTRY_COLUMNS} FROM agent_presets p WHERE p.source = ?1");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([SOURCE], map_entry).map_err(|e| e.to_string())?;
    rows.map(|row| row.map(|entry| (entry.id.clone(), entry)))
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())
}

fn changed_fields(old: &Entry, new: &Entry) -> Vec<&'static str> {
    [
        ("name", old.name != new.name),
        ("args", old.args != new.args),
        ("permissionMode", old.permission_mode != new.permission_mode),
        ("cwd", old.cwd != new.cwd),
        ("settings", old.settings != new.settings),
    ]
    .into_iter()
    .filter_map(|(field, changed)| changed.then_some(field))
    .collect()
}

/// Bring the file presets in `conn` in step with the file's text, and return one log entry per change. A
/// file that can't be read or parsed changes nothing, and a rejected preset keeps its last good version.
pub(crate) fn import(conn: &Connection, text: Result<String, String>, now_ms: i64) -> Vec<Value> {
    match text.and_then(|text| parse_doc(&text)).and_then(|items| apply_import(conn, &items, now_ms)) {
        Ok(lines) => lines,
        Err(reason) => vec![json!({"change": "file-error", "reason": reason})],
    }
}

fn apply_import(conn: &Connection, items: &[Value], now_ms: i64) -> Result<Vec<Value>, String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let existing = stored(&tx)?;
    let mut seen = HashSet::new();
    let mut lines = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let (entry, ignored) = match parse_entry(item) {
            Ok(parsed) => parsed,
            Err((id, reason)) => {
                if let Some(id) = &id {
                    seen.insert(id.clone());
                }
                lines.push(json!({"change": "rejected", "id": id, "reason": reason}));
                continue;
            }
        };
        if !seen.insert(entry.id.clone()) {
            lines.push(json!({"change": "rejected", "id": entry.id, "reason": "the id appears more than once"}));
            continue;
        }
        let mut line = match existing.get(&entry.id) {
            None => {
                let taken: bool = tx
                    .query_row("SELECT EXISTS(SELECT 1 FROM agent_presets WHERE id = ?1)", [&entry.id], |r| r.get(0))
                    .map_err(|e| e.to_string())?;
                if taken {
                    lines.push(json!({"change": "rejected", "id": entry.id, "reason": "the id belongs to a preset made in VelaTerm"}));
                    continue;
                }
                tx.execute(
                    "INSERT INTO agent_presets
                       (id, name, base_kind, agent_args, permission_mode, sort_order, created_at, source, cwd, settings_path)
                     VALUES (?1, ?2, 'claude', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        entry.id,
                        entry.name,
                        entry.args,
                        entry.permission_mode,
                        now_ms + index as i64,
                        now_ms / 1000,
                        SOURCE,
                        entry.cwd,
                        entry.settings
                    ],
                )
                .map_err(|e| e.to_string())?;
                json!({"change": "added", "id": entry.id})
            }
            Some(old) => {
                let fields = changed_fields(old, &entry);
                if fields.is_empty() {
                    continue;
                }
                tx.execute(
                    "UPDATE agent_presets
                        SET name = ?2, agent_args = ?3, permission_mode = ?4, cwd = ?5, settings_path = ?6, exec_path = NULL
                      WHERE id = ?1 AND source = ?7",
                    params![entry.id, entry.name, entry.args, entry.permission_mode, entry.cwd, entry.settings, SOURCE],
                )
                .map_err(|e| e.to_string())?;
                json!({"change": "updated", "id": entry.id, "fields": fields})
            }
        };
        if !ignored.is_empty() {
            line["ignored"] = json!(ignored);
        }
        lines.push(line);
    }
    let mut gone: Vec<&String> = existing.keys().filter(|id| !seen.contains(*id)).collect();
    gone.sort();
    for id in gone {
        tx.execute("DELETE FROM agent_presets WHERE id = ?1 AND source = ?2", params![id, SOURCE])
            .map_err(|e| e.to_string())?;
        lines.push(json!({"change": "removed", "id": id}));
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(lines)
}

/// Append entries to the import log.
pub(crate) fn log(lines: &[Value]) {
    let Some(path) = log_path() else { return };
    if let Err(e) = append_log(&path, lines, now().as_secs()) {
        crate::diagnostic_warn!("presets: cannot write {}: {e}", path.display());
    }
}

fn append_log(path: &Path, lines: &[Value], secs: u64) -> std::io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let ts = crate::agent::usage::unix_to_rfc3339_utc(secs);
    let mut text = String::new();
    for line in lines {
        let mut line = line.clone();
        if let Some(obj) = line.as_object_mut() {
            obj.insert("ts".into(), json!(ts));
        }
        text.push_str(&line.to_string());
        text.push('\n');
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path)?.write_all(text.as_bytes())
}

/// Size and modification time of the file at its last import, so an unchanged file isn't read again.
static LAST_SEEN: Mutex<Option<(SystemTime, u64)>> = Mutex::new(None);

/// Import the file if it has changed since the last look. A missing file changes nothing; to drop every
/// file preset, write one with an empty list.
pub fn refresh(app: &AppCtx) {
    let Some(path) = file_path() else { return };
    let mut last = LAST_SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(meta) = std::fs::metadata(&path) else { return };
    let stamp = (meta.modified().unwrap_or(UNIX_EPOCH), meta.len());
    if *last == Some(stamp) {
        return;
    }
    *last = Some(stamp);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()));
    let lines = match app.db().conn.lock() {
        Ok(conn) => import(&conn, text, now().as_millis() as i64),
        Err(_) => return,
    };
    log(&lines);
    if lines.iter().any(|l| matches!(l["change"].as_str(), Some("added" | "updated" | "removed"))) {
        app.emit(PRESETS_CHANGED, ());
    }
}

/// Follow the file for as long as VelaTerm runs.
pub fn start(app: AppCtx) {
    let _ = std::thread::Builder::new().name("preset-file".into()).spawn(move || loop {
        refresh(&app);
        std::thread::sleep(POLL);
    });
}

// ── Launching ──

/// The file preset a session is linked to, if the preset is still in the file and drives its kind.
fn linked(conn: &Connection, session_id: &str) -> Result<Option<Entry>, String> {
    let sql = format!(
        "SELECT {ENTRY_COLUMNS} FROM sessions s JOIN agent_presets p ON p.id = s.agent_preset_id
          WHERE s.id = ?1 AND p.source = ?2 AND s.kind = p.base_kind"
    );
    conn.query_row(&sql, params![session_id, SOURCE], map_entry)
        .optional()
        .map_err(|e| format!("Failed to read the session's preset: {e}"))
}

/// Before a launch: catch up with the file, then copy a linked file preset's current values onto the
/// session's row. Every launch path reads that row, so they all follow the preset, and a preset that has
/// left the file leaves its last values behind. Returns the preset's settings file, if it names one.
pub fn prepare_launch(app: &AppCtx, session_id: &str) -> Result<Option<String>, String> {
    refresh(app);
    let conn = app.db().conn.lock().map_err(|_| "The database is unavailable")?;
    follow(&conn, session_id)
}

pub(crate) fn follow(conn: &Connection, session_id: &str) -> Result<Option<String>, String> {
    let Some(preset) = linked(conn, session_id)? else { return Ok(None) };
    conn.execute(
        "UPDATE sessions SET agent_args = ?2, permission_mode = ?3, agent_path = NULL WHERE id = ?1",
        params![session_id, preset.args, preset.permission_mode],
    )
    .map_err(|e| format!("Failed to apply the session's preset: {e}"))?;
    // A model or effort chosen in VelaTerm is stored apart from the arguments, and the launch prefers it.
    let chosen = crate::agent::session_settings::from_args(SessionKind::Claude, preset.args.as_deref());
    if chosen.model.is_some() || chosen.effort.is_some() {
        conn.execute(
            "UPDATE session_model_settings SET model = COALESCE(?2, model), effort = COALESCE(?3, effort)
              WHERE session_id = ?1",
            params![session_id, chosen.model, chosen.effort],
        )
        .map_err(|e| format!("Failed to apply the session's preset: {e}"))?;
    }
    Ok(preset.settings)
}

/// The settings file of a session's linked file preset, without changing anything.
pub fn settings_path(conn: &Connection, session_id: &str) -> Result<Option<String>, String> {
    Ok(linked(conn, session_id)?.and_then(|preset| preset.settings))
}

/// A preset's settings file, which must hold a JSON object. A launch stops rather than run without it,
/// because the file may carry the rules that keep the session in check.
pub fn read_settings(file: &str) -> Result<Value, String> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| format!("Cannot read the preset's settings file {file}: {e}"))?;
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("The preset's settings file {file} is not valid JSON: {e}"))?;
    if !value.is_object() {
        return Err(format!("The preset's settings file {file} must hold a JSON object"));
    }
    Ok(value)
}

/// VelaTerm's own settings JSON layered over a preset's settings file. The file's keys stay, VelaTerm's
/// own keys win, and VelaTerm's hooks run after the file's for each event, so status reporting keeps
/// working. One `--settings` then carries both.
pub fn merge_settings(own: &str, file: &str) -> Result<String, String> {
    let mut merged = read_settings(file)?;
    let own: Value = serde_json::from_str(own).map_err(|e| format!("Failed to prepare Claude settings: {e}"))?;
    let target = merged.as_object_mut().expect("read_settings returns an object");
    for (key, value) in own.as_object().into_iter().flatten() {
        if key != "hooks" {
            target.insert(key.clone(), value.clone());
            continue;
        }
        let hooks = target.entry("hooks").or_insert_with(|| json!({}));
        let hooks = hooks
            .as_object_mut()
            .ok_or_else(|| format!("The preset's settings file {file} has hooks that are not an object"))?;
        for (event, entries) in value.as_object().into_iter().flatten() {
            let list = hooks.entry(event.clone()).or_insert_with(|| json!([]));
            let list = list
                .as_array_mut()
                .ok_or_else(|| format!("The preset's settings file {file} has hooks.{event} that is not a list"))?;
            list.extend(entries.as_array().into_iter().flatten().cloned());
        }
    }
    Ok(merged.to_string())
}

// ── Linking ──

/// Link a session to a file preset. The session takes the preset's values from its next launch.
///
/// A preset's `cwd` only places new sessions, so a session linked from another folder would run the role
/// in the wrong place. Unless `force`, the link is refused when the session's folder (its own, else its
/// project's, as when it starts) isn't the preset's `cwd`. Returns whether `force` let a mismatch through.
pub fn link(conn: &Connection, session_id: &str, preset_id: &str, force: bool) -> Result<bool, String> {
    let (kind, own, root): (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT s.kind, s.cwd, p.root_path FROM sessions s LEFT JOIN projects p ON p.id = s.project_id
              WHERE s.id = ?1",
            [session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("The session no longer exists")?;
    let (base, cwd): (String, Option<String>) = conn
        .query_row(
            "SELECT base_kind, cwd FROM agent_presets WHERE id = ?1 AND source = ?2",
            params![preset_id, SOURCE],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("presets.json has no preset {preset_id}"))?;
    if kind != base {
        return Err(format!("A {base} preset can't drive a {kind} session"));
    }
    let folder = own.filter(|d| !d.trim().is_empty()).or(root);
    let mismatch = match (&cwd, &folder) {
        (Some(cwd), Some(folder)) => !same_dir(folder, cwd),
        (Some(_), None) => true,
        (None, _) => false,
    };
    if mismatch && !force {
        return Err(format!(
            "The session's folder {} isn't the preset's cwd {}. Pass --force to link it anyway",
            folder.as_deref().unwrap_or("(none)"),
            cwd.as_deref().unwrap_or_default()
        ));
    }
    conn.execute("UPDATE sessions SET agent_preset_id = ?2 WHERE id = ?1", params![session_id, preset_id])
        .map_err(|e| e.to_string())?;
    Ok(mismatch)
}

/// Whether two folders are the same place: resolved through links where they exist, with either slash,
/// no trailing slash, and ignoring case where the file system usually does.
fn same_dir(a: &str, b: &str) -> bool {
    let key = |p: &str| {
        let path = std::fs::canonicalize(p).map_or_else(|_| p.to_string(), |c| c.to_string_lossy().into_owned());
        let path = path.strip_prefix(r"\\?\").unwrap_or(&path).replace('\\', "/");
        let path = path.trim_end_matches('/');
        if cfg!(any(windows, target_os = "macos")) { path.to_lowercase() } else { path.to_string() }
    };
    key(a) == key(b)
}

/// Unlink a session from its file preset, keeping the values the preset last gave it. Returns the preset
/// it was linked to, or `None` when it had no link. A link to a preset made in VelaTerm is refused: that
/// one only carries the session's name and icon.
pub fn unlink(conn: &Connection, session_id: &str) -> Result<Option<String>, String> {
    let current: Option<String> = conn
        .query_row("SELECT agent_preset_id FROM sessions WHERE id = ?1", [session_id], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("The session no longer exists")?;
    let Some(preset_id) = current else { return Ok(None) };
    let source: Option<Option<String>> = conn
        .query_row("SELECT source FROM agent_presets WHERE id = ?1", [&preset_id], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(None) = source {
        return Err("The session's preset was made in VelaTerm, not presets.json".into());
    }
    conn.execute("UPDATE sessions SET agent_preset_id = NULL WHERE id = ?1", [session_id])
        .map_err(|e| e.to_string())?;
    Ok(Some(preset_id))
}

/// Every file preset, in menu order, with the sessions linked to it.
pub fn list(conn: &Connection) -> Result<Value, String> {
    let sql = format!("SELECT {ENTRY_COLUMNS} FROM agent_presets p WHERE p.source = ?1 ORDER BY p.sort_order, p.created_at");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let presets = stmt
        .query_map([SOURCE], map_entry)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let mut sessions = conn
        .prepare("SELECT id, name FROM sessions WHERE agent_preset_id = ?1 AND archived_at IS NULL ORDER BY created_at")
        .map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(presets.len());
    for preset in presets {
        let linked = sessions
            .query_map([&preset.id], |r| Ok(json!({"sessionId": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?})))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        out.push(json!({
            "id": preset.id,
            "name": preset.name,
            "args": preset.args,
            "permissionMode": preset.permission_mode,
            "cwd": preset.cwd,
            "settings": preset.settings,
            "sessions": linked,
        }));
    }
    Ok(json!(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::schema::SCHEMA).unwrap();
        conn.execute("INSERT INTO projects (id, name, root_path, created_at) VALUES ('p', 'p', ?1, 0)", [abs("/work")])
            .unwrap();
        for (id, kind) in [("s", "claude"), ("t", "terminal")] {
            conn.execute(
                "INSERT INTO sessions (id, project_id, name, kind, agent_args, permission_mode, agent_path, created_at)
                 VALUES (?1, 'p', ?1, ?2, '--old', 'plan', '/bin/other', 0)",
                params![id, kind],
            )
            .unwrap();
        }
        conn
    }

    fn file(presets: Value) -> Result<String, String> {
        Ok(json!({"version": 1, "presets": presets}).to_string())
    }

    fn abs(path: &str) -> String {
        if cfg!(windows) { format!("C:{path}") } else { path.to_string() }
    }

    fn lead(args: Value) -> Value {
        json!({"id": "f-co/lead", "name": "lead", "args": args, "permissionMode": "auto", "cwd": abs("/work")})
    }

    fn row(conn: &Connection, id: &str) -> (Option<String>, Option<String>, Option<String>) {
        conn.query_row("SELECT agent_args, permission_mode, agent_path FROM sessions WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap()
    }

    #[test]
    fn import_adds_updates_and_removes_only_file_presets() {
        let conn = db();
        crate::db::repo::create_agent_preset(&conn, "mine", SessionKind::Claude, None, Some("--x"), None, None).unwrap();
        let lines = import(&conn, file(json!([lead(json!(["--model", "opus"]))])), 1000);
        assert_eq!(lines, vec![json!({"change": "added", "id": "f-co/lead"})]);
        let presets = crate::db::repo::list_agent_presets(&conn).unwrap();
        let added = presets.iter().find(|p| p.id == "f-co/lead").unwrap();
        assert_eq!(added.source.as_deref(), Some(SOURCE));
        assert_eq!(added.agent_args.as_deref(), Some("--model opus"));
        assert_eq!(added.cwd.as_deref(), Some(abs("/work").as_str()));

        assert!(import(&conn, file(json!([lead(json!(["--model", "opus"]))])), 2000).is_empty(), "no change, no line");

        let lines = import(&conn, file(json!([lead(json!(["--model", "sonnet"]))])), 3000);
        assert_eq!(lines, vec![json!({"change": "updated", "id": "f-co/lead", "fields": ["args"]})]);

        let lines = import(&conn, file(json!([])), 4000);
        assert_eq!(lines, vec![json!({"change": "removed", "id": "f-co/lead"})]);
        let left = crate::db::repo::list_agent_presets(&conn).unwrap();
        assert_eq!(left.len(), 1, "the preset made in VelaTerm stays");
        assert_eq!(left[0].name, "mine");
    }

    #[test]
    fn a_broken_file_changes_nothing() {
        let conn = db();
        import(&conn, file(json!([lead(json!([]))])), 1000);
        for text in ["{not json", r#"{"version":2,"presets":[]}"#, r#"{"version":1}"#] {
            let lines = import(&conn, Ok(text.into()), 2000);
            assert_eq!(lines.len(), 1);
            assert_eq!(lines[0]["change"], "file-error", "{text}");
        }
        let lines = import(&conn, Err("cannot read".into()), 2000);
        assert_eq!(lines[0]["reason"], "cannot read");
        assert_eq!(crate::db::repo::list_agent_presets(&conn).unwrap().len(), 1);
    }

    #[test]
    fn the_file_cannot_raise_permissions_or_take_reserved_flags() {
        let conn = db();
        import(&conn, file(json!([lead(json!(["--model", "opus"]))])), 1000);
        let mut bypass = lead(json!([]));
        bypass["permissionMode"] = json!("bypassPermissions");
        for (preset, needle) in [
            (bypass, "bypassPermissions"),
            (lead(json!(["--dangerously-skip-permissions"])), "--dangerously-skip-permissions"),
            (lead(json!(["--allow-dangerously-skip-permissions"])), "--allow-dangerously-skip-permissions"),
            (lead(json!(["--permission-mode", "default"])), "--permission-mode"),
            (lead(json!(["--settings=/x.json"])), "--settings"),
            (lead(json!(["--setting-sources", "user"])), "--setting-sources"),
            (lead(json!(["--resume", "abc"])), "--resume"),
            (lead(json!(["--session-id", "abc"])), "--session-id"),
        ] {
            let lines = import(&conn, file(json!([preset])), 2000);
            assert_eq!(lines.len(), 1, "{needle}");
            assert_eq!(lines[0]["change"], "rejected", "{needle}");
            assert_eq!(lines[0]["id"], "f-co/lead");
            assert!(lines[0]["reason"].as_str().unwrap().contains(needle), "{needle}: {}", lines[0]["reason"]);
        }
        let kept = crate::db::repo::list_agent_presets(&conn).unwrap();
        assert_eq!(kept[0].agent_args.as_deref(), Some("--model opus"), "a rejected preset keeps its last good version");
    }

    #[test]
    fn unsafe_arguments_are_rejected_and_others_quoted() {
        for bad in ["a\\b", "a\"b", "$HOME", "%PATH%", "a`b", "a!b", "", "a  b", "a\nb"] {
            assert!(check_arg(bad).is_err(), "{bad:?}");
        }
        assert_eq!(quote("--add-dir"), "--add-dir");
        assert_eq!(quote("C:/Users/me/x.json"), "C:/Users/me/x.json");
        assert_eq!(quote("plugin:zulip@market"), "\"plugin:zulip@market\"");
        assert_eq!(quote("C:/My Notes"), "\"C:/My Notes\"");
        let conn = db();
        import(&conn, file(json!([lead(json!(["--add-dir", "C:/My Notes", "--channels", "plugin:z@m"]))])), 1);
        let args = crate::db::repo::list_agent_presets(&conn).unwrap()[0].agent_args.clone().unwrap();
        assert_eq!(args, "--add-dir \"C:/My Notes\" --channels \"plugin:z@m\"");
        assert_eq!(
            crate::agent::inject::split_extra_args(Some(&args)),
            vec!["--add-dir", "C:/My Notes", "--channels", "plugin:z@m"],
            "the chat view splits it back into the same arguments"
        );
    }

    #[test]
    fn rejected_entries_name_their_reason() {
        let conn = db();
        crate::db::repo::create_agent_preset(&conn, "mine", SessionKind::Claude, None, None, None, None).unwrap();
        let mine = crate::db::repo::list_agent_presets(&conn).unwrap()[0].id.clone();
        let lines = import(
            &conn,
            file(json!([
                {"name": "no id"},
                {"id": "a"},
                {"id": "b", "name": "b", "kind": "codex"},
                {"id": "c", "name": "c", "cwd": "relative/dir"},
                {"id": "d", "name": "d", "args": "--model opus"},
                {"id": "e", "name": "e", "permissionMode": "yolo"},
                {"id": "f", "name": "f"},
                {"id": "f", "name": "f again"},
                {"id": mine, "name": "taken"},
                {"id": "g", "name": "g", "execPath": "/bin/evil"},
            ])),
            1,
        );
        let changes: Vec<(String, Value)> =
            lines.iter().map(|l| (l["change"].as_str().unwrap().to_string(), l["id"].clone())).collect();
        assert_eq!(
            changes,
            vec![
                ("rejected".into(), Value::Null),
                ("rejected".into(), json!("a")),
                ("rejected".into(), json!("b")),
                ("rejected".into(), json!("c")),
                ("rejected".into(), json!("d")),
                ("rejected".into(), json!("e")),
                ("added".into(), json!("f")),
                ("rejected".into(), json!("f")),
                ("rejected".into(), json!(mine)),
                ("added".into(), json!("g")),
            ]
        );
        assert_eq!(lines[9]["ignored"], json!(["execPath"]));
        let g = crate::db::repo::list_agent_presets(&conn).unwrap().into_iter().find(|p| p.id == "g").unwrap();
        assert_eq!(g.exec_path, None, "a file preset never carries an executable");
    }

    #[test]
    fn a_linked_session_follows_its_preset_at_each_launch() {
        let conn = db();
        let mut preset = lead(json!(["--model", "opus"]));
        preset["settings"] = json!(abs("/s.json"));
        import(&conn, file(json!([preset.clone()])), 1);
        assert_eq!(follow(&conn, "s").unwrap(), None, "an unlinked session is left alone");
        assert_eq!(row(&conn, "s"), (Some("--old".into()), Some("plan".into()), Some("/bin/other".into())));

        link(&conn, "s", "f-co/lead", false).unwrap();
        conn.execute("INSERT INTO session_model_settings (session_id, model, effort) VALUES ('s', 'haiku', 'low')", [])
            .unwrap();
        assert_eq!(follow(&conn, "s").unwrap(), Some(abs("/s.json")));
        assert_eq!(row(&conn, "s"), (Some("--model opus".into()), Some("auto".into()), None));
        let chosen = crate::agent::session_settings::stored(&conn, "s").unwrap().unwrap().0;
        assert_eq!((chosen.model.as_deref(), chosen.effort.as_deref()), (Some("opus"), Some("low")));

        preset["args"] = json!(["--model", "sonnet", "--effort", "high"]);
        import(&conn, file(json!([preset])), 2);
        follow(&conn, "s").unwrap();
        assert_eq!(row(&conn, "s").0.as_deref(), Some("--model sonnet --effort high"));

        // A preset that leaves the file leaves its last values behind.
        import(&conn, file(json!([])), 3);
        assert_eq!(follow(&conn, "s").unwrap(), None);
        assert_eq!(row(&conn, "s").0.as_deref(), Some("--model sonnet --effort high"));
    }

    #[test]
    fn a_preset_made_in_velaterm_keeps_copy_at_creation() {
        let conn = db();
        let mine = crate::db::repo::create_agent_preset(&conn, "mine", SessionKind::Claude, None, Some("--new"), None, None)
            .unwrap();
        conn.execute("UPDATE sessions SET agent_preset_id = ?1 WHERE id = 's'", [&mine.id]).unwrap();
        assert_eq!(follow(&conn, "s").unwrap(), None);
        assert_eq!(row(&conn, "s").0.as_deref(), Some("--old"));
        assert!(link(&conn, "s", &mine.id, false).is_err(), "only file presets can be linked");
        assert!(unlink(&conn, "s").is_err(), "and only their links removed");
        assert!(crate::db::repo::update_agent_preset(&conn, &mine.id, "x", None, None, None, None).is_ok());
    }

    #[test]
    fn file_presets_are_read_only_in_velaterm() {
        let conn = db();
        import(&conn, file(json!([lead(json!([]))])), 1);
        let err = crate::db::repo::update_agent_preset(&conn, "f-co/lead", "x", None, None, None, None).unwrap_err();
        assert!(err.contains("presets.json"), "{err}");
        assert!(crate::db::repo::delete_agent_preset(&conn, "f-co/lead").is_err());
    }

    #[test]
    fn link_checks_kind_and_unlink_keeps_last_values() {
        let conn = db();
        import(&conn, file(json!([lead(json!(["--model", "opus"]))])), 1);
        assert!(link(&conn, "t", "f-co/lead", false).unwrap_err().contains("terminal"));
        assert!(link(&conn, "s", "missing", false).is_err());
        assert!(link(&conn, "gone", "f-co/lead", false).is_err());
        assert_eq!(unlink(&conn, "s").unwrap(), None);
        link(&conn, "s", "f-co/lead", false).unwrap();
        let listed = list(&conn).unwrap();
        assert_eq!(listed[0]["sessions"], json!([{"sessionId": "s", "name": "s"}]));
        follow(&conn, "s").unwrap();
        assert_eq!(unlink(&conn, "s").unwrap().as_deref(), Some("f-co/lead"));
        assert_eq!(row(&conn, "s").0.as_deref(), Some("--model opus"));
        import(&conn, file(json!([lead(json!(["--model", "haiku"]))])), 2);
        assert_eq!(follow(&conn, "s").unwrap(), None);
        assert_eq!(row(&conn, "s").0.as_deref(), Some("--model opus"), "an unlinked session no longer follows");
    }

    #[test]
    fn link_refuses_a_session_outside_the_preset_folder_unless_forced() {
        let conn = db();
        import(&conn, file(json!([lead(json!([])), {"id": "f-co/free", "name": "free"}])), 1);
        // The project's folder is the preset's, written with a trailing slash.
        conn.execute("UPDATE projects SET root_path = ?1", [abs("/work/")]).unwrap();
        assert_eq!(link(&conn, "s", "f-co/lead", false), Ok(false));
        // The session's own folder comes before its project's.
        conn.execute("UPDATE sessions SET cwd = ?1 WHERE id = 's'", [abs("/elsewhere")]).unwrap();
        let err = link(&conn, "s", "f-co/lead", false).unwrap_err();
        assert!(err.contains("elsewhere") && err.contains("--force"), "{err}");
        assert_eq!(link(&conn, "s", "f-co/lead", true), Ok(true));
        // A blank folder of its own falls back to the project's.
        conn.execute("UPDATE sessions SET cwd = '  ' WHERE id = 's'", []).unwrap();
        assert_eq!(link(&conn, "s", "f-co/lead", false), Ok(false));
        // A preset with no cwd links a session anywhere.
        conn.execute("UPDATE sessions SET cwd = ?1 WHERE id = 's'", [abs("/elsewhere")]).unwrap();
        assert_eq!(link(&conn, "s", "f-co/free", false), Ok(false));
    }

    #[test]
    fn same_dir_ignores_slash_style_and_a_trailing_slash() {
        assert!(same_dir(&abs("/work/a"), &abs("/work/a/")));
        assert!(!same_dir(&abs("/work/a"), &abs("/work/b")));
        if cfg!(windows) {
            assert!(same_dir("C:/Work/A", r"c:\work\a\"));
            // An existing folder resolves on both sides, whatever its spelling.
            let tmp = std::env::temp_dir().to_string_lossy().into_owned();
            assert!(same_dir(&tmp, &tmp.replace('\\', "/").to_uppercase()));
        }
    }

    #[test]
    fn settings_merge_keeps_the_file_and_velaterm_hooks() {
        let dir = std::env::temp_dir().join(format!("vlx-preset-settings-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("role.json");
        std::fs::write(
            &path,
            r#"{"enabledPlugins":{"x@m":true},"theme":"light","hooks":{"Stop":[{"hooks":[{"type":"command","command":"mine"}]}]}}"#,
        )
        .unwrap();
        let own = r#"{"theme":"dark","hooks":{"Stop":[{"hooks":[{"type":"http","url":"vlx"}]}],"UserPromptSubmit":[{"hooks":[]}]}}"#;
        let merged: Value = serde_json::from_str(&merge_settings(own, path.to_str().unwrap()).unwrap()).unwrap();
        assert_eq!(merged["enabledPlugins"], json!({"x@m": true}));
        assert_eq!(merged["theme"], "dark", "VelaTerm's own keys win");
        assert_eq!(merged["hooks"]["Stop"].as_array().unwrap().len(), 2, "the file's hook runs, then VelaTerm's");
        assert_eq!(merged["hooks"]["Stop"][1]["hooks"][0]["url"], "vlx");
        assert!(merged["hooks"]["UserPromptSubmit"].is_array());

        std::fs::write(&path, "[1]").unwrap();
        assert!(merge_settings(own, path.to_str().unwrap()).is_err());
        std::fs::write(&path, r#"{"hooks":[]}"#).unwrap();
        assert!(merge_settings(own, path.to_str().unwrap()).is_err());
        assert!(merge_settings(own, dir.join("missing.json").to_str().unwrap()).is_err(), "a launch fails closed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_log_appends_one_stamped_line_per_change() {
        let dir = std::env::temp_dir().join(format!("vlx-preset-log-{}", uuid::Uuid::new_v4()));
        let path = dir.join("presets-import.log");
        append_log(&path, &[], 0).unwrap();
        assert!(!path.exists(), "nothing to say, nothing written");
        append_log(&path, &[json!({"change": "added", "id": "a"})], 1_785_196_800).unwrap();
        append_log(&path, &[json!({"change": "removed", "id": "a"})], 1_785_196_801).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["ts"], "2026-07-28T00:00:00Z");
        assert_eq!(lines[1]["change"], "removed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
