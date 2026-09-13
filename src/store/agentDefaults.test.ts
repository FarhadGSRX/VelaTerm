//! Coverage for per-agent default persistence. The view choice lives in the same object as launch args,
//! permission mode, and executable path, and `setAgentDefault` rebuilds that object field by field; these
//! tests pin that the view survives edits to the other fields and that an entry holding only a view is kept.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  createWorktree: vi.fn().mockRejectedValue(new Error("no repo")),
  getSessionCwd: vi.fn().mockResolvedValue(null),
  ptyKill: vi.fn().mockResolvedValue(undefined),
  ptyWrite: vi.fn().mockResolvedValue(undefined),
  listShells: vi.fn().mockResolvedValue([]),
}));
vi.mock("../ipc/tree", () => ({
  listTree: vi.fn().mockResolvedValue({ projects: [], groups: [], sessions: [] }),
  setCollapsed: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../notify", () => ({
  notify: vi.fn(),
  getNotifyPermission: vi.fn().mockResolvedValue("granted"),
  requestNotifyPermission: vi.fn().mockResolvedValue("granted"),
  getEffectiveNotifyPermission: vi.fn().mockResolvedValue("granted"),
  requestEffectiveNotifyPermission: vi.fn().mockResolvedValue("granted"),
}));

import { useTermStore } from "./termStore";

describe("per-agent default view persistence", () => {
  beforeEach(() => {
    // jsdom has no matchMedia, and persisting settings re-resolves the theme.
    vi.stubGlobal(
      "matchMedia",
      vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
    );
    useTermStore.setState({ agentDefaults: {} });
    localStorage.clear();
  });

  afterEach(() => vi.unstubAllGlobals());

  it("keeps a saved view when other defaults change", () => {
    const set = useTermStore.getState().setAgentDefault;
    set("omp", { engine: "chat" });
    expect(useTermStore.getState().agentDefaults.omp).toEqual({ engine: "chat" });

    set("omp", { args: "--model opus" });
    expect(useTermStore.getState().agentDefaults.omp).toEqual({
      engine: "chat",
      args: "--model opus",
    });

    // Clearing the arguments must not take the view with them.
    set("omp", { args: "" });
    expect(useTermStore.getState().agentDefaults.omp).toEqual({ engine: "chat" });
  });
});
