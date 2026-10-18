import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { Backdrop } from "../../../components/Backdrop";
import { useImageMenu } from "./useImageMenu";
import { useT } from "../../../i18n";

/** Display the attachment's original bytes outside the transcript's clipped, virtualized layout. */
export function ChatImagePreview({ src, alt, onClose,original,loadOriginal }: { src: string; alt: string; onClose: () => void;original?:()=>Promise<string>;loadOriginal?:(signal?:AbortSignal)=>Promise<string> }) {
  const t = useT();
  const dialogRef = useRef<HTMLDivElement>(null);
  const [fit, setFit] = useState(true);
  const [failed, setFailed] = useState(false);
  const [fullSrc,setFullSrc]=useState<string|null>(()=>loadOriginal ? null:src);
  const [readError,setReadError]=useState<string|null>(null);
  const { onContextMenu, imageMenu } = useImageMenu(fullSrc??src, () => setFit(false),original);
  useEffect(()=>{
    if (!loadOriginal) return;
    const controller=new AbortController();
    let disposed=false;
    void loadOriginal(controller.signal).then(value=>{if (!disposed) setFullSrc(value);}).catch(cause=>{if (!disposed) setReadError(String(cause));});
    return ()=>{disposed=true;controller.abort();};
  },[loadOriginal]);

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
      className="sv-image-preview sv-image-preview-panel"
      role="dialog"
      aria-modal="true"
      tabIndex={-1}
      aria-label={alt || t("crepe.image")}
      onKeyDown={(event) => {event.stopPropagation();if (event.key==="Escape") onClose();}}
    >
        <div className="sv-image-preview-toolbar">
          <button type="button" aria-pressed={fit} onClick={() => setFit(true)}>
            {t("doc.imgFit")}
          </button>
          <button type="button" aria-pressed={!fit} onClick={() => setFit(false)}>
            {t("doc.imgActual")}
          </button>
          <button type="button" className="sv-image-preview-close" onClick={onClose} autoFocus>
            {t("common.close")}
          </button>
        </div>
        <div className={`sv-image-preview-body${fit ? " is-fit" : ""}`}>
          {readError ? <span role="alert">{readError}</span> : fullSrc===null ? <span role="status">{t("common.loading")}</span> : failed ? (
            <span role="alert">{t("doc.imgDecodeFailed")}</span>
          ) : (
            <img src={fullSrc} alt={alt} onContextMenu={onContextMenu} onError={() => setFailed(true)} />
          )}
        </div>
      {imageMenu}
    </div></Backdrop>,
    document.body,
  );
}
