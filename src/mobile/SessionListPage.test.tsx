import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  createWorktree: vi.fn(),
  getSessionCwd: vi.fn().mockResolvedValue(null),
  ptyKill: vi.fn().mockResolvedValue(undefined),
  ptyWrite: vi.fn().mockResolvedValue(undefined),
  listShells: vi.fn().mockResolvedValue([]),
}));
vi.mock("../ipc/tree", () => ({ listTree: vi.fn().mockResolvedValue({ projects: [], groups: [], sessions: [] }) }));
vi.mock("../notify", () => ({
  notify: vi.fn(),
  getNotifyPermission: vi.fn().mockResolvedValue("granted"),
  requestNotifyPermission: vi.fn().mockResolvedValue("granted"),
  getEffectiveNotifyPermission: vi.fn().mockResolvedValue("granted"),
  requestEffectiveNotifyPermission: vi.fn().mockResolvedValue("granted"),
}));

import { useTermStore } from "../store/termStore";
import type { Project, Session } from "../types";
import { SessionListPage } from "./SessionListPage";

const session = (id: string): Session => ({
  id,
  projectId: "p1",
  groupId: null,
  name: id,
  kind: "claude",
  collapsed: false,
  sortOrder: 0,
  createdAt: 0,
});

const working = { status: "running", agent: "claude", agentState: "working" } as const;
const waiting = { status: "running", agent: "claude", agentState: "waiting" } as const;

describe("mobile status chips", () => {
  beforeEach(() => {
    useTermStore.setState({
      projects: [{ id: "p1", name: "project", collapsed: false } as Project],
      groups: [],
      sessions: [session("busy"), session("later")],
      runtimes: { busy: working, later: waiting },
      notifications: {},
      treeFilter: "",
      sidebarTreeViews: [{
        id: "main",
        name: "Main",
        kind: "sessions",
        treeFilter: "",
        statusFilter: null,
        statusFilterIds: null,
        markFilter: null,
        collapsedOverrides: null,
      }],
      primarySidebarTreeViewId: "main",
      statusFilter: null,
      statusFilterIds: null,
      dynamicStatusFilter: true,
    });
  });
  afterEach(cleanup);

  it("lists a session that starts working after the filter was selected", () => {
    render(<SessionListPage onOpen={() => {}} />);
    fireEvent.click(screen.getByText("Working").closest("button")!);
    expect(screen.queryByText("later")).toBeNull();

    act(() => useTermStore.setState((s) => ({ runtimes: { ...s.runtimes, later: working } })));

    expect(screen.getByText("later")).toBeTruthy();
    expect(screen.getByText("busy")).toBeTruthy();
  });

  it("keeps the selected snapshot when dynamic additions are off", () => {
    useTermStore.setState({ dynamicStatusFilter: false });
    render(<SessionListPage onOpen={() => {}} />);
    fireEvent.click(screen.getByText("Working").closest("button")!);

    act(() => useTermStore.setState((s) => ({ runtimes: { ...s.runtimes, later: working } })));

    expect(screen.queryByText("later")).toBeNull();
  });
});
