//! Global translation configuration, separate from the agent that owns the current conversation.
import { useEffect, useRef, useState } from "react";
import Select from "../components/Select";
import { kindIconEl } from "../layout/sessionViewers/sessionMeta";
import AgentSelect from "../components/AgentSelect";
import { ModelEffortFields } from "../components/LaunchFields";
import { Field, FieldStack } from "../components/Field";
import { ServiceEditor } from "./ServiceEditor";
import { useT } from "../i18n";
import { invoke } from "../ipc/transport";
import type { TranslationConfig } from "../ipc/translation";
import type { SessionKind } from "../types";
import { useTranslationSettings } from "./state";
import { useConversationKindSelection, saveConversationKinds, retryConversationKinds } from "./kindSelection";
import { navigateTranslation, translationUrl, useTranslationRoute } from "./navigation";
import "./translation.css";

export function translationError(error: unknown, t: ReturnType<typeof useT>): string {
  const raw = String(error);
  if (raw.includes("translation:structure")) return t("translation.structure");
  if (/translation:http:402\b/.test(raw)) return t("translation.paymentRequired");
  if (/translation:http:429\b/.test(raw)) return t("translation.rateLimited");
  if (/translation:(network|http|timeout)/.test(raw)) return t("translation.network");
  if (/translation:(configuration|language|url)/.test(raw)) return t("translation.invalid");
  if (raw.includes("translation:canceled")) return t("translation.canceled");
  return t("translation.unavailable");
}

export function TranslationSettingsSection() {
  const t = useT(); const { value, error, refresh } = useTranslationSettings();
  const kindSelection = useConversationKindSelection();
  const route = useTranslationRoute(); const serviceRoute = route.get("translationService");
  const [draft, setDraft] = useState<TranslationConfig | null>(null);
  const dirty = useRef(false); const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  useEffect(() => { if (value && !dirty.current) setDraft(value.config); }, [value]);
  const edit = (patch: Partial<TranslationConfig>) => { dirty.current = true; setNote(""); setDraft(prev => prev ? { ...prev, ...patch } : null); };
  const run = async (action: () => Promise<unknown>, success = "") => {
    setBusy(true); setNote("");
    try { await action(); await refresh(); setNote(success); } catch (e) { setNote(translationError(e, t)); } finally { setBusy(false); }
  };
  if (!value || !draft) return <section id="translation-settings" className="translation-section"><h3>{t("translation.title")}</h3><p role={error ? "alert" : "status"}>{error ? t("translation.unavailable") : t("common.loading")}</p>{error && <button onClick={() => void refresh()}>{t("common.retry")}</button>}</section>;
  const kinds = kindSelection.pending ?? value.config.conversationKinds;
  const selected = value.services.find(service => service.id === draft.serviceId);
  const agentSpec = value.agents.find(agent => agent.id === draft.agent);
  const ready = draft.engine === "google" || (draft.engine === "api" ? !!selected : !!agentSpec?.available);
  return <section id="translation-settings" className="translation-section">
    <h3>{t("translation.title")}</h3>
    <div className="translation-card">
      <label className="translation-toggle"><span>{t("translation.enable")}</span><input type="checkbox" checked={value.config.enabled} disabled={busy} onChange={e => void run(() => invoke("translation_enable", { enabled: e.target.checked }))} /></label>
      <p className="translation-hint">{t("translation.visibility")}</p>
      <Field as="div" label={t("translation.conversationKinds")} hint={t("translation.conversationKindsHint")}>
        <Select<SessionKind> multiple value={kinds} onChange={saveConversationKinds} width="100%" menuPortal
          options={value.conversationKinds.map(kind => ({ value: kind.id, label: kind.label, icon: <span aria-hidden="true" style={{ display: "inline-flex" }}>{kindIconEl(kind.id, 14)}</span> }))}
          ariaLabel={t("translation.conversationKinds")} placeholder={t("translation.chooseConversationKinds")}
          summary={kinds.length === value.conversationKinds.length ? t("translation.allConversationKinds") : undefined}
          selectAllLabel={t("common.selectAll")} clearLabel={t("importSessions.clearSelection")} />
      </Field>
      {kindSelection.saving && <p className="translation-hint" role="status">{t("doc.saving")}</p>}
      {kindSelection.error && <div className="translation-actions" role="alert"><span className="translation-hint">{t("translation.kindSaveFailed")}</span><button type="button" onClick={retryConversationKinds}>{t("common.retry")}</button></div>}
      {!kinds.length && <p className="translation-hint">{t("translation.noConversationKinds")}</p>}
    </div>
    <div className="translation-card">
      <FieldStack>
        <Field as="div" label={t("translation.engine")}><Select value={draft.engine} width="100%" menuPortal disabled={busy || !value.editable} onChange={engine => edit({ engine })}
          options={value.engines.map(engine => ({ value: engine, label: engine === "agent" ? t("orch.agentLabel") : t(engine === "google" ? "translation.google" : "translation.api") }))} ariaLabel={t("translation.engine")} /></Field>
        {draft.engine === "agent" ? <>
          <Field as="div" label={t("orch.agentLabel")}><AgentSelect value={draft.agent} options={value.agents} disabled={busy || !value.editable} onChange={agent => edit({ agent, model: "", effort: "" })} /></Field>
          {agentSpec && <ModelEffortFields key={agentSpec.id} spec={agentSpec} model={draft.model} effort={draft.effort}
            onChange={(model, effort) => edit({ model, effort })} disabled={busy || !value.editable} />}
        </> : draft.engine === "google" ? <p className="translation-hint">{t("translation.googleHint")}</p> : <>
          <Field as="div" label={t("translation.service")}><Select value={draft.serviceId} width="100%" menuPortal disabled={busy || !value.editable} onChange={serviceId => edit({ serviceId })}
            options={value.services.map(service => ({ value: service.id, label: service.name }))} placeholder={t("translation.newService")} ariaLabel={t("translation.service")} /></Field>
          {selected && <><p className="translation-hint">{selected.baseUrl} · {selected.model}</p><p className="translation-hint">{t(selected.hasKey ? "translation.keyConfigured" : "translation.keyNotConfigured")}</p></>}
          <div className="translation-actions">
            <button type="button" disabled={busy || !value.editable} aria-haspopup="dialog" onClick={() => navigateTranslation(translationUrl({ translationService: "new" }))}>{t("translation.newService")}</button>
            {selected && <button type="button" disabled={busy || !value.editable} aria-haspopup="dialog" onClick={() => navigateTranslation(translationUrl({ translationService: selected.id }))}>{t("translation.editService")}</button>}
          </div>
        </>}
      </FieldStack>
      {!value.editable && <p className="translation-hint">{t("translation.hostOnly")}</p>}
      <div className="translation-actions">
        <button className="translation-primary" disabled={busy || !value.editable || !ready} onClick={() => void run(async () => { await invoke("translation_save", { config: draft }); dirty.current = false; }, t("translation.saved"))}>{t("translation.saveConfig")}</button>
        <button disabled={busy || dirty.current || !ready} onClick={() => void run(() => invoke("translation_test"), t("translation.testSuccess"))}>{busy ? t("common.loading") : t("translation.test")}</button>
      </div>
      <p className="translation-hint">{t("translation.testHint")}</p>
      {note && <p role="status" className="translation-hint">{note}</p>}
    </div>
    {serviceRoute && <ServiceEditor key={serviceRoute} service={value.services.find(service => service.id === serviceRoute)} isNew={serviceRoute === "new"} editable={value.editable}
      onCancel={() => navigateTranslation(translationUrl({ translationService: null }))} formatError={error => translationError(error, t)}
      onSave={async id => { await invoke("translation_save", { config: { ...draft, engine: "api", serviceId: id } }); dirty.current = false; await refresh(); navigateTranslation(translationUrl({ translationService: null })); }}
      onDelete={async id => { await invoke("translation_service_delete", { id }); dirty.current = false; await refresh(); navigateTranslation(translationUrl({ translationService: null })); }} />}
  </section>;
}
