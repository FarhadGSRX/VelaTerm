import type { ChangedFile, GitChangeSide } from "../../ipc/commands";

/** Project one backend file into the comparison the user selected. */
export function filesForSide(files: ChangedFile[], side: GitChangeSide): ChangedFile[] {
  return files.filter((file) => {
    if (side === "all") return true;
    if (side === "conflicted") return file.status === "conflicted";
    if (file.status === "conflicted") return false;
    if (side === "staged") return Boolean(file.index);
    if (side === "untracked") return file.worktree === "untracked";
    return Boolean(file.worktree) && file.worktree !== "untracked";
  }).map((file) => side === "all" || side === "conflicted" ? file : {
    ...file,
    status: side === "staged" ? file.index : file.worktree,
    originalPath: side === "staged" || file.worktree === "renamed" ? file.originalPath : null,
    additions: side === "staged" ? file.stagedAdditions : file.unstagedAdditions,
    deletions: side === "staged" ? file.stagedDeletions : file.unstagedDeletions,
    binary: side === "staged" ? file.stagedBinary : file.unstagedBinary,
  });
}
