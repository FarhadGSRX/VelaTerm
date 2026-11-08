//! Commit message drafts, kept per repository so switching tabs, repositories, or restarting the app
//! does not throw away a message that was not committed yet. Only the text is kept: an amend toggle
//! left on from an earlier visit would make the next commit rewrite history unexpectedly.

const PREFIX = "vlx.git.commitDraft:";

/** Draft message for a repository, or an empty string. */
export function readCommitDraft(repo: string): string {
  try {
    return localStorage.getItem(PREFIX + repo) ?? "";
  } catch {
    return "";
  }
}

/** Stores the draft; an empty message removes it. */
export function writeCommitDraft(repo: string, message: string): void {
  try {
    if (message.trim()) localStorage.setItem(PREFIX + repo, message);
    else localStorage.removeItem(PREFIX + repo);
  } catch {
    // Storage can be unavailable (private browsing, quota); the draft then lives only in memory.
  }
}
