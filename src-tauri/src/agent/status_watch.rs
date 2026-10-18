//! Process-wide view of which sessions are working, asking, or waiting, plus a way to block until that
//! changes.
//!
//! The authoritative state machine already knows all of this: agent hooks post to the local service and
//! it maps them to [`AgentState`]. What was missing is a way for something *outside* the frontend to ask,
//! and to wait. `vstat` is that consumer, and an orchestration's coordinator session lives on the waiting
//! form: it blocks here instead of polling, so watching several child sessions costs nothing between
//! events.
//!
//! **The version counter is the point.** Without it a waiter that was busy handling one change would miss
//! the next one and then block until its timeout, reporting nothing while work had actually finished. A
//! caller passes back the version it last saw; if the world moved on meanwhile, it returns immediately.
//! This is the same idea as an incremental read cursor.
//!
//! One `StatusWatch` per process, like the hook server it is fed by.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::pty::AgentState;

/// One session's last known state.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusRow {
    pub session_id: String,
    pub state: AgentState,
    /// Unix seconds when this state was recorded, for showing how long it has been that way.
    pub updated_at: i64,
    /// Unix seconds when the session entered this state. Unlike `updated_at`, a re-report of the same
    /// state leaves it alone, so it says how long a session has really been idle.
    pub state_since: i64,
}

#[derive(Default)]
struct Inner {
    /// Bumped on every recorded change; callers use it as a cursor.
    version: u64,
    states: HashMap<String, StatusRow>,
}

pub struct StatusWatch {
    inner: Mutex<Inner>,
    cv: Condvar,
}

/// The process's status watch.
fn watch() -> &'static StatusWatch {
    static WATCH: OnceLock<StatusWatch> = OnceLock::new();
    WATCH.get_or_init(StatusWatch::new)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Record a session's state and wake anything waiting.
///
/// Called from the hook service's request loop and from the chat engine's state reports, so it must stay
/// a fast in-memory operation: that loop also carries agent status callbacks and cannot be delayed.
///
/// An unchanged state still refreshes the timestamp but does **not** bump the version, so a session
/// re-reporting `working` on every tool call cannot wake every waiter each time.
pub fn record(session_id: &str, state: AgentState) {
    watch().record(session_id, state)
}

/// Mark a session that has already reported as no longer busy, for when its process ends.
///
/// Only a known session is touched: a plain terminal that never reported must not start appearing in
/// listings just because it closed. An agent whose process exits mid-turn sends no Stop, and without
/// this a watcher would wait on it forever.
pub fn settle(session_id: &str) {
    watch().settle(session_id)
}

/// Forget a session, so a deleted one stops appearing in listings.
pub fn forget(session_id: &str) {
    watch().forget(session_id)
}

/// Current version and every known session's state, ordered by id for a stable listing.
pub fn snapshot() -> (u64, Vec<StatusRow>) {
    watch().snapshot()
}

/// Block until the version moves past `since`, or `timeout` elapses.
///
/// Returns the current version and rows either way; a caller distinguishes "something happened" from
/// "nothing happened" by comparing the returned version with the one it passed in.
///
/// Callers run this off the hook service's accept loop — it parks for as long as the caller asked.
pub fn wait_for_change(since: u64, timeout: Duration) -> (u64, Vec<StatusRow>) {
    watch().wait_for_change(since, timeout)
}

/// The functions above, on one watch. The process uses a single one; tests build their own so that
/// sessions recorded elsewhere in the test binary cannot move the version they assert on.
impl StatusWatch {
    fn new() -> Self {
        StatusWatch {
            inner: Mutex::new(Inner::default()),
            cv: Condvar::new(),
        }
    }

    fn record(&self, session_id: &str, state: AgentState) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        let prev = inner.states.get(session_id);
        let changed = prev.map(|prev| prev.state != state).unwrap_or(true);
        let now = now_secs();
        let state_since = prev.filter(|_| !changed).map(|prev| prev.state_since).unwrap_or(now);
        inner.states.insert(
            session_id.to_string(),
            StatusRow {
                session_id: session_id.to_string(),
                state,
                updated_at: now,
                state_since,
            },
        );
        if changed {
            inner.version += 1;
            drop(inner);
            self.cv.notify_all();
        }
    }

    fn settle(&self, session_id: &str) {
        let known = self
            .inner
            .lock()
            .map(|inner| inner.states.contains_key(session_id))
            .unwrap_or(false);
        if known {
            self.record(session_id, AgentState::Waiting);
        }
    }

    fn forget(&self, session_id: &str) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if inner.states.remove(session_id).is_some() {
            inner.version += 1;
            drop(inner);
            self.cv.notify_all();
        }
    }

    fn snapshot(&self) -> (u64, Vec<StatusRow>) {
        let Ok(inner) = self.inner.lock() else {
            return (0, Vec::new());
        };
        let mut rows: Vec<StatusRow> = inner.states.values().cloned().collect();
        rows.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        (inner.version, rows)
    }

    fn wait_for_change(&self, since: u64, timeout: Duration) -> (u64, Vec<StatusRow>) {
        let Ok(inner) = self.inner.lock() else {
            return (0, Vec::new());
        };
        let (inner, _) = self
            .cv
            .wait_timeout_while(inner, timeout, |i| i.version <= since)
            .unwrap_or_else(|e| e.into_inner());
        let mut rows: Vec<StatusRow> = inner.states.values().cloned().collect();
        rows.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        (inner.version, rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_bumps_the_version_only_on_a_real_change() {
        let watch = StatusWatch::new();
        let (before, _) = watch.snapshot();
        watch.record("a", AgentState::Working);
        let (after_first, rows) = watch.snapshot();
        assert!(after_first > before, "a new session is a change");
        assert_eq!(
            rows.iter().find(|r| r.session_id == "a").map(|r| r.state),
            Some(AgentState::Working)
        );

        // Re-reporting the same state must not wake waiters: an agent posts `working` on every tool
        // call, and each one would otherwise be an event.
        watch.record("a", AgentState::Working);
        let (after_same, _) = watch.snapshot();
        assert_eq!(after_same, after_first);

        watch.record("a", AgentState::Waiting);
        let (after_change, _) = watch.snapshot();
        assert!(after_change > after_first);
    }

    #[test]
    fn state_since_survives_a_re_report_and_moves_on_a_change() {
        let watch = StatusWatch::new();
        watch.record("a", AgentState::Waiting);
        // Backdate the entry so a re-report in the same second would still show if it reset the clock.
        watch.inner.lock().unwrap().states.get_mut("a").unwrap().state_since = 5;
        watch.record("a", AgentState::Waiting);
        let since = |w: &StatusWatch| w.snapshot().1.iter().find(|r| r.session_id == "a").unwrap().state_since;
        assert_eq!(since(&watch), 5, "the same state again is not a new state");
        watch.record("a", AgentState::Working);
        assert!(since(&watch) > 5, "a different state starts its own clock");
    }

    #[test]
    fn wait_returns_at_once_when_the_caller_is_behind() {
        let watch = StatusWatch::new();
        watch.record("a", AgentState::Working);
        let (version, _) = watch.snapshot();
        // The change already happened, so waiting on an older version must not park at all — this is
        // exactly the lost-wakeup case the version counter exists for.
        let started = std::time::Instant::now();
        let (got, _) = watch.wait_for_change(version - 1, Duration::from_secs(30));
        assert!(got >= version);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "waiting on a stale version must return immediately, took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn wait_wakes_on_a_change_and_otherwise_times_out() {
        let watch = std::sync::Arc::new(StatusWatch::new());
        let (version, _) = watch.snapshot();
        let recorder = watch.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            recorder.record("b", AgentState::Asking);
        });
        let started = std::time::Instant::now();
        let (got, rows) = watch.wait_for_change(version, Duration::from_secs(10));
        assert!(got > version, "the waiter should have been woken");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            rows.iter().find(|r| r.session_id == "b").map(|r| r.state),
            Some(AgentState::Asking)
        );

        // With nothing happening it returns on the deadline rather than hanging.
        let (current, _) = watch.snapshot();
        let started = std::time::Instant::now();
        let (after, _) = watch.wait_for_change(current, Duration::from_millis(200));
        assert_eq!(after, current, "a timeout reports no change");
        assert!(started.elapsed() >= Duration::from_millis(150));
    }

    #[test]
    fn settle_only_touches_sessions_that_have_reported() {
        let watch = StatusWatch::new();
        watch.record("a", AgentState::Working);
        let (version, _) = watch.snapshot();
        watch.settle("a");
        let (after, rows) = watch.snapshot();
        assert!(after > version, "an exiting worker is a change");
        assert_eq!(
            rows.iter().find(|r| r.session_id == "a").map(|r| r.state),
            Some(AgentState::Waiting)
        );
        // A session nobody has heard from stays absent: closing a terminal is not a report.
        watch.settle("b");
        let (unchanged, rows) = watch.snapshot();
        assert_eq!(unchanged, after);
        assert!(!rows.iter().any(|r| r.session_id == "b"));
    }

    #[test]
    fn forget_drops_a_session_from_listings() {
        let watch = StatusWatch::new();
        watch.record("a", AgentState::Working);
        assert!(watch.snapshot().1.iter().any(|r| r.session_id == "a"));
        watch.forget("a");
        assert!(!watch.snapshot().1.iter().any(|r| r.session_id == "a"));
        // Forgetting something unknown is a no-op, not a spurious wakeup.
        let (version, _) = watch.snapshot();
        watch.forget("never-existed");
        assert_eq!(watch.snapshot().0, version);
    }
}
