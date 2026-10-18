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
