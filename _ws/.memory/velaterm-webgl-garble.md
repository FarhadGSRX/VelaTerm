---
name: velaterm-webgl-garble
description: "Garbled Claude Code screens in VelaTerm = WebGL renderer (fix: renderer DOM). Fully investigated 2026-10-16; do not re-investigate."
metadata:
  node_type: memory
  type: project
  originSessionId: 9ffb5d2b-7c06-4313-a12e-91c2e12ee603
  modified: 2026-10-16T22:39:43.624Z
---

Garbled Claude Code TUI in VelaTerm (stale prompt/effort/status-line rows above the live frame) is caused by the
**WebGL terminal renderer**. Workaround in use since 2026-10-16: Settings → Terminal renderer → **DOM** (the default).
fg confirmed it by toggling DOM/WebGL/DOM.

Mechanism: Claude's full-screen renderer sends DEC 2026 synchronized-output brackets only when `WT_SESSION` is set,
so in VelaTerm (through ConPTY) frames arrive unsynchronized and WebGL leaves stale rows. The buffer is correct.

Ruled out, so don't retest: the second client / attach `nudge_winch` / size ownership; resizing, restarting VelaTerm or
re-running `claude`; Claude 2.1.301 vs 2.1.302; `TERM_PROGRAM`/`TERM` values; Claude's DECSTBM renderer
(`TERM_PROGRAM=vscode`); ConPTY flags, query replies, emoji widths and the xterm.js parser (a byte capture through
portable-pty 0.8.1, replayed into real xterm.js 6, came out clean). `WT_SESSION=test` fixes it, but setting it was
rejected as the fix.

**Why:** two hours of testing went into this; the full table is in `Code/VelaTerm/_ws/notes/Known-Issue-WebGL-Garble.md`.
**How to apply:** if garbling is reported, first ask which renderer is on. A real fix (VelaTerm WebGL handling vs
`@xterm/addon-webgl`) is optional future work, and the note says how to reproduce it. Related: [[velaterm-fg-roadmap]].
