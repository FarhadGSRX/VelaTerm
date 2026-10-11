import { useEffect, useRef, useState } from "react";
import Select from "../../components/Select";
import { useT } from "../../i18n";
import { platform } from "../../platform";
import type { FontCatalog } from "../../platform/types";

interface FontCatalogState {
  catalog: FontCatalog | null;
  loading: boolean;
}

/** Refresh when settings open or the app regains focus after a font installation. */
export function useFontCatalog(): FontCatalogState {
  const [state, setState] = useState<FontCatalogState>({ catalog: null, loading: true });
  useEffect(() => {
    let revision = 0;
    let alive = true;
    const refresh = async () => {
      const current = ++revision;
      setState({ catalog: null, loading: true });
      try {
        const catalog = await platform.fonts.catalog();
        if (alive && current === revision) setState({ catalog, loading: false });
      } catch {
        if (alive && current === revision) setState({ catalog: null, loading: false });
      }
    };
    void refresh();
    window.addEventListener("focus", refresh);
    return () => {
      alive = false;
      window.removeEventListener("focus", refresh);
    };
  }, []);
  return state;
}

// These are CSS keywords, not an installed-font catalog. Quoted keywords remain literal names.
const GENERIC_FAMILIES = new Set([
  "serif", "sans-serif", "monospace", "cursive", "fantasy", "system-ui",
  "ui-serif", "ui-sans-serif", "ui-monospace", "ui-rounded", "emoji", "math", "fangsong",
]);

/** A successful catalog match is evidence of availability; a miss is only unknown.
 * Keep CSS fallback lists intact, including quoted family names containing commas. */
export function isKnownFontSelection(value: string | null, catalog: FontCatalog | null): boolean {
  if (!value) return true;
  const families = new Set(catalog?.families.map((family) => family.toLowerCase()) ?? []);
  let quote = "";
  let token = "";
  let quoted = false;
  let matched = false;
  const accept = () => {
    const name = token.trim().toLowerCase();
    if (families.has(name) || (!quoted && GENERIC_FAMILIES.has(name))) matched = true;
    token = "";
    quoted = false;
  };
  for (let i = 0; i < value.length; i++) {
    const char = value[i];
    // CSS hexadecimal escapes require full CSS parsing; report unknown rather than guess.
    if (char === "\\") return false;
    if (quote) {
      if (char === quote) quote = "";
      else token += char;
    } else if (char === '"' || char === "'") {
      quote = char;
      quoted = true;
    } else if (char === ",") {
      accept();
    } else {
      token += char;
    }
  }
  if (quote) return false;
  accept();
  return matched;
}

/** Proportional presets, offered for the interface font only — xterm draws on a fixed-cell grid, so a
 * variable-width terminal face would break alignment. Values are full CSS stacks: fontStack() passes any
 * comma-bearing value through verbatim, which is what makes a non-monospace interface font work at all.
 * Every stack ends in a generic family, so isKnownFontSelection never flags one as unconfirmed. */
const UI_FONTS: { label: string; stack: string }[] = [
  { label: "System UI", stack: 'system-ui, -apple-system, "Segoe UI", sans-serif' },
  { label: "Inter", stack: "Inter, system-ui, sans-serif" },
  { label: "Segoe UI", stack: '"Segoe UI", system-ui, sans-serif' },
  { label: "Helvetica Neue", stack: '"Helvetica Neue", Helvetica, Arial, sans-serif' },
  { label: "Roboto", stack: "Roboto, system-ui, sans-serif" },
  { label: "Noto Sans", stack: '"Noto Sans", system-ui, sans-serif' },
];

/** Only enumerated families are suggested. Preserve saved/custom names even when enumeration fails.
 * `proportional` adds the UI-font presets above the catalog; leave it off for the terminal. */
export function FontSelect({ value, onChange, label, fonts, proportional = false }: {
  value: string | null;
  onChange: (value: string | null) => void;
  label: string;
  fonts: FontCatalogState;
  /** Also offer the proportional UI-font presets, grouped above the enumerated families. */
  proportional?: boolean;
}) {
  const t = useT();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const committed = useRef(false);
  const { catalog, loading } = fonts;
  const families = catalog?.families ?? [];
  const uiPreset = proportional ? UI_FONTS.find((f) => f.stack === value) : undefined;
  const listed = value != null && (uiPreset != null || families.includes(value));
  const CUSTOM = "\u0000custom";
  const options = [
    { value: "", label: t("settings.fontDefault") },
    { value: CUSTOM, label: t("settings.fontCustom") },
    ...(value != null && !listed ? [{ value, label: value, separatorBefore: true }] : []),
    // Presets lead when offered. Each is stored as a whole CSS stack but listed by its short label, which
    // Select echoes on the trigger because it renders the option matching the current value.
    ...(proportional ? UI_FONTS.map((f, index) => ({ value: f.stack, label: f.label, separatorBefore: index === 0 })) : []),
    ...families.map((family, index) => ({ value: family, label: family, separatorBefore: index === 0 })),
  ];
  const finish = (save: boolean) => {
    if (committed.current) return;
    committed.current = true;
    if (save) onChange(draft.trim() || null);
    setEditing(false);
  };
  const message = loading ? t("common.loading")
    : !catalog?.systemFontsAvailable ? t("settings.fontListUnavailable")
    : !isKnownFontSelection(value, catalog) ? t("settings.fontUnconfirmed")
    : null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4, width: 160 }}>
      {editing ? (
        <input
          autoFocus
          aria-label={label}
          value={draft}
          placeholder="Fira Code"
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              finish(true);
            } else if (event.key === "Escape") {
              event.stopPropagation();
              finish(false);
            }
          }}
          onBlur={() => finish(true)}
          style={{ width: "100%", boxSizing: "border-box", height: 26, padding: "0 8px", background: "var(--bg-active)", color: "var(--text)", border: "1px solid var(--accent)", borderRadius: 6, fontSize: 11.5, fontFamily: "inherit", outline: "none" }}
        />
      ) : (
        <Select
          value={value ?? ""}
          onChange={(next) => {
            if (next === CUSTOM) {
              setDraft(value ?? "");
              committed.current = false;
              setEditing(true);
            } else onChange(next || null);
          }}
          options={options}
          size="sm"
          width={160}
          menuWidth={280}
          menuPortal
          align="right"
          ariaLabel={label}
        />
      )}
      {message && <div role="status" style={{ fontSize: 10.5, color: "var(--text-muted)", lineHeight: 1.5 }}>{message}</div>}
    </div>
  );
}
