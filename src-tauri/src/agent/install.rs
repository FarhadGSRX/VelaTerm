//! Agent installation guidance as the single source of truth for each host platform.
//!
//! When the interactive-shell launch guard reports `not found on PATH`, AgentInstallCard obtains its
//! recommended command, documentation link, and authentication guidance here. Installation changes need
//! only this module, following the same pattern as `inject::permission_flag`.
//!
//! Commands branch by host OS and prefer native installers without Node, falling back to global npm with
//! `needs_node=true`. They run directly in the agent session's PowerShell on Windows or login shell on Unix.
//!
//! Only local agent types have guidance; terminal and browser sessions do not.

/// Platform-specific agent installation guidance serialized to frontend camelCase.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRecipe {
    /// Agent display name such as Claude Code or Codex.
    pub label: String,
    /// Executable command name.
    pub bin: String,
    /// Recommended command executable directly in the session shell.
    pub command: String,
    /// Whether installation requires Node/npm, allowing the frontend to warn accordingly.
    pub needs_node: bool,
    /// Official installation documentation URL.
    pub docs_url: String,
    /// English one-line authentication step still required after installing the binary.
    pub auth_hint: String,
}

/// Return platform guidance for a frontend AgentKind, or None for unknown/unsupported types.
pub fn install_recipe(agent: &str) -> Option<InstallRecipe> {
    // Select commands/installers from the compile-time host OS.
    let win = cfg!(target_os = "windows");
    let r = match agent {
        "claude" => InstallRecipe {
            label: "Claude Code".into(),
            bin: "claude".into(),
            // Official native installer: PowerShell on Windows or curl on Unix, with no Node dependency.
            command: if win {
                "irm https://claude.ai/install.ps1 | iex".into()
            } else {
                "curl -fsSL https://claude.ai/install.sh | bash".into()
            },
            needs_node: false,
            docs_url: "https://code.claude.com/docs/en/setup".into(),
            auth_hint: "Run `claude` and log in via the browser when prompted.".into(),
        },
        "codex" => InstallRecipe {
            label: "Codex".into(),
            bin: "codex".into(),
            // No standalone first-party installer. Use scoped @openai/codex; unscoped codex is unrelated.
            command: "npm install -g @openai/codex".into(),
            needs_node: true,
            docs_url: "https://developers.openai.com/codex/cli".into(),
            auth_hint: "Run `codex` and sign in with your ChatGPT account or an API key.".into(),
        },
        "opencode" => InstallRecipe {
            label: "OpenCode".into(),
            bin: "opencode".into(),
            // Unix has a native curl installer; Windows falls back to global npm. The package's postinstall is what
            // creates bin/opencode.exe, and newer npm skips install scripts unless allowed, so allow it explicitly
            // (older npm only warns about the unknown flag and runs scripts anyway).
            command: if win {
                "npm install -g --allow-scripts=opencode-ai opencode-ai".into()
            } else {
                "curl -fsSL https://opencode.ai/install | bash".into()
            },
            needs_node: win,
            docs_url: "https://opencode.ai/docs/".into(),
            auth_hint: "Run `opencode`, then `/login` (or set a provider API key).".into(),
        },
        "copilot" => InstallRecipe {
            label: "GitHub Copilot CLI".into(),
            bin: "copilot".into(),
            // Global npm only, requiring Node 22+ and an existing Copilot subscription.
            command: "npm install -g @github/copilot".into(),
            needs_node: true,
            docs_url: "https://docs.github.com/copilot/how-tos/set-up/install-copilot-cli".into(),
            auth_hint: "Requires Node 22+. Run `copilot`, then `/login` with your GitHub account."
                .into(),
        },
        "cursor" => InstallRecipe {
            label: "Cursor CLI".into(),
            bin: "cursor-agent".into(),
            // Official Node-free installer: Windows PowerShell with win32 or Unix curl.
            command: if win {
                "irm 'https://cursor.com/install?win32=true' | iex".into()
            } else {
                "curl https://cursor.com/install -fsS | bash".into()
            },
            needs_node: false,
            docs_url: "https://cursor.com/docs/cli/installation".into(),
            auth_hint: "Run `cursor-agent login` to authenticate.".into(),
        },
        "cline" => InstallRecipe {
            label: "Cline".into(),
            bin: "cline".into(),
            // Global npm only with the same Node-dependent command on Windows and Unix.
            command: "npm install -g cline".into(),
            needs_node: true,
            docs_url: "https://docs.cline.bot/cli/installation".into(),
            auth_hint: "Run `cline auth` to configure your provider and API key.".into(),
        },
        "pi" => InstallRecipe {
            label: "Pi".into(),
            bin: "pi".into(),
            // Global npm only; --ignore-scripts avoids running dependency packaging scripts.
            command: "npm install -g --ignore-scripts @earendil-works/pi-coding-agent".into(),
            needs_node: true,
            docs_url: "https://pi.dev/".into(),
            auth_hint:
                "Run `pi`, then `/login` (Claude/ChatGPT/Copilot) or set a provider API key.".into(),
        },
        "omp" => InstallRecipe {
            label: "OMP".into(),
            bin: "omp".into(),
            // Official Node-free installer; it drops a single prebuilt binary into ~/.local/bin.
            command: if win {
                "irm https://omp.sh/install.ps1 | iex".into()
            } else {
                "curl -fsSL https://omp.sh/install | sh".into()
            },
            needs_node: false,
            docs_url: "https://omp.sh/".into(),
            auth_hint: "Run `omp`, complete the setup, or use `/login` inside a session.".into(),
        },
        "antigravity" => InstallRecipe {
            label: "Antigravity CLI".into(),
            // The executable is `agy`, matching inject.rs, not `antigravity`.
            bin: "agy".into(),
            // Official Node-free PowerShell/curl installer; `agy install` can configure PATH afterward.
            command: if win {
                "irm https://antigravity.google/cli/install.ps1 | iex".into()
            } else {
                "curl -fsSL https://antigravity.google/cli/install.sh | bash".into()
            },
            needs_node: false,
            docs_url: "https://antigravity.google/docs/cli-overview".into(),
            auth_hint: "Run `agy` and sign in with your Google account when prompted.".into(),
        },
        "crush" => InstallRecipe {
            label: "Crush".into(),
            bin: "crush".into(),
            // macOS uses the official Homebrew tap; Linux/Windows use global npm, whose postinstall downloads the Go
            // binary. Allow that script so the binary arrives during install instead of on the first launch.
            command: if cfg!(target_os = "macos") {
                "brew install charmbracelet/tap/crush".into()
            } else {
                "npm install -g --allow-scripts=@charmland/crush @charmland/crush".into()
            },
            needs_node: !cfg!(target_os = "macos"),
            docs_url: "https://github.com/charmbracelet/crush".into(),
            auth_hint:
                "Run `crush` and pick a provider in onboarding (sign in or set a provider API key)."
                    .into(),
        },
        "kimi" => InstallRecipe {
            label: "Kimi Code (K3)".into(),
            bin: "kimi".into(),
            command: if win {
                "irm https://code.kimi.com/kimi-code/install.ps1 | iex".into()
            } else {
                "curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash".into()
            },
            needs_node: false,
            docs_url: "https://www.kimi.com/code/docs/en/kimi-code-cli/guides/getting-started.html"
                .into(),
            auth_hint:
                "Run `kimi`, then `/login` to sign in with Kimi Code or configure an API key."
                    .into(),
        },
        "kiro" => InstallRecipe {
            // The official installer is a POSIX shell script. Windows support is unconfirmed and VelaTerm's
            // Windows sessions default to PowerShell, so hide one-click installation there while keeping the
            // documentation link and any manually configured executable path.
            label: "Kiro".into(),
            bin: "kiro-cli".into(),
            command: if win {
                String::new()
            } else {
                "curl -fsSL https://cli.kiro.dev/install | bash".into()
            },
            needs_node: false,
            docs_url: "https://kiro.dev/docs/cli/".into(),
            auth_hint: if win {
                "The Kiro CLI installer targets macOS and Linux. Install it in a supported environment or set a `kiro-cli` path in Settings > Agents.".into()
            } else {
                "Run `kiro-cli` and complete the sign-in prompt; installing the binary alone does not authenticate it.".into()
            },
        },
        "grok" => InstallRecipe {
            label: "Grok Build (Grok 4.5)".into(),
            bin: "grok".into(),
            // Windows uses global npm; allow its postinstall, which places grok.exe under ~/.grok/bin.
            command: if win {
                "npm install -g --allow-scripts=@xai-official/grok @xai-official/grok".into()
            } else {
                "curl -fsSL https://x.ai/cli/install.sh | bash".into()
            },
            needs_node: win,
            docs_url: "https://docs.x.ai/build/overview".into(),
            auth_hint:
                "Run `grok login` to sign in, or set `XAI_API_KEY`; use `grok models` to list available models."
                    .into(),
        },
        "zoo" => {
            // Zoo Code currently reuses Roo CLI and `roo`. Its installer lacks Windows support and there is
            // no public npm package, so return an empty Windows command to hide one-click installation while
            // retaining documentation and allowing a manually built roo.exe path.
            let unsupported = win || cfg!(all(target_os = "macos", target_arch = "x86_64"));
            InstallRecipe {
                label: "Zoo Code".into(),
                bin: "roo".into(),
                command: if unsupported {
                    String::new()
                } else {
                    "curl -fsSL https://raw.githubusercontent.com/RooCodeInc/Roo-Code/main/apps/cli/install.sh | sh"
                        .into()
                },
                needs_node: true,
                docs_url: "https://docs.zoocode.dev/update-notes/v3.39".into(),
                auth_hint: if unsupported {
                    "Zoo Code CLI currently ships for macOS Apple Silicon and Linux x64/ARM64. Use a supported environment or set a manually built `roo` path in Settings > Agents.".into()
                } else {
                    "Run `roo` with a provider API key (for example `OPENROUTER_API_KEY`) or pass provider/model launch arguments.".into()
                },
            }
        }
        _ => return None,
    };
    Some(r)
}

/// Detect an installed executable only at known locations produced by the recommended command. Stat each
/// candidate, require Unix executability, return an absolute match, and never guess.
///
/// One-click installers often modify a profile that the current session has not reloaded. Retry Launch uses
/// this result to fill an empty executable-path setting so the next launch uses the absolute path.
///
/// A generated Windows command wrapper whose payload is gone is not a match: that state comes from a failed
/// or interrupted install, and handing out the wrapper only produces an opaque cmd.exe path error.
///
/// Strategies mirror install_recipe: native installers stat fixed locations; global npm installations query
/// `npm prefix -g`. Unix uses a login shell for profile/nvm/fnm accuracy; Windows uses cmd /C.
pub fn locate_installed_bin(agent: &str) -> Option<String> {
    let (bin, mut candidates, use_npm) = agent_layout(agent)?;
    if use_npm {
        push_npm_candidates(&mut candidates, bin);
    }
    candidates
        .into_iter()
        .find(|p| super::executable::is_executable_file(p))
        .map(|p| p.to_string_lossy().to_string())
}

/// Whether the recommended npm installation exists in name only: the generated command wrapper is on
/// disk but the program it forwards to is gone.
///
/// A failed, interrupted, or quarantined install leaves exactly that state, because npm removes the
/// package while keeping the wrapper it created. Launching that wrapper prints only cmd.exe's bare path
/// error, so the launch path reports the agent as missing and shows the installation guidance instead.
pub fn dangling_npm_install(agent: &str) -> bool {
    let Some((bin, _, true)) = agent_layout(agent) else {
        return false;
    };
    let Some(prefix) = npm_global_prefix() else {
        return false;
    };
    let shim = npm_bin_candidate(&prefix, bin, cfg!(target_os = "windows"));
    shim.is_file() && !super::executable::is_executable_file(&shim)
}

/// Install layout for one agent on this host: the command name, fixed-location candidates in priority
/// order, and whether the recommended installation is a global npm package. npm-only types leave the
/// fixed list empty; their candidate comes from `npm_bin_candidate`.
fn agent_layout(agent: &str) -> Option<(&'static str, Vec<std::path::PathBuf>, bool)> {
    let win = cfg!(target_os = "windows");
    let home = crate::host::home_dir()?;
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    let layout = match agent {
        // Claude's official installer targets ~/.local/bin.
        "claude" => {
            candidates.push(
                home.join(".local")
                    .join("bin")
                    .join(exe_name("claude", win)),
            );
            ("claude", false)
        }
        // Cursor's Unix installer links ~/.local/bin/cursor-agent. The Windows installer copies its
        // cursor-agent* launchers into %LOCALAPPDATA%\cursor-agent instead.
        "cursor" => {
            if win {
                let root = local_app_data(&home).join("cursor-agent");
                candidates.push(root.join("cursor-agent.exe"));
                candidates.push(root.join("cursor-agent.cmd"));
            }
            candidates.push(
                home.join(".local")
                    .join("bin")
                    .join(exe_name("cursor-agent", win)),
            );
            ("cursor-agent", false)
        }
        // OpenCode's Unix script uses ~/.opencode/bin or, in some versions, ~/.local/bin; Windows uses npm.
        "opencode" => {
            if !win {
                candidates.push(home.join(".opencode").join("bin").join("opencode"));
                candidates.push(home.join(".local").join("bin").join("opencode"));
            }
            ("opencode", win)
        }
        "codex" => ("codex", true),
        "copilot" => ("copilot", true),
        // Cline is global npm only, so infer it from the npm prefix.
        "cline" => ("cline", true),
        // Pi has no native installer and uses global npm only.
        "pi" => ("pi", true),
        // OMP's official installer writes a single binary to PI_INSTALL_DIR when set, otherwise ~/.local/bin/omp
        // on Unix and %LOCALAPPDATA%\omp\omp.exe on Windows, so stat those paths rather than an npm prefix.
        "omp" => {
            if let Some(dir) = std::env::var_os("PI_INSTALL_DIR").filter(|d| !d.is_empty()) {
                candidates.push(std::path::PathBuf::from(dir).join(exe_name("omp", win)));
            }
            if win {
                candidates.push(local_app_data(&home).join("omp").join("omp.exe"));
            }
            candidates.push(home.join(".local").join("bin").join(exe_name("omp", win)));
            ("omp", false)
        }
        // Grok's native installer targets ~/.grok/bin; also probe ~/.local/bin for manually linked installs.
        // Windows uses the official npm fallback.
        "grok" => {
            if !win {
                candidates.push(home.join(".grok").join("bin").join("grok"));
                candidates.push(home.join(".local").join("bin").join("grok"));
            }
            ("grok", win)
        }
        // Antigravity's installer writes ~/.local/bin/agy on Unix and %LOCALAPPDATA%\agy\bin\agy.exe on Windows;
        // otherwise fall back to command-name launch, `agy install`, or a manual setting.
        "antigravity" => {
            if win {
                candidates.push(local_app_data(&home).join("agy").join("bin").join("agy.exe"));
            }
            candidates.push(home.join(".local").join("bin").join(exe_name("agy", win)));
            ("agy", false)
        }
        // Crush uses Homebrew prefixes on macOS and global npm prefixes on Linux/Windows.
        "crush" => {
            if cfg!(target_os = "macos") {
                candidates.push(std::path::PathBuf::from("/opt/homebrew/bin/crush"));
                candidates.push(std::path::PathBuf::from("/usr/local/bin/crush"));
                ("crush", false)
            } else {
                ("crush", true)
            }
        }
        // Kimi installers have used ~/.kimi-code/bin and ~/.local/bin; prefer KIMI_CODE_HOME/bin when set.
        "kimi" => {
            if let Some(root) = std::env::var_os("KIMI_CODE_HOME") {
                candidates.push(
                    std::path::PathBuf::from(root)
                        .join("bin")
                        .join(exe_name("kimi", win)),
                );
            }
            candidates.push(
                home.join(".kimi-code")
                    .join("bin")
                    .join(exe_name("kimi", win)),
            );
            candidates.push(home.join(".local").join("bin").join(exe_name("kimi", win)));
            ("kimi", false)
        }
        // The Kiro installer drops the binary in ~/.local/bin; honor KIRO_HOME/bin when it is set.
        "kiro" => {
            if let Some(root) = std::env::var_os("KIRO_HOME") {
                candidates.push(
                    std::path::PathBuf::from(root)
                        .join("bin")
                        .join(exe_name("kiro-cli", win)),
                );
            }
            candidates.push(
                home.join(".local")
                    .join("bin")
                    .join(exe_name("kiro-cli", win)),
            );
            candidates.push(
                home.join(".kiro")
                    .join("bin")
                    .join(exe_name("kiro-cli", win)),
            );
            ("kiro-cli", false)
        }
        // Zoo/Roo installs at ~/.roo/cli/bin/roo with a ~/.local/bin symlink, so probe both.
        "zoo" => {
            candidates.push(home.join(".local").join("bin").join(exe_name("roo", win)));
            candidates.push(
                home.join(".roo")
                    .join("cli")
                    .join("bin")
                    .join(exe_name("roo", win)),
            );
            ("roo", false)
        }
        _ => return None,
    };
    Some((layout.0, candidates, layout.1))
}

/// Adds the global npm candidates for one command: first the real program a generated wrapper forwards
/// to, then the wrapper itself as the fallback.
///
/// Launching the payload directly avoids cmd.exe re-parsing the wrapper's arguments, which can mangle
/// structured values such as JSON.
fn push_npm_candidates(candidates: &mut Vec<std::path::PathBuf>, bin: &str) {
    let Some(prefix) = npm_global_prefix() else {
        return;
    };
    let shim = npm_bin_candidate(&prefix, bin, cfg!(target_os = "windows"));
    if let Some(exe) = super::executable::shim_payload_exe(&shim) {
        candidates.push(exe);
    }
    candidates.push(shim);
}

/// Executable filename with `.exe` for native Windows installers and a bare name on Unix.
fn exe_name(name: &str, win: bool) -> String {
    if win {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// `%LOCALAPPDATA%`, falling back to `<home>\AppData\Local` when the variable is missing.
fn local_app_data(home: &std::path::Path) -> std::path::PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join("AppData").join("Local"))
}

/// Global npm executable path: `<prefix>/bin/<name>` on Unix or `<prefix>\<name>.cmd` on Windows.
fn npm_bin_candidate(prefix: &std::path::Path, bin: &str, win: bool) -> std::path::PathBuf {
    if win {
        prefix.join(format!("{bin}.cmd"))
    } else {
        prefix.join("bin").join(bin)
    }
}

/// Query `npm prefix -g`. Unix runs `$SHELL -lc`, falling back through zsh/bash/sh, so GUI launches load
/// profile-managed npm and the correct nvm/fnm prefix. Windows uses cmd /C. Any failure returns None.
fn npm_global_prefix() -> Option<std::path::PathBuf> {
    let out = if cfg!(target_os = "windows") {
        crate::host::command("cmd")
            .args(["/C", "npm prefix -g"])
            .output()
            .ok()?
    } else {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                ["/bin/zsh", "/bin/bash"]
                    .iter()
                    .find(|p| std::path::Path::new(p).exists())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "/bin/sh".to_string());
        crate::host::command(shell)
            .args(["-lc", "npm prefix -g"])
            .output()
            .ok()?
    };
    if !out.status.success() {
        return None;
    }
    // Shell profiles may print noise; npm's response is the last nonempty line.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())?
        .trim()
        .to_string();
    let p = std::path::PathBuf::from(line);
    p.is_dir().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_layout_matches_the_host() {
        // The npm flag drives both candidate probing and dangling-wrapper detection, so it must follow
        // the platform conditions the recipes document.
        let win = cfg!(target_os = "windows");
        let mac = cfg!(target_os = "macos");
        let expected = [
            ("opencode", win),
            ("grok", win),
            ("crush", !mac),
            ("codex", true),
            ("copilot", true),
            ("cline", true),
            ("pi", true),
            ("claude", false),
            ("cursor", false),
            ("omp", false),
            ("antigravity", false),
            ("kimi", false),
            ("kiro", false),
            ("zoo", false),
        ];
        for (agent, npm) in expected {
            let (bin, _, use_npm) =
                agent_layout(agent).unwrap_or_else(|| panic!("no layout for {agent}"));
            assert!(!bin.is_empty(), "the command name for {agent} must not be empty");
            assert_eq!(use_npm, npm, "the npm flag for {agent} on this host");
        }
    }

    #[test]
    fn known_agents_have_nonempty_recipe() {
        for a in [
            "claude",
            "codex",
            "opencode",
            "copilot",
            "cursor",
            "antigravity",
            "cline",
            "pi",
            "omp",
            "crush",
        ] {
            let r = install_recipe(a).unwrap_or_else(|| panic!("no install recipe for {a}"));
            assert!(!r.command.is_empty(), "the install command for {a} must not be empty");
            assert!(!r.bin.is_empty(), "the bin for {a} must not be empty");
            assert!(r.docs_url.starts_with("https://"), "the documentation link for {a} should be https");
        }
    }

    #[test]
    fn omp_recipe_uses_the_official_installer() {
        // OMP ships a prebuilt binary through its own installer, so the recipe must not require Node, and its
        // binary name has to match the command inject.rs launches.
        let r = install_recipe("omp").unwrap();
        assert_eq!(r.bin, "omp");
        assert!(!r.needs_node, "OMP's installer downloads a binary and needs no Node");
        assert!(r.command.contains("omp.sh/install"), "the recipe should use the official installer");
    }

    #[test]
    fn pi_recipe_is_scoped_npm_package() {
        // Pi is scoped global npm only and its binary name matches inject.rs.
        let r = install_recipe("pi").unwrap();
        assert_eq!(r.bin, "pi");
        assert!(r.needs_node, "pi installs through npm, so Node is required first");
        assert!(
            r.command.contains("@earendil-works/pi-coding-agent"),
            "the pi install command should point at the scoped npm package"
        );
    }

    #[test]
    fn unknown_agent_has_no_recipe() {
        assert!(install_recipe("terminal").is_none());
        assert!(install_recipe("").is_none());
    }

    #[test]
    fn cursor_bin_is_cursor_agent() {
        // Binary names must match inject.rs, including cursor-agent.
        assert_eq!(install_recipe("cursor").unwrap().bin, "cursor-agent");
    }

    #[test]
    fn npm_candidate_layout_per_platform() {
        // Unix uses prefix/bin/name; Windows places the .cmd shim directly under prefix.
        let prefix = std::path::Path::new("/usr/local");
        assert_eq!(
            npm_bin_candidate(prefix, "codex", false),
            std::path::PathBuf::from("/usr/local/bin/codex")
        );
        // Build the Windows prefix/name.cmd expectation with path joins so Unix-hosted tests remain portable.
        let winp = std::path::Path::new(r"C:\Users\x\AppData\Roaming\npm");
        assert_eq!(
            npm_bin_candidate(winp, "codex", true),
            winp.join("codex.cmd")
        );
    }

    #[test]
    fn npm_recipes_allow_the_postinstall_that_creates_the_binary() {
        // Newer npm skips install scripts unless allowed; these packages produce their binary there.
        if cfg!(target_os = "windows") {
            let r = install_recipe("opencode").unwrap();
            assert_eq!(r.command, "npm install -g --allow-scripts=opencode-ai opencode-ai");
            let r = install_recipe("grok").unwrap();
            assert!(r.command.contains("--allow-scripts=@xai-official/grok"));
        }
        if !cfg!(target_os = "macos") {
            let r = install_recipe("crush").unwrap();
            assert!(r.command.contains("--allow-scripts=@charmland/crush"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_installers_are_probed_under_local_app_data() {
        // Cursor, OMP, and Antigravity install under %LOCALAPPDATA% on Windows, not ~/.local/bin.
        let local = local_app_data(&crate::host::home_dir().unwrap());
        let (_, cursor, _) = agent_layout("cursor").unwrap();
        assert_eq!(cursor[0], local.join("cursor-agent").join("cursor-agent.exe"));
        let (_, omp, _) = agent_layout("omp").unwrap();
        assert!(omp.contains(&local.join("omp").join("omp.exe")));
        let (_, agy, _) = agent_layout("antigravity").unwrap();
        assert_eq!(agy[0], local.join("agy").join("bin").join("agy.exe"));
    }

    #[test]
    fn exe_name_suffix() {
        assert_eq!(exe_name("claude", false), "claude");
        assert_eq!(exe_name("claude", true), "claude.exe");
    }

    #[test]
    fn locate_unknown_agent_is_none() {
        // Do not probe unknown or unguided types, matching install_recipe coverage.
        assert!(locate_installed_bin("terminal").is_none());
        assert!(locate_installed_bin("").is_none());
    }
}
