import { useState } from "react";
import { useT } from "../../i18n";
import { setCompletionMode, useCompletionConfig } from "../../terminal/completion/config";
import { Field, Seg } from "./settingsParts";

export function CompletionModeField() {
  const t = useT();
  const config = useCompletionConfig();
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);
  // Hide the setting on a host without the shell integration (Windows for now) instead of leaving a switch that
  // changes nothing, whatever value it holds.
  if (config && !config.available) return null;
  return <>
    <Field label={t("settings.completionMode")}>
      {config ? <Seg value={config.mode}
        options={config.modes.map(mode => [mode, t(mode === "auto" ? "settings.completionAuto" : mode === "tab" ? "settings.completionTab" : "settings.completionOff")])}
        disabledOptions={saving ? config.modes : []}
        onChange={mode => {
          setSaving(true); setFailed(false);
          void setCompletionMode(mode).catch(() => setFailed(true)).finally(() => setSaving(false));
        }} /> : <span role="status">{t("settings.completionUnavailable")}</span>}
    </Field>
    <p style={{ fontSize: 11, color: "var(--text-dim)", margin: "6px 0 12px" }}>
      {failed ? t("settings.completionUnavailable") : t("settings.completionHint")}
    </p>
  </>;
}
