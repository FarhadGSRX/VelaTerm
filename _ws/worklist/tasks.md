---
Type: info
Tags: [tasks]
---
# Tasks — VelaTerm

## Now

- [x] Launch presets from F-Co's Team Config: VelaTerm follows `~/.config/velaterm/presets.json` while running, and a linked session takes its preset's current args at each resume; `vpreset` links existing sessions (fg's go 2026-10-31; request and format in `#ws-velaterm` msgs 1640–1650) #task/cc/evo/velaterm-lead ✅ 2026-10-31 (fdfc3a6, Evo live test passed)
- [x] Roll the presets build out to CX1 (runbook step 6b) and the Mac, on fg's go #task/cc/evo/velaterm-lead ✅ 2026-10-31 (all three on 5facf8b; `#ws-velaterm` msgs 1673, 1676)

## Next

- [ ] `vpreset link` refuses a session whose folder (its own, else its project's) isn't the preset's `cwd`, unless `--force` (offered in Zulip F-Co › review the F-Co design, msg 129; waits on f-co-mgr's merge) #task/cc/evo/velaterm-lead
- [ ] VelaTerm writes `projects["<worktree>"].hasTrustDialogAccepted` in `~/.claude.json` for each worktree it creates (vspawn-tree, New Agent Session), per `trust_seed.py` `write()`, so a woken worktree session doesn't stall on the trust prompt. Only when the worktree's main repo already has `hasTrustDialogAccepted: true` of its own, so a worktree never gets more trust than its repo. Name it in the rebuild notes for fg: it makes VelaTerm a new writer of `~/.claude.json` (agreed with claude-os-mgr and technet-sync-lead, `#ws-claude-os` 1679–1684). Same rebuild as the link check #task/cc/evo/velaterm-lead

- [x] vtell wake of a vstopped session starts Claude in the wrong folder (home, not the session cwd) and stalls on the trust prompt; vstat reports it `waiting` (fg's go via evo/captain, 2026-10-18; report in `#ws-velaterm` msgs 770, 774) #task/cc/evo/velaterm-lead ⏫ ✅ 2026-10-18 (18519e9, live check passed)

## Soon

- [x] Hand-sort projects and collections within a group (drag to reorder). Today a project can't be moved above another once it's created (fg, 2026-10-19) #task ✅ 2026-10-26 (cd25bf0, fg confirmed)
- [ ] Bug: with a markdown viewer open, focus sometimes doesn't return to the Claude prompt box, and fg has to press Shift+Tab to get back to it (fg, 2026-10-19) #task

## Recurring

## Later / Maybe
