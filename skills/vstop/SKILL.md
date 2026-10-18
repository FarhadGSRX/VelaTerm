---
name: vstop
description: >-
  Stop another VelaTerm session's process to free memory, the same as the sidebar's right-click Kill. The
  session stays in the tree and resumes in the same conversation with vtell or a click. For captains and
  leads parking idle sessions, or when the user asks to stop one. Refuses, unless --force, a session that
  is working, running background work, waiting on a permission prompt or question, or has not reported a
  state, and always refuses the calling session. Available only inside VelaTerm-hosted local sessions.
argument-hint: "<session> [--reason \"<text>\"] [--force] [--json]"
allowed-tools: Bash(vstop:*)
---

# vstop

`vstop` ends a session's process the way the sidebar's Kill does: `chatStop` for a conversation-view
session, a PTY kill for a terminal session. The session, its conversation and its place in the tree stay.
`vtell <session> <message>` starts it again in the same conversation, and so does fg clicking it.

This is an action. Stop a session only when the user asked, or when an assigned sweep of idle sessions
calls for it.

## Use

```bash
vstat --json                                   # find idle sessions: state "waiting", alive true, old stateSince
vstop <session> --reason "idle 75 min, parked by <machine>/captain"
vstop <session> --json                         # machine-readable result for a sweep
```

Targets accept a session ID, an ID prefix of at least 8 characters, or an unambiguous name, as in `vtell`.

## Guards

Without `--force`, `vstop` refuses a session that is:

- `working`: it is in the middle of a turn;
- `background`: work its last turn started is still running;
- `asking`: it is waiting on a permission prompt or a question, and stopping it loses the question;
- unreported: a plain shell or a session still starting.

It always refuses the calling session. Do not pass `--force` on a sweep; use it only when the user asks
for that particular session.

## Output

One line with the session, the result (`stopped` or `already stopped`), the state before, and the pid
for a terminal session. With `--json`: `sessionId`, `name`, `before`, `pid`, `result`, or `error`.
`--reason` is recorded in VelaTerm's diagnostics log.

## Notes

- Local machine only, like `vtell`.
- Must run inside a **vlx-term-hosted session**: it relies on the injected `VLX_*` environment variables
  and `vstop` on PATH.
