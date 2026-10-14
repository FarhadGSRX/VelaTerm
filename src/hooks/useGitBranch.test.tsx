//! The branch shown for a path follows checkouts: a change inside the watched repository's `.git`, or the
//! window regaining focus, re-probes the branch instead of serving the first answer forever.

import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FsChange } from "../ipc/events";

const mocks = vi.hoisted(() => ({
  getGitStatus: vi.fn(),
  gitWatch: vi.fn(),
  fsListeners: [] as ((event: FsChange) => void)[],
}));

vi.mock("../ipc/info", () => ({ getGitStatus: mocks.getGitStatus }));
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

function Branch({ path }: { path: string }) {
  return <span data-testid="branch">{useGitBranch(path) ?? "none"}</span>;
}

function status(branch: string) {
  return { isRepo: true, branch, isWorktree: false, worktreePath: null };
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("useGitBranch", () => {
  it("re-probes after a change inside the watched repository's .git", async () => {
    mocks.gitWatch.mockResolvedValue({ root: "/repo-a", supported: true });
    mocks.getGitStatus.mockResolvedValue(status("feature/x"));
    render(<Branch path="/repo-a/sub" />);
    await waitFor(() => expect(screen.getByTestId("branch").textContent).toBe("feature/x"));
    expect(mocks.gitWatch).toHaveBeenCalledWith("/repo-a/sub");

    mocks.getGitStatus.mockResolvedValue(status("main"));
    // Listing changes and other repositories leave the branch alone.
    act(() => {
      for (const l of mocks.fsListeners) l({ root: "/repo-a", dirs: ["src"], git: false });
      for (const l of mocks.fsListeners) l({ root: "/other", dirs: null, git: true });
    });
    await new Promise((r) => setTimeout(r, 200));
    expect(screen.getByTestId("branch").textContent).toBe("feature/x");

    act(() => {
      for (const l of mocks.fsListeners) l({ root: "/repo-a", dirs: [], git: true });
    });
    await waitFor(() => expect(screen.getByTestId("branch").textContent).toBe("main"));
  });

  it("re-probes on window focus where the repository cannot be watched", async () => {
    mocks.gitWatch.mockResolvedValue({ root: "/repo-b", supported: false });
    mocks.getGitStatus.mockResolvedValue(status("feature/y"));
    render(<Branch path="/repo-b" />);
    await waitFor(() => expect(screen.getByTestId("branch").textContent).toBe("feature/y"));

    mocks.getGitStatus.mockResolvedValue(status("main"));
    act(() => { window.dispatchEvent(new Event("focus")); });
    await waitFor(() => expect(screen.getByTestId("branch").textContent).toBe("main"));
  });

  it("stops listening once nothing displays a branch", async () => {
    mocks.gitWatch.mockResolvedValue({ root: "/repo-c", supported: true });
    mocks.getGitStatus.mockResolvedValue(status("main"));
    const { unmount } = render(<Branch path="/repo-c" />);
    await waitFor(() => expect(mocks.fsListeners.length).toBe(1));
    unmount();
    await waitFor(() => expect(mocks.fsListeners.length).toBe(0));
  });
});
