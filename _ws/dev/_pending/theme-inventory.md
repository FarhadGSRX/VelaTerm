# Theme Colour Inventory — VelaTerm

> Companion to `theme-surface.md` (the decision document: apply path, insertion points,
> persistence, prior art, blast radius). This file holds §1 of that report — the complete cited
> inventory of every place colour is defined. Section anchors (§1a, §1c, §1d.1, §1e …) referenced
> from `theme-surface.md` all live here.

Repo `C:\_fg2\Code\VelaTerm` @ `d098f5a`. Read-only recon; no build was run, so every claim is
static reading of source, not observed runtime behaviour.

---

## 0. Method + counts

A scripted sweep over `src/**/*.{ts,tsx,css}` for colour literals (`#rgb`, `#rrggbb`, `rgb[a]()`,
`oklch()`, `hsl[a]()`), classified by whether the literal sits in a comment, in a
`var(--x, <literal>)` fallback slot, or bare in a declaration:

| class | lines | meaning |
|---|---|---|
| token definitions, `src/styles/vlinx.css` | 53 | the design system itself |
| xterm palette, `src/theme.ts:83-101` | 16 | the ANSI 16 + cursor/selection |
| everything else (bare literals) | 115 | see §1d |
| `var(--x, <literal>)` fallbacks | 49 | across 24 files, §1e |
| literals mentioned in comments only | 16 | not rendered |
| in `*.test.ts` | 3 | `grokBgRemap.test.ts` |

**115 bare literals across ~45 files** — the largest single group is one repeated drop-shadow (§1d.1).

---
## 1. Colour inventory

### (a) The token system — three stacked layers

**Layer 1 — `src/styles/vlinx.css` (458 lines).** The only file defining raw colour for chrome.

| block | lines | contents |
|---|---|---|
| `:root` non-colour + accent | `vlinx.css:10-25` | `--font-mono`, radii `--r-sm/md/lg`, `--pane-gap`, `--accent`/`--accent-soft`/`--accent-line`, `--grok-brand-color`, density `--row-h`/`--ui-fs`/`--term-fs` |
| `:root` pre-mount colour fallback | `vlinx.css:26-30` | only `--bg-0` + `--text`, dark-valued |
| light pre-mount fallback | `vlinx.css:32-36` | `@media (prefers-color-scheme: light)` overriding `--bg-0`, `--text`, `--grok-brand-color` |
| accent presets | `vlinx.css:37-40` | 4 lines × 3 props: `[data-accent="green\|blue\|amber\|violet"]` |
| density presets | `vlinx.css:42-43` | non-colour |
| **`[data-theme="dark"]`** | `vlinx.css:46-70` | **30 custom properties** |
| **`[data-theme="light"]`** | `vlinx.css:72-99` | **the same 30 properties** — verified identical key sets by diff |

The 30 per-scheme tokens: `--grok-brand-color`, `--bg-0/1/2`, `--bg-term`, `--bg-hover`,
`--bg-active`, `--border`, `--border-strong`, `--text`, `--text-mid`, `--text-dim`, `--text-faint`,
`--shadow`, `--term-cmd`, `--term-out`, `--green`, `--yellow`, `--red`, `--cyan`, `--mag`, `--blue`,
`--c-blue`, `--c-yellow`, `--c-cyan`, `--c-mag`, `--c-red`, `--c-green`, `--c-dim`, `--c-faint`.
(The brief said ~31; it is exactly 30 in each block.)

Only three colour literals appear in the rest of the file, all in component rules:
`vlinx.css:221` (`rgba(0,0,0,0.38)` shadow), `vlinx.css:251` (`rgb(0 0 0 / 0.2)` shadow),
`vlinx.css:445` (`oklch(0.6 0.18 25 / 0.12)` danger hover). Everything else is `var()`.

**Layer 2 — `src/styles/index.css:16-34`, an alias/semantic layer the brief does not list.**
It defines 13 further variables, *all* as pure `var()` indirections onto Layer 1 — no literals:

- surface aliases: `--bg-app`, `--bg-panel`, `--bg-elevated`, `--bg-terminal` (`index.css:17-20`)
- text aliases: `--text-primary`, `--text-secondary`, `--text-muted` (`index.css:21-23`)
- **six semantic status colours**: `--status-running/exited/error/working/asking/waiting`
  (`index.css:28-33`)

The header at `index.css:5-13` states its purpose: keep un-rewritten shared components following a
theme change untouched. **Consequence for this feature: overriding the 30 Layer-1 tokens propagates
into all 13 Layer-2 aliases for free.** The comment at `index.css:26-27` is worth reading before any
design — `--status-working` is deliberately pinned to `--green`, *not* `--accent`, "so the semantic
status colours do not drift when the accent skin changes."

**Quantified — which layer components actually consume** (scripted count of `var(--…)` references
across `src/**/*.{ts,tsx,css}`, excluding the two stylesheets that define them, excluding tests):

| | references | files |
|---|---|---|
| **Layer-1 raw names** (`--bg-0`, `--text`, `--border`, `--accent`, …33 names) | **1306** | **62** |
| **Layer-2 alias names** (`--bg-app`, `--text-primary`, `--status-*`, …13 names) | **267** | **33** |

**~83% of all token consumption goes straight to the Layer-1 names.** 32 of the 33 alias-consuming
files also use raw names (only one file is alias-only; 30 are raw-only), so the two layers are
interleaved rather than stratified. Heaviest single tokens: `--text` (483), `--border` (171),
`--accent` (155), `--text-dim` (143).

**This is the argument for where to inject: at Layer 1.** Overriding the 30 vlinx tokens covers both
columns — the 1306 direct references *and*, through `var()` indirection, the 267 alias ones.
Injecting at Layer 2 instead would reach 17% of the surface. (Noted in passing: `--bg-terminal` is
defined at `index.css:20` and referenced **zero** times — a dead alias.)

**Layer 3 — Tailwind v4.** `index.css:1` imports it, but there is **no `@theme` block anywhere** and
**zero Tailwind colour utilities in `src/`** (grep for `class="… bg-|text-|border-<hue>-<n>"`
returns 0 matches). Tailwind contributes nothing to colour resolution. README.md:63 calling it
"Tailwind v4 with CSS-variable themes" overstates it.

### (b) The xterm palette — `src/theme.ts:72-103`

`XTERM_THEME: Record<ResolvedTheme, ITheme>` — a frozen module constant, 16 ANSI + `background`,
`foreground`, `cursor`, `cursorAccent`, `selectionBackground`, `selectionInactiveBackground` per
scheme. **Completely disconnected from the CSS token layer** — hand-synced by comment only:

- `theme.ts:73` "Match the background to Vlinx `--bg-term`"
- `vlinx.css:51` "Terminal background source of truth. `XTERM_THEME[dark].background` must match to
  avoid visible seams"
- `vlinx.css:78` the same note for light
- `theme.ts:93` "Keep pure `#ffffff` in lockstep with vlinx.css `--bg-term` (light)"

Four comments guarding one invariant across two files, enforced by nothing.

### (c) Secondary themed surfaces — all already token-driven

| file | lines | literals | verdict |
|---|---|---|---|
| `src/components/diff/codeMirrorTheme.ts` | 86 | **0** | fully `var(--*)`; 31 highlight tags mapped to `--accent/--mag/--cyan/--green/--yellow/--red/--text*` at `codeMirrorTheme.ts:19-49` |
| `src/layout/CenterPane/doc/docTheme.css` | 556 | **1** (`:450` shadow) | rest is `var()`; `docTheme.css:33-34` even routes Crepe's shadows through `--shadow` |
| `src/mobile/mobile.css` | 319 | **3** (`:207` shadow, `:263` red fallback, `:264` `#fff`) | header at `mobile.css:2-3` states it follows vlinx variables |

These ride the token layer at zero cost. `codeMirrorTheme.ts:3` flags a known duplicate: an
equivalent definition still lives inside `SourceEditor.tsx`, "that can later be consolidated."

### (d) One-off literals — 115 bare, grouped

**1. Drop shadows — 33 sites, the single largest group.** `boxShadow: "0 Npx Npx rgba(0,0,0,0.3–0.5)"`
repeated verbatim: `sessionMenuDialogs.tsx` ×12 (`:78,200,262,425,597,797,955,1231,1301,1477,1699,1770`),
plus `ContextMenu.tsx:91`, `FormModal.tsx:130`, `MergeModal.tsx:99,336`, `SpawnConfirmModal.tsx:108,228`,
`UpdateModal.tsx:266`, `NotifyGuideModal.tsx:50`, `ErrorLogModal.tsx:90`,
`GitbashDownloadBanner.tsx:90`, `ChangesModal.tsx:170`, `DocView.tsx:888`, `SearchBar.tsx:47`,
`TabBar.tsx:484`, `TerminalView.tsx:778,846`, `GlobalSearch.tsx:90`, `ArchivePanel.tsx:120`,
`FilesTab.tsx:807`, `docTheme.css:450`, `mobile.css:207`.
**Theme-relevant.** A `--shadow` token exists (`vlinx.css:60,87`) and *is* used correctly in ~10
other places (`QuitConfirmModal.tsx:70`, `ShareModal.tsx:207`, `StatusBar.tsx:346,577`,
`ConnectRemotePanel.tsx:437,887`, `LiveTabsOverLimitDialog.tsx:81`, `ProjectTree.tsx:832`,
`docTheme.css:33-34`). These 33 are inconsistent bypasses, and they are already wrong in light mode.

**2. Modal scrims — 8 sites.** `background: "rgba(0,0,0,0.45)"`: `Backdrop.tsx:43`,
`ChangesModal.tsx:157`, `MergeModal.tsx:322`, `DocView.tsx:875`, `sessionMenuDialogs.tsx:409,581`,
`LiveTabsOverLimitDialog.tsx:69` (`0.4`), `main.tsx:60` (`0.85`).
**Theme-relevant but low priority** — a black scrim is defensible in both schemes.

**3. `#fff` on filled accent/danger surfaces — 14 sites.** `TitleBar.tsx:214`, `StatusBar.tsx:653`,
`settingsBehaviorFields.tsx:242`, `SettingsModal.tsx:1037,1105`, `NotifyGuideModal.tsx:76`,
`UpdateModal.tsx:406,419`, `TerminalView.tsx:425`, `ProjectTree.tsx:839` (inside an HTML string),
`index.css:135` (`.vlx-btn-primary`), `mobile.css:264`.
**Theme-relevant and the main contrast hazard** — see §6.

**4. Terminal search highlight — 10 sites, triplicated.** `#d29922` / `#f0883e`:
`registry.ts:149-152` (live terminals), `RecordingViewer.tsx:27-30` (recording playback),
`highlight.tsx:13,19` (the same two colours re-expressed as `rgba(210,153,34,0.40)` /
`rgba(240,136,62,0.62)`, with `highlight.tsx:5-6,11` documenting the intent to match).
**Theme-relevant**, and the one xterm-adjacent colour that `setXtermTheme()` does *not* reach.

**5. The accent swatch map — 4 lines, `SettingsModal.tsx:48-53`.**
`ACCENT_HEX = { green: "#3fcf8e", blue: "#5b9dff", amber: "#f5b14c", violet: "#b08bff" }`, consumed
at `SettingsModal.tsx:596` (`background: ACCENT_HEX[a]`) to paint the picker swatches.
**A second, divergent copy of the accent palette** — these hexes are not the `oklch()` values at
`vlinx.css:37-40`. Any user-accent work must reconcile this or the picker lies.

**6. Hardcoded error red — 2 sites.** `SettingsModal.tsx:1068`, `settingsPanels.tsx:156` (`#e5484d`),
while sibling code uses `var(--danger, #e5484d)`. Theme-relevant, trivially fixable.

**7-11. Incidental — leave alone (41 sites).**

- *Crash/error screens, 18:* `ErrorBoundary.tsx:36,67,68,77,78,84,85,87,98,99,103,114,115` and
  `main.tsx:61,70,72,73,74`. Last-resort surfaces that must render when React or the stylesheet is
  broken; token dependence defeats the purpose. `debug.ts:27` (a `console.log` `%c` format) likewise.
- *Brand marks, 13, must NOT be themed:* `sessionMeta.tsx:29-43` (12 agent identity colours, each
  commented with its justification), `brandIcons.tsx:214`, `BrowserQuickAccess.tsx:20`. The contrast
  case is `--grok-brand-color` (`vlinx.css:21,35,47,73` → `brandIcons.tsx:220`) — tokenised precisely
  because Grok's mark is monochrome and must invert per scheme; `ShareModal.tsx:42` states the rule.
- *QR-code backgrounds, 2, must stay `#fff`:* `ShareModal.tsx:322`, `RemoteAccessPanel.tsx:590`.
  Scanner reliability, not styling.
- *PDF export fallbacks, 7:* `DocView.tsx:421-427` — prior art, not debt (§5).
- *`grokBgRemap.ts:68`,* literal `"#ffffff"` — see §6.

### (e) The 49 `var(--x, <literal>)` fallbacks

**Seven variables are referenced but defined nowhere in the codebase**, so the literal fallback
always wins today. Verified by set-differencing every `var(--…)` use in `src/` against every
definition in `vlinx.css` + `index.css`.

| variable | uses | example call site | fallback(s) in use | class |
|---|---|---|---|---|
| `--danger` | 25 (on 21 lines) | `src/components/diff/changeStatus.ts:8` | `#e05252`, `#ff6b6b`, `#e5484d` | **theme-relevant** |
| `--warning` | 3 | `src/components/MergeModal.tsx:408` | `#c9a227` | **theme-relevant** |
| `--orange` | 1 | `src/layout/RightPanel/InfoTab.tsx:145` | `#d8954a` | **theme-relevant** |
| `--accent-dim` | 2 | `src/components/QuitConfirmModal.tsx:97` | see below | **theme-relevant** |
| `--accent-rgb` | 2 | `src/components/QuitConfirmModal.tsx:97` | `63,207,142` | **theme-relevant — format constraint, see below** |
| `--bg-3` | 8 | `src/layout/TitleBar/SettingsModal.tsx:178` | `rgba(128,128,128,0.12)` | structural (a hover tier) |
| `--panel-bg` | 1 | `src/components/GitbashDownloadBanner.tsx:87` | `#2a2a2a` | structural (a `--bg-1` alias) |

Full `--danger` call sites: `changeStatus.ts:8`, `ChangesModal.tsx:82,209,286`, `MergeModal.tsx:521`,
`ErrorLogModal.tsx:162`, `GitTab.tsx:99`, `TitleBar.tsx:215`, `settingsPanels.tsx:632`,
`ConnectRemotePanel.tsx:594,611,628,825,831`, `RemoteAccessPanel.tsx:739,752,856,888,891,908,952`.
Three different fallback reds are in use — already inconsistent before any theme feature.
`--bg-3`: `SettingsModal.tsx:178,310,492,779`, `settingsPanels.tsx:295`, `StatusBar.tsx:380`,
`BrowserView.tsx:58`, `BrowserQuickAccess.tsx:100`.

**`--accent-rgb` constrains the theme file format.** Both its call sites compose it as an rgb triple
inside `rgba()` — `src/components/QuitConfirmModal.tsx:97` and
`src/layout/CenterPane/LiveTabsOverLimitDialog.tsx:115`, identically:

```
"var(--accent-dim, rgba(var(--accent-rgb, 63,207,142), 0.12))"
```

So `--accent-rgb` must hold a bare comma-separated triple (`63,207,142`), **not** a colour value. A
theme format that accepts only hex or only `oklch()` cannot satisfy it — the app would have to derive
the triple (the `DocView.tsx:402-417` canvas trick does exactly this conversion already, §5), or the
format must carry a separate channel for it, or these two sites get rewritten to
`color-mix()`/relative-colour syntax. Flagging it because it is the one undefined variable whose
*shape*, not just its value, is load-bearing.

This cuts both ways: those surfaces are theme-blind now, **but a user theme that defines these seven
names takes them over with zero component edits.**

