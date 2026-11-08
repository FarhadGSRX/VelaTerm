//! When a Claude that has reported no state still counts as at its prompt.
//!
//! A Claude in a terminal reports state only from its hooks, and none fires when it is resumed with no
//! prompt (a tab clicked after a restart). Such a session has no state until its next turn, which can be
//! never. Reading "no state" as "still starting" then leaves `vtell` queueing to it with nothing to deliver
//! the message, `vstop` refusing it, and `--quit-if-quiet` blocked by it.
//!
//! The rule is a guess from time, because Claude gives no signal: a Claude with a saved conversation has
//! nothing to ask at launch (no folder-trust prompt for a folder it has already worked in), so once its
//! terminal has been up for [`PROMPT_AFTER`] it is taken to be waiting. A Claude with no saved conversation
//! (a fresh spawn, possibly at a trust prompt) keeps "no state means starting". For the quit check alone,
//! any Claude with no state counts as quiet after [`STUCK_AFTER`].

use std::time::Duration;

use crate::{models::SessionKind, pty::AgentState};

/// Terminal uptime after which a resumed Claude with a saved conversation counts as at its prompt.
pub const PROMPT_AFTER: Duration = Duration::from_secs(15);
/// Terminal uptime after which the quit check counts any Claude with no state as quiet.
pub const STUCK_AFTER: Duration = Duration::from_secs(600);

/// The state to act on for `vtell` and `vstop`: the reported one, or `Waiting` for a Claude that has a saved
/// conversation, reported nothing, and whose terminal has been up for [`PROMPT_AFTER`] or more. `uptime` is
/// `None` when the terminal is not running.
pub(crate) fn effective_state(
    kind: SessionKind,
    has_saved_conversation: bool,
    uptime: Option<Duration>,
    reported: Option<AgentState>,
) -> Option<AgentState> {
    if reported.is_some() {
        return reported;
    }
    match uptime {
        Some(up) if kind == SessionKind::Claude && has_saved_conversation && up >= PROMPT_AFTER => {
            Some(AgentState::Waiting)
        }
        _ => None,
    }
}

/// The state the quit check acts on: [`effective_state`], and also `Waiting` for any Claude that has
/// reported nothing once its terminal has been up for [`STUCK_AFTER`].
pub(crate) fn quit_state(
    kind: SessionKind,
    has_saved_conversation: bool,
    uptime: Option<Duration>,
    reported: Option<AgentState>,
) -> Option<AgentState> {
    let state = effective_state(kind, has_saved_conversation, uptime, reported);
    match uptime {
        Some(up) if state.is_none() && kind == SessionKind::Claude && up >= STUCK_AFTER => Some(AgentState::Waiting),
        _ => state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECS: fn(u64) -> Option<Duration> = |s| Some(Duration::from_secs(s));

    #[test]
    fn a_reported_state_always_wins() {
        for state in [AgentState::Working, AgentState::Asking, AgentState::Background, AgentState::Waiting] {
            assert_eq!(effective_state(SessionKind::Claude, true, SECS(3600), Some(state)), Some(state));
            assert_eq!(quit_state(SessionKind::Claude, false, SECS(3600), Some(state)), Some(state));
        }
    }

    #[test]
    fn a_saved_conversation_is_at_its_prompt_after_fifteen_seconds() {
        assert_eq!(effective_state(SessionKind::Claude, true, SECS(14), None), None);
        assert_eq!(effective_state(SessionKind::Claude, true, SECS(15), None), Some(AgentState::Waiting));
        assert_eq!(effective_state(SessionKind::Claude, true, SECS(60), None), Some(AgentState::Waiting));
    }

    #[test]
    fn no_saved_conversation_keeps_no_state_as_starting() {
        assert_eq!(effective_state(SessionKind::Claude, false, SECS(60), None), None);
        assert_eq!(effective_state(SessionKind::Claude, false, SECS(599), None), None);
    }

    #[test]
    fn a_terminal_that_is_not_running_is_never_at_a_prompt() {
        assert_eq!(effective_state(SessionKind::Claude, true, None, None), None);
        assert_eq!(quit_state(SessionKind::Claude, true, None, None), None);
    }

    #[test]
    fn other_kinds_are_not_guessed_at() {
        assert_eq!(effective_state(SessionKind::Terminal, true, SECS(60), None), None);
        assert_eq!(quit_state(SessionKind::Terminal, true, SECS(3600), None), None);
    }

    #[test]
    fn the_quit_check_also_counts_any_stuck_claude_after_ten_minutes() {
        assert_eq!(quit_state(SessionKind::Claude, false, SECS(599), None), None);
        assert_eq!(quit_state(SessionKind::Claude, false, SECS(600), None), Some(AgentState::Waiting));
        assert_eq!(quit_state(SessionKind::Claude, true, SECS(20), None), Some(AgentState::Waiting));
        assert_eq!(quit_state(SessionKind::Claude, true, SECS(10), None), None);
    }
}
