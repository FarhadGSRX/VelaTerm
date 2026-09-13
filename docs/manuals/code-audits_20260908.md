# Code audits

Open a project's context menu, choose **Experimental** → **Code audit**, then **New audit**. Select **Codex** or **Claude Code**, a model and reasoning effort, and the source scope. Choose **Start audit**. The report page shows the actual scan phase; **Agent session** shows the original tool calls, analysis, and pending permission requests. Resolve requests there to continue. The report appears when Codex Security seals its results.

The audit runs on the machine hosting the project. It uses the selected local CLI's existing executable, login, configuration, and permission handling. VelaTerm adds a session-local Codex Security MCP server without replacing other configured MCP servers or introducing a model API client. The native CLI must be installed, authenticated, and within its account limits. Native CLI configuration may itself select a provider; VelaTerm does not rewrite it.

## Models and reasoning effort

Choices come from VelaTerm's backend model catalogue. **Default model** retains the native selection. An explicit model can expose supported reasoning-effort levels; **Auto** retains native effort selection. The backend validates explicit combinations, and the audit saves the selected values for history and reruns. Catalogue availability does not establish account entitlement. Provider failures, including exhausted usage limits, remain failed audits.

New audits request analysis in the interface language. Protocol fields, paths, identifiers, and quoted source evidence remain unchanged.

## Codex Security integration

VelaTerm bundles the public Apache-2.0 release `@openai/codex-security@0.1.26`, whose plugin manifest is version `0.1.95`. Its MCP runtime identifies itself as `0.1.159`; these are separate upstream version fields. The original skills, references, schemas, scripts, runtime, and license are retained under `src-tauri/resources/codex-security/`. `UPSTREAM.json` records the package source, integrity value, and file hashes.

Every new audit registers a real upstream workbench scan. The native agent reads the bundled entry skill and uses the upstream MCP tools and canonical artifact contract. This replaces the earlier independently implemented fixed-size batch workflow. Historical records from that workflow remain readable and are explicitly identified as legacy results.

Both engines use the same upstream workflow. The public TypeScript SDK binds the Codex runtime and does not expose a Claude Code executor, so VelaTerm integrates through the upstream MCP server and local workbench. Claude Code support is a VelaTerm adapter; it is not an upstream claim of official Claude support.

The runtime requires Node.js supported by the upstream package (22.13+ on the supported 22, 24, or 26 lines) and Python 3.10+. Python 3.10 also needs `tomli`; Python 3.11+ supplies `tomllib`. The application materializes verified resources in its data directory. It does not install them into the user's Codex or Claude configuration.

## Scope and workflow

- **Whole repository** uses the upstream Standard scan with a fixed target snapshot and repository security policy.
- **Directory or file** uses Standard with an exact relative include path. Supporting source may be read to understand a call chain, without claiming that supporting areas received a full audit.
- **Working tree changes** uses the upstream diff skill and a fixed Git change set. It includes staged, unstaged, and untracked changes supported by the upstream target resolver, and retains baseline evidence for deleted files.

Standard reads `skills/security-scan/SKILL.md` and `references/core-scan.md`. It performs preflight checks, independent baseline and architecture analysis, source-anchored investigations, candidate checkpoints, counterevidence review, validation, and coverage reconciliation. Independent workers are used when the native runtime supports them; otherwise the upstream sequential fallback and its limitations must be disclosed. A worker-capacity setting alone is not evidence that independent workers ran.

Working-tree audits read `skills/security-diff-scan/SKILL.md`. They fix the target, build the threat model, prepare and enumerate review items, discover candidates, validate them, analyze attack paths, and submit the semantic result draft for completion. Reviewing only the current contents of changed files is not a substitute for this flow.

The upstream workbench currently accepts directory scopes. To preserve VelaTerm's existing file selection, the installed runtime applies two narrow, hash-recorded patches: permit an existing file as the scope and count that file as one snapshot entry. Original bundled files remain unchanged; the installed `ADAPTER.json` records both original and adapted hashes. This does not expand a selected file into a whole-directory scan.

The audit instruction prohibits source edits, remediation, publication, and native configuration changes. Native tool permissions remain in force. These instructions are not an operating-system sandbox. Validation must state what was actually checked; a static finding does not imply successful exploit reproduction.

## Reports, coverage, and cancellation

The authoritative results are the upstream `scan-manifest.json`, `findings.json`, and `coverage.json`. The upstream finalizer validates and seals them, then generates the readable report. VelaTerm imports those artifacts and checks finding locations and exact source lines, treating LF and CRLF as equivalent line endings. Structural validity and source provenance do not establish exploitability by themselves. If source evidence fails verification, the audit remains **Failed** and findings are not accepted as validated. The sealed upstream report remains available for inspection; Markdown exports prepend the host validation failure without rewriting the original report.

Before sealing, the adapter checks draft paths, line ranges, and excerpts with the same source verifier used during import. If evidence does not match, the scan remains unsealed and the native agent receives the affected locations so it can correct the draft. The original Codex Security completion tool then generates and seals the report. This check does not rewrite excerpts, remove findings, or modify sealed results. Import repeats the verification so later source changes cannot silently turn invalid evidence into a successful result.

Coverage comes from upstream review receipts and dispositions, not from an agent's claimed count of files read. Unresolved candidates, deferred work, exclusions, or completion warnings remain visible. A sealed scan with partial coverage is shown as **Partial coverage**. Even complete coverage does not guarantee that all vulnerabilities were found.

When the agent finishes a chat turn while the scan still has work to do, VelaTerm continues the same session and scan. It retains existing findings and worker results. Permission requests remain pending, and cancellation or a provider error stops automatic continuation. If three consecutive turns finish without changing the upstream review progress or saved findings and coverage, the audit stops with an incomplete-scan error and retains its checkpoints.

The report page presents retained findings, source evidence, and coverage limits. Expand a finding through its own URL to read its attack path, validation method and conclusion when supplied, remediation advice, and reported source excerpts. Check the audit status before treating these excerpts as verified. The separate **Report artifacts** tab retains the complete upstream-generated report and scan ID. Running scans show the upstream package and plugin versions, actual phase and progress, and warnings. Before a sealed report exists, progress and checkpoint information are shown instead of an empty final assessment. The audit conversation's ordinary composer and rewind are disabled so an unrelated turn cannot replace the active workflow; permission requests remain usable.

**Cancel** stops the upstream scan and the native execution. Saved checkpoints remain available when the upstream workbench can seal them, with their canceled or failed status intact. Late results cannot revive a canceled audit. After a host restart, VelaTerm reconciles upstream state: a completed sealed result can be imported, while unfinished execution is marked interrupted and its retained artifacts remain distinguishable from completion.

**Start audit** on an existing record creates a new scan against the current target, using the saved engine and selection. It does not resume a stopped model turn or reuse an old vulnerability verdict.

Audit history, creation, report details, and conversation tabs have stable URLs supporting direct access, refresh, and browser navigation. On narrow screens, history and detail use separate views.

## Exports and capability boundaries

Markdown exports contain the upstream-generated report when available; otherwise they clearly identify themselves as an incomplete scan receipt. JSON exports retain the complete imported canonical data alongside application metadata. The upstream scan directory also retains its generated SARIF and other artifacts. Exports can contain source excerpts and vulnerability details.

This interface exposes Standard and working-tree diff audits. Other bundled skills, such as Deep scans, remediation, fix verification, and external issue tracking, are retained as upstream resources but are not separate supported actions in this interface. An audit does not apply fixes, publish findings, create issues, or open pull requests.

## Diagnostics

The backend records audit lifecycle and upstream phase metadata in `runtime-*.log` in the application log directory. `VLX_LOG_DIR` overrides the shared location; when it is unset, `VLX_SECURITY_LOG_DIR` overrides the location for security events; `VLX_SECURITY_LOG_LEVEL=error` limits output to failures and `off` disables it. Console and file output use the same timestamp, level, and job identifier format. Prompt and response metadata use sizes and SHA-256; full source, credentials, and full AI responses are not copied into this log. Provider token counts are cumulative session usage when available. The native conversation retains its normal recording.

For a failed audit, read the provider explanation on the report or open **Agent session**. Resolve login, runtime, permission, or account-limit conditions before starting another audit. A connected MCP server establishes tool availability; it does not establish that a model completed the scan.

Upstream references: [CLI overview](https://learn.chatgpt.com/docs/security/cli), [CLI reference](https://learn.chatgpt.com/docs/security/cli/reference), [public source](https://github.com/openai/codex-security).

Runtime logs use a bounded asynchronous writer. Queue acceptance does not guarantee persistence after a forced exit or disk failure. Custom model names are represented by process-local opaque identifiers; diagnostics do not retain raw provider messages.
