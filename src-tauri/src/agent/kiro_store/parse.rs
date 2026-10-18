//! Parse only shapes observed in native Kiro stores. Unknown events fail explicitly.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use serde_json::Value;
use super::{Context, KiroMessage, Session, valid_id};

fn damaged(field: &str) -> String { format!("Kiro history is damaged: invalid {field}") }
fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value.as_str().ok_or_else(|| damaged(field))
}
fn id<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    let value = text(value, field)?;
    if !valid_id(value) { return Err(damaged(field)); }
    Ok(value)
}
pub(crate) fn directory(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() { return Err(damaged("project directory")); }
    Ok(path)
}

/// Native ISO timestamps include UTC or numeric offsets and up to nanosecond precision.
fn iso(value: &str) -> Result<time::OffsetDateTime, String> {
    let fail = || damaged("ISO timestamp");
    if !value.is_ascii() || value.len() < 20 { return Err(fail()); }
    let n = |s: &str| if s.bytes().all(|c| c.is_ascii_digit()) { s.parse::<u32>().map_err(|_| fail()) } else { Err(fail()) };
    if &value[4..5] != "-" || &value[7..8] != "-" || &value[10..11] != "T" || &value[13..14] != ":" || &value[16..17] != ":" { return Err(fail()); }
    let date = time::Date::from_calendar_date(n(&value[..4])? as i32, time::Month::try_from(n(&value[5..7])? as u8).map_err(|_| fail())?, n(&value[8..10])? as u8).map_err(|_| fail())?;
    let mut rest = &value[19..];
    let mut nanos = 0;
    if rest.starts_with('.') {
        let digits = rest[1..].bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || digits > 9 { return Err(fail()); }
        nanos = n(&rest[1..1+digits])? * 10u32.pow(9-digits as u32);
        rest = &rest[1+digits..];
    }
    let offset = if rest == "Z" { time::UtcOffset::UTC } else {
        if rest.len() != 6 || !matches!(&rest[..1], "+" | "-") || &rest[3..4] != ":" { return Err(fail()); }
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        time::UtcOffset::from_hms(sign * n(&rest[1..3])? as i8, sign * n(&rest[4..6])? as i8, 0).map_err(|_| fail())?
    };
    let clock = time::Time::from_hms_nano(n(&value[11..13])? as u8, n(&value[14..16])? as u8, n(&value[17..19])? as u8, nanos).map_err(|_| fail())?;
    Ok(time::PrimitiveDateTime::new(date, clock).assume_offset(offset))
}
fn timestamp(dt: time::OffsetDateTime) -> String {
    let dt = dt.to_offset(time::UtcOffset::UTC);
    let fraction = if dt.nanosecond() == 0 { String::new() } else { format!(".{:09}",dt.nanosecond()).trim_end_matches('0').to_string() };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{fraction}Z",dt.year(),u8::from(dt.month()),dt.day(),dt.hour(),dt.minute(),dt.second())
}
pub(crate) fn iso_optional(value: &Value) -> Result<Option<String>, String> {
    if value.is_null() { return Ok(None); }
    Ok(Some(timestamp(iso(text(value,"timestamp")?)?)))
}
pub(crate) fn seconds(value: &Value) -> Result<Option<String>, String> {
    if value.is_null() { return Ok(None); }
    let value = value.as_i64().ok_or_else(|| damaged("seconds timestamp"))?;
    let dt = time::OffsetDateTime::from_unix_timestamp(value).map_err(|_| damaged("seconds timestamp"))?;
    if !(0..=9999).contains(&dt.year()) { return Err(damaged("seconds timestamp")); }
    Ok(Some(timestamp(dt)))
}
pub(crate) fn millis(value: &Value) -> Result<i64, String> {
    let dt = iso(text(value,"session time")?)?;
    i64::try_from(dt.unix_timestamp_nanos()/1_000_000).map_err(|_| damaged("session time"))
}

pub(crate) fn context(model: &Value, percent: &Value) -> Result<Context, String> {
    let mut out = Context::default();
    if !model.is_null() {
        let m = text(&model["model_id"], "model ID")?;
        if m.is_empty() { return Err(damaged("model ID")); }
        out.model = Some(m.to_string());
        out.limit = match model.get("context_window_tokens") {
            Some(v) if !v.is_null() => Some(v.as_u64().filter(|v| *v > 0).ok_or_else(|| damaged("context capacity"))?),
            _ => None,
        };
    }
    if !percent.is_null() {
        out.percent = Some(percent.as_f64().filter(|p| p.is_finite() && (0.0..=100.0).contains(p)).ok_or_else(|| damaged("context percentage"))?);
    }
    Ok(out)
}

fn blocks(value: &Value) -> Result<String, String> {
    let blocks = value.as_array().ok_or_else(|| damaged("content blocks"))?;
    let mut out = String::new();
    for block in blocks {
        if block["kind"] != "text" { return Err("Kiro history contains non-text content that is not supported yet".into()); }
        out.push_str(text(&block["data"],"text content")?);
    }
    Ok(out)
}
fn title(value: Option<&str>, messages: &Result<Vec<KiroMessage>, String>, fallback: &str) -> String {
    let first = messages.as_ref().ok().and_then(|m| m.iter().find(|m| m.role == "user")).map(|m| m.text.as_str());
    value.filter(|s| !s.trim().is_empty()).or(first).unwrap_or(fallback).split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect()
}

pub(super) fn file(expected_id: &str, meta: &Value, log: &[u8]) -> Result<Session, String> {
    let session_id = id(&meta["session_id"], "session ID")?;
    if session_id != expected_id { return Err("Kiro history has conflicting filename and session identities".into()); }
    let state = &meta["session_state"];
    if state["version"] != "v1" { return Err("Kiro history metadata version is not supported".into()); }
    if id(&state["rts_model_state"]["conversation_id"], "conversation ID")? != session_id {
        return Err("Kiro history has conflicting session and conversation identities".into());
    }
    let cwd = directory(text(&meta["cwd"],"project directory")?)?;
    let created = millis(&meta["created_at"])?;
    let updated_at = millis(&meta["updated_at"])?;
    if updated_at < created { return Err(damaged("session time order")); }
    let context = context(&state["rts_model_state"]["model_info"], &state["rts_model_state"]["context_usage_percentage"])?;
    let messages = file_messages(log, &state["conversation_metadata"]);
    let title = title(meta["title"].as_str(), &messages, session_id);
    Ok(Session { id: session_id.into(), cwd, title, updated_at, context, messages, revision: String::new() })
}

fn file_messages(log: &[u8], metadata: &Value) -> Result<Vec<KiroMessage>, String> {
    let log = std::str::from_utf8(log).map_err(|_| damaged("event encoding"))?;
    let mut messages = Vec::new();
    let mut events = HashMap::new();
    for line in log.lines().filter(|l| !l.trim().is_empty()) {
        let event: Value = serde_json::from_str(line).map_err(|_| damaged("event JSON"))?;
        if event["version"] != "v1" { return Err("Kiro history event version is not supported".into()); }
        let role = match event["kind"].as_str() {
            Some("Prompt") => "user",
            Some("AssistantMessage") => "assistant",
            _ => return Err("Kiro history contains an event type that is not supported yet".into()),
        };
        let data = &event["data"];
        let native_id = id(&data["message_id"],"message ID")?.to_string();
        let message = KiroMessage { role, text: blocks(&data["content"])?, timestamp: seconds(&data["meta"]["timestamp"])?, native_id: Some(native_id.clone()) };
        if events.insert(native_id, event).is_some() { return Err(damaged("duplicate message ID")); }
        messages.push(message);
    }
    let turns = metadata["user_turn_metadatas"].as_array().ok_or_else(|| damaged("turn metadata"))?;
    let mut assigned = HashSet::new();
    for turn in turns {
        let ids = turn["message_ids"].as_array().ok_or_else(|| damaged("turn message IDs"))?;
        let mut assistant = None;
        for value in ids {
            let mid = id(value,"turn message ID")?;
            if !assigned.insert(mid.to_string()) { return Err(damaged("message assigned to multiple turns")); }
            let event = events.get(mid).ok_or_else(|| damaged("turn referencing a missing message"))?;
            if event["kind"] == "AssistantMessage" { assistant = Some(event); }
        }
        if !turn["result"].is_null() {
            let result = turn["result"].get("Ok").ok_or("Kiro history turn result is not supported yet")?;
            let assistant = assistant.ok_or_else(|| damaged("turn assistant reference"))?;
            // result.Ok.id belongs to a different namespace. Join only through message_ids.
            if result["content"] != assistant["data"]["content"] { return Err(damaged("turn result content")); }
            seconds(&result["meta"]["timestamp"])?;
        }
        iso_optional(&turn["end_timestamp"])?;
    }
    Ok(messages)
}

pub(super) fn sqlite(expected_id: &str, cwd: &str, created: i64, updated_at: i64, value: &Value) -> Result<Session, String> {
    let session_id = id(&value["conversation_id"],"conversation ID")?;
    if session_id != expected_id { return Err("Kiro history has conflicting database and conversation identities".into()); }
    if updated_at < created { return Err(damaged("database time order")); }
    let project = directory(cwd)?;
    let history = value["history"].as_array().ok_or_else(|| damaged("SQLite history"))?;
    for row in history {
        let recorded = &row["user"]["env_context"]["env_state"]["current_working_directory"];
        if !recorded.is_null() && !super::same_dir(&project, &directory(text(recorded,"message directory")?)?) {
            return Err("Kiro history has conflicting database and message project identities".into());
        }
    }
    let latest = history.last().map(|h| &h["request_metadata"]).unwrap_or(&Value::Null);
    let context = context(&value["model_info"], &latest["context_usage_percentage"])?;
    let messages = (|| {
        if !value["next_message"].is_null() || !value["latest_summary"].is_null() {
            return Err("Kiro history has pending or compacted content that is not supported yet".into());
        }
        let mut out = Vec::new();
        let mut message_ids = HashSet::new();
        for row in history {
            let tool_ids = &row["request_metadata"]["tool_use_ids_and_names"];
            if !tool_ids.is_null() && tool_ids.as_array().is_none_or(|ids| !ids.is_empty()) {
                return Err("Kiro history contains tool activity that is not supported yet".into());
            }
            let user = &row["user"];
            let prompt = user["content"].get("Prompt").ok_or("Kiro history contains a non-text user event that is not supported yet")?;
            let assistant = row["assistant"].get("Response").ok_or("Kiro history contains a non-text assistant event that is not supported yet")?;
            if !user["images"].is_null() && user["images"].as_array().is_none_or(|a| !a.is_empty()) {
                return Err("Kiro history contains images that are not supported yet".into());
            }
            let native_id = id(&assistant["message_id"],"assistant message ID")?.to_string();
            if !message_ids.insert(native_id.clone()) { return Err(damaged("duplicate assistant message ID")); }
            out.push(KiroMessage { role: "user", text: text(&prompt["prompt"],"user prompt")?.into(), timestamp: iso_optional(&user["timestamp"])?, native_id: None });
            out.push(KiroMessage { role: "assistant", text: text(&assistant["content"],"assistant text")?.into(), timestamp: None, native_id: Some(native_id) });
        }
        Ok(out)
    })();
    let title = title(None, &messages, session_id);
    Ok(Session { id: session_id.into(), cwd: project, title, updated_at, context, messages, revision: String::new() })
}
