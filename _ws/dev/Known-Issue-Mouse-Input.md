---
Type: data
Date Created: 2026-10-18 15:10
Last Updated: 2026-10-18 15:10
Tags: [velaterm, bug, windows]
---
# Known issue: the terminal intermittently takes no mouse input (Windows/WebView2)

**Unsolved.** Kept from `Next-Steps.md` (retired 2026-10-18). Seen 2026-08-27 on Main-PC, in the installed themed build, and not since.

The terminal received **no pointer events at all**: no drag-select, no right-click menu, and clicking *removed* focus from xterm's textarea rather than granting it, so only Tab/Shift+Tab traversal restored the cursor. It happened in a plain PowerShell tab as well as under Claude Code, so it is **not** mouse reporting. The parallel debug build never showed it.

**It cleared and has not returned.** Sequence: renderer `dom`→`webgl` + restart (appeared fixed), then `webgl`→`dom` + restart (**still fixed**). So the renderer is *not* the cause: the theory was tested in both directions and failed the second. Whatever cleared it is confounded with the two restarts.

> [!warning] Do not resurrect the `termRenderer` explanation
> It survived one direction and died on the other. A one-directional fix on an intermittent bug is not evidence of cause. (The garbled-screen problem is a different bug, and there the renderer *is* the cause: [[Code/VelaTerm/_ws/notes/Known-Issue-WebGL-Garble|Known issue: WebGL garble]].)

**Eliminated with evidence; do not re-chase:**
- `macOptionClickForcesSelection`: a Mac-only branch (`SelectionService.ts:429`), ruled out by the bug appearing in a plain shell, which enables no mouse reporting.
- The capture-phase mouse handlers, which are guarded on `e.button === 2`.
- `TermScrollbar`, an 8px strip.
- WebView2 persisted zoom, unset in `EBWebView\Default\Preferences`.
- `paneStyle`, which is padding only.
- The theme patcher's CSS targets (`index.css` is not among them).
- The `.xterm-helpers` full-width overlay: upstream reverted it in `c6a4ab1`, and it is absent from `3411de7`.

**If it recurs, capture evidence BEFORE restarting**, because the restart is what destroys the repro. The instrument is `pnpm dev:desktop` in the build worktree: it runs **in parallel with the installed app** (single-instance is `#[cfg(not(debug_assertions))]`) and ships DevTools; a cold debug build is about 7 min. Read `document.elementFromPoint(x,y)` over the terminal and `document.activeElement`. Also record **whether it survives a restart**: that one fact separates a startup-state bug from an accumulated-state bug. It was never established, so it has to be captured live; do not try to reconstruct it from process start times.
