//! Kiro CLI status bridge.
//!
//! Kiro reads lifecycle hooks from an agent configuration file, and an agent takes effect only when it is
//! selected with `--agent <name>`. Rather than editing the user's own agent, clone their default agent into
//! `~/.kiro/agents/vlx-term.json`, merge our hook entries into the clone, and launch that shadow agent. Their
//! prompt, tools, and MCP servers are preserved, and re-cloning on every launch keeps the copy current. This
//! mirrors the shadow-configuration approach already used for Crush in `agent/crush.rs`.
//!
//! On Unix the hook entries invoke a static forwarding script under `~/.kiro/vlx-term/`; on Windows they call
//! the VelaTerm executable's native `--agent-hook kiro` bridge, so no PowerShell script is involved. Neither
//! stores a port, token, or session ID: both read only the `VLX_*` variables inherited by the managed process
//! and no-op when `kiro-cli` runs in an ordinary terminal.
//!
//! Kiro exposes no permission-request hook, so Kiro sessions have no `asking` state; `working` and `waiting`
//! are both authoritative. The script deliberately writes nothing to stdout: Kiro treats a `preToolUse` hook as
//! blocking only on exit code 2, and an empty successful response means "no opinion", leaving the user's own
//! permission prompts untouched.
//!
//! The standalone global hooks directory `~/.kiro/hooks/` documented for Kiro CLI 3.0 is deliberately not used:
//! `kiro-cli 2.16.2`, the current stable build, contains no reference to that path and would silently ignore
//! such a file.

use std::path::{Path, PathBuf};

/// Shadow agent name, used both as the file stem and as the `--agent` argument.
pub const AGENT_NAME: &str = "vlx-term";

/// Substring identifying hook entries VelaTerm owns, so re-installing replaces them instead of accumulating
/// duplicates while leaving the user's own hook entries in the cloned config untouched.
///
/// Matched through [`is_ours`], which normalizes separators first: the command embeds a real filesystem
/// path, so on Windows it reads `vlx-term\hook.ps1` and a plain `contains` against this forward-slash
/// form never matched — every re-install then appended another copy of our entries.
const MARKER: &str = "vlx-term/hook.";

/// Argument prefix of the native bridge command used on Windows, which embeds no script path.
const BRIDGE_MARKER: &str = "--agent-hook kiro ";

/// Whether a hook command string is one VelaTerm wrote: a script command on either path separator, including
/// the PowerShell script earlier Windows versions wrote, or the native bridge command.
fn is_ours(command: &str) -> bool {
    command.replace('\\', "/").contains(MARKER) || command.contains(BRIDGE_MARKER)
}

/// Agent-config hook trigger paired with the `e=` value forwarded to the local hook server.
///
/// `agentSpawn` reports `boot`, which captures Kiro's own session ID as a resume anchor without changing state.
/// Prompt submission and both tool phases report `working`, and `stop` reports `waiting`.
const EVENTS: [(&str, &str); 5] = [
    ("agentSpawn", "boot"),
    ("userPromptSubmit", "kiro_working"),
    ("preToolUse", "kiro_working"),
    ("postToolUse", "kiro_working"),
    ("stop", "kiro_waiting"),
];

/// Static POSIX forwarding script. `$1` carries the `e=` value, so one script serves every trigger.
const SH_SCRIPT: &str = r#"#!/bin/sh
# VelaTerm Kiro status bridge (auto-installed by VelaTerm; safe to delete).
# Active only in Kiro sessions launched by VelaTerm; otherwise consume stdin and exit successfully.
if [ -z "$VLX_SPAWN_URL" ] || [ -z "$VLX_SESSION_ID" ] || [ -z "$VLX_TOKEN" ]; then
  cat >/dev/null 2>&1
  exit 0
fi
curl -s -m 3 -X POST -H "Content-Type: application/json" --data-binary @- \
  "$VLX_SPAWN_URL/hook/$VLX_SESSION_ID?t=$VLX_TOKEN&e=$1" >/dev/null 2>&1
exit 0
"#;

/// Kiro hook installation audit: record only phase, result, and duration, never configuration, paths, tokens, or session content.
pub fn audit_install(status: &str, duration_ms: u128) {
    crate::diagnostics::record(if status=="failed"{"WARN"}else{"INFO"},"kiro_hook_install",serde_json::json!({"status":status,"durationMs":duration_ms as u64,"inputCount":1,"outputCount":usize::from(status=="success")}));
}

/// Kiro configuration root: `KIRO_HOME` when set, otherwise `~/.kiro`.
pub(crate) fn kiro_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("KIRO_HOME") {
        return Some(PathBuf::from(home));
    }
    crate::host::home_dir().map(|home| home.join(".kiro"))
}

/// Unix forwarding script path: `<kiro_home>/vlx-term/hook.sh`.
fn script_path(home: &Path) -> PathBuf {
    home.join("vlx-term").join("hook.sh")
}

/// PowerShell bridge written by earlier Windows versions, deleted on install.
fn legacy_ps1_path(home: &Path) -> PathBuf {
    home.join("vlx-term").join("hook.ps1")
}

/// Build the command string Kiro runs for one event against `target`: the script on Unix, the VelaTerm
/// executable on Windows.
///
/// POSIX single quoting stops the hook shell from re-evaluating paths containing spaces, `$`, backticks, or
/// quotes. Kiro does not document which shell runs hooks on Windows, so the native bridge command uses the
/// form cmd.exe, PowerShell, and POSIX shells all parse alike.
fn hook_command(target: &Path, event: &str) -> String {
    if cfg!(windows) {
        let args = format!("{BRIDGE_MARKER}{event}");
        super::hook_bridge::command(target, super::hook_bridge::Shell::Any, &args)
    } else {
        let path = target.to_string_lossy();
        format!("sh '{}' {event}", path.replace('\'', "'\\''"))
    }
}

/// Read the user's configured default agent name from `<kiro_home>/settings/cli.json`.
///
/// Returns None when the file is missing, unreadable, invalid, or already points at the shadow agent, which
/// would otherwise make the clone feed on itself.
fn default_agent_name(home: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(home.join("settings").join("cli.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let name = value.get("chat.defaultAgent")?.as_str()?.trim();
    if name.is_empty() || name == AGENT_NAME {
        return None;
    }
    Some(name.to_string())
}

/// Clone the user's default agent configuration as the base for the shadow agent.
///
/// Any problem (no default configured, file missing, invalid JSON, or not a JSON object) yields an empty base
/// so the shadow agent still carries our hooks and Kiro falls back to its built-in defaults for everything else.
fn base_config(home: &Path) -> serde_json::Map<String, serde_json::Value> {
    let empty = serde_json::Map::new();
    let Some(name) = default_agent_name(home) else {
        return empty;
    };
    let path = home.join("agents").join(format!("{name}.json"));
    let Ok(raw) = std::fs::read_to_string(path) else {
        return empty;
    };
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => empty,
    }
}

/// Merge VelaTerm's hook entries into a cloned configuration, preserving the user's own entries.
///
/// Entries previously written by VelaTerm are identified by [`MARKER`] and replaced rather than duplicated, so
/// re-installing after an upgrade converges instead of accumulating.
fn merge_hooks(base: &mut serde_json::Map<String, serde_json::Value>, target: &Path) {
    let mut hooks = match base.remove("hooks") {
        Some(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    for (trigger, event) in EVENTS {
        let mut entries: Vec<serde_json::Value> = hooks
            .remove(trigger)
            .and_then(|v| match v {
                serde_json::Value::Array(items) => Some(items),
                _ => None,
            })
            .unwrap_or_default()
            .into_iter()
            .filter(|item| {
                !item
                    .get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(is_ours)
            })
            .collect();
        entries.push(serde_json::json!({
            "command": hook_command(target, event),
            "timeoutMs": 5000,
        }));
        hooks.insert(trigger.to_string(), serde_json::Value::Array(entries));
    }
    base.insert("hooks".to_string(), serde_json::Value::Object(hooks));
}

/// Generate the shadow agent configuration as pretty-printed JSON for easy user inspection.
fn agent_json(home: &Path, target: &Path) -> String {
    let mut config = base_config(home);
    config.insert(
        "name".to_string(),
        serde_json::Value::String(AGENT_NAME.to_string()),
    );
    config.insert(
        "description".to_string(),
        serde_json::Value::String(
            "VelaTerm status bridge. Clone of your default agent plus observe-only lifecycle hooks."
                .to_string(),
        ),
    );
    merge_hooks(&mut config, target);
    serde_json::to_string_pretty(&serde_json::Value::Object(config))
        .expect("agent JSON serialization must succeed")
}

/// Write `content` only when it differs, so an unchanged install leaves mtime alone.
fn write_if_changed(path: &Path, content: &str) -> Result<(), String> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create Kiro configuration directory: {e}"))?;
    }
    std::fs::write(path, content).map_err(|e| format!("Failed to write {}: {e}", path.display()))
}

/// Install or refresh the forwarding bridge and the shadow agent, returning the agent name to pass to
/// `--agent`. Unix writes the forwarding script; Windows points the hooks at `exe` and removes the PowerShell
/// script earlier versions wrote.
///
/// Missing home directories or permission failures return Err for logging only. The caller then omits
/// `--agent`, so Kiro still launches with the user's own agent and simply reports no authoritative state.
pub fn install(exe: &Path) -> Result<String, String> {
    let home = kiro_home().ok_or("Failed to resolve Kiro home directory")?;
    install_at(&home, exe)
}

/// Like [`install`], with an explicit Kiro home so tests can run against a temporary directory.
fn install_at(home: &Path, exe: &Path) -> Result<String, String> {
    let target = if cfg!(windows) {
        let _ = std::fs::remove_file(legacy_ps1_path(home));
        exe.to_path_buf()
    } else {
        let script = script_path(home);
        write_if_changed(&script, SH_SCRIPT)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&script)
                .map_err(|e| format!("Failed to inspect Kiro hook script: {e}"))?
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&script, permissions)
                .map_err(|e| format!("Failed to make Kiro hook script executable: {e}"))?;
        }
        script
    };

    let agent = home.join("agents").join(format!("{AGENT_NAME}.json"));
    write_if_changed(&agent, &agent_json(home, &target))?;
    Ok(AGENT_NAME.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Isolated Kiro home under the system temp directory, recreated empty for each test.
    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vlx-kiro-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp home");
        dir
    }

    /// Executable path handed to `install_at`; only Windows embeds it in the hook commands.
    const EXE: &str = "/opt/VelaTerm/velaterm";

    /// Write a default agent config and point `chat.defaultAgent` at it.
    fn seed_default_agent(home: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(home.join("agents")).expect("agents dir");
        std::fs::write(home.join("agents").join(format!("{name}.json")), body).expect("agent file");
        std::fs::create_dir_all(home.join("settings")).expect("settings dir");
        std::fs::write(
            home.join("settings").join("cli.json"),
            format!("{{\"chat.defaultAgent\": \"{name}\"}}"),
        )
        .expect("settings file");
    }

    /// Read back the `e=` value baked into one trigger's first VelaTerm entry.
    fn event_of(value: &serde_json::Value, trigger: &str) -> String {
        let entries = value["hooks"][trigger]
            .as_array()
            .unwrap_or_else(|| panic!("missing trigger {trigger}"));
        let command = entries
            .iter()
            .find_map(|e| e["command"].as_str().filter(|c| is_ours(c)))
            .unwrap_or_else(|| panic!("no VelaTerm entry for {trigger}"));
        command
            .rsplit(' ')
            .next()
            .expect("command must end with the event")
            .to_string()
    }

    #[test]
    fn agent_json_maps_every_trigger() {
        let home = temp_home("triggers");
        let json = agent_json(&home, &script_path(&home));
        let value: serde_json::Value = serde_json::from_str(&json).expect("must be valid JSON");
        assert_eq!(value["name"], AGENT_NAME);
        assert_eq!(event_of(&value, "agentSpawn"), "boot");
        assert_eq!(event_of(&value, "userPromptSubmit"), "kiro_working");
        assert_eq!(event_of(&value, "preToolUse"), "kiro_working");
        assert_eq!(event_of(&value, "postToolUse"), "kiro_working");
        assert_eq!(event_of(&value, "stop"), "kiro_waiting");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn clone_preserves_user_settings_and_their_own_hooks() {
        let home = temp_home("clone");
        seed_default_agent(
            &home,
            "mine",
            r#"{"name":"mine","prompt":"be terse","tools":["fs_read"],
                "mcpServers":{"db":{"command":"dbmcp"}},
                "hooks":{"stop":[{"command":"echo mine"}]}}"#,
        );
        let json = agent_json(&home, &script_path(&home));
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        // The user's own settings survive the clone.
        assert_eq!(value["prompt"], "be terse");
        assert_eq!(value["tools"][0], "fs_read");
        assert_eq!(value["mcpServers"]["db"]["command"], "dbmcp");
        // Their own stop hook is kept alongside ours rather than replaced.
        let stop = value["hooks"]["stop"].as_array().expect("stop array");
        assert_eq!(stop.len(), 2);
        assert!(stop.iter().any(|e| e["command"] == "echo mine"));
        assert_eq!(event_of(&value, "stop"), "kiro_waiting");
        // The clone is renamed so `--agent vlx-term` resolves to it.
        assert_eq!(value["name"], AGENT_NAME);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn reinstall_replaces_our_entries_instead_of_duplicating() {
        let home = temp_home("dedupe");
        install_at(&home, Path::new(EXE)).expect("first install");
        install_at(&home, Path::new(EXE)).expect("second install");
        let raw = std::fs::read_to_string(home.join("agents").join("vlx-term.json")).expect("read");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
        for (trigger, _) in EVENTS {
            let entries = value["hooks"][trigger].as_array().expect("array");
            let ours = entries
                .iter()
                .filter(|e| e["command"].as_str().is_some_and(is_ours))
                .count();
            assert_eq!(ours, 1, "{trigger} must keep exactly one VelaTerm entry");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn invalid_or_missing_default_agent_degrades_to_hooks_only() {
        let home = temp_home("bad-default");
        seed_default_agent(&home, "broken", "{ not json");
        let json = agent_json(&home, &script_path(&home));
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(value["name"], AGENT_NAME);
        assert_eq!(event_of(&value, "stop"), "kiro_waiting");
        assert!(value.get("prompt").is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn default_agent_pointing_at_the_shadow_is_ignored() {
        // Without this guard the shadow agent would clone itself once the user set it as their default.
        let home = temp_home("self-clone");
        std::fs::create_dir_all(home.join("settings")).expect("settings dir");
        std::fs::write(
            home.join("settings").join("cli.json"),
            r#"{"chat.defaultAgent": "vlx-term"}"#,
        )
        .expect("settings file");
        assert!(default_agent_name(&home).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn hook_command_quotes_paths_with_spaces() {
        let command = hook_command(Path::new("/Users/a b/.kiro/vlx-term/hook.sh"), "kiro_working");
        if cfg!(windows) {
            // A nonexistent path has no 8.3 form, so the native bridge falls back to double quotes.
            assert_eq!(command, "\"/Users/a b/.kiro/vlx-term/hook.sh\" --agent-hook kiro kiro_working");
        } else {
            assert!(command.contains("'/Users/a b/.kiro/vlx-term/hook.sh'"));
        }
        assert!(command.ends_with(" kiro_working"));
    }

    #[test]
    fn script_never_writes_to_stdout() {
        // An observing hook must stay silent: Kiro reads stdout, and echoing a decision would bypass the
        // user's own permission prompts. The native bridge's Kiro contract is silent too (see hook_bridge).
        assert!(!SH_SCRIPT.contains("echo '{"));
    }

    #[test]
    fn earlier_windows_powershell_entries_are_recognized_as_ours() {
        assert!(is_ours(r#"powershell -NoProfile -ExecutionPolicy Bypass -File "C:\Users\a\.kiro\vlx-term\hook.ps1" boot"#));
        assert!(is_ours("C:/Users/a/AppData/Local/velaterm/velaterm.exe --agent-hook kiro kiro_waiting"));
        assert!(!is_ours("echo mine"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_install_calls_the_native_bridge_and_removes_the_script() {
        let home = temp_home("native");
        std::fs::create_dir_all(home.join("vlx-term")).expect("bridge dir");
        std::fs::write(legacy_ps1_path(&home), "# old bridge").expect("old script");
        let exe = home.join("velaterm.exe");
        install_at(&home, &exe).expect("install");
        assert!(!legacy_ps1_path(&home).exists(), "the earlier PowerShell bridge should be removed");
        let raw = std::fs::read_to_string(home.join("agents").join("vlx-term.json")).expect("read");
        assert!(!raw.to_ascii_lowercase().contains("powershell"), "{raw}");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
        assert_eq!(event_of(&value, "stop"), "kiro_waiting");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn install_is_idempotent() {
        let home = temp_home("idempotent");
        let name = install_at(&home, Path::new(EXE)).expect("first install");
        assert_eq!(name, AGENT_NAME);
        let agent = home.join("agents").join("vlx-term.json");
        let first = std::fs::metadata(&agent).expect("metadata").modified().ok();
        install_at(&home, Path::new(EXE)).expect("second install");
        let second = std::fs::metadata(&agent).expect("metadata").modified().ok();
        assert_eq!(first, second, "unchanged content must not rewrite the file");
        let _ = std::fs::remove_dir_all(&home);
    }
}
