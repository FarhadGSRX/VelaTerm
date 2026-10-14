import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChangesModal } from "./ChangesModal";

const mocks = vi.hoisted(() => ({ files: vi.fn(), diff: vi.fn(), close: vi.fn(),
  state: {} as Record<string, unknown> }));
vi.mock("../../ipc/commands", () => ({ gitChangedFiles: mocks.files, gitFileDiff: mocks.diff,
  gitCommitFiles: mocks.files, gitCommitFileDiff: mocks.diff }));
vi.mock("../../i18n", () => ({ useT: () => (key: string) => key }));
vi.mock("../../store/termStore", () => ({ useTermStore: (select: (s: unknown) => unknown) => select(mocks.state) }));
vi.mock("../../hooks/nativeViewSuspend", () => ({ useSuspendNativeViews: vi.fn() }));
// URL syncing has its own tests; here the dialog is driven through the store mock alone.
vi.mock("./changesRoute", () => ({ useChangesRoute: () => {}, replaceChangesPath: () => {} }));

const staged = { path: "中文文件.txt", status: "modified", index: "modified", worktree: "modified",
  additions: 3, deletions: 0, stagedAdditions: 1, stagedDeletions: 0, unstagedAdditions: 2, unstagedDeletions: 0 };
afterEach(cleanup);
beforeEach(() => {
  vi.clearAllMocks();
  mocks.state = { changesCwd: "/repo", changesCommit: null, changesPath: null, changesSide: "staged", closeChanges: mocks.close };
  mocks.files.mockResolvedValue({ files: [staged, { ...staged, path: "untracked.txt", index: "", worktree: "untracked" }], truncated: false });
  mocks.diff.mockResolvedValue({ path: staged.path, original: "", modified: "", binary: true });
});

it("shows only the requested area and sends its comparison to the backend", async () => {
  render(<ChangesModal />);
  await waitFor(() => expect(mocks.diff).toHaveBeenCalledWith("/repo", staged.path, "staged"));
  expect(screen.queryByText("untracked.txt")).toBeNull();
  expect(screen.getByText("changes.compareStaged")).toBeTruthy();
  expect(screen.getByText("+1")).toBeTruthy();
  expect(screen.getByRole("dialog").querySelector(".changes-file-path")?.textContent).toBe(staged.path);
});

it("supports file selection by keyboard and Escape dismissal", async () => {
  mocks.state.changesSide = "all";
  render(<ChangesModal />);
  await screen.findByText("untracked.txt");
  const row = screen.getByText("untracked.txt").closest("button")!;
  row.focus();
  expect(document.activeElement).toBe(row);
  fireEvent.click(row);
  await waitFor(() => expect(mocks.diff).toHaveBeenLastCalledWith("/repo", "untracked.txt", "all"));
  fireEvent.keyDown(row, { key: "Escape" });
  expect(mocks.close).toHaveBeenCalledTimes(1);
});

it("explains the text size limit without calling the file binary", async () => {
  mocks.diff.mockResolvedValue({ path: staged.path, original: "", modified: "", binary: false, tooLarge: true });
  render(<ChangesModal />);
  expect(await screen.findByText("changes.tooLarge")).toBeTruthy();
  expect(screen.queryByText("changes.binary")).toBeNull();
});
