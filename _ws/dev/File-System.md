# File-System Conventions

Where things live in this repo and why. Companion to `Architecture.md`.

## Repo layout

| Path | Role |
|---|---|
| `src/` | React 19 + TS frontend, 188 tracked files. Built by vite into `dist/`. |
| `src/layout/` | The five screen regions, one folder each (TitleBar, LeftSidebar, CenterPane, RightPanel, StatusBar). |
| `src/store/` | Zustand state. `termStore.ts` is 3,679 lines and owns nearly everything; `settings.ts` and `docTab.ts` are plain modules re-exported through it (`termStore.ts:92-95`) so consumers keep one import path. |
| `src/ipc/` | Typed RPC surface + transport. `commands.ts` (591 ln), `events.ts`, `tree.ts`, `transport.ts`, `wsClient.ts`, `settingsSync.ts`. |
| `src/platform/` | **The only place** allowed to import `@tauri-apps/*` or `electron`. ESLint-enforced (`platform/types.ts:7-9`). |
| `src/terminal/` | xterm integration: `registry.ts`, `outputScheduler.ts`, `screenDetect.ts`, IME fixes. |
| `src/mobile/` | Parallel mobile client sharing the data layer, not a responsive variant. |
| `src/styles/` | `vlinx.css` (design tokens), `index.css` (semantic alias layer + control classes), `fonts.css`. |
| `src-tauri/` | Rust host. `src/` 58 files; `icons/`, `capabilities/`, `resources/`. |
| `src-tauri/src/web/` | axum + WebSocket server for remote clients; embeds `../dist` via `rust_embed`. |
| `electron/` | Alternative shell. Not what ships on Windows. |
| `scripts/` | Dev launchers (`*.sh` via `run-sh.cjs`), plus `gitbash/zz-vlx-term.sh` (the login stamp). |
| `docs/manuals/` | User manuals. Treat as possibly stale — see below. |
| `_ws/`, `+Intro.md`, `CLAUDE.md` | fg's notes and manifest, committed on `fg-main` only (not upstream). |

## Naming and conventions

- Frontend files are `PascalCase.tsx` for components, `camelCase.ts` for modules. Tests sit beside
  their subject as `*.test.ts(x)`.
- Rust modules are snake_case; per-agent integrations are `agent/<name>.rs` on a uniform
  `home()` / hook-template / `install()` shape — except Claude and Codex, which are inlined in
  `agent/inject.rs`.
- Config that must differ per platform is a sibling override: `tauri.conf.json` +
  `tauri.windows.conf.json`.

## Line endings

`.gitattributes` at the repo root pins `*.sh text eol=lf` — and only that. Everything else follows
the local `core.autocrlf`. On Windows **set `core.autocrlf false` for this repo**; the vault's
Syncthing layer is not git-aware and CRLF drift here has bitten before.

⚠ Setting `autocrlf` *after* cloning does not fix an already-CRLF tree: git's stat cache reports the
tree clean while files are still CRLF. Force it with `git rm --cached -r . && git reset --hard`.
Verified state at `d098f5a`: **399 tracked files = 315 text + 84 binary, 0 CRLF**.

## Our notes live in `_ws/`, on `fg-main` only

Since 2026-10-18 the notes are committed in the repo at `_ws/`, on `fg-main` only, and carried by git like the code. Before that they were a vault sibling folder (`Code/_VelaTerm-notes/`, briefly `Code/_VelaTerm-ws/`), and before that briefly in-repo at `_devnotes/`, hidden with `.git/info/exclude`. That first in-repo attempt failed because the exclude file is per-machine and never travelled, and because untracked notes were one `git add -A` from a PR. Committing them fixes the first problem. Cutting every PR branch from `main` fixes the second. The repo is public, so the notes are too: check the outgoing diff before pushing. Reasoning: `_ws/Decisions.md` (2026-10-18).

- `dev/` — how the code works; `dev/_pending/` holds raw agent survey output awaiting reconciliation,
  safe to delete once folded into the authored docs.
- `notes/` — working notes and runbooks · `worklist/` — tasks and inbox · `handoff/` — session handoffs.
- `_refs/` — outside sources.

The `.git/info/exclude` entries are left in place on Windows as cheap insurance in case anyone
recreates `_devnotes/` inside the checkout.

## Cross-machine model for this repo

**Model C: git is the transport, Syncthing is not.** `/Code/VelaTerm` is in `.stignore` on both
machines. One sync mechanism per folder — never both, or you get `.sync-conflict` files and CRLF
damage. `origin` is the fork; `upstream` fetches from `vlinx-io/VelaTerm` and its **push URL is
deliberately disabled**.

## Build artifacts (never committed, never synced)

`node_modules/`, `dist/`, `src-tauri/target/`. All three are already excluded at any depth by the
vault's `.stignore`; `target/` alone reaches multiple GB.

## What an outside contributor cannot build

`tauri.windows.conf.json` bundles `resources/gitbash/**/*`, but that tree is produced by an
unpublished `fetch-gitbash.ps1` and `src-tauri/resources/` ships only two notify scripts. `cargo
build` and `pnpm build` are unaffected — it is a packaging-time dependency. Use
`pnpm tauri build --no-bundle` to get a bare exe; a full Windows installer is not buildable from the
public tree.
