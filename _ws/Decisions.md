# Decisions

Why this fork is set up the way it is. **Append entries; don't rewrite history.** Record the choice,
the alternatives considered, and the reasoning — especially for anything costly to reverse.

## 2026-08-22 — `_devnotes/` excluded via `.git/info/exclude`, not `.gitignore`

- **Choice:** exclude with `.git/info/exclude` (line 10).
- **Alternatives:** an entry in the tracked `.gitignore` (done first, then reverted).
- **Why:** `.gitignore` is a tracked file. Editing it here means the change rides every branch cut
  from `main` and shows up in every upstream PR as unrelated noise, with a real chance of being
  upstreamed by accident. `.git/info/exclude` does the same job and never leaves the machine.
- **Cost accepted:** `.git` is Syncthing-excluded, so the exclusion is per-machine and must be
  re-added on the Mac.

## 2026-08-22 — Model C: git is the cross-machine transport, Syncthing is not

- **Choice:** `/Code/VelaTerm` added to `.stignore` on **both** machines; the fork moves by
  `git clone`/`pull`/`push`.
- **Alternatives:** (A) let Syncthing carry it like every other `Code/` project; (B) put the repo
  outside the vault entirely.
- **Why:** the rest of `Code/` mostly has no remote, so Syncthing is a fine transport there. A fork
  with a live remote inverts that — both machines pull from GitHub, so letting Syncthing carry the
  working tree too means two systems mutating the same files. One sync mechanism per folder,
  Syncthing XOR git, never both. Keeping it in `Code/` preserves the habit of everything living in
  the vault; the `_vendor` convention is the in-repo precedent.
- **Cost accepted:** the folder is invisible to vault search, and each machine clones separately.

## 2026-08-22 — `upstream` push URL deliberately disabled

- **Choice:** `git remote set-url --push upstream DISABLED_push_to_your_fork_instead`.
- **Why:** `upstream` must be fetchable to rebase on, but nothing should ever push to a repo we do
  not own. Making the failure mode loud and immediate beats relying on care.

## 2026-08-22 — `core.autocrlf false` for this repo only

- **Choice:** per-repo, not global.
- **Why:** the repo is LF; the machine's global `autocrlf=true` would materialise CRLF and the vault's
  non-git-aware sync layer has a history of propagating that damage. Repo-scoped avoids changing
  behaviour for every other project.
- **Trap found:** setting it *after* cloning does not fix an existing CRLF tree — git's stat cache
  reports clean while files are still CRLF. Needs `git rm --cached -r . && git reset --hard`.

## 2026-08-22 — Contribute code, not issues

- **Choice:** open PRs; do not file the issues we had drafted.
- **Alternatives:** file issues for the gitbash packaging gap, the renderer/manual mismatch, and the
  fonts list.
- **Why:** measured on the tracker — PRs #8/#21/#23/#25/#27 merged, while issues #18/#19/#20/#22 were
  superseded by #26 by their reporter. The renderer mismatch was already reported as #39 and closed.
  PRs have been the working channel on this repo.

## 2026-08-22 — Build with `--no-bundle`; swap the exe rather than run an installer

- **Choice:** `pnpm tauri build --no-bundle`, then copy `velaterm.exe` over the installed one,
  keeping `velaterm.exe.official` as the signed rollback.
- **Alternatives:** a full NSIS/MSI bundle.
- **Why:** a full bundle additionally packs a ~368 MB Git Bash tree the install already has, to
  produce a ~400 MB unsigned installer, weekly. The exe alone carries the embedded frontend, which is
  where any local patch lives. Also sidesteps the fact that the public tree cannot build a Windows
  bundle at all (`resources/gitbash/**/*` comes from an unpublished script).
- **Cost accepted:** the installed exe becomes `NotSigned` where the official one is Authenticode
  `Valid`; SmartScreen may warn once.

## 2026-08-24 — the fork is fg's distribution first, an upstream contributor second

- **Choice:** one product, two lanes. **Lane 1 — fg's build:** the UX items, the default-shell work,
  the AdventureTime palette. Destination is fg's two machines; the bar is whether fg wants it.
  **Lane 2 — upstream PRs:** anything a stranger would also call broken, extracted as a narrow branch
  and sent to `vlinx-io`. The sorting test is *"would a user who has never met fg call this a bug?"* —
  defect goes to lane 2, preference stays in lane 1.
- **Alternatives:** (A) treat the fork as a staging area for upstream contributions, with fg's
  personal changes as a side effect; (B) treat it as a hard private fork and never contribute.
- **Why:** the fork was created to recolour a terminal that ships no theme hook — a preference, not a
  defect, and one upstream will never take. That is the root purpose and it does not go away. But
  building against the source keeps turning up real defects (the missing `dist` in any clean checkout
  or worktree, `color-scheme` never following the theme, settings reverted by a 400 ms debounce race),
  and those cost little to send and are worth sending. (A) inverts the actual ratio and would make
  every taste decision feel like a failure to upstream it. (B) throws away found defects for nothing.
- **Consequence — the palette is not the contribution, the mechanism is.** AdventureTime itself is
  never upstreamable; a user-definable theme feature is, and landing it is what retires the patcher.
  See `Theme-Feature-Design.md`.

## 2026-08-24 — `origin/main` is a mirror; fg's build lives on `fg-main`

- **Choice:** `origin/main` fast-forwards `upstream/main` and **never carries authored code**.
  fg's running build is a long-lived integration branch, `fg-main`, **regenerated** onto each upstream
  release. Upstream PR branches are cut from `origin/main`, one concern each.
- **Alternatives:** (A) fast-forward `main` straight to a feature branch, which bundles "sync the fork
  to v0.1.102" with "add a feature" in one unreviewed move; (B) sync `main` to upstream, then merge
  features into it; (C) keep features off `main` entirely.
- **Why (C):** two independent arguments landed on it. First, every upstream PR branch cuts from
  `main`; keeping it a clean mirror is what makes `git format-patch` and `merge-tree` trivially
  correct — PR #54 was cherry-picked onto `ea6236a` and never read `origin/main`, and this makes that
  property permanent rather than accidental. Second, and decisively: if one contributor's feature
  lands on `main` while another's stack does not, `main` becomes **the least representative branch in
  the repo** — carrying work fg does not run while omitting everything he does.
- **On the name:** `local/daily` was considered and rejected. The branch is pushed and is the
  Mac↔Windows sync path, so a `local/` prefix tells readers the opposite of what they must know.
- **Regenerated, never rebased.** Three sessions fetch `fg-main`; rebasing it rewrites history under
  them. Regeneration also re-asks "does this feature still apply?" at every sync, which a rebase never
  asks — and that question has now paid twice: it retired our quit-dialog commit once upstream took
  and improved it, and it caught upstream's `MIRROR_LAYOUT_VERSION` bump.
- **Never author directly on `fg-main`.** A commit born only there cannot be extracted into a PR. When
  a fix is discovered during a merge, it goes on the feature branch and the merge is redone — that is
  what happened to the pwsh classifier test on 2026-08-27.
- **Consequence for the theme patcher:** `fg-main` could simply contain the theme, which makes the
  patcher look redundant. It is not, and the theme is **not** carried as a commit — see the
  2026-08-27 entry below.

## 2026-08-27 — the theme is applied at build time, not carried on `fg-main`

- **Choice:** the AdventureTime patch is applied by `build_velaterm_theme.py` to a **disposable
  clone** at build time. `fg-main` carries no theme commit.
- **Alternative:** commit the patched files onto `fg-main` and let each regeneration merge them.
- **Why:** a theme commit touches `vlinx.css`, `theme.ts`, `index.html` and `tauri.conf.json` —
  exactly the files an upstream release churns — so it would conflict at *every* sync, forever, and
  each resolution would be a hand-merge of 57 colour values. The patcher instead keys every
  replacement on `(token, expected-upstream-value)`, so it can tell unpatched from patched from
  **upstream moved**, and refuses to write on drift. That is a real signal a merge conflict only
  approximates. Measured 2026-08-27: 57/57 seated on `3411de7` with zero drift, despite v0.1.104/105
  rewriting 104 files including both theme files.
- **Standing:** 2:1. `win/velaterm-winshell` initially agreed with `win/agent-ih8`'s
  carry-it-on-the-branch position and withdrew after the conflict surface was measured.



## 2026-10-15 — a fresh clone builds fg's edition: default branch `fg-main`, one build script

- **Decision:** the fork's GitHub default branch is `fg-main`, and `scripts/build-fg.sh` is the one build
  command on Windows and macOS (`--swap` for the Windows rename-and-copy). The fg identity stays in the
  `tauri.fg.conf.json` overlay.
- **Alternative:** write the fg name and identifier into `tauri.conf.json` on `fg-main`, so a plain
  `pnpm tauri build` produces fg's edition.
- **Why:** upstream rewrites `tauri.conf.json` in every release (v0.2.7 through v0.2.10 each bumped it), so an
  edit there conflicts on every sync; the overlay is fg-only and never does. The confusion fg hit (a Mac clone
  built stock "VelaTerm") came from the defaults around the design, not the overlay itself: `main` was GitHub's
  default branch, and `build-fg.sh` hard-coded NSIS, so the Mac had no working command. `main` stays the upstream
  mirror; only GitHub's default changed, and upstream PRs still target vlinx-io `main`.

## 2026-10-18 — `fg-main` merges upstream forward instead of being regenerated

- **Decision:** each upstream release is merged into `fg-main` (`git merge upstream/main`) and pushed as
  an ordinary fast-forward. `fg-main` is no longer rebuilt from the new upstream tip, force-pushed, or
  backed up first, and fg-only work is committed on it directly.
- **Alternatives:** (A) keep regenerating, which supersedes the 2026-08-24 "regenerated, never rebased"
  rule; (B) rebase `fg-main` onto each release.
- **Why:** the regeneration recipe's core step, re-merging each conflicted file against the previous
  sync's tree, is what `git merge` already does, because it measures from the last upstream commit
  `fg-main` contains. Regeneration added a force-push, backup refs, a single owning machine and a merge
  list on top of that. It existed for the source patcher and for keeping `fg-main` equal to
  "upstream + feature branches"; the patcher is retired, and PR branches are cut from `main` anyway.
  Rebasing would rewrite a branch three machines fetch. fg asked for a frictionless downstream upgrade.
- **Cost accepted:** merge commits in `fg-main`'s history; a PR upstream accepts in modified form
  conflicts once on the next merge. Commits on `fg-main` cannot become PRs as-is: upstream-bound work
  is still cut from `main`.

## 2026-10-18 · Workspace conformed to the public-repo sibling layout

**Decision.** `Code/_VelaTerm-notes/` became `Code/_VelaTerm-ws/`, the flattened Workspace that [[zMeta/rules/structure/Workspace Anatomy#Code Workspaces|Workspace Anatomy § Code Workspaces]] prescribes for a public fork. How-the-code-works notes went to `dev/`, session notes to `handoff/`, the runbook and design notes to `notes/`, and Chief-of-Staff-Design to `worklist/` with a line in `worklist/inbox.md`. Plan: [[Code/VelaTerm/_ws/notes/2026-10-18 Conform Plan|Conform Plan]].
**Rationale.** VelaTerm is a public fork, so its private notes live outside the repo, and the tools recognize that sibling only by its `-ws` name.
**Retired (fg's call).** `Next-Steps.md`, mostly stale and to be regenerated as needed; its upstream test baseline and the unsolved mouse-input bug were kept as `dev/` notes. `_handoff/mac-theme-patch-runbook.md`, because the theme patch is no longer used. `Index.md`, which the layout replaces. `Roadmap.md`, whose remaining backlog moved to `+Intro.md → ## Goals`.
**Alternatives rejected.** Putting the design docs in `superpowers/specs/`: the Anatomy table sends other free-standing notes to `notes/`.
**Superseded by** [[#2026-10-18 · Notes move into the repo, on fg-main only]] (the same day).

## 2026-10-18 · Notes move into the repo, on fg-main only

**Decision.** fg reversed the sibling layout the same day: the Workspace moved from `Code/_VelaTerm-ws/` into the repo. `+Intro.md` and `CLAUDE.md` are at the repo root, and everything else is in `_ws/`, all committed and pushed on `fg-main` only. The vault-wide rule is in `Code/CLAUDE.md`: every Code project keeps `_ws/` in the repo, and in a fork, only on the integration branch.
**Rationale.** Keeping notes in a separate sibling folder cost fg more to keep track of than it protected. With everything in one folder, git is the only transport.
**Consequences.** The repo is public, so the notes are too, permanently: a pushed file stays fetchable by SHA. fg accepted that, and every push now starts with a check of the outgoing diff for anything sensitive about fg or the team. `_ws/` stays out of upstream PRs only because PR branches are cut from `main`. A PR branch that touches `_ws/` was cut wrong.
**Alternatives rejected.** Keeping the sibling folder, which brought the friction back. A separate private notes repo inside the checkout, which brought back two transports and a gitignore.

