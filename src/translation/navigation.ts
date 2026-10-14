//! Recoverable settings and conversation-language surfaces; URLs contain no credentials or drafts.
import { useEffect, useState } from "react";
export function translationUrl(values: Record<string, string | null>) {
  const url = new URL(window.location.href);
  for (const [key, value] of Object.entries(values)) { if (value === null) url.searchParams.delete(key); else url.searchParams.set(key, value); }
  return url.href;
}
export function navigateTranslation(href: string, replace = false) {
  if (href === window.location.href) return;
  window.history[replace ? "replaceState" : "pushState"](null, "", href);
  window.dispatchEvent(new PopStateEvent("popstate"));
}
export function useTranslationRoute() {
  const [query, setQuery] = useState(() => window.location.search);
  useEffect(() => { const update = () => setQuery(window.location.search); window.addEventListener("popstate", update); return () => window.removeEventListener("popstate", update); }, []);
  return new URLSearchParams(query);
}
export function settingsTranslationUrl() { return translationUrl({ settings: "translation", settingsSection: null, settingsTranslation: "1", translationService: null, translation: null, translationSession: null }); }
export function followTranslationLink(event: React.MouseEvent<HTMLAnchorElement>) {
  if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  event.preventDefault(); navigateTranslation(event.currentTarget.href);
}
