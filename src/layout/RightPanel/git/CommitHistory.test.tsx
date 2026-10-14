import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { CommitInfo } from "../../../ipc/commands";

const mocks = vi.hoisted(() => ({ gitLogPage: vi.fn(), gitCommitFiles: vi.fn() }));
vi.mock("../../../ipc/commands", () => mocks);

import { CommitHistory } from "./CommitHistory";

const commit = (id: string, subject: string, parents: string[] = []): CommitInfo => ({
  hash: id.slice(0, 7),
  id,
  subject,
  author: "a",
  relative: "1 minute ago",
  parents,
  refs: [],
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("CommitHistory refresh", () => {
  it("refreshes in place: no loading flash, and an expanded commit stays open", async () => {
    const older = commit("b".repeat(40), "older");
    const newer = commit("a".repeat(40), "newer", [older.id]);
    mocks.gitCommitFiles.mockResolvedValue([{ path: "a.ts", status: "modified", additions: 1, deletions: 0 }]);
    mocks.gitLogPage.mockResolvedValue([newer, older]);

    const props = { cwd: "/repo", total: 2, allBranches: false };
    const view = render(<CommitHistory {...props} tick={0} />);
    await screen.findByText("newer");
    fireEvent.click(screen.getByText("older"));
    await act(async () => {});
    expect(document.querySelector(".git-commit-files")).not.toBeNull();

    // The panel's poll: the same history comes back.
    view.rerender(<CommitHistory {...props} tick={1} />);
    expect(screen.queryByText("Loading…")).toBeNull();
    expect(screen.getByText("newer")).toBeTruthy();
    await act(async () => {});
    expect(mocks.gitLogPage).toHaveBeenLastCalledWith("/repo", expect.any(Number), 0, false);
    expect(document.querySelector(".git-commit-files")).not.toBeNull();

    // A new commit lands on top: it appears, nothing blanks, and the open commit is still open.
    const newest = commit("c".repeat(40), "newest", [newer.id]);
    mocks.gitLogPage.mockResolvedValue([newest, newer, older]);
    view.rerender(<CommitHistory {...props} total={3} tick={2} />);
    expect(screen.queryByText("Loading…")).toBeNull();
    await screen.findByText("newest");
    expect(screen.getByText("older")).toBeTruthy();
    expect(document.querySelector(".git-commit-files")).not.toBeNull();
  });

  it("shows the full commit message on hover, not just the hash", async () => {
    mocks.gitLogPage.mockResolvedValue([commit("e".repeat(40), "Merge branch 'fg-main' into feature")]);
    render(<CommitHistory cwd="/repo" total={1} allBranches={false} tick={0} />);
    const row = (await screen.findByText("Merge branch 'fg-main' into feature")).closest("button")!;
    expect(row.title.split("\n")[0]).toBe("Merge branch 'fg-main' into feature");
    expect(row.title).toContain("eeeeeee");
  });

  it("starts over when the repository changes", async () => {
    mocks.gitCommitFiles.mockResolvedValue([{ path: "a.ts", status: "modified", additions: 1, deletions: 0 }]);
    mocks.gitLogPage.mockResolvedValue([commit("a".repeat(40), "first repo")]);
    const view = render(<CommitHistory cwd="/one" total={1} allBranches={false} tick={0} />);
    await screen.findByText("first repo");

    mocks.gitLogPage.mockResolvedValue([commit("d".repeat(40), "second repo")]);
    view.rerender(<CommitHistory cwd="/two" total={1} allBranches={false} tick={0} />);
    await screen.findByText("second repo");
    expect(screen.queryByText("first repo")).toBeNull();
  });
});
