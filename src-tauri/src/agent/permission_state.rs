//! Permission configuration and runtime evidence are separate facts.

use crate::{db::repo, host::AppCtx, models::SessionKind};
use super::permission_catalog;

#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionState {
    configured: String,
    current: Option<String>,
    launch: Option<String>,
    pending: Option<String>,
    activation: &'static str,
    running: bool,
}

fn normalize(kind: SessionKind, mode: Option<&str>) -> Result<String, String> {
    if permission_catalog::modes(kind).is_empty() {
        Ok(if matches!(mode, Some("skip" | "bypassPermissions" | "full-access")) { "skip" } else { "default" }.into())
    } else {
        permission_catalog::normalize(kind, mode).map(str::to_string)
    }
}

fn state(configured: String, runtime: Option<(String, Option<String>)>, chat: bool, kind: SessionKind) -> PermissionState {
    let Some((launch, current)) = runtime else {
        return PermissionState { pending: Some(configured.clone()), configured, current: None,
            launch: None, activation: "nextStart", running: false };
    };
    let activation = if launch != configured { "restart" }
        else if chat && kind == SessionKind::Codex && current.as_ref() != Some(&configured) { "nextTurn" }
        else if current.is_some() { "applied" } else { "unconfirmed" };
    let pending = matches!(activation, "restart" | "nextTurn").then(|| configured.clone());
    PermissionState { configured, current, launch: (!(chat && kind == SessionKind::Codex)).then_some(launch), pending, activation, running: true }
}

pub fn read(app: &AppCtx, session_id: &str) -> Result<PermissionState, String> {
    let (session, stored) = {
        let conn = app.db().conn.lock().unwrap();
        let session = repo::get_session(&conn, session_id)?.ok_or("Session not found")?;
        let stored = permission_catalog::effective(&conn, session.kind, session.permission_mode.as_deref())?;
        (session, stored)
    };
    let configured = normalize(session.kind, stored.as_deref())?;
    let chat = session.engine == "chat";
    let runtime = if chat {
        app.chat().permission_state(session_id)
    } else {
        app.pty().launch_permission_mode(session_id)
            .map(|mode| normalize(session.kind, mode.as_deref()).map(|launch| (launch, None)))
            .transpose()?
    };
    Ok(state(configured, runtime, chat, session.kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_terminal_changes_remain_pending_until_a_new_launch() {
        for kind in [SessionKind::Claude, SessionKind::Codex, SessionKind::Opencode,
            SessionKind::Copilot, SessionKind::Cursor, SessionKind::Antigravity, SessionKind::Cline,
            SessionKind::Omp, SessionKind::Crush, SessionKind::Kimi, SessionKind::Kiro,
            SessionKind::Grok, SessionKind::Zoo] {
            let original = normalize(kind, None).unwrap();
            let chosen = normalize(kind, Some("skip")).unwrap();
            let before = state(chosen.clone(), Some((original.clone(), None)), false, kind);
            assert_eq!(before.activation, "restart", "{kind:?}");
            assert_eq!(before.current, None);
            assert_eq!(before.launch.as_deref(), Some(original.as_str()));
            assert_eq!(before.pending.as_deref(), Some(chosen.as_str()));
            let after = state(chosen.clone(), Some((chosen, None)), false, kind);
            assert_eq!(after.activation, "unconfirmed", "{kind:?}");
            assert_eq!(after.current, None); // Native terminal changes cannot be observed.
            assert_eq!(after.pending, None);
        }
    }

    #[test]
    fn codex_uses_acknowledged_policy_until_the_next_turn_accepts_the_change() {
        let before = state("full-access".into(), Some(("full-access".into(), Some("auto".into()))), true, SessionKind::Codex);
        assert_eq!(before.current.as_deref(), Some("auto"));
        assert_eq!(before.activation, "nextTurn");
        assert_eq!(before.pending.as_deref(), Some("full-access"));
        let after = state("full-access".into(), Some(("full-access".into(), Some("full-access".into()))), true, SessionKind::Codex);
        assert_eq!(after.activation, "applied");
        assert_eq!(after.pending, None);
    }

    #[test]
    fn stopped_sessions_and_unconfirmed_claude_launches_do_not_claim_active_permissions() {
        let stopped = state("default".into(), None, true, SessionKind::Claude);
        assert!(!stopped.running);
        assert_eq!(stopped.activation, "nextStart");
        let initial = state("default".into(), Some(("default".into(), None)), true, SessionKind::Claude);
        assert_eq!(initial.activation, "unconfirmed");
        assert_eq!(initial.current, None);
        let edited = state("plan".into(), Some(("default".into(), Some("default".into()))), true, SessionKind::Claude);
        assert_eq!(edited.current.as_deref(), Some("default"));
        assert_eq!(edited.activation, "restart");
    }
}
