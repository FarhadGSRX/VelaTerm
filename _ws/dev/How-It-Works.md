# How It Works

The **runtime** narrative — what happens when VelaTerm runs. (Static structure → `Architecture.md`.)

Reconciled 2026-08-22 at fork commit `d098f5a`. Nothing here was observed against a running build
except where explicitly marked **[verified live]**; the rest is read from source.

## Entry point

`velaterm.exe` (Tauri host) creates the main window and starts the WebView, which loads the frontend
**embedded in the binary** — `tauri.conf.json` sets `frontendDist: "../dist"`, and
`src-tauri/src/web/mod.rs` additionally embeds the same folder with `rust_embed` for the remote
server. There is nothing on disk to edit; changing the UI means rebuilding.

`index.html` paints a background before React mounts (`index.html:18-22`) purely to avoid a white
flash, and declares `color-scheme` off `prefers-color-scheme`.

## Startup flow

1. **Pre-mount paint.** `index.html` sets `html, body { background: … }` and `:root { color-scheme }`.
   `vlinx.css`'s bare `:root` block carries a matching `--bg-0`/`--text` fallback. Three places
   hand-maintain this window background (`index.html:19`, `tauri.conf.json:21`, `electron/main.cjs:602`)
   and the light values are known to disagree.
2. **React mounts** (`main.tsx:39-43`) inside `ErrorBoundary` → `LoginGate`. Remote clients stop here
   until authenticated; desktop passes straight through.
3. **Client kind is chosen once.** `isMobileView()` (`mobile/detect.ts:10-31`) picks `MobileApp` or
   `App`. Desktop shells never qualify as mobile.
4. **Settings hydrate in two tiers.** `loadSettings()` reads the `localStorage` cache for a
   flash-free first frame; `reconcileSettings()` (`App.tsx:94-104`, `settingsSync.ts:88-115`) then
   treats the backend as authoritative — overwriting local where the backend has a key, seeding the
   backend where it does not — and reapplies anything it changed. This is the only expected flash.
5. **Theme applies.** `applyTheme()` resolves `system|dark|light`, writes `documentElement.dataset.theme`,
   persists the mode, and calls `setXtermTheme()`. `applyVisual()` writes `data-accent|density|pane|
   divider|nav` and sets `--font-mono`/`--ui-fs` inline (`theme.ts:156-159`).
   Note: **neither updates `color-scheme`**, so native in-page controls keep whatever the OS implied.
6. **Tree loads.** `App` calls `loadTree()` → `ipc/tree.ts:listTree()` over the transport; the store
   mirrors SQLite state.
7. **Workspace restores.** Previously open sessions come back as *dormant* panes — restored but not
   yet spawned — rather than firing N shells at once.

## Opening a terminal

1. UI action → store mutation → `ipc/commands.ts` call over the transport.
2. Rust spawns via `portable-pty`. Before the command runs, `appimage::scrub_pty()` undoes AppImage
   env rewrites, then the env is set: `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM`
   (`vlx-term` for managed agents so they prefer hooks over OSC notifications; `ghostty` for plain
   shells so OSC 9 works) — `pty/manager.rs:386-...`.
   ⚠ **The PTY inherits this process's environment and nothing strips `NO_COLOR`.** A parent with
   `NO_COLOR=1` produces a monochrome agent TUI while the same code advertises truecolor.
   **[verified live]** — reproduced 2026-08-22 by launching VelaTerm from an agent session.
3. On Windows only, Claude's `--settings` gets an explicit `theme` value injected
   (`pty/manager.rs:554-566`) because ConPTY answers OSC 11 with its own always-dark background, so
   Claude's automatic light/dark detection would be wrong. This is the *only* colour-adjacent thing
   the Rust side does; it is a brightness string, not a palette.
4. Output streams back as `Channel` messages (desktop) or binary WS frames (remote).

## Terminal output scheduling

`terminal/outputScheduler.ts` sits in front of `term.write` so N noisy background agents cannot
starve the focused terminal — xterm time-slices per terminal, which is not enough when a dozen exist.

- Focused terminal writes immediately; background output is coalesced under a **global** 128KB/frame
  budget (`outputScheduler.ts:59`).
- For 250ms after a keystroke the background budget drops to a 4KB trickle, so typing and IME stay
  responsive.
- Flushes are chunked at 64KB because xterm yields the main thread *between* `write` calls, not
  within one — an unchunked multi-MB backlog would freeze the UI.
- A 4MB per-session ceiling triggers fast drain; bytes are never dropped.
- Settings ▸ Advanced ▸ "Foreground-priority output" is the kill switch back to unscheduled writes.

## Agent state detection

`terminal/screenDetect.ts` infers working / asking / waiting by reading the bottom rows of
`term.buffer.active`, always anchored at `baseY` rather than `viewportY` so scrolling back does not
corrupt live state. Per-agent dispatch; **Codex is deliberately a no-op** because its state is
hook-authoritative and must never be guessed from screen text (`screenDetect.ts:56-59`).

## Presets from a file

VelaTerm follows `~/.config/velaterm/presets.json` while it runs (`agent/preset_file.rs`). F-Co's Team Config generates the file, one preset per role on each machine. The format and its rules are in `skills/vpreset/SKILL.md`, and the request and agreement are in #ws-velaterm msgs 1640–1650.

- **Import.** A poll every 2 s, and a check right before each launch of a linked session, compares the file's size and modification time with the last import. A changed file is imported in one transaction: presets are added and updated by `id`, and file presets no longer in it are removed (`agent_presets.source = 'file'`). Presets made in VelaTerm are never touched. A file that won't parse changes nothing. A rejected preset keeps its last good version.
- **Following.** A preset made in VelaTerm is copied onto a session once, at creation. A file preset is followed instead: `prepare_launch` copies its args and permission mode onto the session's row, and clears `agent_path`, before each launch path reads the row. The paths are the desktop `pty_spawn`, the web `pty_spawn` (`web/ws.rs`), the vtell wake (`tell_tui.rs`), spawn-request sessions (`pty/manager.rs`) and `chat_start`. A `--model` or `--effort` in the args also overwrites `session_model_settings`, because the terminal prefers that stored choice over the args. `cwd` is used only when a session is created, since a resume has to start in the conversation's own folder.
- **Settings.** VelaTerm already passes its own `--settings` (inline JSON with its status hooks), so a preset's `settings` file is merged into that JSON in `pty/manager.rs`. The file's keys stay, VelaTerm's keys win, and VelaTerm's hooks are added after the file's for each event. The chat engine passes no `--settings`, so there the file goes as `--settings <path>`. A settings file that can't be read stops the launch.
- **Safety.** Any agent can write the file, so it can't set `bypassPermissions`, an executable path, or the flags VelaTerm owns (`--settings`, `--setting-sources`, `--permission-mode`, the skip-permissions flags, `--resume`/`-r`, `--continue`/`-c`, `--fork-session`, `--session-id`). Args are quoted once for every shell, so backslashes, double quotes, backticks, `$`, `%`, `!` and control characters are refused.
- **Linking.** New sessions from a file preset are linked by `agent_preset_id`. Existing ones are linked with `vpreset link` (`agent/preset_cli.rs`, the `/preset` route) or the Preset picker in Edit Session (`set_session_preset`). File presets are read-only in VelaTerm. A preset's `cwd` places only new sessions, so `vpreset link` refuses a session whose folder (its own `cwd`, else its project's root, compared by `same_dir`) isn't the preset's `cwd`, unless `--force`; a forced link is logged with `"forced": true`. The Edit Session picker passes `force`, since fg's pick there is deliberate (agreed in Zulip F-Co › review the F-Co design, msgs 129–131).
- **Log.** Every change, link and unlink is appended to `~/.config/velaterm/presets-import.log` as JSON Lines, for the Monitor to post to `ops`.

## Folder trust for new worktrees

Claude Code trusts a folder when it, or a folder above it, has `projects["<path>"].hasTrustDialogAccepted: true` in `~/.claude.json`, and the walk up stops at a git root, so every worktree needs its own entry. Without one, a woken or spawned session in a worktree stops at the trust prompt. When VelaTerm creates a worktree (`git::worktree_add_in` for New Agent Session, `git::materialize_owned_worktree` for vspawn-tree and plan-execute), `agent/claude_trust.rs` adds that entry, but only when the repository's main checkout (the first `git worktree list` entry) already has its own, so a worktree never gets more trust than its repo.

The write follows Claude-OS's `trust_seed.py`. It re-reads the file just before an atomic rename, merges the one key, and retries up to five times if Claude Code wrote the file in between. It never removes a key or sets one to false. VelaTerm's `serde_json` sorts keys, so the file is parsed into an order-keeping value, and Claude Code's key order, 2-space indent and trailing newline (or lack of one) are kept. A failure is only logged (`claude_trust_seed_failed`), and the worktree is still created. This makes VelaTerm a writer of `~/.claude.json`; fg approved it through `ask_fg` on 2026-11-01 (Zulip F-Co › roll out the F-Co design), after claude-os-mgr and technet-sync-lead agreed in #ws-claude-os msgs 1679–1687.

## One session, several viewers

A single PTY can be attached to desktop, remote browser and mobile at once. A **fit owner / mirror
viewer** state machine elects exactly one client to resize the PTY; everyone else CSS-scales a pinned
grid. Teardown is asymmetric on purpose: desktop unmount kills the PTY, browser unmount only detaches.

## State and side effects

Reads/writes:

- `%APPDATA%\<identifier>\vlx-term.db` — SQLite (projects, sessions, groups, `app_settings`, FTS).
  **Which database depends on the `identifier`**, so a self-built binary silently uses a different one.
- `localStorage` — settings cache + sidebar view layout (debounced 200ms, v1→v2 migration).
- `<data_dir>/gitbash-full/` — the optional full Git Bash download, distinct from the bundled minimal
  tree beside the exe.

External surface: the axum server (opt-in remote access) with token auth, E2EE pairing over a URL
fragment that is never sent to the server (`wsClient.ts:118-144`), rate limiting, and an
origin-gated command dispatch table.
