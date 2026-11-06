---
name: public-push-diff-gate
description: "Run the outgoing-diff check as its own step and read it before pushing; keep private project names out of this public repo's worklist"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 9ffb5d2b-7c06-4313-a12e-91c2e12ee603
  modified: 2026-11-06T02:10:34.670Z
---

Run `git diff origin/<branch>..HEAD` in its own call, read the result, and only then push in a separate call. Never chain the diff and the push in one command. In `_ws/` and in commit messages, describe a private project by its role, never by its name.

**Why:** on 2026-11-05 I chained the two, so the check couldn't stop anything, and a commit put a private project's name into this public fork for the first time. Deleted content stays fetchable by SHA, so it can't be taken back.

**How to apply:** before every push from this repo, ask whether anything in the diff or commit message names something that isn't already public: projects, retention rules, machines beyond those already in the notes.
