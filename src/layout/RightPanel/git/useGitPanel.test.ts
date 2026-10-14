import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useGitPanel } from "./useGitPanel";

const mocks = vi.hoisted(() => ({ status: vi.fn(), files: vi.fn(), count: vi.fn(), suggest: vi.fn(), ignore: vi.fn(),
  watch: vi.fn(), changed: null as null | ((event: { root: string; dirs: string[] | null; git: boolean }) => void) }));
vi.mock("../../../ipc/info", () => ({ getGitStatus: mocks.status }));
vi.mock("../../../ipc/commands", () => ({ gitChangedFiles: mocks.files, gitCommitCount: mocks.count,
  gitIgnoreSuggestion: mocks.suggest, gitIgnoreFolder: mocks.ignore, gitWatch: mocks.watch }));
vi.mock("../../../ipc/events", () => ({
  onFsChanged: (cb: (event: { root: string; dirs: string[] | null; git: boolean }) => void) => { mocks.changed = cb; return Promise.resolve(() => { mocks.changed = null; }); },
}));
const status = (head = "first") => ({ isRepo: true, head, branch: "main", ahead: 0, behind: 0, staged: 0, unstaged: 1, untracked: 0 });

beforeEach(() => {
  vi.useFakeTimers();
  vi.resetAllMocks();
  mocks.status.mockResolvedValue(status());
  mocks.files.mockResolvedValue({ files: [{ path: "example.txt", index: "", worktree: "modified" }], truncated: false });
  mocks.count.mockResolvedValue(1);
  // Unwatched by default, so the poll-driven tests below behave as on Linux.
  mocks.watch.mockResolvedValue({ root: "/repo", supported: false });
});
afterEach(() => { cleanup(); vi.useRealTimers(); });

describe("Git panel background refresh", () => {
  it("refreshes on change events for its root and polls only as a safety net when watched", async () => {
    mocks.watch.mockResolvedValue({ root: "/repo", supported: true });
    renderHook(() => useGitPanel("/repo/sub"));
    await act(async () => {});
    expect(mocks.watch).toHaveBeenCalledWith("/repo/sub");
    expect(mocks.files).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    expect(mocks.files).toHaveBeenCalledTimes(1);
    await act(async () => { mocks.changed?.({ root: "/elsewhere", dirs: [], git: false }); await vi.advanceTimersByTimeAsync(300); });
    expect(mocks.files).toHaveBeenCalledTimes(1);
    await act(async () => {
      mocks.changed?.({ root: "/repo", dirs: [""], git: false });
      mocks.changed?.({ root: "/repo", dirs: null, git: true });
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(mocks.files).toHaveBeenCalledTimes(2);
    await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
    expect(mocks.files).toHaveBeenCalledTimes(3);
    expect(mocks.watch.mock.calls.length).toBeGreaterThanOrEqual(2);
  });

  it("keeps its snapshot on failure and only recounts commits after HEAD changes", async () => {
    const view = renderHook(() => useGitPanel("/repo"));
    await act(async () => {});
    expect(view.result.current.files).toHaveLength(1);
    mocks.files.mockRejectedValueOnce(new Error("Temporarily unavailable"));
    await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
    expect(view.result.current.isRepo).toBe(true);
    expect(view.result.current.files).toHaveLength(1);
    expect(view.result.current.err).toContain("Temporarily unavailable");
    await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
    expect(view.result.current.err).toBe("");
    expect(mocks.count).toHaveBeenCalledTimes(1);
    mocks.status.mockResolvedValueOnce(status("amended"));
    await act(async () => { view.result.current.refresh(); });
    expect(mocks.count).toHaveBeenCalledTimes(2);
    expect(view.result.current.status?.head).toBe("amended");
  });

  it("does not overlap slow polls and ignores the old repository response", async () => {
    let resolve!: (value: ReturnType<typeof status>) => void;
    mocks.status.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    const view = renderHook(({ path }) => useGitPanel(path), { initialProps: { path: "/old" } });
    await act(async () => { await vi.advanceTimersByTimeAsync(15000); });
    expect(mocks.status).toHaveBeenCalledTimes(1);
    view.rerender({ path: "/new" });
    await act(async () => {});
    expect(view.result.current.status?.branch).toBe("main");
    await act(async () => { resolve({ ...status("old"), branch: "old-branch" }); });
    expect(view.result.current.status?.branch).toBe("main");
    expect(view.result.current.status?.head).toBe("first");
  });

  it("keeps the same file list object when a poll returns nothing new", async () => {
    const view = renderHook(() => useGitPanel("/repo"));
    await act(async () => {});
    const first = view.result.current.files;
    await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
    expect(mocks.files).toHaveBeenCalledTimes(2);
    expect(view.result.current.files).toBe(first);
  });

  it("looks up the folder to ignore once per flood and adds it on request", async () => {
    mocks.files.mockResolvedValue({ files: [{ path: "pkg/node_modules/a.js", index: "", worktree: "untracked" }], truncated: true });
    mocks.suggest.mockResolvedValue("pkg/node_modules");
    mocks.ignore.mockResolvedValue(true);
    const view = renderHook(() => useGitPanel("/repo"));
    await act(async () => {});
    expect(view.result.current.truncated).toBe(true);
    expect(view.result.current.ignoreSuggestion).toBe("pkg/node_modules");
    await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
    expect(mocks.suggest).toHaveBeenCalledTimes(1);
    await act(async () => { view.result.current.ignoreFolder(); });
    expect(mocks.ignore).toHaveBeenCalledWith("/repo", "pkg/node_modules");
    // The list may still be over the cap, so the suggestion is looked up again.
    expect(mocks.suggest).toHaveBeenCalledTimes(2);
  });

  it("hides a dismissed suggestion", async () => {
    mocks.files.mockResolvedValue({ files: [], truncated: true });
    mocks.suggest.mockResolvedValue("build");
    const view = renderHook(() => useGitPanel("/dismiss"));
    await act(async () => {});
    expect(view.result.current.ignoreSuggestion).toBe("build");
    act(() => { view.result.current.dismissIgnoreSuggestion(); });
    expect(view.result.current.ignoreSuggestion).toBeNull();
  });
});
