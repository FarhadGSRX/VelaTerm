---
name: vstat
description: >-
  You CAN see whether other vlx-term sessions are busy. This reports each session's current state —
  working, asking for permission, waiting for input — straight from the same status the sidebar shows.
  Use it when the user asks how a run or another session is doing ("is it done yet", "what are they up
  to", "how is the orchestration going") in any language, or when checking execution-session activity.
  Read-only and instant; nothing is sent to any session. Available only inside vlx-term-hosted sessions.
argument-hint: "[<orch-id>|latest] [--wait] [--follow] [--timeout N] [--json]"
allowed-tools: Bash(vstat:*)
---

# vstat

`vstat` prints which vlx-term sessions are working, asking, waiting, or running background work. The
answer comes from the same authoritative status the sidebar displays, not from reading screens.

For planning/execution task progress, use `vflow list [session]` to find workflow and execute IDs, then `vflow status <workflow-id>` from a workflow member session for task states, independent rounds and delivery receipts. For ordinary session hierarchy and saved properties, use `vself [session] --json`. `vstat` reports activity only; idle does not establish acceptance.

## Forms

```bash
vstat                     # every session known to vlx-term
vstat latest              # the most recent legacy orchestration from this session
vstat <orch-id>           # one specific run
vstat latest --wait       # block until something in that run changes, then print
vstat latest --follow     # keep printing changes until every agent has stopped
vstat --json              # machine-readable output, with any of the above
```

Each line is `<id prefix>  <state>  <kind>  <name>  <how long in that state>`. A state of `unknown` means
the session has not reported yet, which usually means it is still starting.

## When to use which

- **Plain `vstat` or `vstat latest`** answers "how is it going" right now. Use this from a conversation.
- **`--wait`** is for scripts that want the next change without polling. It returns after one change or
  after `--timeout` seconds (default 60, at most 300).
- **`--follow`** remains available to historical orchestration coordinator tabs. Do not run it from a conversation: it
  holds the turn until every agent has stopped.

## Reading the result

- `working`: the agent is in the middle of a turn.
- `asking`: it is blocked on a permission prompt and needs a person.
- `waiting`: it has stopped and is waiting for input. For a child session that was given one task, this
  means the task is finished, or the agent gave up; read its conversation with `vrefer` to tell which.
- `background`: its turn has ended, but work that turn started (a background task or a `vrun` command) is
  still running. The agent usually resumes on its own when that work finishes, so the task is not done.
- In `--json`, `stateSince` is the Unix time the session entered its current state, and `alive` says
  whether a process is running behind it. A session stopped with `vstop` reads `waiting` with
  `alive: false`.

## Notes

- Must run inside a **vlx-term-hosted session**: it relies on the injected `VLX_*` environment variables
  and `vstat` on PATH; if missing it reports "not inside a VelaTerm session" and exits.
- `latest` and orchestration IDs refer to historical runs from the retired vorch command. New task-splitting workflows use `vflow status`; the unfiltered `vstat` listing still includes their active sessions.
