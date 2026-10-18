# UX Roadmap — fg's wishlist

fg's own list of UX changes for VelaTerm, sized against the code on 2026-08-23. Sizing is from
reading, not from building — treat it as an estimate, not a promise.

**Eleven of fifteen items are shipped** on branch `feat/ux-settings-tab-and-paste-default` (items 1,
3, 4, 5, 6, 7, 8, 11). The branch is **pushed** to `origin` as of 2026-08-23. Each item below carries its own status. For the separate user-definable **theme** feature
see `Theme-Feature-Design.md`; item 2 below collides with it.

---

## First, the two answers that change the asks

### Everything in `PersistedSettings` persists for free

`src/store/settings.ts` defines one interface, `PersistedSettings`, serialised whole into a single
localStorage key `vlx-settings` (`saveSettings`), and mirrored to the backend `app_settings` table by
`pushSetting`. `vlx-settings` is already in `SYNCED_KEYS` (`src/ipc/settingsSync.ts:29`), so it also
syncs across shells (desktop / browser / remote).

**Consequence:** adding a new user setting costs *zero* persistence work — add a field to
`PersistedSettings`, add a default to `SETTINGS_DEFAULTS`, done. No migration, no new key, no Rust.
`loadSettings` already merges over defaults, so an older stored blob missing the field is safe.

The six settings that live *outside* that blob, each its own key and each also in `SYNCED_KEYS`:
`vlx-theme`, `vlx-lang`, `vlx-sound`, `vlx-notify`, `vlx-clean-images`, `vlx-record-sessions`.

### The "file tree in the right sidebar" is not a sidebar

There is no right *sidebar*. `src/layout/RightPanel/` is an inspector with **three tabs — Files,
Info, Git** — selected by `inspectorTab` and pointed at the current session's cwd. The file tree is
`RightPanel/FilesTab.tsx`. The **left** sidebar (`src/layout/LeftSidebar/`) is the session/project
tree — a different thing entirely, not a file browser.

So item 1 is not "move a panel to the other side"; see below.

---

## What the four mystery Behavior settings do

All four are documented in `src/store/settings.ts`; the doc comments just never reach the UI. This is
exactly the gap item 4 wants to close.

| UI label | Field | Default | What it actually does |
|---|---|---|---|
| **Tabs** (Single / Multi) | `singleTabMode` | `single` | Single-tab **reuses the current tab** for each session you open and keeps the previous tree alive in the background. Multi gives each its own visible tab. |
| **Dynamic status filter additions** | `dynamicStatusFilter` | `on` | Whether an active status filter's **frozen list is allowed to grow**. See the section below — the filter is a snapshot, not a live predicate. |
| **Background limit** | `maxLiveTabs` | `32` (8/16/32/64) | Cap on the **background pool** (`liveTabs`), which only fills in single-tab mode. Nothing to do with visible tabs. See the section below. |
| **Confirm before spawn** | `spawnConfirm` | `on` | Applies to **derived** sessions (`vspawn`), not to tabs you open yourself. On: a dialog lets you review and edit the spawn prompt first. Off: the session is created and started immediately. |

**Multi-tab vs single-tab, plainly:** single-tab is a "one window, many background workers" model —
you always look at one session, the rest keep running behind it up to `maxLiveTabs`. Multi-tab is the
conventional model. The default is **single**, which is why `Background limit` exists and why it
disappears if you switch to multi.

### Why the status filter is a snapshot

`statusFilterIds: Record<string, true> | null` (`termStore.ts:156`) is a frozen set of session **IDs**,
captured when you apply the filter. The sidebar renders that set; it does **not** re-evaluate the
predicate. The reason is flicker: agent status changes every few seconds, so a live filter would make
rows appear and vanish under the cursor mid-click.

`dynamicStatusFilter` therefore controls exactly one thing — whether newly-matching sessions are
**appended** to the frozen set. `appendSidebarTreeViewStatusMatches` (`termStore.ts:3249`) is
union-only: it adds, never removes. Sessions already in the list survive even after they stop
matching, in **both** modes.

- **Off** — apply "working" with 3 sessions working, and those 3 are the list forever. A 4th that
  starts working later never appears until you re-apply.
- **On** — the 4th joins the list.

Suggested clearer label for item 4: *"Add newly matching sessions to an active filter."*

### Two tab pools — and why the hybrid does not exist

From `termStore.ts:2100-2145`:

- **`openTabs`** — visible tabs. Always alive, never evicted, and **subject to no limit whatsoever.**
- **`liveTabs`** — the background pool. Populated **only** on the single-tab-mode path, where opening a
  session pushes the outgoing tab's pane tree into the background and reuses the tab slot.

**In multi-tab mode `liveTabs` stays empty and `maxLiveTabs` never fires** — hence the setting hiding
itself. Nothing is killed for being off-screen. (Multi mode having no cap at all is arguably the
bigger UX hazard.)

Two further details:

- Only tabs containing a **named** session get backgrounded (`oldHasNamed`); purely ephemeral tabs are
  deleted outright.
- Eviction uses `pickEvictTab`, which takes the oldest **inactive** tab. If every background tab is
  active it stops, sets `liveEvictAsk`, and prompts through `LiveTabsOverLimitDialog` instead of
  killing anything. Successful evictions post a `liveEvictNotice`.

**fg's hybrid** — several visible sessions plus a backgrounded remainder — does not exist as *tabs*,
but partly exists as *panes*: single-tab mode's one tab holds a `paneTree` that can be split, so
several sessions are visible while the rest sit in the background pool. Same model, different noun.
Implementing it as real tabs (N visible, remainder backgrounded) means teaching the single-tab swap
path to keep more than one entry in `openTabs` — a **large** change, bigger than anything else here.

---

## Item triage

Ordered cheapest-first, not by importance.

### 3. Image paste default → Native — ✅ **DONE** (branch `feat/ux-settings-tab-and-paste-default`)

`SETTINGS_DEFAULTS.imagePasteMode: "upload"` → `"agent"` (`src/store/settings.ts`). Safe everywhere:
`ImagePasteModeField` computes `effectiveMode = nativeAvailable ? mode : "upload"`, so browser and
remote shells keep uploading regardless of the stored value.

Flipped unconditionally — **fg reports native paste working fine on Windows** as of 2026-08-22, which
retires the #26 §2/§3 concern for our purposes. Re-check before proposing it upstream, since the
maintainer still lists those sections as open.

### 5. Settings reopens the last section — ✅ **DONE** (same branch)

`SettingsModal.tsx:656` is `const [cat, setCat] = useState<Cat>("appearance")` — pure component
state, so it resets every open. `Cat` is
`"appearance" | "terminal" | "behavior" | "advanced" | "agents" | "shortcuts" | "general"`.

Shipped as `SettingsTab` in `theme.ts` + a `settingsTab` field on `PersistedSettings`, with a
`setSettingsTab` store action mirroring `setInspectorTab`.

⚠ **Design note worth keeping.** The obvious move — read `cat` straight from the store — is wrong, and
the existing tests caught it. Section navigation must stay **local `useState`, seeded from the store
and writing through on change**. Reading it from the store routes every tab click through
`persistAndApplyVisual` (a debounced write plus a full visual re-apply) just to move between panels,
and it breaks any consumer holding the store as a static object — which is exactly how
`settingsBehaviorFields.test.tsx` mocks it.

### 6. Quit dialog "save workspace" — ✅ **FIXED** (commit `e0c7050`) — a real upstream bug

fg reproduced it on Windows 2026-08-23: ticked and confirmed, box still unticked next launch.

**Root cause — a race with process termination.** `setSaveWorkspaceOnQuit` reaches localStorage
synchronously, but `pushSetting` debounces the *backend* write by 400 ms (`settingsSync.ts:45`), and
`confirm_quit` kills the process well inside that window. Startup `reconcileSettings` then treats the
backend as authoritative and overwrites any differing local value. So the skipped flush does not
merely fail to save the answer — **it reverts the copy already written to localStorage.**

Fix: `await flushNow()` before approving the exit. That function is exported for exactly this case.

⚠ **Not specific to the checkbox.** *Any* setting changed in the last 400 ms before quitting was
reverted the same way. The quit dialog is just the one place where a write is always immediately
followed by termination. **Good upstream PR candidate** — narrow, reproducible, with a test.

### Why no dialog appears on the Mac — by design, not a bug

`lib.rs:922` gates the `CloseRequested` arm behind `#[cfg(not(target_os = "macos"))]`. On
Windows/Linux, closing the main window is intercepted so cancelling does not leave a windowless
background process. On macOS closing a window is not quitting (Cmd+W ≠ Cmd+Q), so that arm is
deliberately absent.

The `ExitRequested` arm is cross-platform, so **Cmd+Q on the Mac does prompt** — fg most likely
closed the window rather than quitting. Correct macOS behaviour; nothing to change.

### 4. Explain settings in the UI — ✅ **DONE** (same branch)

Shipped as an optional `hint` prop on `Field`, rendering the existing `Icons.info` glyph beside the
label with the text as a native `title` plus `aria-label`. Wired to all four Behavior settings.

Chose a tooltip over the codebase's existing dim-paragraph pattern (`recordSessionsHint`,
`imagePasteHint`, `cleanImagesHint`) because four paragraphs would dominate a dense panel, and the
hints are reference material rather than something to re-read every visit. `Field` now carries it
generically, so any other section can opt in.

**Known ceiling, recorded in the code:** a native `title` reaches pointer users and assistive tech
but **does not appear on keyboard focus**. A real popover component would fix that; the panel has so
far done without one.

⚠ **The 11-locale cost was mandatory, not optional.** `t()` does fall back per key
(`dict[key] ?? en[key]`), but every dictionary is declared `const ja: typeof en`, so adding a key to
`en.ts` fails `tsc` for the other ten until they all have it. 44 strings. The translations were
written here and **have not been reviewed by native speakers** — worth flagging on any upstream PR.

### 7. Ctrl+Tab tab cycling — **small-medium, but note what already exists**

**Cmd/Ctrl+1–9 already switches tabs by index** (`useKeyboardShortcuts.ts:33-34`, plus a `0` case at
:58). Cycling does not exist.

Two structural constraints, both from `src/hooks/shortcutRegistry.ts`:

- Registry combos are encoded `mod[+shift][+alt]+<single A-Z letter>`. **`Tab` is not representable**,
  so this cannot be a customizable `ShortcutAction`; it has to be a *structural* shortcut like the
  numerics, which the registry header explicitly reserves as non-customizable to avoid conflicts.
- With the terminal focused, xterm will consume Tab unless it is intercepted in `usePtySession`'s
  `customKeyEventHandler` — the same mechanism `APP_ALT_KEYS` uses for the Ctrl+Alt combos.

Worth deciding: true MRU cycling (hold Ctrl, repeated Tab, commit on release) is materially harder
than plain next/previous. Recommend next/previous first.

### 1. File tree as a left-panel view — **medium; the architecture invites it**

fg clarified the intent: the left panel already splits into **multiple independently-filtered views**,
and he wants a file tree available as one of them. That is a better fit than a panel swap.

`sidebarTreeViews` is already an array of views, each with `id`, `name`, `treeFilter`, `statusFilter`,
`statusFilterIds`, `markFilter`, `collapsedOverrides`, laid out by `sidebarTreeLayout.ts`
(`computeSidebarViewLayout` / `computeSidebarDividers`). The shape to add is a discriminator —
`kind: "sessions" | "files"` — with the render site choosing `FilesTab` over `ProjectTree`.

`FilesTab` is already prop-driven (`rootPath`, `rootName`), so it is reusable unmodified. The open
question is what a files view's root should be: the active session's cwd (like `RightPanel`), or a
pinned path chosen per view — the latter is more useful in a multi-view sidebar but adds a picker.

**Ask fg which root behaviour he wants before building.**

### 8. Focus the left sidebar + arrow-key navigation — ✅ **DONE** (commit `868606b`)

Down/Up move one row and clamp at the ends, Home/End jump, Right opens a closed node then steps into
it, Left closes an open one then climbs to the parent, Enter activates. `focusSidebar` shortcut:
**Cmd+Shift+E** on macOS, **Ctrl+Alt+S** elsewhere (Ctrl+Shift is unavailable under the platform rule
in `shortcutRegistry.ts`), and it expands a collapsed sidebar before focusing.

⚠ **Roving tabindex is the wrong pattern here, and the sizing above was wrong to name it.** The rows
are virtualized — a focused row unmounts the moment it scrolls out of view and focus falls back to
`document.body` mid-navigation. DOM focus therefore stays on the `.tree` scroll container permanently
and the current row is published via **`aria-activedescendant`**. Same reason the cursor is stored as
a **row ID, not an index**: it tracks its row across a reorder, and simply goes away when the row is
filtered out.

**The logic is a pure module** — `treeKeyNav.ts`, `treeKeyAction(rows, index, key)` returning
`move | toggle | activate | null`. ProjectTree already flattens the recursion into `rows` for the
virtualizer, so a keystroke is a function of `(rows, cursor, key)` and needs no React. Follows how
`sidebarTreeLayout.ts` is factored, and it is the only way to test this without the virtualizer:
**under jsdom the virtualizer reports zero rows** (zero heights), which is exactly why
`sidebarViewKind.test.tsx` mocks `ProjectTree` out entirely.

Mouse and keyboard now share `activateNode`, extracted from `handleClick`. Modifier gestures
(Cmd-click toggle, Shift-click range) stay mouse-only.

⚠ **Known test gap, deliberate.** The pure logic has 10 tests; the **React wiring in ProjectTree is
untested** for the jsdom reason above. If that wiring matters enough later, it needs a virtualizer
mock or a real browser test, not more unit tests.

⚠ **A trap worth remembering.** The first version of the "Right does not act as Down on a leaf" test
passed against a deliberately broken implementation, because the leaf in the fixture was also the
last row — so a naive step-down hit the end-of-list clamp and returned null for the wrong reason. The
fixture now puts a sibling after the leaf. **The revert-proof caught this; the test alone did not.**

### 2. Custom accent colour (hex picker) — **entangled; do not build standalone**

Today `ACCENTS = ["auto", "green", "blue", "amber", "violet"]` (`SettingsModal.tsx:54`) with CSS
presets in oklch at `vlinx.css:37-40`. A custom accent must set `--accent`, `--accent-soft`,
`--accent-line` **and** `--accent-rgb` — the last as a **bare `r,g,b` triple**, because it is composed
inside `rgba()` at `QuitConfirmModal.tsx:97` and `LiveTabsOverLimitDialog.tsx:115`. A hex-only or
oklch-only format cannot satisfy that.

The injection seam exists: inline `style.setProperty` outranks the `[data-accent]` rules without
`!important` (`theme.ts:156-159` already does this for `--font-mono` / `--ui-fs`).

⚠ **The blocker is legibility, not plumbing.** All four shipped accents *already* fail WCAG AA for the
white text drawn on them (measured 2.06–2.66 against 4.5). xterm self-clamps at
`minimumContrastRatio: 4.5`; the CSS chrome has no equivalent, and 14 sites hardcode white on
accent-filled surfaces. **A free hex picker makes unreadable primary buttons reachable in one click,
and nothing stops it.**

This *is* theme-feature slices 1 and 3. Build it there, not as a separate setting.

### 10. Rename a session → run `/rename` inside the Claude session — **small-medium, needs verification**

fg's idea: when a Claude session is renamed in the left sidebar, optionally push that rename into the
agent itself. Gated by a setting.

Inline rename already exists in `LeftSidebar` (see its header comment). Writing into a session is
`ptyWrite(sessionId, ...)` — the same call `TerminalView` uses for native image paste.

Unresolved before building:

- ✅ **`/rename` exists.** Verified 2026-08-23 against the Claude Code binary (v2.1.241): usage is
  `/rename <name>`, bare `/rename` auto-generates a name from conversation context. **It also updates
  the terminal tab title by default** — there is a setting to turn that off ("Whether /rename updates
  the terminal tab title (defaults to true)"). So the item is live, but see the loop hazard below.
- Only meaningful for `claude`-kind sessions — needs a kind check, and a decision about Codex and the
  other agents.
- **Injecting text mid-turn would corrupt the agent's input.** Needs a guard on session state (idle at
  a prompt, not `working`/`asking`), and a decision about what to do when it is busy: skip, queue, or
  warn.
- ⚠ **Feedback-loop hazard, new.** Since `/rename` writes the terminal tab title, a sidebar rename that
  injects `/rename` will come back as a title-change escape sequence. If VelaTerm ever renames a
  session *from* the received title, that closes a loop. Check whether VelaTerm consumes OSC 0/2
  title sequences before wiring this.

### 11. Separate interface font from terminal font — **already works; it is a discoverability bug**

The settings are **already separate**: `uiFontFamily` / `uiFontSize` and `termFontFamily` /
`termFontSize` are four distinct `PersistedSettings` fields, with two independent `FontSelect` controls
(`SettingsModal.tsx:840` and `:861`).

`fontStack()` in `theme.ts` passes any value **containing a comma straight through** as a literal CSS
stack. So typing `Inter, system-ui, sans-serif` into the UI Font field makes the whole interface
proportional today — **confirmed working by fg, 2026-08-23.**

Two real defects behind the confusion, both cosmetic:

- **The `FontSelect` dropdown only pre-populates monospace fonts**, so nothing suggests a proportional
  face is even allowed. This is the actual fix: offer a few proportional options in the UI Font list.
- The variable the UI font writes to is named **`--font-mono`** (`theme.ts:156`), and its default stack
  ends in `monospace`. A misnomer once the UI font can be proportional. Renaming it touches
  `vlinx.css` broadly — cheap but wide, so bundle it with the theme work rather than alone.

### 12. Pinned script pane in the sidebar — **fg: high priority. FULLY UNBLOCKED 2026-08-23**

fg wants CC-Fleet-Watcher living in a left-sidebar pane. Settled with fg: the pane **owns a command
and restarts it** (not a session dragged in from the tree), and the **user picks any script** per pane.

**Shares its foundation with item 1.** Both need a `kind` discriminator on `SidebarTreeView` so a view
can render something other than `ProjectTree`. Do item 1 first; this is the second consumer.

#### What the recon settled

- **The render site is one line.** `LeftSidebar.tsx:245` is
  `<ProjectTree view={view} isPrimary={isPrimary} {...treeHandlers} />`. A `kind` branch goes here.
  The layout engine (`sidebarTreeLayout.ts`) is view-agnostic already — splits, tabs, dividers and
  rects need no changes.
- ⚠ **Do NOT reuse `TerminalView`.** It is a *session pane*: header with status/title/git branch,
  split and close buttons, context menu, image paste, drop handling. Almost none of that belongs in a
  sidebar column. Reuse **`usePtySession`** instead — the hook `TerminalView` is built on, which owns
  the xterm, the fit addon, the renderer choice and the output scheduler.
- ⚠ **`usePtySession(session: Session, cwd?, hidden?)` takes a `Session`, not a command.** So the pane
  needs a `Session`-shaped object. Two options, and this is the main design decision:
  - **Register a real session** in `sessions[]` — simplest, but the pinned script then shows up in the
    project tree, the status filters and the status-bar counts. Almost certainly wrong.
  - **Synthesise an unregistered `Session`** held by the view. `countByAgentState` iterates `sessions`,
    so a runtime whose id has no matching session does not reach the counts — this looks viable and is
    the recommended path, but it is **[Unverified]**: confirm nothing else joins on session id.
- **Persistence is its own versioned store, not `vlx-settings`.** Sidebar views live in
  `vlx-sidebar-tree-views` with a discriminated `version: 1 | 2` payload and a loader that sanitises
  every field (length caps, control-character stripping). Adding a command field means **a V3 payload**
  plus following that sanitising convention — the free-lunch rule for `PersistedSettings` does **not**
  apply here.
- ⚠ **Sidebar views are NOT in `SYNCED_KEYS`.** They are per-shell, so a pinned script does not follow
  fg to the browser or remote shell. Fine, but worth knowing before he expects otherwise.

#### Decisions — all three answered 2026-08-23

1. ✅ **ANSWERED 2026-08-23 — explicit Start button by default, with a per-pane "start automatically"
   option in its config.** Doable, and cheaply: the default matches how workspace restore already
   behaves (terminals come back without restarting), and the opt-in is one boolean on the view's
   persisted payload — the same V3 field work the command string already requires, so it adds a
   checkbox and no new machinery. Show the command in the pane header either way, so an auto-starting
   pane is never silent about what it runs.
2. ✅ **ANSWERED 2026-08-23 — show the exit code and wait, with a restart button.** Auto-restart is
   NOT its own setting: if fg wants it, it rides the same opt-in checkbox as auto-start (decision 1),
   so one choice governs "this pane runs without me touching it". Keeps a crash-looping script from
   being the default behaviour.
3. ✅ **ANSWERED 2026-08-23 — force the DOM renderer for sidebar panes**, regardless of the global
   renderer setting. A pinned pane is a *permanently* mounted extra terminal, which makes the GPU
   context exhaustion `settings.ts` warns about likelier than a normal session does, and a
   low-throughput text column gains nothing from WebGL.

**Not a small feature.** Bigger than anything shipped so far, and the `Session` question should be
verified before any code is written.

### 13. Favorites in the file tree — ✅ **BUILT** on `feat/file-tree-favorites`

fg, 2026-08-23: the file tree gets a favorites section pinning chosen files to the top — "or a separate
panel/pane if easier". **It is not easier.** Sized against the code, not built.

`FilesTab.tsx` (829 lines) holds a `FileNodeT` tree built from `rootPath`, flattens it for render
through `flattenVisible(root, showHidden, q)` (:80), and draws each row with `FileRow` (:103). It
already carries a context menu (`menu` state, :228) and a filter box, so the two UI affordances a
favorites feature needs are both present.

**Recommended shape — a pinned section inside the existing tree.** Prepend the favorites to the
flattened row list rather than rendering a second tree: one call site (`flattenVisible`'s consumer),
one new context-menu item, one divider row. A pinned entry is a path string, resolved against the
current tree so a favourite that no longer exists renders greyed rather than crashing.

**The separate-pane version is strictly more work**, not less: it means a third `kind` on
`SidebarTreeView` (`"sessions" | "files" | "favorites"`), which drags in the V3 payload work,
the layout engine, and the same picker question item 1 raised. Only worth it if fg wants favourites
visible *while* looking at a different folder — which is a real argument, but it is a different
feature from "pin to the top".

**The one open decision — scope.** Global favourites, or per-project? Per-project is the more useful
default (a favourite is usually "the file I keep opening in *this* repo"), and it also settles
persistence:

- **Per-project** → the paths belong with project state, not with `vlx-settings`.
- **Global** → a `favoritePaths: string[]` field on `PersistedSettings` is free and syncs across
  desktop / browser / remote for nothing, per the rule at the top of this file. But absolute paths
  that sync to a *different machine* are meaningless, so the free ride is partly illusory — the
  Mac's `/Users/farhad/...` favourites arriving on the Windows box would all be dead entries.

⚠ **Not `vlx-sidebar-tree-views`.** That store is per-shell and NOT in `SYNCED_KEYS`; putting
favourites there means they vanish when fg opens the browser shell.

✅ **fg answered 2026-08-23: per-project.** Built on branch `feat/file-tree-favorites`, off
`06b74b0`.

**What shipped**, and where the sizing above was wrong:

- ⚠ **The sizing said "prepend the favourites to the flattened row list". There is no flattened row
  list to prepend to.** `flattenVisible` exists but serves *keyboard navigation only* — the tree
  actually renders as one recursive `<FileRow node={root}>`. The pinned block is therefore its own
  section rendered above that call, inside the same scroller.
- ⚠ **`rootPath` cannot stand in for project identity.** Both call sites pass
  `session?.cwd || project?.rootPath`, so the root is usually a *session cwd* — a subdirectory or a
  worktree. Keying favourites by root path would have fragmented them per-session, which is not what
  "per-project" means. `FilesTab` now takes a `projectId` prop; both call sites already had the
  project in hand, so it was one line each.
- **Persistence:** `favoritePaths: Record<projectId, string[]>` on `PersistedSettings` — free, per the
  rule at the top of this file, and synced across shells. The last removal for a project **deletes its
  key** rather than leaving `[]`, so an emptied project leaves nothing behind in the synced blob.
- **Files only.** A pinned directory would have to answer "expand it where?", and fg asked for files
  pinned to the top. The context-menu entry is hidden for directories and when there is no project.
- Three locale keys × 11, and one new `star` icon (there was no pin or star in `Icons.tsx`).

**Deliberately not built** — say the word if any of these matter:
- **No existence check.** A favourite whose file was deleted stays listed and errors on open through
  the tab's normal error row. Greying out dead entries needs a stat per favourite per render.
- **No reordering.** Insertion order, which the tests pin.
- **No keyboard nav into the pinned block** — `onTreeKeyDown` still walks `flattenVisible`, which
  covers the tree only. Mouse and context menu work; arrows skip the pinned rows.

### 14. "Collapse all" in the file tree — ✅ **BUILT** on `feat/files-collapse-all`

fg, 2026-08-23. The file tree has no way to collapse everything at once.

`FilesTab.tsx` holds expansion state on the nodes themselves (`FileNodeT`, built by `toNode`) rather
than in a separate set, and `flattenVisible(root, showHidden, q)` (:80) walks that state to decide
what renders. So "collapse all" is a walk of the loaded tree clearing the expanded flag, plus a
button in the existing filter/header row — no new state, no persistence question.

Both open questions were settled as the sizing suggested:

- **The root stays open.** Collapsing it hides the entire tree and reads as a broken panel.
- **The control is hidden while a filter is active**, because filtering force-expands
  (`open = filter ? true : !!node.open`) and collapsing then changes state the user cannot see. It is
  hidden for the same reason when nothing below the root is open — no button that visibly does
  nothing.

`collapseAllBelow` and `hasOpenDescendant` are **exported** so the recursion is testable without
rendering the tab (which needs a backend directory listing). Nine tests; the revert-proof used a
naive "close the top level and the root" implementation and five of them failed, including the
deep-nesting and root-stays-open cases.

Loaded children survive the collapse, so reopening a directory does not refetch it.

**The sibling gap is still open.** The session tree in the left sidebar has the same absence, and it
stores collapse state differently — a shared `collapsed` per node plus per-view `collapsedOverrides`
— so it is a separate change, not this one applied twice. **Not built; ask fg before assuming he
wants it.**

### 15. A pane that survives session switching — ✅ **BUILT (option A)** on `feat/sticky-pane`

fg, 2026-08-23: opening a markdown file gives a tab next to the coding tabs, and switching sessions in
the left panel swaps the coding tab while the markdown tab stays open — all correct. What he wants is
to see them **side by side**: one pane holding the markdown (or a terminal), the other keeping the
single-tab session-swapping behaviour. "Some tabs consistently across different coding sessions."

#### Why it does not work today, exactly

- **The swap replaces the whole tab, not a pane.** `openSession`'s reuse path ends with
  `openTabs[idx] = newTabId` and moves the outgoing tab's **entire pane tree** into `liveTabs` intact
  (`termStore.ts` ~2244-2300). Split that tab and put markdown in one half, and both halves get
  backgrounded together and replaced by the incoming session's tree.
- **What already makes the markdown tab survive** is `pinnedTabs`: `isReusableTab` refuses to reuse a
  pinned tab, so document, browser, explicit-new-tab and terminal tabs are never chosen as the swap
  slot. That is persistence at the **tab** level — adjacent, never side by side. fg has correctly
  noticed the behaviour; it just is not a split.
- ⚠ **The blocker is not the swap logic. Documents are not pane-tree citizens at all.** `LeafNode` is
  `{ kind: "leaf"; paneId; sessionId }` — a leaf binds **one session** and nothing else
  (`layout/CenterPane/paneTree.ts:6-10`). In `CenterPane`, session pane trees, `docTabs` and
  `browserTabs` are **three parallel top-level renderings**, each filling the whole stage and hidden
  with `display:none` when inactive (`CenterPane.tsx:240-262`). There is no representation in which a
  document sits inside a split.

#### Two ways to give fg what he asked for

**Option A — a sticky region beside the stage. Recommended.** Do not make documents pane citizens.
Add one persisted `stickyTabId` plus a splitter, and render the tab stage in one half with the sticky
tab's view in the other, exempting that tab from the `hidden` gating it already has. Everything is
**already permanently mounted** — docs, browsers and terminals are all kept alive and merely hidden —
so this is largely a layout change plus one exemption. It never touches `openSession`, the pane tree,
or the eviction model. It also generalises for free: the sticky slot can hold a document, a browser
tab or a terminal, because all three are already stage-level views.

**Option B — make documents real pane-tree leaves.** `LeafNode` gains a discriminator, and
`splitAt` / `removeLeaf` / `removeSession` / `computeLayout` / `collectSessionIds` all learn about
leaves that are not sessions. Then the single-tab swap must be taught to replace **one leaf** instead
of the tab — which the background pool cannot express today, because `liveTabs` and `paneTrees` are
keyed by **tab** and hold whole trees. This is the general, "correct" version and it is **large**,
touching the same machinery the hybrid-tabs note at the top of this file already flags as the biggest
change available here.

✅ **fg chose A, 2026-08-23. Built on branch `feat/sticky-pane`** (off `feat/file-tree-favorites`,
so that branch carries favourites too).

**How it works.** Right-click any tab → "Keep Visible in Split". The stage splits into two bands;
each rendering layer is confined to one. Because every view is *already* permanently mounted and only
hidden with `display:none`, the sticky tab only has to be exempted from that hiding — no new mounting
model. Documents and browsers get a **positioned wrapper** rather than a new prop, since `.docview` is
`position:absolute; inset:0` and so fills whatever positioned ancestor it is given.

Tab-local rects are **rescaled** into their band rather than recomputed, so splits keep their
proportions on both sides. `rectIntoRegion` / `dividerIntoRegion` live in `paneTree.ts` next to
`computeLayout`, which makes them unit-testable without rendering.

⚠ **`parentRect` must be rescaled along with the divider.** The drag handler turns pixels into
percentages by dividing through `parentRect`, so leaving it at full-stage width makes a dragged
divider outrun the pointer. Easy to miss — the divider *looks* right until you drag it.

- Activating the sticky tab itself collapses the split rather than showing the same tab twice.
- Closing it empties the slot; otherwise a region stays pinned to a tab that no longer exists.
- The sticky tab keeps its own splits resizable — `Divider` already targets a tab by ID.

⚠ **The slot is in-memory only.** Tab IDs are not stable across a restart, so persisting it would
restore a dangling reference. If fg wants it to survive restarts it needs to key off something
durable (a document path, a session ID that workspace-restore preserves) — a separate change.

### 9. Git branch/graph visualisation in the Git tab — **deferred**

fg: "discuss later." Target is `src/layout/RightPanel/GitTab.tsx`, which currently shows
branch/ahead/behind values only.

---

## Build waves — parallel teammates, dispatched 2026-08-23

Ordering is forced by two dependencies: **item 12 needs item 1's `kind` discriminator**, and **item 8
rewrites the same `LeftSidebar` rows item 1 touches**. Everything else is independent.

**Wave 1 — in flight**, three Opus teammates in isolated git worktrees, all branched from `e0c7050`:

| Teammate | Item | Branch |
|---|---|---|
| `sidebar-views` | 1 — `kind` discriminator + files view | `feat/sidebar-view-kinds` |
| `tab-cycling` | 7 — Ctrl+Tab / Ctrl+Shift+Tab | `feat/tab-cycling` |
| `ui-fonts` | 11 — proportional Interface font presets | `feat/proportional-ui-fonts` |

**Wave 2 — after wave 1 merges:** item 8 (sidebar keyboard nav), then item 12 (pinned script pane).

⚠ **Worktrees prevent clobbering, not merge conflicts.** Two collisions were designed out in the
briefs rather than left to chance:

- **`termStore.ts`** — items 1 and 7 both need it. Pinned to disjoint regions: sidebar views at
  ~145-340 and ~3139-3270, tab actions at ~2100-2550.
- **The 11 locale files** — items 1 and 11 both add keys. Anchors checked first: `tree.*` sits at
  en.ts:436, `settings.uiFont*` at en.ts:222. Two hundred lines apart, so no overlapping hunks.

Every brief carries the same bar: `tsc` clean · eslint showing **exactly** the 2 pre-existing
`ConnectionBanner` errors · full suite green against the 32-file / 230-test baseline · and the new
test **proven to fail when its change is reverted**, with the failure message reported back.

Decisions were front-loaded into the briefs rather than left open, because the `spawning-teammates`
skill records a measured failure mode: a teammate given an unconditional "ask if ambiguous" rule
applies its own severity filter and guesses anyway. Each brief enumerates specific decision classes
instead.

## Suggested order

1. ~~**3, 5**~~ — ✅ done, commits `700e2cd` and `0312af7`.
2. ~~**4**~~ — ✅ done, commit `c8e6509`.
3. ~~**6**~~ — ✅ fixed, commit `e0c7050`. It was a bug, not a feature.
4. **7** — next/previous tab cycling.
5. **1** — after fg picks (a), (b) or (c).
6. ~~**8**~~ — ✅ sidebar keyboard navigation, done 2026-08-23 (see below).
7. ~~**13**~~ — ✅ file-tree favourites, built on `feat/file-tree-favorites`.
8. ~~**15**~~ — ✅ sticky pane (option A), built on `feat/sticky-pane`.
9. ~~**14**~~ — ✅ file-tree "collapse all", built on `feat/files-collapse-all`.
10. **2** — inside the theme feature, not before it.
