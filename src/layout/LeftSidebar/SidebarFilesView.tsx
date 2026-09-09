//! Body of a left-sidebar view whose kind is "files": the right panel's directory tree, rooted at the active
//! session's working directory. FilesTab is reused as-is; only root resolution and the fill wrapper live here.

import { FilesTab } from "../RightPanel/FilesTab";
import { useTermStore } from "../../store/termStore";

/**
 * The real file tree inside a sidebar pane.
 *
 * The root follows the active session, the same source the right panel's Files tab uses, so the two never
 * disagree about which directory the user is looking at. There is deliberately no per-view pinned path: a
 * sidebar view is a projection of current state, and pinning would make this the one pane that stops
 * tracking what the user is doing.
 */
export function SidebarFilesView() {
  const activeSessionId = useTermStore((s) => s.activeSessionId);
  const sessions = useTermStore((s) => s.sessions);
  const ephemeralSessions = useTermStore((s) => s.ephemeralSessions);
  const projects = useTermStore((s) => s.projects);

  const session = activeSessionId
    ? sessions.find((s) => s.id === activeSessionId) ?? ephemeralSessions[activeSessionId]
    : undefined;
  const project = session ? projects.find((p) => p.id === session.projectId) : undefined;

  // `.sidebar-tree-body` is a positioned block, but FilesTab expects to be a flex child that fills its parent.
  // Absolute inset-0 gives it that height without a new stylesheet rule.
  return (
    <div style={{ position: "absolute", inset: 0, display: "flex", flexDirection: "column" }}>
      <FilesTab
        rootPath={session?.cwd || project?.rootPath || null}
        rootName={project?.name ?? null}
        projectId={project?.id ?? null}
      />
    </div>
  );
}
