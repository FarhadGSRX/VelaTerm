//! Files-tree reconciliation preserves local expansion and lazy children while replacing backend facts.

import type { DirEntry } from "../../ipc/info";

export interface FileNodeT {
  name: string;
  path: string;
  isDir: boolean;
  isHidden: boolean;
  badge?: string | null;
  open?: boolean;
  loaded?: boolean;
  loading?: boolean;
  children?: FileNodeT[];
}

export function toNode(entry: DirEntry, parentPath: string): FileNodeT {
  return {
    name: entry.name,
    path: `${parentPath.replace(/[\\/]$/, "")}/${entry.name}`,
    isDir: entry.isDir,
    isHidden: entry.isHidden,
    badge: entry.gitBadge,
    open: false,
    loaded: false,
  };
}

export function reconcileChildren(node: FileNodeT, entries: DirEntry[]): FileNodeT {
  const previous = new Map(node.children?.map((child) => [child.path, child]));
  const children = entries.map((entry) => {
    const fresh = toNode(entry, node.path);
    const old = previous.get(fresh.path);
    return old && old.isDir === fresh.isDir ? {
      ...old, name: fresh.name, isHidden: fresh.isHidden, badge: fresh.badge,
    } : fresh;
  });
  return { ...node, children, loaded: true, loading: false };
}

export function updateFileNode(root: FileNodeT, path: string, update: (node: FileNodeT) => FileNodeT): FileNodeT {
  if (root.path === path) return update(root);
  if (!root.children) return root;
  const children = root.children.map((child) => updateFileNode(child, path, update));
  return children.some((child, index) => child !== root.children![index]) ? { ...root, children } : root;
}

/** Only expanded, already loaded directories need background queries. */
export function visibleDirectories(root: FileNodeT): string[] {
  if (!root.isDir || !root.open) return [];
  return [ ...(root.loaded ? [root.path] : []), ...(root.children ?? []).flatMap(visibleDirectories) ];
}

/**
 * Directories to re-read after a change event: the shown ones among `dirs` (relative to the root,
 * `""` being the root), or every shown one when the event could not list them or touched git's own
 * files, which moves the Git badges of any directory.
 */
export function directoriesToRefresh(root: FileNodeT, dirs: string[] | null, git: boolean): string[] {
  const shown = visibleDirectories(root);
  if (dirs === null || git) return shown;
  const base = root.path.replace(/[\\/]$/, "");
  const wanted = new Set(dirs.map((dir) => (dir ? `${base}/${dir}` : root.path)));
  return shown.filter((path) => wanted.has(path));
}
