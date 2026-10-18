# Upstream Sync Runbook

How to bring `fg-main` up to a new upstream release and get every machine running it.

**`fg-main` moves forward by merging upstream into it.** It is fg's integration branch: upstream plus fg's
own work, which is committed on it directly or merged into it. Since 2026-10-18 it is never regenerated
from scratch and never force-pushed. `git merge` already re-applies the last sync's resolutions, because it
merges against the last upstream commit `fg-main` contains. The old regeneration recipe reproduced that by
hand, at the cost of a force-push, backup refs and a single owning machine.

**Goal: run this in a few turns without fg's intervention.** He is needed at **exactly one point** —
step 8, restarting the app. Everything before it is agent work, and **steps 6-7 are inert until he
restarts**: the build touches nothing he is using, and the swap renames a file the running app keeps using.
So neither needs his permission.

> **Only upgrading this machine to the current `fg-main`?** Steps 6-9 are the whole job: build, swap, fg
> restarts, then report what is new and check what you can.

> **Do not ask him to authorise the build.** Proceed unless a stop-condition below fires. Tell him it is
> running; do not wait for an answer. (On Evo, if the Open-Model-Testing lab is online, its OK comes first; see step 6.)

### Stop and ask fg — the only conditions

| Condition | Why it stops |
|---|---|
| Any of the four gates red | a real defect, or a merge we got wrong |
| Failure set is **not a subset** of the measured baseline | a genuine regression |
| A conflict resolution we are not confident in | he owns the product behaviour |
| Anything outward-facing (a PR, a push to upstream, a force-push anywhere) | his identity, his call |

None of those firing means: merge, push, build, swap, and hand him step 8.

> **Ownership.** Steps 0-5 run on **any one machine**: claim the sync in mesh room `#velaterm` first, so
> two machines do not merge the same release at once. A second merge would not lose anything (its push is
> refused as non-fast-forward), but it wastes a resolution. **6-9 every machine, independently**: each box
> builds, swaps and reports its own install; only the SHA is shared · 8 fg restarts.
>
> These notes are in the repo (`_ws/`, on `fg-main`), so they travel by git: pull before editing, commit
> and push after. Claim a doc in `#velaterm` before a large edit, since every machine writes to it.

> **Upstream PRs are cut from `main`, never from `fg-main`.** `fg-main` carries merges and fg-only work an
> upstream PR must not contain. To run a PR's change before upstream takes it, merge its branch into
> `fg-main`.

---

## 0 — Detect

```bash
cd "${FG_VAULT_ROOT:-$HOME/_fg3}/Code/VelaTerm"
git fetch upstream && git fetch origin
git log --oneline -6 upstream/main
git rev-list --left-right --count upstream/main...origin/fg-main   # behind / ahead
```

Left `0` means `fg-main` already has everything upstream released; there is nothing to sync.

⚠ **Quote the SHA. Never the tag, never the version string.** Upstream's tags have run one release behind
since v0.1.103, and recent releases ship untagged (`56dad0f`, `7dd2b0b`). A tag checkout silently gives you
the *previous* release under the name you asked for. Two commits can also carry the same version string;
v0.1.108 has two.

---

## 1 — Assess (read-only; write nothing in this step)

`B=$(git merge-base origin/fg-main upstream/main)` is the last upstream commit `fg-main` contains: the base
this merge measures from.

**Scale.** `git diff --stat $B..upstream/main | tail -3`.

**Collision set** — which of our files upstream also touched. These are where conflicts can come from:

```bash
git diff --name-only $B..origin/fg-main   | sort > /tmp/ours.txt
git diff --name-only $B..upstream/main    | sort > /tmp/theirs.txt
comm -12 /tmp/ours.txt /tmp/theirs.txt
```

**Has upstream built something we carry? Check the CHANGE, not the branch.** Branch containment answers a
different question. Look for the *symbol* upstream:

```bash
git show upstream/main:<file> | grep -n "<symbol>"
git log --all --oneline -S"<symbol>" -- <file>      # who introduced it, ever
```

If upstream now has its own version of one of our features, the merge keeps both unless the lines collide.
Decide which one stays before merging, and say so in the room.

> 2026-09-09: `worktree_remove` had been fixed upstream with our exact reasoning while our PR sat unsent
> for two weeks. `-S"main_repo_of"` returned only upstream commits — proof we never even wrote it.

⚠ **Anchor on the symbol, not the substring.** `grep "sidebarView"` matches `sidebarViewParts`, a different
function in a different file. Use `grep "function sidebarView"`. And do not `| head` a supersession check:
on 2026-09-19 it showed 5 of 11 hits, and the decisive ones were #6-9. `wc -l` first.

**Mirror compatibility.** `git show upstream/main:src/store/mirrorLayout.ts | grep MIRROR_LAYOUT_VERSION`.
If it changed, mirror mode between the machines breaks this round and needs its own plan.

**Theme registry movement.** `git diff --stat $B..upstream/main -- src/themes.ts src/themes.test.ts`. Our
`"adventure-time"` entry rides on upstream's `ThemeDef` shape and `CHROME_TOKENS` list. If either changed,
the entry may need new fields, and `tsc` plus `themes.test.ts` are where that shows up.

### Open upstream PRs

```bash
gh pr list --repo vlinx-io/VelaTerm --author FarhadGSRX --state open --json number,headRefName,mergeable
```

A PR branch is cut from `main`, so the sync does not touch it.

- **Merged upstream since the last sync:** nothing to do. If `fg-main` already carries the same change, the
  merge sees identical lines and takes them once. If upstream changed it on the way in, expect one
  conflict there; take upstream's version unless ours is deliberately different.
- **Still open:** leave it alone unless `mergeable` turns `CONFLICTING`. If it needs updating, rebase it
  onto the new upstream tip and force-push **the PR branch only**:

  ```bash
  git rebase --onto <new-upstream-sha> <old-base-sha> <pr-branch>
  git push --force-with-lease origin <pr-branch>
  ```

  ⚠ A force-push to an open PR is **outward-facing**: it rewrites what a maintainer may be looking at. That
  is a stop-condition; ask fg.

> Do not rebase a PR branch merely because the base moved. Upstream merges PRs without asking for
> rebases, so a stale-but-clean PR needs no update, while a needless force-push costs credibility.

---

## 2 — Mirror `main`

`main` is an exact mirror of `upstream/main`, and the base every PR branch is cut from. Never author on it.

```bash
git rev-list --left-right --count origin/main...upstream/main   # LEFT MUST BE 0
git push origin upstream/main:refs/heads/main
git ls-remote --heads origin main                                # must equal rev-parse upstream/main
```

Left ≠ 0 means someone authored on `main`; stop and resolve that first.

---

## 3 — Merge upstream into `fg-main`

Merge in the live repo, on `fg-main`, with a clean worktree:

```bash
git switch fg-main && git pull --ff-only origin fg-main
git status --short                          # must be empty
git config rerere.enabled true              # once per machine: git remembers each resolution
git merge --no-ff upstream/main -m "Merge upstream <version> (<sha>) into fg-main"
```

**Before resolving a conflict, read how the last sync resolved that file.** Earlier syncs are recorded as
merge commits on `fg-main`:

```bash
git log --merges --first-parent --oneline -5 fg-main     # the recent syncs
git show <that-merge> -- <file>                           # its combined diff: how the file was resolved
```

Syncs before 2026-10-18 were regenerations, so their resolutions live in a
`fix: reconcile the <version> merge with our features` commit instead. Read it the same way.

Resolve, `git add`, `git merge --continue`. If upstream rewrote a file our feature depends on and the
resolution is a judgement call, that is a stop-condition.

**The merge commit is not pushed until steps 4-5 are done.** Until then nothing has left this
machine, and `git merge --abort` (or, after committing, `git reset --hard ORIG_HEAD`) undoes it.

---

## 4 — `pnpm install`, then the gates

Upstream adds packages between releases, and **a missing dependency reads as merge damage**. In the
v0.1.108 sync upstream added `turndown`; without the install, `tsc` reported seven errors in
`selectionCopy.ts` that looked exactly like a botched resolution.

```bash
pnpm install --frozen-lockfile
```

**Four gates, all green before pushing:** `tsc` · `eslint` · `vitest` · `cargo test`.

Windows suite flags (the Rust suite hangs here without the skip). On Evo, keep to 2 jobs:

```bash
cargo test -j 2 --lib --no-default-features -- --test-threads=2 --skip wait_capture_does_not_expire_before_late_first_prompt
npx vitest run --pool=threads --maxWorkers=2
```

⚠ **macOS `cargo test`: set `PYTHON`.** The `security::*` tests shell out to `python3` and need `tomllib`
(3.11+); the Mac system `python3` is 3.9.6, so four tests fail with the module byte-identical to upstream.
The code reads env `PYTHON` (`security/native.rs`, `security/upstream.rs`):

```bash
PYTHON=$(uv python find 3.12) cargo test      # 2026-09-19: 1054 passed / 0 failed / 15 ignored
```

⚠ **`skills/vkb` tests fail in a CRLF worktree.** With `core.autocrlf=true` (Evo), `include_str!` reads
`\r\n` and `knowledge_skills_replace_retired_name_without_implicit_invocation` fails on `"name: vkb\n"`. It
is part of that machine's baseline, not merge damage.

⚠ **A red suite run under load is not evidence.** Re-run red on a quiet box before reporting it, and diff
the failure *sets*, not the counts. Green under load is still green.

---

## 5 — Review the QUIET files

> **A conflict marker fires on overlapping edits to the same LINES. It cannot fire on a dependency
> between DISTANT lines. No merge tool detects that — it is what "merge" means.**

So **reviewing the conflicted files is not reviewing the merge.** The auto-merged set, larger and
unexamined, is where this class lives.

> **Worked example, 2026-09-09.** Upstream removed an early `if (!mod) return` in `useKeyboardShortcuts`
> and pushed the modifier check into each branch. Our Tab-cycling sat between those branches and relied
> on the gate that vanished. The file **auto-merged with zero conflicts** and swallowed a plain Tab, so
> shell completion would have stopped working.

The instrument is a test pinning behaviour nobody is currently changing, which is the argument for keeping
tests that look redundant.

**Second instrument, cheap: diff our per-file delta before and after.** For every file we change, our delta
against upstream should survive the merge unchanged:

```bash
P=$(git rev-parse HEAD^1)                      # fg-main before the merge
B=$(git merge-base HEAD^1 HEAD^2)              # the upstream it was based on
for F in $(git diff --name-only $B $P); do
  before=$(git diff --numstat $B $P -- "$F"); after=$(git diff --numstat upstream/main HEAD -- "$F")
  [ "$before" = "$after" ] || echo "$F: $before → $after"
done
```

Identical counts mean our feature rode through unchanged. Anything listed needs a reason you can name. On
2026-09-19 a `both` resolution had kept upstream's one-line `<FilesTab>` beside ours, rendering the tab
twice; no test covered it, and the count did.

**Then push.** It is an ordinary fast-forward push; no force, no backup ref:

```bash
git push origin fg-main
git ls-remote --heads origin fg-main          # quote this SHA in #velaterm
```

---

## 6 — Build (every machine, independently)

**Build in a fixed worktree outside the vault, never in the live repo.** `$FG_VAULT_ROOT/Code/VelaTerm` is
git's only transport for that tree, and a build checks out whatever it ships. A worktree shares the live
repo's objects and needs no second clone.

```bash
VAULT="${FG_VAULT_ROOT:-$HOME/_fg3}"
git -C "$VAULT/Code/VelaTerm" fetch origin fg-main
WT="$HOME/.velaterm-build"
[ -d "$WT" ] || git -C "$VAULT/Code/VelaTerm" worktree add --detach "$WT" origin/fg-main
git -C "$WT" checkout -q --detach origin/fg-main
NEW=$(git -C "$WT" rev-parse HEAD); echo "${NEW:0:7}"   # the SHA you are shipping; quote it from here on
```

**Windows, first build in a new worktree:** `src-tauri/resources/gitbash` is gitignored, and the build
refuses to start without it. Copy it from the install:

```bash
[ -f "$WT/src-tauri/resources/gitbash/usr/bin/bash.exe" ] ||
  cp -r "$LOCALAPPDATA/VelaTerm fg/resources/gitbash" "$WT/src-tauri/resources/"
```

**Prerequisites:** a Rust toolchain (`rustup` stable; `cargo` on PATH), and `pnpm` or `corepack` on PATH.
`build-fg.sh` needs one of the latter two. Without either, a shim that runs `npx pnpm@<version>` works, with the
version from `package.json`'s `packageManager` field. On 2026-10-18 the Mac had neither pnpm nor corepack,
and its `~/.cargo` and `~/.rustup` had been deleted.

Then build, **backgrounded** (cold ≈ 11 min, warm 3-5):

```bash
cd "$WT" && pnpm install --frozen-lockfile
export CARGO_TARGET_DIR="$HOME/.velaterm-target"
scripts/build-fg.sh --swap    # Windows → $CARGO_TARGET_DIR/release/velaterm-fg.exe
scripts/build-fg.sh           # macOS   → $CARGO_TARGET_DIR/release/bundle/macos/VelaTerm fg.app
```

`scripts/build-fg.sh` is the one build command on both machines. It builds the fg edition
(`tauri.fg.conf.json`: its own name, data dir and binary, updater off). There is no patch step: the
Adventure Time theme is in the tree (`src/themes.ts`), and `themes.test.ts` pins it in step 4's gates. The
retired source patcher (`.claude/_tools/spaces/adventuretime-theme/build_velaterm_theme.py`) must not be run.

- **`VLX_USER_INTEGRATIONS`** defaults to `on`: the edition owns the `vela` command and the `~/.claude` /
  `~/.codex` skills. Set it to `off` only on a machine that still runs the official VelaTerm.
- **Evo:** the Open-Model-Testing lab runs GPU evals there. Check the mesh `roster` for the lab's
  sessions (the members of `open-model-testing`). If one is online, post in `open-model-testing` and wait for its
  OK; if none is, go ahead. Either way, build with `CARGO_BUILD_JOBS=2`.
- **The build cache must postdate the `_fg2` → `_fg3` move.** A target dir from before it fails with
  `failed to read file '\\?\C:\_fg3\…\app_hide.toml'`. The fixed `CARGO_TARGET_DIR` above avoids it; if an
  older one is in use, delete it.
- `M src-tauri/Cargo.toml` in the worktree is a **`core.autocrlf` phantom**: `git diff --ignore-cr-at-eol`
  returns empty.

---

## 7 — Swap the install (every machine, independently)

**Every build reports `ProductVersion` 0.2.9, so the version proves nothing. The commit is embedded:**

```bash
commit_of() { LC_ALL=C grep -aoE '[0-9a-f]{40,}' "$1" | awk '{for(i=1;i<=length($0)-39;i++) print substr($0,i,40)}' |
  sort -u | comm -12 - <(git -C "$WT" rev-list --all | sort); }
```

It prints the full SHA a binary was built from: about 1s on Windows, 9s on macOS. It tests every 40-character
window of each hex run, because the macOS linker packs the version string directly before the SHA
(`…0.2.98fd603d…`), and a plain `grep -o '[0-9a-f]{40}'` then returns a window shifted by one digit that
matches no commit. Empty output means no known commit, never "check passed". Each binary has given
exactly one SHA so far; compare with `grep -qx "$NEW"` rather than `=`, so a second line cannot fail a correct build. Name each backup for it: a backup named
for a SHA can be rebuilt with step 6, and one named `.old` or for a version cannot.

### Windows

Claude sessions run inside `velaterm-fg.exe`, so **rename, then copy**. Windows lets a running binary be
renamed, and the live app keeps running off the renamed file. An installer closes the app and kills the
session doing the swap.

```bash
DIR="$LOCALAPPDATA/VelaTerm fg"
powershell -NoProfile -c "Get-Process | ? Path -like '*VelaTerm*' | Select Name,Path"   # must list it
OLD=$(commit_of "$DIR/velaterm-fg.exe"); echo "${OLD:0:7}"
mv "$DIR/velaterm-fg.exe" "$DIR/velaterm-fg-${OLD:0:7}-backup.exe"
cp "$CARGO_TARGET_DIR/release/velaterm-fg.exe" "$DIR/velaterm-fg.exe"
commit_of "$DIR/velaterm-fg.exe" | grep -qx "$NEW" && echo swapped
# keep exactly one backup, the one just made
for f in "$DIR"/velaterm-fg-*-backup.exe; do [ "$f" = "$DIR/velaterm-fg-${OLD:0:7}-backup.exe" ] || rm -- "$f"; done
```

The process check must print the running app before its silence means anything: a check that cannot see a
running app returns the same nothing as one that is correctly gone.

### macOS

**Stage beside the running app; never swap under it.** The agent session is a child of the app, so it
cannot be what quits it. `pgrep` is blind to VelaTerm on the Mac; use `ps`, and prove it prints a positive
while the app is up:

```bash
ps -Ao pid,comm | grep "VelaTerm fg.app/Contents/MacOS/velaterm-fg"
OLD=$(commit_of "/Applications/VelaTerm fg.app/Contents/MacOS/velaterm-fg")
ditto "$CARGO_TARGET_DIR/release/bundle/macos/VelaTerm fg.app" "/Applications/VelaTerm fg-${NEW:0:7}.app"
commit_of "/Applications/VelaTerm fg-${NEW:0:7}.app/Contents/MacOS/velaterm-fg" | grep -qx "$NEW" && echo staged
```

`~/velaterm-swap.sh` (fg runs it from Ghostty) waits on that `ps` check, renames the old bundle to
`VelaTerm fg-<old-sha>-backup.app`, moves the staged one into place, keeps one backup, and relaunches. It
was rewritten for `VelaTerm fg.app` on 2026-10-18, and its `ps` wait was proved positive while the app ran.

---

## 8 — fg restarts

**This is fg's only step.** Post the old and new SHAs in `#velaterm`, then ask him to restart VelaTerm
(Windows) or run `~/velaterm-swap.sh` (macOS). The restart ends this session; he resumes it.

There is no restart checklist. fg tries new features on Evo first, before the other machines pull them, and
brings any problem back to that session. Rollback, if he asks for it: quit, rename the new binary aside,
rename the backup back, reopen.

---

## 9 — Report what is new, check what you can, report complete

**List what this build adds over the one it replaced:**

```bash
git -C "$WT" log --no-merges --format='%h %an  %s' "$OLD..$NEW"
```

fg's commits are his features. Upstream's are summarised per release, not listed one by one. Then, for each
fg feature:

- **The agent can check it itself:** check it. The commands below run on the new build.
- **It needs a person** (a click, a key, how something looks): one line inviting fg to try it, saying where.
- **Then report the upgrade complete** in `#velaterm` and to fg: the machine, `old → new`, what was checked
  and what passed, and what is left for him to try. If a check fails, say so and stop. Do not roll back
  unless he asks.

On Evo, the first machine to get a feature, test the feature fully with fg. On the other machines,
confirm it is present.

**Checks for recent features** (add a row whenever a feature lands on `fg-main`):

| Feature | Since | Check |
|---|---|---|
| New build running | every upgrade | `commit_of` the installed binary prints `$NEW`, and the app's process started after the swap |
| `vela` command and skills managed by the edition | `350733c` | Windows: `Get-Command vela` finds a `vela.cmd` whose first line is `@REM VelaTerm managed vela command` (it goes in the first writable user PATH folder; on Evo that is `AppData\Local\agy\bin`, not `~/.local/bin`); `~/.claude/skills/vstat/SKILL.md` is dated after the restart. macOS: `vela` installs only from Settings (it needs admin) |
| Agents may call vspawn | `d47a663` | `grep -c disable-model-invocation ~/.claude/skills/vspawn/SKILL.md` → `0` |
| `vstop` | `8fd603d` | `vstop --help` exits 0; `vstat --json` has `stateSince`; `~/.claude/skills/vstop/SKILL.md` exists. Evo, once (passed 2026-10-18): spawn a throwaway session, `vstop` it, `vstat` shows `alive: false`, `vtell` resumes it |
| WSL probe skipped with no distribution | `7bec7d6` | Windows without WSL: invite fg to open a new terminal and confirm no `wsl.exe` error appears |

---

## Old backup refs

`origin` still holds `backup/pre-<version>-regen` refs from the regeneration era. Each one is typically
the only ref that reaches its commit, and nothing in this runbook needs them any more. Delete them only
when fg asks: deleting one orphans that commit permanently.

---

## The debug build — the instrument for any UI bug here

`pnpm dev:desktop` in the build worktree (`~/.velaterm-build`) runs a **second VelaTerm alongside the installed one** and
ships DevTools. Debug builds skip the single-instance plugin (`lib.rs` — grep `tauri_plugin_single_instance`, currently ~451, `#[cfg(not(debug_assertions))]`)
and compile in `open_devtools` (`commands.rs` — grep `pub fn open_devtools`, currently ~750), which release builds do not. Cold build ≈ 7 min. This
does not disturb the session running inside the installed app.

---

## Traps index

| Trap | Instrument |
|---|---|
| Tags run a release behind; some releases untagged | quote the SHA |
| Branch containment ≠ concern superseded | `git log --all -S"<symbol>"` |
| Conflict markers miss semantic coupling | tests that pin unchanged behaviour |
| Suite verdict is load-dependent | re-run red quiet; diff failure *sets* |
| A PR branch cut from `fg-main` carries fg-only work | cut PR branches from `main` |
| Two machines merging the same release | claim the sync in `#velaterm` first |
| Building in the live repo | the fixed worktree `~/.velaterm-build` (step 6) |
| Build cache from before the `_fg2` → `_fg3` move | a fresh `CARGO_TARGET_DIR` (step 6) |
| A heavy build on Evo starves the lab's GPU evals | if the lab is online, its OK in `open-model-testing` first; `CARGO_BUILD_JOBS=2` |
| A backup named `.old` or for a version | name it for the SHA `commit_of` finds in the binary (step 7) |
| `commit_of` silent on a macOS binary | test every 40-char window; the version string runs into the SHA (step 7) |
| No `pnpm`/`corepack`, or no Rust toolchain | step 6 prerequisites |
| `M Cargo.toml` after checkout | `git diff --ignore-cr-at-eol` → empty |
| `pgrep`/process-name blindness | prove the positive first |
| Closing VelaTerm kills the session | rename-then-copy (Windows), stage beside it (macOS); fg restarts |
| A session cannot observe its own restart | ask fg; transcript ≠ process continuity |
| `$?` after a pipe is the pipe's last stage | redirect to a file, read `$?`, then filter |
| Two paths differing only by case (`fontSelect.test.tsx` / `FontSelect.test.tsx`) | APFS and NTFS hold one file for both index entries; `git ls-files \| grep -i` — keep one name |
| `security::*` tests red on macOS | `PYTHON=$(uv python find 3.12)`; the system 3.9 has no `tomllib` |
| A `both` resolution that keeps upstream's line *and* ours | per-file `--numstat` before and after the merge (step 5) |
| `\| head` on a supersession check reads a truncated list as complete (NO_COLOR, 2026-09-19: 11 hits, 5 shown, the decisive ones were #6–9) | `wc -l` first, or no `head` when the question is "does upstream already have this" |
