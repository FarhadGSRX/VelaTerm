import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CommitInfo } from "../../../ipc/commands";
import { CommitHistory } from "./CommitHistory";

const mocks = vi.hoisted(() => ({ log: vi.fn(), files: vi.fn(), open: vi.fn() }));
vi.mock("../../../ipc/commands", () => ({ gitLogPage: mocks.log, gitCommitFiles: mocks.files }));
vi.mock("../../../i18n", () => ({ useT: () => (key: string) => key }));
vi.mock("../../../store/termStore", () => ({ useTermStore: (select: (s: unknown) => unknown) => select({ openChanges: mocks.open }) }));

function commits(count: number): CommitInfo[] {
  return Array.from({ length: count }, (_, i) => ({
    oid: String(i + 1).padStart(40, "0"), hash: String(i + 1), subject: `Commit ${i + 1}`,
    message: `Commit ${i + 1}\n\nComplete multiline body\nSecond line`, author: "Example", relative: "today",
  }));
}

afterEach(cleanup);
beforeEach(() => {
  vi.resetAllMocks();
  mocks.files.mockResolvedValue([{ path: "example.txt", status: "modified", additions: 1, deletions: 0 }]);
});

describe("CommitHistory", () => {
  it("keeps rows and expansion during a HEAD refresh and includes the full message", async () => {
    const rows = commits(1);
    mocks.log.mockResolvedValueOnce(rows);
    const view = render(<CommitHistory cwd="/repo" total={1} head="first" />);
    await screen.findByText("Commit 1");
    const button = view.container.querySelector<HTMLButtonElement>(".git-commit-row")!;
    expect(button.title).toContain("Complete multiline body\nSecond line");
    fireEvent.click(button);
    await screen.findByText("example.txt");
    let resolve!: (rows: CommitInfo[]) => void;
    mocks.log.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    view.rerender(<CommitHistory cwd="/repo" total={1} head="amended" />);
    expect(screen.getByText("example.txt")).toBeTruthy();
    expect(view.container.querySelector(".git-commit-row")?.getAttribute("aria-expanded")).toBe("true");
    await act(async () => { resolve(rows); });
    expect(screen.getByText("example.txt")).toBeTruthy();
    expect(mocks.files).toHaveBeenCalledTimes(1);
    view.rerender(<CommitHistory cwd="/repo" total={1} head="amended" />);
    expect(mocks.log).toHaveBeenCalledTimes(2);
  });

  it("retains the loaded range and scroll anchor when a new commit is prepended", async () => {
    const rows = commits(75);
    const prepended = [{ ...commits(1)[0], oid: "a".repeat(40), hash: "new", subject: "New commit" }, ...rows];
    mocks.log.mockImplementation(async (_cwd, limit, offset, head) => (head === "new" ? prepended : rows).slice(offset, offset + limit));
    const view = render(<div data-git-scroll><CommitHistory cwd="/repo" total={75} head="old" /></div>);
    await screen.findByText("Commit 30");
    fireEvent.click(screen.getByText("git.loadMore"));
    await screen.findByText("Commit 60");
    fireEvent.click(screen.getByText("Commit 60"));
    await screen.findByText("example.txt");
    const parent = view.container.firstElementChild as HTMLElement;
    parent.scrollTop = 600;
    const rect = (top: number, bottom: number) => ({ top, bottom, left: 0, right: 300, width: 300, height: bottom - top, x: 0, y: top, toJSON() {} });
    parent.getBoundingClientRect = () => rect(0, 300);
    const geometry = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      const index = Array.from(parent.querySelectorAll("[data-commit-oid]")).indexOf(this);
      return rect(index * 20 - parent.scrollTop, (index + 1) * 20 - parent.scrollTop);
    });
    view.rerender(<div data-git-scroll><CommitHistory cwd="/repo" total={76} head="new" /></div>);
    await screen.findByText("New commit");
    expect(view.container.querySelectorAll(".git-commit-item")).toHaveLength(61);
    expect(screen.getByText("Commit 60")).toBeTruthy();
    expect(screen.getByText("example.txt")).toBeTruthy();
    expect(parent.scrollTop).toBe(620);
    expect(new Set(Array.from(view.container.querySelectorAll<HTMLElement>("[data-commit-oid]")).map((row) => row.dataset.commitOid)).size).toBe(61);
    geometry.mockRestore();
  });

  it("ignores a late response from the previous repository", async () => {
    let resolve!: (rows: CommitInfo[]) => void;
    mocks.log.mockImplementationOnce(() => new Promise((done) => { resolve = done; })).mockResolvedValueOnce([{ ...commits(1)[0], subject: "Other repository" }]);
    const view = render(<CommitHistory cwd="/old" total={1} head="old" />);
    view.rerender(<CommitHistory cwd="/new" total={1} head="new" />);
    await screen.findByText("Other repository");
    await act(async () => { resolve(commits(1)); });
    expect(screen.queryByText("Commit 1")).toBeNull();
    expect(screen.getByText("Other repository")).toBeTruthy();
  });

  it("keeps readable history when a refresh fails", async () => {
    mocks.log.mockResolvedValueOnce(commits(1)).mockRejectedValueOnce(new Error("Refresh unavailable"));
    const view = render(<CommitHistory cwd="/repo" total={1} head="old" />);
    await screen.findByText("Commit 1");
    fireEvent.click(screen.getByText("Commit 1"));
    await screen.findByText("example.txt");
    view.rerender(<CommitHistory cwd="/repo" total={1} head="new" />);
    await waitFor(() => expect(screen.getByRole("status").textContent).toContain("Refresh unavailable"));
    expect(screen.getByText("example.txt")).toBeTruthy();
  });
});
