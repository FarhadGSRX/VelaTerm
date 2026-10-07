import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useTermStore } from "../../store/termStore";
import { RunStrip } from "./RunStrip";

vi.mock("../../ipc/runs", () => ({
  runStop: vi.fn(() => Promise.resolve()),
  runLogTail: vi.fn(() => Promise.resolve({ text: "compiled\nall tests passed", running: false, exitCode: 0 })),
}));

import { runLogTail, runStop } from "../../ipc/runs";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  useTermStore.setState({ backgroundRuns: {}, notifications: {}, runtimes: {} });
});

function publish(runs: { label: string; command: string; startedAt: number }[]) {
  act(() => useTermStore.getState().applySessionStates({ s: { runs } }));
}

it("lists the session's running commands from its record and disappears when they end", () => {
  const { container } = render(<RunStrip sessionId="s" />);
  expect(container.querySelector(".run-strip")).toBeNull();

  publish([{ label: "build", command: "cargo build --release", startedAt: Date.now() - 65_000 }]);
  expect(screen.getByText("build")).toBeTruthy();
  expect(screen.getByText("Running for 1:05")).toBeTruthy();
  expect(screen.getByText("cargo build --release")).toBeTruthy();

  publish([]);
  expect(container.querySelector(".run-strip")).toBeNull();
  expect(useTermStore.getState().backgroundRuns.s).toBeUndefined();
});

it("stops a command only on a confirming second click", async () => {
  render(<RunStrip sessionId="s" />);
  publish([{ label: "build", command: "make", startedAt: Date.now() }]);

  fireEvent.click(screen.getByRole("button", { name: "Stop" }));
  expect(runStop).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Confirm stop" }));
  await waitFor(() => expect(runStop).toHaveBeenCalledWith("build"));
});

it("opens the log and keeps it open with the outcome after the command ends", async () => {
  render(<RunStrip sessionId="s" />);
  publish([{ label: "build", command: "make", startedAt: Date.now() }]);

  fireEvent.click(screen.getByRole("button", { name: "Log" }));
  expect(await screen.findByText("Finished with exit code 0")).toBeTruthy();
  expect(screen.getByRole("dialog", { name: "Log: build" }).textContent).toContain("all tests passed");
  expect(runLogTail).toHaveBeenCalledWith("build");

  publish([]);
  expect(screen.getByRole("dialog", { name: "Log: build" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("shows a log's colours instead of raw escape codes", async () => {
  vi.mocked(runLogTail).mockResolvedValueOnce({ text: "\x1b[1;32m✓ ok\x1b[0m\nplain", running: false, exitCode: 0 });
  render(<RunStrip sessionId="s" />);
  publish([{ label: "build", command: "make", startedAt: Date.now() }]);
  fireEvent.click(screen.getByText("Log"));
  const ok = await screen.findByText("✓ ok", { exact: false });
  expect(ok.tagName).toBe("SPAN");
  expect(ok.style.color).toBe("var(--green)");
  expect(document.querySelector(".run-log-text")?.textContent).toBe("✓ ok\nplain");
});
