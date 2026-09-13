import { t, useT, type I18nKey } from "../i18n";
import { permissionLabelKey } from "./AgentPermissionSelect";
import type { SessionPermissionState } from "../hooks/useSessionPermissionState";

function modeName(mode: string) {
  return mode === "skip" ? t("statusbar.permSkip") : t(permissionLabelKey(mode));
}

export function currentPermissionLabel(state?: SessionPermissionState) {
  if (!state) return t("permission.stateUnavailable");
  if (!state.running) return modeName(state.configured);
  return state.current ? modeName(state.current) : t("permission.currentUnknown");
}

export function pendingPermissionLabel(state?: SessionPermissionState) {
  if (!state?.pending || !state.running || state.activation === "nextStart") return null;
  return t(`permission.${state.activation}` as I18nKey) + ": " + modeName(state.pending);
}

export function PermissionStateDetails({ state, error }: { state?: SessionPermissionState; error?: string }) {
  const t = useT();
  if (state && (!state.running || (state.activation === "applied" && !state.pending))) return null;
  return <span role="status" aria-label={t("info.permission")} style={{ display: "inline-flex", flexWrap: "wrap", gap: "6px 10px", alignItems: "center", fontSize: 11 }}>
    {state ? <>
      {!state.current && state.running && <span title={t("permission.unconfirmedHint")}>
        {state.launch ? t("permission.launch", modeName(state.launch)) : t("permission.currentUnknown")}
      </span>}
      {state.pending && <span>{pendingPermissionLabel(state)}</span>}
    </> : <span title={error}>{t("permission.stateUnavailable")}</span>}
  </span>;
}
