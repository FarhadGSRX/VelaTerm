---
Type: data
Date Created: 2026-10-18 15:20
Last Updated: 2026-10-18 15:20
Tags: [conform, velaterm]
---
# Conform plan — VelaTerm (2026-10-18)

Pilot run of `/workspace-conform`, by `evo/velaterm-evo`, approved by fg in session. Decision record: [[Code/VelaTerm/_ws/Decisions#2026-10-18 · Workspace conformed to the public-repo sibling layout|Decisions]].

## Moves (vault-migrate; every run net_new_broken 0)

| Was | Now |
|---|---|
| `Code/_VelaTerm-notes/` | `Code/_VelaTerm-ws/` |
| `Architecture.md`, `How-It-Works.md`, `File-System.md` | `dev/` |
| `_pending/` | `dev/_pending/` |
| `_handoff/`, `History.md` | `handoff/` (file names unchanged) |
| `Upstream-Sync-Runbook.md`, `Known-Issue-WebGL-Garble.md`, `Theme-Feature-Design.md`, `UX-Roadmap.md` | `notes/` |
| `Chief-of-Staff-Design.md` | `worklist/` |
| `Decisions.md`, `_refs/` | unchanged: the sibling's top level is its `_ws/` |

The first flatten map crashed live (`WinError 183`) without moving anything: `History.md → handoff/` created `handoff/` before the `_handoff → handoff` folder rename. Splitting it into two maps (`map2a`, then `map2b`) ran clean. The dry run cannot catch this collision.

## Judgment calls, as fg settled them

- `_handoff/mac-theme-patch-runbook.md`: deleted; the theme patch is no longer used.
- `Chief-of-Staff-Design.md`: moved to `worklist/`, with a line in `worklist/inbox.md` pointing to it.
- `Next-Steps.md`: deleted, as mostly stale. Kept first: the upstream test baseline → `dev/Upstream-Test-Baseline.md`, and the unsolved mouse-input bug → `dev/Known-Issue-Mouse-Input.md`. Its two open items were the Chief of Staff (now the inbox line) and the theme feature (shipped in-tree since v0.2.9).
- `Roadmap.md`: its remaining backlog went to `+Intro.md → ## Goals`, dated 2026-08. The rest was done or stale: PR #50, the theme treadmill, retiring the patcher.
- `Index.md`: deleted; the layout and `CLAUDE.md` replace it.
- Design docs stay in `notes/`, per the Anatomy table.
- `+Intro.md`: the notes-path line, the transport line (Syncthing is retired), and `## Goals`. Its frontmatter already conformed. It had no task dashboard to replace.
- `CLAUDE.md`: created, with the `## F-Co` block (room `#ws-velaterm`, lead `evo/velaterm-evo`).

## Sweep of path mentions

Rewritten: `Code/CLAUDE.md` (Current contents); memory `velaterm-fg-roadmap`, `velaterm-webgl-garble`, fg2 `project_velaterm_windows` and `project_adventuretime_theme`; and inside the Workspace, `+Intro.md`, `dev/File-System.md`, `notes/Theme-Feature-Design.md`, `worklist/Chief-of-Staff-Design.md` and `_refs/README.md`.

Left as written, as history: `handoff/*` (dated sessions and `History.md`), `dev/_pending/theme-surface.md` (an old survey), `.claude/_tools/consult_codex/_consults/*` and `.claude/_tools/_devnotes/sync-architecture/*` (dated logs), and `_fg2-Memory-Saved/*` (a saved copy of old memory).

Snapshot: `~/_fg3_conform_snapshot/velaterm_20261018-1453/`.
