//! Shared executable selection for terminal, chat, and background agent operations.

use crate::{host::AppCtx, models::SessionKind};
use std::path::Path;

pub fn command_name(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Cursor => "cursor-agent",
        SessionKind::Antigravity => "agy",
        SessionKind::Kiro => "kiro-cli",
        SessionKind::Zoo => "roo",
        _ => kind.as_str(),
    }
}

fn configured_path(raw: Option<&str>) -> Option<String> {
    let path = raw?.trim();
    if path.is_empty() {
        return None;
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        if let Some(home) = crate::host::home_dir() {
            return Some(home.join(rest).to_string_lossy().into_owned());
        }
    }
    Some(path.to_string())
}

fn select(
    session: Option<&str>,
    global: Option<&str>,
    discover: impl FnOnce() -> Option<String>,
) -> Option<String> {
    configured_path(session)
        .or_else(|| configured_path(global))
        .or_else(discover)
}

/// Explicit paths remain authoritative even if missing; never silently launch a different installation.
pub fn resolve(app: &AppCtx, kind: SessionKind, session_path: Option<&str>) -> Option<String> {
    if matches!(kind, SessionKind::Terminal | SessionKind::Browser) {
        return None;
    }
    if let Some(path) = configured_path(session_path) {
        return Some(path);
    }
    let settings = {
        let conn = app.db().conn.lock().ok()?;
        crate::db::repo::get_app_settings(&conn)
            .ok()?
            .remove("vlx-settings")
    };
    let settings: serde_json::Value = settings
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let global = settings
        .get("agentDefaults")
        .and_then(|v| v.get(kind.as_str()))
        .and_then(|v| v.get("path"))
        .and_then(|v| v.as_str());
    select(session_path, global, || {
        super::install::locate_installed_bin(kind.as_str())
            .or_else(|| find_on_path(command_name(kind)))
            .or_else(|| find_in_shell(command_name(kind)))
    })
}

pub fn for_session(app: &AppCtx, session: &crate::models::Session) -> String {
    resolve(app, session.kind, session.agent_path.as_deref())
        .unwrap_or_else(|| command_name(session.kind).to_string())
}

pub fn resolve_session(
    app: &AppCtx,
    id: &str,
    kind: SessionKind,
) -> Result<Option<String>, String> {
    if matches!(kind, SessionKind::Terminal | SessionKind::Browser) {
        return Ok(None);
    }
    let path = {
        let conn = app.db().conn.lock().unwrap();
        crate::db::repo::get_agent_path(&conn, id)?
    };
    Ok(resolve(app, kind, path.as_deref()))
}

/// Include the executable's directory so npm wrappers can find their adjacent Node runtime.
pub fn prepare_command(command: &mut std::process::Command, bin: &str) {
    let Some(parent) = Path::new(bin).parent().filter(|p| p.is_absolute()) else {
        return;
    };
    let existing = command
        .get_envs()
        .find(|(key, _)| *key == "PATH")
        .and_then(|(_, value)| value.map(|v| v.to_os_string()))
        .or_else(|| crate::appimage::clean_var("PATH").map(Into::into))
        .unwrap_or_default();
    let mut paths: Vec<_> = std::env::split_paths(&existing).collect();
    if !paths.iter().any(|p| p == parent) {
        paths.insert(0, parent.to_path_buf());
    }
    if let Ok(path) = std::env::join_paths(paths) {
        command.env("PATH", path);
    }
}

/// Match terminal startup when installation paths are set only in interactive shell profiles.
/// The command name comes from `command_name`; no session path is interpolated into shell code.
fn find_in_shell(bin: &str) -> Option<String> {
    #[cfg(windows)]
    {
        let _ = bin;
        None
    }
    #[cfg(not(windows))]
    {
        use std::process::Stdio;
        use std::time::{Duration, Instant};
        let shell = crate::appimage::clean_var("SHELL")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "/bin/sh".into());
        let mut child = crate::host::command(shell)
            .args(["-lic", &format!("command -v {bin}")])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
        if !child.wait().ok()?.success() {
            return None;
        }
        // A shell startup file can leave a background process holding stdout open. Read only
        // buffered output after shell exit so that process cannot extend the probe deadline.
        use std::io::Read;
        use std::os::fd::AsRawFd;
        let stdout = child.stdout.take()?;
        let fd = stdout.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return None;
        }
        let mut output = Vec::new();
        let _ = stdout.take(65536).read_to_end(&mut output);
        String::from_utf8_lossy(&output)
            .lines()
            .rev()
            .map(str::trim)
            .find(|p| Path::new(p).is_absolute() && is_executable_file(Path::new(p)))
            .map(str::to_owned)
    }
}

/// First executable of this name on PATH.
///
/// The agent may have been installed by a package manager none of the fixed-location probes know about,
/// so PATH is the last word before declaring it absent.
pub(crate) fn find_on_path(bin: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in exe_names(bin) {
            let candidate = dir.join(&name);
            if is_executable_file(&candidate) {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Filenames to try for one command, covering Windows's extension-based lookup.
fn exe_names(bin: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        return ["exe", "cmd", "bat"]
            .iter()
            .map(|ext| format!("{bin}.{ext}"))
            .collect();
    }
    #[cfg(not(windows))]
    {
        vec![bin.to_string()]
    }
}

pub(crate) fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    // Windows has no permission bit to read here; existing as a file is as far as this check goes.
    #[cfg(unix)]
    let runnable = {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let runnable = true;
    runnable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_and_chat_read_the_same_registered_path_and_updates() {
        let dir = std::env::temp_dir().join(format!("vlx-executable-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Db::open(&dir.join("test.db")).unwrap();
        let app = AppCtx::Headless(std::sync::Arc::new(crate::host::HeadlessHost::new(
            dir.clone(),
            db,
        )));
        let mut session = {
            let conn = app.db().conn.lock().unwrap();
            let project =
                crate::db::repo::create_virtual_project(&conn, "Executable test").unwrap();
            crate::db::repo::create_session(
                &conn,
                &project.id,
                None,
                "Codex",
                SessionKind::Codex,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        for path in ["/registered/first/codex", "/registered/second/codex"] {
            {
                let conn = app.db().conn.lock().unwrap();
                crate::db::repo::set_app_settings(
                    &conn,
                    &std::collections::HashMap::from([(
                        "vlx-settings".into(),
                        serde_json::json!({"agentDefaults":{"codex":{"path":path}}}).to_string(),
                    )]),
                )
                .unwrap();
            }
            assert_eq!(for_session(&app, &session), path);
            assert_eq!(
                resolve_session(&app, &session.id, session.kind)
                    .unwrap()
                    .as_deref(),
                Some(path)
            );
        }
        session.agent_path = Some("/session/codex".into());
        {
            let conn = app.db().conn.lock().unwrap();
            conn.execute(
                "UPDATE sessions SET agent_path = ?1 WHERE id = ?2",
                rusqlite::params![session.agent_path, session.id],
            )
            .unwrap();
        }
        assert_eq!(for_session(&app, &session), "/session/codex");
        assert_eq!(
            resolve_session(&app, &session.id, session.kind)
                .unwrap()
                .as_deref(),
            Some("/session/codex")
        );
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn direct_launch_finds_an_adjacent_runtime_with_minimal_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("vlx-agent-runtime-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("agent");
        let runtime = dir.join("vlx-test-runtime");
        std::fs::write(&bin, "#!/usr/bin/env vlx-test-runtime\n").unwrap();
        std::fs::write(&runtime, "#!/bin/sh\nprintf runtime-ok\n").unwrap();
        for path in [&bin, &runtime] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut command = std::process::Command::new(&bin);
        command.env("PATH", "/usr/bin:/bin");
        prepare_command(&mut command, bin.to_str().unwrap());
        let output = command.output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"runtime-ok");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn interactive_shell_returns_an_executable_path() {
        let path = find_in_shell("sh").expect("the login shell should resolve sh");
        assert!(Path::new(&path).is_absolute());
        assert!(is_executable_file(Path::new(&path)));
    }

    #[test]
    fn explicit_paths_win_without_discovery() {
        assert_eq!(
            select(Some(" /missing/session "), Some("/global"), || panic!(
                "unexpected discovery"
            )),
            Some("/missing/session".into())
        );
        assert_eq!(
            select(Some("  "), Some(" /global "), || panic!(
                "unexpected discovery"
            )),
            Some("/global".into())
        );
    }

    #[test]
    fn empty_paths_use_discovery() {
        assert_eq!(
            select(None, Some(" "), || Some("/discovered".into())),
            Some("/discovered".into())
        );
        assert_eq!(select(None, None, || None), None);
    }

    #[test]
    fn configured_home_is_expanded() {
        if let Some(home) = crate::host::home_dir() {
            assert_eq!(
                configured_path(Some(" ~/.local/bin/codex ")),
                Some(home.join(".local/bin/codex").to_string_lossy().into_owned())
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn runtime_path_preserves_existing_entries() {
        let mut command = std::process::Command::new("/opt/agent/bin/codex");
        command.env("PATH", "/usr/bin:/bin");
        prepare_command(&mut command, "/opt/agent/bin/codex");
        let path = command
            .get_envs()
            .find(|(key, _)| *key == "PATH")
            .unwrap()
            .1
            .unwrap();
        assert_eq!(
            std::env::split_paths(path).collect::<Vec<_>>(),
            vec![
                std::path::PathBuf::from("/opt/agent/bin"),
                "/usr/bin".into(),
                "/bin".into()
            ]
        );
    }
}
