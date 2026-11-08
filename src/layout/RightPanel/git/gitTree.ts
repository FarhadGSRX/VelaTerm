//! Turns one group's flat change list into the rows the Git panel draws: plain file rows in list
//! view, or folders with their files in tree view. Folder rows carry every file below them so a
//! folder action (stage, unstage, discard) applies to exactly what the folder shows.
//!
//! Chains of folders holding a single folder and no files collapse into one row (`src/app/ui`), as
//! most editors do, so a deep path does not cost a row per level.

import type { ChangedFile } from "../../../ipc/commands";

export type GitViewMode = "list" | "tree";

export type GitRow =
  | { kind: "file"; file: ChangedFile; depth: number }
  | { kind: "dir"; path: string; name: string; depth: number; files: ChangedFile[]; open: boolean };

interface Dir {
  name: string;
  path: string;
  dirs: Map<string, Dir>;
  files: ChangedFile[];
}

const newDir = (name: string, path: string): Dir => ({ name, path, dirs: new Map(), files: [] });

/** Files below a folder, in path order. */
function descendants(dir: Dir): ChangedFile[] {
  return [...dir.files, ...[...dir.dirs.values()].flatMap(descendants)];
}

/**
 * Build the visible rows. `collapsed` holds folder paths the user closed; their contents are
 * skipped. In list mode every file is a row at depth zero and `collapsed` is ignored.
 */
export function buildGitRows(files: ChangedFile[], mode: GitViewMode, collapsed: ReadonlySet<string>): GitRow[] {
  if (mode === "list") return files.map((file) => ({ kind: "file", file, depth: 0 }));
  const root = newDir("", "");
  for (const file of files) {
    // An untracked directory is reported as `dir/`; it is a file row of its own, not a folder.
    const trimmed = file.path.endsWith("/") ? file.path.slice(0, -1) : file.path;
    const parts = trimmed.split("/");
    parts.pop();
    let dir = root;
    for (const part of parts) {
      const path = dir.path ? `${dir.path}/${part}` : part;
      let next = dir.dirs.get(part);
      if (!next) {
        next = newDir(part, path);
        dir.dirs.set(part, next);
      }
      dir = next;
    }
    dir.files.push(file);
  }
  const rows: GitRow[] = [];
  const walk = (dir: Dir, depth: number) => {
    const folders = [...dir.dirs.values()].sort((a, b) => a.name.localeCompare(b.name));
    for (let folder of folders) {
      let name = folder.name;
      while (folder.files.length === 0 && folder.dirs.size === 1) {
        folder = [...folder.dirs.values()][0];
        name = `${name}/${folder.name}`;
      }
      const open = !collapsed.has(folder.path);
      rows.push({ kind: "dir", path: folder.path, name, depth, files: descendants(folder), open });
      if (open) walk(folder, depth + 1);
    }
    for (const file of dir.files) rows.push({ kind: "file", file, depth });
  };
  walk(root, 0);
  return rows;
}

/** Last path segment of a changed file, keeping the trailing slash of an untracked directory. */
export function baseName(path: string): string {
  const trimmed = path.endsWith("/") ? path.slice(0, -1) : path;
  const name = trimmed.slice(trimmed.lastIndexOf("/") + 1);
  return path.endsWith("/") ? `${name}/` : name;
}
