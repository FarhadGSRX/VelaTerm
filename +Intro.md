---
Type: Code-Project
Workspace: Node
Upstream: https://github.com/vlinx-io/VelaTerm.git
Date Created: 2026-08-22 00:00
Last Updated: 2026-10-18 15:15
Tags: [code, fork, terminal, tauri, rust, typescript]
Status: 🚧
---
# VelaTerm — vault manifest

Fork of [vlinx-io/VelaTerm](https://github.com/vlinx-io/VelaTerm) (MIT) — a Tauri 2 terminal /
session manager built around AI coding agents. fg's fork is `FarhadGSRX/VelaTerm`.

See the repo's own `README.md`, and the notes beside this file, for project detail. This file is the
vault's manifest entry.

**The code and its notes are both in `Code/VelaTerm/`.** The notes are in `_ws/` (`worklist/`, `notes/`, `dev/`, `handoff/`, `Decisions.md`), committed on `fg-main` only. See "Cross-machine model" below.

## Why this fork exists

**fg's personal VelaTerm distribution — a terminal he actually runs, fitted to him.** Upstream
contribution is a real second lane, but it is opportunistic rather than the point.

VelaTerm ships **no user theme file and no custom-CSS hook**, so matching it to the AdventureTime
palette means patching source and rebuilding
(`.claude/_tools/spaces/adventuretime-theme/build_velaterm_theme.py`). That is the root reason the
fork exists, and it is a *preference* — upstream will never take fg's palette. Since then the fork has
also accumulated fg's own UX work (sidebar keyboard nav, sticky pane, favourites, tab cycling, a
configurable default shell) which is likewise taste rather than defect.

Building against the source keeps turning up real bugs, though, and those are worth sending:

| Lane | Destination | Bar |
|---|---|---|
| **1 — fg's build** | fg's two machines | does fg want it |
| **2 — upstream PRs** | `vlinx-io/VelaTerm` | narrow, one concern, reviewable by a stranger |

**The test that sorts any piece of work:** *would a user who has never met fg call this a bug?*
Defect → lane 2. Preference → lane 1.

The theme is the instructive case: the palette is never upstreamable, but the **mechanism** it needs —
user-definable themes — is, and landing that is what retires the patcher. *The palette is not the
contribution; the mechanism is.* See `Theme-Feature-Design.md`.

## Goals

Backlog carried over from the retired `Roadmap.md` (written 2026-08). Ideas, not commitments; committed tasks go in `worklist/tasks.md`, created when the first one lands.

- Report that the public repo cannot build a Windows bundle (`fetch-gitbash.ps1` is unpublished). Not filed as an issue; PRs have been the effective channel here, so it would need a PR.
- Agent-integration asymmetry: ten agents follow a uniform `agent/*.rs` shape, while Claude and Codex are special-cased inside `agent/inject.rs`. Relevant to upstream #43.

## ⚠ Upstream's tags are offset by one release — quote the SHA, never a tag

Verified 2026-08-27. From v0.1.103 onward each tag sits on the commit **before** its release commit:

| tag | commit | commit message | `package.json` |
|---|---|---|---|
| `v0.1.102` | `ea6236a` | release: v0.1.102 | 0.1.102 ✓ |
| `v0.1.103` | `c6a4ab1` | fix: Codex launch on Windows… | — |
| `v0.1.104` | `d49d088` | fix: bound the pre-exit flush… | 0.1.102 |
| `v0.1.105` | `124f4a6` | release: v0.1.104 | 0.1.104 |
| *(none)* | `56dad0f` | release: v0.1.105 | 0.1.105 |
| *(none)* | `7dd2b0b` | release: v0.1.108 | 0.1.108 |
| `v0.2.0` | `af9e7de` | release: v0.2.0 | 0.2.0 ✓ |
| `v0.2.1` | `68a4bc2` | release: v0.2.1 | 0.2.1 ✓ |
| `v0.2.2` | `7874081` | release: v0.2.2 | 0.2.2 ✓ |
| *(none)* | `b570e29` | Merge PR #75 (mobile i18n) | 0.2.2 — `upstream/main` on 2026-09-19 |

**`git checkout v0.1.105` yields `124f4a6`, whose content is 0.1.104** — one release behind, with a
tag name that matches what you asked for, so nothing announces it. `56dad0f`, the real 0.1.105, carries
no tag at all, nor do v0.1.106–108. The 0.2.x tags land on their release commits again, but `main`
already runs past the newest tag (`b570e29` ≠ `v0.2.2`), so the rule does not relax.

Consequences: never point the theme patcher's `--ref` at a tag; never identify a sync target by
version string. **The SHA is the only honest identifier.**

## Branch model

| Branch | What it is | Rule |
|---|---|---|
| `main` | mirror of `upstream/main` | fast-forward only; **never author here** |
| `fg-main` | fg's integration branch — everything he runs | upstream is **merged forward** into it each release (since 2026-10-18); never rebased or force-pushed; fg-only work is committed on it directly |
| `fix/*`, `feat/*`, `pr/*` | one concern each, for upstream PRs | cut from `main`; merged into `fg-main` when fg wants the change before upstream takes it |
| `_ws/`, `+Intro.md`, `CLAUDE.md` | fg's notes and manifest (not a branch) | committed on `fg-main` only. A PR branch whose diff against `main` touches any of them was cut wrong: re-cut it from `main` and cherry-pick the change |

`main` stays clean for two reasons: every PR branch cuts from it, so `git format-patch` and
`merge-tree` stay trivially correct; and if one contributor's feature landed there while another's
stack did not, `main` would become the least representative branch in the repo — carrying work fg does
not run while omitting everything he does. Reasoning in `Decisions.md` (2026-08-24).

## Cross-machine model — read before touching this folder

**Git is the only transport, for the code and the notes alike.** The vault sync skips unprefixed `Code/*` projects, so this whole folder moves by `git clone`/`pull`/`push`. One sync mechanism per folder; never both, or you get `.sync-conflict` files and CRLF damage.

- Each machine clones once (`gh repo clone FarhadGSRX/VelaTerm Code/VelaTerm`). `fg-main` is the default branch, so the clone has the notes. Pull before editing them and commit what you change: an uncommitted note reaches no other machine.
- **The repo is public, and so is everything on `fg-main`, the notes included.** fg accepted that on 2026-10-18 so there is one place and one transport. Before every push, check the outgoing diff (`git diff origin/fg-main..HEAD`) for anything sensitive about fg or the team: personal details, tokens or keys, internal hostnames, private contacts. A pushed file stays fetchable by SHA even after a delete. Patient data never goes in.
- `_ws/` stays out of upstream PRs because PR branches are cut from `main` (Branch model, above), not because anything ignores it.
- On Windows, set `core.autocrlf false` for this repo. Setting it *after* cloning does not fix an already-CRLF tree; force it with `git rm --cached -r . && git reset --hard`.

## Remotes

| Remote | Points at | Notes |
|---|---|---|
| `origin` | `FarhadGSRX/VelaTerm` | push here |
| `upstream` | `vlinx-io/VelaTerm` | fetch only — push URL deliberately disabled |

## Not to be confused with

`~/.cache/adventuretime-theme/velaterm-src` — the theme patcher's disposable checkout. Its `--clone`
runs `git reset --hard`, which would destroy uncommitted work. **Never point the patcher at this
folder.**
