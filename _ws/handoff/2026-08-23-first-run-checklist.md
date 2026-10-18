# First-run checklist — RESULT: ALL VERIFIED IN THE APP

> ✅ **CLOSED 2026-08-24.** Every item below was exercised by fg in a real build and passed. Verified
> twice: first on `2a584fa` (themed v0.1.102), then again on **`99544db`** (themed v0.1.103 + all five
> feature branches), which is what he now runs from `/Applications/VelaTerm.app`.
>
> **Tier 1 — the two I flagged as highest-risk both came through clean.** The sticky pane rendered
> correctly including the divider drag and a terminal inside the region — the `parentRect` rescale
> and the xterm refit, neither of which any test covered. Sidebar keyboard navigation works including
> the virtualized-scroll case.
>
> **Tier 2 — the hand-resolved `FontSelect` merge is correct**: proportional presets are offered and
> are NOT mislabelled "Not installed on this device". The comma guard holds.
>
> **Tier 3 — all confirmed**, including **favourites persisting across a real app restart** (the one
> item that could not be proven without one) and both fixes from fg's own findings: Escape leaves the
> session tree, and Collapse All is visible rather than hidden.
>
> **Nothing on this list is outstanding.** Two defects were found by running it — the keyboard trap
> and Collapse All's discoverability — both fixed in `d282be1` and both re-verified above. That is the
> return on running a checklist against a real build instead of trusting 361 unit tests.

# Original checklist — themed 0.1.102 + our 11 items

Build: `/Applications/VelaTerm.app`, commit `2a584fa` (`feat/files-collapse-all`).
**Nothing here has ever run as an application.** 352 tests are unit tests over pure functions and
store actions. Ordered by where a bug is most likely, not by feature order.

## Tier 1 — verified only by arithmetic, highest risk

### Sticky pane (item 15) — right-click any tab → "Keep Visible in Split"
- [ ] The split renders at all; no overlap or gap at the boundary.
- [ ] **Drag the sticky divider.** ⚠ The likeliest bug in the whole build: the drag handler converts
      pixels through `parentRect`, which had to be rescaled with the divider. If it was wrong the
      divider **outruns or lags the pointer**. Watch the pointer, not the result.
- [ ] **Put a TERMINAL in the sticky region**, then drag the divider. ⚠ Completely unverified:
      xterm needs a `fit()` on resize, and the sticky region resizes by a path no test covers.
      Look for clipped columns, a wrong-sized grid, or text that does not reflow.
- [ ] Switch sessions in the sidebar — sticky content must NOT change.
- [ ] Split the sticky tab itself; its own dividers should still drag.
- [ ] Click the sticky tab to activate it → split collapses (not the same tab twice).
- [ ] Close the sticky tab → region disappears, no ghost.

### Sidebar keyboard nav (item 8)
- [ ] `Cmd+Shift+E` focuses the tree from anywhere.
- [ ] Arrows move a visible cursor ring; Home/End jump.
- [ ] **Arrow down past the visible rows** — ⚠ rows are virtualized; the cursor must scroll into
      view rather than vanish or lose focus to the page.
- [ ] Left closes / climbs, Right opens / steps in. Right on a leaf does nothing.
- [ ] Enter opens the session.
- [ ] With a terminal focused, arrows still reach the TERMINAL, not the tree.

## Tier 2 — a merge I resolved by hand

### Proportional UI font (item 11) + upstream's new availability check
- [ ] Settings → Appearance → **Interface Font** → pick a proportional preset.
- [ ] ⚠ It must NOT say **"Not installed on this device."** Upstream v0.1.102 added a font probe that
      tests a single family name; our presets are whole CSS stacks. I guarded it on a comma. If that
      guard is wrong, every working preset is mislabelled.

## Tier 3 — straightforward, but new

- [ ] **Favourites (13):** right-click a file → Add to Favorites. Pins above the tree.
      **Restart the app** — must persist. Open a different project — must be a different list.
- [ ] **Collapse all (14):** header button in Files. Hidden while filtering and when nothing is open.
- [ ] **Quit dialog (6):** tick "save workspace", confirm, relaunch → **must stick.** This is the
      data-loss fix; it is also the upstream PR.
- [ ] **Tab cycling (7):** `Cmd+Tab` / `Cmd+Shift+Tab` inside the app.
- [ ] **Settings section (5):** close Settings on a section, reopen → same section.
- [ ] **Sidebar files view (1):** a left-sidebar view set to Files.
- [ ] **Image paste (3):** defaults to native.

## Known gaps — not bugs, do not report
- A favourite whose file was deleted stays listed and errors on open.
- Arrow keys skip the pinned favourites block (tree only).
- The sticky slot does NOT survive an app restart (in-memory; tab IDs are not stable).

## Untestable alone
Mirror mode + sticky (`10a6eed`) needs two clients — desktop plus a browser at the remote URL.
