//! The model catalogue Pi or OMP reports, used by the composer's model chip.
//!
//! A running process already answered `get_available_models`, and the engine caches that answer. With no
//! process, a short-lived one is started only to ask the same question; it never opens a conversation and
//! is killed as soon as it answers.

use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;

use crate::agent::chat::pi_protocol::{self as wire, PiIncoming, PiVariant};
use crate::models::SessionKind;

/// How long the throwaway process may take to answer.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(20);

/// Read the catalogue. `extra_args` are the session's own launch arguments, so a model list matches the
/// environment the conversation would actually start in.
pub fn list(
    kind: SessionKind,
    bin: &str,
    cwd: Option<&str>,
    extra_args: &[String],
) -> Result<Vec<Value>, String> {
    let variant = PiVariant::of(kind).ok_or("This session is not a Pi or OMP conversation")?;
    Ok(wire::to_chat_models(variant, &list_raw(kind, bin, cwd, extra_args)?))
}

/// Launch selectors retain the provider so identical native model IDs cannot select another provider.
pub fn list_for_launch(kind: SessionKind, bin: &str, cwd: Option<&str>, extra_args: &[String]) -> Result<Vec<Value>, String> {
    let variant = PiVariant::of(kind).ok_or("This session is not a Pi or OMP conversation")?;
    let mut models = list_raw(kind, bin, cwd, extra_args)?;
    for model in &mut models {
        let Some(id) = model["id"].as_str().filter(|id| !id.is_empty()) else { continue };
        let provider = model["provider"].as_str().unwrap_or_default();
        let selector = model["selector"].as_str().filter(|s| !s.is_empty())
            .map(str::to_owned).unwrap_or_else(|| if provider.is_empty() { id.to_owned() } else { format!("{provider}/{id}") });
        model["id"] = Value::String(selector);
    }
    Ok(wire::to_chat_models(variant, &models))
}

fn list_raw(kind: SessionKind, bin: &str, cwd: Option<&str>, extra_args: &[String]) -> Result<Vec<Value>, String> {
    let variant = PiVariant::of(kind).ok_or("This session is not a Pi or OMP conversation")?;
    let mut cmd = crate::host::command(bin);
    crate::agent::executable::prepare_command(&mut cmd, bin);
    cmd.args(wire::launch_args(variant, None, None, false));
    cmd.args(extra_args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    for key in crate::pty::manager::AGENT_HARNESS_MARKERS {
        cmd.env_remove(key);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start the agent {bin:?} to read its model catalogue: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("The agent has no input stream")?;
    let stdout = child.stdout.take().ok_or("The agent has no output stream")?;

    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let send = |stdin: &mut std::process::ChildStdin, value: &Value| {
        let line = format!("{value}\n");
        let _ = stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush());
    };
    if variant == PiVariant::Omp {
        send(&mut stdin, &wire::negotiate("vlx-catalog-negotiate"));
    }
    send(&mut stdin, &wire::get_available_models("vlx-catalog-models"));

    let deadline = std::time::Instant::now() + LOOKUP_TIMEOUT;
    let mut models = None;
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            break;
        }
        match rx.recv_timeout(deadline - now) {
            Ok(line) => {
                if let Some(PiIncoming::Response { command, success, data, .. }) =
                    wire::parse_line(&line)
                {
                    if command == "get_available_models" {
                        if success {
                            models = Some(
                                data.get("models")
                                    .and_then(Value::as_array)
                                    .cloned()
                                    .unwrap_or_default(),
                            );
                        }
                        break;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    let models = models.ok_or("The agent did not report its model catalogue")?;
    Ok(models)
}

/// The context window a model has according to the catalogue the agent itself cached on disk.
///
/// Neither Pi nor OMP records the window in its session file, so the Info panel needs this lookup to turn
/// the last prompt's token count into a share of the window. Pi writes the catalogue as JSON under its state
/// directory; OMP keeps the same entries in a SQLite table there. Nothing here writes, and a missing,
/// locked or unreadable store simply leaves the limit unknown.
pub fn stored_context_window(kind: SessionKind, provider: Option<&str>, model: &str) -> Option<u64> {
    match PiVariant::of(kind)? {
        PiVariant::Pi => pi_store_window(provider, model),
        PiVariant::Omp => omp_store_window(provider, model),
    }
}

fn pi_store_window(provider: Option<&str>, model: &str) -> Option<u64> {
    let path = crate::agent::resume::pi_agent_home(SessionKind::Pi)?.join("models-store.json");
    let store: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    window_from_store(&store, provider, model)
}

/// Looks one model up in Pi's catalogue shape: provider keys, each holding a `models` array. The named
/// provider wins when it lists the model; otherwise every provider is scanned, because a session may
/// record the model under a provider the catalogue spells differently.
fn window_from_store(store: &Value, provider: Option<&str>, model: &str) -> Option<u64> {
    fn listed(provider: &Value, model: &str) -> Option<u64> {
        provider
            .get("models")?
            .as_array()?
            .iter()
            .find(|entry| entry.get("id").and_then(Value::as_str) == Some(model))
            .and_then(|entry| entry.get("contextWindow").and_then(Value::as_u64))
            .filter(|window| *window > 0)
    }
    if let Some(window) = provider
        .and_then(|name| store.get(name))
        .and_then(|entry| listed(entry, model))
    {
        return Some(window);
    }
    store
        .as_object()?
        .values()
        .find_map(|entry| listed(entry, model))
}

/// Reads OMP's catalogue cache. `provider_id` carries a version suffix for some providers, so the row is
/// matched on either the exact identifier or its prefix before falling back to any provider that lists it.
fn omp_store_window(provider: Option<&str>, model: &str) -> Option<u64> {
    let path = crate::agent::resume::pi_agent_home(SessionKind::Omp)?.join("models.db");
    if !path.is_file() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    // OMP writes to this database while it runs; wait briefly rather than fail on a busy lock.
    let _ = conn.busy_timeout(std::time::Duration::from_millis(500));
    omp_window_from_conn(&conn, provider, model)
}

/// The catalogue query itself, so a test can run it against a database of its own.
fn omp_window_from_conn(
    conn: &rusqlite::Connection,
    provider: Option<&str>,
    model: &str,
) -> Option<u64> {
    use rusqlite::OptionalExtension;
    let positive = |row: Option<u64>| row.filter(|window| *window > 0);
    if let Some(provider) = provider {
        let query = "SELECT json_extract(value, '$.contextWindow') FROM model_cache, json_each(model_cache.models) \
                     WHERE (model_cache.provider_id = ?2 OR model_cache.provider_id LIKE ?2 || ':%') \
                     AND json_extract(value, '$.id') = ?1 LIMIT 1";
        let found = conn
            .query_row(query, rusqlite::params![model, provider], |row| row.get::<_, Option<u64>>(0))
            .optional();
        if let Ok(found) = found {
            if let Some(window) = positive(found.flatten()) {
                return Some(window);
            }
        }
    }
    conn.query_row(
        "SELECT json_extract(value, '$.contextWindow') FROM model_cache, json_each(model_cache.models) \
         WHERE json_extract(value, '$.id') = ?1 LIMIT 1",
        rusqlite::params![model],
        |row| row.get::<_, Option<u64>>(0),
    )
    .optional()
    .ok()
    .flatten()
    .flatten()
    .filter(|window| *window > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_stored_catalogue_supplies_the_context_window() {
        let store = json!({
            "deepseek": {"models": [{"id": "deepseek-flash", "contextWindow": 1_000_000}]},
            "anthropic": {"models": [{"id": "claude-sonnet-4-5", "contextWindow": 200_000}]}
        });
        assert_eq!(
            window_from_store(&store, Some("deepseek"), "deepseek-flash"),
            Some(1_000_000)
        );
        // An unknown provider name still resolves the model, because the session's spelling may differ.
        assert_eq!(
            window_from_store(&store, Some("openrouter"), "claude-sonnet-4-5"),
            Some(200_000)
        );
        assert_eq!(window_from_store(&store, None, "missing"), None);
    }

    #[test]
    fn omp_catalogue_rows_match_by_provider_prefix_or_anywhere() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE model_cache (provider_id TEXT PRIMARY KEY, models TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO model_cache (provider_id, models) VALUES (?1, ?2)",
            rusqlite::params![
                "deepseek",
                json!([{"id": "deepseek-v4-pro", "contextWindow": 128_000}]).to_string()
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO model_cache (provider_id, models) VALUES (?1, ?2)",
            rusqlite::params![
                "github-copilot:models-v2:abc",
                json!([{"id": "claude-fable-5", "contextWindow": 1_000_000}]).to_string()
            ],
        )
        .unwrap();
        assert_eq!(
            omp_window_from_conn(&conn, Some("deepseek"), "deepseek-v4-pro"),
            Some(128_000)
        );
        assert_eq!(
            omp_window_from_conn(&conn, Some("github-copilot"), "claude-fable-5"),
            Some(1_000_000)
        );
        // An unnamed provider is no reason to lose the window.
        assert_eq!(
            omp_window_from_conn(&conn, None, "claude-fable-5"),
            Some(1_000_000)
        );
        assert_eq!(omp_window_from_conn(&conn, None, "unknown"), None);
    }
}
