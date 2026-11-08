//! Commands started with `vrun`, tracked per session so the session can show them and stay busy while
//! they run.
//!
//! `vrun` leaves the work in its own process session and records it under `<data dir>/runs/<label>/`: the
//! PID, the log, the exit code once it ends, and a `meta.json` naming the session that started it. That
//! directory, not memory, is the source of truth. The waiting half of `vrun` can time out or stop to
//! report progress and exit while the work goes on, and VelaTerm itself can restart in the middle of an hour-long build, so the registry
//! is rebuilt by scanning it: whenever `vrun` announces a start or a finish, and on a short poll that
//! notices work ending with nobody left to announce it.
//!
//! Terminal sessions depend on this entirely. In a terminal the agent reports only its own turns, and a
//! turn ends as soon as the work has been started, so without the registry a session looks finished for
//! the whole time its build runs.
//!
//! A run's record outlives the run by a day, so the result can still be collected, and is then swept away.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use crate::host::AppCtx;
use crate::pty::{AgentState, StatusSignal};

/// What `vrun` writes beside the PID once the work has started.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMeta {
    /// The session whose environment started it; empty outside a VelaTerm session.
    #[serde(default)]
    pub session_id: String,
    pub command: Vec<String>,
    /// Milliseconds since the epoch.
    pub started_at: u64,
    /// The supervisor waiting on the command; it leads the run's process group.
    pub pid: u32,
    /// When waiting gives up, in milliseconds since the epoch; absent when no `-t` limit was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<u64>,
    /// Seconds between progress reports, counted from the start. Absent in records older than the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<u64>,
}

pub const META_FILE: &str = "meta.json";

/// One live run as a session's clients see it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunBrief {
    pub label: String,
    /// The command line as given to `vrun`.
    pub command: String,
    /// Milliseconds since the epoch.
    pub started_at: u64,
}

/// How often live runs are checked for work that ended without an announcement.
const POLL: Duration = Duration::from_secs(3);

/// How long a session held on background waits, once its last run ends, for its agent to report on the
/// result before it is returned to waiting on its own.
///
/// An agent that started the work in the background is woken when it ends and answers in a turn of its
/// own; that turn's end reports waiting through the normal path. In a terminal nothing announces the
/// start of that turn, so the hold simply outlasts it. An agent that is not woken at all leaves the
/// session on background for this long after the work is done.
#[cfg(not(test))]
const DRAIN_GRACE: Duration = Duration::from_secs(30);
#[cfg(test)]
const DRAIN_GRACE: Duration = Duration::from_millis(100);

/// How long a run's record stays after the run ends. `vrun --wait` collects the result from it after a
/// progress report, and `vrun --status` and the log viewer read it later still; a day also covers a
/// session picked up again the next morning.
const KEEP_ENDED: Duration = Duration::from_secs(24 * 60 * 60);
/// How often records past that are swept away.
const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);
/// Where swept records are moved before they are deleted, beside `runs/`.
const TRASH_DIR: &str = "runs-trash";

/// How much of a log the viewer shows.
const LOG_BYTES: u64 = 256 * 1024;
const LOG_LINES: usize = 400;

#[derive(Default)]
struct Registry {
    /// Live runs by session, ordered by start.
    live: HashMap<String, Vec<RunBrief>>,
    /// Sessions whose agent last reported waiting while their runs kept them on background.
    held: HashSet<String>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

/// Where `vrun` records its runs: beside the injected `bin/` directory it is started from.
fn runs_dir(app: &AppCtx) -> Option<PathBuf> {
    app.data_dir().ok().map(|dir| dir.join("runs"))
}

#[cfg(unix)]
pub(crate) fn alive(pid: u32) -> bool {
    // Signal 0 checks for existence without touching the process.
    pid != 0 && unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
}

#[cfg(windows)]
pub(crate) fn alive(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // STILL_ACTIVE: the exit code a process reports until it has exited.
    const STILL_ACTIVE: u32 = 259;
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return false;
    };
    let mut code = 0u32;
    let queried = unsafe { GetExitCodeProcess(handle, &mut code) }.is_ok();
    let _ = unsafe { CloseHandle(handle) };
    queried && code == STILL_ACTIVE
}

/// A label names a directory under `runs/`, so it may not reach outside it.
pub(crate) fn valid_label(label: &str) -> bool {
    !label.trim().is_empty() && !label.contains('/') && !label.contains('\\') && label != "." && label != ".."
}

/// The run recorded under `dir`, if it belongs to a session and is still going.
fn live_run(dir: &Path) -> Option<(String, RunBrief)> {
    if dir.join("exit").exists() {
        return None;
    }
    let meta: RunMeta = serde_json::from_str(&std::fs::read_to_string(dir.join(META_FILE)).ok()?).ok()?;
    if meta.session_id.is_empty() || !alive(meta.pid) {
        return None;
    }
    let label = dir.file_name()?.to_string_lossy().into_owned();
    Some((meta.session_id, RunBrief { label, command: meta.command.join(" "), started_at: meta.started_at }))
}

/// Every live run under `dir`, by session.
fn scan(dir: &Path) -> HashMap<String, Vec<RunBrief>> {
    let mut live: HashMap<String, Vec<RunBrief>> = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return live };
    for entry in entries.flatten() {
        if let Some((session_id, brief)) = live_run(&entry.path()) {
            live.entry(session_id).or_default().push(brief);
        }
    }
    for runs in live.values_mut() {
        runs.sort_by(|a, b| a.started_at.cmp(&b.started_at).then_with(|| a.label.cmp(&b.label)));
    }
    live
}

/// Rescan the runs directory and publish what changed.
///
/// Called when `vrun` announces a start or a finish, and by the poll. The scan checks every record and
/// reads one small file per unfinished run, so it stays cheap only while [`sweep`] keeps the directory to
/// about a day of runs.
pub fn refresh(app: &AppCtx) {
    let Some(dir) = runs_dir(app) else { return };
    apply(app, scan(&dir));
}

fn apply(app: &AppCtx, next: HashMap<String, Vec<RunBrief>>) {
    let (changed, drained) = {
        let mut registry = registry().lock().unwrap();
        let mut sessions: HashSet<String> = registry.live.keys().cloned().collect();
        sessions.extend(next.keys().cloned());
        let mut changed = Vec::new();
        let mut drained = Vec::new();
        for session_id in sessions {
            let before = registry.live.get(&session_id).cloned().unwrap_or_default();
            let after = next.get(&session_id).cloned().unwrap_or_default();
            if before == after {
                continue;
            }
            if !before.is_empty() && after.is_empty() && registry.held.contains(&session_id) {
                drained.push(session_id.clone());
            }
            changed.push((session_id, after));
        }
        registry.live = next;
        (changed, drained)
    };
    for (session_id, runs) in changed {
        crate::session_state::set_runs(app, &session_id, runs);
    }
    for session_id in drained {
        schedule_drain(app, &session_id);
    }
}

/// Start the poll that notices runs ending with nobody left to announce it, and sweeps ended runs away
/// at startup and every hour. Idempotent.
pub fn start(app: AppCtx) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(move || {
        let mut swept: Option<Instant> = None;
        loop {
            if swept.map_or(true, |at| at.elapsed() >= SWEEP_EVERY) {
                sweep(&app);
                swept = Some(Instant::now());
            }
            refresh(&app);
            std::thread::sleep(POLL);
        }
    });
}

/// Delete the records of runs that ended more than [`KEEP_ENDED`] ago.
///
/// Nothing else removes them, and agents pick a fresh label for most runs, so they pile up: a month of
/// use left 4,209, every one of them checked by the poll every three seconds.
fn sweep(app: &AppCtx) {
    let (Some(dir), Ok(data)) = (runs_dir(app), app.data_dir()) else { return };
    sweep_dir(&dir, &data.join(TRASH_DIR), SystemTime::now());
}

fn sweep_dir(dir: &Path, trash: &Path, now: SystemTime) {
    // Whatever an interrupted sweep left behind.
    let _ = std::fs::remove_dir_all(trash);
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) && expired(&entry.path(), now) {
            remove_run(&entry.path(), trash, now);
        }
    }
}

/// Whether the run under `dir` ended more than [`KEEP_ENDED`] before `now`. A run still going has not
/// ended: it has no exit code and its supervisor is alive.
fn expired(dir: &Path, now: SystemTime) -> bool {
    let modified = |path: &Path| std::fs::metadata(path).and_then(|meta| meta.modified()).ok();
    let ended = match modified(&dir.join("exit")) {
        Some(at) => at,
        None => {
            let meta: Option<RunMeta> =
                std::fs::read_to_string(dir.join(META_FILE)).ok().and_then(|text| serde_json::from_str(&text).ok());
            if meta.is_some_and(|meta| alive(meta.pid)) {
                return false;
            }
            // Stopped before it could record an exit code: its last output or last record is as close to
            // the end as there is.
            let latest = [modified(&dir.join("log")), modified(&dir.join(META_FILE)), modified(dir)].into_iter().flatten().max();
            let Some(at) = latest else { return false };
            at
        }
    };
    now.duration_since(ended).is_ok_and(|age| age > KEEP_ENDED)
}

/// Move a record aside under its start lock, then delete it there.
///
/// `vrun` holds the same lock while it starts a run, so a start reusing the label is never cut in half,
/// and once the lock is free its record is new and no longer expired. Moving the directory out first
/// lets the next start find a clean one even if deleting takes a while.
fn remove_run(dir: &Path, trash: &Path, now: SystemTime) {
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join("start.lock"));
    let Ok(lock) = lock else { return };
    if lock.try_lock().is_err() || !expired(dir, now) || std::fs::create_dir_all(trash).is_err() {
        return;
    }
    let aside = trash.join(uuid::Uuid::new_v4().to_string());
    let moved = std::fs::rename(dir, &aside).is_ok();
    drop(lock);
    // Windows may refuse to move a directory while a file in it is open; with the lock let go it can.
    if moved || std::fs::rename(dir, &aside).is_ok() {
        let _ = std::fs::remove_dir_all(&aside);
    }
}

/// Whether a session has work of its own running under `vrun`.
pub fn has_live(session_id: &str) -> bool {
    registry().lock().unwrap().live.get(session_id).is_some_and(|runs| !runs.is_empty())
}

/// The state to report for a session whose agent just reported `state`.
///
/// An agent that finished its turn while its runs go on is not done, so waiting becomes background and
/// the session is remembered as held. Every other report passes through and ends the hold: the agent
/// has spoken again, and what it says next is what counts.
pub fn hold(session_id: &str, state: AgentState) -> AgentState {
    let mut registry = registry().lock().unwrap();
    let live = registry.live.get(session_id).is_some_and(|runs| !runs.is_empty());
    if state == AgentState::Waiting && live {
        registry.held.insert(session_id.to_string());
        AgentState::Background
    } else {
        registry.held.remove(session_id);
        state
    }
}

/// Return a held session to waiting once its runs are over and its agent has not reported since.
///
/// The check and the report happen under the registry lock, so an agent reporting at the same moment
/// cannot have its state overwritten by this one.
fn schedule_drain(app: &AppCtx, session_id: &str) {
    let (app, session_id) = (app.clone(), session_id.to_string());
    std::thread::spawn(move || {
        std::thread::sleep(DRAIN_GRACE);
        let mut registry = registry().lock().unwrap();
        let live = registry.live.get(&session_id).is_some_and(|runs| !runs.is_empty());
        if live || !registry.held.remove(&session_id) {
            return;
        }
        crate::agent::status_watch::record(&session_id, AgentState::Waiting);
        app.emit(
            &StatusSignal::event_name(&session_id),
            StatusSignal::State { state: AgentState::Waiting, silent: false, authoritative: true },
        );
    });
}

/// The end of a run's log, and whether the run is still going.
pub fn log_tail(app: &AppCtx, label: &str) -> Result<serde_json::Value, String> {
    if !valid_label(label) {
        return Err("Invalid run label".to_string());
    }
    let dir = runs_dir(app).ok_or("The runs directory is unavailable")?.join(label);
    if !dir.is_dir() {
        return Err(format!("No run is recorded under {label}"));
    }
    let text = super::run_wait::tail_lines(&dir.join("log"), LOG_BYTES, LOG_LINES);
    let running = live_run(&dir).is_some();
    let exit_code = std::fs::read_to_string(dir.join("exit")).ok().and_then(|code| code.trim().parse::<i32>().ok());
    Ok(serde_json::json!({ "text": text, "running": running, "exitCode": exit_code }))
}

/// Stop a live run and everything it started.
///
/// `vrun` starts the work under a supervisor that leads its own process group, so the whole group is
/// signalled: a build script's compiler children would otherwise outlive it. The supervisor ignores the
/// signal long enough to record how the command ended.
pub fn stop(app: &AppCtx, label: &str) -> Result<(), String> {
    if !valid_label(label) {
        return Err("Invalid run label".to_string());
    }
    let dir = runs_dir(app).ok_or("The runs directory is unavailable")?.join(label);
    if live_run(&dir).is_none() {
        return Err(format!("{label} is not running"));
    }
    let meta: RunMeta = std::fs::read_to_string(dir.join(META_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or("The run record is unreadable")?;
    terminate(meta.pid)?;
    refresh(app);
    Ok(())
}

#[cfg(unix)]
fn terminate(pid: u32) -> Result<(), String> {
    let group = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGTERM) } == 0;
    if group || unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } == 0 {
        Ok(())
    } else {
        Err(format!("Failed to stop PID {pid}: {}", std::io::Error::last_os_error()))
    }
}

#[cfg(windows)]
fn terminate(pid: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let status = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("Failed to stop PID {pid}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("Failed to stop PID {pid}")) }
}

/// Serialize tests that touch the process-wide registry and start each from an empty one.
#[cfg(test)]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let guard = LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner());
    *registry().lock().unwrap() = Registry::default();
    guard
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(tag: &str) -> (AppCtx, PathBuf) {
        let dir = std::env::temp_dir().join(format!("vlx-runs-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("runs")).unwrap();
        let db = crate::db::Db::open(&dir.join("t.db")).unwrap();
        (AppCtx::Headless(std::sync::Arc::new(crate::host::HeadlessHost::new(dir.clone(), db))), dir.join("runs"))
    }

    fn record(runs: &Path, label: &str, session_id: &str, pid: u32, started_at: u64) {
        let dir = runs.join(label);
        std::fs::create_dir_all(&dir).unwrap();
        let meta = RunMeta { session_id: session_id.into(), command: vec!["sleep".into(), "60".into()], started_at, pid, deadline: None, interval: None };
        std::fs::write(dir.join(META_FILE), serde_json::to_string(&meta).unwrap()).unwrap();
        std::fs::write(dir.join("log"), "line one\nline two\n").unwrap();
    }

    /// Only an unfinished run of a live process that names its session is listed, oldest first.
    #[test]
    fn a_scan_lists_live_runs_by_session() {
        let _lock = test_lock();
        let (_app, runs) = ctx("scan");
        let own = std::process::id();
        record(&runs, "build", "s1", own, 20);
        record(&runs, "tests", "s1", own, 10);
        record(&runs, "finished", "s1", own, 5);
        std::fs::write(runs.join("finished").join("exit"), "0").unwrap();
        record(&runs, "orphan", "", own, 1);
        record(&runs, "gone", "s2", u32::MAX / 2, 1);

        let live = scan(&runs);
        assert_eq!(live.len(), 1, "only s1 has live runs: {live:?}");
        let labels: Vec<&str> = live["s1"].iter().map(|run| run.label.as_str()).collect();
        assert_eq!(labels, ["tests", "build"]);
        assert_eq!(live["s1"][0].command, "sleep 60");
    }

    /// While a run goes on, an agent finishing its turn leaves the session on background. Once the run ends
    /// and the agent says nothing more, the session returns to waiting by itself.
    #[test]
    fn a_live_run_holds_background_until_it_ends() {
        let _lock = test_lock();
        let _state = crate::session_state::test_lock();
        let (app, runs) = ctx("hold");
        let sid = format!("hold-{}", uuid::Uuid::new_v4());
        record(&runs, "build", &sid, std::process::id(), 1);
        refresh(&app);
        assert!(has_live(&sid));
        assert_eq!(crate::session_state::snapshot()[&sid].runs.len(), 1);

        assert_eq!(hold(&sid, AgentState::Waiting), AgentState::Background);
        assert_eq!(hold(&sid, AgentState::Asking), AgentState::Asking, "a question always gets through");
        assert_eq!(hold(&sid, AgentState::Waiting), AgentState::Background);

        std::fs::write(runs.join("build").join("exit"), "0").unwrap();
        refresh(&app);
        assert!(!has_live(&sid));
        assert!(crate::session_state::snapshot()[&sid].runs.is_empty());
        std::thread::sleep(DRAIN_GRACE * 4);
        let watched = crate::agent::status_watch::snapshot().1.into_iter().find(|row| row.session_id == sid).map(|row| row.state);
        assert_eq!(watched, Some(AgentState::Waiting));
        assert_eq!(crate::session_state::snapshot()[&sid].agent_state.as_deref(), Some("waiting"));
    }

    /// An agent that speaks after the run ends decides the state itself; the fallback must not override it.
    #[test]
    fn a_report_after_the_run_ends_cancels_the_fallback() {
        let _lock = test_lock();
        let _state = crate::session_state::test_lock();
        let (app, runs) = ctx("cancel");
        let sid = format!("cancel-{}", uuid::Uuid::new_v4());
        record(&runs, "build", &sid, std::process::id(), 1);
        refresh(&app);
        assert_eq!(hold(&sid, AgentState::Waiting), AgentState::Background);

        std::fs::write(runs.join("build").join("exit"), "0").unwrap();
        refresh(&app);
        assert_eq!(hold(&sid, AgentState::Working), AgentState::Working, "the agent answers the result");
        std::thread::sleep(DRAIN_GRACE * 4);
        assert!(crate::agent::status_watch::snapshot().1.iter().all(|row| row.session_id != sid));
    }

    /// Labels name directories under `runs/`, so a path cannot be smuggled through one.
    #[test]
    fn labels_cannot_leave_the_runs_directory() {
        let (app, _runs) = ctx("labels");
        for label in ["", "..", "../x", "a/b", "a\\b"] {
            assert!(log_tail(&app, label).is_err(), "{label:?}");
            assert!(stop(&app, label).is_err(), "{label:?}");
        }
    }

    /// Ended runs stay for a day, stopped ones included, and are then removed; live runs always stay.
    #[test]
    fn a_sweep_removes_runs_a_day_after_they_end() {
        let (_app, runs) = ctx("sweep");
        let trash = runs.with_file_name(TRASH_DIR);
        record(&runs, "live", "s1", std::process::id(), 1);
        record(&runs, "finished", "s1", std::process::id(), 1);
        std::fs::write(runs.join("finished").join("exit"), "0").unwrap();
        record(&runs, "stopped", "s1", u32::MAX / 2, 1);

        let hour_later = SystemTime::now() + Duration::from_secs(60 * 60);
        sweep_dir(&runs, &trash, hour_later);
        for label in ["live", "finished", "stopped"] {
            assert!(runs.join(label).is_dir(), "{label} is kept within the day");
        }

        let two_days_later = SystemTime::now() + 2 * KEEP_ENDED;
        sweep_dir(&runs, &trash, two_days_later);
        assert!(runs.join("live").is_dir(), "a live run is never swept");
        assert!(!runs.join("finished").exists());
        assert!(!runs.join("stopped").exists());
        assert_eq!(std::fs::read_dir(&trash).unwrap().count(), 0, "nothing is left aside");
    }

    /// A `vrun` starting under a label holds its start lock, and the sweep leaves that record alone.
    #[test]
    fn a_sweep_skips_a_label_being_started() {
        let (_app, runs) = ctx("sweep-lock");
        record(&runs, "build", "s1", std::process::id(), 1);
        std::fs::write(runs.join("build").join("exit"), "0").unwrap();
        let starting = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
            .open(runs.join("build").join("start.lock")).unwrap();
        starting.try_lock().unwrap();
        sweep_dir(&runs, &runs.with_file_name(TRASH_DIR), SystemTime::now() + 2 * KEEP_ENDED);
        assert!(runs.join("build").join("log").is_file());
    }

    /// The viewer reads the log's end and says whether the run is still going.
    #[test]
    fn the_log_viewer_reads_the_end_and_the_outcome() {
        let _lock = test_lock();
        let (app, runs) = ctx("log");
        record(&runs, "build", "s1", std::process::id(), 1);
        let live = log_tail(&app, "build").unwrap();
        assert_eq!(live["text"], "line one\nline two");
        assert_eq!(live["running"], true);
        std::fs::write(runs.join("build").join("exit"), "3").unwrap();
        let done = log_tail(&app, "build").unwrap();
        assert_eq!(done["running"], false);
        assert_eq!(done["exitCode"], 3);
    }
}
