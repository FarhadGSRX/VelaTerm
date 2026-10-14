import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ChangedFile } from "../../../ipc/commands";
import { groupChanges } from "./useGitPanel";
import { GitTab, rangeBetween } from "./GitTab";

const mocks = vi.hoisted(() => ({
  run: vi.fn(async (action: () => Promise<unknown>) => { await action(); return true; }),
  panel: {} as Record<string, unknown>,
  stage: vi.fn(async () => {}),
  unstage: vi.fn(async () => {}),
  discard: vi.fn(async () => {}),
  open: vi.fn(),
  openDoc: vi.fn(),
  copy: vi.fn(async () => {}),
  send: vi.fn(async () => {}),
  agent: null as null | { label: string; sends: boolean; send: (text: string) => Promise<void> },
}));
vi.mock("../../../i18n", () => ({ useT: () => (key: string, ...args: unknown[]) => [key, ...args].join("|") }));
vi.mock("../../../store/termStore", () => {
  const state = { openChanges: mocks.open, openDocTab: mocks.openDoc };
  const useTermStore = (select: (s: unknown) => unknown) => select(state);
  useTermStore.getState = () => state;
  return { useTermStore };
});
vi.mock("./useRepoTarget", () => ({ useRepoTarget: () => ({ repoPath: "/repo/sub", repos: [], rescan: vi.fn() }) }));
vi.mock("./useGitPanel", async (original) => ({ ...await original<object>(), useGitPanel: () => mocks.panel }));
vi.mock("./agentHandoff", () => ({ useAgentHandoff: () => mocks.agent }));
vi.mock("../../../ipc/info", () => ({ copyText: mocks.copy, revealPath: vi.fn() }));
vi.mock("../../../ipc/commands", async (original) => ({
  ...await original<object>(),
  gitStage: mocks.stage,
  gitUnstage: mocks.unstage,
  gitDiscard: mocks.discard,
}));

const change = (path: string, extra: Partial<ChangedFile> = {}): ChangedFile => ({
  path, originalPath: null, status: "modified", index: "", worktree: "modified", additions: 1, deletions: 0, binary: false,
  stagedAdditions: 0, stagedDeletions: 0, stagedBinary: false, unstagedAdditions: 1, unstagedDeletions: 0, unstagedBinary: false,
  ...extra,
});

function setFiles(files: ChangedFile[]) {
  mocks.panel = {
    status: { isRepo: true, branch: "main", head: "h", ahead: 0, behind: 0, staged: 0, unstaged: 0, untracked: 0, root: "/repo" },
    files, groups: groupChanges(files), busy: false, isRepo: true, err: "", actionErr: "", commitCount: 1,
    run: mocks.run, refresh: vi.fn(), clearActionError: vi.fn(), truncated: false, ignoreSuggestion: null,
    ignoreFolder: vi.fn(), dismissIgnoreSuggestion: vi.fn(),
  };
}

const row = (container: HTMLElement, path: string) =>
  container.querySelector<HTMLElement>(`[data-git-path="${path}"] .git-row-main`)!;

afterEach(cleanup);
beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mocks.agent = null;
  setFiles([change("a.txt"), change("b.txt"), change("c.txt"), change("d.txt")]);
});

it("computes a range in display order whichever end comes first", () => {
  expect(rangeBetween(["a", "b", "c", "d"], "c", "a")).toEqual(["a", "b", "c"]);
  expect(rangeBetween(["a", "b"], "x", "b")).toEqual(["b"]);
});

it("selects with modifier clicks and stages the selection from the bulk bar", async () => {
  const view = render(<GitTab path="/repo/sub" />);
  fireEvent.click(row(view.container, "a.txt"));
  expect(mocks.open).toHaveBeenCalledWith("/repo/sub", { path: "a.txt", side: "unstaged" });
  fireEvent.click(row(view.container, "c.txt"), { shiftKey: true });
  fireEvent.click(row(view.container, "d.txt"), { metaKey: true });
  expect(mocks.open).toHaveBeenCalledTimes(1);
  expect(view.container.querySelectorAll(".git-row.sel")).toHaveLength(4);
  fireEvent.click(row(view.container, "b.txt"), { metaKey: true });
  expect(screen.getByText("git.selectedCount|3")).toBeTruthy();
  const bulk = within(view.container.querySelector<HTMLElement>(".git-bulk")!);
  await act(async () => { fireEvent.click(bulk.getByRole("button", { name: "git.stage" })); });
  expect(mocks.stage).toHaveBeenCalledWith("/repo/sub", ["a.txt", "c.txt", "d.txt"]);
});

it("confirms Discard all at the top of the group before discarding", async () => {
  const view = render(<GitTab path="/repo/sub" />);
  fireEvent.click(screen.getByRole("button", { name: "git.discardAll" }));
  expect(mocks.discard).not.toHaveBeenCalled();
  expect(screen.getByText("git.discardConfirm|4")).toBeTruthy();
  const confirm = within(view.container.querySelector<HTMLElement>(".git-confirm")!);
  await act(async () => { fireEvent.click(confirm.getByRole("button", { name: "git.discard" })); });
  expect(mocks.discard).toHaveBeenCalledWith("/repo/sub", ["a.txt", "b.txt", "c.txt", "d.txt"]);
});

it("offers file actions in the context menu using paths from the repository root", async () => {
  const view = render(<GitTab path="/repo/sub" />);
  fireEvent.contextMenu(view.container.querySelector('[data-git-path="b.txt"]')!);
  fireEvent.click(screen.getByText("files.copyPath"));
  expect(mocks.copy).toHaveBeenCalledWith("/repo/b.txt");
  fireEvent.contextMenu(view.container.querySelector('[data-git-path="b.txt"]')!);
  fireEvent.click(screen.getByText("panel.openInEditor"));
  expect(mocks.openDoc).toHaveBeenCalledWith("/repo/b.txt");
});

it("groups files under folders in tree view and acts on a whole folder", async () => {
  setFiles([change("src/app/a.ts"), change("src/app/b.ts"), change("README.md")]);
  const view = render(<GitTab path="/repo/sub" />);
  fireEvent.click(screen.getByRole("button", { name: "git.viewAsTree" }));
  expect(localStorage.getItem("vlx.git.viewMode")).toBe("tree");
  const folder = view.container.querySelector<HTMLElement>('[data-git-dir="src/app"]')!;
  expect(folder.textContent).toContain("src/app");
  await act(async () => { fireEvent.click(folder.querySelector<HTMLElement>('[aria-label="git.stage"]')!); });
  expect(mocks.stage).toHaveBeenCalledWith("/repo/sub", ["src/app/a.ts", "src/app/b.ts"]);
  fireEvent.click(folder.querySelector(".git-row-main")!);
  expect(view.container.querySelector('[data-git-path="src/app/a.ts"]')).toBeNull();
});

it("labels conflicts, marks them resolved, and hands them to the session's agent", async () => {
  const conflict = change("x.txt", { status: "conflicted", index: "modified", conflict: "both-modified" });
  setFiles([conflict]);
  mocks.panel.status = { ...(mocks.panel.status as object), operation: "merge" };
  mocks.agent = { label: "Claude", sends: true, send: mocks.send };
  const view = render(<GitTab path="/repo/sub" />);
  expect(screen.getByText("git.conflictKind.both-modified")).toBeTruthy();
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "git.ask.resolve|Claude" })); });
  expect(mocks.send).toHaveBeenCalledWith("git.ask.resolvePrompt|- x.txt");
  await act(async () => { fireEvent.click(view.container.querySelector<HTMLElement>('[aria-label="git.markResolved"]')!); });
  expect(mocks.stage).toHaveBeenCalledWith("/repo/sub", ["x.txt"]);
});

it("asks the agent to fix a failed action and to commit staged work", async () => {
  setFiles([change("s.txt", { index: "modified", worktree: "" })]);
  mocks.panel.actionErr = "fatal: push rejected";
  mocks.agent = { label: "Codex", sends: false, send: mocks.send };
  render(<GitTab path="/repo/sub" />);
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "git.ask.fix|Codex" })); });
  expect(mocks.send).toHaveBeenLastCalledWith("git.ask.fixPrompt|fatal: push rejected");
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "git.ask.commit|Codex" })); });
  expect(mocks.send).toHaveBeenLastCalledWith("git.ask.commitPrompt");
});
