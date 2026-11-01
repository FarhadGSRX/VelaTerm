---
name: vpreset
description: >-
  List the presets VelaTerm follows from ~/.config/velaterm/presets.json, and link a session to one or
  unlink it. A linked session takes its preset's current arguments, permission mode and settings file
  each time it starts, so an edit to the file applies at its next resume without restarting VelaTerm.
  Use when the user asks to attach sessions to their role presets, or a rollout script links them.
  Available only inside VelaTerm-hosted local sessions.
argument-hint: "list | link <session> <preset-id> [--force] | unlink <session> [--json]"
allowed-tools: Bash(vpreset:*)
---

# vpreset

VelaTerm follows a generated presets file, `~/.config/velaterm/presets.json`. Presets from it appear in
the new-session menu, and a session created from one is linked to it. `vpreset` links sessions that
already exist.

This is an action: link or unlink a session only when the user asked, or an assigned rollout calls for it.

## Use

```bash
vpreset list                                  # file presets and the sessions linked to each
vpreset link <session> <preset-id>            # e.g. vpreset link velaterm-lead f-co/velaterm-lead
vpreset link <session> <preset-id> --force    # link even though the session's folder isn't the preset's cwd
vpreset unlink <session>                      # stop following; keeps the values the preset last gave it
vpreset list --json                           # machine-readable, for a script
```

Sessions accept a session ID, an ID prefix of at least 8 characters, or an unambiguous name, as in `vtell`.
A link takes effect at the session's next start: a click, or a `vtell` after `vstop`. A running session
keeps the process it has.

A preset's `cwd` only places new sessions; a linked session keeps starting in its own folder. So `link`
refuses a session whose folder (its own, else its project's) isn't the preset's `cwd`, and names both.
Check that you have the right session before passing `--force`. A forced link is logged with
`"forced": true`. The Preset picker in Edit Session isn't held to the folder.

## The file

```json
{"version": 1, "presets": [{"id": "f-co/velaterm-lead", "name": "velaterm-lead", "kind": "claude",
  "cwd": "C:/Users/me/_fg3/Code/VelaTerm", "args": ["--model", "opus", "--add-dir", "C:/Users/me/_fg3"],
  "permissionMode": "auto", "settings": "C:/Users/me/.config/velaterm/settings/velaterm-lead.json"}]}
```

- `id` and `name` are required. `kind` is `claude`. `cwd` is used only when a session is created.
- `args` has one element per argument, unquoted. It may not include `--settings`, `--setting-sources`,
  `--permission-mode`, `--dangerously-skip-permissions`, `--allow-dangerously-skip-permissions`,
  `--resume`, `--continue`, `--fork-session` or `--session-id`, and no argument may contain a backslash
  (use forward slashes), a double quote, a backtick, `$`, `%`, `!` or a control character.
- `permissionMode` is `default`, `acceptEdits`, `plan` or `auto`; `bypassPermissions` is refused.
- `settings` is merged into VelaTerm's own `--settings` at each launch. A launch stops if it can't be read.
- Paths are absolute. `execPath` is ignored. Write the file atomically (a temp file, then rename).

A rejected preset keeps its last good version. A file that won't parse changes nothing.

## Log

Every change is appended to `~/.config/velaterm/presets-import.log`, one JSON object per line, with `ts`,
`change` (`added`, `updated`, `removed`, `rejected`, `file-error`, `linked`, `unlinked`), `id`, and
`fields`, `reason`, `sessionId` or `forced` as they apply.

## Notes

- Local machine only, like `vtell`.
- Must run inside a **vlx-term-hosted session**: it relies on the injected `VLX_*` environment variables
  and `vpreset` on PATH.
