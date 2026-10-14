import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { groupChanges } from "./useGitPanel";
import { GitTab } from "./GitTab";
import { CommitBox } from "./CommitBox";

const mocks = vi.hoisted(() => ({
  run: vi.fn(async (action: () => Promise<unknown>) => { await action(); return true; }),
  panel: {} as Record<string, unknown>,
  fetch: vi.fn(async () => {}),
  pull: vi.fn(async () => {}),
  push: vi.fn(async () => {}),
  abort: vi.fn(async () => {}),
  cont: vi.fn(async () => {}),
  log: vi.fn(),
}));
vi.mock("../../../i18n", () => ({ useT: () => (key: string, ...args: unknown[]) => [key, ...args].join("|") }));
vi.mock("../../../store/termStore", () => ({ useTermStore: (select: (s: unknown) => unknown) => select({ openChanges: vi.fn() }) }));
vi.mock("./useRepoTarget", () => ({ useRepoTarget: () => ({ repoPath: "/repo", repos: [], rescan: vi.fn() }) }));
vi.mock("./useGitPanel", async (original) => ({ ...await original<object>(), useGitPanel: () => mocks.panel }));
vi.mock("../../../ipc/commands", async (original) => ({
  ...await original<object>(),
  gitFetch: mocks.fetch,
  gitPull: mocks.pull,
  gitPush: mocks.push,
  gitAbortOperation: mocks.abort,
  gitContinueOperation: mocks.cont,
  gitLogPage: mocks.log,
}));

afterEach(cleanup);
beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mocks.panel = {
    status: { isRepo: true, branch: "main", head: "h1", upstream: "origin/main", hasRemote: true, ahead: 2, behind: 1,
      staged: 0, unstaged: 0, untracked: 0 },
    files: [], groups: groupChanges([]), busy: false, isRepo: true, err: "", actionErr: "", commitCount: 3,
    run: mocks.run, refresh: vi.fn(), truncated: false, ignoreSuggestion: null, ignoreFolder: vi.fn(),
    dismissIgnoreSuggestion: vi.fn(),
  };
});

it("pulls fast-forward only and pushes from the buttons showing pending counts", async () => {
  render(<GitTab path="/repo" />);
  const pull = screen.getByRole("button", { name: /git\.pull1/ });
  const push = screen.getByRole("button", { name: /git\.push2/ });
  await act(async () => { fireEvent.click(pull); });
  expect(mocks.pull).toHaveBeenCalledWith("/repo", "ff-only");
  await act(async () => { fireEvent.click(push); });
  expect(mocks.push).toHaveBeenCalledWith("/repo", false);
});

it("offers Publish for a branch without upstream and hides sync without any remote", async () => {
  mocks.panel.status = { ...(mocks.panel.status as object), upstream: null, ahead: 0, behind: 0 };
  const view = render(<GitTab path="/repo" />);
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "git.publish" })); });
  expect(mocks.push).toHaveBeenCalledWith("/repo", false);
  view.unmount();
  mocks.panel.status = { ...(mocks.panel.status as object), hasRemote: false };
  const bare = render(<GitTab path="/repo" />);
  expect(bare.container.querySelector(".git-sync")).toBeNull();
});

it("asks before a force push and uses the menu for other pull strategies", async () => {
  render(<GitTab path="/repo" />);
  fireEvent.click(screen.getByRole("button", { name: "git.moreActions" }));
  await act(async () => { fireEvent.click(screen.getByText("git.pullRebase")); });
  expect(mocks.pull).toHaveBeenCalledWith("/repo", "rebase");
  fireEvent.click(screen.getByRole("button", { name: "git.moreActions" }));
  fireEvent.click(screen.getByText("git.forcePush"));
  expect(mocks.push).not.toHaveBeenCalled();
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "git.forcePushAction" })); });
  expect(mocks.push).toHaveBeenCalledWith("/repo", true);
});

it("replaces the commit box with continue and abort while an operation is stopped", async () => {
  const conflict = { path: "a.txt", originalPath: null, status: "conflicted", index: "modified", worktree: "modified",
    additions: 0, deletions: 0, binary: false };
  mocks.panel = { ...mocks.panel, status: { ...(mocks.panel.status as object), operation: "rebase" },
    files: [conflict], groups: groupChanges([conflict as never]) };
  const view = render(<GitTab path="/repo" />);
  expect(view.container.querySelector(".git-commit")).toBeNull();
  expect(screen.getByText("git.operation.rebase")).toBeTruthy();
  // Pulling or pushing mid-rebase is blocked.
  expect((screen.getByRole("button", { name: /git\.pull/ }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByRole("button", { name: "git.continue" }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "git.abort" }));
  expect(screen.getByText("git.abortConfirm")).toBeTruthy();
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "git.abort" })); });
  expect(mocks.abort).toHaveBeenCalledWith("/repo");
});

it("keeps the commit draft per repository across remounts", () => {
  const onCommit = vi.fn(async () => true);
  const first = render(<CommitBox cwd="/a" head="h1" stagedCount={1} busy={false} onCommit={onCommit} />);
  fireEvent.change(first.getByRole("textbox"), { target: { value: "Work in progress" } });
  first.unmount();
  const again = render(<CommitBox cwd="/a" head="h1" stagedCount={1} busy={false} onCommit={onCommit} />);
  expect((again.getByRole("textbox") as HTMLTextAreaElement).value).toBe("Work in progress");
  again.unmount();
  const other = render(<CommitBox cwd="/b" head="h1" stagedCount={1} busy={false} onCommit={onCommit} />);
  expect((other.getByRole("textbox") as HTMLTextAreaElement).value).toBe("");
});

it("keeps text typed while a slow commit runs", async () => {
  let finish: (ok: boolean) => void = () => {};
  const onCommit = vi.fn(() => new Promise<boolean>((resolve) => { finish = resolve; }));
  const view = render(<CommitBox cwd="/a" head="h1" stagedCount={1} busy={false} onCommit={onCommit} />);
  const box = view.getByRole("textbox") as HTMLTextAreaElement;
  fireEvent.change(box, { target: { value: "First" } });
  fireEvent.click(view.getByRole("button", { name: /git\.commitCount/ }));
  fireEvent.change(box, { target: { value: "Second" } });
  await act(async () => { finish(true); });
  expect(onCommit).toHaveBeenCalledWith("First", false, null);
  expect(box.value).toBe("Second");
});

it("names the amend target, fills its message, and binds the commit to it", async () => {
  mocks.log.mockResolvedValue([{ oid: "h1", hash: "abc1234", subject: "Old subject", author: "", relative: "", message: "Old subject\n\nBody" }]);
  const onCommit = vi.fn(async () => true);
  const view = render(<CommitBox cwd="/a" head="h1" stagedCount={0} busy={false} onCommit={onCommit} />);
  await act(async () => { fireEvent.click(view.getByRole("checkbox")); });
  expect(mocks.log).toHaveBeenCalledWith("/a", 1, 0, "h1");
  expect(view.getByText("git.amendTarget|abc1234|Old subject")).toBeTruthy();
  const box = view.getByRole("textbox") as HTMLTextAreaElement;
  expect(box.value).toBe("Old subject\n\nBody");
  await act(async () => { fireEvent.click(view.getByRole("button", { name: "git.amendCommit" })); });
  expect(onCommit).toHaveBeenCalledWith("Old subject\n\nBody", true, "h1");
});

it("clears an untouched prefilled message when amend is turned off and disables amend before the first commit", async () => {
  mocks.log.mockResolvedValue([{ oid: "h1", hash: "abc1234", subject: "Old", author: "", relative: "", message: "Old" }]);
  const view = render(<CommitBox cwd="/a" head="h1" stagedCount={0} busy={false} onCommit={vi.fn()} />);
  const amend = view.getByRole("checkbox");
  await act(async () => { fireEvent.click(amend); });
  expect((view.getByRole("textbox") as HTMLTextAreaElement).value).toBe("Old");
  fireEvent.click(amend);
  expect((view.getByRole("textbox") as HTMLTextAreaElement).value).toBe("");
  view.rerender(<CommitBox cwd="/a" head={null} stagedCount={0} busy={false} onCommit={vi.fn()} />);
  expect((view.getByRole("checkbox") as HTMLInputElement).disabled).toBe(true);
});
