import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BranchCompareSection } from "./BranchCompareSection";

const mocks = vi.hoisted(() => ({ compare: vi.fn(), open: vi.fn(), provider: vi.fn(), openPr: vi.fn(), external: vi.fn() }));
vi.mock("../../../i18n", () => ({ useT: () => (key: string) => key }));
vi.mock("../../../store/termStore", () => ({ useTermStore: (select: (s: unknown) => unknown) => select({ openChanges: mocks.open }) }));
vi.mock("../../../ipc/commands", () => ({ gitBranchCompare: mocks.compare, gitPrProvider: mocks.provider, gitOpenPullRequest: mocks.openPr }));
vi.mock("../../../platform", () => ({ platform: { opener: { openExternal: mocks.external } } }));
vi.mock("../../../hooks/nativeViewSuspend", () => ({ useSuspendNativeViews: vi.fn() }));

const result = {
  base: "origin/main", mergeBase: "aaaa", head: "bbbb", ahead: 2, behind: 1, candidates: ["main", "origin/main"],
  files: [{ path: "src/a.ts", status: "modified", index: "modified", worktree: "", additions: 3, deletions: 1, binary: false }],
};

afterEach(cleanup);
beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mocks.provider.mockResolvedValue({ provider: null, available: false, reason: "not_gitea" });
});

const mount = () => render(<BranchCompareSection cwd="/repo" root="/repo" head="bbbb" branch="feature" refreshToken={0} open onToggle={vi.fn()} />);

it("lists the branch's files and opens them as a range from the merge base", async () => {
  mocks.compare.mockResolvedValue(result);
  const view = mount();
  await act(async () => {});
  expect(mocks.compare).toHaveBeenCalledWith("/repo", null);
  expect(view.container.textContent).toContain("↑2 ↓1");
  fireEvent.click(view.container.querySelector('[data-git-path="src/a.ts"] .git-row-main')!);
  expect(mocks.open).toHaveBeenCalledWith("/repo", { path: "src/a.ts", commit: "bbbb", base: { from: "aaaa", label: "origin/main" } });
});

it("stays hidden when there is nothing to compare with", async () => {
  mocks.compare.mockResolvedValue(null);
  const view = mount();
  await act(async () => {});
  expect(view.container.innerHTML).toBe("");
});

it("remembers the chosen base and drops one that no longer exists", async () => {
  localStorage.setItem("vlx.git.compareBase:/repo", "gone");
  mocks.compare.mockImplementation(async (_cwd: string, base: string | null) => {
    if (base === "gone") throw new Error("Branch not found: gone");
    return result;
  });
  mount();
  await act(async () => {});
  expect(mocks.compare).toHaveBeenNthCalledWith(1, "/repo", "gone");
  expect(mocks.compare).toHaveBeenLastCalledWith("/repo", null);
  expect(screen.getByText("git.branchChanges")).toBeTruthy();
});

it("opens a Gitea pull request into the compared base and links to it", async () => {
  mocks.compare.mockResolvedValue(result);
  mocks.provider.mockResolvedValue({ provider: "gitea", available: true, reason: "ok" });
  mocks.openPr.mockResolvedValue({ url: "https://gitea.example/o/r/pulls/7", message: "Pull request opened." });
  mount();
  await act(async () => {});
  fireEvent.click(screen.getByRole("button", { name: "git.pr.open" }));
  const dialog = screen.getByRole("dialog");
  fireEvent.change(dialog.querySelector("input")!, { target: { value: "Add feature" } });
  await act(async () => { fireEvent.click(within(dialog).getByRole("button", { name: "git.pr.open" })); });
  expect(mocks.openPr).toHaveBeenCalledWith("/repo", "origin/main", "Add feature", "");
  fireEvent.click(screen.getByText("git.pr.opened"));
  expect(mocks.external).toHaveBeenCalledWith("https://gitea.example/o/r/pulls/7");
});

it("explains why a pull request cannot be opened", async () => {
  mocks.compare.mockResolvedValue(result);
  mocks.provider.mockResolvedValue({ provider: "gitea", available: false, reason: "no_token" });
  mount();
  await act(async () => {});
  expect((screen.getByRole("button", { name: "git.pr.open" }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText("git.pr.noToken")).toBeTruthy();
});
