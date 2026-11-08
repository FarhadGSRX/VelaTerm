//! Service drafts and credentials stay local to the dialog; keys are loaded only when the local service editor opens.
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { FormModal, type FieldDef } from "../components/FormModal";
import { PasswordField } from "../components/PasswordField";
import { CopyBtn } from "../layout/sessionMenuShared";
import { useT } from "../i18n";
import { invoke } from "../ipc/transport";
import type { TranslationService } from "../ipc/translation";

export function ServiceEditor({ service, isNew, editable, onSave, onDelete, onCancel, formatError }: {
  service?: TranslationService;
  isNew: boolean;
  editable: boolean;
  onSave: (id: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onCancel: () => void;
  formatError: (error: unknown) => string;
}) {
  const t = useT();
  const [values, setValues] = useState({ name: service?.name ?? "", baseUrl: service?.baseUrl ?? "", model: service?.model ?? "", apiKey: "" } as Record<string, string>);
  const [deleting, setDeleting] = useState(false);
  const [loaded, setLoaded] = useState(!service?.hasKey);
  const [loadError, setLoadError] = useState(false);
  const [loadAttempt, setLoadAttempt] = useState(0);
  useEffect(() => {
    if (!editable || !service?.hasKey) return;
    let canceled = false;
    setLoadError(false);
    invoke<string>("translation_service_key", { id: service.id }).then(apiKey => {
      if (canceled) return;
      setValues(values => ({ ...values, apiKey })); setLoaded(true);
    }).catch(() => { if (!canceled) setLoadError(true); });
    return () => { canceled = true; };
  }, [editable, service?.id, service?.hasKey, loadAttempt]);
  // Retain the new service ID if saving its engine selection fails, so a retry cannot create duplicates.
  const savedId = useRef(service?.id);
  const title = t(isNew ? "translation.newService" : "translation.editService");
  if (!editable || (!isNew && !service)) return createPortal(<FormModal title={title}
    description={t(editable ? "translation.unavailable" : "translation.hostOnly")} fields={[]} submitLabel={t("common.close")}
    onCancel={onCancel} onSubmit={onCancel} />, document.body);
  if (!loaded) return createPortal(<FormModal key="loading" title={title} width={460} fields={[]}
    description={loadError ? t("translation.unavailable") : t("common.loading")}
    submitLabel={loadError ? t("common.retry") : t("common.loading")} submitDisabled={!loadError}
    onCancel={onCancel} onSubmit={() => setLoadAttempt(attempt => attempt + 1)} />, document.body);
  if (deleting && service) return createPortal(<FormModal key="delete" title={`${t("common.delete")}: ${service.name}`}
    description={t("translation.deleteServiceConfirm")} fields={[]} submitLabel={t("common.delete")}
    onCancel={() => setDeleting(false)} onSubmit={() => onDelete(service.id)} formatSubmitError={formatError} />, document.body);
  const fields: FieldDef[] = [
    { key: "name", label: t("translation.name"), required: true },
    { key: "protocol", label: t("translation.protocol"), render: () => <span className="translation-hint">{t("translation.openai")}</span> },
    { key: "baseUrl", label: t("gitea.baseUrl"), required: true, placeholder: "https://api.example.com/v1" },
    { key: "model", label: t("spawn.modelLabel"), required: true },
    { key: "apiKey", label: t("translation.apiKey"), render: (apiKey, onChange, context) =>
      <PasswordField ariaLabel={t("translation.apiKey")} value={apiKey} onChange={onChange} disabled={context.disabled}
        showLabel={t("translation.showKey")} hideLabel={t("translation.hideKey")}
        actions={<CopyBtn value={apiKey} disabled={context.disabled || !apiKey} label={t("translation.copyKey")} copiedLabel={t("translation.keyCopied")} tooltip />} /> },
  ];
  return createPortal(<FormModal key="edit" title={title} width={460} fields={fields} initial={values} onValuesChange={setValues}
    submitLabel={t("common.save")} submittingLabel={t("doc.saving")} onCancel={onCancel} formatSubmitError={formatError}
    secondaryAction={service ? { label: t("common.delete"), danger: true, onClick: () => setDeleting(true) } : undefined}
    onSubmit={async draft => {
      const id = await invoke<string>("translation_service_save", { service: { id: savedId.current, name: draft.name, baseUrl: draft.baseUrl,
        model: draft.model, apiKey: draft.apiKey, clearKey: !draft.apiKey } });
      savedId.current = id;
      await onSave(id);
    }} />, document.body);
}
