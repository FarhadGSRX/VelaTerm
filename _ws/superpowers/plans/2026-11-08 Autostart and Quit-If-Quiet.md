---
Type: plan
Date Created: 2026-11-08
Status: 🟢
---
# Autostart and Quit-If-Quiet

Asked by f-co-mgr as VelaTerm's sponsor (Zulip F-Co › Guard and Monitor: who does what, 3321, 3325, 3327, 3384). The goal: VelaTerm can be restarted on any machine without fg. An outside job asks the running app to quit only when every session is quiet, swaps the build and relaunches it. Sessions linked to an `autostart` preset then come back by themselves. One build, on `fg-main` at 28b66e9, before the v0.3.0 sync.

## Rules for the builder

- Read `CLAUDE.md` at the repo root first, and follow its Build and test section. Commit as the git global identity, and end each commit message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Don't push, and don't touch `_ws/` or `+Intro.md`.
- **Check your base first.** Run `git log --oneline -1`. If HEAD isn't 28b66e9 or a descendant of it, run `git reset --hard 28b66e9` before any edit. Earlier worktrees were cut from an old upstream commit.
- `cargo test -j 2 --lib --no-default-features <filter> -- --test-threads=2`, filtered to the modules you touch, with `CARGO_TARGET_DIR=C:\Users\Farhad\_fg3\Code\VelaTerm\src-tauri\target`, and `C:\Program Files\Git\usr\bin` prepended to `PATH`. 15 tests fail at baseline, all Windows-only and listed one per line in `C:\Users\Farhad\AppData\Local\Temp\claude\522dcfce8a4806bf\9ffb5d2b-7c06-4313-a12e-91c2e12ee603\scratchpad\baseline-fails.txt`. A failure on that list isn't yours.
- Never run `vtell`, `vstop`, `vspawn` or any other `v*` command against a live session, never start the app, and never launch `velaterm-fg.exe`. The live checks are the lead's.
- Use the PowerShell tool; the Bash tool on this machine fails intermittently on quoting.
- If the code contradicts an assumption here, stop and say what you found in your report rather than forcing the spec.
- Report: branch, worktree path, commit SHAs, files changed, tests run and their results, each departure from this spec and why, and open questions. Under 450 words.

## 1. `autostart` on a preset

**Shape.** An entry in `~/.config/velaterm/presets.json` gets an optional string `autostart`, the first message for the resumed session. Absent, `null` or an empty or whitespace-only string means off. Any other type rejects that preset entry, the way `private` does (`agent/preset_file.rs` `parse_entry`). Keep it at most 4,000 characters, else reject the entry. Store it in a new `agent_presets.autostart TEXT` column, added by `db/mod.rs` `migrate` and `db/schema.rs`, as `private` was in 5267ae2. It follows the import and change-detection path that `private` uses.

**At launch.** In `lib.rs` setup, after `agent::preset_file::start(...)`, start a thread that:
- waits 5 s, so the hook server, the preset import and the local services are up;
- lists the sessions to start: not archived, kind Claude, engine `tui`, linked to a file preset whose `autostart` is non-empty, with a saved Claude conversation (`agent_session_id`), and no PTY running. Make this selection a pure function over rows so it can be unit-tested;
- for each, oldest link first, 2 s apart, resumes it with the message as the launch prompt, through the same path `vtell` uses to wake a stopped session (`agent/tell_tui.rs` `wake`). Factor that into one `pub(crate)` function both callers use. It already runs `preset_file::prepare_launch`, so args, permission mode, settings and the private flag apply as on any start;
- records one diagnostics event per session (`autostart`, with the session ID, and `started`, `blocked`, `skipped: <reason>` or the error). Never record the message text.

It runs once per process start, never on a window reload. A session without a saved conversation is skipped and logged. This changes nothing for sessions whose preset has no `autostart`.

**Tests.** Parsing with the field absent, null, empty, a string, a non-string and too long; the migration; import and change detection; and the selection function, covering archived, not linked, linked with no autostart, already running, no saved conversation, a non-Claude kind, and the ordering.

## 2. `--quit-if-quiet`

**Why.** Every ordinary quit (Dock, menu, window close, `osascript quit`) raises VelaTerm's quit dialog and waits (`lib.rs:1286-1306`), so an unattended job can't quit it. A programmatic exit with a code skips the dialog and still runs the exit cleanup (`lib.rs:1320-1340`).

**What.** A second launch of the app with `--quit-if-quiet` reaches the running instance through the single-instance handoff (`lib.rs:603-610`, release builds).
- **In the handoff callback:** when the arguments contain `--quit-if-quiet`, do not open a project or focus the window. Decide whether every session is quiet. If it is, write the result file, then call `app.exit(0)`. If not, write the result file and stay up.
- **Quiet means:** every session with a running PTY passes the same check `vstop` makes without `--force` (`agent/stop.rs` `refusal`), including the Bash background-shell check. A Claude with a running PTY and no reported state counts as busy, since it may be starting. A plain shell terminal (not an agent kind) doesn't block. A native chat session with a turn in progress is busy. Make the decision a pure function over a list of (session, kind, state, marks) so it can be unit-tested.
- **The result file:** `<app data dir>/quit-if-quiet.json`, rewritten on each request: `{"at": <ms>, "result": "quitting" | "busy" | "not-running", "busy": [{"sessionId", "name", "why"}]}`. Session names and reasons only, no text from any session. Also record a diagnostics event with the same fields.
- **With no instance running:** a process started with `--quit-if-quiet` that turns out to be the only instance writes `"not-running"` and exits 0 at once, before any window or service starts. In debug builds, which have no single-instance plugin, the same early exit applies.
- [Unverified] The handoff on macOS. Code it the same way on every platform. The lead has the Mac crew check it.

**Tests.** The quiet decision for each case (all waiting, one working, one asking, one in `vrun` background, one with a marked Bash shell, a Claude with no state, a plain terminal with no state, a chat session mid-turn), and the argument detection.

## After the build (the lead)

Full `cargo test` and vitest, a release build, then the live checks on Evo:
- **Autostart:** link a throwaway to a test preset with `"autostart": "Reply STARTED and nothing else."`, restart VelaTerm, and see it come back and reply without a tab.
- **The quiet check:** run `velaterm-fg.exe --quit-if-quiet` while one session is working, and expect `busy` with the app staying up. Then run it with all sessions quiet, and expect `quitting` with the app exiting and no dialog. That last part needs fg nearby, since it ends Evo's sessions.

Then the Mac crew checks the handoff on macOS, and technet-mgr's Mac job is written against it.
