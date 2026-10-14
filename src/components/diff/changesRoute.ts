//! URL state for the changes dialog, so a diff survives a reload, can be bookmarked or shared, and
//! closes and reopens with the browser's back and forward.
//!
//! The store stays the source of truth while the app runs. Opening the dialog pushes a history
//! entry, choosing another file replaces it, and closing pushes the URL without the parameters.
//! On load and on back/forward the URL is read back into the store.

import { useEffect } from "react";
import type { GitChangeSide } from "../../ipc/commands";
import { memoryNavigate } from "../../layout/Memory/navigation";
import { useTermStore } from "../../store/termStore";

const KEYS = ["changes", "changesSide", "changesPath", "changesCommit", "changesBase", "changesBaseLabel"] as const;
const SIDES: GitChangeSide[] = ["all", "staged", "unstaged", "untracked", "conflicted"];

interface Route {
  cwd: string;
  side: GitChangeSide;
  path: string | null;
  commit: string | null;
  base: { from: string; label: string } | null;
}

export function readChangesRoute(search: string): Route | null {
  const params = new URLSearchParams(search);
  const cwd = params.get("changes");
  if (!cwd) return null;
  const side = params.get("changesSide") as GitChangeSide | null;
  const commit = params.get("changesCommit");
  const from = params.get("changesBase");
  return {
    cwd,
    side: side && SIDES.includes(side) ? side : "all",
    path: params.get("changesPath"),
    commit,
    base: commit && from ? { from, label: params.get("changesBaseLabel") ?? from.slice(0, 12) } : null,
  };
}

export function changesUrl(href: string, route: Route | null): string {
  const url = new URL(href);
  for (const key of KEYS) url.searchParams.delete(key);
  if (route) {
    url.searchParams.set("changes", route.cwd);
    if (route.side !== "all") url.searchParams.set("changesSide", route.side);
    if (route.path) url.searchParams.set("changesPath", route.path);
    if (route.commit) url.searchParams.set("changesCommit", route.commit);
    if (route.base) {
      url.searchParams.set("changesBase", route.base.from);
      url.searchParams.set("changesBaseLabel", route.base.label);
    }
  }
  return url.href;
}

function storeRoute(): Route | null {
  const s = useTermStore.getState();
  return s.changesCwd
    ? { cwd: s.changesCwd, side: s.changesSide ?? "all", path: s.changesPath, commit: s.changesCommit, base: s.changesBase }
    : null;
}

const same = (a: Route | null, b: Route | null) => JSON.stringify(a) === JSON.stringify(b);

/** Point the URL at the file shown in the open dialog, without adding a history entry. */
export function replaceChangesPath(path: string | null): void {
  const current = readChangesRoute(window.location.search);
  if (!current || current.path === path) return;
  window.history.replaceState(window.history.state, "", changesUrl(window.location.href, { ...current, path }));
}

/** Keep the dialog and the URL in step. Mount once, where the dialog is mounted. */
export function useChangesRoute(): void {
  useEffect(() => {
    const apply = () => {
      const route = readChangesRoute(window.location.search);
      if (same(route, storeRoute())) return;
      const store = useTermStore.getState();
      if (!route) store.closeChanges();
      else store.openChanges(route.cwd, {
        path: route.path ?? undefined, side: route.side, commit: route.commit ?? undefined, base: route.base ?? undefined,
      });
    };
    apply();
    window.addEventListener("popstate", apply);
    const unsubscribe = useTermStore.subscribe((state, previous) => {
      if (state.changesCwd === previous.changesCwd && state.changesCommit === previous.changesCommit
        && state.changesSide === previous.changesSide && state.changesBase === previous.changesBase
        && state.changesPath === previous.changesPath) return;
      const next = storeRoute();
      const shown = readChangesRoute(window.location.search);
      if (same(next, shown)) return;
      // Picking a file inside an open dialog is not a new place to go back to.
      const sameDialog = next && shown && next.cwd === shown.cwd && next.commit === shown.commit
        && next.side === shown.side && JSON.stringify(next.base) === JSON.stringify(shown.base);
      // Forced: opening or closing this dialog never discards a draft, so the unsaved-draft guard
      // has nothing to protect.
      memoryNavigate(changesUrl(window.location.href, next), Boolean(sameDialog), true);
    });
    return () => {
      window.removeEventListener("popstate", apply);
      unsubscribe();
    };
  }, []);
}
