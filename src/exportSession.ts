//! Session-context export flow shared by the sidebar context menu and the knowledge-base Collections view:
//! - Desktop: choose a path in the system Save As dialog, then let the backend generate and write the file.
//! - Browser: generate the file on the backend and download it through an authorized HTTP ticket.
//! Filenames follow the global `title_YYYYMMDD_HHmm.md` convention. Failures produce a non-disruptive system notification.

import { t } from "./i18n";
import { chatExportContext } from "./ipc/chat";
import { platform } from "./platform";
import { notify } from "./notify";
import type { Session } from "./types";

export interface ExportProgress {name: string; controller: AbortController;}
let progress: readonly ExportProgress[]=[];
const progressListeners=new Set<()=>void>();
export const exportProgressSnapshot=()=>progress;
export function subscribeExportProgress(listener:()=>void) {progressListeners.add(listener);return ()=>{progressListeners.delete(listener);};}
function updateProgress(next:readonly ExportProgress[]) {progress=next;for (const listener of progressListeners) listener();}

/** Whether context can be exported: a local agent session with a parseable transcript (Claude/Codex with a captured ID). */
export function canExportContext(s: Pick<Session, "kind" | "agentSessionId">): boolean {
  return (s.kind === "claude" || s.kind === "codex") && !!s.agentSessionId;
}

/** Current local time formatted as `YYYYMMDD_HHmm` for filenames and `YYYY-MM-DD HH:mm` for document headers. */
function nowParts(): { stamp: string; human: string } {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  const ymd = `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}`;
  const hm = `${p(d.getHours())}${p(d.getMinutes())}`;
  return {
    stamp: `${ymd}_${hm}`,
    human: `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`,
  };
}

/** Convert a session name to a safe filename by removing path separators and platform-reserved characters. */
function safeName(name: string): string {
  const s = name.replace(/[\\/:*?"<>|\r\n]/g, "_").trim();
  return s || t("common.session");
}

/**
 * Export a session's complete context as a Markdown file. Return silently if the user cancels the Save dialog;
 * report failures such as a deleted transcript through a system notification.
 */
export async function exportSessionToFile(
  session: Pick<Session, "id" | "name">,
): Promise<void> {
  const { stamp, human } = nowParts();
  const fileName = `${safeName(session.name)}_${t("export.contextSuffix")}_${stamp}.md`;
  let controller:AbortController | undefined;
  try {
    let destination:string | undefined;
    if (platform.env.isTauri || platform.env.isElectron) {
      const dest = await platform.dialog.saveFile({
        defaultPath: fileName,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (!dest) return; // User canceled.
      destination=dest;
    }
    controller=new AbortController();updateProgress([...progress,{name:session.name,controller}]);
    const url = await chatExportContext(session.id, human, destination, controller.signal);
    if (destination) return;
    if (!url) throw new Error(t("chat.history.unavailable"));
    const a = document.createElement("a");
    a.href = url;
    a.download = fileName;
    document.body.appendChild(a);
    a.click();
    a.remove();
  } catch (e) {
    if (!controller?.signal.aborted) void notify(null, t("export.failedTitle"), String(e));
  } finally {
    if (controller) updateProgress(progress.filter(item=>item.controller!==controller));
  }
}
