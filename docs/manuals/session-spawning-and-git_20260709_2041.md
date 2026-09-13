# Session Spawning & Git Collaboration

Created: 2026-07-09 20:41 · Updated: 2026-09-13

> Two features that work as a pair: spawning subtasks into independent child sessions with `vspawn` (optionally in isolated git worktrees), and a graphical merge to bring parallel branches back together. Together they make "several agents working the same repo in parallel" an everyday workflow.

## 1. What spawning is

From inside any session, split a subtask off: VelaTerm creates a new child session **under** the current node (the tree becomes hierarchical: parent → child → grandchild), optionally gives it an isolated git worktree, and feeds the task description in as the new session's **first message** — the new agent starts working the moment it boots, no re-explaining needed.

What you get is a real session in the tree — its own process, fully interactive, resumable — that you can watch or step into at any time. It is not a hidden background task.

## 2. Four entry points

| Entry | Who uses it | Worktree |
|-------|-------------|----------|
| Terminal command `vspawn "task"` | You, in any session | Off by default; `--worktree` enables |
| Terminal command `vspawn-tree "task"` | Same | Always on |
| `/vspawn task` in Claude or `$vspawn task` in Codex | The agent spawns it (and expands the task into a rich self-contained prompt) | Off by default |
| `/vspawn-tree task` in Claude or `$vspawn-tree task` in Codex | Same | Always on |

The two terminal commands are injected into every session's PATH automatically — zero install.

Both commands use the directory where they are invoked as the child's starting directory and worktree repository.
When launching a task for another repository, pass `--cwd <path>` explicitly, e.g.
`vspawn-tree --cwd /work/project "fix the parser"`. This is especially important in a collection, which has no
project directory of its own. The agent skills add `--cwd` automatically when the conversation identifies one
unambiguous repository.

Both commands also take `--model <name>` and `--effort <level>` for the child session, e.g.
`vspawn --model opus --effort high "port the parser"`. Model names and effort levels belong to the agent that will
run, and VelaTerm turns them into that agent's own flags (`--effort` for Claude and Kiro, `--reasoning-effort` for
Grok, `--thinking` for Cline, and so on); an agent whose CLI has no effort setting does not offer this control, and explicit unsupported overrides are rejected before creating a session.
Without these flags the child inherits the parent session's launch arguments, which is the previous behaviour.

> **Prerequisite for agent skills:** enable **Vela Skills** in Settings ▸ General. This installs `vspawn`, `vspawn-tree`, and `vopen` into both `~/.claude/skills/` and the Codex skills directory (`$CODEX_HOME/skills` when set, otherwise `~/.codex/skills`); they're kept up to date automatically on app upgrades. After enabling, start a new Claude or Codex thread so the agent picks them up. Without this, only the terminal-typed `vspawn` / `vspawn-tree` commands work.

### Planning, execution and review

The **New Session → New Planning and Execution Session…** menu opens the same launch dialog with an empty task. Select the planner and executor’s agent, model and effort, enter the working directory, and choose the current directory, one shared worktree, or a separate worktree for each session. Opening or cancelling the dialog creates nothing. The planner is created in the selected project, group or parent session; the executor is its child. There is no extra initiating conversation, so the final audit stays in the planning conversation. The task field accepts pasted images with previews and removal controls. In plan-execute mode, images accompany the planner’s initial task and the executor’s first assignment; retries preserve the original attachments. Ordinary spawned agents receive uploaded image paths, as with terminal image paste. Invalid model identifiers remain editable in the dialog and are rejected before launch, with the specific reason shown. The dialog has a directly accessible URL and supports browser back and forward navigation.

`vspawn --plan-execute` creates a new planning session under the initiating session. The planner prepares the task, creates one executor beneath itself, and reviews the executor's reports. Corrections go back to that same executor. The planner accepts the result only after checking the actual changes and verification evidence.

```bash
vspawn --plan-execute \
  --plan-agent claude --plan-model opus --plan-effort high \
  --exec-agent codex --exec-model gpt-5.5 --exec-effort medium \
  "Implement the agreed changes and verify all acceptance criteria"
```

The six role settings are independent and editable together in the existing confirmation card. Agent choices come from the backend and currently include Claude, Codex, OpenCode, Pi and OMP. A model or effort must be supported by the selected agent and model; provider rejection is reported rather than treated as success. Changing an agent clears only that role's model and effort draft. Ordinary agent flags, `--model` and `--effort` provide planner defaults when the corresponding role flags are absent. Without an explicit agent, a chat-capable parent supplies its kind; other parent types default to Claude.

Model and effort suggestions use the backend's agent adapters: Claude's version-aware catalogue, Codex's `model/list`, OpenCode's connected providers and model variants, and Pi/OMP's `get_available_models` with reasoning metadata. Pi and OMP model selectors retain the provider to distinguish identical model IDs. Discovery uses the configured executable and, where applicable, the task's working directory. Selecting a model shows its supported effort levels and clears an incompatible previous effort; leaving the model unspecified shows the catalogue's combined levels. A failed lookup displays its error and a retry action while preserving the draft. Both launch entry points use these fields, as do ordinary and batch spawn dialogs.

The launch dialog or `--worktree-mode` option selects `run.config.worktreeMode`: `none` keeps all workflow conversations in the requested directory; `shared` gives the planner and every executor one new worktree; `each` gives the planner its own worktree and each executor a separate worktree and branch. Executor worktrees in `each` mode start from the planner's current commit and are stored beside the other worktrees. Creation failure prevents the affected session from starting; it does not fall back to a shared directory. New worktrees exclude uncommitted changes. The CLI's existing `--worktree` flag selects the shared mode unless overridden by an explicit mode or in the dialog. Older workflows retain their shared-directory behavior. Include the current conversation's decisions, existing modifications, constraints and required delivery location in the task; separate conversations receive an explicit task handoff, not a complete copy of the initiator's history.

Communication uses VelaTerm's authenticated local endpoints and the existing native chat engine. The executor submits results with `vtell --report --round N`, reading the current round from `vflow status`. The backend automatically targets that executor's planner and changes the round to review. A report sends a message immediately, or queues it if the planner is busy. The backend listens for turn completion and process exit; a turn that ends without the required handoff blocks the workflow. Normal coordination does not poll. Opening another tab does not stop an active workflow.

Messages display the originating agent's icon, session name and planning or execution role. Platform failure notices use the VelaTerm identity. Attribution is restored from the backend delivery ledger when reopening a conversation. The provider receives an ordinary user-role input with source metadata; this is VelaTerm orchestration, not a provider-specific cross-session messaging API.

The backend supplies each role with the workflow protocol. Dispatch, acceptance, blocking, stopping and status use `vflow`; execution reports use `vtell --report`. Use `vflow status <workflow-id>` for a manual health check, and `vflow stop <workflow-id>` to stop further rounds and request interruption. Interrupt errors are returned explicitly; an operation already in progress may take time to stop. Reports and corrections use stable message IDs, so retrying an uncertain request cannot silently duplicate it. A blocked workflow still accepts ordinary `vtell` messages, which leave its state unchanged. Its assigned executor can submit the current round directly with `vtell --report`, moving it to review without another dispatch or a round increment. A new planner dispatch assigns further work; completed or explicitly stopped workflows reject new execution reports. Permission requests still require the user's answer. If the initiator uses a terminal, its final report remains available in the planning conversation and workflow record.

This mode requires a build containing the new backend and command shims. Updating the skill text alone does not add messaging support to an older running VelaTerm process.

### Send a message to another session

`vtell` sends a message to an existing native chat conversation. It accepts a full session ID, an ID prefix of at least eight characters, or an unambiguous session name. The recipient sees the sender's agent icon and session name in its chat history, including after reopening. An idle recipient starts a new turn; a busy recipient queues the message. A stopped native chat process can resume in the same conversation. Archived sessions, self-targets and sessions without native chat support are rejected.

```bash
vtell <session-id> "The requested information"
vtell <session-id> --message-id msg-UUID < message.txt
vtell --report --round N --message-id msg-UUID < result.txt
```

Ordinary messages do not change workflow state. Only `--report` submits an executor's result for review; it requires the recorded round and targets the associated planner. An explicit report target must identify that planner. When the target is omitted, provide the report on stdin. Messages support up to 65,536 bytes of UTF-8 text. Retain the message ID and exact contents for retries; when no ID is supplied, the command generates and prints one before sending. A `sent` or `queued` receipt confirms submission, not completion of the recipient's work.

The `vtell` skill is included in the Vela Skills bundle for Claude and Codex. Its backend uses the same native chat adapters as the workflow: Claude, Codex, OpenCode, Pi and OMP. Ordinary session messages have their own delivery records; workflow reports retain their workflow ID, round and execution role.

## 3. Confirm before spawn

By default every spawn first shows a confirmation card (top-right, non-modal, doesn't steal focus):

- The card shows the requesting session and working directory, followed by the task instructions, run settings, and directory choices. Task instructions become the child's first message. A plain terminal opens the directory without executing those instructions.
- Model suggestions and reasoning options come from the backend. You can enter a model identifier manually if no suggestions are available. A request's explicit choices take precedence over inherited settings; clearing a model or effort field restores the agent's default for that setting.
- “Start session” starts the child; “Cancel” discards the request. Additional requests remain in the queue, with their count shown in the card. The card does not take focus, and its contents scroll within a short window while the action buttons remain visible.
- To skip confirmation for individual requests, turn off “Confirm before spawn” in Settings ▸ Behavior.

### Review a batch of sessions

Enable **Automatically split into multiple tasks** in the planning/execution launch dialog, or pass `--split-tasks` with `vspawn --plan-execute`. A new planner inspects the task and submits a proposal with `vflow propose`. One planner manages all execution sessions, with a separate workflow ID and correction round for each task.

The proposal is saved by the backend and opens **Review execution tasks**. Select tasks from the list to edit their names, instructions, agents, models and reasoning effort. Removing a task can be undone. Switching tasks preserves draft edits. The dialog has a stable URL, can be opened directly, and supports browser back and forward. Closing it leaves the proposal pending; the pending-proposal links reopen it. Cancelling blocks the workflow and creates no executors. A refreshed page reloads the saved proposal; unsubmitted form edits remain temporary.

Menu launches have two confirmation stages: the initial dialog configures the planner, execution defaults, task splitting and directory mode; after planning, the task-review dialog lets the user confirm each proposed task and its execution settings. Confirmation saves the approved tasks before starting their execution sessions. Model and effort options come from the backend.

When invoked as a skill with task splitting, `vspawn` and `vspawn-tree` automatically add `--yes`, unless the user explicitly asks to review the initial configuration. The normal path therefore shows only the final task-proposal dialog; `--yes` never bypasses that review. The planner starts with the supplied settings, with omitted values resolved by the backend. The final dialog can edit each executor's settings, but not the already-started planner's settings or directory mode. Non-split skill invocations and plain CLI commands retain their existing launch-confirmation behavior; a plain command needs an explicit `--yes` to skip the initial card regardless of the confirmation setting.

Pass `--worktree-mode none|shared|each` with `--plan-execute` to select the directory mode without a launch dialog. The explicit mode overrides the legacy `--worktree` flag regardless of argument order. Without it, `vspawn` uses the current directory and `vspawn-tree` shares one new worktree. The final task dialog retains the saved mode: executors share the planner's directory in `none` and `shared` modes, and each gets a separate worktree in `each` mode. If immediate planner launch fails, the original request and settings remain available for retry; this recovery card is not a second confirmation in the successful path.

See the [article and video examples](../samples/plan-execute/README.md) for a dependency-free demonstration project, both dialogs, and review and recovery scenarios.

Each executor still uses `vtell --report --round N`. Reports arrive in the same planning conversation, tagged with the task workflow ID and round. The planner dispatches corrections and accepts results using each task's workflow ID; sibling rounds stay unchanged. Overall acceptance is rejected until every task is accepted. Missing reports, failed delivery and blockers retain their task identities for recovery. Retrying an uncertain confirmation preserves the approved assignments and starts only undelivered work.

The former `vorch` skill, command shim and batch creation endpoint have been retired. Its historical database records remain readable through the legacy `vstat` filters; new workflows use `vflow status <workflow-id>` for task progress and acceptance state.

## 4. Worktrees: parallel without stepping on each other

With the worktree option on, the child session works on a new branch in its own working directory — isolated from the main workspace and from other children. That's the right shape for multi-agent parallelism. Management lives in the session context menu under "Worktree ▸": view changes, copy / open the worktree folder, delete the worktree (with optional force to discard uncommitted changes).

Spawning from a non-git directory still works; it just falls back to sharing the parent's directory, with no worktree.

## 5. Merge: graphical Git merge

When the work is done, bring it home. Any session whose working directory is a git repo (worktree or not): right-click → "Git ▸ Merge…" opens the merge dialog:

- **You pick both source and target branches**, and can swap direction — merge a child's branch back into main, or pull main into the child to refresh its baseline, all from the same dialog.
- The merge executes in the working tree where the target branch is checked out; if the source tree has uncommitted changes, they're committed first so nothing is lost.
- On conflict, the scene is left in place for you (or an agent) to resolve in the terminal and continue.

Merging does not auto-delete the worktree — clean it up afterwards via "Worktree ▸ Delete worktree…" once you're sure.

## 6. A typical workflow

1. In the main session, have claude analyze the task, then `/vspawn-tree refactor the checkout module` to spawn it (requires Vela Skills — see §2).
2. Review the prompt on the confirmation card and hit Launch — the child starts in its own worktree, visible in the sidebar under its parent, dot turning green.
3. You keep working in the main session; when the child asks something or finishes, its dot turns yellow and a notification fires.
4. Step in to review, then right-click → "Git ▸ Merge…" to merge its branch back into main.
5. "Worktree ▸ Delete worktree…" to clean up, then archive the child session for the record.
