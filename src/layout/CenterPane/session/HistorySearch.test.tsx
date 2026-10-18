import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
vi.mock("../../../ipc/chat", async original => ({ ...await original<typeof import("../../../ipc/chat")>(), chatSearch: vi.fn(), chatReadCancel: vi.fn() }));
import { chatReadCancel, chatSearch } from "../../../ipc/chat";
import { HistorySearch } from "./HistorySearch";

beforeEach(() => {
  window.history.replaceState(null, "", "/");
  vi.mocked(chatSearch).mockReset();
  vi.mocked(chatReadCancel).mockReset().mockResolvedValue();
});
afterEach(cleanup);

it("does not request any history for an empty search", async () => {
  render(<HistorySearch sessionId="s" onLocate={vi.fn()} onClose={vi.fn()} />);
  await new Promise(resolve => setTimeout(resolve, 250));
  expect(chatSearch).not.toHaveBeenCalled();
});

it("reads snippets and cancels the read job when the search closes", async () => {
  vi.mocked(chatSearch).mockResolvedValue({ hits: [{ rowId: "r", snippet: "a needle", offset: 2 }], jobId: "j", coverage: "complete", nextCursor: null, scannedBytes: 10 });
  const locate = vi.fn().mockResolvedValue(undefined);
  const { unmount } = render(<HistorySearch sessionId="s" onLocate={locate} onClose={vi.fn()} />);
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "needle" } });
  await screen.findByText("a needle");
  expect(chatSearch).toHaveBeenCalledWith("s", "needle", undefined);
  fireEvent.click(screen.getByText("a needle"));
  await waitFor(() => expect(locate).toHaveBeenCalledWith({ rowId: "r", snippet: "a needle", offset: 2 }, "needle"));
  unmount();
  expect(chatReadCancel).toHaveBeenCalledWith("s", "j");
});

it("continues bounded empty pages and preserves an incomplete coverage result", async () => {
  vi.mocked(chatSearch).mockResolvedValueOnce({ hits: [], jobId: "j", coverage: "partial", nextCursor: "j.1", scannedBytes: 100 })
    .mockResolvedValueOnce({ hits: [], jobId: "j", coverage: "partial", nextCursor: null, scannedBytes: 200 });
  render(<HistorySearch sessionId="s" onLocate={vi.fn()} onClose={vi.fn()} />);
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "needle" } });
  await screen.findByText("History is still being indexed");
  expect(chatSearch).toHaveBeenCalledTimes(2);
  expect(chatSearch).toHaveBeenLastCalledWith("s", "needle", "j.1");
  expect(screen.queryByText("No results")).toBeNull();
});
