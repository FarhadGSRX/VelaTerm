use crate::{agent::chat::history, host::AppCtx};

#[derive(Clone, Default, serde::Serialize)]
pub struct Preview { pub title: String, pub body: String }

/// A notification is a short, plain-text excerpt, never reasoning or a tool result.
pub(crate) fn plain(text: &str, limit: usize) -> String {
    let mut fenced = false;
    let mut prose = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") { fenced = !fenced; continue; }
        if fenced { continue; }
        if !prose.is_empty() { prose.push(' '); }
        prose.extend(line.trim_start_matches(['#', '>', '-', '*', ' ']).chars().take(16384usize.saturating_sub(prose.chars().count())));
        // Bound intermediate work even when a single assistant reply is unusually large.
        if prose.len() > 16384 { break; }
    }
    let mut text = prose.replace("**", "").replace("__", "").replace('`', "");
    // Keep link labels without copying Markdown destinations into the notification.
    while let Some(mid) = text.find("](") {
        let Some(start) = text[..mid].rfind('[') else { break; };
        let Some(end) = text[mid + 2..].find(')') else { break; };
        text.replace_range(mid..mid + end + 3, "");
        text.remove(start);
    }
    let normalized: String = text.chars().filter(|c| !c.is_control() || c.is_whitespace()).collect();
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= limit { return normalized; }
    let mut result: String = normalized.chars().take(limit.saturating_sub(1)).collect();
    result.push('…'); result
}

/// Stop at the current user turn; a previous turn's answer must never become a new result.
pub(crate) fn latest_reply<'a>(rows: impl Iterator<Item = (&'a str, &'a str, Option<i64>)>, at: i64) -> String {
    for (kind, text, timestamp) in rows {
        if timestamp.is_some_and(|time| time > at) { continue; }
        if kind == "user" { break; }
        if kind == "assistant" {
            let text = plain(text, 240);
            if !text.is_empty() { return text; }
        }
    }
    String::new()
}

pub(super) fn for_event(app: &AppCtx, id: &str, state: &str, at: i64) -> Result<Preview, String> {
    let session = {
        let conn = app.db().conn.lock().map_err(|_| "Database unavailable")?;
        crate::db::repo::get_session(&conn, id)?.ok_or("Session unavailable")?
    };
    let title = plain(&session.name, 80);
    if !matches!(state, "waiting" | "asking") { return Ok(Preview { title, body: String::new() }); }
    let body = if session.engine == "chat" {
        app.chat().notification_excerpt(id, state == "asking", at)
    } else { None };
    let body = body.unwrap_or_else(|| {
        let Some(agent) = session.agent_session_id.as_deref() else { return String::new(); };
        let Ok(rows) = history::read(session.kind, agent) else { return String::new(); };
        latest_reply(rows.iter().rev().map(|row| (row.kind, row.text.as_deref().unwrap_or(""),
            history::parsed_at(row.timestamp.as_deref()))), at)
    });
    Ok(Preview { title, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excerpts_keep_prose_and_unicode_without_markdown_or_tool_output() {
        assert_eq!(plain("## 完成\n\n已修复 **通知跳转**，见 [会话](https://example.invalid/)。\n```sh\nsecret command\n```", 240), "完成 已修复 通知跳转，见 会话。");
        let value = plain(&"🙂".repeat(300), 240);
        assert_eq!(value.chars().count(), 240); assert!(value.ends_with('…'));
        assert_eq!(plain("one\u{0}\n two\tthree", 240), "one two three");
    }
    #[test]
    fn the_latest_reply_is_scoped_to_the_event_turn() {
        let rows = [("user", "first", Some(1)), ("assistant", "old reply", Some(2)),
            ("user", "second", Some(3)), ("thinking", "private reasoning", Some(4)),
            ("tool", "raw output", Some(5)), ("assistant", "Current result", Some(6)),
            ("user", "third", Some(7)), ("assistant", "Later result", Some(8))];
        assert_eq!(latest_reply(rows.iter().copied().rev(), 6), "Current result");
        assert_eq!(latest_reply(rows.iter().copied().rev(), 5), "");
        assert_eq!(latest_reply(rows.iter().copied().rev(), 8), "Later result");
    }
}
