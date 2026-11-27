# Moving a VelaTerm Role to herdr

How one F-Co role leaves VelaTerm and resumes, on the same conversation, in the machine's herdr core. Evo moved all 21 of its roles this way on 2026-11-26 and 27, and CX1 uses the same steps. The plan and its rulings live in F-Co's herdr move plan and the Zulip topics `F-Co › move Evo from VelaTerm to herdr` (steps: 4496, 4519, 4715) and `F-Co › move CX1 from VelaTerm to herdr`.

## The Scripts

Each script is read-only towards VelaTerm and works out its own paths (`$HOME`, `FG_VAULT_ROOT`, and VelaTerm's data folder: `%APPDATA%` on Windows, `~/Library/Application Support` on macOS, or `VLX_DB` for the database itself). The Python and node scripts run on Windows, macOS and Linux; `move-role.ps1` is Windows-only, and `move-role.mjs` does the same everywhere (`HC_BIN` and `HERDR_APP_BIN` override where `hc` and the app's CLI are).

| Script | Step | What it does |
|---|---|---|
| `precheck.py --machine <m> <slug>,…` | 1–2 | Each role's VelaTerm row (taken by the registry's `velaterm_id`), its kind and engine, its saved conversation, the transcript baseline, `launch.mjs --print` against its VelaTerm preset, and whether a process holds it. |
| `wait-archived.py <velaterm id>` | 3 | Waits until fg's archive lands, then prints `archived_at`. Run it in the background. |
| `add-job.mjs <slug> --like <role> --cwd <dir> --space <space> [--write]` | before 1, Jobs only | Records an existing Job in `~/.config/f-co/jobs.json` with the like-role's argv and the Job page, through `spawn.mjs`'s `writeJob`. Without `--write` it only prints the entry. Then `register.js <slug> --launch "node …/launch.mjs <slug>"`, which keeps the record's conversation. |
| `node move-role.mjs --slug --conversation --cwd --zulip [--space-name] [--wait] [--private]` | 4, 5, 10a | The same as `move-role.ps1` below, on any platform. |
| `move-role.ps1 -Slug -Conversation -Cwd -Zulip [-SpaceName] [-Private]` | 4, 5, 10a | Launches through the F-Co launcher, then reports state, labels and the registry entry. It names the tab, and the space only if this launch made it. |

For state without the screen or the title, use the harness's `node <clone>/plugins/f-co/scripts/state.mjs <slug>`.

## The Steps

**Before the launch (driver)**
1. **Precheck.** Run `precheck.py`. Keep the baseline, check `launch_matches_preset`, and note `launch_cwd`. A transcript's first `cwd` can be a stale old path, so always launch from `launch_cwd`.
2. **Quiet.** The role posts "idle, ready" in the move topic and ends its turn.
3. **Kill + Archive in one go (fg).** In VelaTerm's sidebar, right-click → Kill, then Archive, right after "idle, ready". A role that's stopped but not archived is resumed in VelaTerm by any mention, because its registry entry still says VelaTerm. That happened three times on Evo. Then the driver confirms `archived_at` is set and that nothing holds the conversation, **just before** the launch.

**Launch (driver)**
4. **Launch** with `move-role.ps1`, from a core that herdr-app started. No `--first`, because the launcher adds the role's own autostart message when it has one.
5. **Registry (automatic).** Within about 20 s, `sessions.json` `by_name` shows `host: hc` and `hc_terminal`, with no `velaterm_id`.

**Check (a second agent, never the driver)**
6. One process holds the conversation, with the role's full launch line.
7. The same transcript grows past the baseline, and no new file appears.
8. `state.mjs` shows `claude-code`, `idle` or `working`.
9. Its settings, plugins, auto mode and status line load (from its screen).
10. The right space and tab (10a renames the tab for every role, since the app names a new tab with a number). A headless machine has no app (the Intel Mac runs herdr-core only, as the Herdr app has no Intel build), so `move-role.mjs` skips 10a there, and it's done from the app that shows that machine.
11. One mention, and it replies within about a minute.

**After**
12. The driver posts pass or fail in the move topic, with the terminal. f-co-mgr flips the role's Team Config row to `Start: herdr`.

**Rollback:** end Claude in herdr, fg restores the session from VelaTerm's Archived Sessions, and cc-zulip-lead points the name back at VelaTerm.

## Special Cases

- **Private role.** Use metadata only. Never `hc read`, `hc attach`, or raw `hc get` (its `title` is Claude's conversation title), and never open, list or measure its transcript; `precheck.py` doesn't touch it. Check 7 becomes: the registry's `conversation_id` equals the resumed ID, the process line has `--resume <id>` with no `--fork-session`, and the role confirms its history. fg does check 9 by looking. Its launcher keeps herdr's keep off, so no screen snapshot is saved. Launch it with `move-role.ps1 -Private`.
- **Codex role.** It has no launcher line. Launch it by hand with `hc run --bg --no-parent --name <slug> --label herdr.space=<space> --label herdr.tab=<slug> --cwd <folder> -- <codex.exe> resume <thread> --no-daemon`. It needs cc-zulip 0.1.36 or later, and `env_vars = ["VLX_SESSION_ID", "HC_TERMINAL_ID"]` in `~/.codex/config.toml`. Never park it: `park.mjs` refuses Codex, and a parked Codex can't be woken yet. To restart it, `hc send <t> /quit`, then `hc rm`. **Watch the first start's screen:** Codex can show an update prompt, and on CX1 it ran `npm install -g @openai/codex` with no input (0.167.2 → 0.172.0, 2026-11-27). Press Esc at the prompt, or pin the version first, if the tool owner holds updates. Where Codex is installed through npm, launch `node <npm prefix>/node_modules/@openai/codex/bin/codex.js` rather than `codex.cmd`. Its first start may stop at Codex's hooks-trust prompt, which fg answers. When the thread's recorded folder differs from `--cwd`, `codex resume` asks "Working directory": choose **use current directory**, the role's Team Config folder (a thread's recorded folder can be stale, as a transcript's can), and not an "Always" option.
- **Conversation View role** (`engine` is `chat`). The view saves Claude's own session ID, so a terminal `claude --resume <id>` picks up the same transcript. CX1's first move proved it (2026-11-27): the same `.jsonl` grew, and no new file appeared.
- **A role that launches the others** (the driver, the checker, the monitor) moves last, and another agent launches it: from another machine with `hc run --on <machine>`, or from another session there.
