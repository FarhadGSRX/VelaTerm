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

describe("the sticky slot under mirror mode", () => {
  it("survives a round trip through the published layout", async () => {
    const { buildMirrorLayout, sanitizeMirrorLayout } = await import("./mirrorLayout");
    const source = {
      openTabs: ["main", "doc-1"],
      liveTabs: [],
      pinnedTabs: [],
      stickyTabId: "doc-1",
      stickySize: 42,
      activeTabId: "main",
      lastActiveSessionTabId: "main",
      activeSessionId: "main",
      focusedPaneId: "p1",
      paneTrees: { main: { kind: "leaf" as const, paneId: "p1", sessionId: "main" } },
      ephemeralSessions: {},
      docTabs: {},
      browserTabs: {},
      sessions: [],
      selection: [],
      inspectTarget: null,
      leftCollapsed: false,
      rightCollapsed: false,
      inspectorTab: "files" as const,
      sidebarTreeViews: [],
      sidebarTreeTabs: [],
      primarySidebarTreeViewId: "main",
      activeSidebarTreeViewId: "main",
    };
    const round = sanitizeMirrorLayout(JSON.parse(JSON.stringify(buildMirrorLayout(source))));
    expect(round?.center.stickyTabId).toBe("doc-1");
    expect(round?.center.stickySize).toBe(42);
  });

  it("reads a stock peer's snapshot as no sticky tab, rather than rejecting it", async () => {
    const { buildMirrorLayout, sanitizeMirrorLayout } = await import("./mirrorLayout");
    const source = {
      openTabs: ["main"],
      liveTabs: [],
      pinnedTabs: [],
      activeTabId: "main",
      lastActiveSessionTabId: "main",
      activeSessionId: "main",
      focusedPaneId: "p1",
      paneTrees: { main: { kind: "leaf" as const, paneId: "p1", sessionId: "main" } },
      ephemeralSessions: {},
      docTabs: {},
      browserTabs: {},
      sessions: [],
      selection: [],
      inspectTarget: null,
      leftCollapsed: false,
      rightCollapsed: false,
      inspectorTab: "files" as const,
      sidebarTreeViews: [],
      sidebarTreeTabs: [],
      primarySidebarTreeViewId: "main",
      activeSidebarTreeViewId: "main",
    };
    // Strip the fields the way a stock v0.1.102 client would never have sent them at all.
    const wire = JSON.parse(JSON.stringify(buildMirrorLayout(source)));
    delete wire.center.stickyTabId;
    delete wire.center.stickySize;
    const round = sanitizeMirrorLayout(wire);
    // The snapshot must still be accepted -- rejecting it would break mirroring with stock clients.
    expect(round).not.toBeNull();
    expect(round?.center.stickyTabId).toBeNull();
    expect(round?.center.stickySize).toBe(35);
  });

  it("refuses a sticky tab the snapshot does not carry as open", async () => {
    const { sanitizeMirrorLayout, MIRROR_LAYOUT_VERSION } = await import("./mirrorLayout");
    const round = sanitizeMirrorLayout({
      v: MIRROR_LAYOUT_VERSION,
      center: { openTabs: ["main"], stickyTabId: "ghost-tab", paneTrees: {} },
      left: {},
      right: {},
    });
    expect(round?.center.stickyTabId).toBeNull();
  });

  it("clamps a peer's out-of-range width instead of trusting it", async () => {
    const { sanitizeMirrorLayout, MIRROR_LAYOUT_VERSION } = await import("./mirrorLayout");
    const wide = sanitizeMirrorLayout({
      v: MIRROR_LAYOUT_VERSION,
      center: { openTabs: [], stickySize: 300, paneTrees: {} },
      left: {},
      right: {},
    });
    expect(wide?.center.stickySize).toBe(85);
  });
});
