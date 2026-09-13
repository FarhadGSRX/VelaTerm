import { safeError } from "../ipc/diagnosticSafety";

/** Preserve the opener plugin's link behavior only in pages that load the application frontend. */
export function installTauriLinkHandler(openExternal: (url: string) => Promise<void>): () => void {
  // Include native remote-connection windows, which keep local opener access even with WS transport.
  if (!("__TAURI_INTERNALS__" in window)) return () => {};

  const onClick = (event: MouseEvent) => {
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.altKey) return;
    const anchor = event.composedPath().find((node): node is HTMLAnchorElement => node instanceof HTMLAnchorElement);
    if (!anchor?.href || (anchor.target !== "_blank" && !event.ctrlKey && !event.shiftKey)) return;
    let url: URL;
    try {
      url = new URL(anchor.href);
    } catch {
      return;
    }
    if (!["http:", "https:", "mailto:", "tel:"].includes(url.protocol)) return;
    event.preventDefault();
    void openExternal(url.href).catch((error: unknown) => {
      console.warn("[links] failed to open external link", safeError(error));
    });
  };
  // Bubble after React so links with application-specific actions can handle their own navigation.
  window.addEventListener("click", onClick);
  return () => window.removeEventListener("click", onClick);
}
