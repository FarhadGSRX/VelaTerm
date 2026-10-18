# Theme Surface Map — where colour resolves in VelaTerm

Recon for a planned upstream contribution: user-supplied colour themes. **Map only, not a design.**
Repo `C:\_fg2\Code\VelaTerm` @ `d098f5a`. Read-only; no build was run, so every claim below is
static reading of source, not observed runtime behaviour.

---

> **The full colour inventory (§1) lives in `theme-inventory.md`** — every file where a colour is
> defined, with `path:line` and counts. References to §1a / §1c / §1d.n / §1e below point there.

---

## 1. Colour inventory — moved

See **`_devnotes/_pending/theme-inventory.md`**. Summary of what it establishes, since the sections
below lean on it:

- Colour resolves through **three stacked layers**: `src/styles/vlinx.css` (30 tokens per scheme in
  `[data-theme="dark"]` `:46-70` / `[data-theme="light"]` `:72-99`, plus 4 `[data-accent]` presets at
  `:37-40` and a `:root` pre-mount fallback at `:26-36`); `src/styles/index.css:16-34` (13 semantic
  aliases, pure `var()` indirection, including six `--status-*`); and Tailwind, which contributes
  **nothing** — no `@theme` block, zero colour utilities in `src/`.
- **1306 token references / 62 files** hit the Layer-1 raw names against **267 / 33 files** for the
  Layer-2 aliases — ~83% direct. This is the argument for injecting at Layer 1.
- The xterm palette (`src/theme.ts:72-103`) is a frozen module constant, **disconnected from the CSS
  tokens** and hand-synced by four comments (`theme.ts:73,93`, `vlinx.css:51,78`).
- Secondary surfaces (`codeMirrorTheme.ts`, `docTheme.css`, `mobile.css`) are already token-driven —
  4 literals between them — so they ride the token layer for free.
- **115 bare literals** remain, dominated by 33 duplicated drop-shadows that bypass the existing
  `--shadow` token, 14 `#fff`-on-accent sites, and 10 triplicated search-highlight colours.
- **Seven variables are used but defined nowhere**, always behind `var(--x, <literal>)`. `--danger`,
  `--warning`, `--orange`, `--accent-dim`, `--accent-rgb` are theme-relevant; `--bg-3`, `--panel-bg`
  are structural. **`--accent-rgb` constrains the theme file format** — both call sites compose it as
  a bare comma triple inside `rgba()`, so a hex-only or oklch-only format cannot satisfy it.
---

## 2. The apply path, end to end

**Startup, before React.** `index.html:17-24` inlines `html, body { background: oklch(0.2 0.006 260) }`
plus a `prefers-color-scheme: light` override, purely to kill the white flash (the comment at
`index.html:14-16` names `[data-theme] --bg-0` as source of truth). `vlinx.css:26-36` then supplies
`--bg-0`/`--text` on bare `:root` for the same window, deliberately at equal specificity so the
later `[data-theme]` rules win once written.

**Mount.** `main.tsx:11` imports `styles/index.css` (which `@import`s `vlinx.css` at
`index.css:2`); `main.tsx:39-43` renders `<App/>` or `<MobileApp/>` behind `LoginGate`.

**First paint.** `App.tsx:62-63` calls `applyAppearance()` in the mount effect
(`MobileApp.tsx:36-37` does the identical thing). `termStore.ts:3469-3473`:

```ts
applyAppearance: () => { applyTheme(get().theme); persistAndApplyVisual(get); notifyAgentsColorScheme(get); }
```

- `applyTheme(mode)` — `theme.ts:133-138` — resolves `system`→`dark|light` (`theme.ts:114-116`),
  writes `document.documentElement.dataset.theme`, mirrors the mode to `localStorage["vlx-theme"]`,
  then calls `setXtermTheme(XTERM_THEME[resolved])`.
- `persistAndApplyVisual()` — `termStore.ts:~1240-1271` — snapshots the settings slice, calls
  `saveSettings(ps)` (`settings.ts:154-163`), then `applyVisual(visualOf(ps))` (`settings.ts:164-172`).
- `applyVisual(s)` — `theme.ts:144-160` — writes `data-accent` (resolving `auto` via
  `effectiveAccent()`, `theme.ts:124-130`), `data-density`, `data-pane`, `data-divider`, `data-nav`,
  and sets/removes inline `--font-mono` / `--ui-fs` on `documentElement`.
- `notifyAgentsColorScheme()` — `termStore.ts:1363-1377` — on a *changed* resolved scheme, writes
  DEC `CSI ? 997 ; 1|2 n` into the PTY of every live **claude** session so the agent re-queries OSC 11.

**CSS resolves.** `data-theme` selects one of the two 30-token blocks; `data-accent` overrides the
three accent tokens; `index.css`'s aliases follow through `var()` indirection; every component rule
and inline `style={{ color: "var(--…)" }}` reads the result. No JS re-render is needed —
`vlinx.css:138-141` even swaps the logo through CSS `content:` precisely to avoid one.

**User toggles the scheme.** Title bar → `setTheme`/`toggleTheme` (`termStore.ts:3086-3100`):
`applyTheme` → `pushSetting("vlx-theme", mode)` → `persistAndApplyVisual` → `set({theme})` →
`notifyAgentsColorScheme`. Same four hops.

**User changes an appearance setting.** `SettingsModal.tsx:792-838` renders five `<Field>`s
(accent / density / pane / divider / nav); each setter lands in the store and ends at
`persistAndApplyVisual(get)`. Note this path **never calls `applyTheme`, so it never calls
`setXtermTheme`** — accent changes do not touch the terminal.

**Live re-skin of running terminals.** Only `setXtermTheme()` — `registry.ts:126-130`:

```ts
for (const term of registry.values()) term.options.theme = theme;
```

xterm repaints from `options.theme`. Confirmed as the *only* mutation of `options.theme` in the repo;
`options.fontFamily`/`fontSize` are updated separately at `usePtySession.ts:881-883`. New terminals
read the constant directly at construction: `usePtySession.ts:226` and `RecordingViewer.tsx:74`, both
`XTERM_THEME[resolveTheme(useTermStore.getState().theme)]`.

**Cross-shell.** `App.tsx:92-103` awaits `reconcileSettings()`; if `vlx-theme` or `vlx-settings`
changed, `hydrateSettingsFromCache()` (`termStore.ts:3474-3489`) re-runs `applyTheme` + `applyVisual`
+ `notifyAgentsColorScheme`. `App.tsx:105-107` re-runs `applyAppearance()` on OS scheme change while
in `system` mode.

---

## 3. Where a user theme would have to hook in

Five distinct insertion points. They are not mutually exclusive; a full feature needs at least one
from group A **and** one from group B.

### Group A — the CSS side

**A1. Inline custom properties on `documentElement`.** `theme.ts:156-159` already does exactly this
for two tokens: `root.style.setProperty("--font-mono", …)` / `removeProperty` when null.
*Tradeoff:* inline declarations outrank every selector, so this is the only mechanism that
**survives the `[data-accent]` preset lines at `vlinx.css:37-40` and the `[data-theme]` blocks
without touching them** — no `!important`, no specificity war, no CSS edits. Removal is symmetric
(`removeProperty`), so "revert to built-in" is free. Cost: the token list becomes a JS-side contract;
nothing stops a theme from setting a name that no rule reads.
*This is the existing pattern and the lowest-risk hook.* **Inject at the Layer-1 names** — §1a
quantifies why: 1306 of the 1573 token references (83%) point at them directly, and the 267 Layer-2
alias references follow automatically through `var()`.

**A2. An injected `<style>` element.** A generated `:root{…}` or a whole stylesheet appended to
`<head>`.
*Tradeoff:* lets a theme reach beyond variables (rules, selectors). But it loses to `[data-accent]`
on specificity unless it also matches `[data-accent]` or uses `!important`, it must be ordered after
`index.css`, and arbitrary user CSS is a much larger support and security surface. There is
currently **no such hook** — grep for `customCss|userCss|injectCss|customTheme` returns nothing,
confirmed.

**A3. A new `data-*` attribute value.** Extending `AccentName` (`theme.ts:19`) and adding rows at
`vlinx.css:37-40`.
*Tradeoff:* fits the existing idiom perfectly for *presets*, but cannot express a user-supplied
colour — the values must be in the stylesheet at build time. Only useful if the feature ships more
built-in themes rather than user themes.

### Group B — the xterm side

**B1. Make `XTERM_THEME` mutable / read through an accessor.** Three call sites read it:
`theme.ts:137` (`setXtermTheme(XTERM_THEME[resolved])`), `usePtySession.ts:226`,
`RecordingViewer.tsx:74`. All three read `XTERM_THEME[resolved]` at call time, so a mutable record
or a `getXtermTheme(resolved)` accessor reaches every one of them.
*Tradeoff:* small, contained, obvious. But the constant is `export`ed, so any consumer holding a
reference to the inner object would go stale.

**B2. Derive the `ITheme` from the CSS variables at apply time.** `DocView.tsx:402-427` already
proves this works in-repo (see §5): read `--bg-term`/`--text`/`--accent` etc. via
`getComputedStyle`, convert `oklch()`→`#rrggbb` through a 1px canvas.
*Tradeoff:* structurally eliminates the four-comment hand-sync invariant between `vlinx.css:51,78`
and `theme.ts:73,93` — one source of truth. Costs a `getComputedStyle` + canvas round-trip per apply
(negligible at this frequency), and requires the theme to define the ANSI 16 somewhere anyway, since
the token layer has only 6 hues (`--green/--yellow/--red/--cyan/--mag/--blue`), not 16.

**B3 (required regardless of B1/B2).** The appearance path does **not** currently call
`setXtermTheme` — only `applyTheme` does (§2). If terminal colours become settable outside the
light/dark toggle, `persistAndApplyVisual` / `applyVisual` needs that call too, or terminals will
lag until the next scheme switch.

### The four surfaces a theme cannot reach through A or B

1. `ACCENT_HEX` at `SettingsModal.tsx:48-53` — the picker swatches. Needs its own wiring.
2. Search-highlight colours — `registry.ts:149-152`, `RecordingViewer.tsx:27-30`,
   `highlight.tsx:13,19`. Not part of `ITheme`; passed as `SearchAddon` decoration options.
3. The pre-mount literals — `index.html:19,22`, `tauri.conf.json:21`, `electron/main.cjs:602` (§6).
4. The 33 hardcoded shadows and 14 `#fff`s (§1d.1, §1d.3) — plain literals in `style={{}}` objects.

---

## 4. Persistence

### The corrected model — read this before anything else in this section

**VelaTerm settings persistence is two-tier: a synchronous localStorage cache in front of an
authoritative SQLite backend.** It is *not* "SQLite, not localStorage" (an earlier version of this
recon's brief said that; it is wrong, and the correction was verified independently before being
written here).

- `saveSettings()` (`settings.ts:154-163`) writes `localStorage["vlx-settings"]` **first**, then
  mirrors to the backend via `pushSetting()`.
- `settingsSync.ts:8-15` states it verbatim: *"localStorage is a synchronous cache available on the
  first frame with no flash; the backend is authoritative."*
- `SYNCED_KEYS` (`settingsSync.ts:29-37`) lists seven keys, including **both** `vlx-settings` and
  `vlx-theme` — so the colour-scheme mode is on the same footing as everything else, not an exception.

**Reconciliation semantics, and why they matter here.** `reconcileSettings()`
(`settingsSync.ts:88-115`) runs once at startup: where the backend **has** a key it overwrites the
local cache and adds the key to a returned change set so callers reapply; where the backend **lacks**
a key it seeds the backend from the local value and changes nothing locally. That asymmetry is
exactly the *"at most one flash"* behaviour documented at `settingsSync.ts:10-15` — the local cache
paints the first frame instantly, and only a value another shell changed since last run can cause a
single repaint. **A user theme riding this path inherits that guarantee for free**; one stored any
other way has to re-derive it. `App.tsx:92-103` is the consumer: a changed `vlx-theme` or
`vlx-settings` triggers `hydrateSettingsFromCache()`, which re-runs `applyTheme` + `applyVisual`.

**The rest of the mechanism.**

- `pushSetting(key, value)` (`settingsSync.ts:50-55`) — 400 ms debounce, gated on `syncEnabled`.
- `flushNow()` (`settingsSync.ts:65-76`) — `invoke("set_app_settings", { entries })`.
- Rust: `command_core.rs:391-404` → `repo::get_app_settings` / `set_app_settings`; schema at
  `src-tauri/src/db/schema.rs:78-82` is **a generic `app_settings(key TEXT PRIMARY KEY, value TEXT
  NOT NULL, updated_at INTEGER)`** table. `db/mod.rs:19,264` note the file is owner-only because
  this table holds the remote-access Argon2id verifier.

**Option 1 — a new synced localStorage key (e.g. a `vlx-theme-custom` JSON blob).**
Add one entry to `SYNCED_KEYS`; everything else is inherited. **Zero Rust changes** — `app_settings`
is already an untyped key→value store. Gives: no-flash first-frame read, cross-shell sharing
(Tauri WKWebView ↔ Electron Chromium, which cannot share a WebView store — `settingsSync.ts:3-6`),
and the mobile/remote browser client picks it up through the same `reconcileSettings()` call in
`MobileApp.tsx:37`. Costs: not human-editable, not portable as a file, and the blob rides in a SQLite
row so "share your theme" means copy-paste of JSON.

**Option 2 — a `theme.json` in the app data dir.** Nothing in the current code reads a
user-authored config file; this needs new Rust: a resolved path under the Tauri app-data dir, a read
command, `tauri.conf.json`'s `plugins`/capabilities as applicable, and — if the file should be
editable while the app runs — a watcher plus an event to the frontend. It also does *not* traverse
the remote/browser client: a phone hitting the web UI has no filesystem, so the file would have to be
served through the backend anyway, i.e. Option 2 ends up needing Option 1's transport regardless.
Gives: human-editable, diffable, shareable, and portable across machines by any file-sync.

**Option 3 — both.** File as the authoring format, `app_settings` as the transport, matching the
cache/authority split the app already uses. Highest cost, but it is the only shape that satisfies
"survives restart **and** is portable between machines **and** works for the remote client."

**Ordering hazard.** `settingsSync.ts:17-20` documents a race gate: `pushSetting` must not fire
before `reconcileSettings()` completes, or startup's `applyAppearance` pushes a stale local value
over the authoritative backend. Any new theme key inherits this constraint.

---

## 5. Prior art in-repo

**No custom-theme work exists.** Verified: grep for
`custom.?(theme|css|color|palette) | user.?(theme|css|palette) | theme.?(file|json|import|export|pack)`
across `src/`, `src-tauri/src/`, `docs/`, `README.md` → **zero hits**. No theme-related TODO/FIXME
(the only `TODO` near styling is `index.css:49`, about xterm's IME overlay). `git log -40` shows no
theme-adjacent commit since the `2a642ae` public source release; the one non-local branch,
`feat/agent-orchestration`, is unrelated by name. `docs/manuals/settings-and-shortcuts_20260709_2041.md:22`
documents the current state as final: *"Accent | Accent color: follow theme, or one of four fixed colors."*

**But three genuine partial abstractions already exist and should be reused, not reinvented:**

1. **`DocView.tsx:402-427` — a working CSS-variable → hex bridge.** `cssVar(name)` reads a computed
   custom property off `documentElement`; `toHex(cssColor, fallback)` paints it into a 1×1 canvas and
   reads back `#rrggbb`, with the comment at `:401-402` explaining exactly why (react-pdf understands
   hex/rgb but not `oklch()`, and the design tokens are `oklch()`). This is precisely the machinery
   insertion point **B2** needs, already written and shipping.
2. **`theme.ts:156-159` — runtime custom-property injection with symmetric removal.** The A1 pattern,
   already in production for `--font-mono` and `--ui-fs`.
3. **`index.css:16-34` — the semantic alias layer.** Purpose-built (see its header, `index.css:5-13`)
   so that overriding base tokens propagates without touching components. Reaches 17% of token
   consumption on its own; see the quantification in §1a.

### Against the open upstream issues

Context supplied by the lead, not verified by me — I have no GitHub access from this session, so I
have **not** read the issue text and cannot confirm scope. Taking it as given that **#26 (open)**
covers Windows theming gaps and supersedes #18/#19/#20/#22, and **#22 (closed)** requested terminal
background images:

**#22 — terminal background images: no anticipation in code, at all.** Grep across `src/`,
`src-tauri/src/`, `electron/` for `backgroundImage|background-image|allowTransparency|wallpaper`
returns **zero hits**. The terminal background is a single opaque colour in two places
(`vlinx.css:51,78` `--bg-term`; `theme.ts:83,94` `ITheme.background`). Nothing is structured to carry
an image, and xterm's `ITheme` has no image field — background images would need the
`allowTransparency` route plus a layer behind the canvas, which is a separate feature from a colour
theme, not a subset of it. Worth stating plainly so a theme-format design does not get pulled into
scope for it.

**#26 — Windows theming gaps: three concrete things in code plausibly bear on it.** [Inference] — I
am matching code to a one-line description of the issue, not to its text:

1. **`color-scheme` never follows the app's chosen mode. ⚠ UNTESTED AT RUNTIME — verify live before
   this goes anywhere near a PR.** The *static* facts are solid: `color-scheme` is declared in exactly
   two places, `index.html:18` and `index.html:21`, both keyed to `prefers-color-scheme`, and
   `applyTheme()` (`theme.ts:133-138`) writes only `dataset.theme` — it never touches `color-scheme`
   (grep confirms no other declaration anywhere in `src/`, `index.html`, `src-tauri/`, `electron/`).
   [Inference] from there: a user on a light OS who forces the app to dark keeps `color-scheme: light`
   for the window's lifetime, so Chromium/WebView2 paints native form controls, `<select>` popups and
   the default canvas light inside a dark app. **I have not observed this happening** — the causal
   step from "the property is never updated" to "native widgets visibly mismatch" is reasoning about
   engine behaviour, not a measurement. The app's *own* scrollbars are out of scope either way:
   `vlinx.css:114-117` styles them from `--border-strong`/`--text-faint`.
   **If it holds, it is a fixable bug rather than an OS ceiling**, and a one-line fix — which is a
   materially different conclusion from "the OS draws it, nothing to be done."
2. **The window chrome is not themed by the app.** `TitleBar.tsx:2`: *"The window retains native
   decorations; this row sits below the system title bar."* That is the whole of what the code says.
   [Unverified] whether the native title bar is *uncolorable* or merely *uncolored* — nothing in the
   repo attempts it, which is evidence of absence, not of impossibility. Do not treat a measured
   title-bar colour as proof of an OS ceiling without testing item 1 first; the two questions are
   related and the `color-scheme` finding is the reason to re-open this one.
3. **The two window background literals have no light variant.** `tauri.conf.json:21` `#23262b` and
   `electron/main.cjs:602` `#1e1e1e` are single compile-time values (§6). On Windows both shells show
   a dark frame during window creation regardless of the user's scheme.

None of these is *theme-feature* code — they are pre-existing gaps a theme feature would either
inherit or be expected to close. Flagging them so the design session can decide scope deliberately.

---

## 6. Blast radius

**The light scheme.** Every one of the 30 tokens exists in both blocks and the key sets are
identical (verified by diff) — a theme format that supplies only one scheme leaves the other on
built-ins, which is a legible-but-jarring mixed state, not a crash. The 33 hardcoded black shadows
(§1d.1) are *already* wrong in light mode; a light-leaning custom theme makes that more visible, not
less.

**The pre-mount flash — four literals, three different values.** These must agree and currently do
not:

| location | dark | light |
|---|---|---|
| `index.html:19,22` | `oklch(0.2 0.006 260)` | `oklch(0.95 0.004 260)` |
| `vlinx.css:29 / 35` | `oklch(0.2 0.006 260)` | `#ffffff` |
| `src-tauri/tauri.conf.json:21` | `#23262b` | — (single value) |
| `electron/main.cjs:602` | `#1e1e1e` | — (single value) |

The dark row is three hand-maintained near-matches. **The light row visibly disagrees**: a 0.95-
lightness grey against pure white. Tauri and Electron each carry one window colour with no light
variant at all. None of these can read a user theme — `tauri.conf.json` is compile-time, and
`index.html`'s inline `<style>` runs before any JS. A user theme with a distinct background will
flash the built-in dark grey on every cold start unless the design addresses it (the Tauri/Electron
window colour would need to be set at window-creation time from persisted state, which is a Rust /
`main.cjs` change).

**Contrast and legibility.**

- `usePtySession.ts:216` sets xterm `minimumContrastRatio: 4.5` with the comment "Enforce minimum
  contrast so ANSI white remains readable on light themes." This is a real existing guarantee: xterm
  will silently adjust a user's ANSI colours against their background. A theme author's palette may
  therefore not render literally — worth documenting, not fighting.
- **No equivalent guarantee exists for the CSS chrome.** The 14 `#fff`-on-accent sites (§1d.3,
  notably `index.css:135` `.vlx-btn-primary` and `TitleBar.tsx:214`) assume the accent is dark enough
  for white text. A pale custom accent renders primary buttons unreadable, with no clamp anywhere.
  This is the sharpest failure mode of a user-accent feature.
- `usePtySession.ts:223` `drawBoldTextInBrightColors: true` couples bold text to the bright ANSI 8 —
  a theme that sets bright≈normal loses the bold distinction.

**The mobile / remote client.** Not a separate skin: `main.tsx:41` renders `MobileApp` from the same
bundle, `MobileApp.tsx:36-37` calls the same `applyAppearance()`, and `mobile.css` is variable-driven
(§1c). So a token-based theme reaches it for free — **provided persistence goes through
`app_settings`** (§4 Option 1), since a phone browser has neither the desktop's localStorage nor its
filesystem.

**Agent notification.** `notifyAgentsColorScheme` (`termStore.ts:1363-1377`) dedupes on
`lastNotifiedScheme`, a `ResolvedTheme`. A theme change that keeps the same light/dark polarity emits
**no** DEC 2031 notification, so running Claude sessions keep their old TUI colours until the next
real scheme flip. `docs/changelog.md:237` and `docs/manuals/ai-agent-sessions_20260709_2041.md:79`
both advertise instant re-skin — a user theme would quietly break that promise unless the dedupe key
changes.

**GrokDay canvas remap.** `grokBgRemap.ts` rewrites near-white truecolor *background* SGR/OSC 11
bytes in the PTY stream to literal `#ffffff` (`grokBgRemap.ts:68`), applied unconditionally for
`session.kind === "grok"` at `usePtySession.ts:583` — **not gated on light mode**. It hardcodes the
assumption that the light terminal background is pure white (`vlinx.css:78`). A custom theme with an
off-white or dark-with-grok background gets a white patch inside the terminal.

**i18n.** Any new Settings control adds keys to **11 locale files** (`src/i18n/locales/`: de, en, es,
fr, ja, ko, pt-BR, ru, vi, zh-CN, zh-TW), following the `settings.accent` / `settings.accentAuto`
pattern at `en.ts:101-102`.

**Tests.** Effectively nil. `tests/` contains only `_mh/`; the sole colour-touching unit test is
`src/terminal/grokBgRemap.test.ts`. `vitest.setup.ts` exists solely to repair `localStorage` for
jsdom, and its comment explicitly names `loadTheme` in `theme.ts` as the module that breaks without
it — so a theme loader reading storage at module init inherits that fragility.

---

## 7. What I could not determine

- **Runtime behaviour.** No build was run (per brief). Everything above is static reading; the apply
  path is traced through call sites, not observed.
- **Exact colour equivalence** of `oklch(0.2 0.006 260)` vs `#23262b` vs `#1e1e1e`. I did not compute
  the conversion. I can assert the *light* pair disagrees (0.95 lightness ≠ pure white); the three
  dark values I can only say are separately maintained literals, not that they render identically.
- **The upstream issues themselves.** No GitHub access from this session: I could not read #26, #22,
  or the superseded #18/#19/#20. Everything in §5 about them is **inference from code absence**.
  That is legitimate evidence for *"nothing in this repo anticipates it"* and is **not** evidence
  about what the issues actually request — do not let it stand in for reading them.
- **Runtime confirmation of the `color-scheme` desync (§5, item 1).** The static facts are verified;
  the visible consequence is reasoned, not observed. Flagged inline as well.
- **Whether upstream has unpublished theme work.** Only `origin`'s `feat/agent-orchestration` is
  visible from this clone.
- **`skills/` folder contents** — not inspected; nothing in the colour grep pointed into it.
- **Whether `SourceEditor.tsx` duplicates `codeMirrorTheme.ts` literally.** `codeMirrorTheme.ts:3`
  says it carries "an equivalent definition"; a hex grep of `SourceEditor.tsx` found no literals, so
  the duplicate is presumably also `var()`-driven, but I did not read it to confirm.
