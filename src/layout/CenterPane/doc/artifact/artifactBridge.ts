//! Messages from the script the backend injects into every artifact page.
//!
//! The page is untrusted, so its messages are accepted only from the iframe we created and only in the
//! shapes listed here, with every field type-checked and clipped. They feed displays (error count, console,
//! blocked-request banner, scroll restore) and never trigger an action.

/** Source tag the injected script puts on every message. */
export const BRIDGE_SOURCE = "vlx-artifact";

/** Longest message text kept; the script already clips, this guards against a page posting its own. */
const MAX_TEXT = 2000;

export type BridgeMessage =
  | { type: "ready" }
  | { type: "console"; level: "error" | "warn"; text: string }
  | { type: "error"; text: string; line: number }
  | { type: "blocked"; uri: string; directive: string }
  | { type: "scroll"; y: number };

function clip(v: unknown, max = MAX_TEXT): string | null {
  return typeof v === "string" ? v.slice(0, max) : null;
}

function int(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) && v >= 0 ? Math.min(Math.floor(v), 10_000_000) : null;
}

/** Validate one posted message; null for anything that is not a well-formed bridge message. */
export function parseBridgeMessage(data: unknown): BridgeMessage | null {
  if (!data || typeof data !== "object") return null;
  const m = data as Record<string, unknown>;
  if (m.source !== BRIDGE_SOURCE) return null;
  switch (m.type) {
    case "ready":
      return { type: "ready" };
    case "console": {
      const text = clip(m.text);
      if (text === null || (m.level !== "error" && m.level !== "warn")) return null;
      return { type: "console", level: m.level, text };
    }
    case "error": {
      const text = clip(m.text);
      if (text === null) return null;
      return { type: "error", text, line: int(m.line) ?? 0 };
    }
    case "blocked": {
      const uri = clip(m.uri, 300);
      const directive = clip(m.directive, 64);
      if (uri === null || directive === null) return null;
      return { type: "blocked", uri, directive };
    }
    case "scroll": {
      const y = int(m.y);
      return y === null ? null : { type: "scroll", y };
    }
    default:
      return null;
  }
}

/** One line in the console drawer. */
export interface ConsoleEntry {
  id: number;
  level: "error" | "warn";
  text: string;
  line?: number;
}

/** Entries kept per page load; the oldest are dropped first. */
export const MAX_CONSOLE = 200;

/** Append an entry, keeping the newest `MAX_CONSOLE`. */
export function pushConsole(list: ConsoleEntry[], entry: ConsoleEntry): ConsoleEntry[] {
  const next = list.length >= MAX_CONSOLE ? list.slice(list.length - MAX_CONSOLE + 1) : list.slice();
  next.push(entry);
  return next;
}

/** Text handed to an agent: the page path and its errors, so it can fix them without asking. */
export function consoleReport(path: string, entries: ConsoleEntry[]): string {
  const lines = entries
    .filter((e) => e.level === "error")
    .slice(-20)
    .map((e) => `- ${e.text}${e.line ? ` (line ${e.line})` : ""}`);
  return `The page ${path} reports these errors in the preview:\n${lines.join("\n")}\nPlease fix them.`;
}
