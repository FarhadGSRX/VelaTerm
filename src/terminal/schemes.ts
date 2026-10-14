//! Named terminal color schemes, chosen in Settings → Appearance.
//!
//! "auto" keeps the built-in VelaTerm palettes (`XTERM_THEME` in theme.ts), which follow the app's light/dark
//! mode. A named scheme replaces the whole xterm palette regardless of that mode, and its `base` says which
//! brightness it is drawn for: the terminal pane's background follows the scheme, and Claude Code is launched
//! with the matching light/dark theme so its own colors stay legible on it.
//!
//! Palettes are the schemes' published values (AdventureTime from iTerm2-Color-Schemes), except that One Half
//! Light's `white` is darkened from its background color so white text stays visible. Each one sets the
//! selection and cursor-accent colors explicitly, for the same reason XTERM_THEME does: xterm's defaults
//! disappear on some backgrounds.

import type { ITheme } from "@xterm/xterm";

export type ResolvedBase = "dark" | "light";

export interface TerminalSchemeDef {
  label: string;
  base: ResolvedBase;
  theme: ITheme;
}

export const TERMINAL_SCHEMES = {
  "adventure-time": {
    label: "Adventure Time",
    base: "dark",
    theme: {
      background: "#1f1d45", foreground: "#f8dcc0", cursor: "#efbf38", cursorAccent: "#08080a",
      selectionBackground: "#706b4e", selectionInactiveBackground: "#4c484a",
      black: "#050404", red: "#bd0013", green: "#4ab118", yellow: "#e7741e",
      blue: "#0f4ac6", magenta: "#665993", cyan: "#70a598", white: "#f8dcc0",
      brightBlack: "#4e7cbf", brightRed: "#fc5f5a", brightGreen: "#9eff6e", brightYellow: "#efc11a",
      brightBlue: "#1997c6", brightMagenta: "#9b5953", brightCyan: "#c8faf4", brightWhite: "#f6f5fb",
    },
  },
  dracula: {
    label: "Dracula",
    base: "dark",
    theme: {
      background: "#282a36", foreground: "#f8f8f2", cursor: "#f8f8f2", cursorAccent: "#282a36",
      selectionBackground: "#44475a", selectionInactiveBackground: "#383a4a",
      black: "#21222c", red: "#ff5555", green: "#50fa7b", yellow: "#f1fa8c",
      blue: "#bd93f9", magenta: "#ff79c6", cyan: "#8be9fd", white: "#f8f8f2",
      brightBlack: "#6272a4", brightRed: "#ff6e6e", brightGreen: "#69ff94", brightYellow: "#ffffa5",
      brightBlue: "#d6acff", brightMagenta: "#ff92df", brightCyan: "#a4ffff", brightWhite: "#ffffff",
    },
  },
  "gruvbox-dark": {
    label: "Gruvbox Dark",
    base: "dark",
    theme: {
      background: "#282828", foreground: "#ebdbb2", cursor: "#ebdbb2", cursorAccent: "#282828",
      selectionBackground: "#504945", selectionInactiveBackground: "#3c3836",
      black: "#282828", red: "#cc241d", green: "#98971a", yellow: "#d79921",
      blue: "#458588", magenta: "#b16286", cyan: "#689d6a", white: "#a89984",
      brightBlack: "#928374", brightRed: "#fb4934", brightGreen: "#b8bb26", brightYellow: "#fabd2f",
      brightBlue: "#83a598", brightMagenta: "#d3869b", brightCyan: "#8ec07c", brightWhite: "#ebdbb2",
    },
  },
  nord: {
    label: "Nord",
    base: "dark",
    theme: {
      background: "#2e3440", foreground: "#d8dee9", cursor: "#d8dee9", cursorAccent: "#2e3440",
      selectionBackground: "#434c5e", selectionInactiveBackground: "#3b4252",
      black: "#3b4252", red: "#bf616a", green: "#a3be8c", yellow: "#ebcb8b",
      blue: "#81a1c1", magenta: "#b48ead", cyan: "#88c0d0", white: "#e5e9f0",
      brightBlack: "#4c566a", brightRed: "#bf616a", brightGreen: "#a3be8c", brightYellow: "#ebcb8b",
      brightBlue: "#81a1c1", brightMagenta: "#b48ead", brightCyan: "#8fbcbb", brightWhite: "#eceff4",
    },
  },
  "tokyo-night": {
    label: "Tokyo Night",
    base: "dark",
    theme: {
      background: "#1a1b26", foreground: "#c0caf5", cursor: "#c0caf5", cursorAccent: "#1a1b26",
      selectionBackground: "#33467c", selectionInactiveBackground: "#283457",
      black: "#15161e", red: "#f7768e", green: "#9ece6a", yellow: "#e0af68",
      blue: "#7aa2f7", magenta: "#bb9af7", cyan: "#7dcfff", white: "#a9b1d6",
      brightBlack: "#414868", brightRed: "#f7768e", brightGreen: "#9ece6a", brightYellow: "#e0af68",
      brightBlue: "#7aa2f7", brightMagenta: "#bb9af7", brightCyan: "#7dcfff", brightWhite: "#c0caf5",
    },
  },
  "one-half-light": {
    label: "One Half Light",
    base: "light",
    theme: {
      background: "#fafafa", foreground: "#383a42", cursor: "#383a42", cursorAccent: "#fafafa",
      selectionBackground: "#bfceff", selectionInactiveBackground: "#dde4f5",
      black: "#383a42", red: "#e45649", green: "#50a14f", yellow: "#c18401",
      blue: "#0184bc", magenta: "#a626a4", cyan: "#0997b3", white: "#a0a1a7",
      brightBlack: "#4f525e", brightRed: "#e06c75", brightGreen: "#98c379", brightYellow: "#e5c07b",
      brightBlue: "#61afef", brightMagenta: "#c678dd", brightCyan: "#56b6c2", brightWhite: "#ffffff",
    },
  },
} satisfies Record<string, TerminalSchemeDef>;

export type NamedTerminalScheme = keyof typeof TERMINAL_SCHEMES;
export type TerminalScheme = "auto" | NamedTerminalScheme;

/** The scheme a saved value names, or null for "auto" and for anything unrecognised. */
export function namedScheme(value: unknown): TerminalSchemeDef | null {
  return typeof value === "string" && Object.hasOwn(TERMINAL_SCHEMES, value)
    ? TERMINAL_SCHEMES[value as NamedTerminalScheme]
    : null;
}

/** Restore a saved choice from untrusted storage; unknown values fall back to the built-in palettes. */
export function sanitizeTerminalScheme(value: unknown): TerminalScheme {
  return namedScheme(value) ? (value as NamedTerminalScheme) : "auto";
}
