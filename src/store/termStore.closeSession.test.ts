//! Killing a session's process must retire its unread marker. The tree node outlives the process, so
//! `focusReturned`'s stale sweep -- which only drops IDs whose session is gone -- never reaches it, and
//! the sidebar's blue dot would otherwise sit on a dead session claiming there is something to read.

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
import type { Session } from "../types";

const session = (id: string): Session => ({
  id,
  projectId: "p1",
  groupId: null,
  parentSessionId: null,
  name: id,
  kind: "claude",
  collapsed: false,
  sortOrder: 0,
  createdAt: 0,
});

let seq = 0;
let sessionId = "";

beforeEach(() => {
  sessionId = `close-session-${++seq}`;
  useTermStore.setState({
    sessions: [session(sessionId)],
    ephemeralSessions: {},
    runtimes: { [sessionId]: { status: "running" } },
    notifications: { [sessionId]: Date.now() },
    paneTrees: {},
    openTabs: [],
    liveTabs: [],
    activeTabId: null,
  });
});

describe("closeSession and the unread marker", () => {
  it("clears the unread marker when the process stops", () => {
    expect(sessionId in useTermStore.getState().notifications).toBe(true);
    useTermStore.getState().closeSession(sessionId);
    expect(sessionId in useTermStore.getState().notifications).toBe(false);
  });

  it("leaves the session node in the tree, which is why the stale sweep cannot do this job", () => {
    // Restore the marker and run the ONLY other thing that clears markers: focusReturned's sweep. It
    // keys off the session having disappeared, and the node survives the kill -- so it does nothing
    // here. This is what makes the fix belong in closeSession.
    useTermStore.setState({ notifications: { [sessionId]: Date.now() } });
    useTermStore.getState().focusReturned();
    expect(sessionId in useTermStore.getState().notifications).toBe(true);
    expect(useTermStore.getState().sessions.some((s) => s.id === sessionId)).toBe(true);
  });

  it("does not disturb other sessions' unread markers", () => {
    const other = `${sessionId}-other`;
    useTermStore.setState({
      sessions: [session(sessionId), session(other)],
      notifications: { [sessionId]: 1, [other]: 2 },
    });
    useTermStore.getState().closeSession(sessionId);
    const { notifications } = useTermStore.getState();
    expect(sessionId in notifications).toBe(false);
    expect(notifications[other]).toBe(2);
  });
});
