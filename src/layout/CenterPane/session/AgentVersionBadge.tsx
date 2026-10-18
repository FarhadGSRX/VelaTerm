import { useEffect, useState } from "react";
import { useT } from "../../../i18n";
import { agentVersion, type AgentVersion } from "../../../ipc/agentVersion";
import { onTransportReconnect } from "../../../ipc/transport";
import { useTermStore } from "../../../store/termStore";
import type { Session } from "../../../types";

/** The server owns executable selection, release sources, and version comparison. */
export function AgentVersionBadge({ session, hidden }: { session: Session; hidden: boolean }) {
  const t = useT();
  const globalPath = useTermStore(s => s.agentDefaults[session.kind]?.path);
  const startedAt = useTermStore(s => s.runtimes[session.id]?.startedAt);
  const identity = JSON.stringify([session.id, session.kind, session.agentPath, globalPath, startedAt]);
  const [result, setResult] = useState<{ identity: string; data: AgentVersion | null; failed: boolean }>();

  useEffect(() => {
    if (hidden) return;
    let alive = true;
    let pending = false;
    const refresh = async () => {
      if (pending || document.visibilityState === "hidden") return;
      pending = true;
      try {
        const data = await agentVersion(session.id);
        if (!alive) return;
        setResult({ identity, data, failed: false });
        const checked = await agentVersion(session.id, true);
        if (alive) setResult({ identity, data: checked, failed: false });
      } catch {
        if (alive) setResult(previous => ({ identity, data: previous?.identity === identity ? previous.data : null, failed: true }));
      } finally {
        pending = false;
      }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 5 * 60 * 1000);
    const visible = () => { if (document.visibilityState === "visible") void refresh(); };
    window.addEventListener("focus", visible);
    document.addEventListener("visibilitychange", visible);
    const unlisten = onTransportReconnect(() => void refresh());
    return () => {
      alive = false;
      window.clearInterval(timer);
      window.removeEventListener("focus", visible);
      document.removeEventListener("visibilitychange", visible);
      unlisten();
    };
  }, [identity, session.id, hidden]);

  const current = result?.identity === identity ? result : undefined;
  const data = current?.data;
  const failed = current?.failed || data?.checkStatus === "failed";
  const updateVersion = data?.version && data.updateAvailable ? data.latestVersion : null;
  const updateUrl = updateVersion ? data?.docsUrl : null;
  const version = data?.version ? `v${data.version}` : t(current ? "agentVersion.unavailable" : "agentVersion.loading");
  const versionTitle = data?.version
    ? t(data.running ? "agentVersion.running" : "agentVersion.installed", data.version)
    : t("agentVersion.unavailable");
  const title = [versionTitle, failed ? t("agentVersion.checkFailed") : null].filter(Boolean).join("\n");

  // The available update stays visible beside the version; without official instructions it is plain text.
  return <span className="sv-agent-version" role="status" aria-label={t("agentVersion.label")}>
    <span className="sv-agent-version-current" title={title}>{version}</span>
    {updateVersion && (updateUrl ? <a
      className="sv-agent-version-update" href={updateUrl} target="_blank" rel="noopener noreferrer"
      title={t("agentVersion.upgrade", updateVersion)}
    >{t("agentVersion.update", updateVersion)}</a> : <span className="sv-agent-version-update">{t("agentVersion.update", updateVersion)}</span>)}
  </span>;
}
