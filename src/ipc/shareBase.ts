/**
 * Public share surface path prefix.
 *
 * Shared conversations are served from the host through the account relay under `/r/<grantId>/`, so every
 * runtime URL this bundle builds — HTTP fetches and the WebSocket endpoint — must stay under that prefix.
 * A normal LAN or loopback visit sees an empty prefix and keeps today's root-relative behavior.
 */
const matched = /^\/r\/[0-9a-fA-F-]+(?=\/|$)/.exec(location.pathname);

/** Prefix for runtime URLs, for example `/r/<grantId>`; empty outside the share surface. */
export const shareBasePath = matched ? matched[0] : "";

/** Whether this page was loaded from the public share surface. */
export const isShareSurface = shareBasePath.length > 0;

/** Prefix a root-relative path with the share base, leaving absolute URLs untouched. */
export function apiUrl(path: string): string {
  return path.startsWith("/") ? `${shareBasePath}${path}` : path;
}
