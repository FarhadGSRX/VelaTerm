//! Pending UI edits survive closing Settings. Only confirmed host settings affect conversations.
import { useSyncExternalStore } from "react";
import { invoke } from "../ipc/transport";
import type { SessionKind } from "../types";
import { refreshTranslationSettings } from "./state";

type Snapshot = { pending: SessionKind[] | null; saving: boolean; error: boolean };
let snapshot: Snapshot = { pending: null, saving: false, error: false };
const listeners = new Set<() => void>();
let revision = 0;
let task: Promise<void> | null = null;
const publish = (next: Snapshot) => { snapshot = next; listeners.forEach(listener => listener()); };
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };

function flush() {
  if (task) return;
  task = (async () => {
    while (snapshot.pending) {
      const version = revision;
      const kinds = snapshot.pending;
      try {
        await invoke("translation_set_conversation_kinds", { kinds });
        if (version !== revision) continue;
        const confirmed = await refreshTranslationSettings();
        if (version !== revision) continue;
        if (!confirmed) throw new Error("translation:unavailable");
        publish({ pending: null, saving: false, error: false });
      } catch {
        if (version !== revision) continue;
        publish({ pending: kinds, saving: false, error: true });
        return;
      }
    }
  })().finally(() => {
    task = null;
    if (snapshot.pending && !snapshot.error) flush();
  });
}

export function saveConversationKinds(kinds: SessionKind[]) {
  revision++;
  publish({ pending: [...kinds], saving: true, error: false });
  flush();
}

export function retryConversationKinds() {
  if (snapshot.pending) saveConversationKinds(snapshot.pending);
}

export function useConversationKindSelection() {
  return useSyncExternalStore(subscribe, () => snapshot);
}
