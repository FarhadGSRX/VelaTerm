//! Coverage for the remembered knowledge-base compilation choice: patches merge field by field, null
//! clears a field, and every change persists through the shared settings block.

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

import { SETTINGS_KEY } from "./settings";
import { useTermStore } from "./termStore";

describe("knowledge-base compile preferences", () => {
  beforeEach(() => {
    // jsdom has no matchMedia, and persisting settings re-resolves the theme.
    vi.stubGlobal(
      "matchMedia",
      vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
    );
    useTermStore.setState({ memoryPrefs: {} });
    localStorage.clear();
  });

  afterEach(() => vi.unstubAllGlobals());

  it("merges patches field by field and clears fields set to null", () => {
    const set = useTermStore.getState().setMemoryPrefs;
    set({ agent: "codex", model: "gpt-5", effort: "high" });
    expect(useTermStore.getState().memoryPrefs).toEqual({ agent: "codex", model: "gpt-5", effort: "high" });

    set({ model: "gpt-5.1" });
    expect(useTermStore.getState().memoryPrefs).toEqual({ agent: "codex", model: "gpt-5.1", effort: "high" });

    set({ agent: "claude", model: null, effort: null });
    expect(useTermStore.getState().memoryPrefs).toEqual({ agent: "claude" });
  });

  it("persists the choice in the shared settings block", () => {
    useTermStore.getState().setMemoryPrefs({ agent: "codex", model: "gpt-5", effort: "high" });
    const saved = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? "{}") as { memoryPrefs?: unknown };
    expect(saved.memoryPrefs).toEqual({ agent: "codex", model: "gpt-5", effort: "high" });
  });
});
