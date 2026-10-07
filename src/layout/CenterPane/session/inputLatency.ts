//! Optional composer input latency logging (Settings > Advanced > Input latency log).
//!
//! A keystroke is timed from its keydown to the first animation frame after its text reaches the
//! textarea (`input` or `compositionupdate`). Only keystrokes slower than the configured threshold are
//! written to the diagnostic log, as timings plus a key category; the typed text is never recorded.
//!
//! WebKit input methods complicate the pairing of keys and text:
//! - Directly committed full-width punctuation delivers its `input` before its own keydown. A keydown
//!   arriving within `REORDERED_KEYDOWN_MS` of the last text is treated as that late keydown and skipped.
//! - Some pass-through symbols deliver their text only when the next key is pressed. The oldest waiting
//!   keydown is therefore kept until text arrives, so the measured delay covers the whole wait.

import { diagnosticEvent } from "../../../ipc/transport";
import { useTermStore } from "../../../store/termStore";

const REORDERED_KEYDOWN_MS = 30;
/** A keydown still waiting after this long produced no text (for example a key the IME swallowed). */
const STALE_MS = 5000;

type KeyKind = "letter" | "digit" | "space" | "symbol" | "composition" | "other";

interface Pending {
  at: number;
  queueMs: number;
  kind: KeyKind;
  composing: boolean;
}

function producesText(e: KeyboardEvent): boolean {
  if (e.isComposing || e.keyCode === 229) return true;
  return e.key.length === 1 && !e.ctrlKey && !e.metaKey;
}

function keyKind(e: KeyboardEvent): KeyKind {
  const key = e.key;
  if ([...key].length !== 1) return e.keyCode === 229 ? "composition" : "other";
  if (key === " ") return "space";
  if (/\p{L}/u.test(key)) return e.keyCode === 229 ? "composition" : "letter";
  if (/\p{N}/u.test(key)) return "digit";
  return /[\p{P}\p{S}]/u.test(key) ? "symbol" : "other";
}

/**
 * Listens at the document so the composer can mount and remount freely; only events whose target is the
 * textarea returned by `getInput` are measured. Returns a cleanup that removes the listeners.
 */
export function attachInputLatencyLog(getInput: () => HTMLTextAreaElement | null, sessionId: string): () => void {
  let lastTextAt = Number.NEGATIVE_INFINITY;
  let pending: Pending | null = null;

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.target !== getInput() || !producesText(e)) return;
    const now = performance.now();
    if (now - lastTextAt <= REORDERED_KEYDOWN_MS) return;
    if (pending && now - pending.at < STALE_MS) return;
    pending = { at: now, queueMs: Math.max(0, now - e.timeStamp), kind: keyKind(e), composing: e.isComposing };
  };

  const onText = (e: Event) => {
    if (e.target !== getInput()) return;
    const arrived = performance.now();
    lastTextAt = arrived;
    const sample = pending;
    pending = null;
    if (!sample || arrived - sample.at > STALE_MS) return;
    requestAnimationFrame(() => {
      const painted = performance.now();
      const totalMs = Math.round(painted - sample.at);
      const thresholdMs = useTermStore.getState().inputLatencyThresholdMs;
      if (totalMs < thresholdMs) return;
      diagnosticEvent("input_latency", {
        sessionId,
        keyKind: sample.kind,
        composing: sample.composing,
        queueMs: Math.round(sample.queueMs),
        keyToTextMs: Math.round(arrived - sample.at),
        renderMs: Math.round(painted - arrived),
        totalMs,
        thresholdMs,
        domNodes: document.getElementsByTagName("*").length,
      });
    });
  };

  document.addEventListener("keydown", onKeyDown, true);
  document.addEventListener("input", onText, true);
  document.addEventListener("compositionupdate", onText, true);
  return () => {
    document.removeEventListener("keydown", onKeyDown, true);
    document.removeEventListener("input", onText, true);
    document.removeEventListener("compositionupdate", onText, true);
  };
}
