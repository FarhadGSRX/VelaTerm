---
name: evo-build-ram-limit
description: "Evo has only 32 GB RAM on the Windows side (96 GB reserved for GPU); heavy cargo/vitest runs can kill evo/visual-scientist's GPU eval runs"
metadata:
  node_type: memory
  type: project
  originSessionId: 9ffb5d2b-7c06-4313-a12e-91c2e12ee603
  modified: 2026-10-18T17:18:30.551Z
---

Evo-X3 reserves 96 GB for the GPU, leaving about 32 GB to Windows. On 2026-10-17 my parallel `cargo test` (target-r) plus vitest dropped free RAM below 4 GB, and evo/visual-scientist's (Open-Model-Testing) memory watchdog stopped a GPU model eval at item 336/405.

**Why:** model-eval queues run on Evo for hours, sometimes overnight, and are killed when free RAM gets low.
**How to apply:** on Evo, run cargo with `-j 2` and `--test-threads=2`, don't run cargo and a full vitest at the same time, and before any heavy build check the roster: if the lab (evo/visual-scientist, evo/omt-lead) is online, post in `open-model-testing` and wait for its OK. If it's offline, go ahead (fg, 2026-10-18). Related: [[velaterm-fg-roadmap]].
