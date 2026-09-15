import { t, useT, type I18nKey } from "../i18n";
import { permissionLabelKey } from "./AgentPermissionSelect";
import type { SessionPermissionState } from "../hooks/useSessionPermissionState";

function modeName(mode: string) {
  return mode === "skip" ? t("statusbar.permSkip") : t(permissionLabelKey(mode));
}

// The launch arguments already carry the permission, so the launch mode stands in until the agent confirms one.
export function effectivePermission(state?: SessionPermissionState) {
  return state?.running ? state.current ?? state.launch : null;
}

export function currentPermissionLabel(state?: SessionPermissionState) {
  if (!state) return t("permission.stateUnavailable");
  if (!state.running) return modeName(state.configured);
  const effective = effectivePermission(state);
  return effective ? modeName(effective) : t("permission.currentUnknown");
}

export function pendingPermissionLabel(state?: SessionPermissionState) {
  if (!state?.pending || !state.running || state.activation === "nextStart") return null;
  return t(`permission.${state.activation}` as I18nKey) + ": " + modeName(state.pending);
}

export function PermissionStateDetails({ state, error }: { state?: SessionPermissionState; error?: string }) {
  const t = useT();
  const unknown = !!state?.running && !effectivePermission(state);
  if (state && (!state.running || (!unknown && !state.pending))) return null;
  return <span role="status" aria-label={t("info.permission")} style={{ display: "inline-flex", flexWrap: "wrap", gap: "6px 10px", alignItems: "center", fontSize: 11 }}>
    {state ? <>
      {unknown && <span>{t("permission.currentUnknown")}</span>}
      {state.pending && <span>{pendingPermissionLabel(state)}</span>}
    </> : <span title={error}>{t("permission.stateUnavailable")}</span>}
  </span>;
}
