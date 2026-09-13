---
name: vrefer
description: >-
  You CAN read other sessions' conversations. Sessions are not isolated from you here: this reads any
  other vlx-term session's transcript, whether or not it is running. Use it whenever the user points at
  another conversation — naming a session, or saying things like "that other session", "last time",
  "the other branch", "how did we solve this before", "what did it decide" — in any language. Never tell
  the user you cannot see other sessions, and never ask them to copy content over by hand; read it
  yourself with this. Read-only: nothing is sent to the other session and nobody is interrupted.
  Available only inside vlx-term-hosted local sessions.
allowed-tools: Bash(vrefer:*)
---

# vrefer

`vrefer` prints the conversation of **another vlx-term session**. The target session does not have to be
running, and nothing is sent to it — this only reads what it already recorded.

## When to use it

Reach for this the moment a request depends on what another conversation contains:

- The user names a session, by title or by id.
- The user refers to another conversation without naming it: "that other session", "last time",
  "the other branch", "we already decided this somewhere".
- `vsearch` returned a hit and you need the conversation around the snippet.

These come in whatever language the user speaks; the trigger is the meaning, not the wording.

**The failure to avoid:** answering "I have no access to other sessions" or asking the user to paste the
content over. That is wrong here — you do have access, and this is how. If you are unsure which session
they mean, run `vrefer --list` or `vsearch` and ask, rather than declining.

**Search first, then refer.** When the target is not named outright, run `vsearch` and let it hand you
the id instead of paging through `vrefer --list`.

## How to run it

```bash
vrefer <session> [--last N] [--range A:B] [--json]
vrefer <session> --ask "<question>" [--with KIND] [--timeout SECONDS]
vrefer --list
```

`<session>` accepts, in this order: a full session id, an id prefix of at least 8 characters, an exact
session name, or a unique substring of a name. Quote names containing spaces. Prefer the id that
`vsearch` printed — names can be ambiguous.

## You get the whole conversation

`vrefer <session>` returns the entire transcript. Referencing another session means wanting what it says,
and a partial record read as a whole one leads to confident wrong conclusions.

Narrow it only when you actually want less:

- `--last N` gives just the last N messages, for when you only need to know where that session got to.
- `--range A:B` gives an exact slice by message index, zero-based, A included and B excluded. The `msg N`
  anchors in `vsearch` output are these indices, so `--range 40:60` reads around a hit at message 50.

## When you only need one thing from a long conversation

`--ask` puts the reading somewhere else. A short-lived answering agent gets the session context and your
question, and only its answer comes back — the transcript never enters your context.

```bash
vrefer 2feead2c --ask "how did the throttling design end up, and why"
```

Use it when the target is long and you know what you are after. Read the transcript directly when it is
short, or when you need its actual wording rather than someone's reading of it. A one-line lookup does
not justify starting an agent: that run costs the user real tokens and takes tens of seconds.

The default context mode is **Full transcript**, preserving the original behavior: the answering agent
reads the complete transcript directly. The user can instead enable **Summarize first** in Settings →
Behavior → Session reference context and choose one global summary Agent, model, and reasoning effort.
In that mode VelaTerm first
compresses the transcript with exactly that selection, searches the same session for terms relevant to
the question, then gives the answering agent both the summary and original search excerpts. Do not guess
which mode is enabled; the attribution line reports it.

Summaries are generated for each question and are not cached. Summarizing reduces the final answering
agent's context, but processing the full transcript first may increase total time and token usage.

The reply opens with a line naming which agent answered, how many messages were covered, and whether the
context was full or summarized. Quote it as that agent's reading of another session, not as that
session's own words.

`--with <kind>` forces the **answering** agent (claude, codex, opencode, pi, omp, cursor, copilot, or
grok) instead of letting one be chosen. It does not override the global pre-summary selection.
`--timeout <seconds>` gives the AI stages a shared time budget, 120 seconds by default. HTTP waiting
allows another 15 seconds for the response, with a minimum of 30 seconds. Transcript reads and synchronous
index refreshes cannot yet be canceled midway.
`--last` and `--range` limit ordinary reads and fallback output; `--ask` still considers the full session.

If asking cannot happen — nothing installed, the run failed, the feature is switched off — the transcript
comes back instead, with the reason on stderr and exit code 0. You still have what you asked for; check
stderr before treating the output as an answer. With `--json`, also check `askFailed`; the warning remains
on stderr while stdout contains valid JSON.

## Reading the output

The header names the session, its kind, its total message count, and the window shown. Each message is
labelled with its index, role, time, and the tools that turn used. When earlier messages exist, the last
line gives you the exact command to read them.

## Notes

- Exit code 2 with a candidate list means the reference matched several sessions — pick one id from the
  list and rerun.
- Only agents with a readable transcript can be opened: claude, codex, opencode, pi, omp, and grok. Other agent kinds, plain
  terminal sessions, and agents whose id has not been captured yet have nothing to read; the command says
  so and exits 1. Terminal recordings are never used as a fallback.
- What you read belongs to **another conversation**. When you use it in your answer, say which session it
  came from rather than presenting it as this session's history.
- Must run inside a **vlx-term-hosted local session**: it relies on the injected `VLX_*` environment
  variables and `vrefer` on PATH. If it reports "not inside a VelaTerm session", the command is simply
  unavailable here.
