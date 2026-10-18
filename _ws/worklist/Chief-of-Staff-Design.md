# Chief of Staff — feature brief

**Status: design not started. Research done, recorded here so it is not re-derived.**
**Priority: fg's #1 for our local VelaTerm, as of 2026-09-09.** Theme picker drops below it.

> **You are the session picking this up.** Read this whole file, then run `/grill-me` with fg to turn it
> into a design. Do not start implementing. Do not re-run the survey in §2 — it is done and cited.

---

## 1 — What fg wants

- **One or a few primary sessions to talk to,** like the leads of particular projects, and those leads start their own worker sessions as needed.
- **Bring back an old crew.** When work on a feature went quiet weeks ago and new ideas arrive, fg tells the VelaTerm lead, and it brings back the sessions it worked with (the ones in its mesh room) and restarts that work.
- **A chief of staff:** one primary session fg speaks with, which re-invokes or re-instantiates other Claude sessions inside VelaTerm.
- **Park and revive.** After a period of quiet (maybe 12 to 48 hours), stop a session's process, perhaps compacting it first, keep the session, and revive it days or weeks later to continue the work.

**Scope beyond VelaTerm:** fg wants this across the entire vault, not limited to one project.

**Constraints he set:**
- **Local-only for now. No upstream PR until it is genuinely useful and effective.** (2026-09-09)
- **Do not complicate the schema** — no new branches, folders or conventions where an existing one fits.

---

## 2 — What VelaTerm already does — DO NOT RE-DERIVE

All verified by reading the source at `26cbb4d` on 2026-09-08/09.

### The `sessions` table already models the whole thing

`src-tauri/src/db/schema.rs:41-67`:

| Column | What it gives the feature |
|---|---|
| `agent_session_id` | **the Claude Code session UUID** — the revival key |
| `parent_session_id` | self-referencing FK → the lead/worker **tree** already exists |
| `group_id` | cohort membership |
| `archived_at` | "reversible soft hiding and read-only playback" (`db/mod.rs:112`) |
| `cwd`, `agent_args`, `permission_mode`, `agent_preset_id`, `agent_path`, `worktree_path` | everything needed to relaunch identically |
| `kind`, `mark`, `hotkey`, `fork_pending` | session type, user marks, fork state |

### Revival is already implemented

`src-tauri/src/agent/inject.rs:721-729` builds the relaunch command:

```rust
// Claude: `claude [--resume <id>] --settings <value> ["$VLX_INIT_PROMPT"]`
Some(rid) => format!("--resume {rid} --settings {settings}")
// and a --fork-session variant
```

Ten agents are covered, not just Claude — Codex, Copilot (`--resume=<id>`), Cursor, Kiro
(`--resume-id`), Grok, Cline, Pi, Antigravity (`inject.rs:827-1065`).

`src-tauri/src/agent/resume.rs` is the **capture** side: it scans `~/.claude`, `~/.codex`, `~/.cursor`,
`~/.pi` for rollout/session files and matches them to a session by `cwd` + mtime, with a 2-second margin.

Upstream's own manual states the contract (`docs/manuals/ai-agent-sessions_20260709_2041.md:37`):

> "After that — whether you closed the tab or quit the app — reopening the node relaunches the agent with
> its resume flag (e.g. `claude --resume <id>`) and the context comes right back. Before resuming, VelaTerm
> verifies the conversation still exists; if it was deleted, it silently falls back to a fresh start
> instead of hanging."

### The mesh already provides durable cohort identity

`claude-mesh` names survive restarts. From `set_name`'s own contract: a claimed name "is remembered and
comes back to you across a restart or a reboot," and adopting an **offline** name inherits "its rooms, its
read positions and its unread mentions." Verified live: `win/velaterm-winshell` was re-adopted across a
kill on 2026-09-09, and `mac/velaterm-mac` returned after 12 days offline with 31 queued mentions intact.

Rooms are the natural cohort key. The roster already carries a per-session `summary` string that persists
while the session is offline — see `mac/velaterm-mac`'s 12-day-stale summary, §5.

---

## 3 — The reframe that changes the build

**There is nothing to "store."** A Claude Code session is durable on disk the moment it exists — the
transcript is `~/.claude/projects/<slug>/*.jsonl`, independent of whether the process lives. VelaTerm
already records the UUID and already knows how to relaunch against it.

So "kill it after 24h and store it" describes something we get for free today. **Killing is purely
resource reclamation, not preservation**, and conflating the two is what makes the feature feel large.

**`/compact` before killing is counterproductive.** Compaction is lossy and buys nothing, because the full
`.jsonl` persists regardless. If context is too big on revival, compact *then* — when you know what the new
task needs.

**What is actually missing**, in descending order of value:

1. **Discovery.** "Show me the sessions that were working on VelaTerm branches three weeks ago." The data
   is all in `vlx-term.db`; nothing surfaces it as a cohort.
2. **Orchestrated revival.** A lead re-opening its workers, rather than fg clicking each node.
3. **A quiescence reaper.** The only genuinely new mechanism — and the one to build **last, or not at
   all**, until we measure what idle sessions actually cost. That is an afternoon's measurement.

---

## 4 — The design risk to settle in `/grill-me` FIRST

**A revived session is not the same as a live one.** `--resume` restores the transcript but not the working
tree, running tasks, watches, or the "where were we" that lived in fg's head. **A three-week-old session
resumed against a codebase that has moved four releases can be worse than a fresh session handed a good
handoff note** — it argues from stale facts with full confidence.

We hit exactly that failure mode twice on 2026-09-09, from *our own* stale context:
- `mac/velaterm-mac` returned after 12 days and asserted "fg has not asked" for a sync — true when it went
  offline, false on arrival, and load-bearing for what it planned to do.
- Its roster summary described a branch state that had been superseded the moment that branch was pushed.

**So the question the design must answer before any code:**

> Is the **transcript** the artifact worth reviving, or is the **handoff note**?

Our `handoff/` pattern already does the second, and does it well. It is entirely plausible that the Chief
of Staff should revive *notes plus a fresh session*, not old sessions — which is a much smaller build and
sidesteps the stale-confidence failure entirely. **Do not assume the transcript is the answer because it is
the thing that happens to be stored.**

A middle option worth pricing: revive the transcript **but inject a delta briefing** — what changed in the
repo, the branch, the roadmap since that session went quiet. That is the only version where the transcript's
value and its staleness are both addressed.

---

## 5 — Evidence already in hand about staleness at scale

The mesh roster on 2026-09-09 held **79 members**, the overwhelming majority offline `agent-xxx` sessions
from a single week, most with no summary and `cwd: /Users/farhad/_fg2`. That roster *is* a partial version
of the registry this feature wants — and its condition is the argument for the feature and a warning about
it at once:

- Auto-named sessions are reaped an hour after going offline; **claimed names persist indefinitely**. So
  the durable set is exactly the named ones, and naming is currently ad hoc.
- Persisted summaries go stale silently and are read as current. `mac/velaterm-mac`'s was 12 days old and
  described a session that no longer existed.

**Whatever the design does about summaries, staleness must be visible in the surface itself** — an age, a
SHA the summary was written against, something.

---

## 6 — Cheapest first step, if the design lands on "revive sessions"

The lead reads VelaTerm's SQLite directly — done twice already this week for settings diffing:

```sql
SELECT id, name, agent_session_id, cwd, group_id, parent_session_id, archived_at
FROM sessions WHERE group_id = ? AND agent_session_id IS NOT NULL
```

DB path (Windows, themed build): `%APPDATA%\io.vlinx.vlxterm.release\vlx-term.db`
⚠ The theme patcher rewrites `identifier` → `io.vlinx.vlxterm.release`, so a themed build and a stock build
read **different databases**. Read-only via `sqlite3.connect('file:'+db+'?mode=ro', uri=True)`.

That yields a cohort roster with resumable UUIDs for very little code, and tells us whether the
orchestration layer is worth building before we build it.

---

## 7 — Open questions for `/grill-me`

1. **Transcript or handoff?** (§4 — settle this first; everything else depends on it.)
2. What is a "cohort" — a mesh room, a VelaTerm `group_id`, a project, or a `parent_session_id` subtree?
   Three of the four already exist.
3. Does the lead persist across fg's own restarts, and how does it re-find itself? (`set_name` adoption.)
4. What does "quiesced" mean — no PTY output? no user input? no mesh traffic? And is killing worth
   anything, once we measure the RAM?
5. Cross-machine: one lead per machine, or one lead that can revive on either? A session's `.jsonl` lives
   on the machine that made it, so a Mac session cannot be resumed on Windows.
6. Does this live **inside VelaTerm** (Rust/TS, PR-able eventually) or **outside** it (a skill + the DB)?
   fg's "no PR until it's useful" makes outside-first the cheaper path, and the DB is readable today.
7. What does fg actually *say* to the lead? The interface is the feature; design the sentence first.

---

## 8 — What NOT to do

- Do not build a session store. One exists — `sessions` + `.jsonl`.
- Do not `/compact` before killing. Lossy, and buys nothing (§3).
- Do not build the reaper first. Measure the cost of idle sessions before assuming it needs solving.
- Do not send any of this upstream until fg says it is useful. **His words: "until it's really useful and
  effective, I don't think we'll submit a PR for it."**
- Do not add new folders, branches or conventions where an existing one fits.

---

## Related

- `Theme-Feature-Design.md` — the other feature brief; the format precedent for this file. Deprioritized
  below this one as of 2026-09-09, still on the roadmap, still intended as an upstream PR.
- `Upstream-Sync-Runbook.md` — if upstream moves under this work.
- Memory `project_claude_mesh` — the mesh's addressing, wake and durability contract.
