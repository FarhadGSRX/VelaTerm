//! Cooperative cancellation for the SSH and WSL connect flows.
//!
//! The connect panel passes a caller-chosen id to `ssh_connect` / `wsl_connect` and may later call
//! `connect_cancel` with the same id. The command registers a flag before moving to its blocking thread and
//! enters it there as the thread's current flag, so the long loops along the way (server download, upload,
//! stage boundaries) can poll `check()` without threading a token through every signature. On any other
//! thread, or outside a connect, `check()` is always Ok.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Stable error prefix for a connect the user stopped; the frontend drops it instead of showing it.
pub const CANCELLED_TAG: &str = "__VLX_CONNECT_CANCELLED__";

fn registry() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static R: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// A registered connect. Dropping it forgets the id.
pub struct Token {
    id: Option<String>,
    flag: Arc<AtomicBool>,
}

/// Register a connect under `id`; the command does this before its first await, long before a user can
/// click Stop. Without an id the token simply never cancels.
pub fn register(id: Option<String>) -> Token {
    let id = id.filter(|s| !s.is_empty());
    let flag = Arc::new(AtomicBool::new(false));
    if let Some(id) = &id {
        registry().lock().unwrap().insert(id.clone(), flag.clone());
    }
    Token { id, flag }
}

impl Token {
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// A handle to move onto the blocking thread; the token itself stays with the command and keeps the id
    /// registered until the command returns.
    pub fn handle(&self) -> Handle {
        Handle(self.flag.clone())
    }
}

/// The flag of a registered connect, carried to the thread that runs it.
pub struct Handle(Arc<AtomicBool>);

impl Handle {
    /// Make this the current thread's flag until the guard drops.
    pub fn enter(&self) -> Scope {
        CURRENT.with(|c| *c.borrow_mut() = Some(self.0.clone()));
        Scope
    }
}

impl Drop for Token {
    fn drop(&mut self) {
        if let Some(id) = &self.id {
            registry().lock().unwrap().remove(id);
        }
    }
}

/// Guard returned by `Handle::enter`; clears the thread's current flag on drop.
pub struct Scope;

impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|c| *c.borrow_mut() = None);
    }
}

/// Request cancellation of the connect registered under `id`. An unknown id (the connect already
/// finished) is ignored, so nothing is left behind in the registry.
pub fn cancel(id: &str) {
    if let Some(flag) = registry().lock().unwrap().get(id) {
        flag.store(true, Ordering::SeqCst);
    }
}

/// Err(CANCELLED_TAG) once the current thread's connect has been cancelled; Ok everywhere else.
pub fn check() -> Result<(), String> {
    let cancelled = CURRENT.with(|c| c.borrow().as_ref().is_some_and(|f| f.load(Ordering::SeqCst)));
    if cancelled { Err(CANCELLED_TAG.to_string()) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_follows_the_entered_token_only() {
        assert!(check().is_ok(), "no token entered");
        let token = register(Some("cancel-test-a".into()));
        {
            let _scope = token.handle().enter();
            assert!(check().is_ok());
            cancel("cancel-test-a");
            assert!(token.is_cancelled());
            assert_eq!(check().unwrap_err(), CANCELLED_TAG);
        }
        assert!(check().is_ok(), "the scope cleared the current flag");
        drop(token);
        assert!(!registry().lock().unwrap().contains_key("cancel-test-a"));
    }

    #[test]
    fn cancel_of_unknown_id_leaves_nothing_behind() {
        cancel("cancel-test-b");
        assert!(!registry().lock().unwrap().contains_key("cancel-test-b"));
        let token = register(Some("cancel-test-b".into()));
        assert!(!token.is_cancelled());
    }

    #[test]
    fn missing_id_never_cancels() {
        let token = register(None);
        cancel("");
        assert!(!token.is_cancelled());
    }
}
