//! Notebook imports run outside the dialog so switching views or opening a note never cancels an
//! upload. Components subscribe to the manager instead of owning the transfer.
import { useSyncExternalStore } from "react";
import { t } from "../../i18n";
import { notebookImportAbort, notebookImportBegin, notebookImportChunk, notebookImportCommit, type ImportResult } from "../../ipc/notebook";
import { notebookError } from "./shared";

export interface NotebookImportState {
  importId: string; vaultId: string; destination: string;
  files: number; done: number; bytes: number; sent: number;
  current: string; cancelling: boolean; startedAt: number;
}
export interface NotebookImportOutcome {
  vaultId: string; destination: string; status: "completed" | "failed" | "cancelled";
  imported: number; skipped: number; error: string;
}
/** Thrown to the caller that started the import; a cancelled import is not an error to report. */
export class NotebookImportCancelled extends Error {}
/** One import per vault at a time; a second start reports this code through notebookError. */
const BUSY = "kb_import_busy";

const active = new Map<string, NotebookImportState>();
let snapshot: ReadonlyMap<string, NotebookImportState> = active;
const listeners = new Set<() => void>();
function publish() {
  snapshot = new Map(active);
  for (const notify of listeners) notify();
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
/** Imports currently running in this window, keyed by vault ID. */
export function useNotebookImports() { return useSyncExternalStore(subscribe, () => snapshot); }

/** Cancel a running import; the upload loop stops at the next chunk boundary. */
export function cancelNotebookImport(vaultId: string) {
  const state = active.get(vaultId);
  if (!state || state.cancelling) return;
  state.cancelling = true;
  publish();
}

export async function startNotebookImport(vaultId: string, destination: string, files: File[], chunkBytes: number): Promise<ImportResult> {
  if (active.has(vaultId)) throw new Error(BUSY);
  const plan = await notebookImportBegin(vaultId, destination, files.map(file => ({ path: file.webkitRelativePath || file.name, size: file.size })));
  const state: NotebookImportState = {
    importId: plan.importId, vaultId, destination,
    files: plan.indices.length, done: 0, bytes: plan.bytes, sent: 0,
    current: "", cancelling: false, startedAt: Date.now(),
  };
  active.set(vaultId, state);
  publish();
  let sent = 0;
  try {
    for (const index of plan.indices) {
      const file = files[index];
      state.current = file.webkitRelativePath || file.name;
      publish();
      for (let offset = 0; offset < file.size;) {
        if (state.cancelling) throw new NotebookImportCancelled();
        const bytes = new Uint8Array(await file.slice(offset, offset + chunkBytes).arrayBuffer());
        let binary = ""; for (let start = 0; start < bytes.length; start += 8192) binary += String.fromCharCode(...bytes.subarray(start, start + 8192));
        const result = await notebookImportChunk(vaultId, plan.importId, index, offset, btoa(binary));
        offset = result.offset; sent += bytes.length; state.sent = sent;
        publish();
      }
      state.done += 1;
      publish();
    }
    if (state.cancelling) throw new NotebookImportCancelled();
    const result = await notebookImportCommit(vaultId, plan.importId);
    settle({ vaultId, destination, status: "completed", imported: result.imported, skipped: result.skipped, error: "" });
    return result;
  } catch (error) {
    const cancelled = error instanceof NotebookImportCancelled;
    const code = cancelled ? "" : String(error).match(/kb_[a-z_]+/)?.[0] ?? "kb_io";
    await notebookImportAbort(vaultId, plan.importId, code || undefined).catch(() => {});
    settle({ vaultId, destination, status: cancelled ? "cancelled" : "failed", imported: 0, skipped: 0, error: code });
    throw error;
  } finally {
    active.delete(vaultId);
    publish();
  }
}

/**
 * Announce the outcome. In-app surfaces refresh from the window event; a system notification
 * covers the case where no knowledge base surface is mounted or the window is in the background.
 */
function settle(outcome: NotebookImportOutcome) {
  window.dispatchEvent(new CustomEvent<NotebookImportOutcome>("notebook:importFinished", { detail: outcome }));
  if (outcome.status === "cancelled") return;
  if (listeners.size && document.hasFocus()) return;
  const body = outcome.status === "completed"
    ? `${t("nb.imported")}: ${outcome.imported} · ${t("nb.skipped")}: ${outcome.skipped}`
    : notebookError(outcome.error);
  void import("../../notify")
    .then(({ notify }) => notify(null, t(outcome.status === "completed" ? "nb.importDone" : "nb.importFailed"), body, true))
    .catch(() => {});
}
