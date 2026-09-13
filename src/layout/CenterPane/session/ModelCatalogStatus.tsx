import { useEffect, useRef, useState } from "react";
import { useT } from "../../../i18n";
import { invoke, listen } from "../../../ipc/transport";

type CatalogStatus = {
  source: "website" | "cache" | "bundled";
  revision: number | null;
  checkedAt: number | null;
  error: string | null;
  refreshing: boolean;
};

/** The backend owns synchronization; opening the menu reads its status without starting a download. */
export function ModelCatalogStatus({ onChanged }: { onChanged: () => void }) {
  const t = useT();
  const [status, setStatus] = useState<CatalogStatus | null>(null);
  const [failed, setFailed] = useState(false);
  const [pending, setPending] = useState(false);
  const changed = useRef(onChanged);
  changed.current = onChanged;
  useEffect(() => {
    let disposed = false;
    let previous: string | undefined;
    const receive = (next: CatalogStatus) => {
      if (disposed || !next) return;
      setStatus(next);
      setFailed(false);
      const key = `${next.revision}:${next.checkedAt}:${next.source}`;
      if (key !== previous) { previous = key; changed.current(); }
    };
    const read = () => { void invoke<CatalogStatus>("model_catalog_status").then(receive).catch(() => { if (!disposed) setFailed(true); }); };
    const subscription = listen<CatalogStatus>("model-catalog://changed", receive);
    void subscription.then(read).catch(() => { if (!disposed) setFailed(true); });
    read();
    const timer = setInterval(read, 30_000);
    return () => { disposed = true; clearInterval(timer); void subscription.then(stop => stop()).catch(() => {}); };
  }, []);
  const refresh = async () => {
    setPending(true);
    try {
      const next = await invoke<CatalogStatus>("model_catalog_refresh");
      setStatus(next); setFailed(false); changed.current();
    } catch { setFailed(true); }
    finally { setPending(false); }
  };
  return <div className="sv-model-catalog-status">
    <div role="status">
      {status ? t(status.source === "website" ? "chat.catalogWebsite" : status.source === "cache" ? "chat.catalogCache" : "chat.catalogBundled") : t("common.loading")}
      {status?.revision != null && <span> · v{status.revision}</span>}
      {status?.checkedAt && <div>{t("chat.catalogChecked", new Date(status.checkedAt * 1000).toLocaleString())}</div>}
      {(failed || status?.error) && <div className="sv-model-catalog-error">{t("chat.catalogFailed")}</div>}
    </div>
    <button type="button" className="sv-model-catalog-refresh" disabled={pending || status?.refreshing} onClick={() => void refresh()}>
      {pending || status?.refreshing ? t("common.loading") : t("chat.catalogRefresh")}
    </button>
  </div>;
}
