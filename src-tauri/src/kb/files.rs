use super::*;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{Read, Write};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub path: String,
    pub absolute_path: String,
    pub name: String,
    pub kind: String,
    pub size: u64,
    pub updated_at: i64,
    pub favorite: bool,
    pub tags: Vec<String>,
    pub summary: String,
}
pub struct Note {
    pub path: String,
    pub content: String,
    pub digest: String,
}
pub struct Snapshot {
    pub nodes: Vec<Node>,
    pub notes: Vec<Note>,
    pub skipped: usize,
}
fn markdown(p: &Path) -> bool {
    p.extension().is_some_and(|s| {
        s.to_string_lossy().eq_ignore_ascii_case("md")
            || s.to_string_lossy().eq_ignore_ascii_case("markdown")
    })
}
fn excerpt(content: &str) -> String {
    let body = content
        .strip_prefix("---\n")
        .and_then(|s| s.split_once("\n---").map(|p| p.1))
        .unwrap_or(content);
    body.lines()
        .map(str::trim)
        .filter(|s| {
            !s.is_empty() && !s.starts_with(['#', '|']) && !s.starts_with("```") && *s != "---"
        })
        .take(4)
        .map(|s| {
            s.trim_start_matches(['-', '*', ' '])
                .replace("[ ] ", "")
                .replace("[x] ", "")
                .replace("[[", "")
                .replace("]]", "")
                .replace("**", "")
                .replace('`', "")
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(180)
        .collect()
}
pub fn scan(app: &AppCtx, v: &Vault) -> Result<Snapshot> {
    let base = root(v)?;
    let mut todo = vec![base.clone()];
    let mut snapshot = Snapshot {
        nodes: vec![],
        notes: vec![],
        skipped: 0,
    };
    let conn = app.db().conn.lock().map_err(err)?;
    let mut statement = conn
        .prepare("SELECT path FROM kb_favorites WHERE vault_id=?1")
        .map_err(err)?;
    let favorite: HashSet<String> = statement
        .query_map([&v.id], |r| r.get(0))
        .map_err(err)?
        .collect::<std::result::Result<_, _>>()
        .map_err(err)?;
    let mut live = HashSet::new();
    while let Some(dir) = todo.pop() {
        for item in std::fs::read_dir(dir).map_err(err)? {
            let item = item.map_err(err)?;
            let name = item.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let meta = item.path().symlink_metadata().map_err(err)?;
            if meta.file_type().is_symlink() || (!meta.is_file() && !meta.is_dir()) {
                snapshot.skipped += 1;
                continue;
            }
            if snapshot.nodes.len() >= MAX_FILES {
                return Err("kb_too_large".into());
            }
            let rel = item
                .path()
                .strip_prefix(&base)
                .map_err(err)?
                .to_string_lossy()
                .replace('\\', "/");
            // Paths which cannot be represented by the command contract remain untouched.
            if relative(&rel, false).is_err() {
                snapshot.skipped += 1;
                continue;
            }
            let modified = meta
                .modified()
                .map_err(err)?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let mut node = Node {
                path: rel.clone(),
                absolute_path: item.path().to_string_lossy().into_owned(),
                name,
                kind: if meta.is_dir() {
                    "folder"
                } else if markdown(&item.path()) {
                    "note"
                } else {
                    "asset"
                }
                .into(),
                size: meta.len(),
                updated_at: modified.as_millis() as i64,
                favorite: favorite.contains(&rel),
                tags: vec![],
                summary: String::new(),
            };
            if meta.is_dir() {
                todo.push(item.path());
            }
            if node.kind == "note" {
                if meta.len() > MAX_NOTE {
                    node.kind = "large".into();
                    snapshot.skipped += 1;
                } else {
                    let stamp = modified.as_nanos().to_string();
                    let cached:Option<(String,String)>=conn.query_row("SELECT content,digest FROM kb_files WHERE vault_id=?1 AND path=?2 AND mtime=?3 AND size=?4",params![v.id,rel,stamp,meta.len()],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
                    let (content, digest) = if let Some(cache) = cached {
                        cache
                    } else {
                        let bytes = read_bytes(&item.path(), MAX_NOTE)?;
                        let digest = hash(&bytes);
                        let content = match String::from_utf8(bytes) {
                            Ok(content) => content,
                            Err(_) => {
                                node.kind = "asset".into();
                                snapshot.skipped += 1;
                                snapshot.nodes.push(node);
                                continue;
                            }
                        };
                        conn.execute("INSERT INTO kb_files(vault_id,path,content,digest,mtime,size) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(vault_id,path) DO UPDATE SET content=excluded.content,digest=excluded.digest,mtime=excluded.mtime,size=excluded.size",params![v.id,rel,content,digest,stamp,meta.len()]).map_err(err)?;
                        (content, digest)
                    };
                    live.insert(rel.clone());
                    node.tags = links::tags(&content);
                    node.summary = excerpt(&content);
                    snapshot.notes.push(Note {
                        path: rel,
                        content,
                        digest,
                    });
                }
            }
            snapshot.nodes.push(node);
        }
    }
    let mut stmt = conn
        .prepare("SELECT path FROM kb_files WHERE vault_id=?1")
        .map_err(err)?;
    let indexed: Vec<String> = stmt
        .query_map([&v.id], |r| r.get(0))
        .map_err(err)?
        .collect::<std::result::Result<_, _>>()
        .map_err(err)?;
    for stale in indexed.into_iter().filter(|p| !live.contains(p)) {
        conn.execute(
            "DELETE FROM kb_files WHERE vault_id=?1 AND path=?2",
            params![v.id, stale],
        )
        .map_err(err)?;
    }
    snapshot
        .nodes
        .sort_by_key(|n| (n.kind != "folder", n.path.to_lowercase()));
    Ok(snapshot)
}
pub fn read_bytes(p: &Path, max: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(p)
        .map_err(|_| "kb_not_found")?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() as u64 > max {
        return Err("kb_too_large".into());
    }
    Ok(bytes)
}
pub fn tree(app: &AppCtx, v: &Vault, args: &Value) -> Result<Value> {
    let snapshot = scan(app, v)?;
    let tag = args["tag"].as_str().unwrap_or("");
    let filter = args["filter"].as_str().unwrap_or("");
    // Content matching lives in `search`; the tree only narrows by tag and favorites.
    let mut entries: Vec<_> = snapshot
        .nodes
        .iter()
        .filter(|n| n.kind == "note")
        .filter(|n| {
            (tag.is_empty() || n.tags.iter().any(|t| t == tag))
                && (filter != "favorites" || n.favorite)
        })
        .cloned()
        .collect();
    if args["sort"].as_str() == Some("title") {
        entries.sort_by_key(|n| n.path.to_lowercase());
    } else {
        entries.sort_by_key(|n| std::cmp::Reverse(n.updated_at));
    }
    let tags: BTreeSet<_> = snapshot
        .nodes
        .iter()
        .flat_map(|n| n.tags.iter().cloned())
        .collect();
    let conn = app.db().conn.lock().map_err(err)?;
    let mut stmt = conn
        .prepare(
            "SELECT id,path,deleted_at FROM kb_trash WHERE vault_id=?1 ORDER BY deleted_at DESC",
        )
        .map_err(err)?;
    let trash:Vec<Value>=stmt.query_map([&v.id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"path":r.get::<_,String>(1)?,"deletedAt":r.get::<_,i64>(2)?}))).map_err(err)?.collect::<std::result::Result<_,_>>().map_err(err)?;
    Ok(
        json!({"vault":v,"nodes":snapshot.nodes,"entries":entries,"tags":tags,"trash":trash,"skipped":snapshot.skipped}),
    )
}
pub fn get(app: &AppCtx, v: &Vault, raw: &str) -> Result<Value> {
    let p = path(v, raw, false)?;
    if !markdown(&p) {
        return Err("kb_invalid".into());
    }
    let snapshot = scan(app, v)?;
    let bytes = read_bytes(&p, MAX_NOTE)?;
    let digest = hash(&bytes);
    let content = String::from_utf8(bytes).map_err(|_| "kb_encoding")?;
    let paths: Vec<_> = snapshot
        .nodes
        .iter()
        .filter(|n| n.kind != "folder")
        .map(|n| n.path.clone())
        .collect();
    let outgoing: Vec<_> = links::parse(&content)
        .into_iter()
        .map(|link| {
            let target = links::resolve(raw, &link, &paths);
            json!({"target":link.target,"label":link.label,"embed":link.embed,"path":target})
        })
        .collect();
    let backlinks: Vec<_> = snapshot
        .notes
        .iter()
        .filter(|n| {
            n.path != raw
                && links::parse(&n.content)
                    .iter()
                    .any(|l| links::resolve(&n.path, l, &paths).as_deref() == Some(raw))
        })
        .map(|n| json!({"path":n.path}))
        .collect();
    Ok(
        json!({"vaultId":v.id,"path":raw,"absolutePath":p,"content":content,"digest":digest,"outline":links::outline(&content),"links":outgoing,"backlinks":backlinks,"tags":links::tags(&content),"favorite":snapshot.nodes.iter().any(|n|n.path==raw && n.favorite)}),
    )
}
pub fn atomic_write(p: &Path, bytes: &[u8], expected: &str) -> Result<()> {
    if hash(&read_bytes(p, MAX_NOTE)?) != expected {
        return Err("kb_conflict".into());
    }
    let temp = p.with_file_name(format!(".vkb-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(err)?;
        file.write_all(bytes).map_err(err)?;
        file.sync_all().map_err(err)?;
        std::fs::set_permissions(&temp, std::fs::metadata(p).map_err(err)?.permissions())
            .map_err(err)?;
        if hash(&read_bytes(p, MAX_NOTE)?) != expected {
            return Err("kb_conflict".into());
        }
        std::fs::rename(&temp, p).map_err(err)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
pub fn save(app: &AppCtx, v: &Vault, args: &Value) -> Result<Value> {
    let raw = required(args, "path")?;
    let p = path(v, raw, false)?;
    if !markdown(&p) {
        return Err("kb_invalid".into());
    }
    let content = args["content"].as_str().ok_or("kb_invalid")?;
    if content.len() as u64 > MAX_NOTE {
        return Err("kb_too_large".into());
    }
    atomic_write(&p, content.as_bytes(), required(args, "digest")?)?;
    get(app, v, raw)
}
pub fn create(_app: &AppCtx, v: &Vault, args: &Value) -> Result<Value> {
    let parent = args["parent"].as_str().unwrap_or("");
    let directory = path(v, parent, true)?;
    if !directory.is_dir() {
        return Err("kb_not_found".into());
    }
    let folder = args["kind"].as_str() == Some("folder");
    let name = args["name"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(if folder { "New folder" } else { "Untitled.md" });
    let name = if !folder && !markdown(Path::new(name)) {
        format!("{name}.md")
    } else {
        name.into()
    };
    if name.contains('/') {
        return Err("kb_invalid_path".into());
    }
    let raw = if parent.is_empty() {
        name
    } else {
        format!("{parent}/{name}")
    };
    let p = path(v, &raw, false)?;
    let absolute = p.to_string_lossy().into_owned();
    if p.exists() {
        return Err("kb_exists".into());
    }
    if folder {
        std::fs::create_dir(p).map_err(err)?;
    } else {
        let content = args["content"].as_str().unwrap_or("");
        if content.len() as u64 > MAX_NOTE {
            return Err("kb_too_large".into());
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(p)
            .map_err(err)?;
        file.write_all(content.as_bytes()).map_err(err)?;
    }
    Ok(json!({"path":raw,"kind":if folder{"folder"}else{"note"},"absolutePath":absolute}))
}
pub fn move_path(app: &AppCtx, v: &Vault, args: &Value) -> Result<Value> {
    let from = required(args, "from")?;
    let to = required(args, "to")?;
    let source = path(v, from, false)?;
    let destination = path(v, to, false)?;
    if !source.exists() {
        return Err("kb_not_found".into());
    }
    if destination.exists() || to.starts_with(&format!("{from}/")) {
        return Err("kb_exists".into());
    }
    if !destination.parent().is_some_and(Path::is_dir) {
        return Err("kb_not_found".into());
    }
    let snapshot = scan(app, v)?;
    let paths: Vec<_> = snapshot
        .nodes
        .iter()
        .filter(|n| n.kind != "folder")
        .map(|n| n.path.clone())
        .collect();
    let mut changes = vec![];
    for note in &snapshot.notes {
        let moved = note.path == from || note.path.starts_with(&format!("{from}/"));
        let new_path = if moved {
            format!("{to}{}", &note.path[from.len()..])
        } else {
            note.path.clone()
        };
        let rewritten = links::rewrite(&note.content, &note.path, &new_path, &paths, from, to);
        if rewritten != note.content {
            changes.push((note, new_path, rewritten));
        }
    }
    for (note, _, _) in &changes {
        if hash(&read_bytes(&path(v, &note.path, false)?, MAX_NOTE)?) != note.digest {
            return Err("kb_conflict".into());
        }
    }
    std::fs::rename(&source, &destination).map_err(err)?;
    let mut written: Vec<(&Note, String, String)> = vec![];
    for (note, new_path, content) in &changes {
        let outcome = path(v, new_path, false)
            .and_then(|p| atomic_write(&p, content.as_bytes(), &note.digest));
        if let Err(error) = outcome {
            let mut incomplete = false;
            for (original, saved_path, saved_hash) in written.into_iter().rev() {
                if path(v, &saved_path, false)
                    .and_then(|p| atomic_write(&p, original.content.as_bytes(), &saved_hash))
                    .is_err()
                {
                    incomplete = true;
                }
            }
            if std::fs::rename(&destination, &source).is_err() || incomplete {
                return Err("kb_move_incomplete".into());
            }
            return Err(error);
        }
        written.push((note, new_path.clone(), hash(content.as_bytes())));
    }
    let conn = app.db().conn.lock().map_err(err)?;
    let mut stmt = conn
        .prepare("SELECT path FROM kb_favorites WHERE vault_id=?1")
        .map_err(err)?;
    let favorites: Vec<String> = stmt
        .query_map([&v.id], |r| r.get(0))
        .map_err(err)?
        .collect::<std::result::Result<_, _>>()
        .map_err(err)?;
    for p in favorites
        .into_iter()
        .filter(|p| p == from || p.starts_with(&format!("{from}/")))
    {
        conn.execute(
            "UPDATE kb_favorites SET path=?1 WHERE vault_id=?2 AND path=?3",
            params![format!("{to}{}", &p[from.len()..]), v.id, p],
        )
        .map_err(err)?;
    }
    drop(stmt);
    drop(conn);
    Ok(json!({"path":to,"updatedLinks":changes.len()}))
}
pub fn trash(app: &AppCtx, v: &Vault, raw: &str) -> Result<Value> {
    let source = path(v, raw, false)?;
    if !source.exists() {
        return Err("kb_not_found".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let trash_root = root(v)?.join(".vkb-trash");
    if trash_root
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err("kb_invalid_path".into());
    }
    std::fs::create_dir_all(&trash_root).map_err(err)?;
    let destination = trash_root.join(&id);
    std::fs::rename(&source, &destination).map_err(err)?;
    if let Err(e) = app.db().conn.lock().map_err(err)?.execute(
        "INSERT INTO kb_trash(id,vault_id,path,deleted_at) VALUES(?1,?2,?3,?4)",
        params![id, v.id, raw, now()],
    ) {
        let _ = std::fs::rename(destination, source);
        return Err(err(e));
    }
    Ok(json!({"id":id}))
}
pub fn restore(app: &AppCtx, v: &Vault, id: &str) -> Result<Value> {
    let conn = app.db().conn.lock().map_err(err)?;
    let raw: String = conn
        .query_row(
            "SELECT path FROM kb_trash WHERE id=?1 AND vault_id=?2",
            params![id, v.id],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?
        .ok_or("kb_not_found")?;
    let dest = path(v, &raw, false)?;
    if dest.exists() {
        return Err("kb_exists".into());
    }
    let source = root(v)?.join(".vkb-trash").join(id);
    if source
        .parent()
        .unwrap()
        .symlink_metadata()
        .map_err(err)?
        .file_type()
        .is_symlink()
        || source
            .symlink_metadata()
            .map_err(err)?
            .file_type()
            .is_symlink()
    {
        return Err("kb_invalid_path".into());
    }
    std::fs::create_dir_all(dest.parent().ok_or("kb_invalid_path")?).map_err(err)?;
    std::fs::rename(&source, &dest).map_err(err)?;
    conn.execute("DELETE FROM kb_trash WHERE id=?1", [id])
        .map_err(err)?;
    Ok(json!({"path":raw}))
}
/// One search result; the wire shape is the frozen `kb_search` contract.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Hit {
    vault_id: String,
    vault_name: String,
    path: String,
    absolute_path: String,
    name: String,
    summary: String,
    line: usize,
    matches: usize,
    score: usize,
    updated_at: i64,
    favorite: bool,
    tags: Vec<String>,
    /// Notes this one links to directly, in link order. Populated for the returned page only.
    related: Vec<Related>,
}
/// A note reached from a hit's own links, inside the same vault.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Related {
    path: String,
    name: String,
    absolute_path: String,
}
/// Body facts about a matched note before vault and file metadata are attached.
struct Found {
    summary: String,
    line: usize,
    matches: usize,
    score: usize,
}
/// First occurrence of any term in an already lowercased line.
fn first_hit(lowered: &str, terms: &[&str]) -> Option<usize> {
    terms.iter().filter_map(|t| lowered.find(*t)).min()
}
/// Window of at most `limit` characters around a hit, marked with `…` where the line was trimmed.
fn clip(line: &str, lowered: &str, offset: usize, limit: usize) -> String {
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= limit {
        return line.trim().to_string();
    }
    let hit = lowered[..offset].chars().count().min(chars.len() - 1);
    let start = hit
        .saturating_sub(limit / 3)
        .min(chars.len().saturating_sub(limit));
    let end = start + limit;
    // Reserve one character per trimmed end so the snippet never exceeds `limit`.
    let room = limit.saturating_sub(usize::from(start > 0) + usize::from(end < chars.len()));
    let mut start = start;
    let mut end = end;
    if end - start > room {
        end = (start + room).max(hit + 1).min(chars.len());
        start = end.saturating_sub(room);
    }
    let mut text = String::new();
    if start > 0 {
        text.push('…');
    }
    text.extend(&chars[start..end]);
    if end < chars.len() {
        text.push('…');
    }
    text
}
/// First non-empty line, used when only the name or path matched.
fn leading_line(content: &str, limit: usize) -> String {
    content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(limit)
        .collect()
}
/// Match every term against name/path/body and derive snippet, hit line, count and rank score.
fn match_note(path: &str, name: &str, content: &str, terms: &[&str]) -> Option<Found> {
    let path_lower = path.to_lowercase();
    let name_lower = name.to_lowercase();
    let body_lower = content.to_lowercase();
    if !terms
        .iter()
        .all(|t| path_lower.contains(t) || body_lower.contains(t))
    {
        return None;
    }
    let matches: usize = terms.iter().map(|t| body_lower.matches(t).count()).sum();
    let (line, summary) = content
        .lines()
        .zip(body_lower.lines())
        .enumerate()
        .find_map(|(i, (raw, low))| first_hit(low, terms).map(|o| (i + 1, clip(raw, low, o, 200))))
        .unwrap_or_else(|| (0, leading_line(content, 220)));
    let score = usize::from(terms.iter().all(|t| name_lower.contains(t))) * 100
        + usize::from(terms.iter().all(|t| path_lower.contains(t))) * 20
        + matches.min(20) * 5;
    Some(Found {
        summary,
        line,
        matches,
        score,
    })
}
pub fn search(app: &AppCtx, args: &Value) -> Result<Value> {
    let query = args["query"].as_str().unwrap_or("").trim().to_lowercase();
    if query.chars().count() > 1000 {
        return Err("kb_invalid".into());
    }
    let limit = args["limit"].as_u64().unwrap_or(100).clamp(1, 200) as usize;
    let terms: Vec<&str> = query.split_whitespace().collect();
    let mut entries: Vec<Hit> = vec![];
    let mut unavailable = vec![];
    if !terms.is_empty() {
        let targets = match args.get("vaultId").and_then(Value::as_str) {
            Some(id) => vec![vault(app, id)?],
            None => vaults(app)?,
        };
        for v in targets {
            match scan(app, &v) {
                Ok(snapshot) => {
                    let nodes: HashMap<&str, &Node> =
                        snapshot.nodes.iter().map(|n| (n.path.as_str(), n)).collect();
                    for note in &snapshot.notes {
                        let node = nodes.get(note.path.as_str());
                        let name = node
                            .map(|n| n.name.as_str())
                            .unwrap_or_else(|| note.path.rsplit('/').next().unwrap_or(&note.path));
                        let found = match match_note(&note.path, name, &note.content, &terms) {
                            Some(found) => found,
                            None => continue,
                        };
                        entries.push(Hit {
                            vault_id: v.id.clone(),
                            vault_name: v.name.clone(),
                            path: note.path.clone(),
                            absolute_path: node.map(|n| n.absolute_path.clone()).unwrap_or_default(),
                            name: name.to_string(),
                            summary: found.summary,
                            line: found.line,
                            matches: found.matches,
                            score: found.score,
                            updated_at: node.map(|n| n.updated_at).unwrap_or_default(),
                            favorite: node.is_some_and(|n| n.favorite),
                            tags: node.map(|n| n.tags.clone()).unwrap_or_default(),
                            related: vec![],
                        });
                    }
                }
                Err(_) => unavailable.push(json!({"vaultId":v.id,"vaultName":v.name})),
            }
        }
    }
    entries.sort_by(|a, b| b.score.cmp(&a.score).then(b.updated_at.cmp(&a.updated_at)));
    let total = entries.len();
    let has_more = total > limit;
    entries.truncate(limit);
    attach_related(app, &mut entries)?;
    Ok(json!({"entries":entries,"total":total,"hasMore":has_more,"unavailable":unavailable}))
}

/// Resolve each returned hit's direct links inside its own vault.
///
/// Only the returned page is parsed, so a broad query does not pay for the links of every match. A
/// vault whose root disappeared between the scan and this pass simply yields no links, and a hit whose
/// cached content is gone is left as-is: related notes are an addition to the result, not the result.
fn attach_related(app: &AppCtx, entries: &mut [Hit]) -> Result<()> {
    let all = vaults(app)?;
    let conn = app.db().conn.lock().map_err(err)?;
    let mut cache: HashMap<String, Option<(std::path::PathBuf, Vec<String>)>> = HashMap::new();
    for entry in entries.iter_mut() {
        if !cache.contains_key(&entry.vault_id) {
            let loaded = all.iter().find(|v| v.id == entry.vault_id).and_then(|v| {
                let base = root(v).ok()?;
                let mut stmt = conn
                    .prepare("SELECT path FROM kb_files WHERE vault_id=?1")
                    .ok()?;
                let paths: Vec<String> = stmt
                    .query_map([&entry.vault_id], |r| r.get(0))
                    .ok()?
                    .collect::<std::result::Result<_, _>>()
                    .ok()?;
                Some((base, paths))
            });
            cache.insert(entry.vault_id.clone(), loaded);
        }
        let Some((base, paths)) = cache.get(&entry.vault_id).and_then(Option::as_ref) else {
            continue;
        };
        let content: Option<String> = conn
            .query_row(
                "SELECT content FROM kb_files WHERE vault_id=?1 AND path=?2",
                params![entry.vault_id, entry.path],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let Some(content) = content else {
            continue;
        };
        let mut seen = HashSet::new();
        let mut related = Vec::new();
        for link in links::parse(&content) {
            let Some(target) = links::resolve(&entry.path, &link, paths) else {
                continue;
            };
            if target == entry.path || !seen.insert(target.clone()) {
                continue;
            }
            related.push(Related {
                name: target.rsplit('/').next().unwrap_or(&target).to_string(),
                absolute_path: base.join(&target).to_string_lossy().into_owned(),
                path: target,
            });
        }
        entry.related = related;
    }
    Ok(())
}
