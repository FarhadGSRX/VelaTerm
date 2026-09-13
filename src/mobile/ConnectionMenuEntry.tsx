import { useT } from "../i18n";

/** The native host handles this UI-only request; browsers do not expose connection management. */
export function ConnectionMenuEntry() {
  const t = useT();
  if (!(window as Window & { __VELATERM_CONNECTION_MENU__?: boolean }).__VELATERM_CONNECTION_MENU__) return null;
  return <button type="button" onClick={event => {
    event.currentTarget.closest("details")?.removeAttribute("open");
    window.location.assign("velaterm-ui://connections");
  }}>{t("mobile.connections")}</button>;
}
