//! Message box above the soft keyboard in the mobile terminal view. The user writes, edits, dictates,
//! or composes CJK text in a native textarea and sends the whole message to the PTY at once, instead of
//! typing keystroke by keystroke into xterm's hidden input.
//!
//! Return inserts a newline, as on the phone keyboard; the send button, Cmd+Enter, or Ctrl+Enter
//! submits. Buttons prevent the pointerdown default so the textarea keeps focus and the keyboard stays up.

import { useLayoutEffect, useRef, useState } from "react";
import Icons from "../components/Icons";
import { useT } from "../i18n";
import { imagesFromClipboard } from "../terminal/imageInput";
import { submitToTerminal } from "./terminalSubmit";

// Unsent text per session for this page load, so closing the box or leaving the session keeps it.
const drafts = new Map<string, string>();

function storeDraft(sessionId: string, text: string) {
  if (text) drafts.set(sessionId, text);
  else drafts.delete(sessionId);
}

export function TerminalComposer({
  sessionId,
  inputRef,
  bracketed,
  onClose,
  onImages,
}: {
  sessionId: string;
  /** Owned by the parent, which focuses the textarea in the same tap that opens the box. */
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
  /** Whether to wrap the message in bracketed paste, read at send time. */
  bracketed: () => boolean;
  onClose: () => void;
  /** Pasted images take the shared upload-and-inject path used by the terminal. */
  onImages: (files: File[]) => void;
}) {
  const t = useT();
  const [draft, setDraftState] = useState(() => drafts.get(sessionId) ?? "");
  const [sending, setSending] = useState(false);
  const [failed, setFailed] = useState(false);
  const draftRef = useRef(draft);
  const sendingRef = useRef(false);

  const setDraft = (text: string) => {
    draftRef.current = text;
    storeDraft(sessionId, text);
    setDraftState(text);
  };

  // Grow with the text; the stylesheet caps the height and the textarea scrolls beyond it.
  useLayoutEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [draft, inputRef]);

  const send = async () => {
    const text = draftRef.current;
    if (!text.trim() || sendingRef.current) return;
    sendingRef.current = true;
    setSending(true);
    setFailed(false);
    try {
      await submitToTerminal(sessionId, text, bracketed());
      // Keep anything typed while the message was going out.
      if (draftRef.current === text) setDraft("");
    } catch (e) {
      console.warn("[mobile] terminal message failed", e);
      setFailed(true);
    } finally {
      sendingRef.current = false;
      setSending(false);
    }
  };

  const keepFocus = (e: React.SyntheticEvent) => e.preventDefault();

  return (
    <div className="m-compose">
      {failed && <p className="m-compose-error" role="alert">{t("chat.submission.failed")}</p>}
      <div className="m-compose-row">
        <button type="button" className="m-compose-btn" aria-label={t("common.close")}
          title={t("common.close")} onPointerDown={keepFocus} onClick={onClose}>
          <Icons.close size={16} />
        </button>
        <textarea
          ref={inputRef}
          rows={1}
          value={draft}
          aria-label={t("mobile.compose")}
          placeholder={t("mobile.composePlaceholder")}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void send();
            }
          }}
          onPaste={(e) => {
            const imgs = imagesFromClipboard(e.clipboardData);
            if (imgs.length) {
              e.preventDefault();
              onImages(imgs);
            }
          }}
        />
        <button type="button" className="m-compose-btn m-compose-send"
          disabled={sending || !draft.trim()}
          aria-label={sending ? t("chat.submission.sending") : t("session.send")}
          title={sending ? t("chat.submission.sending") : t("session.send")}
          onPointerDown={keepFocus} onClick={() => void send()}>
          <Icons.arrowRight size={18} />
        </button>
      </div>
    </div>
  );
}
