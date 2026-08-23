//! The sticky slot: one tab kept visible beside whatever tab is active, so it survives single-tab
//! session swapping. The slot holds a tab ID, so the invariant that matters is that a closed tab
//! never stays in it -- that would render an empty region with no way to clear it.

import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  createWorktree: vi.fn(),
  getSessionCwd: vi.fn().mockResolvedValue(null),
  ptyKill: vi.fn().mockResolvedValue(undefined),
  ptyWrite: vi.fn().mockResolvedValue(undefined),
  listShells: vi.fn().mockResolvedValue([]),
}));
vi.mock("../ipc/tree", () => ({
  listTree: vi.fn().mockResolvedValue({ projects: [], groups: [], sessions: [] }),
}));
vi.mock("../notify", () => ({
  notify: vi.fn(),
  getNotifyPermission: vi.fn().mockResolvedValue("granted"),
  requestNotifyPermission: vi.fn().mockResolvedValue("granted"),
  getEffectiveNotifyPermission: vi.fn().mockResolvedValue("granted"),
  requestEffectiveNotifyPermission: vi.fn().mockResolvedValue("granted"),
}));

import { useTermStore } from "./termStore";

const st = () => useTermStore.getState();

beforeEach(() => {
  useTermStore.setState({
    stickyTabId: null,
    stickySize: 35,
    openTabs: [],
    pinnedTabs: [],
    paneTrees: {},
    docTabs: {},
    browserTabs: {},
    activeTabId: null,
  });
});

describe("setStickyTab", () => {
  it("puts a tab in the slot", () => {
    st().setStickyTab("doc-1");
    expect(st().stickyTabId).toBe("doc-1");
  });

  it("toggles the same tab back out, so the menu entry is its own undo", () => {
    st().setStickyTab("doc-1");
    st().setStickyTab("doc-1");
    expect(st().stickyTabId).toBeNull();
  });

  it("replaces the occupant rather than keeping two", () => {
    st().setStickyTab("doc-1");
    st().setStickyTab("term-2");
    expect(st().stickyTabId).toBe("term-2");
  });

  it("clears the slot when passed null", () => {
    st().setStickyTab("doc-1");
    st().setStickyTab(null);
    expect(st().stickyTabId).toBeNull();
  });
});

describe("resizeSticky", () => {
  it("takes a normal width", () => {
    st().resizeSticky(50);
    expect(st().stickySize).toBe(50);
  });

  it("clamps both ends so neither side can be squeezed away", () => {
    st().resizeSticky(99);
    expect(st().stickySize).toBe(85);
    st().resizeSticky(-20);
    expect(st().stickySize).toBe(15);
  });
});

describe("closing the sticky tab", () => {
  it("empties the slot, leaving no region pinned to a tab that is gone", () => {
    useTermStore.setState({
      openTabs: ["doc-1"],
      docTabs: { "doc-1": { id: "doc-1", path: "/a.md", title: "a.md", kind: "markdown" } as never },
      stickyTabId: "doc-1",
      activeTabId: "doc-1",
    });
    st().closeTab("doc-1");
    expect(st().stickyTabId).toBeNull();
  });

  it("leaves the slot alone when a different tab closes", () => {
    useTermStore.setState({
      openTabs: ["doc-1", "doc-2"],
      docTabs: {
        "doc-1": { id: "doc-1", path: "/a.md", title: "a.md", kind: "markdown" } as never,
        "doc-2": { id: "doc-2", path: "/b.md", title: "b.md", kind: "markdown" } as never,
      },
      stickyTabId: "doc-1",
      activeTabId: "doc-2",
    });
    st().closeTab("doc-2");
    expect(st().stickyTabId).toBe("doc-1");
  });
});
