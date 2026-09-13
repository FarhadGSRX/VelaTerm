//! Transactional memory documents, independent full-text search and immutable revisions.
use super::{now, Edit, Entry};
use crate::host::AppCtx;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};

pub fn key(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    let array = |col| -> rusqlite::Result<Vec<String>> {
        let raw: String = r.get(col)?;
        serde_json::from_str(&raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e))
        })
    };
    Ok(Entry {
        id: r.get(0)?,
        session_id: r.get(10)?,
        title: r.get(1)?,
        summary: r.get(2)?,
        content: r.get(3)?,
        tags: array(4)?,
        related: array(5)?,
        sources: array(6)?,
        version: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}
const COLS: &str =
    "id,title,summary,content,tags,related,sources,version,created_at,updated_at,session_id";

pub fn get(conn: &Connection, id: &str) -> Result<Option<Entry>, String> {
    conn.query_row(
        &format!("SELECT {COLS} FROM memory_entries WHERE id=?1"),
        [id],
        row,
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn catalog(conn: &Connection) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare("SELECT id,title FROM memory_entries ORDER BY title_key")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?}))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn all(conn: &Connection) -> Result<Vec<Entry>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLS} FROM memory_entries ORDER BY title_key"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row).map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn validate(entry: &mut Entry) -> Result<(), String> {
    entry.title = entry.title.trim().to_string();
    entry.summary = entry.summary.trim().to_string();
    entry.content = entry.content.trim().to_string();
    if entry.title.is_empty()
        || entry.title.chars().count() > 200
        || entry.title.chars().any(char::is_control)
        || entry.summary.chars().count() > 2000
        || entry.content.is_empty()
        || entry.content.chars().count() > 200_000
    {
        return Err("memory_invalid:entry_size".into());
    }
    entry.tags = entry
        .tags
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    entry.tags.sort();
    entry.tags.dedup();
    entry.related.sort();
    entry.related.dedup();
    entry.related.retain(|id| id != &entry.id);
    entry.sources.sort();
    entry.sources.dedup();
    if entry.tags.len() > 32
        || entry
            .tags
            .iter()
            .any(|s| s.chars().count() > 80 || s.chars().any(char::is_control))
        || entry.related.len() > 100
    {
        return Err("memory_invalid:metadata".into());
    }
    Ok(())
}

/// Caller owns a transaction so a failed link, conflict or revision write rolls everything back.
pub fn put(conn: &Connection, entry: Entry, expected: i64, author: &str) -> Result<Entry, String> {
    put_entry(conn, entry, expected, author, true)
}

/// Shared write path. `keep_session` preserves the stored session grouping for edits and generated
/// entries, whose callers do not carry it; moving an entry passes false to write its new grouping.
fn put_entry(
    conn: &Connection,
    mut entry: Entry,
    expected: i64,
    author: &str,
    keep_session: bool,
) -> Result<Entry, String> {
    validate(&mut entry)?;
    let existing = get(conn, &entry.id)?;
    if existing.as_ref().map_or(0, |e| e.version) != expected {
        return Err("memory_conflict".into());
    }
    if keep_session {
        if let Some(prior) = &existing {
            entry.session_id = prior.session_id.clone();
        }
    }
    let duplicate: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_entries WHERE title_key=?1 AND id<>?2)",
            params![
                json!([entry.session_id, key(&entry.title)]).to_string(),
                entry.id
            ],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if duplicate {
        return Err("memory_duplicate_title".into());
    }
    for id in &entry.sources {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM memory_sources WHERE id=?1)",
                [id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err("memory_invalid:source".into());
        }
    }
    entry.version = expected + 1;
    entry.created_at = existing.as_ref().map_or_else(now, |e| e.created_at);
    entry.updated_at = now();
    let serialized = |v: &Vec<String>| serde_json::to_string(v).map_err(|e| e.to_string());
    conn.execute("INSERT INTO memory_entries(id,title,title_key,summary,content,tags,related,sources,version,created_at,updated_at,session_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(id) DO UPDATE SET title=excluded.title,title_key=excluded.title_key,summary=excluded.summary,content=excluded.content,tags=excluded.tags,related=excluded.related,sources=excluded.sources,version=excluded.version,updated_at=excluded.updated_at,session_id=excluded.session_id",
        params![entry.id, entry.title, json!([entry.session_id, key(&entry.title)]).to_string(), entry.summary, entry.content, serialized(&entry.tags)?, serialized(&entry.related)?, serialized(&entry.sources)?, entry.version, entry.created_at, entry.updated_at, entry.session_id]).map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO memory_versions(entry_id,version,snapshot,author,created_at) VALUES(?1,?2,?3,?4,?5)", params![entry.id, entry.version, serde_json::to_string(&entry).map_err(|e| e.to_string())?, author, entry.updated_at]).map_err(|e| e.to_string())?;
    Ok(entry)
}

pub fn validate_links(conn: &Connection, entries: &[Entry]) -> Result<(), String> {
    for entry in entries {
        for id in &entry.related {
            if get(conn, id)?.is_none() {
                return Err("memory_invalid:related".into());
            }
        }
    }
    Ok(())
}

pub fn edit(conn: &Connection, input: Edit, author: &str) -> Result<Entry, String> {
    let prior = input
        .id
        .as_deref()
        .map(|id| get(conn, id))
        .transpose()?
        .flatten();
    if input.id.is_some() && prior.is_none() {
        return Err("memory_conflict".into());
    }
    let entry = Entry {
        id: input.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        session_id: prior
            .as_ref()
            .map_or_else(String::new, |e| e.session_id.clone()),
        title: input.title,
        summary: input.summary,
        content: input.content,
        tags: input.tags,
        related: input.related,
        sources: prior.map_or_else(Vec::new, |e| e.sources),
        version: 0,
        created_at: 0,
        updated_at: 0,
    };
    validate_links(conn, std::slice::from_ref(&entry))?;
    put(conn, entry, input.version, author)
}

/// Rename an entry from the knowledge tree. The stored title is the only field changed; a new
/// revision records the edit exactly like a manual save.
pub fn rename_entry(
    conn: &Connection,
    id: &str,
    expected: i64,
    title: &str,
    author: &str,
) -> Result<Entry, String> {
    let mut entry = get(conn, id)?.ok_or_else(|| "memory_not_found".to_string())?;
    if entry.version != expected {
        return Err("memory_conflict".into());
    }
    entry.title = title.to_string();
    entry.version = 0;
    put(conn, entry, expected, author)
}

/// Reassign an entry to another session group. Passing an empty session groups it under the manual
/// collection. Revisions keep the move, so history stays complete.
pub fn move_entry(
    conn: &Connection,
    id: &str,
    expected: i64,
    session_id: &str,
    author: &str,
) -> Result<Entry, String> {
    let mut entry = get(conn, id)?.ok_or_else(|| "memory_not_found".to_string())?;
    if entry.version != expected {
        return Err("memory_conflict".into());
    }
    entry.session_id = session_id.to_string();
    entry.version = 0;
    put_entry(conn, entry, expected, author, false)
}

/// Session ids that make up a knowledge-tree group. The tree's synthetic ids map back to the stored
/// grouping: `__manual__` is the empty session id, `__unknown__` collects sessions whose project
/// snapshot is gone.
pub fn group_sessions(
    conn: &Connection,
    kind: &str,
    id: &str,
) -> Result<Vec<String>, String> {
    if kind == "session" {
        return Ok(vec![if id == "__manual__" { String::new() } else { id.to_string() }]);
    }
    if kind != "project" {
        return Err("memory_invalid:group".into());
    }
    if id == "__manual__" {
        return Ok(vec![String::new()]);
    }
    let mut stmt = conn
        .prepare(
            "SELECT id FROM memory_sessions WHERE (?1='__unknown__' AND project_id='') OR (?1<>'__unknown__' AND project_id=?1)",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Remove entries and revise every incoming link that pointed at them. Callers own the transaction.
pub fn remove(conn: &Connection, ids: &[String]) -> Result<usize, String> {
    if ids.is_empty() {
        return Ok(0);
    }
    let targets: HashSet<&str> = ids.iter().map(String::as_str).collect();
    for mut entry in all(conn)? {
        if targets.contains(entry.id.as_str()) {
            continue;
        }
        let before = entry.related.len();
        entry.related.retain(|target| !targets.contains(target.as_str()));
        if entry.related.len() != before {
            let version = entry.version;
            put(conn, entry, version, "unlink")?;
        }
    }
    let mut removed = 0;
    for id in ids {
        removed += conn
            .execute("DELETE FROM memory_entries WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
    }
    Ok(removed)
}

/// Delete every entry under one knowledge-tree group and return how many were removed.
pub fn delete_group(conn: &Connection, kind: &str, id: &str) -> Result<usize, String> {
    let sessions = group_sessions(conn, kind, id)?;
    let mut ids: Vec<String> = Vec::new();
    for session in sessions {
        let mut stmt = conn
            .prepare("SELECT id FROM memory_entries WHERE session_id=?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&session], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        ids.extend(rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?);
    }
    remove(conn, &ids)
}

/// Direct associations for search hits: outgoing links plus incoming references, resolved to live
/// entries only. A target deleted without unlinking is dropped rather than listed without a title.
fn related_entries(
    conn: &Connection,
    entries: &[Entry],
) -> Result<HashMap<String, Vec<Value>>, String> {
    let targets: Vec<&str> = entries
        .iter()
        .flat_map(|entry| entry.related.iter().map(String::as_str))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut titles: HashMap<String, String> = HashMap::new();
    if !targets.is_empty() {
        let json = serde_json::to_string(&targets).map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id,title FROM memory_entries WHERE id IN (SELECT value FROM json_each(?1))")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&json], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, title) = row.map_err(|e| e.to_string())?;
            titles.insert(id, title);
        }
    }
    let mut incoming: HashMap<String, Vec<(String, String)>> = HashMap::new();
    {
        let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
        let json = serde_json::to_string(&ids).map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT e.id,e.title,j.value FROM memory_entries e, json_each(e.related) j WHERE j.value IN (SELECT value FROM json_each(?1))")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&json], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, title, target) = row.map_err(|e| e.to_string())?;
            incoming.entry(target).or_default().push((id, title));
        }
    }
    let mut result = HashMap::new();
    for entry in entries {
        let mut seen = BTreeSet::new();
        let mut list = Vec::new();
        for id in &entry.related {
            if id == &entry.id || !seen.insert(id.clone()) {
                continue;
            }
            if let Some(title) = titles.get(id) {
                list.push(json!({"id":id,"title":title}));
            }
        }
        for (id, title) in incoming.get(&entry.id).into_iter().flatten() {
            if id == &entry.id || !seen.insert(id.clone()) {
                continue;
            }
            list.push(json!({"id":id,"title":title}));
        }
        result.insert(entry.id.clone(), list);
    }
    Ok(result)
}

/// SQL score for relevance ordering: a full title match beats a full summary match, which beats tags;
/// term frequency in the body breaks ties within a tier.
///
/// The terms are appended to `values` as numbered parameters. `None` means the query has no words and
/// there is nothing to rank by.
fn relevance_score(values: &mut Vec<rusqlite::types::Value>, query: &str) -> Option<String> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return None;
    }
    let mut in_title = Vec::new();
    let mut in_summary = Vec::new();
    let mut in_tags = Vec::new();
    let mut occurrences = Vec::new();
    for word in words {
        values.push(word.clone().into());
        let n = values.len();
        in_title.push(format!("instr(lower(title),?{n})>0"));
        in_summary.push(format!("instr(lower(summary),?{n})>0"));
        in_tags.push(format!("instr(lower(tags),?{n})>0"));
        // Character-length difference divided by the term's own length is the occurrence count.
        occurrences.push(format!(
            "(length(lower(content))-length(replace(lower(content),?{n},'')))/length(?{n})"
        ));
    }
    Some(format!(
        "(CASE WHEN {} THEN 100 ELSE 0 END+CASE WHEN {} THEN 20 ELSE 0 END+CASE WHEN {} THEN 10 ELSE 0 END+min({},20)*5)",
        in_title.join(" AND "),
        in_summary.join(" AND "),
        in_tags.join(" AND "),
        occurrences.join("+")
    ))
}

pub fn list(app: &AppCtx, args: &Value) -> Result<Value, String> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if query.chars().count() > 500 {
        return Err("memory_invalid:query".into());
    }
    let tag = args.get("tag").and_then(Value::as_str).unwrap_or("");
    let page = args
        .get("page")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(100_000);
    let sort_title = args.get("sort").and_then(Value::as_str) == Some("title");
    let conn = app.db().conn.lock().map_err(|e| e.to_string())?;
    let mut conditions = vec![
        "(?1='' OR EXISTS(SELECT 1 FROM json_each(memory_entries.tags) WHERE value=?1))"
            .to_string(),
    ];
    let mut values: Vec<rusqlite::types::Value> = vec![tag.to_string().into()];
    for word in query.split_whitespace() {
        let n = values.len() + 1;
        if word.chars().count() >= 3 {
            conditions.push(format!(
                "rowid IN (SELECT rowid FROM memory_fts WHERE memory_fts MATCH ?{n})"
            ));
            values.push(format!("\"{}\"", word.replace('"', "\"\"")).into());
        } else {
            conditions.push(format!(
                "instr(lower(title||' '||summary||' '||content||' '||tags),lower(?{n}))>0"
            ));
            values.push(word.to_string().into());
        }
    }
    let tree = super::hierarchy::tree(&conn, &conditions.join(" AND "), &values)?;
    let selected = args
        .get("selectedId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|id| get(&conn, id))
        .transpose()?
        .flatten();
    let session_filter = selected
        .as_ref()
        .map(|e| {
            if e.session_id.is_empty() {
                "__manual__"
            } else {
                &e.session_id
            }
        })
        .or_else(|| args.get("sessionId").and_then(Value::as_str));
    if let Some(session) = session_filter {
        let n = values.len() + 1;
        conditions.push(format!("session_id=?{n}"));
        values.push(
            if session == "__manual__" { "" } else { session }
                .to_string()
                .into(),
        );
    } else if let Some(project) = args.get("projectId").and_then(Value::as_str) {
        let n = values.len() + 1;
        if project == "__manual__" {
            conditions.push("session_id=''".into());
        } else {
            conditions.push(format!(
                "session_id IN (SELECT id FROM memory_sessions WHERE project_id=?{n})"
            ));
            values.push(
                if project == "__unknown__" {
                    ""
                } else {
                    project
                }
                .to_string()
                .into(),
            );
        }
    }
    let filter = conditions.join(" AND ");
    let total: i64 = conn
        .query_row(
            &format!("SELECT count(*) FROM memory_entries WHERE {filter}"),
            rusqlite::params_from_iter(values.iter()),
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    // Searches rank by relevance: a title hit outranks a summary hit, which outranks tags and raw term
    // frequency. The heuristic works for every language, while BM25 over the trigram index cannot rank
    // the two-character Chinese words that are common in queries here.
    let mut select_values = values;
    let order = if sort_title {
        "title COLLATE NOCASE ASC,id".to_string()
    } else if let Some(score) = relevance_score(&mut select_values, query) {
        format!("{score} DESC,updated_at DESC,id")
    } else {
        "updated_at DESC,id".to_string()
    };
    let sql = format!(
        "SELECT {COLS} FROM memory_entries WHERE {filter} ORDER BY {order} LIMIT 40 OFFSET {}",
        page * 40
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let entries = stmt
        .query_map(rusqlite::params_from_iter(select_values.iter()), row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    // Search results carry their directly related entries so a reader can follow the association without
    // opening each hit. Browsing stays lean and returns the previous shape.
    let expanding = !query.is_empty();
    let related = if expanding {
        related_entries(&conn, &entries)?
    } else {
        HashMap::new()
    };
    // The list only returns snippets. Complete documents are fetched individually for reading/editing.
    let entries: Vec<Value> = entries.into_iter().map(|e| {
        let mut value = json!({"id":e.id,"title":e.title,"summary":e.summary,"tags":e.tags,"version":e.version,"updatedAt":e.updated_at,"sourceCount":e.sources.len(),"sessionId":e.session_id});
        if expanding {
            value["related"] = json!(related.get(&e.id).cloned().unwrap_or_default());
        }
        value
    }).collect();
    let mut stmt = conn.prepare("SELECT DISTINCT value FROM memory_entries,json_each(memory_entries.tags) ORDER BY value").map_err(|e| e.to_string())?;
    let tags = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(
        json!({"selectedSessionId":session_filter,"projects":tree,"entries":entries,"total":total,"pageSize":40,"tags":tags}),
    )
}

pub fn detail(app: &AppCtx, id: &str, version: Option<i64>) -> Result<Value, String> {
    let conn = app.db().conn.lock().map_err(|e| e.to_string())?;
    let entry = get(&conn, id)?.ok_or("memory_not_found")?;
    let mut stmt = conn
        .prepare("SELECT id,title FROM memory_entries ORDER BY title_key")
        .map_err(|e| e.to_string())?;
    let catalog = stmt
        .query_map([], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?}))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT id,title FROM memory_entries WHERE EXISTS(SELECT 1 FROM json_each(related) WHERE value=?1)").map_err(|e| e.to_string())?;
    let backlinks = stmt
        .query_map([id], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?}))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT version,author,created_at FROM memory_versions WHERE entry_id=?1 ORDER BY version DESC").map_err(|e| e.to_string())?;
    let versions = stmt.query_map([id], |r| Ok(json!({"version":r.get::<_,i64>(0)?,"author":r.get::<_,String>(1)?,"createdAt":r.get::<_,i64>(2)?}))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    let revision: Option<Entry> = match version {
        Some(version) => {
            let raw: String = conn
                .query_row(
                    "SELECT snapshot FROM memory_versions WHERE entry_id=?1 AND version=?2",
                    params![id, version],
                    |r| r.get(0),
                )
                .map_err(|_| "memory_not_found")?;
            Some(serde_json::from_str(&raw).map_err(|e| format!("{e}"))?)
        }
        None => None,
    };
    let source_ids: HashSet<&str> = revision
        .as_ref()
        .unwrap_or(&entry)
        .sources
        .iter()
        .map(String::as_str)
        .collect();
    let mut stmt = conn.prepare("SELECT id,session_name,kind,created_at,digest,session_id FROM memory_sources ORDER BY created_at").map_err(|e| e.to_string())?;
    let sources = stmt.query_map([], |r| Ok(json!({"id":r.get::<_,String>(0)?,"sessionName":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"createdAt":r.get::<_,i64>(3)?,"digest":r.get::<_,String>(4)?,"sessionId":r.get::<_,String>(5)?}))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?.into_iter().filter(|s| source_ids.contains(s["id"].as_str().unwrap_or(""))).collect::<Vec<_>>();
    Ok(
        json!({"location":super::hierarchy::location(&conn,&entry.session_id)?,"entry":entry,"revision":revision,"catalog":catalog,"backlinks":backlinks,"versions":versions,"sources":sources}),
    )
}

pub fn restore(app: &AppCtx, id: &str, args: &Value) -> Result<Value, String> {
    let target = args
        .get("targetVersion")
        .and_then(Value::as_i64)
        .ok_or("memory_invalid:version")?;
    let expected = args
        .get("version")
        .and_then(Value::as_i64)
        .ok_or("memory_invalid:version")?;
    let mut conn = app.db().conn.lock().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let snapshot: String = tx
        .query_row(
            "SELECT snapshot FROM memory_versions WHERE entry_id=?1 AND version=?2",
            params![id, target],
            |r| r.get(0),
        )
        .map_err(|_| "memory_not_found")?;
    let mut entry: Entry = serde_json::from_str(&snapshot).map_err(|e| e.to_string())?;
    // Deleted targets are no longer navigable; restoring text must not resurrect deleted documents.
    let live: HashSet<String> = all(&tx)?.into_iter().map(|e| e.id).collect();
    entry.related.retain(|id| live.contains(id));
    let result = put(&tx, entry, expected, "restore")?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(json!(result))
}

pub fn source(app: &AppCtx, id: &str) -> Result<Value, String> {
    let conn = app.db().conn.lock().map_err(|e| e.to_string())?;
    conn.query_row("SELECT id,session_id,session_name,kind,agent_session_id,digest,content,created_at FROM memory_sources WHERE id=?1", [id], |r| Ok(json!({"id":r.get::<_,String>(0)?,"sessionId":r.get::<_,String>(1)?,"sessionName":r.get::<_,String>(2)?,"kind":r.get::<_,String>(3)?,"agentSessionId":r.get::<_,String>(4)?,"digest":r.get::<_,String>(5)?,"content":r.get::<_,String>(6)?,"createdAt":r.get::<_,i64>(7)?}))).optional().map_err(|e| e.to_string())?.ok_or("memory_not_found".into())
}
