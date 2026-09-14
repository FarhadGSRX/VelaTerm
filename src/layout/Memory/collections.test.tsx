// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { setLang } from "../../i18n";
import type { Session } from "../../types";
import { MemoryCollections, MemoryCollectionsDirectory } from "./MemoryCollections";

const api = vi.hoisted(() => ({ collections: vi.fn(), list: vi.fn(), search: vi.fn() }));
vi.mock("../../ipc/memory", () => ({ memoryCollections: api.collections, memoryList: api.list, memoryGet: vi.fn() }));
vi.mock("../../ipc/commands", () => ({ searchSessionContent: api.search, readAgentTranscript: vi.fn().mockResolvedValue([]) }));
vi.mock("../../exportSession", () => ({ canExportContext: () => false, exportSessionToFile: vi.fn() }));
vi.mock("../sessionViewers/SessionContentViewer", () => ({
  SessionContentViewer: ({ session }: { session: { id: string } }) => <div>content:{session.id}</div>,
}));
const store = vi.hoisted(() => ({
  loadArchived: vi.fn(), restoreSession: vi.fn(), deleteNode: vi.fn(),
  projects: [], groups: [], sessions: [], archivedSessions: [] as unknown[],
}));
vi.mock("../../store/termStore", () => ({
  useTermStore: Object.assign(
    (select: (s: unknown) => unknown) => select(store),
    { getState: () => store },
  ),
}));

const archived: Session = { id: "s1", projectId: "p", name: "Archived one", kind: "claude", collapsed: false, sortOrder: 1, createdAt: 1, archivedAt: 100, agentSessionId: "agent-1" };
const collections = {
  projects: [{ id: "p", name: "Alpha", kind: "project", count: 1, sessions: [{ id: "s1", name: "Archived one", kind: "claude", count: 1, archivedAt: 100, groupPath: ["Work"] }] }],
  sessions: [archived],
};

beforeEach(() => {
  setLang("en");
  window.history.replaceState(null, "", "/?memory=collections");
  api.collections.mockReset().mockResolvedValue(collections);
  api.list.mockReset().mockResolvedValue({ projects: [], selectedSessionId: null, entries: [], total: 0, pageSize: 40, tags: [] });
  api.search.mockReset().mockResolvedValue([]);
  store.loadArchived.mockReset();
  store.restoreSession.mockReset().mockResolvedValue(undefined);
  store.deleteNode.mockReset().mockResolvedValue(undefined);
});
afterEach(cleanup);

it("browses archived sessions grouped by their original project", async () => {
  render(<MemoryCollections sessionId="" />);
  const link = await screen.findByRole("link", { name: /Archived one/ });
  expect(screen.getByText("Alpha")).toBeTruthy();
  expect(screen.getByText(/Work/)).toBeTruthy();
  const params = new URLSearchParams(link.getAttribute("href")!.split("?")[1]);
  expect(params.get("memory")).toBe("collection/s1");
  expect(params.get("memoryCollectionProject")).toBe("p");
  expect(store.loadArchived).toHaveBeenCalled();
});

it("opens a conversation with the entries generated from it", async () => {
  window.history.replaceState(null, "", "/?memory=collection/s1&memoryCollectionProject=p");
  api.list.mockResolvedValue({
    projects: [], selectedSessionId: "s1", total: 1, pageSize: 40, tags: [],
    entries: [{ id: "e1", title: "Stored topic", summary: "Reusable", tags: [], updatedAt: 0, sourceCount: 1 }],
  });
  render(<MemoryCollections sessionId="s1" />);
  expect(await screen.findByRole("link", { name: /Stored topic/ })).toBeTruthy();
  expect(api.list).toHaveBeenCalledWith(expect.objectContaining({ sessionId: "s1" }));
  fireEvent.click(screen.getByRole("tab", { name: "Conversation" }));
  await waitFor(() => expect(new URLSearchParams(location.search).get("memoryCollectionTab")).toBe("conversation"));
  expect(await screen.findByText("content:s1")).toBeTruthy();
});

it("searches archived content from the query parameter and keeps it in the URL", async () => {
  window.history.replaceState(null, "", "/?memory=collections&memoryCollectionQuery=needle");
  api.search.mockResolvedValue([{ sessionId: "s1", name: "Archived one", kind: "claude", archived: true, source: "transcript", matchCount: 1, matches: [{ messageIndex: 0, ordinal: 1, snippet: "the needle here", matched: ["needle"] }] }]);
  render(<MemoryCollections sessionId="" />);
  expect(screen.getByRole("textbox", { name: "Search archived content…" })).toHaveProperty("value", "needle");
  expect(await screen.findByText("Archived one")).toBeTruthy();
  await waitFor(() => expect(api.search).toHaveBeenCalledWith("needle", "archived"));
  fireEvent.change(screen.getByRole("textbox", { name: "Search archived content…" }), { target: { value: "tea" } });
  await waitFor(() => expect(new URLSearchParams(location.search).get("memoryCollectionQuery")).toBe("tea"));
});

it("restores and permanently deletes an archived session from its row", async () => {
  render(<MemoryCollections sessionId="" />);
  fireEvent.click((await screen.findAllByRole("button", { name: "Restore to normal session" }))[0]);
  await waitFor(() => expect(store.restoreSession).toHaveBeenCalledWith("s1"));
  fireEvent.click(screen.getAllByRole("button", { name: "Delete permanently (with recording)" })[0]);
  await waitFor(() => expect(store.deleteNode).toHaveBeenCalledWith("session", "s1"));
});

it("shows the collections hierarchy in the knowledge tree", async () => {
  render(<MemoryCollectionsDirectory />);
  const project = await screen.findByRole("link", { name: /Alpha/ });
  expect(new URLSearchParams(project.getAttribute("href")!.split("?")[1]).get("memoryCollectionProject")).toBe("p");
  fireEvent.click(screen.getByRole("button", { name: "Alpha" }));
  const session = await screen.findByRole("link", { name: /Archived one/ });
  expect(new URLSearchParams(session.getAttribute("href")!.split("?")[1]).get("memory")).toBe("collection/s1");
});
