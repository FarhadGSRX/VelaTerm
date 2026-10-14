//! Change notification for directories shown in the right panel, so the Files and Git tabs refresh
//! when something changes on disk instead of re-reading everything every few seconds.
//!
//! A client asks to watch a directory and gets back the root string events will carry. Watches are
//! leases: each request renews one for [`LEASE`], and a reaper drops roots nobody renewed, so a
//! client that disconnects or navigates away needs no unwatch call. Several clients, or the Files
//! and Git tabs of one client, share one watcher per directory.
//!
//! Each event says which directories changed, relative to the root (`""` is the root itself), so the
//! Files tab re-reads only those it shows. Changes inside a `.git` directory are not listing changes
//! but do move Git status, so they set `git` instead. Past [`MAX_DIRS`] directories in one burst the
//! list is dropped (`dirs: null`) and the client refreshes everything it shows.
//!
//! Recursive watching is cheap on macOS (one FSEvents stream) and Windows (one directory handle),
//! but on Linux inotify needs a watch per directory and a project with a large dependency folder can
//! exhaust the per-user limit, which would break watching for every other program. Linux therefore
//! reports `supported: false` and clients keep polling.

use crate::host::AppCtx;
use serde::Serialize;
use std::path::Path;

/// Event emitted with `{ root, dirs, git }` after something below a watched directory changed.
pub const CHANGED_EVENT: &str = "fs://changed";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WatchInfo {
    /// The watched directory; events carry this exact string.
    pub root: String,
    /// False when this platform does not watch, so the caller should keep polling.
    pub supported: bool,
}

/// What a changed path means to the panel: a directory whose listing changed, relative to the
/// root with `/` separators, or a change inside git's own files.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Dir(String),
    Git,
}

/// Classify a changed path below `root`. None for paths outside it and for churn every git command
/// produces (objects, logs, lock files), which changes nothing the panel shows.
pub(crate) fn classify(root: &Path, path: &Path) -> Option<Change> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    if let Some(at) = parts.iter().position(|p| p == ".git") {
        if name.ends_with(".lock") || matches!(parts.get(at + 1).map(String::as_str), Some("objects" | "logs" | "lfs")) {
            return None;
        }
        return Some(Change::Git);
    }
    // The listing that changed is the parent's; a change to the root itself has no parent below it.
    let dir = if parts.is_empty() { Vec::new() } else { parts[..parts.len() - 1].to_vec() };
    Some(Change::Dir(dir.join("/")))
}

/// Start or renew the watch on `root`, an existing directory.
pub fn watch(app: &AppCtx, root: &str) -> Result<WatchInfo, String> {
    if !Path::new(root).is_dir() {
        return Err(format!("Not a directory: {root}"));
    }
    imp::watch(app, root)
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;

    pub fn watch(_app: &AppCtx, root: &str) -> Result<WatchInfo, String> {
        Ok(WatchInfo { root: root.to_string(), supported: false })
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::*;
    use notify::{RecursiveMode, Watcher};
    use std::collections::{BTreeSet, HashMap};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    /// How long one request keeps a watch alive; clients renew well within it.
    const LEASE: Duration = Duration::from_secs(90);
    /// Changes arriving within this window collapse into one event: a checkout or build touches
    /// thousands of files at once.
    pub(super) const DEBOUNCE: Duration = Duration::from_millis(300);
    /// Upper bound on directories watched at once; the least recently renewed one makes room.
    const MAX_ROOTS: usize = 32;
    /// Distinct changed directories reported per event before falling back to "refresh everything".
    const MAX_DIRS: usize = 256;

    struct Entry {
        _watcher: notify::RecommendedWatcher,
        renewed: Instant,
    }

    /// Changes gathered during one debounce window.
    #[derive(Default)]
    struct Batch {
        scheduled: bool,
        dirs: BTreeSet<String>,
        overflow: bool,
        git: bool,
    }

    fn registry() -> &'static Mutex<HashMap<String, Entry>> {
        static REGISTRY: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            // Drop leases nobody renewed. One thread for the process, started with the first watch.
            std::thread::spawn(|| loop {
                std::thread::sleep(Duration::from_secs(30));
                if let Ok(mut map) = registry().lock() {
                    map.retain(|_, entry| entry.renewed.elapsed() < LEASE);
                }
            });
            Mutex::new(HashMap::new())
        })
    }

    pub fn watch(app: &AppCtx, root: &str) -> Result<WatchInfo, String> {
        let mut map = registry().lock().map_err(|_| "Watch registry unavailable")?;
        if let Some(entry) = map.get_mut(root) {
            entry.renewed = Instant::now();
            return Ok(WatchInfo { root: root.to_string(), supported: true });
        }
        if map.len() >= MAX_ROOTS {
            if let Some(oldest) = map.iter().min_by_key(|(_, e)| e.renewed).map(|(k, _)| k.clone()) {
                map.remove(&oldest);
            }
        }
        match start(app.clone(), root) {
            Ok(watcher) => {
                map.insert(root.to_string(), Entry { _watcher: watcher, renewed: Instant::now() });
                Ok(WatchInfo { root: root.to_string(), supported: true })
            }
            // Watching is an optimization; the client falls back to polling.
            Err(_) => Ok(WatchInfo { root: root.to_string(), supported: false }),
        }
    }

    #[cfg(test)]
    pub(crate) fn forget(root: &str) {
        registry().lock().unwrap().remove(root);
    }

    fn start(app: AppCtx, root: &str) -> Result<notify::RecommendedWatcher, String> {
        let given = PathBuf::from(root);
        // Events report resolved paths (FSEvents expands /var to /private/var, for one), so match
        // against the resolved root and translate back to the caller's string.
        let canonical = std::fs::canonicalize(&given).unwrap_or_else(|_| given.clone());
        // A linked worktree keeps its index and HEAD outside the working tree and its refs in the
        // shared git directory; watch those too, reporting their changes as `.git` changes.
        let git_dirs: Vec<PathBuf> = git_dirs(root)
            .into_iter()
            .filter(|d| !d.starts_with(&canonical))
            .collect();
        let batch = Arc::new(Mutex::new(Batch::default()));
        let event_root = root.to_string();
        let roots = (canonical.clone(), given.clone(), git_dirs.clone());
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            let (canonical, given, git_dirs) = &roots;
            let changes: Vec<Change> = event
                .paths
                .iter()
                .filter_map(|p| {
                    if git_dirs.iter().any(|d| p.starts_with(d)) {
                        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        let churn = name.ends_with(".lock")
                            || p.components().any(|c| matches!(c.as_os_str().to_str(), Some("objects" | "logs" | "lfs")));
                        return (!churn).then_some(Change::Git);
                    }
                    classify(canonical, p).or_else(|| classify(given, p))
                })
                .collect();
            if changes.is_empty() {
                return;
            }
            let mut b = batch.lock().unwrap();
            for change in changes {
                match change {
                    Change::Git => b.git = true,
                    Change::Dir(dir) if !b.overflow => {
                        b.dirs.insert(dir);
                        if b.dirs.len() > MAX_DIRS {
                            b.overflow = true;
                            b.dirs.clear();
                        }
                    }
                    Change::Dir(_) => {}
                }
            }
            // The first change schedules one emit; later ones inside the window join the batch.
            if b.scheduled {
                return;
            }
            b.scheduled = true;
            let (app, root, batch) = (app.clone(), event_root.clone(), batch.clone());
            std::thread::spawn(move || {
                std::thread::sleep(DEBOUNCE);
                let taken = std::mem::take(&mut *batch.lock().unwrap());
                let dirs = (!taken.overflow).then(|| taken.dirs.into_iter().collect::<Vec<_>>());
                app.emit(CHANGED_EVENT, serde_json::json!({ "root": root, "dirs": dirs, "git": taken.git }));
            });
        })
        .map_err(|e| e.to_string())?;
        watcher.watch(&given, RecursiveMode::Recursive).map_err(|e| e.to_string())?;
        for dir in git_dirs {
            let _ = watcher.watch(&dir, RecursiveMode::Recursive);
        }
        Ok(watcher)
    }

    /// The git directory and common git directory of the repository containing `root`, resolved;
    /// empty outside a repository. `--path-format=absolute` would need Git 2.31, so the common
    /// directory, which may be relative, is resolved against `root`.
    fn git_dirs(root: &str) -> Vec<PathBuf> {
        let Ok(out) = crate::host::command("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "--absolute-git-dir", "--git-common-dir"])
            .output()
        else {
            return Vec::new();
        };
        if !out.status.success() {
            return Vec::new();
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| {
                let path = Path::new(root).join(l);
                std::fs::canonicalize(&path).unwrap_or(path)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_map_to_their_parent_listing_or_to_git() {
        let root = Path::new("/r");
        assert_eq!(classify(root, Path::new("/r/a.txt")), Some(Change::Dir(String::new())));
        assert_eq!(classify(root, Path::new("/r/src/app/x.ts")), Some(Change::Dir("src/app".into())));
        assert_eq!(classify(root, Path::new("/r")), Some(Change::Dir(String::new())));
        assert_eq!(classify(root, Path::new("/r/.git/index")), Some(Change::Git));
        assert_eq!(classify(root, Path::new("/r/.git/refs/heads/main")), Some(Change::Git));
        assert_eq!(classify(root, Path::new("/r/sub/.git/HEAD")), Some(Change::Git));
        assert_eq!(classify(root, Path::new("/r/.git/index.lock")), None);
        assert_eq!(classify(root, Path::new("/r/.git/objects/ab/cd")), None);
        assert_eq!(classify(root, Path::new("/r/.git/logs/HEAD")), None);
        assert_eq!(classify(root, Path::new("/elsewhere/x")), None);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn a_burst_of_changes_emits_one_event_naming_the_directories() {
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("vlx-fs-watch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let root = dir.to_str().unwrap().to_string();
        let data = std::env::temp_dir().join(format!("vlx-fs-watch-data-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data).unwrap();
        let db = crate::db::Db::open(&data.join("t.db")).unwrap();
        let app = AppCtx::Headless(Arc::new(crate::host::HeadlessHost::new(data.clone(), db)));
        let events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
        let sink = events.clone();
        app.listen(CHANGED_EVENT, move |payload| sink.lock().unwrap().push(serde_json::from_str(payload).unwrap()));

        let info = watch(&app, &root).unwrap();
        assert_eq!(info, WatchInfo { root: root.clone(), supported: true });
        // FSEvents may still report the directories created above; let that settle first.
        std::thread::sleep(Duration::from_millis(800));
        events.lock().unwrap().clear();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        std::fs::write(dir.join("sub/b.txt"), "x").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while events.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(imp::DEBOUNCE * 2);
        let seen = events.lock().unwrap().clone();
        assert!(!seen.is_empty(), "no change event");
        assert!(seen.len() <= 2, "changes were not debounced: {seen:?}");
        assert!(seen.iter().all(|e| e["root"] == root.as_str() && e["git"] == false));
        let dirs: Vec<String> = seen.iter().flat_map(|e| e["dirs"].as_array().cloned().unwrap_or_default())
            .map(|d| d.as_str().unwrap().to_string()).collect();
        assert!(dirs.contains(&String::new()) && dirs.contains(&"sub".to_string()), "{dirs:?}");
        // Renewing shares the watcher.
        assert_eq!(watch(&app, &root).unwrap(), info);
        imp::forget(&root);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&data);
    }
}
