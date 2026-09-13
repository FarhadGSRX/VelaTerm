---
name: vask
description: >-
  Ask a focused question about another vlx-term session without importing its full conversation into
  the current context. This is the skill form of `vrefer SESSION --ask "QUESTION"`. Use it when the
  target session is known and the user needs a conclusion, decision, result, reason, or status from it.
  Read-only: nothing is sent to the target session and nobody is interrupted. Available only inside
  vlx-term-hosted local sessions.
allowed-tools: Bash(vrefer:*), Bash(vsearch:*)
---

# vask

Use `vask` as the focused-question form of `vrefer --ask`. There is no separate shell command: invoke
the existing read-only command with the session reference and the user's question.

```bash
vrefer <session> --ask "<question>"
```

If the target is not known, search first:

```bash
vsearch <distinctive words...>
vrefer <id-from-search> --ask "<question>"
```

`<session>` accepts a full id, an id prefix of at least eight characters, an exact session name, or a
unique substring. Quote names containing spaces. If several sessions match, select an id from the
candidate list and retry rather than guessing.

By default the answering agent receives the complete transcript. If the user enabled **Summarize first**
in Settings → Agents, VelaTerm compresses the transcript with the one globally selected Agent, model,
and reasoning effort, adds relevant original full-text search excerpts from the same session, and then
asks the answering agent. This setting is automatic; the skill command and `--ask` syntax do not change.

Use `--with <kind>` only when the user explicitly wants a particular **answering** agent. It does not
change the configured pre-summary Agent. Use `--timeout <seconds>` when the default 120-second overall
limit is unsuitable.

The returned text is an agent's reading of another session, not a verbatim transcript. When exact wording
or broader context matters, follow up with `vrefer <session>` or `vrefer <session> --range A:B`.

If answering fails, `vrefer` explains the reason on stderr and returns the transcript as its fallback.
Check stderr before treating stdout as a completed answer.
