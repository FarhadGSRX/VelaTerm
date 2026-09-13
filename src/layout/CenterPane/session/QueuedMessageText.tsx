//! The text of one queued message above the composer.
//!
//! A queued message can be arbitrarily long, and drawing all of it pushes the conversation off screen
//! while it waits. The text is therefore clamped to two lines; when it does not fit, a button opens a
//! dialog with the whole message. Clicking the text itself keeps its existing meaning of opening the
//! editor, so viewing and editing stay separate actions.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import Icons from "../../../components/Icons";
import { useSuspendNativeViews } from "../../../hooks/nativeViewSuspend";
import { useBackdropDismiss } from "../../../hooks/useBackdropDismiss";
import { useT } from "../../../i18n";

export function QueuedMessageText({ text, disabled, onEdit }: {
  text: string;
  disabled: boolean;
  onEdit: () => void;
}) {
  const t = useT();
  const linesRef = useRef<HTMLSpanElement | null>(null);
  const [clamped, setClamped] = useState(false);
  const [preview, setPreview] = useState(false);

  // Measure before paint, then again whenever the pane or the text changes: the same message may fit on
  // two lines at one width and overflow at another.
  useLayoutEffect(() => {
    const lines = linesRef.current;
    if (!lines) return;
    const measure = () => setClamped(lines.scrollHeight > lines.clientHeight + 1);
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(lines);
    return () => observer?.disconnect();
  }, [text]);

  return (
    <>
      <button className="sv-queue-text" disabled={disabled} title={t("chat.queue.edit")} onClick={onEdit}>
        <span className="sv-queue-text-lines" ref={linesRef}>{text}</span>
      </button>
      {clamped && (
        <button
          type="button"
          className="sv-queue-view"
          title={t("chat.queue.view")}
          aria-label={t("chat.queue.view")}
          onClick={() => setPreview(true)}
        >
          <Icons.eye size={13} />
        </button>
      )}
      {preview && <QueuedMessagePreview text={text} onClose={() => setPreview(false)} />}
    </>
  );
}

/** The whole queued message, read-only, outside the composer's clamped row. */
function QueuedMessagePreview({ text, onClose }: { text: string; onClose: () => void }) {
  const t = useT();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const backdrop = useBackdropDismiss(onClose);
  useSuspendNativeViews();

  useEffect(() => {
    const dialog = dialogRef.current;
    const previousFocus = document.activeElement;
    dialog?.showModal();
    return () => {
      dialog?.close();
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  }, []);

  return createPortal(
    <dialog
      ref={dialogRef}
      className="sv-queue-preview"
      aria-label={t("chat.queue.view")}
      {...backdrop}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onKeyDown={(event) => event.stopPropagation()}
    >
      <div className="sv-queue-preview-panel">
        <div className="sv-queue-preview-toolbar">
          <button type="button" className="sv-queue-preview-close" onClick={onClose} autoFocus>
            {t("common.close")}
          </button>
        </div>
        <div className="sv-queue-preview-body">{text}</div>
      </div>
    </dialog>,
    document.body,
  );
}
