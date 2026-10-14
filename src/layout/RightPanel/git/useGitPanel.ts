//! State for the right panel's Git tab: repository status, the split change list, the commit count,
//! and the mutations the panel can run.
//!
//! Git has no change notification of its own. Where the backend can watch the repository (macOS,
//! Windows) the tab refreshes on its `git://changed` events and polls only as a slow safety net;
//! elsewhere it polls while on screen. Either way it refreshes when the window regains focus and the
//! moment one of its own mutations lands, so an action never leaves a stale row on screen.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  gitChangedFiles,
  gitCommitCount,
  gitIgnoreFolder,
  gitIgnoreSuggestion,
  gitWatch,
  type ChangedFile,
} from "../../../ipc/commands";
import { onFsChanged } from "../../../ipc/events";
import { getGitStatus } from "../../../ipc/info";
import type { GitStatus } from "../../../types";

/** Poll interval while the tab is visible and the repository is not watched. */
const POLL_MS = 5000;
/** With a watch, poll this rarely as a safety net; it also renews the backend's 90-second lease. */
const WATCHED_POLL_MS = 30_000;
/** Change events within this window collapse into one refresh. */
const EVENT_DEBOUNCE_MS = 150;

/** Suggestions dismissed this app run, keyed by repository and folder. */
const dismissedSuggestions = new Set<string>();

/** The three working-tree groups the panel lists, derived from each file's index and worktree sides. */
export interface ChangeGroups {
  /** Files with something staged. */
  staged: ChangedFile[];
  conflicts: ChangedFile[];
  /** Tracked files changed in the worktree but not staged. */
  changes: ChangedFile[];
  /** Files git does not track yet. */
  untracked: ChangedFile[];
}

export interface GitPanelState {
  status: GitStatus | null;
  files: ChangedFile[] | null;
  /** The backend stopped listing at its cap; `files` holds only the first part, without line counts. */
  truncated: boolean;
  /** Folder that floods a truncated list and could be added to `.gitignore`. */
  ignoreSuggestion: string | null;
  /** Adds `ignoreSuggestion` to the repository's `.gitignore`, then refreshes. */
  ignoreFolder: () => void;
  dismissIgnoreSuggestion: () => void;
  groups: ChangeGroups;
  commitCount: number;
  isRepo: boolean;
  /** Load error for the whole tab, such as a directory that is not a repository. */
  err: string;
  /** Error from the last mutation the user triggered; cleared by the next successful one. */
  actionErr: string;
  /** True while a mutation is in flight, so the panel can disable its buttons. */
  busy: boolean;
  refresh: () => void;
  /** Explicit refreshes also retry immutable history after a previous request failure. */
  historyRefresh: number;
  clearActionError: () => void;
  /** Runs one mutation, then refreshes. Resolves to true when it succeeded. */
  run: (action: () => Promise<unknown>) => Promise<boolean>;
}

/**
 * Split the flat backend list into the panel's groups. A partly staged file legitimately appears in
 * both `staged` and `changes`: that is what git's two status columns mean, and hiding one half would
 * misreport what a commit is about to include.
 */
export function groupChanges(files: ChangedFile[] | null): ChangeGroups {
  const groups: ChangeGroups = { staged: [], changes: [], untracked: [], conflicts: [] };
  for (const f of files ?? []) {
    if (f.status === "conflicted") { groups.conflicts.push(f); continue; }
    if (f.index) groups.staged.push(f);
    if (f.worktree === "untracked") groups.untracked.push(f);
    else if (f.worktree) groups.changes.push(f);
  }
  return groups;
}

export function useGitPanel(path: string | null): GitPanelState {
  const [snapshot, setSnapshot] = useState<{
    path: string | null;
    status: GitStatus | null;
    files: ChangedFile[] | null;
    truncated: boolean;
    commitCount: number;
    err: string;
  }>({ path, status: null, files: null, truncated: false, commitCount: 0, err: "" });
  const [suggestion, setSuggestion] = useState<{ path: string; folder: string } | null>(null);
  // Dismissals live in a module set that outlasts the tab; this only re-renders after one.
  const [, setDismissTick] = useState(0);
  // Whether the suggestion was already looked up for the current flood; it costs a full status scan.
  const suggestionCheckedRef = useRef(false);
  const [actionErr, setActionErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [historyRefresh, setHistoryRefresh] = useState(0);
  const reloadRef = useRef<() => void>(() => {});
  const generationRef = useRef(0);
  const mutationRef = useRef(false);
  const refresh = useCallback(() => { setHistoryRefresh((n) => n + 1); reloadRef.current(); }, []);
  const clearActionError = useCallback(() => setActionErr(""), []);

  useEffect(() => {
    generationRef.current++;
    let cancelled = false;
    let running = false;
    let pending = false;
    let lastHead: string | null | undefined;
    let count = 0;
    // Serialized result of the last sample. An unchanged poll keeps the previous state object so
    // React skips the re-render; a long list re-rendered every few seconds kept the UI busy.
    let lastKey = "";
    suggestionCheckedRef.current = false;
    setSuggestion(null);
    setSnapshot({ path, status: null, files: null, truncated: false, commitCount: 0, err: "" });
    setActionErr("");
    mutationRef.current = false;
    setBusy(false);

    const sample = async (force = false) => {
      if (!path || cancelled) return;
      if (running) {
        if (force) pending = true;
        return;
      }
      running = true;
      try {
        const [s, f] = await Promise.allSettled([getGitStatus(path), gitChangedFiles(path)]);
        if (cancelled) return;
        if (s.status === "rejected") throw s.reason;
        if (f.status === "rejected" && s.value.isRepo) throw f.reason;
        if (s.value.isRepo && (lastHead === undefined || s.value.head !== lastHead)) {
          // Commit count is independent of worktree edits and only changes with HEAD.
          count = await gitCommitCount(path);
        } else if (!s.value.isRepo) count = 0;
        if (cancelled) return;
        lastHead = s.value.head ?? null;
        const list = f.status === "fulfilled" ? f.value : { files: [], truncated: false };
        const key = JSON.stringify([s.value, list, count]);
        const unchanged = key === lastKey;
        lastKey = key;
        setSnapshot((previous) => unchanged && previous.path === path && !previous.err ? previous
          : { path, status: s.value, files: list.files, truncated: list.truncated, commitCount: count, err: "" });
        if (!list.truncated) {
          suggestionCheckedRef.current = false;
          setSuggestion(null);
        } else if (!suggestionCheckedRef.current) {
          suggestionCheckedRef.current = true;
          void gitIgnoreSuggestion(path)
            .then((folder) => { if (!cancelled) setSuggestion(folder ? { path, folder } : null); })
            .catch(() => {});
        }
      } catch (e) {
        if (!cancelled) setSnapshot((previous) => ({ ...previous, err: String(e) }));
      } finally {
        running = false;
        if (pending && !cancelled) {
          pending = false;
          void sample();
        }
      }
    };
    reloadRef.current = () => { void sample(true); };
    const visibleSample = () => { if (!document.hidden) void sample(); };
    void sample();

    // Watched repository: refresh on change events, poll rarely. Until the lease answers, and on
    // platforms that cannot watch, keep the ordinary poll.
    let watchedRoot: string | null = null;
    let lastPoll = Date.now();
    let debounce: ReturnType<typeof setTimeout> | null = null;
    const renew = () => {
      if (!path) return;
      void gitWatch(path)
        .then((info) => { if (!cancelled) watchedRoot = info.supported ? info.root : null; })
        .catch(() => { if (!cancelled) watchedRoot = null; });
    };
    renew();
    const unlisten = onFsChanged(({ root }) => {
      if (cancelled || root !== watchedRoot || document.hidden) return;
      if (debounce) clearTimeout(debounce);
      debounce = setTimeout(() => { debounce = null; void sample(true); }, EVENT_DEBOUNCE_MS);
    });
    const timer = setInterval(() => {
      const interval = watchedRoot ? WATCHED_POLL_MS : POLL_MS;
      if (Date.now() - lastPoll < interval - 100) return;
      lastPoll = Date.now();
      if (watchedRoot) renew();
      visibleSample();
    }, POLL_MS);
    window.addEventListener("focus", visibleSample);
    document.addEventListener("visibilitychange", visibleSample);
    return () => {
      cancelled = true;
      if (debounce) clearTimeout(debounce);
      void unlisten.then((stop) => stop());
      clearInterval(timer);
      window.removeEventListener("focus", visibleSample);
      document.removeEventListener("visibilitychange", visibleSample);
      reloadRef.current = () => {};
    };
  }, [path]);

  const run = useCallback(async (action: () => Promise<unknown>) => {
    if (mutationRef.current) return false;
    mutationRef.current = true;
    const generation = generationRef.current;
    setBusy(true);
    try {
      await action();
      if (generation === generationRef.current) setActionErr("");
      return true;
    } catch (e) {
      if (generation === generationRef.current) setActionErr(String(e));
      return false;
    } finally {
      if (generation === generationRef.current) { mutationRef.current = false; setBusy(false); refresh(); }
    }
  }, [refresh]);

  const suggested = suggestion && suggestion.path === path
    && !dismissedSuggestions.has(`${suggestion.path}\n${suggestion.folder}`) ? suggestion.folder : null;

  const ignoreFolder = useCallback(() => {
    if (!path || !suggested) return;
    // Look the suggestion up again afterwards: the list may still be over the cap for another reason.
    suggestionCheckedRef.current = false;
    setSuggestion(null);
    void run(() => gitIgnoreFolder(path, suggested));
  }, [path, suggested, run]);

  const dismissIgnoreSuggestion = useCallback(() => {
    if (!path || !suggested) return;
    dismissedSuggestions.add(`${path}\n${suggested}`);
    setDismissTick((n) => n + 1);
  }, [path, suggested]);

  // Do not show an old repository during the render before the path effect resets it.
  const current = snapshot.path === path ? snapshot : null;
  const status = current?.status ?? null;
  const files = current?.files ?? null;
  const truncated = current?.truncated ?? false;
  const commitCount = current?.commitCount ?? 0;
  const err = current?.err ?? "";

  const groups = useMemo(() => groupChanges(files), [files]);

  return {
    status,
    files,
    truncated,
    ignoreSuggestion: suggested,
    ignoreFolder,
    dismissIgnoreSuggestion,
    groups,
    commitCount,
    isRepo: status?.isRepo ?? false,
    err,
    actionErr,
    busy,
    refresh,
    historyRefresh,
    clearActionError,
    run,
  };
}
