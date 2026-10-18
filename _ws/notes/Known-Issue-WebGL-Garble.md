# Known issue: garbled Claude Code screen with the WebGL terminal renderer

**Status (2026-10-16):** cause found, worked around, not fixed. **Workaround: Settings → Terminal renderer → DOM**
(DOM is also the default). Do not re-investigate the ruled-out causes below.

## Symptom

On Evo (Windows 11, VelaTerm fg 0.2.9), almost every new Claude Code TUI session drew debris above its prompt:
stale copies of the `◐ medium · /effort` line, prompt-box rules ending in a fragment such as `❯ Tr`, and status-line
pieces (`📁 Fg-Doc-…`), offset from the live frame. It happened in every tab type (Claude agent tabs; PowerShell 7,
PowerShell 5, cmd and Git Bash terminal tabs running plain `claude`), and also on re-running `claude` in an existing tab.

## Cause

The **WebGL renderer** leaves stale rows on screen when Claude Code redraws **without synchronized output** (DEC
mode 2026). The terminal *buffer* is correct; only the WebGL painting is wrong.

- Switching to DOM fixed it immediately; switching back to WebGL brought it back on a fresh `claude`; back to DOM fixed it
  again (fg, 2026-10-16).
- Claude Code (2.1.30x) now runs full-screen in the alternate screen (`ESC[?1049h`) and redraws by cell diff.
- It wraps each frame in `ESC[?2026h … ESC[?2026l` **only when it believes the terminal is Windows Terminal**
  (`WT_SESSION` set). Through Windows ConPTY its synchronized-output probe gets no positive answer, so in VelaTerm the
  frames arrive unsynchronized and WebGL paints intermediate states it never fully repaints.
- [Inference] It surfaced in mid-October 2026 because Claude moved to the full-screen renderer; the same WebGL setting
  was fine with the older inline UI.

## What was tested (all on Evo, 2026-10-16)

| Test | Result | Rules out |
|---|---|---|
| Logs: a second client (`ws-4`, CX1's browser mirror over the remote-access server) attached to every new session during startup; theory: attach-time `nudge_winch` row resize corrupts Claude's first frame | Closing CX1's tab, then stopping Evo's remote server: still garbled | Second client, attach nudge, size ownership (#149 logic was behaving: rejected resizes were correct) |
| Resize the garbled pane; `/exit` and re-run `claude` in the same tab; restart VelaTerm | Still garbled | Stale-size / spawn-race theories |
| Send a message first ("hello"), then open new sessions | Still garbled | "Claude saves window width after a turn" |
| Plain `claude` in **Windows Terminal** | Clean | Claude Code alone |
| Claude 2.1.301 vs 2.1.302 in VelaTerm (2.1.302 installed 2026-10-16 16:59) — run 2.1.301 by copying `versions\2.1.301` to an `.exe` (the file has no extension; running it directly opens Notepad) | Both garbled | A 2.1.302 regression |
| `$env:TERM_PROGRAM=$null; $env:TERM_PROGRAM_VERSION=$null; $env:TERM=$null; claude` | Garbled | VelaTerm's `TERM_PROGRAM` (`vlx-term` agent / `ghostty` terminal tabs) |
| `$env:WT_SESSION="test"; claude` | **Clean** | — (pointed at the sync-output difference) |
| `$env:TERM_PROGRAM="vscode"; claude` (makes Claude treat it as xterm.js, which disables its DECSTBM scroll-region renderer, gated by flag `tengu_marlin_porch`) | Garbled | Claude's DECSTBM renderer |
| Byte capture: `claude` 2.1.302 in a ConPTY created exactly as VelaTerm does (portable-pty 0.8.1, `RESIZE_QUIRK \| WIN32_INPUT_MODE`, 179×49), queries answered as xterm.js 6 answers them (DA1 `?1;2c`, DECRQM 2026 `?2026;2$y`, CPR, OSC 11 — verified against real xterm.js), with/without mid-start resize, mouse motion and focus events | Final screen clean in pyte **and in real xterm.js 6 + Unicode11** (headless Edge, buffer read-back) | ConPTY flags, query replies, resize, mouse/focus input, emoji widths, xterm.js parser |
| Same capture with vs without `WT_SESSION` | Bytes identical except `ESC[?2026h/l` around every frame | Every other `WT_SESSION` branch in Claude (hyperlinks, AltGr, wheel, progress OSC 9;4) as the cause |
| Settings → Terminal renderer: WebGL → DOM → WebGL → DOM | Clean / garbled / clean | **Confirms: WebGL renderer** |

Test harnesses (session scratchpad, not kept): a Rust `ptybridge` using portable-pty 0.8.1, a Python driver that
answers queries and records raw output, and an `xterm.js`-in-headless-Edge page that replays a capture and dumps the
buffer.

## If someone wants to fix it properly

Reproduce in WebGL: headless Edge with software WebGL (SwiftShader), replay a captured Claude stream **chunk by chunk with
delays** so intermediate frames get painted, then compare a screenshot with the buffer. Then decide between:
- VelaTerm's own WebGL handling (`src/term/rendererAddons.ts`, the conditional reveal redraws in `usePtySession.ts`)
  → a VelaTerm fix and upstream PR;
- xterm.js `@xterm/addon-webgl` itself → an xterm.js issue, plus a VelaTerm workaround (full `term.refresh` after an
  output burst goes quiet).

Rejected workaround: setting `WT_SESSION` in VelaTerm tabs. It works, but it makes Claude and other tools believe they
are in Windows Terminal (e.g. `/terminal-setup` would edit Windows Terminal's settings).
