//! `compose` from an artifact page (fg-only). A page in its session's panel folder puts one line into the message
//! box of the conversation it sits beside, and the user still presses Enter. Every rule here refuses; none trims or
//! rewrites the text, because a changed command could mean something else.

/** Longest text a page may compose. */
export const COMPOSE_MAX = 500;
/** One compose a second into the same conversation. */
export const COMPOSE_INTERVAL_MS = 1000;

export type ComposeRefusal =
  | "not-beside"
  | "not-allowed"
  | "other-session"
  | "empty"
  | "too-long"
  | "control"
  | "too-soon";

// C0 and C1 controls, DEL, and the Unicode line and paragraph separators: anything a terminal or an editor might
// take as Enter or as a key.
const CONTROL = /[\u0000-\u001f\u007f-\u009f\u2028\u2029]/;

export interface ComposeRequest {
  text: string;
  /** The session whose panel folder holds the page, from its grant; null when it is in no panel folder. */
  panelSession: string | null | undefined;
  /** The conversation the panel sits beside; null when the page is open as a tab. */
  beside: string | null | undefined;
  now: number;
}

/** What a conversation view did with the text: took it into its box, or wasn't there. */
export type ComposeFill = "inserted" | "absent";

const lastAccepted = new Map<string, number>();

/** Why a compose is refused before it reaches the message box, or null when it may go ahead. */
export function composeRefusal(r: ComposeRequest & { last?: number }): ComposeRefusal | null {
  if (!r.beside) return "not-beside";
  if (!r.panelSession) return "not-allowed";
  if (r.panelSession !== r.beside) return "other-session";
  if (!r.text.trim()) return "empty";
  if (r.text.length > COMPOSE_MAX) return "too-long";
  if (CONTROL.test(r.text)) return "control";
  if (r.last !== undefined && r.now - r.last < COMPOSE_INTERVAL_MS) return "too-soon";
  return null;
}

/** Check a compose, hand it to `fill` when it passes, and count it toward the rate limit only once it is in. */
export function runCompose(r: ComposeRequest, fill: (session: string, text: string) => ComposeFill): ComposeRefusal | null {
  const refused = composeRefusal({ ...r, last: r.beside ? lastAccepted.get(r.beside) : undefined });
  if (refused) return refused;
  const session = r.beside as string;
  if (fill(session, r.text) === "absent") return "not-beside";
  lastAccepted.set(session, r.now);
  return null;
}

/** The message box after a compose: the text alone in an empty box, else on its own line after what is there. */
export function composeIntoDraft(current: string, text: string): string {
  return current.trim() ? `${current.replace(/\s+$/, "")}\n${text}` : text;
}

/** Forget the rate-limit history; for tests. */
export function resetComposeHistory(): void {
  lastAccepted.clear();
}
