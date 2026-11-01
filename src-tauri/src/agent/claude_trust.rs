//! Folder trust for the worktrees VelaTerm creates, so a woken or spawned Claude session in one doesn't
//! stop at Claude Code's trust prompt.
//!
//! Claude Code treats a folder as trusted when it, or a folder above it, has
//! `projects["<path>"].hasTrustDialogAccepted: true` in `~/.claude.json`. The walk up stops at a git root,
//! so each worktree needs its own entry. VelaTerm adds one for a worktree it creates only when the
//! repository's main checkout already has its own, so a worktree never gets more trust than its repo.
//!
//! The write follows `trust_seed.py` (Claude-OS machine profile): re-read the file just before an atomic
//! replace, merge the one key, keep every other key and the file's order, retry if Claude Code wrote it in
//! between, and never remove anything or set it to false.

use std::path::{Path, PathBuf};

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::json;

const KEY: &str = "hasTrustDialogAccepted";
const RETRIES: u32 = 5;

/// Trust a worktree VelaTerm just created in `repo`, if the repository's main checkout is trusted. Never
/// fails the caller: a worktree without trust only means Claude asks once.
pub fn seed_worktree(worktree: &str, repo: &str) {
    let Some(config) = config_path() else { return };
    if !config.is_file() {
        return;
    }
    let main = crate::git::worktree_list(repo)
        .ok()
        .and_then(|entries| entries.first().map(|entry| entry.path.clone()))
        .unwrap_or_else(|| repo.to_string());
    match seed(&config, worktree, &main) {
        Ok(true) => {
            crate::diagnostics::record("INFO", "claude_trust_seeded", json!({"worktree": key_for(worktree)}));
        }
        Ok(false) => {}
        Err(error) => {
            let data = json!({"worktree": key_for(worktree), "error": error});
            crate::diagnostics::record("WARN", "claude_trust_seed_failed", data);
        }
    }
}

/// `~/.claude.json`. Tests never reach the real one.
fn config_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::host::home_dir().map(|home| home.join(".claude.json"))
}

/// Claude Code's key for a folder: its real path, with forward slashes on Windows.
fn key_for(path: &str) -> String {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let text = real.to_string_lossy();
    if !cfg!(windows) {
        return text.into_owned();
    }
    let text = match text.strip_prefix(r"\\?\UNC\") {
        Some(share) => format!(r"\\{share}"),
        None => text.strip_prefix(r"\\?\").unwrap_or(&text).to_string(),
    };
    text.replace('\\', "/")
}

/// Add the worktree's entry when the main checkout has its own. Returns whether the file changed.
fn seed(config: &Path, worktree: &str, main: &str) -> Result<bool, String> {
    let (worktree, main) = (key_for(worktree), key_for(main));
    if worktree == main {
        return Ok(false);
    }
    for attempt in 1..=RETRIES {
        let raw = std::fs::read(config).map_err(|e| format!("Cannot read {}: {e}", config.display()))?;
        let text = std::str::from_utf8(&raw).map_err(|e| format!("{} is not UTF-8: {e}", config.display()))?;
        let mut data: Json = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| format!("{} is not valid JSON: {e}", config.display()))?;
        let Json::Obj(top) = &mut data else { return Err(format!("{} is not a JSON object", config.display())) };
        let Some(Json::Obj(projects)) = get_mut(top, "projects") else { return Ok(false) };
        if !trusted(projects, &main) || trusted(projects, &worktree) {
            return Ok(false);
        }
        match get_mut(projects, &worktree) {
            Some(Json::Obj(entry)) => set(entry, KEY, Json::Bool(true)),
            _ => set(projects, &worktree, Json::Obj(vec![(KEY.to_string(), Json::Bool(true))])),
        }
        let mut out = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
        if raw.ends_with(b"\n") {
            out.push('\n');
        }
        let tmp = config.with_file_name(format!(".claude.json.velaterm.{}.tmp", std::process::id()));
        std::fs::write(&tmp, out).map_err(|e| format!("Cannot write {}: {e}", tmp.display()))?;
        // Claude Code may have written the file since it was read; if so, start over from its version.
        let unchanged = std::fs::read(config).map(|now| now == raw).unwrap_or(false);
        if unchanged && std::fs::rename(&tmp, config).is_ok() {
            return Ok(true);
        }
        let _ = std::fs::remove_file(&tmp);
        std::thread::sleep(std::time::Duration::from_millis(200 * u64::from(attempt)));
    }
    Err(format!("{} kept changing; gave up after {RETRIES} tries", config.display()))
}

fn trusted(projects: &[(String, Json)], key: &str) -> bool {
    matches!(get(projects, key), Some(Json::Obj(entry)) if matches!(get(entry, KEY), Some(Json::Bool(true))))
}

// ── An order-keeping JSON value ──
//
// VelaTerm's serde_json sorts object keys, and rewriting Claude Code's file in sorted order would churn
// all of it. Objects here keep their keys in file order; a repeated key resolves to its last value, as in
// JavaScript.

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(serde_json::Number),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

fn get<'a>(obj: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    obj.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn get_mut<'a>(obj: &'a mut [(String, Json)], key: &str) -> Option<&'a mut Json> {
    obj.iter_mut().rev().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn set(obj: &mut Vec<(String, Json)>, key: &str, value: Json) {
    match get_mut(obj, key) {
        Some(slot) => *slot = value,
        None => obj.push((key.to_string(), value)),
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = Json;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_unit<E>(self) -> Result<Json, E> {
                Ok(Json::Null)
            }
            fn visit_bool<E>(self, v: bool) -> Result<Json, E> {
                Ok(Json::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Json, E> {
                Ok(Json::Num(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Json, E> {
                Ok(Json::Num(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Json, E> {
                serde_json::Number::from_f64(v).map(Json::Num).ok_or_else(|| E::custom("a number is not finite"))
            }
            fn visit_str<E>(self, v: &str) -> Result<Json, E> {
                Ok(Json::Str(v.to_string()))
            }
            fn visit_string<E>(self, v: String) -> Result<Json, E> {
                Ok(Json::Str(v))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Json::Arr(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, Json>()? {
                    entries.push(entry);
                }
                Ok(Json::Obj(entries))
            }
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Json::Null => serializer.serialize_unit(),
            Json::Bool(v) => serializer.serialize_bool(*v),
            Json::Num(v) => v.serialize(serializer),
            Json::Str(v) => serializer.serialize_str(v),
            Json::Arr(items) => items.serialize(serializer),
            Json::Obj(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A scratch folder holding a main checkout, a worktree folder and a config file.
    fn fixture(config: &str) -> (Dir, PathBuf, String, String) {
        let root = std::env::temp_dir().join(format!("vlx-trust-{}", uuid::Uuid::new_v4()));
        for dir in ["repo", "repo/.vlx-worktrees/wt"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let (repo, wt) = (key_for(root.join("repo").to_str().unwrap()), key_for(root.join("repo/.vlx-worktrees/wt").to_str().unwrap()));
        let path = root.join(".claude.json");
        std::fs::write(&path, config.replace("$REPO", &repo).replace("$WT", &wt)).unwrap();
        (Dir(root), path, repo, wt)
    }

    #[test]
    fn a_worktree_of_a_trusted_repo_gets_one_key_and_the_file_keeps_its_order() {
        let (_dir, config, repo, wt) = fixture(
            r#"{"zeta": 1, "numStartups": 2.5, "projects": {"$REPO": {"allowedTools": [], "hasTrustDialogAccepted": true}}, "alpha": "é"}"#,
        );
        assert_eq!(seed(&config, &wt, &repo), Ok(true));
        let text = std::fs::read_to_string(&config).unwrap();
        let keys: Vec<_> = ["\"zeta\"", "\"numStartups\"", "\"projects\"", "\"alpha\""].iter().map(|k| text.find(k).unwrap()).collect();
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "top-level order kept: {text}");
        assert!(text.starts_with("{\n  \"zeta\": 1,") && !text.ends_with('\n'), "{text}");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["projects"][&wt], json!({"hasTrustDialogAccepted": true}));
        assert_eq!(value["projects"][&repo]["allowedTools"], json!([]));
        assert_eq!((value["numStartups"].clone(), value["alpha"].clone()), (json!(2.5), json!("é")));
        // Already trusted: nothing to write.
        assert_eq!(seed(&config, &wt, &repo), Ok(false));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), text);
    }

    #[test]
    fn an_existing_entry_keeps_its_keys_and_a_false_flag_is_raised() {
        let (_dir, config, repo, wt) = fixture(
            "{\n  \"projects\": {\"$REPO\": {\"hasTrustDialogAccepted\": true}, \"$WT\": {\"lastCost\": 0.25, \"hasTrustDialogAccepted\": false}}\n}\n",
        );
        assert_eq!(seed(&config, &wt, &repo), Ok(true));
        let text = std::fs::read_to_string(&config).unwrap();
        assert!(text.ends_with("}\n"), "a trailing newline stays");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["projects"][&wt], json!({"lastCost": 0.25, "hasTrustDialogAccepted": true}));
    }

    #[test]
    fn an_untrusted_repo_gives_its_worktrees_nothing() {
        for config in [
            r#"{"projects": {"$REPO": {"hasTrustDialogAccepted": false}}}"#,
            r#"{"projects": {"$REPO": {}}}"#,
            r#"{"projects": {}}"#,
            r#"{"numStartups": 1}"#,
        ] {
            let (_dir, path, repo, wt) = fixture(config);
            let before = std::fs::read_to_string(&path).unwrap();
            assert_eq!(seed(&path, &wt, &repo), Ok(false), "{config}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        }
    }

    #[test]
    fn the_main_checkout_itself_and_a_broken_file_are_left_alone() {
        let (_dir, config, repo, _) = fixture(r#"{"projects": {"$REPO": {"hasTrustDialogAccepted": true}}}"#);
        assert_eq!(seed(&config, &repo, &repo), Ok(false));
        std::fs::write(&config, "{not json").unwrap();
        let (_d2, _, repo2, wt2) = fixture("{}");
        assert!(seed(&config, &wt2, &repo2).unwrap_err().contains("not valid JSON"));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "{not json");
    }

    #[test]
    fn keys_are_real_paths_with_forward_slashes_on_windows() {
        let tmp = std::env::temp_dir();
        let key = key_for(tmp.to_str().unwrap());
        assert!(!key.contains('\\') || !cfg!(windows), "{key}");
        assert!(!key.starts_with("//?/"), "{key}");
        assert_eq!(key_for(&key), key, "a key maps to itself");
    }
}
