//! Named terminal schemes: saved values are validated, a named scheme replaces the palette whatever the app
//! mode, and the terminal pane and Claude's light/dark theme follow the scheme's own brightness.

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { applyVisual, terminalBase, xtermTheme, XTERM_THEME, type VisualSettings } from "../theme";
import { TERMINAL_SCHEMES, sanitizeTerminalScheme } from "./schemes";

const visual = (terminalScheme: VisualSettings["terminalScheme"]): VisualSettings => ({
  darkStyle: "classic",
  accent: "auto",
  density: "regular",
  paneStyle: "flush",
  dividerStyle: "subtle",
  navLayout: "tree",
  terminalScheme,
  uiFontFamily: null,
  uiFontSize: null,
  chatFontFamily: null,
  chatFontSize: 13.5,
  chatLineHeight: 1.5,
});

describe("terminal schemes", () => {
  // The app applies a theme before any visual settings, so applyVisual never has to ask the OS.
  beforeEach(() => {
    document.documentElement.dataset.theme = "dark";
  });
  afterEach(() => applyVisual(visual("auto")));

  it("keeps known names and falls back to auto for anything else", () => {
    expect(sanitizeTerminalScheme("adventure-time")).toBe("adventure-time");
    expect(sanitizeTerminalScheme("auto")).toBe("auto");
    expect(sanitizeTerminalScheme("toString")).toBe("auto");
    expect(sanitizeTerminalScheme(42)).toBe("auto");
    expect(sanitizeTerminalScheme(undefined)).toBe("auto");
  });

  it("uses the built-in palettes until a scheme is chosen", () => {
    applyVisual(visual("auto"));
    expect(xtermTheme("light")).toBe(XTERM_THEME.light);
    expect(terminalBase("light")).toBe("light");
    expect(document.documentElement.style.getPropertyValue("--bg-term")).toBe("");
  });

  it("applies a named scheme in either mode and follows its brightness", () => {
    applyVisual(visual("adventure-time"));
    const scheme = TERMINAL_SCHEMES["adventure-time"];
    expect(xtermTheme("light")).toBe(scheme.theme);
    expect(xtermTheme("dark")).toBe(scheme.theme);
    expect(terminalBase("light")).toBe("dark");
    expect(document.documentElement.style.getPropertyValue("--bg-term")).toBe("#1f1d45");

    applyVisual(visual("one-half-light"));
    expect(terminalBase("dark")).toBe("light");
  });

  it("gives every scheme the colors xterm would otherwise leave invisible", () => {
    for (const { theme } of Object.values(TERMINAL_SCHEMES)) {
      expect(theme.selectionBackground).toBeTruthy();
      expect(theme.cursorAccent).toBeTruthy();
      expect(theme.white).not.toBe(theme.background);
    }
  });
});
