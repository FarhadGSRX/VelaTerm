# Architecture

The **static** structure of VelaTerm — what it *is*. (Runtime behavior → `How-It-Works.md`.)

Reconciled 2026-08-22 from three parallel surveys at fork commit `d098f5a`; raw reports kept in
`_pending/` (`frontend-map.md`, `backend-map.md`, `theme-surface.md` + `theme-inventory.md`).
Claims here are cited `path:line`; anything uncited is a summary judgement, not a measurement.

## What it is

A Tauri 2 desktop app: Rust backend + system WebView running a React 19 / TypeScript frontend, with
xterm.js 5.5 terminals over `portable-pty` (borrowed from wezterm). Persistence is SQLite via
rusqlite. The pitch is a terminal *manager* built around AI coding agents — hierarchical sessions,
agent state visibility, survival across restarts.

## Components

### Frontend (`src/`, 188 tracked files)

Entry chain: `index.html:31` → `src/main.tsx`, rendering
`<ErrorBoundary><LoginGate>{isMobileView() ? <MobileApp/> : <App/>}</LoginGate></ErrorBoundary>`
(`main.tsx:39-43`). `LoginGate` holds remote clients behind token auth before `App` mounts, because
`App` calls `loadTree()` over the transport immediately.

Five fixed regions plus ~15 modals (`App.tsx:202-237`):

| Region | Owns |
|---|---|
| `layout/TitleBar/` | branding, theme toggle, Settings, remote-access panels |
| `layout/LeftSidebar/` (889 ln) | project/group/session tree, search, archive, recursively splittable tree views |
| `layout/CenterPane/` | tab bar + recursive split panes; terminal / dormant / doc / browser views |
| `layout/RightPanel/` | Inspector — Files / Info / Git, follows focused session cwd |
| `layout/StatusBar/` (665 ln) | counts, branch marker, permission + notification toggles, remote status |

`src/mobile/` is a **parallel client**, selected once at startup by `isMobileView()`
(`mobile/detect.ts:10-31`). It reuses the same store/transport/session hook but has its own
two-level navigation instead of tabs and splits.

### Backend (`src-tauri/src/`, 58 tracked files)

Default feature `gui`; `--no-default-features` builds a headless "vela-server" that drops Tauri/wry
and keeps the pty/agent/web/db core (`Cargo.toml` features block).

- `pty/` — session spawn, output streaming, resize, the ConPTY quirks.
- `agent/` — per-agent detection, launch injection, hook install, status. Ten agents get their own
  file on a uniform `home()`/hook-template/`install()` shape; **Claude and Codex do not** — their
  logic lives inline in `agent/inject.rs`. That asymmetry is where surprises hide.
- `web/` — axum SPA + WebSocket server for remote browser/mobile clients: `auth`, `e2ee`, `tunnel`,
  `rate_limit`, `dispatch`, `ws`. Serves the same `../dist` bundle the desktop embeds.
- `db/` — SQLite schema + queries. `app_settings` is a generic key/value table
  (`db/schema.rs:78-82`), which is why frontend settings need no schema change to extend.
- `command_core.rs` — the bulk of the Tauri command surface.

## Layers / boundaries

**The platform boundary is machine-enforced, not conventional.** ESLint forbids importing
`@tauri-apps/*` or `electron` anywhere outside `src/platform/` (`platform/types.ts:7-9`,
`platform/index.ts:4-6`). A PR that reaches around it fails CI.

Two abstraction levels sit between UI and OS:

1. `src/ipc/transport.ts` — picks Tauri `invoke`/`Channel` vs a WebSocket at runtime on `isTauri`
   (`transport.ts:19-22`). Most calls funnel through one `desktop_call` dispatch command; a
   whitelist (`DIRECT_DESKTOP_CMDS`, `transport.ts:105-128`) bypasses it for hot paths
   (`pty_write`, `pty_resize` — a thread hop per keystroke matters) and for calls that must run on
   the window-host thread.
2. `src/platform/` — non-IPC OS capability: dialogs, clipboard, badge, notifications, window focus,
   CLI install. Selects Electron vs Tauri implementation by `env.isElectron` (`platform/index.ts:14`).

**`isTauri` and "has a native host" are deliberately different booleans.** Remote-connection windows
are local `wry` windows that force WS transport while keeping `__TAURI_INTERNALS__` for local native
calls (`platform/types.ts:29-35`).

**Remote clients are a security boundary, and it defends itself.** `web/dispatch.rs:173-176` blocks
7 management and 5 secret commands for remote origins *before* argument parsing, with canary tests
(`EXPECTED_GATED`, `EXPECTED_SECRET`) written to fail loudly if either list silently shrinks.

## Data flow

Tree data (projects / groups / sessions) is SQLite-authoritative; the frontend mirrors it via
`loadTree()` and mutates through per-action IPC that patches locally then usually re-loads.

Settings are **two-tier, and the direction is easy to get backwards**: `localStorage` is a
synchronous first-frame cache, the backend is authoritative (`ipc/settingsSync.ts:8-15`).
`saveSettings()` writes local first, then mirrors via debounced `pushSetting()`
(`store/settings.ts:154-163`). Startup `reconcileSettings()` overwrites local from backend when the
backend has a key, and seeds the backend from local when it does not — hence "at most one flash".
Seven keys sync (`settingsSync.ts:29-37`).

PTY bytes flow backend → frontend as Tauri `Channel` messages or binary WS frames
(`[1-byte sid length][sid][raw bytes]`, `ipc/wsClient.ts:1-33`), into a scheduler, then into xterm.

## Key abstractions

- **Session** — the unit of everything. Has persisted identity, a runtime (status/pid/agent state),
  and zero or more views.
- **`termStore`** — one Zustand store, `store/termStore.ts`, **3,679 lines**, no `persist`
  middleware; persistence is manual because Tauri's WKWebView and Electron's Chromium cannot share a
  WebView store (`settingsSync.ts:3-6`). Nearly all cross-cutting state lives here.
- **Transport** — the runtime-selected IPC backend; the reason one frontend serves desktop, browser
  and mobile.
- **Pane tree** — `paneTrees: Record<SessionId, PaneNode>`, one recursive split tree per tab. Panes
  stay mounted and hidden with `display:none` so xterm and the PTY survive tab switches.
- **Fit owner vs mirror viewer** — exactly one client resizes a shared PTY; the rest CSS-scale a
  pinned grid. Desktop unmount kills the PTY; browser unmount only detaches.
- **Output scheduler** — `terminal/outputScheduler.ts`, a global write budget across *all* terminals.
- **Design tokens** — ~30 CSS custom properties under `[data-theme]`, plus `[data-accent|density|
  pane|divider|nav]` attribute selectors, written by `theme.ts`. See `theme-surface.md`.

## Extension points that already exist

Places the code is visibly built to be extended — relevant when planning contributions:

- `theme.ts:156-159` sets `--font-mono` / `--ui-fs` inline via `style.setProperty`. Inline beats the
  `[data-accent]` preset rules at `vlinx.css:37-40` with no `!important` — the seam a user theme
  would use.
- `terminal/registry.ts::setXtermTheme()` already live-updates every registered terminal.
- `SYNCED_KEYS` (`settingsSync.ts:29-37`) + the generic `app_settings` table — new persisted
  settings need no schema migration.
- `agent/*.rs` — the ten uniform per-agent modules are an obvious template for adding an agent
  (cf. upstream #43).

## Known structural gaps

- **A clean public checkout cannot build a Windows bundle.** `tauri.windows.conf.json` bundles
  `resources/gitbash/**/*`, but `src-tauri/resources/` contains only two notify scripts, and neither
  `fetch-gitbash.ps1` nor `release.sh` is tracked. `cargo build` is fine — it is a packaging-time
  dependency. `tauri build --no-bundle` is the contributor workaround.
- **The shipped `identifier` is the dev one.** `tauri.conf.json` says `io.vlinx.vlxterm`; releases
  use `io.vlinx.vlxterm.release`. Tauri derives `app_data_dir()` from it, so a self-built binary
  opens a *different database* and looks like data loss. The suffix also gates
  `web/mod.rs::is_production_identifier()`, which decides whether plaintext LAN HTTP is permitted —
  so building from source is a silent security downgrade as well.
- **`color-scheme` is set once and never updated** (`index.html:18,21`; `applyTheme()` does not touch
  it). Force dark on a light-mode OS and native in-page controls stay light for the window's life.
