import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { changesUrl, readChangesRoute, useChangesRoute } from "./changesRoute";

const state = vi.hoisted(() => {
  type S = Record<string, unknown>;
  const listeners = new Set<(s: S, p: S) => void>();
  const store: { s: S } = { s: { changesCwd: null, changesPath: null, changesSide: "all", changesCommit: null, changesBase: null } };
  const set = (patch: S) => { const previous = store.s; store.s = { ...store.s, ...patch }; listeners.forEach((l) => l(store.s, previous)); };
  store.s.openChanges = (cwd: string, opts: Record<string, unknown> = {}) => set({ changesCwd: cwd, changesPath: opts.path ?? null,
    changesSide: opts.side ?? "all", changesCommit: opts.commit ?? null, changesBase: opts.commit && opts.base ? opts.base : null });
  store.s.closeChanges = () => set({ changesCwd: null, changesPath: null, changesSide: "all", changesCommit: null, changesBase: null });
  return { store, listeners };
});
vi.mock("../../store/termStore", () => {
  const useTermStore = () => null;
  useTermStore.getState = () => state.store.s;
  useTermStore.subscribe = (l: (s: Record<string, unknown>, p: Record<string, unknown>) => void) => {
    state.listeners.add(l); return () => state.listeners.delete(l);
  };
  return { useTermStore };
});

beforeEach(() => window.history.replaceState(null, "", "/?inspector=git"));
afterEach(() => (state.store.s.closeChanges as () => void)());

it("round-trips a range comparison through the URL", () => {
  const href = changesUrl("http://x/?inspector=git", { cwd: "/r", side: "all", path: "a b.ts", commit: "bbbb", base: { from: "aaaa", label: "origin/main" } });
  expect(new URL(href).searchParams.get("inspector")).toBe("git");
  expect(readChangesRoute(new URL(href).search)).toEqual({ cwd: "/r", side: "all", path: "a b.ts", commit: "bbbb", base: { from: "aaaa", label: "origin/main" } });
  expect(readChangesRoute("?changes=/r&changesSide=bogus")?.side).toBe("all");
  expect(readChangesRoute("?inspector=git")).toBeNull();
});

it("opens from the URL on load and writes the URL when the store opens or closes the dialog", () => {
  window.history.replaceState(null, "", "/?inspector=git&changes=%2Fr&changesSide=staged&changesPath=x.ts");
  renderHook(() => useChangesRoute());
  expect(state.store.s).toMatchObject({ changesCwd: "/r", changesSide: "staged", changesPath: "x.ts" });
  act(() => (state.store.s.closeChanges as () => void)());
  expect(window.location.search).toBe("?inspector=git");
  act(() => (state.store.s.openChanges as (c: string, o: object) => void)("/r", { commit: "cccc" }));
  expect(new URLSearchParams(window.location.search).get("changesCommit")).toBe("cccc");
  act(() => { window.history.replaceState(null, "", "/?inspector=git"); window.dispatchEvent(new PopStateEvent("popstate")); });
  expect(state.store.s.changesCwd).toBeNull();
});
