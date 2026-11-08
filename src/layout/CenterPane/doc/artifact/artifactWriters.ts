//! Which session last wrote each artifact, learned from the tool calls conversation views render.
//!
//! The artifact header names that session and "Send to session" fills its composer. This lives in memory
//! only: it is a convenience, and a page opened before any session touched it simply has no writer.

type Listener = () => void;

const writers = new Map<string, string>();
const listeners = new Set<Listener>();

/** Record that `sessionId` wrote `path`. */
export function noteArtifactWriter(path: string, sessionId: string): void {
  if (writers.get(path) === sessionId) return;
  writers.set(path, sessionId);
  for (const l of listeners) l();
}

export function artifactWriter(path: string): string | undefined {
  return writers.get(path);
}

export function subscribeArtifactWriters(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** True for paths the artifact preview renders. */
export function isArtifactPath(path: string): boolean {
  return /\.html?$/i.test(path);
}

/** Event asking a conversation view to show a page in its side panel. */
export const ARTIFACT_OPEN_BESIDE_EVENT = "vlx:artifact-open-beside";

export interface ArtifactOpenBesideDetail {
  sessionId: string;
  path: string;
}

/**
 * Ask the visible conversation of `sessionId` to show `path` beside it. Returns false when none took it (a
 * terminal session, a hidden pane, a phone), and the caller opens a tab instead.
 */
export function openBesideSession(sessionId: string, path: string): boolean {
  const event = new CustomEvent<ArtifactOpenBesideDetail>(ARTIFACT_OPEN_BESIDE_EVENT, { detail: { sessionId, path }, cancelable: true });
  window.dispatchEvent(event);
  return event.defaultPrevented;
}

/** Pages a conversation asked to show beside it while no view of it was on screen, by session; the newest wins. */
const heldBeside = new Map<string, string>();

/**
 * Show `path` beside conversation `sessionId` (fg-only). When no view of it is on screen and `canHold`, the path
 * waits and opens the next time that conversation is shown ("held"); otherwise the caller opens a tab.
 */
export function openBesideOrHold(sessionId: string, path: string, canHold: boolean): "beside" | "held" | "tab" {
  if (openBesideSession(sessionId, path)) return "beside";
  if (!canHold) return "tab";
  heldBeside.set(sessionId, path);
  return "held";
}

/** The page waiting to show beside `sessionId`, removed as it is taken. */
export function takeHeldBeside(sessionId: string): string | undefined {
  const path = heldBeside.get(sessionId);
  heldBeside.delete(sessionId);
  return path;
}

/** Event a conversation view listens for to put text into its composer without sending it. */
export const COMPOSER_INSERT_EVENT = "vlx:composer-insert";

export interface ComposerInsertDetail {
  sessionId: string;
  text: string;
  /** Set by a page's `compose`: the text is one line, added on a new line after anything already typed. */
  compose?: boolean;
  /** Written by the conversation view that took a `compose` event. */
  result?: "inserted";
}

export function insertIntoComposer(sessionId: string, text: string): void {
  window.dispatchEvent(new CustomEvent<ComposerInsertDetail>(COMPOSER_INSERT_EVENT, { detail: { sessionId, text } }));
}

/** Add one line to the message box of `sessionId`; "absent" when no conversation view took it. */
export function composeIntoComposer(sessionId: string, text: string): "inserted" | "absent" {
  const detail: ComposerInsertDetail = { sessionId, text, compose: true };
  window.dispatchEvent(new CustomEvent<ComposerInsertDetail>(COMPOSER_INSERT_EVENT, { detail }));
  return detail.result ?? "absent";
}
