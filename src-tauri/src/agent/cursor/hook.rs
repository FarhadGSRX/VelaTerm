//! Short-lived Cursor hook transport. Never initialize application or GUI state here.
//!
//! The loopback forwarding, payload limits, and fail-open exit are shared with the other native hook
//! bridges in [`crate::agent::hook_bridge`]; Cursor only fixes its event set and its `{}` response.

/// Always exit successfully, including with missing environment or a pipe whose writer never closes.
pub fn run(args: &[String]) -> ! {
    crate::agent::hook_bridge::run(args.get(2).map(String::as_str), &["boot", "working", "waiting"], b"{}\n")
}
