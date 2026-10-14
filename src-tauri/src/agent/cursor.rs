//! Generation and installation of status-bridge hooks for Cursor CLI (`cursor-agent`).
//!
//! Cursor hooks support only fixed configuration files at project, user, or enterprise locations. There
//! is no diskless injection equivalent to Claude `--settings` or OpenCode `OPENCODE_CONFIG_CONTENT`, and
//! three tested `--plugin-dir` layouts failed to activate hooks. To avoid modifying repositories, merge
//! into the user-level `~/.cursor/hooks.json` with the user's consent:
//!
//! - Unix uses an explicitly invoked POSIX shim that launches the hidden `--cursor-hook` entry.
//! - Windows calls the VelaTerm executable's hidden `--cursor-hook` entry directly. Earlier versions used a
//!   hidden PowerShell launcher, which triggered antivirus alerts; installation replaces those entries.
//! - Merge only VelaTerm commands into `~/.cursor/hooks.json`, preserving user entries and refusing
//!   malformed configuration. Recognize both separators when migrating legacy script commands.
//! - Ports, session IDs, and tokens remain in the injected environment. The Unix shim also reads the executable
//!   path from it; the Windows command names the executable directly and is rewritten when the path changes.
//! - The native hook bounds stdin and HTTP IO and always returns `{}` with a successful exit.
//! - Installation is lazy and occurs only when the user actually launches a Cursor session.
//!
//! Event-to-state mapping, verified in interactive CLI mode. Payloads include the conversation/session
//! chat ID used by `--resume`:
//! - `beforeSubmitPrompt` with submitted prompt text → working.
//! - `stop` on completed, aborted, or error → waiting.
//! - `sessionStart` → `boot`, which leaves status unchanged but lets the server capture a resume anchor.
//!
//! Permission-decision hooks such as `beforeShellExecution` and `beforeMCPExecution` are intentionally
//! omitted because their stdout controls approval and an observational `{}` could interfere. Cursor
//! therefore remains working while awaiting permission, with neutral screen detection as fallback.

use std::path::{Path, PathBuf};

mod hook;
pub use hook::run;

/// Legacy marker retained for upgrades from script-based hooks.
const MARKER: &str = "vlx-term/hook.sh";
const NATIVE_MARKER: &str = "velaterm-cursor-hook-v1";
/// Hidden entry argument that the Windows command passes directly to the VelaTerm executable.
const NATIVE_ARG: &str = "--cursor-hook ";
/// Prefix of the PowerShell launcher written by earlier Windows versions, recognized only for removal.
const WINDOWS_COMMAND_PREFIX: &str = "powershell.exe -NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -EncodedCommand ";

/// Explicit `sh` invocation avoids executable-bit and script-association dependencies.
const HOOK_SCRIPT: &str = r#"#!/bin/sh
# VelaTerm status bridge. Dynamic values belong to the managed agent environment.
if [ -n "$VLX_EXE" ] && [ -x "$VLX_EXE" ]; then
  "$VLX_EXE" --cursor-hook "$1" 2>/dev/null || printf '{}\n'
else
  printf '{}\n'
fi
exit 0
"#;

fn is_vlx_command(command: &str) -> bool {
    let normalized = command.replace('\\', "/");
    if normalized.contains(MARKER) || command.contains(NATIVE_ARG) { return true; }
    // Recognize our namespace across launcher updates, without matching unrelated PowerShell hooks.
    use base64::Engine;
    let Some(encoded) = command.strip_prefix(WINDOWS_COMMAND_PREFIX) else { return false; };
    if encoded.len() > 16384 { return false; }
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) else { return false; };
    if bytes.len() % 2 != 0 { return false; }
    let units: Vec<_> = bytes.chunks_exact(2).map(|v| u16::from_le_bytes([v[0], v[1]])).collect();
    String::from_utf16(&units).is_ok_and(|code| code.starts_with(&format!("# {NATIVE_MARKER}\n")))
}

/// Cursor configuration root: `~/.cursor`.
fn cursor_home() -> Option<PathBuf> {
    crate::host::home_dir().map(|h| h.join(".cursor"))
}

/// Hook script path: `<cursor_home>/vlx-term/hook.sh`, isolated from Cursor-owned files.
fn hook_script_path() -> Option<PathBuf> {
    cursor_home().map(|h| h.join("vlx-term").join("hook.sh"))
}

/// User-level global hooks configuration: `<cursor_home>/hooks.json`.
fn hooks_json_path() -> Option<PathBuf> {
    cursor_home().map(|h| h.join("hooks.json"))
}

/// `(Cursor event name, status event)` pairs to register.
const EVENTS: [(&str, &str); 3] = [
    ("sessionStart", "boot"),
    ("beforeSubmitPrompt", "working"),
    ("stop", "waiting"),
];

/// Entry for `target`: the POSIX shim on Unix, the VelaTerm executable on Windows.
///
/// Unix paths are single-quoted for POSIX shells; apostrophes are escaped without expansion. Cursor does not
/// document which shell runs hooks on Windows, so the command uses the form cmd.exe, PowerShell, and POSIX
/// shells all parse alike.
fn hook_entry(target: &Path, event: &str) -> serde_json::Value {
    let command = if cfg!(windows) {
        super::hook_bridge::command(target, super::hook_bridge::Shell::Any, &format!("{NATIVE_ARG}{event}"))
    } else {
        format!("sh '{}' {event}", target.to_string_lossy().replace('\'', "'\"'\"'"))
    };
    serde_json::json!({ "command": command, "timeout": 6 })
}

/// Merge VelaTerm entries into the hooks.json root object in place.
///
/// Create the root, `hooks`, and event arrays as needed; return Err without writing when an existing
/// value has another type. Within each event, replace old commands containing `vlx-term/hook.sh` with
/// the current entry while preserving user-owned entries unchanged.
fn merge_into(root: &mut serde_json::Value, target: &Path) -> Result<(), String> {
    let obj = root
        .as_object_mut()
        .ok_or("hooks.json root is not a JSON object")?;
    // Cursor requires a top-level version; preserve an existing value and add 1 only when missing.
    obj.entry("version").or_insert(serde_json::Value::from(1));
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("hooks.json field 'hooks' is not an object")?;
    for (event, status) in EVENTS {
        let arr = hooks
            .entry(event)
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("hooks.json field hooks.{event} is not an array"))?;
        arr.retain(|e| {
            !e.get("command")
                .and_then(|c| c.as_str())
                .is_some_and(|c| is_vlx_command(c))
        });
        arr.push(hook_entry(target, status));
    }
    Ok(())
}

/// Install the bridge, merge `~/.cursor/hooks.json`, and return the configuration path. Unix writes the shim
/// script; Windows points the entries at `exe`.
///
/// Idempotent: preserve mtime when script and configuration match. Missing home, permissions, or malformed
/// configuration returns Err for logging; Cursor still starts with neutral screen detection only.
pub fn install(exe: &Path) -> Result<PathBuf, String> {
    let script = hook_script_path().ok_or("Failed to resolve user home directory")?;
    let hooks = hooks_json_path().ok_or("Failed to resolve user home directory")?;
    install_at(&script, &hooks, exe)
}

/// As `install`, with explicit target paths for tests.
fn install_at(script: &Path, hooks_json: &Path, exe: &Path) -> Result<PathBuf, String> {
    // Validate before changing either file. Only a missing file means a fresh configuration.
    let existing = match std::fs::read_to_string(hooks_json) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("Failed to read hooks.json; not written: {e}")),
    };
    let mut root: serde_json::Value = match &existing {
        Some(text) => serde_json::from_str(text).map_err(|_| {
            "~/.cursor/hooks.json is not valid JSON; not written (please check manually)"
                .to_string()
        })?,
        None => serde_json::json!({}),
    };
    merge_into(&mut root, if cfg!(windows) { exe } else { script })?;

    if !cfg!(windows) {
        let script_current = std::fs::read_to_string(script)
            .map(|s| s == HOOK_SCRIPT)
            .unwrap_or(false);
        if !script_current {
            if let Some(parent) = script.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create script directory: {e}"))?;
            }
            std::fs::write(script, HOOK_SCRIPT)
                .map_err(|e| format!("Failed to write hook script: {e}"))?;
        }
    }

    let content = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("Failed to serialize hooks.json: {e}"))?;
    if existing.as_deref() != Some(content.as_str()) {
        if let Some(parent) = hooks_json.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create .cursor directory: {e}"))?;
        }
        std::fs::write(hooks_json, &content)
            .map_err(|e| format!("Failed to write hooks.json: {e}"))?;
    }
    Ok(hooks_json.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vlx-cursor-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A launcher command as earlier Windows versions wrote it.
    fn legacy_windows_command() -> String {
        use base64::Engine;
        let code = format!("# {NATIVE_MARKER}\n[Console]::Out.WriteLine('{{}}')\n");
        let bytes: Vec<u8> = code.encode_utf16().flat_map(u16::to_le_bytes).collect();
        format!("{WINDOWS_COMMAND_PREFIX}{}", base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    /// Executable path handed to `install_at`; only Windows embeds it in the hook commands.
    const EXE: &str = "/opt/VelaTerm/velaterm";

    /// What the entries point at on the current platform.
    fn target(script: &Path) -> PathBuf {
        if cfg!(windows) { PathBuf::from(EXE) } else { script.to_path_buf() }
    }

    /// Entries VelaTerm registers for one event on the current platform.
    fn registered(script: &Path, status: &str) -> Vec<serde_json::Value> {
        vec![hook_entry(&target(script), status)]
    }

    #[test]
    fn fresh_install_writes_script_and_hooks() {
        let dir = tmp_dir("fresh");
        let script = dir.join("vlx-term/hook.sh");
        let hooks = dir.join("hooks.json");

        install_at(&script, &hooks, Path::new(EXE)).expect("installation should succeed");

        if cfg!(windows) {
            assert!(!script.exists(), "Windows writes no shim script");
            let raw = std::fs::read_to_string(&hooks).unwrap();
            assert!(!raw.to_ascii_lowercase().contains("powershell"), "{raw}");
            let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
            for (event, status) in EVENTS {
                assert_eq!(v["hooks"][event][0], hook_entry(Path::new(EXE), status));
            }
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        assert_eq!(std::fs::read_to_string(&script).unwrap(), HOOK_SCRIPT);

        // hooks.json has version 1 and one script command with event name for each of three events.
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&hooks).unwrap()).unwrap();
        assert_eq!(v["version"], 1);
        for (event, status) in EVENTS {
            assert_eq!(v["hooks"][event][0], hook_entry(&script, status));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn install_is_idempotent() {
        let dir = tmp_dir("idem");
        let script = dir.join("vlx-term/hook.sh");
        let hooks = dir.join("hooks.json");

        install_at(&script, &hooks, Path::new(EXE)).unwrap();
        let mtime1 = std::fs::metadata(&hooks).unwrap().modified().unwrap();
        let smtime1 = std::fs::metadata(&script).ok().and_then(|m| m.modified().ok());
        install_at(&script, &hooks, Path::new(EXE)).unwrap();
        assert_eq!(
            std::fs::metadata(&hooks).unwrap().modified().unwrap(),
            mtime1,
            "hooks.json should not be rewritten when the contents are identical"
        );
        assert_eq!(
            std::fs::metadata(&script).ok().and_then(|m| m.modified().ok()),
            smtime1,
            "the script should not be rewritten when the contents are identical"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_preserves_user_entries_and_replaces_stale_vlx_entries() {
        let dir = tmp_dir("merge");
        let script = dir.join("vlx-term/hook.sh");
        let hooks = dir.join("hooks.json");

        // Seed user hooks, one old VelaTerm entry, and a user-owned top-level field.
        std::fs::write(
            &hooks,
            r#"{
  "version": 1,
  "custom": "keep-me",
  "hooks": {
    "stop": [
      { "command": "/Users/me/my-own-hook.sh" },
      { "command": "/old/path/vlx-term/hook.sh stale-event" }
    ],
    "afterFileEdit": [ { "command": "/Users/me/format.sh" } ]
  }
}"#,
        )
        .unwrap();

        install_at(&script, &hooks, Path::new(EXE)).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&hooks).unwrap()).unwrap();

        // User content remains unchanged.
        assert_eq!(v["custom"], "keep-me");
        assert_eq!(
            v["hooks"]["afterFileEdit"][0]["command"],
            "/Users/me/format.sh"
        );
        let stop = v["hooks"]["stop"].as_array().unwrap();
        assert!(
            stop.iter()
                .any(|e| e["command"] == "/Users/me/my-own-hook.sh"),
            "the user's own stop entries should be preserved"
        );
        // The old entry is replaced by the current registration.
        let vlx: Vec<_> = stop
            .iter()
            .filter(|e| is_vlx_command(e["command"].as_str().unwrap()))
            .cloned()
            .collect();
        assert_eq!(vlx, registered(&script, "waiting"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_refuses_to_clobber_invalid_json() {
        let dir = tmp_dir("invalid");
        let script = dir.join("vlx-term/hook.sh");
        let hooks = dir.join("hooks.json");
        std::fs::write(&hooks, "{ not json").unwrap();

        assert!(install_at(&script, &hooks, Path::new(EXE)).is_err(), "invalid JSON should be refused rather than written");
        assert_eq!(
            std::fs::read_to_string(&hooks).unwrap(),
            "{ not json",
            "the original file should be left untouched"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_refuses_malformed_structure() {
        let dir = tmp_dir("malform");
        let script = dir.join("vlx-term/hook.sh");
        let hooks = dir.join("hooks.json");
        // Reject a string-valued hooks field without changing the file.
        let original = r#"{"version":1,"hooks":"oops"}"#;
        std::fs::write(&hooks, original).unwrap();

        assert!(install_at(&script, &hooks, Path::new(EXE)).is_err());
        assert_eq!(std::fs::read_to_string(&hooks).unwrap(), original);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_respects_existing_version() {
        // Preserve the user's version 2 rather than resetting it to 1.
        let mut root = serde_json::json!({ "version": 2 });
        merge_into(&mut root, &target(Path::new("/x/vlx-term/hook.sh"))).unwrap();
        assert_eq!(root["version"], 2);
    }

    #[test]
    fn migration_removes_mixed_separator_duplicates_and_preserves_user_fields() {
        let user = serde_json::json!({ "command": "my-hook --keep", "timeout": 42, "custom": true });
        let old = [
            r"C:\Users\with space\.cursor\vlx-term\hook.sh working",
            "sh '/old/.cursor/vlx-term/hook.sh' working",
            "bash C:/old/vlx-term/hook.sh working",
        ];
        let mut root = serde_json::json!({"custom": {"keep": 1}, "version": 2, "hooks": {"afterFileEdit": [user.clone()]}});
        for (event, _) in EVENTS {
            let mut entries: Vec<_> = old.iter().map(|command| serde_json::json!({"command": command})).collect();
            entries.push(user.clone());
            entries.push(serde_json::json!({"command": legacy_windows_command()}));
            root["hooks"][event] = entries.into();
        }
        let script = Path::new("/new/vlx-term/hook.sh");
        merge_into(&mut root, &target(script)).unwrap();
        let once = root.clone();
        merge_into(&mut root, &target(script)).unwrap();
        assert_eq!(root, once);
        assert_eq!(root["custom"]["keep"], 1);
        assert_eq!(root["version"], 2);
        assert_eq!(root["hooks"]["afterFileEdit"], serde_json::json!([user.clone()]));
        for (event, status) in EVENTS {
            let mut expected = vec![user.clone()];
            expected.extend(registered(script, status));
            assert_eq!(root["hooks"][event], serde_json::Value::from(expected));
        }
    }

    #[test]
    fn invalid_utf8_and_unreadable_config_are_not_replaced() {
        let dir = tmp_dir("non-utf8");
        let hooks = dir.join("hooks.json");
        let script = dir.join("vlx-term/hook.sh");
        let invalid = b"{\"custom\":\"\xff\"}";
        std::fs::write(&hooks, invalid).unwrap();
        assert!(install_at(&script, &hooks, Path::new(EXE)).is_err());
        assert_eq!(std::fs::read(&hooks).unwrap(), invalid);
        assert!(!script.exists());
        std::fs::remove_file(&hooks).unwrap();
        std::fs::create_dir(&hooks).unwrap();
        assert!(install_at(&script, &hooks, Path::new(EXE)).is_err());
        assert!(hooks.is_dir());
        assert!(!script.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn malformed_event_array_preserves_both_files() {
        let dir = tmp_dir("bad-event");
        let hooks = dir.join("hooks.json");
        let script = dir.join("hook.sh");
        let original = r#"{"hooks":{"stop":{},"sessionStart":[]},"custom":1}"#;
        std::fs::write(&hooks, original).unwrap();
        std::fs::write(&script, "old script").unwrap();
        assert!(install_at(&script, &hooks, Path::new(EXE)).is_err());
        assert_eq!(std::fs::read_to_string(hooks).unwrap(), original);
        assert_eq!(std::fs::read_to_string(script).unwrap(), "old script");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn native_windows_command_is_recognized_for_replacement() {
        assert!(is_vlx_command("C:/Users/a/AppData/Local/velaterm/velaterm.exe --cursor-hook waiting"));
        assert!(is_vlx_command(&legacy_windows_command()));
        assert!(!is_vlx_command("my-hook --keep"));
    }

    #[cfg(unix)]
    #[test]
    fn posix_launcher_executes_literal_special_paths() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};
        let dir = tmp_dir("special-path").join("space & % ! ^ ' ( ) $ ` 中文");
        let script = dir.join("vlx-term/hook.sh");
        let hooks = dir.join("hooks.json");
        install_at(&script, &hooks, Path::new(EXE)).unwrap();
        let exe = dir.join("fake executable");
        std::fs::write(&exe, "#!/bin/sh\n[ \"$1\" = --cursor-hook ] && [ \"$2\" = working ] || exit 1\nprintf '{\"native\":true}\\n'\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        let entry = hook_entry(&script, "working");
        let output = Command::new("sh").arg("-c").arg(entry["command"].as_str().unwrap())
            .env("VLX_EXE", &exe).stdin(Stdio::null()).output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"{\"native\":true}\n");
        assert!(output.stderr.is_empty());
        std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

}
