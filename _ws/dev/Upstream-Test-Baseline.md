---
Type: data
Date Created: 2026-10-18 15:10
Last Updated: 2026-10-18 15:10
Tags: [velaterm, testing]
---
# Upstream test baseline

Upstream `main` is not green, so measure against it, not against zero. Kept from `Next-Steps.md` (retired 2026-10-18). The table was measured 2026-08-22 at `d098f5a`, with our commits stashed so the comparison was clean; re-measure before relying on the numbers.

| Check | Result on upstream `main` |
|---|---|
| `pnpm lint` | **fails** — 2 errors, `src/remote/ConnectionBanner.tsx:47,84`, dynamic `@tauri-apps` imports violating their own `no-restricted-syntax` rule |
| `pnpm test` | **fails** — 3 files; 3–4 individual tests. The count varies run to run, so at least one is flaky |
| `cargo fmt --check` | **drifts** — many files. Known upstream as #41 (closed) |
| CI | **none** — no `.github/`, no Actions. Nothing runs on a PR |

**Do not try to fix these inside a feature PR.** #39 criticised a branch precisely for bundling unrelated changes. The right move is what PR #50 did: state that your commits leave these results unchanged, and leave them alone.

**Consequence for our own discipline:** "all checks pass" is not available as a completion signal here. Verify each change directly instead: reproduce the failure, apply the fix, reproduce the pass, and prove the new test fails without the fix.

⚠ **Run vitest as `vitest run --pool=threads` on Windows.** The default forks pool fails to start workers there (11–30 `Failed to start forks worker` per run); one run executed 1 of 31 files and looked like a catastrophic regression that did not exist. Compare against the same pool on the same machine.

The Rust suite has its own Windows baseline: `cargo test --lib --no-default-features` has known Unix-spawn failures, so compare failure *sets* against upstream `main`, not counts. Flags and load caveats: [[Code/VelaTerm/_ws/notes/Upstream-Sync-Runbook|Upstream Sync Runbook]] § 4.
