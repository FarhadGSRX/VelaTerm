//! The text of one queued message above the composer.
//!
//! A queued message can be arbitrarily long, and drawing all of it pushes the conversation off screen
//! while it waits. The text is therefore clamped to two lines; when it does not fit, a button opens a
//! dialog with the whole message. Clicking the text itself keeps its existing meaning of opening the
//! editor, so viewing and editing stay separate actions.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import Icons from "../../../components/Icons";
import { Backdrop } from "../../../components/Backdrop";
import { useT } from "../../../i18n";
import type { MessageOrigin,QueuedTextPiece } from "../../../ipc/chat";
import { MessageSender } from "./MessageSender";
import { detailTextParts } from "./DeferredContents";

export function QueuedMessageText({ text, origin, disabled, onEdit,loadPart }: {
  text: string;
  origin?: MessageOrigin;
  disabled: boolean;
  onEdit: () => void;
  loadPart?: (sequence:number,signal:AbortSignal)=>Promise<QueuedTextPiece>;
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
      {(clamped || loadPart) && (
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
      {preview && <QueuedMessagePreview text={text} origin={origin} loadPart={loadPart} onClose={() => setPreview(false)} />}
    </>
  );
}

/** The whole queued message, read-only, outside the composer's clamped row. */
function QueuedMessagePreview({ text, origin, onClose,loadPart }: { text: string; origin?: MessageOrigin; onClose: () => void;loadPart?:(sequence:number,signal:AbortSignal)=>Promise<QueuedTextPiece> }) {
  const t = useT();
  const dialogRef = useRef<HTMLDivElement>(null);
  const [sequence,setSequence]=useState(0);
  const [part,setPart]=useState<QueuedTextPiece|null>(null);
  const [error,setError]=useState<string|null>(null);
  useEffect(()=>{
    if (!loadPart) return;
    const controller=new AbortController();setPart(null);setError(null);
    void loadPart(sequence,controller.signal).then(next=>{
      if (!controller.signal.aborted) setPart(next);
    }).catch(cause=>{if (!controller.signal.aborted) setError(String(cause));});
    return ()=>controller.abort();
  },[loadPart,sequence]);

  useEffect(() => {
    const dialog = dialogRef.current;
    const previousFocus = document.activeElement;
    dialog?.focus();
    return () => {
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  }, []);

  return createPortal(
    <Backdrop onClose={onClose}><div
      ref={dialogRef}
      className="sv-queue-preview sv-queue-preview-panel"
      role="dialog"
      aria-modal="true"
      tabIndex={-1}
      aria-label={t("chat.queue.view")}
      onKeyDown={(event) => {event.stopPropagation();if (event.key==="Escape") onClose();}}
    >
        <div className="sv-queue-preview-toolbar">
          {origin && <MessageSender origin={origin} />}
          {loadPart && <>
            <span>{t("chat.history.part")} {sequence+1}</span>
            {sequence>0 && <button className="vlx-btn" onClick={()=>setSequence(sequence-1)}>{t("common.prev")}</button>}
            {part && !part.eof && <button className="vlx-btn" onClick={()=>setSequence(sequence+1)}>{t("common.next")}</button>}
          </>}
          <button type="button" className="sv-queue-preview-close" onClick={onClose} autoFocus>
            {t("common.close")}
          </button>
        </div>
        <div className="sv-queue-preview-body">
          {error?<div role="alert">{error}</div>:loadPart && !part?<div role="status">{t("common.loading")}</div>:
            detailTextParts(part?.text??text).map((piece,index)=><span key={index}>{piece}</span>)}
        </div>
    </div></Backdrop>,
    document.body,
  );
}
