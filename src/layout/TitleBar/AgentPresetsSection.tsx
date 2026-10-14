import { useEffect, useRef, useState } from "react";
import { useT } from "../../i18n";
import { deleteAgentPreset, listAgentPresets } from "../../ipc/presets";
import { useTermStore } from "../../store/termStore";
import { PresetIcon } from "../agentPresetIcon";
import { SectionTitle } from "./settingsParts";
import "./agent-presets.css";

/** Manage saved launch presets through the existing backend commands. */
export function AgentPresetsSection() {
  const t = useT();
  const presets = useTermStore(state => state.agentPresets);
  const [loaded, setLoaded] = useState(false);
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState("");
  const [confirming, setConfirming] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const pending = useRef(false);

  useEffect(() => {
    let alive = true;
    setLoaded(false);
    setError("");
    listAgentPresets().then(list => {
      if (!alive) return;
      useTermStore.setState({ agentPresets: list });
      setLoaded(true);
    }).catch(reason => { if (alive) setError(String(reason)); });
    return () => { alive = false; };
  }, [revision]);

  const remove = async (id: string) => {
    if (pending.current || !loaded) return;
    pending.current = true;
    setDeleting(id);
    setError("");
    try {
      await deleteAgentPreset(id);
      // Update the shared display cache only after the backend confirms deletion. Other clients
      // reconcile through the existing presets://changed broadcast; session launch values stay intact.
      useTermStore.setState(state => ({ agentPresets: state.agentPresets.filter(preset => preset.id !== id) }));
      setConfirming(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      pending.current = false;
      setDeleting(null);
    }
  };

  return <section className="settings-agent-presets" aria-label={t("preset.savedTitle")}>
    <SectionTitle>{t("preset.savedTitle")}</SectionTitle>
    <p className="settings-preset-hint">{t("preset.deleteHint")}</p>
    {error && <div className="settings-preset-error" role="alert">
      <p>{error}</p>
      {!loaded && <button type="button" className="vlx-btn" onClick={() => setRevision(value => value + 1)}>
        {t("common.retry")}
      </button>}
    </div>}
    {!loaded ? !error && <p role="status">{t("common.loading")}</p> : !presets.length ? (
      <p className="settings-preset-hint" role="status">{t("preset.empty")}</p>
    ) : <div className="settings-preset-list">
      {presets.map(preset => <div className="settings-preset-row" key={preset.id}>
        <div className="settings-preset-name"><PresetIcon preset={preset} size={18} /><span>{preset.name}</span></div>
        {confirming === preset.id && <p className="settings-preset-confirm">{t("preset.deleteConfirm", preset.name)}</p>}
        <div className="settings-preset-actions">
          {confirming === preset.id ? <>
            <button type="button" className="vlx-btn" disabled={deleting !== null} onClick={() => setConfirming(null)}>
              {t("common.cancel")}
            </button>
            <button type="button" className="vlx-btn settings-preset-delete" disabled={deleting !== null}
              aria-label={t("preset.deleteNamed", preset.name)} onClick={() => void remove(preset.id)}>
              {t(deleting === preset.id ? "common.loading" : "common.delete")}
            </button>
          </> : <button type="button" className="vlx-btn settings-preset-delete" disabled={deleting !== null}
            aria-label={t("preset.deleteNamed", preset.name)} onClick={() => { setError(""); setConfirming(preset.id); }}>
            {t("common.delete")}
          </button>}
        </div>
      </div>)}
    </div>}
  </section>;
}
