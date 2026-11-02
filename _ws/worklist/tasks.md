---
Type: info
Tags: [tasks]
---
# Tasks — VelaTerm

## Now

- [x] Launch presets from F-Co's Team Config: VelaTerm follows `~/.config/velaterm/presets.json` while running, and a linked session takes its preset's current args at each resume; `vpreset` links existing sessions (fg's go 2026-10-31; request and format in `#ws-velaterm` msgs 1640–1650) #task/cc/evo/velaterm-lead ✅ 2026-10-31 (fdfc3a6, Evo live test passed)
- [x] Roll the presets build out to CX1 (runbook step 6b) and the Mac, on fg's go #task/cc/evo/velaterm-lead ✅ 2026-10-31 (all three on 5facf8b; `#ws-velaterm` msgs 1673, 1676)

- [ ] `vpreset link` refuses a session whose folder (its own, else its project's) isn't the preset's `cwd`, unless `--force` (Zulip F-Co › review the F-Co design, msgs 129–131; fg's build OK in Decision 69) #task/cc/evo/velaterm-lead. Built: d81b6bc, in 58c21b0, now rolling out in f703062. Evo's live check passed on 58c21b0 2026-11-02 (refused without `--force`, linked with it and logged `"forced": true`, a differently spelled same folder linked; Zulip F-Co › roll out the F-Co design, msg 285). Left: CX1's and the Mac's reports on f703062
- [ ] VelaTerm writes `projects["<worktree>"].hasTrustDialogAccepted` in `~/.claude.json` for each worktree it creates, only when the main checkout already has its own (agreed with claude-os-mgr and technet-sync-lead, `#ws-claude-os` 1679–1687; fg approved it through `ask_fg` 2026-11-01, Zulip F-Co › roll out the F-Co design) #task/cc/evo/velaterm-lead. Built: 6125ed7, in 58c21b0 and the same rollout. Evo's vspawn-tree check on 58c21b0 found no worktree made at all, which is the bug below. Left: Evo's and the Mac's vspawn-tree checks on f703062, then post in `#ws-claude-os`
- [ ] On Windows, `vspawn-tree` and plan-execute never got their own worktree: `prepare_owned_worktree` passed canonicalize's `\\?\C:\…` path to git, and the spawn fell back silently to the parent's folder (upstream bug, still in v0.2.14). Found in Evo's live check 2026-11-02; fg approved the fix's build and rollout through `ask_fg` (Zulip F-Co › roll out the F-Co design) #task/cc/evo/velaterm-lead. Built: d0fdde0, in f703062; 9 of the Windows baseline test failures now pass. Left: Evo's swap and restart, CX1's and the Mac's installs (`#ws-velaterm` msg 1710), then an upstream PR if fg wants one

## Next

- [x] vtell wake of a vstopped session starts Claude in the wrong folder (home, not the session cwd) and stalls on the trust prompt; vstat reports it `waiting` (fg's go via evo/captain, 2026-10-18; report in `#ws-velaterm` msgs 770, 774) #task/cc/evo/velaterm-lead ⏫ ✅ 2026-10-18 (18519e9, live check passed)

## Soon

- [x] Hand-sort projects and collections within a group (drag to reorder). Today a project can't be moved above another once it's created (fg, 2026-10-19) #task ✅ 2026-10-26 (cd25bf0, fg confirmed)
- [ ] Bug: with a markdown viewer open, focus sometimes doesn't return to the Claude prompt box, and fg has to press Shift+Tab to get back to it (fg, 2026-10-19) #task

## Recurring

## Later / Maybe
