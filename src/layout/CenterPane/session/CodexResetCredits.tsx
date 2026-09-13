import { useRef, useState } from "react";
import Icons from "../../../components/Icons";
import { useT } from "../../../i18n";
import { codexResetCreditConsume, usageRefresh, type CodexResetOutcome } from "../../../ipc/commands";
import { useTermStore } from "../../../store/termStore";
import { ChipPopover } from "./extras";

// A pending interaction survives pane switches and page reloads. Never replace its UUID after a timeout.
const PENDING_KEY = "vlx-codex-reset-attempt";

export function CodexResetCredits() {
  const t = useT();
  const entry = useTermStore((s) => s.usage?.codex);
  const count = entry?.error ? null : entry?.data?.resetCredits;
  const [pending, setPending] = useState(() => sessionStorage.getItem(PENDING_KEY));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const [outcome, setOutcome] = useState<CodexResetOutcome | null>(null);
  const inFlight = useRef(false);

  async function refresh() {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setError(false);
    try {
      const snap = await usageRefresh("codex", true);
      useTermStore.getState().setUsage(snap);
      if (snap.codex.error) setError(true);
    } catch {
      setError(true);
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  }

  async function consume() {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setError(false);
    setOutcome(null);
    try {
      const key = sessionStorage.getItem(PENDING_KEY) || crypto.randomUUID();
      // Persist before sending. If storage fails, no redemption request is sent.
      sessionStorage.setItem(PENDING_KEY, key);
      setPending(key);
      const result = await codexResetCreditConsume(key);
      useTermStore.getState().setUsage(result.usage);
      setOutcome(result.outcome);
      sessionStorage.removeItem(PENDING_KEY);
      setPending(null);
    } catch {
      setError(true);
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  }

  return (
    <ChipPopover glyph={<Icons.clock size={14} />} label={t("chat.resetCredits.label", count == null ? "—" : String(count))}
      title={t("chat.resetCredits.title")} width={280} fitViewport onOpen={() => void refresh()}>
      <div style={{ padding: 12, display: "grid", gap: 10, whiteSpace: "normal" }}>
        <span>{count == null ? t("chat.resetCredits.unknown") : t("chat.resetCredits.label", String(count))}</span>
        <span>{t("chat.resetCredits.confirm")}</span>
        {outcome && <span role="status">{t(`chat.resetCredits.${outcome}`)}</span>}
        {(error || entry?.error) && <span role="alert">{t("chat.resetCredits.error")}</span>}
        <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
          <button className="sv-popover-action" style={{ whiteSpace: "normal" }} disabled={busy || (!pending && (count == null || count <= 0))} onClick={() => void consume()}>
            {busy ? t("chat.resetCredits.busy") : pending ? t("chat.resetCredits.retry") : t("chat.resetCredits.use")}
          </button>
          <button className="sv-popover-action" style={{ whiteSpace: "normal" }} disabled={busy} onClick={() => void refresh()}>{t("chat.resetCredits.refresh")}</button>
        </div>
      </div>
    </ChipPopover>
  );
}
