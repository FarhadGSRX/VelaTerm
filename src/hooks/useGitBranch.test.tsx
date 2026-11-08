//! The branch shown for a path follows checkouts: a change inside the watched repository's `.git`, or the
//! window regaining focus, re-probes the branch instead of serving the first answer forever.

import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FsChange } from "../ipc/events";

const mocks = vi.hoisted(() => ({
  getGitBranchInfo: vi.fn(),
  gitWatch: vi.fn(),
  fsListeners: [] as ((event: FsChange) => void)[],
}));

vi.mock("../ipc/info", () => ({ getGitBranchInfo: mocks.getGitBranchInfo }));
vi.mock("../ipc/commands", () => ({ gitWatch: mocks.gitWatch }));
vi.mock("../ipc/events", () => ({
  onFsChanged: (cb: (event: FsChange) => void) => {
    mocks.fsListeners.push(cb);
    return Promise.resolve(() => {
      mocks.fsListeners = mocks.fsListeners.filter((l) => l !== cb);
    });
  },
}));

import { useGitBranch } from "./useGitBranch";

function Branch({ path, live = true }: { path: string; live?: boolean }) {
  return <span data-testid="branch">{useGitBranch(path, live) ?? "none"}</span>;
}

function status(branch: string) {
  return { isRepo: true, branch, isWorktree: false, worktreePath: null };
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("useGitBranch", () => {
  it("checks sidebar metadata once without watches or focus refreshes", async () => {
    mocks.getGitBranchInfo.mockResolvedValue(status("main"));
    const view = render(<Branch path="/sidebar-only" live={false} />);
    await waitFor(() => expect(screen.getByTestId("branch").textContent).toBe("main"));
    expect(mocks.gitWatch).not.toHaveBeenCalled();
    expect(mocks.fsListeners).toHaveLength(0);
    act(() => { window.dispatchEvent(new Event("focus")); });
    view.unmount();
    render(<Branch path="/sidebar-only" live={false} />);
    expect(mocks.getGitBranchInfo).toHaveBeenCalledTimes(1);
  });

  it("coalesces repeated refreshes while a slow probe is running", async () => {
    let finish!: (value: ReturnType<typeof status>) => void;
    mocks.gitWatch.mockResolvedValue({ root: "/slow", supported: true });
    mocks.getGitBranchInfo.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }))
      .mockResolvedValue(status("new-branch"));
    render(<Branch path="/slow" />);
    await waitFor(() => expect(mocks.gitWatch).toHaveBeenCalled());
    act(() => {
      for (let i = 0; i < 4; i++) window.dispatchEvent(new Event("focus"));
      for (const listener of mocks.fsListeners) listener({ root: "/slow", dirs: [], git: true, branch: true });
    });
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(mocks.getGitBranchInfo).toHaveBeenCalledTimes(1);
    await act(async () => { finish(status("stale-branch")); });
    await waitFor(() => expect(screen.getByTestId("branch").textContent).toBe("new-branch"));
    expect(mocks.getGitBranchInfo).toHaveBeenCalledTimes(2);
  });

  it("ignores index-only notifications", async () => {
    mocks.gitWatch.mockResolvedValue({ root: "/index-only", supported: true });
    mocks.getGitBranchInfo.mockResolvedValue(status("main"));
    render(<Branch path="/index-only" />);
    await waitFor(() => expect(screen.getByTestId("branch").textContent).toBe("main"));
    act(() => {
      for (const listener of mocks.fsListeners) listener({ root: "/index-only", dirs: [], git: true, branch: false });
    });
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(mocks.getGitBranchInfo).toHaveBeenCalledTimes(1);
  });

  it("stops live refreshes while a cached sidebar consumer remains", async () => {
    mocks.gitWatch.mockResolvedValue({ root: "/shared-live", supported: true });
    mocks.getGitBranchInfo.mockResolvedValue(status("main"));
    const live = render(<Branch path="/shared-live" />);
    await waitFor(() => expect(mocks.getGitBranchInfo).toHaveBeenCalledTimes(1));
    const sidebar = render(<Branch path="/shared-live" live={false} />);
    live.unmount();
    await waitFor(() => expect(mocks.fsListeners).toHaveLength(0));
    act(() => { window.dispatchEvent(new Event("focus")); });
    expect(mocks.getGitBranchInfo).toHaveBeenCalledTimes(1);
    sidebar.unmount();
  });

  it("re-probes after a change inside the watched repository's .git", async () => {
    mocks.gitWatch.mockResolvedValue({ root: "/repo-a", supported: true });
    mocks.getGitBranchInfo.mockResolvedValue(status("feature/x"));
    render(<Branch path="/repo-a/sub" />);
    await waitFor(() => expect(screen.getByTestId("branch").textContent).toBe("feature/x"));
    expect(mocks.gitWatch).toHaveBeenCalledWith("/repo-a/sub");

    mocks.getGitBranchInfo.mockResolvedValue(status("main"));
    // Listing changes and other repositories leave the branch alone.
    act(() => {
      for (const l of mocks.fsListeners) l({ root: "/repo-a", dirs: ["src"], git: false });
      for (const l of mocks.fsListeners) l({ root: "/other", dirs: null, git: true });
    });
    await new Promise((r) => setTimeout(r, 200));
    expect(screen.getByTestId("branch").textContent).toBe("feature/x");

    act(() => {
      for (const l of mocks.fsListeners) l({ root: "/repo-a", dirs: [], git: true, branch: true });
    });
    await waitFor(() => expect(screen.getByTestId("branch").textContent).toBe("main"));
  });

  it("re-probes on window focus where the repository cannot be watched", async () => {
    mocks.gitWatch.mockResolvedValue({ root: "/repo-b", supported: false });
    mocks.getGitBranchInfo.mockResolvedValue(status("feature/y"));
    render(<Branch path="/repo-b" />);
    await waitFor(() => expect(screen.getByTestId("branch").textContent).toBe("feature/y"));

    mocks.getGitBranchInfo.mockResolvedValue(status("main"));
    act(() => { window.dispatchEvent(new Event("focus")); });
    await waitFor(() => expect(screen.getByTestId("branch").textContent).toBe("main"));
  });

  it("stops listening once nothing displays a branch", async () => {
    mocks.gitWatch.mockResolvedValue({ root: "/repo-c", supported: true });
    mocks.getGitBranchInfo.mockResolvedValue(status("main"));
    const { unmount } = render(<Branch path="/repo-c" />);
    await waitFor(() => expect(mocks.fsListeners.length).toBe(1));
    unmount();
    await waitFor(() => expect(mocks.fsListeners.length).toBe(0));
  });
});
