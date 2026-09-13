//! Shared guards for inline rename inputs. WKWebView inserts control characters such as U+001C through
//! beforeinput when Left/Right is pressed past an input boundary. They render as boxes and are unrelated
//! to IME; Chromium is unaffected. Strip C0/C1 and DEL so rename fields accept only normal printable text.

import { useCallback } from "react";

const CTRL_CHARS_RE = /[\u0000-\u001F\u007F-\u009F]/g;
export const stripControlChars = (s: string) => s.replace(CTRL_CHARS_RE, "");

/** Attaches a native beforeinput listener that cancels control-character insertion. React's synthetic
 *  prevention is unreliable in WebKit, while onChange sanitization can leave the DOM unchanged when
 *  state compares equal. Native cancellation prevents insertion and caret movement. Returns cleanup
 *  for React 19 ref callbacks. */
export function useCtrlCharGuard() {
  return useCallback((el: HTMLInputElement | null) => {
    if (!el) return;
    const onBeforeInput = (ev: Event) => {
      const d = (ev as InputEvent).data;
      if (typeof d === "string" && d !== stripControlChars(d)) ev.preventDefault();
    };
    el.addEventListener("beforeinput", onBeforeInput);
    return () => el.removeEventListener("beforeinput", onBeforeInput);
  }, []);
}
