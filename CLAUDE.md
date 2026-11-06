# VelaTerm Workspace — Claude Context

fg's fork of VelaTerm (upstream `vlinx-io/VelaTerm`), the terminal that hosts F-Co's sessions, and its Workspace.

## Local rules

- **The repo is public.** Before every push, run the outgoing diff (`git diff origin/<branch>..HEAD`) as its own step, and read it for anything sensitive about fg or the team before you push. Worklist lines and commit messages are outgoing text too, so refer to a private project by its role, not its name.
- **`_ws/` and `+Intro.md` live on `fg-main` only.** Upstream PR branches are cut from `main`, never from `fg-main`, so they carry neither.
- **On Evo, builds share RAM with the Open-Model-Testing lab.** If a lab session is up, ask in `open-model-testing` before a heavy cargo build. Build with `-j 2`, or `CARGO_BUILD_JOBS=1` for a release.

## F-Co

- Slug: velaterm
- Channel: ws-velaterm
- Lead: evo/velaterm-lead
- Crew: cx1/velaterm-crew, mac/velaterm-crew
- Loop: plan in the lead; Sonnet subagents build where a change is well scoped; `cargo test` and vitest, then a live check on Evo, verify; the crews install each build on CX1 and the Mac and run the runbook's check rows

## Build and test

- Rust, from `src-tauri/`: `cargo test -j 2 --lib --no-default-features -- --test-threads=2`. About 15 Windows-only tests fail on every run (mostly file locks), so compare the failure set with the previous run's instead of expecting zero.
- Frontend: `pnpm test` (vitest). On Evo, don't run it alongside cargo.
- Release build and install, on every machine: `scripts/build-fg.sh`, as the runbook says.

## Where things are

- Start with `+Intro.md`, then the newest file in `_ws/handoff/`.
- Worklist: `_ws/worklist/tasks.md`. Decisions: `_ws/Decisions.md`.
- Upgrading a machine to the current `fg-main`, or syncing upstream: `_ws/notes/Upstream-Sync-Runbook.md`.
- How the code works: `_ws/dev/`.
