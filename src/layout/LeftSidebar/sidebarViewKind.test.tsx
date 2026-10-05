//! Coverage for sidebar view kinds: a view renders either the session tree or the real file tree, the switcher
//! moves it between the two, the choice survives a restart as a V3 payload, and payloads written before view
//! kinds existed come back as session trees.

import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/commands", () => ({
  createWorktree: vi.fn(),
  getSessionCwd: vi.fn().mockResolvedValue(null),
  ptyKill: vi.fn().mockResolvedValue(undefined),
  ptyWrite: vi.fn().mockResolvedValue(undefined),
  listShells: vi.fn().mockResolvedValue([]),
}));
vi.mock("../../ipc/tree", () => ({
  listTree: vi.fn().mockResolvedValue({ projects: [], groups: [], sessions: [] }),
  setCollapsed: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../../notify", () => ({
  notify: vi.fn(),
  getNotifyPermission: vi.fn().mockResolvedValue("granted"),
  requestNotifyPermission: vi.fn().mockResolvedValue("granted"),
  getEffectiveNotifyPermission: vi.fn().mockResolvedValue("granted"),
  requestEffectiveNotifyPermission: vi.fn().mockResolvedValue("granted"),
}));

// Translate to the raw key so buttons are addressable by name, and stub the leaf components: this exercises
// which body a pane picks, not what either body draws.
vi.mock("../../i18n", () => ({
  useT: () => (key: string) => key,
  t: (key: string) => key,
}));
vi.mock("../../components/Icons", () => ({
  default: new Proxy({}, { get: () => () => null }),
}));
vi.mock("./ProjectTree", () => ({
  ProjectTree: () => <div data-testid="project-tree" />,
}));
vi.mock("../RightPanel/FilesTab", () => ({
  FilesTab: ({ rootPath }: { rootPath: string | null }) => (
    <div data-testid="files-tab" data-root={rootPath ?? ""} />
  ),
}));
vi.mock("./ArchivePanel", () => ({ ArchivePanel: () => null }));
vi.mock("../GlobalSearch/GlobalSearch", () => ({ GlobalSearch: () => null }));
vi.mock("../sessionMenu", () => ({
  useSessionMenu: () => ({
    newSessionItems: vi.fn(() => []),
    buildSessionItems: vi.fn(() => []),
    buildScratchItems: vi.fn(() => []),
    buildMoveToMany: vi.fn(() => null),
    buildGitItems: vi.fn(() => []),
    buildMarkItem: vi.fn(() => ({ label: "mark.menu", submenu: [] })),
    openDialog: vi.fn(),
    dialogs: null,
  }),
}));

import { LeftSidebar } from "./LeftSidebar";
import { useTermStore, type SidebarTreeView } from "../../store/termStore";
import type { Project, Session } from "../../types";

const VIEWS_KEY = "vlx-sidebar-tree-views";

const view = (id: string, kind: SidebarTreeView["kind"]): SidebarTreeView => ({
  id,
  name: id,
  kind,
  treeFilter: "",
  statusFilter: null,
  statusFilterIds: null,
  markFilter: null,
  collapsedOverrides: null,
});

const project: Project = {
  id: "p1",
  name: "Proj",
  rootPath: "/tmp/proj",
  collapsed: false,
  sortOrder: 0,
  createdAt: 0,
};

const session: Session = {
  id: "s1",
  projectId: "p1",
  name: "Sess",
  kind: "terminal",
  cwd: "/tmp/proj/sub",
  collapsed: false,
  sortOrder: 0,
  createdAt: 0,
};

/** Put the store into a single-pane sidebar showing `kind`, with one active session under one project. */
function mount(kind: SidebarTreeView["kind"]) {
  useTermStore.setState({
    projects: [project],
    groups: [],
    sessions: [session],
    ephemeralSessions: {},
    activeSessionId: session.id,
    sidebarTreeViews: [view("main", kind)],
    primarySidebarTreeViewId: "main",
    activeSidebarTreeViewId: "main",
  });
  useTermStore.setState({
    sidebarTreeTabs: [{
      id: "tab-1",
      root: { kind: "leaf", paneId: "pane-1", viewId: "main" },
      activeViewId: "main",
    }],
  });
  return render(<LeftSidebar />);
}

beforeEach(() => {
  localStorage.removeItem(VIEWS_KEY);
});

describe("sidebar view kinds", () => {
  it("renders the session tree and its search controls for a sessions view", () => {
    mount("sessions");

    expect(screen.getByTestId("project-tree")).toBeTruthy();
    expect(screen.queryByTestId("files-tab")).toBeNull();
    expect(screen.getByPlaceholderText("tree.searchPlaceholder")).toBeTruthy();
  });

  it("renders the file tree rooted at the active session's cwd for a files view", () => {
    mount("files");

    expect(screen.queryByTestId("project-tree")).toBeNull();
    expect(screen.getByTestId("files-tab").getAttribute("data-root")).toBe("/tmp/proj/sub");
    // The session search box and status filter would do nothing over a file tree, so they are not offered.
    expect(screen.queryByPlaceholderText("tree.searchPlaceholder")).toBeNull();
    expect(screen.queryByRole("button", { name: "tree.filterStatus" })).toBeNull();
  });

  it("switches an existing view between session tree and file tree", () => {
    mount("sessions");

    fireEvent.click(screen.getByRole("button", { name: "tree.viewKind" }));
    expect(useTermStore.getState().sidebarTreeViews[0].kind).toBe("files");
    expect(screen.getByTestId("files-tab")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "tree.viewKind" }));
    expect(useTermStore.getState().sidebarTreeViews[0].kind).toBe("sessions");
    expect(screen.getByTestId("project-tree")).toBeTruthy();
  });

  it("persists the chosen kind as a version 3 payload", () => {
    vi.useFakeTimers();
    try {
      mount("sessions");
      useTermStore.getState().setSidebarTreeViewKind("main", "files");
      vi.advanceTimersByTime(250);

      const saved = JSON.parse(localStorage.getItem(VIEWS_KEY) ?? "{}") as {
        version?: number;
        views?: { id: string; kind?: string }[];
      };
      expect(saved.version).toBe(3);
      expect(saved.views?.find((v) => v.id === "main")?.kind).toBe("files");
    } finally {
      vi.useRealTimers();
    }
  });

  it("loads views written before view kinds existed as session trees", async () => {
    localStorage.setItem(VIEWS_KEY, JSON.stringify({
      version: 2,
      views: [
        { id: "main", name: "Main", treeFilter: "" },
        { id: "view-2", name: "View 2", treeFilter: "" },
      ],
      tabs: [],
      primaryId: "main",
      activeId: "main",
    }));

    vi.resetModules();
    const { useTermStore: restarted } = await import("../../store/termStore");

    expect(restarted.getState().sidebarTreeViews.map((v) => v.kind))
      .toEqual(["sessions", "sessions"]);
  });

  it("restores a saved file view and rejects an unknown kind", async () => {
    localStorage.setItem(VIEWS_KEY, JSON.stringify({
      version: 3,
      views: [
        { id: "main", name: "Main", kind: "files", treeFilter: "" },
        { id: "view-2", name: "View 2", kind: "scripts", treeFilter: "" },
      ],
      tabs: [],
      primaryId: "main",
      activeId: "main",
    }));

    vi.resetModules();
    const { useTermStore: restarted } = await import("../../store/termStore");

    expect(restarted.getState().sidebarTreeViews.map((v) => v.kind))
      .toEqual(["files", "sessions"]);
  });
});
