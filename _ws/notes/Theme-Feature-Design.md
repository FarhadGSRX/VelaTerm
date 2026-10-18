# Design — user-definable colour themes

Working design for the upstream feature. Status: **draft, pre-implementation.** Inputs:
`dev/_pending/theme-surface.md` (decision doc) and `dev/_pending/theme-inventory.md` (cited inventory).

## The framing that matters

**The palette is not the contribution; the mechanism is.** AdventureTime is fg's taste and should not
ship as a built-in — maintainers rightly resist that. It becomes a file users load, and a good demo
in the PR description.

---

## Finding that reorders the whole plan

**All four of VelaTerm's own accents already fail WCAG AA for the white text drawn on them.**
Computed by converting each `[data-accent]` oklch value (`vlinx.css:37-40`) to sRGB and taking the
WCAG contrast ratio against `#fff`:

| accent | sRGB | contrast vs `#fff` | AA (4.5) |
|---|---|---|---|
| green | `#40c786` | 2.16 | **FAIL** |
| blue | `#4ba3f7` | 2.66 | **FAIL** |
| amber | `#f4a437` | 2.06 | **FAIL** |
| violet | `#b48df4` | 2.62 | **FAIL** |

The white is hardcoded: `index.css:132-136` — `.vlx-btn-primary { background: var(--accent); color: #fff }`
— plus 10 further sites setting `background: var(--accent)` with white or near-white text
(`QuitConfirmModal.tsx:156`, `UpdateModal.tsx:405,418,450`, `NotifyGuideModal.tsx:75`,
`DocView.tsx:648`, `docTheme.css:398,410`, …).

Observed in the wild: the amber **Quit** button in the quit-confirmation dialog, 2.06:1.

**This is a shipping accessibility bug, independent of any theme feature.** It also happens to be the
exact prerequisite the theme feature needs, which is why it becomes Slice A.

Contrast: the *terminal* is already protected — xterm is constructed with
`minimumContrastRatio: 4.5` (`usePtySession.ts:216`). The CSS chrome has no equivalent. So the app
already asserts this standard for itself in one half and violates it in the other.

---

## The contrast rule

Introduce **`--accent-fg`**: the foreground to use on top of `--accent`, chosen per accent rather
than hardcoded.

- Built-in accents get an explicit, checked value.
- A user theme may supply one; if absent, derive it by picking whichever of black/white scores higher
  against the accent (standard relative-luminance choice).
- The 11 hardcoded whites become `var(--accent-fg)`.

Deriving rather than validating is deliberate: a theme that would be unreadable becomes readable
instead of rejected. No user gets told their colours are wrong; they just work.

**Open question:** do we also guard `--accent-soft` / `--accent-line` (the 14%/50% alpha variants)?
They sit under text elsewhere. Probably yes, but not measured yet.

---

## Format

### Constraint discovered in recon

`--accent-rgb` is composed as a **bare RGB triple inside `rgba()`** —
`var(--accent-dim, rgba(var(--accent-rgb, 63,207,142), 0.12))` at `QuitConfirmModal.tsx:97` and
`LiveTabsOverLimitDialog.tsx:115`. **A hex-only format cannot satisfy it**, so the loader must parse
colours to RGB rather than pass strings through.

v1 accepts `#rgb` / `#rrggbb` / `#rrggbbaa` only. Trivial to parse, unambiguous, and the alpha
variants and the RGB triple are all derivable from it. Wider CSS colour syntax can come later without
breaking files.

### Shape — small core, derive the rest

A theme must not require ~80 values. The AdventureTime port itself derived nearly everything from a
handful: surfaces are the background at other lightnesses, muted text is a step off the foreground.

```jsonc
{
  "name": "Adventure Time",
  "variant": "dark",              // dark | light — which scheme this replaces
  "core": {
    "bg": "#1f1d45",              // terminal ground; surfaces derive from it
    "fg": "#f8dcc0",
    "accent": "#efbf38",
    "cursor": "#efbf38",
    "selection": "#706b4e"
  },
  "ansi":   ["#050404","#bd0013","#4ab118","#e7741e","#0f4ac6","#665993","#70a598","#f8dcc0"],
  "bright": ["#4e7cbf","#fc5f5a","#9eff6e","#efc11a","#1997c6","#9b5953","#c8faf4","#f6f5fb"],
  "overrides": { "--bg-1": "#191738" }   // optional escape hatch, any token by name
}
```

`core` + `ansi` + `bright` = 21 required values. Everything else derives, with `overrides` as the
escape hatch for anyone who wants exact control.

**Open question — the derivation itself is the risky part.** Deriving surfaces by lightness steps
works for AdventureTime because its palette was built that way. Whether it produces acceptable
results for arbitrary user colours is **unverified**, and it is the single most likely thing to make
this feature feel bad. Alternative: require the ~8 surface tokens explicitly and derive nothing.
Needs a decision before implementation.

---

## Slices

Each independently defensible; if an early one stalls, little is wasted.

**A — `--accent-fg`, replacing the hardcoded whites.** A bug fix, not a feature. Objectively correct
(WCAG is a standard, not taste), fixes a real defect for every current user, and is the prerequisite
for everything below. Small: one token, four built-in values, 11 call sites.

**B — single palette source of truth (pure refactor).** Today the palette exists twice: CSS custom
properties in `vlinx.css` and `XTERM_THEME` in `theme.ts`, hand-kept in sync (their own comments say
so). Collapse to one structure feeding both. **Zero behaviour change** — easy to review, and it is
the load-bearing prerequisite.

**C — load and apply a user theme, no UI.** Read from settings, parse, apply via
`documentElement.style.setProperty` (the `theme.ts:156-159` precedent) and `setXtermTheme()`. Persist
with one `SYNCED_KEYS` entry — `app_settings` is a generic k/v table, so **no Rust changes and no
schema migration**.

**D — import/manage UI.** List, select, import, delete, export. Carries **11 languages** of i18n
strings, which is most of the work.

---

## What this costs upstream to accept

- No Rust changes (confirmed independently by two surveys).
- No DB migration.
- Slice A touches 11 files trivially; B is a refactor; C is ~1 new module; D is UI + i18n.
- Must respect the ESLint boundary: only `src/platform/` may import `@tauri-apps/*`.

## Contribution-strategy note

PRs have been the working channel on this repo: five merged, while the feature-request issues did
not lead anywhere. So: no issue first; lead with Slice A as a small, obviously-correct bug fix
and read the review style off it before investing in B–D.

---

---

# Codex review — revisions to the above

Consulted 2026-08-23 (`consult-codex`, repo-aware attempted). **Caveat: Codex's Windows read-only
sandbox rejected its own shell (`CreateProcess ... blocked by policy`), so it could NOT read the
source. This is a design review of the summary above, not a code-verified one.** Its structural
points still hold; anything it asserts about specific files is inference.

## 1. The format was UI-incomplete — the central flaw

My 21 required values are **terminal-complete but UI-incomplete**. And `overrides` keyed by raw
token name (`--bg-2`) leaks today's accidental CSS architecture as a permanent public API.

Instead, require a small **semantic UI** set, named for meaning rather than internals:

`canvas` · `surface` · `surfaceRaised` · `text` · `textMuted` · `border` · `accent` · `accentText`
— plus, strongly considered: `selection` · `focusRing`

Hover / pressed / subtle-border / disabled are then derived by a **documented** algorithm, still
overridable. Terminal colours stay a separate required section (fg/bg, cursor, cursorAccent,
selection, 8 ANSI + 8 bright).

## 2. Two scopes — its strongest recommendation, and I'm adopting it

```jsonc
{ "scope": "terminal" | "application" | "both" }
```

A **terminal-only** theme leaves the UI palette alone and changes xterm only. This matters more than
it looks: it makes the thousands of existing terminal themes (iTerm, Windows Terminal, base16)
importable **without pretending they define an entire application skin**. An application theme
likewise should never be inferred from ANSI colours — eight ANSI values are not a sensible source for
UI surfaces or syntax highlighting.

## 3. Slice A is a contrast *audit*, not a mechanical replace

Separable technically; socially only if scoped carefully. It visibly flips primary buttons from white
to black text on all four accents — objectively more readable, arguably less "branded", and a
maintainer may reject a global visual change made mechanically. So:

1. Add `--accent-fg` to each shipped accent as an **explicit, reviewable value** (generated once, then
   written down — design intent should be reviewable). Runtime derivation is for *user* colours only.
2. **Classify each of the ~11 sites individually**; do not sed the whites.
3. Also inspect icon fills, SVG strokes, box shadows, focus outlines, spinner colours, nested links —
   changing `color` is insufficient if children hardcode white.
4. Add **automated contrast tests** per built-in accent and interaction state.
5. Include before/after screenshots in the PR.

## 4. The light scheme is where derivation breaks

Confirms the worry. A "raised surfaces are lighter" algorithm is wrong for light themes, which often
need *darker* separators and barely-lighter raised surfaces. Also flagged: shadows tuned for dark
grounds, fixed black/white translucent overlays, status colours that pass on dark and fail on light,
focus rings vanishing against accent or canvas, and portaled popovers mounted outside the themed
subtree.

CodeMirror/doc surfaces are their own contrast domain — gutter, active line, matching bracket, search
match, diagnostics, autocomplete tooltips — and may hold literal colours rather than variables.

## 5. Security — the thing I had underweighted

An imported theme is untrusted input reaching `style.setProperty`. Required:

- **Strictly parse; never pass an imported string through.** Reject `var()`, `url()`, malformed
  functions, and any token-breaking punctuation.
- Cap file size, theme count, name and metadata length.
- **Version the format from day one.**

## 6. Alpha policy

Disallow alpha on foundational colours (canvas, surfaces, text, terminal background) — otherwise
contrast depends on an unstated backing surface. Allow it only on explicit overlay/selection tokens.

## 7. Hex-only v1 — endorsed, with a correction

Fine at the import boundary. But do **not** align the public format with OKLCH merely because the
built-ins use it: authoring format ≠ interchange format. Normalize internally to numeric sRGB + alpha,
convert to OKLCH for contrast/variant maths, emit whatever CSS serves the browser. Wide-gamut is a
deliberate deferral (gamut mapping, cross-webview consistency, clients that can't render P3).

## 8. Cross-client is a protocol, not a setting

If mobile/remote render the same bundle this is easy; if not, a synced theme becomes a **cross-client
protocol** needing version and capability fields. A client that cannot render a theme must fall back
safely while **preserving** it rather than overwriting. Don't sync the active custom-theme ID until
every client can resolve it. Apply before first meaningful paint or it flashes the default.

## 9. Revised slice order — contract before refactor

1. Accessibility / semantic-foreground audit (was Slice A)
2. **Define and test the semantic theme contract**, with light *and* dark reference themes ← new
3. Centralize built-in palette data; generate and validate CSS + xterm outputs (was Slice B)
4. Runtime application: rollback, startup behaviour, schema versioning, invalid-theme recovery
5. Persistence + cross-client capability handling
6. Import/export, no management UI
7. Management UI + translations

The inserted step 2 is the point: **centralize after the contract exists, or you elegantly centralize
the wrong model.**

---

## Open questions

1. ~~Derive surfaces, or require them explicitly?~~ **Resolved:** require a semantic UI set; derive
   only interaction states, by a documented algorithm.
2. Guard `--accent-soft` / `--accent-line` too? (still open)
3. ~~One file for both schemes?~~ Superseded by `scope` + explicit light/dark reference themes.
4. Import by file picker, paste, or both?
5. Terminal font in a theme, or out of scope?
6. **New:** do mobile/remote render this same bundle? Decides whether §8 is trivial or a protocol.
