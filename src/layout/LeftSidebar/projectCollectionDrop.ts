//! Project drags change collection membership or position, leaving session ownership and directories intact.

import { isVirtualProject, type Project } from "../../types";

export const PROJECT_DRAG_MIME = "application/x-vlx-project";
export const hasProjectDrag = (types: readonly string[]) => types.includes(PROJECT_DRAG_MIME);

export type ProjectDropZone = "top" | "center" | "bottom";

export interface ProjectPlacement {
  projectId: string;
  collectionId: string | null;
  /** False when the project only moves within its current collection. */
  changesCollection: boolean;
  /** Null keeps the project's current position, as a drop into a collection does. */
  sortOrder: number | null;
}

const parentOf = (p: Project, projects: Project[]) =>
  projects.find(c => c.id === p.collectionId && isVirtualProject(c))?.id ?? null;

/** True when placing `draggedId` under `collectionId` would put a collection inside itself. */
function createsCycle(draggedId: string, collectionId: string | null, projects: Project[]): boolean {
  let ancestor = collectionId;
  const seen = new Set<string>();
  while (ancestor) {
    if (ancestor === draggedId || seen.has(ancestor)) return true;
    seen.add(ancestor);
    const parent = projects.find(p => p.id === ancestor);
    ancestor = parent ? parentOf(parent, projects) : null;
  }
  return false;
}

/** A collection receives the project; another project receives it beside itself. Reject self-drops and cycles. */
export function resolveProjectDrop(payload: { kind: string; id: string }, target: Project, projects: Project[]) {
  if (payload.kind !== "project" || payload.id === target.id) return null;
  const dragged = projects.find(p => p.id === payload.id);
  if (!dragged || !projects.some(p => p.id === target.id)) return null;
  const collectionId = isVirtualProject(target) ? target.id : parentOf(target, projects);
  if (parentOf(dragged, projects) === collectionId) return null;
  if (createsCycle(dragged.id, collectionId, projects)) return null;
  return { projectId: dragged.id, collectionId };
}

/** A sort order that lands beside `targetId` among `siblings`, which are sorted ascending. */
export function orderBetween(
  siblings: { id: string; sortOrder: number }[],
  targetId: string,
  pos: "before" | "after",
  now = Date.now(),
): number {
  const idx = siblings.findIndex((s) => s.id === targetId);
  if (idx < 0) return now;
  if (pos === "before") {
    if (idx === 0) return siblings[0].sortOrder - 1000;
    return Math.floor((siblings[idx - 1].sortOrder + siblings[idx].sortOrder) / 2);
  }
  if (idx === siblings.length - 1) return now;
  return Math.floor((siblings[idx].sortOrder + siblings[idx + 1].sortOrder) / 2);
}

/**
 * The center of a collection moves the project into it. An edge places the project above or below the target,
 * in the target's collection. Collections always list before projects, so an edge drop beside a row of the other
 * kind lands at the near end of the dragged row's own kind.
 */
export function resolveProjectPlacement(
  payload: { kind: string; id: string },
  target: Project,
  zone: ProjectDropZone,
  projects: Project[],
  now = Date.now(),
): ProjectPlacement | null {
  if (zone === "center") {
    const move = resolveProjectDrop(payload, target, projects);
    return move && { ...move, changesCollection: true, sortOrder: null };
  }
  if (payload.kind !== "project" || payload.id === target.id) return null;
  const dragged = projects.find(p => p.id === payload.id);
  if (!dragged || !projects.some(p => p.id === target.id)) return null;
  const collectionId = parentOf(target, projects);
  if (createsCycle(dragged.id, collectionId, projects)) return null;
  const siblings = projects
    .filter(p => p.id !== dragged.id && parentOf(p, projects) === collectionId
      && isVirtualProject(p) === isVirtualProject(dragged))
    .sort((a, b) => a.sortOrder - b.sortOrder);
  let sortOrder: number;
  if (siblings.some(s => s.id === target.id)) {
    sortOrder = orderBetween(siblings, target.id, zone === "top" ? "before" : "after", now);
  } else if (!isVirtualProject(dragged) && siblings.length > 0) {
    sortOrder = siblings[0].sortOrder - 1000;
  } else {
    sortOrder = now;
  }
  return {
    projectId: dragged.id,
    collectionId,
    changesCollection: parentOf(dragged, projects) !== collectionId,
    sortOrder,
  };
}

/** The members of a collection (null: the top level), collections then projects, each by name, with fresh orders. */
export function alphabeticalOrders(projects: Project[], collectionId: string | null): { id: string; sortOrder: number }[] {
  const byName = (a: Project, b: Project) =>
    a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
  const members = projects.filter(p => parentOf(p, projects) === collectionId && p.id !== collectionId);
  const collections = members.filter(isVirtualProject).sort(byName);
  const plain = members.filter(p => !isVirtualProject(p)).sort(byName);
  return [...collections, ...plain].map((p, i) => ({ id: p.id, sortOrder: (i + 1) * 1000 }));
}
