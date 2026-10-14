//! Dark styles: a saved style is validated, a themed style recolors the chrome through `data-dark-style` and brings
//! its own terminal palette in dark mode only, and an explicitly chosen terminal scheme still wins over it.

import { afterEach, describe, expect, it } from "vitest";
import { applyTheme, applyVisual, sanitizeDarkStyle, xtermTheme, XTERM_THEME, type VisualSettings } from "./theme";
import { TERMINAL_SCHEMES } from "./terminal/schemes";

const visual = (overrides: Partial<VisualSettings>): VisualSettings => ({
  darkStyle: "classic",
  accent: "auto",
  density: "regular",
  paneStyle: "flush",
  dividerStyle: "subtle",
  navLayout: "tree",
  terminalScheme: "auto",
  uiFontFamily: null,
  uiFontSize: null,
  chatFontFamily: null,
  chatFontSize: 13.5,
  chatLineHeight: 1.5,
  ...overrides,
});

describe("dark styles", () => {
  afterEach(() => {
    applyTheme("dark", "classic");
    applyVisual(visual({}));
  });

  it("keeps known styles and falls back to classic for retired or unknown ones", () => {
    expect(sanitizeDarkStyle("adventure-time")).toBe("adventure-time");
    expect(sanitizeDarkStyle("classic")).toBe("classic");
    for (const retired of ["soft", "cool", "crisp", "includes", undefined, 7]) {
      expect(sanitizeDarkStyle(retired)).toBe("classic");
    }
  });

  it("marks the document so the chrome tokens apply", () => {
    applyTheme("dark", "adventure-time");
    expect(document.documentElement.dataset.darkStyle).toBe("adventure-time");
  });

  it("paints the terminal with the style's palette in dark mode and leaves light mode alone", () => {
    applyTheme("dark", "adventure-time");
    expect(xtermTheme("dark")).toBe(TERMINAL_SCHEMES["adventure-time"].theme);
    expect(xtermTheme("light")).toBe(XTERM_THEME.light);

    applyTheme("dark", "classic");
    expect(xtermTheme("dark")).toBe(XTERM_THEME.dark);
  });

  it("lets an explicitly chosen terminal scheme override the style's palette", () => {
    applyTheme("dark", "adventure-time");
    applyVisual(visual({ darkStyle: "adventure-time", terminalScheme: "dracula" }));
    expect(xtermTheme("dark")).toBe(TERMINAL_SCHEMES.dracula.theme);
  });
});
