import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { setLang } from "../../i18n";
import { MemoryCompile, MemoryJobs } from "./MemoryTasks";
import type { MemoryJob } from "../../ipc/memory";

const api = vi.hoisted(() => ({ jobs: vi.fn(), cancel: vi.fn(), start: vi.fn() }));
vi.mock("../../ipc/memory", () => ({
  memoryJobs: api.jobs, memoryCancel: api.cancel, memoryStart: api.start, memoryRetry: vi.fn(),
  memoryOptions: async () => ({ agents: [{ id: "codex", label: "Codex", available: true }], defaultAgent: "codex" }),
  memoryModels: async () => [],
}));
vi.mock("../../store/termStore", () => ({
  useTermStore: Object.assign(
    (select: (s: unknown) => unknown) => select({ sessions: [{ id: "session", name: "Session A" }], archivedSessions: [] }),
    { getState: () => ({ memoryPrefs: {}, setMemoryPrefs: vi.fn() }) },
  ),
}));
const job = (id: string, status: string): MemoryJob => ({
  id, status, sessionName: `Session ${id}`, sourceId: `source-${id}`, agent: "codex", model: "", effort: "",
  stage: "extract", progress: 0, total: 1, error: "", entries: [], createdAt: 0, updatedAt: 0,
});
beforeEach(() => {
  setLang("en"); window.history.replaceState(null, "", "/?memory=jobs");
  api.jobs.mockReset(); api.cancel.mockReset(); api.start.mockReset();
});
afterEach(cleanup);

it("explains replacement and opens the new job returned by submission", async () => {
  api.start.mockResolvedValue({ id: "replacement", reused: false });
  render(<MemoryCompile sessionId="session" />);
  await screen.findByRole("combobox", { name: "Model (optional)" });
  expect(screen.getByText(/Submitting again cancels any unfinished task/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Organize and save" }));
  await waitFor(() => expect(api.start).toHaveBeenCalledWith("session", "codex", "", ""));
  await waitFor(() => expect(new URLSearchParams(location.search).get("memory")).toBe("job/replacement"));
  expect(screen.queryByRole("alert")).toBeNull();
});

it("shows parallel sessions and lets the user cancel a replacement waiting for its predecessor", async () => {
  const jobs = [job("A", "running"), job("B", "running"), job("old", "cancelling"), job("latest", "queued")];
  api.jobs.mockResolvedValue({ jobs, total: jobs.length, pageSize: 40 });
  api.cancel.mockImplementation(async (id) => {
    api.jobs.mockResolvedValue({ jobs: jobs.map(j => j.id === id ? { ...j, status: "cancelled" } : j), total: jobs.length, pageSize: 40 });
  });
  render(<MemoryJobs />);
  expect(await screen.findAllByText("In progress")).toHaveLength(2);
  const old = (await screen.findByRole("link", { name: "Session old" })).closest("article")!;
  expect(within(old).getByText("Cancelling")).toBeTruthy();
  expect(within(old).queryByRole("button", { name: "Cancel" })).toBeNull();
  const latest = screen.getByRole("link", { name: "Session latest" }).closest("article")!;
  expect(within(latest).getByRole("status").textContent).toContain("once the previous task");
  expect(within(latest).getByRole("link", { name: "Session latest" }).getAttribute("href")).toContain("memory=job%2Flatest");
  fireEvent.click(within(latest).getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(api.cancel).toHaveBeenCalledWith("latest"));
  await screen.findByText("Cancelled");
  expect(screen.getAllByText("In progress")).toHaveLength(2);
});
