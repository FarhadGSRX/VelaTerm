# History

Chronological changelog — what changed and when. **Append only.**

## 2026-08-23

- Forked `vlinx-io/VelaTerm` to `FarhadGSRX/VelaTerm`; set up at `Code/VelaTerm` under **Model C**
  (whole folder Syncthing-excluded, git as the only transport).
- Mapped the codebase with three parallel surveys → `Architecture.md`, `How-It-Works.md`,
  `File-System.md`. Raw reports kept in `_pending/`.
- Opened upstream **PR #50** — `fix: let a clean checkout run the dev build`. Two clean-clone
  blockers, both hit for real while trying to start the dev loop: `rust_embed` failing the crate when
  `../dist` is absent, and `dev-desktop.sh` aborting on the unpublished `fetch-gitbash.ps1`.
- Found that **all four shipped accents fail WCAG AA** for the white text drawn on them
  (2.06–2.66 vs 4.5). Reframed the theme work to lead with that as a bug fix.
- Designed the user-theme feature and had it reviewed by Codex → `Theme-Feature-Design.md`.
  Two structural changes came out of the review: a **semantic** UI token set instead of leaking raw
  internal names, and a **`scope: terminal | application | both`** field so existing terminal themes
  are importable.
- Moved these notes OUT of the repo to `Code/VelaTerm-notes/` after `Code-Sync-Architect` argued that
  an in-repo `_devnotes/` under Model C has no transport and is a PR leak path. `Code/CLAUDE.md`
  amended with the exception.
- **Mac theme build shipped.** Ran `build_velaterm_theme.py` on the Intel Mac for the first time:
  `--check` reports 57/57 seated with no drift, `--build --bundle` produced `VelaTerm.app` (binary
  60,314,040 B), and it was installed to `/Applications/VelaTerm.app`. The running process is
  SHA-256-identical to the built binary and carries `io.vlinx.vlxterm.release`. Palette confirmed by
  screenshot. **VelaTerm is now a terminal fg actually uses**, not just an artifact — this Claude
  session runs inside it.
- Two runbook predictions corrected against measurement: the **`.dmg` built** (45 MB, valid image)
  rather than failing with `-1743`, and the "no VelaTerm installed on the Mac" assumption behind the
  updater and app-support-directory notes no longer holds.
- **Six UX items shipped** on `feat/ux-settings-tab-and-paste-default` (10 commits ahead of `main`,
  unpushed): native image-paste default, Settings reopening its last section, hint tooltips on the four
  non-obvious Behavior settings, Cmd/Ctrl+Tab tab cycling, proportional interface-font presets, and a
  `kind` discriminator letting a left-sidebar view render a file tree. Suite 30/222 → **35 files /
  250 tests**.
- **Found and fixed a real upstream bug** (`e0c7050`): the quit dialog's "save workspace" answer was
  reverted on every launch. Settings writes debounce the backend by 400 ms, `confirm_quit` kills the
  process inside that window, and startup reconciliation then overwrites localStorage from the stale
  backend value. Fixed with `await flushNow()` before approving the exit. Affected *any* setting
  changed in the last 400 ms before quitting, not just the checkbox.
- **First use of parallel Opus teammates in isolated worktrees** — three items built concurrently,
  merged after review. All three independently caught that worktrees branch from `main` rather than
  the current checkout; one corrected a factual error in its own brief.
- Corrected the contribution-channel read from session 2: the maintainer *does* answer issues (a
  detailed #26 triage on 2026-08-20), PRs are simply the slow lane. Also learned the maintainer's
  development branch is **private**, so announced fixes are invisible in any public ref — which is how
  PR #52 came to duplicate work already done.
