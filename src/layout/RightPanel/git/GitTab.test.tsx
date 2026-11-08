import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ChangedFile } from "../../../ipc/commands";
import { groupChanges } from "./useGitPanel";
import { GitTab } from "./GitTab";

const mocks = vi.hoisted(() => ({ open: vi.fn(), run: vi.fn(), panel: {} as Record<string, unknown> }));
vi.mock("../../../i18n", () => ({ useT: () => (key: string) => key }));
vi.mock("../../../store/termStore", () => ({ useTermStore: (select: (s: unknown) => unknown) => select({ openChanges: mocks.open }) }));
vi.mock("./useRepoTarget", () => ({ useRepoTarget: () => ({ repoPath: "/repo", repos: [], rescan: vi.fn() }) }));
vi.mock("./useGitPanel", async (original) => ({ ...await original<object>(), useGitPanel: () => mocks.panel }));

const file: ChangedFile = {
  path: "文档/非常长的中文文件名称.md", originalPath: null, status: "added", index: "added", worktree: "modified",
  additions: 3, deletions: 0, binary: false, stagedAdditions: 2, stagedDeletions: 0, stagedBinary: false,
  unstagedAdditions: 1, unstagedDeletions: 0, unstagedBinary: false,
};

afterEach(cleanup);
beforeEach(() => {
  vi.clearAllMocks();
  mocks.panel = { status: { branch: "main", head: "head" }, files: [file], groups: groupChanges([file]),
    busy: false, isRepo: true, err: "", actionErr: "", commitCount: 0, run: mocks.run, refresh: vi.fn(),
    truncated: false, ignoreSuggestion: null, ignoreFolder: vi.fn(), dismissIgnoreSuggestion: vi.fn() };
});

it("opens the correct comparison and displays each side's own status and line count", () => {
  const view = render(<GitTab path="/repo" />);
  const rows = view.container.querySelectorAll<HTMLElement>("[data-git-path]");
  expect(rows).toHaveLength(2);
  expect(rows[0].textContent).toContain("+2");
  expect(rows[1].textContent).toContain("+1");
  expect(rows[0].querySelector(".gb")?.textContent).toBe("A");
  expect(rows[1].querySelector(".gb")?.textContent).toBe("M");
  fireEvent.click(rows[0].querySelector(".git-row-main")!);
  expect(mocks.open).toHaveBeenLastCalledWith("/repo", { path: file.path, side: "staged" });
  fireEvent.click(rows[1].querySelector(".git-row-main")!);
  expect(mocks.open).toHaveBeenLastCalledWith("/repo", { path: file.path, side: "unstaged" });
  expect(rows[0].title).toBe(file.path);
});

it("filters by full path and prevents bulk operations on hidden files", () => {
  render(<GitTab path="/repo" />);
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "missing" } });
  expect(screen.getByText("git.noMatchingFiles")).toBeTruthy();
  expect(screen.queryByText("非常长的中文文件名称.md")).toBeNull();
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "文档/" } });
  expect(screen.getAllByText("非常长的中文文件名称.md")).toHaveLength(2);
  expect((screen.getByRole("button", { name: "git.stageAll" }) as HTMLButtonElement).disabled).toBe(true);
});

it("shows a staged rename only on the side where the rename occurred", () => {
  const renamed = { ...file, status: "renamed", index: "renamed", originalPath: "旧目录/原名称.md" };
  mocks.panel = { ...mocks.panel, files: [renamed], groups: groupChanges([renamed]) };
  const view = render(<GitTab path="/repo" />);
  const rows = view.container.querySelectorAll<HTMLElement>("[data-git-path]");
  expect(rows[0].title).toBe(`${renamed.originalPath} → ${renamed.path}`);
  expect(rows[1].title).toBe(renamed.path);
});

it("places conflicted files in a distinct group", () => {
  const conflict = { ...file, status: "conflicted", index: "modified" };
  const groups = groupChanges([conflict]);
  expect(groups.staged).toHaveLength(0); expect(groups.changes).toHaveLength(0);
  mocks.panel = { ...mocks.panel, files: [conflict], groups };
  const view = render(<GitTab path="/repo" />);
  expect(screen.getByText("git.conflictNotice")).toBeTruthy();
  fireEvent.click(view.container.querySelector(".git-row-main")!);
  expect(mocks.open).toHaveBeenCalledWith("/repo", { path: file.path, side: "conflicted" });
});

it("disables single-file write controls while a mutation is running", () => {
  mocks.panel.busy = true;
  render(<GitTab path="/repo" />);
  const button = screen.getByRole("button", { name: "git.stage" });
  fireEvent.click(button);
  expect(mocks.run).not.toHaveBeenCalled();
  expect((button as HTMLButtonElement).disabled).toBe(true);
});

it("explains a cut-off list and offers the suggested folder for .gitignore", () => {
  const ignoreFolder = vi.fn();
  mocks.panel = { ...mocks.panel, truncated: true, ignoreSuggestion: "pkg/node_modules", ignoreFolder };
  const view = render(<GitTab path="/repo" />);
  expect(view.container.querySelector(".git-limit")?.textContent).toContain("git.tooManyChanges");
  expect(view.container.querySelector("label[for='git-file-filter']")?.textContent).toBe("git.changedFilesOver");
  fireEvent.click(view.getByText("git.addToGitignore"));
  expect(ignoreFolder).toHaveBeenCalledTimes(1);
});

it("shows no limit notice for a complete list", () => {
  const view = render(<GitTab path="/repo" />);
  expect(view.container.querySelector(".git-limit")).toBeNull();
});
