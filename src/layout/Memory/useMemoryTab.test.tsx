// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { useTermStore } from "../../store/termStore";
import { useMemoryTab } from "./useMemoryTab";
import { memoryNavigate, memoryUrl } from "./navigation";

vi.mock("../../debug", () => ({ DEBUG: false, dlog: vi.fn(), checkTabInvariants: vi.fn() }));

function Host() { useMemoryTab(); return null; }
beforeEach(() => {
  vi.useFakeTimers();
  window.history.replaceState(null, "", "/");
  useTermStore.setState({ openTabs: ["a", "b"], activeTabId: "a", activeSessionId: "a", focusedPaneId: null, selection: [{ id: "a", kind: "session" }] });
});
afterEach(() => { cleanup(); vi.clearAllTimers(); vi.useRealTimers(); });

it("restores the previous session when closing memory", () => {
  render(<Host />);
  act(() => memoryNavigate(memoryUrl()));
  expect(useTermStore.getState().activeTabId).toBeNull();
  act(() => memoryNavigate(memoryUrl("")));
  expect(useTermStore.getState().activeTabId).toBe("a");
});

it("switches to another session with the store selection subscription enabled", () => {
  render(<Host />);
  act(() => memoryNavigate(memoryUrl()));
  act(() => useTermStore.getState().setActiveTab("b"));
  expect(location.search).toBe("");
  expect(useTermStore.getState().activeTabId).toBe("b");
});
