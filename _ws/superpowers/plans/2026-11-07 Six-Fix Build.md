---
Type: plan
Date Created: 2026-11-07
Status: 🟢
---
# Six-Fix Build

fg's OK for one build with six fixes came on 2026-11-07 (Zulip ws-velaterm › Orca challenge: CX1 checks, 3116 and 3294). The worklist items are in `_ws/worklist/tasks.md` § Next. The base is `fg-main` at 913b9b5.

## Who builds what

| Part | Items | Builder |
|---|---|---|
| A | 1 `vtell` startup fix and turn-started receipt, 5 helper sender | Sonnet subagent, own worktree |
| B | 3 per-session private flag | Sonnet subagent, own worktree |
| C | 2 `vstop` check for Bash background work, 6 `vrun --status` on Windows | Sonnet subagent, own worktree |
| — | 4 stale settings after a remote client reconnects | the lead, in `src/App.tsx` |

The lead merges the branches, runs the full `cargo test` and vitest, then does the live check on Evo. This build changes how a remote client reconnects (item 4), so the runbook's CX1-to-Evo remote check comes before it goes past Evo.

## Rules for every builder

- Read `CLAUDE.md` at the repo root first. Commit as the git global identity, and end each commit message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Don't push, and don't touch `_ws/` or `+Intro.md`.
- Evo shares RAM with other work. Use `cargo test -j 2 --lib --no-default-features <filter> -- --test-threads=2`, filtered to the modules you touch, and set `CARGO_TARGET_DIR=C:\Users\Farhad\_fg3\Code\VelaTerm\src-tauri\target` so the worktrees share one build directory (cargo waits for its lock, which is expected). The lead runs the full suite after the merge. 15 Windows-only tests fail on every run at 913b9b5, mostly file locks, so a failure in a module you didn't touch is not yours. In PowerShell, first prepend `C:\Program Files\Git\usr\bin` to `PATH`: many tests start `cat`, and fail with "program not found" without it.
- Run vitest only for a frontend file you changed (`pnpm vitest run <file>`), and only when your cargo runs are done.
- Never run `vtell`, `vstop`, `vspawn` or any other `v*` command against a live session, and never start the app. The live checks are the lead's.
- Use the PowerShell tool for shell work; the Bash tool on this machine fails intermittently on quoting.
- If the code contradicts an assumption below, don't force the spec onto it: stop, and say what you found in your report.
- Report: branch, worktree path, commit SHAs, files changed, the tests you ran and their results, each departure from this spec and why, and open questions.

## 1. `vtell` startup fix and turn-started receipt (part A)

**The bug.** `agent/tell_tui.rs` `ready_for_input(None)` is true, so a message to a Claude that hasn't reported any state since its terminal started is typed in at once. Sent about 2.6 s after a spawn it returned `sent` and was lost, 2 of 2. Sent while Claude sat at its folder-trust prompt, it returned `sent`, wasn't delivered, and Claude exited, 2 of 2. For Claude, VelaTerm's hooks report nothing before the first turn: `UserPromptSubmit`/`PreToolUse`/`PostToolUse` report working, `Stop` waiting, `Notification` permission/elicitation asking, and `Notification(idle_prompt)` idle, which Claude sends after about 60 s at its prompt (`agent/inject.rs` around line 119).

**1a. Queue until the session has reported.** `ready_for_input(None)` becomes false, so the message waits in `session_tell_pending` with receipt `queued`. The existing paths deliver it: the `Stop` hook's block when the first turn ends, or the `idle` notification, which types it in. Update the module doc, the function's doc comment and `a_claude_that_has_not_started_a_turn_is_ready_for_input`.

**1b. A receipt that says a turn started.** On the path in `deliver` that types the message in (`type_in`), wait after the write for the session's cached state to become `working`, polling as `confirm_woken` does. Share one helper between them, with the wait as a parameter. Wait 10 s here, and keep the wake path's 30 s.
- `working` within the wait: receipt `sent`.
- No `working` within the wait, terminal still open: new receipt `unconfirmed`, meaning it was typed in but no turn started, so it may be sitting in the prompt box. Record it in `session_tell_delivered`, so a retry with the same message ID reports `unconfirmed` and doesn't type it in again. `delivered_receipt` must return `unconfirmed` for it, not map it to `sent`.
- The terminal closes during the wait: an error that says the message was typed in, then the terminal closed, so it may not have been read. Record it as delivered, so a retry doesn't type it again.
- The `idle` hook path in `on_hook` has no caller waiting and stays as it is.
- Find where `vtell`'s receipts are described to users or agents (its `--help`, any skill or manual text shipped from this repo; search for `queued`) and add `unconfirmed` there.
- Tests: `ready_for_input(None)` is false; the receipt for each outcome of the wait (a pure function over the polled result, so no PTY is needed); a retry of an `unconfirmed` message reports `unconfirmed`.

## 5. Helper sender for `vtell` (part A)

**Why.** The cc-zulip plugin's waker runs as a background helper inside a session's environment, so every `vtell` it sends goes out as that session. `agent/tell.rs` refuses a tell to the sender itself ("Choose another session as the target", line 92), so the waker can't wake the session that started it, and every wake it sends is labelled as that session.

**What.** `vtell --sender "<name>" <target> <text>` sends with no sender session, under the given name.
- CLI: with `--sender`, the request leaves out `sessionId` even when `VLX_SESSION_ID` is set. `--sender` with `--report` or `--round` is refused.
- `tell::Request`: `session_id` gets `#[serde(default)]`, plus a new optional `sender` (camelCase `sender`). Exactly one of a non-empty `sessionId` or a `sender` is required.
- The name, trimmed: 1 to 64 characters with no control characters, else refused.
- With `sender`: no sender session lookup and no self-target check. `origin` is `{"name":"<name>","role":"helper"}`, with no `sessionId` or `agent`. `sender_id` is `helper:<name>`, which also goes into the fingerprint. The wire header format stays as it is.
- The endpoint still needs the app-wide token. Check whether anything else on the way (the HTTP handler, the CLI client) assumes or validates a sender session, and report what you find.
- Frontend: check that a tell whose origin has no `sessionId` renders in a chat pane, showing the name. If it needs a change, make it and add a vitest.
- Tests: request parsing with and without `sender`; both or neither refused; name validation; the origin and `sender_id` written for a helper tell; a helper may target the session whose environment it runs in.

## 3. Per-session private flag (part B)

**Why.** One role's session holds text that shouldn't be kept in VelaTerm's search index. Clearing its rows doesn't hold: `search/index.rs` `refresh_stale` (before each search and at `warm_index`) and `reindex_session` (on every PTY stop) rebuild them from the transcript. On a scratch database a deleted word stayed in the file after delete plus `VACUUM`, and reached zero only after FTS5 `optimize` on both FTS tables, then `VACUUM`, then `PRAGMA wal_checkpoint(TRUNCATE)`.

**What.**
- Storage: a new table, e.g. `session_private(session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE, set_at INTEGER NOT NULL)`, added the way other modules add their `SCHEMA`.
- Set from the preset: an entry in `~/.config/velaterm/presets.json` gets an optional boolean `private`, false when absent (`agent/preset_file.rs`). When a linked session starts (`prepare_launch`) and when it's linked, its row is set or cleared to match its preset. Unlinking leaves it as it is. Show it in `vself` if that is a small change.
- Index: `refresh_stale`, `reindex_session`, `reindex_session_quiet` and `warm_index` never index a private session. The next refresh drops any rows it already has (`drop_sessions`).
- Scrub only when a private session's rows were actually dropped, never on an ordinary refresh: `INSERT INTO session_fts(session_fts) VALUES('optimize')`, the same for `session_words`, then `VACUUM` (outside any transaction), then `PRAGMA wal_checkpoint(TRUNCATE)`. Check which database file the FTS tables live in, and say how long this would hold the connection on a large database.
- `vrefer` and `vrefer --ask`: a request from any other session for a private session's conversation is refused with a plain message ("<name> is private, so another session can't read its conversation"). The session itself is not refused. `vsearch` needs nothing beyond the missing rows, but check it.
- Find every other place that reads a session's transcript and copies its text into storage or hands it to another session (the memory feature, plan-execute, stall checks, anything else). List them with `file:line` in your report. Don't change them.
- Tests: preset parsing with and without `private`; set and clear on prepare and link; a private session is skipped and its existing rows are dropped; after the scrub, a test word's bytes are absent from the database file and its `-wal` file (as in the scratch test); `vrefer` and `--ask` refuse another session and allow the session itself.

## 2. `vstop` refuses Bash background work (part C)

**The bug.** `agent/stop.rs` `refusal` refuses Working, Background, Asking and no state. Background comes only from `vrun` runs (`agent/runs.rs`). A Claude that started a command with the Bash tool's `run_in_background` reports `waiting` when its turn ends, so `vstop` stops it. On Evo the background `sleep` outlived the stop; on CX1 it was gone. On Windows, Claude runs Bash-tool commands through Git Bash: CX1 saw two `bash.exe` processes for one background `sleep`.

**What.**
- Mark: in the Claude hook handling (`agent/server.rs` or wherever hook bodies are read), a `PreToolUse` body with `tool_name` `Bash` and `tool_input.run_in_background` true marks the session as having started background shell work. Keep the mark in memory, per session, and clear it when the session's PTY exits or restarts.
- Check at stop: for a marked session that `refusal` would otherwise allow, look for a shell process below the Claude process in the PTY's process tree: `bash`, `sh`, `zsh`, `dash` or `fish`, with or without `.exe`. Not `cmd`, `pwsh` or `powershell`, which Windows MCP launchers use. Found: refuse unless `--force`, with "it has background shell work still running (Bash run_in_background)". None found: clear the mark and stop as now. `procstat.rs` has `subtree_has_process`; add what's needed to match only below the Claude process.
- `vstop --help` and the usage text say it refuses this case too.
- Tests: parsing the hook body (Bash with and without `run_in_background`, other tools, bad JSON); the shell-name predicate; the refusal decision as a pure function of state, mark and whether a shell was found.

## 6. `vrun --status` on Windows (part C)

**The bug.** `agent/run_wait.rs` `alive()` always returns false on Windows (line 161), so `--status` calls a running command stopped. `agent/runs.rs` has a working Windows `alive()`. Also, once the waiting `vrun` times out (default 1800 s) and exits 124, nothing records the exit code, and `--status` later says "it was stopped, or vrun did not outlive it", which isn't what happened.

**What.**
- Use `runs.rs`'s `alive()` in `run_wait.rs` (make it `pub(crate)`) and remove the stub.
- On a timed-out wait, write a marker file (e.g. `waited`, holding the time) in the run's directory. A new run removes it, as it removes `exit` and `meta.json`. In `--status`, with no exit code, the PID gone and the marker present, print that the command finished after `vrun` stopped waiting at that time and its exit code wasn't recorded.
- Tests: the status message for each case (exit recorded, running, gone with marker, gone without marker, no PID), with the message choice as a pure function.

## 4. Stale settings after a remote client reconnects (the lead)

`src/App.tsx` `onConnState`: on `online`, also call `refreshSettingsFromBackend()`, as at startup. A remote client that kept its old `vlx-settings` across a backend restart wrote the whole copy back on its next preference change (seen on the Mac 2026-11-02).

## After the merge (the lead)

1. Full `cargo test` and vitest, comparing the failure set with the baseline at 913b9b5.
2. Release build on Evo, then the live checks: `vtell` to a fresh spawn and at a trust prompt (`queued`, then delivered once ready); `vtell` to an idle session (`sent` once its turn starts); a helper `--sender` tell to the calling session; `vstop` refusing a session with a background Bash `sleep`; `vrun --status` on a running command; a private session absent from `vsearch` and refused by `vrefer`.
3. The runbook's CX1-to-Evo remote check, with fg at CX1.
4. The crews install it on CX1 and the Mac and run the runbook's check rows.
5. Tell harness-lead about the `private` preset field, cc-zulip-lead about `--sender`, and f-co-mgr once `vstop` refuses Bash background work on all three machines.
