import { useId, useRef, useState } from "react";
import { Backdrop } from "../../../components/Backdrop";
import { useT } from "../../../i18n";
import { chatRestartPermissionMode } from "../../../ipc/chat";

export function usePermissionRestart(sessionId: string, onApplied: () => void, onError: (message: string) => void) {
  const t = useT();
  const dialogId = useId();
  const [pid, setPid] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);
  const handleError = (error: unknown): boolean => {
    const match = String(error).match(/CHAT_PERMISSION_RESTART_REQUIRED:(\d+)/);
    if (!match) return false;
    setPid(Number(match[1]));
    return true;
  };
  const close = () => { if (!inFlight.current) setPid(null); };
  const apply = async () => {
    if (pid === null || inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    try {
      await chatRestartPermissionMode(sessionId, pid);
      onApplied();
    } catch (error) {
      if (error instanceof Error && error.name === "TransportError") {
        // The server may have completed the restart after the connection dropped. Reconnection will
        // restore its authoritative snapshot; do not report a rollback we cannot confirm.
        onError(t("chat.permissionRestart.unconfirmed"));
        return;
      }
      const message = String(error);
      const detail = message.includes("CHAT_PERMISSION_RESTART_TASKS") ? t("chat.permissionRestart.tasks")
        : message.includes("CHAT_PERMISSION_RESTART_STALE") ? t("chat.permissionRestart.stale")
        : message.includes("CHAT_PERMISSION_RESTART_NO_HISTORY") ? t("chat.permissionRestart.noHistory")
        : message.includes("CHAT_PERMISSION_RESTART_BUSY") ? t("chat.permissionRestart.busy") : message;
      onError(t("chat.permissionRestart.failed", detail));
    } finally {
      inFlight.current = false;
      setBusy(false);
      setPid(null);
    }
  };
  const dialog = pid === null ? null : (
    <Backdrop onClose={close}>
      <div className="quit-card" role="alertdialog" aria-modal="true"
        aria-labelledby={`${dialogId}-title`} aria-describedby={`${dialogId}-body`}
        aria-busy={busy} onKeyDown={event => {
          if (event.key === "Escape") { event.stopPropagation(); close(); }
        }}>
        <div className="quit-head">
          <div className="quit-title" id={`${dialogId}-title`}>{t("chat.permissionRestart.title")}</div>
          <div className="quit-body" id={`${dialogId}-body`}>{t("chat.permissionRestart.body")}</div>
        </div>
        <div className="quit-foot">
          <button className="quit-btn ghost" disabled={busy} autoFocus onClick={close}>{t("statusbar.permRestartLater")}</button>
          <button className="quit-btn primary" disabled={busy} onClick={() => void apply()}>
            {t(busy ? "chat.permissionRestart.busy" : "chat.permissionRestart.confirm")}
          </button>
        </div>
      </div>
    </Backdrop>
  );
  return { handleError, dialog };
}
