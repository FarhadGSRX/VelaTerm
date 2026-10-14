//! Views cache host responses and fail closed whenever their configuration cannot be read.
import { createContext, useCallback, useContext, useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { listen } from "../ipc/transport";
import { translationSettings, translationSession, saveTranslationPreferences, type TranslationSettings, type TranslationSession, type TranslationPreferences } from "../ipc/translation";
import { recoverTranslationSubmissions } from "../layout/CenterPane/session/outbox";

type SettingsSnapshot = { value: TranslationSettings | null; error: string | null };
let settingsSnapshot: SettingsSnapshot = { value: null, error: null };
const settingsListeners = new Set<() => void>();
let settingsRequest: Promise<TranslationSettings | null> | null = null;
let settingsRevision = 0;

// One host subscription and poll serve every mounted conversation; this remains a read-only cache.
export function refreshTranslationSettings(): Promise<TranslationSettings | null> {
  settingsRevision++;
  if (settingsRequest) return settingsRequest;
  settingsRequest = (async () => {
    for (;;) {
      const revision = settingsRevision;
      let next: SettingsSnapshot;
      try {
        const value = await translationSettings();
        if (!value?.config) throw new Error("translation:configuration");
        next = { value, error: null };
      } catch (e) { next = { value: null, error: String(e) }; }
      if (revision !== settingsRevision) continue;
      settingsSnapshot = next; settingsListeners.forEach(listener => listener());
      return next.value;
    }
  })().finally(() => { settingsRequest = null; });
  return settingsRequest;
}
let stopSettings: (() => void) | null = null;
function subscribeSettings(listener: () => void) {
  settingsListeners.add(listener);
  if (settingsListeners.size === 1) {
    void refreshTranslationSettings();
    const event = listen("translation://settings", () => void refreshTranslationSettings());
    const focus = () => { void refreshTranslationSettings(); };
    const timer = window.setInterval(focus, 10000);
    window.addEventListener("focus", focus);
    stopSettings = () => { clearInterval(timer); window.removeEventListener("focus", focus); void event.then(off => off()); };
  }
  return () => { settingsListeners.delete(listener); if (!settingsListeners.size) { stopSettings?.(); stopSettings = null; } };
}
export function useTranslationSettings() {
  const snapshot = useSyncExternalStore(subscribeSettings, () => settingsSnapshot);
  return { ...snapshot, refresh: refreshTranslationSettings };
}

type State = { originals: TranslationSession["messages"]; sessionId: string; settings: TranslationSettings | null; value: TranslationSession | null; refresh: (required?: boolean) => Promise<void>; save: (preferences: TranslationPreferences) => Promise<void> };
const Context = createContext<State | null>(null);
export const useConversationTranslation = () => useContext(Context);
export function useTranslationSession(sessionId: string): State {
  const { value: settings } = useTranslationSettings();
  const [value, setValue] = useState<TranslationSession | null>(null);
  const originals = useRef<{ session: string; messages: TranslationSession["messages"] }>({ session: sessionId, messages: [] });
  const request = useRef(0);
  const lifetime = useRef(0);
  const refresh = useCallback(async (required = false) => {
    const generation = lifetime.current;
    for (;;) {
      const token = ++request.current;
      try {
        const next = await translationSession(sessionId);
        if (generation !== lifetime.current) {
          if (required) throw new Error("translation:unavailable");
          return;
        }
        if (token !== request.current) {
          if (required) continue;
          return;
        }
        recoverTranslationSubmissions(sessionId, next.pending ?? []);
        originals.current = { session: sessionId, messages: next.messages };
        setValue(next);
        return;
      } catch (error) {
        if (token === request.current) setValue(null);
        if (required) throw error;
        return;
      }
    }
  }, [sessionId]);
  useEffect(() => {
    setValue(null); void refresh(); const sub = listen(`translation://session/${sessionId}`, () => void refresh());
    const global = listen("translation://settings", () => void refresh());
    const defaults = listen("translation://defaults", () => void refresh());
    const timer = window.setInterval(() => void refresh(), 10000);
    return () => { lifetime.current++; request.current++; void sub.then(off => off()); void global.then(off => off()); void defaults.then(off => off()); clearInterval(timer); };
  }, [refresh, sessionId]);
  const save = useCallback(async (preferences: TranslationPreferences) => { await saveTranslationPreferences(sessionId, preferences); await refresh(); }, [sessionId, refresh]);
  return { sessionId, originals: originals.current.session === sessionId ? originals.current.messages : [], settings, value: settings ? value : null, refresh, save };
}
export function TranslationProvider({ state, children }: { state: State; children: ReactNode }) {
  return <Context.Provider value={state}>{children}</Context.Provider>;
}
