import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Terminal } from "@xterm/xterm";
import { registerTerminal, unregisterTerminal } from "../../terminal/registry";
import { TermScrollbar } from "./TermScrollbar";

vi.mock("../../ipc/commands", () => ({ ptyRedraw: vi.fn() }));

const terminals: Terminal[] = [];
const rect = { x: 0, y: 0, top: 0, left: 0, right: 400, bottom: 300, width: 400, height: 300, toJSON() {} };
beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(rect);
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(300);
});
afterEach(() => {
  cleanup();
  terminals.splice(0).forEach((term) => { unregisterTerminal("scroll-test", term); term.dispose(); });
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function terminal() {
  const term = new Terminal({ allowProposedApi: true, cols: 40, rows: 10 });
  terminals.push(term);
  await new Promise<void>((resolve) => term.write(Array.from({ length: 50 }, (_, i) => `line ${i}\r\n`).join(""), resolve));
  return term;
}
function mount() {
  const containerRef = { current: document.createElement("div") };
  const view = render(<TermScrollbar sessionId="scroll-test" containerRef={containerRef} hidden={false} />);
  const track = () => view.container.querySelector<HTMLElement>("[data-terminal-scrollbar]");
  return { ...view, containerRef, track };
}

it("binds after delayed terminal creation and scrolls through the public API", async () => {
  const view = mount();
  expect(view.track()).toBeNull();
  const term = await terminal();
  act(() => registerTerminal("scroll-test", term));
  await waitFor(() => expect(view.track()).not.toBeNull());
  const intent = vi.fn();
  view.containerRef.current.addEventListener("vlx-terminal-user-scroll", intent);
  fireEvent.click(view.track()!, { clientY: 0 });
  expect(term.buffer.active.viewportY).toBe(0);
  fireEvent.click(view.track()!, { clientY: 300 });
  expect(term.buffer.active.viewportY).toBe(term.buffer.active.baseY);
  expect(intent).toHaveBeenCalledTimes(2);
});

it("updates the thumb from scroll events and retains the pointer's grab offset", async () => {
  const term = await terminal();
  registerTerminal("scroll-test", term);
  const view = mount();
  await waitFor(() => expect(view.track()).not.toBeNull());
  act(() => term.scrollToTop());
  await waitFor(() => expect(view.track()!.firstElementChild).toHaveProperty("style.top", "0px"));
  const thumb = view.track()!.firstElementChild as HTMLElement;
  fireEvent.mouseDown(thumb, { clientY: 10 });
  fireEvent.mouseMove(document, { clientY: 10 });
  expect(term.buffer.active.viewportY).toBe(0);
  fireEvent.mouseMove(document, { clientY: 300 });
  expect(term.buffer.active.viewportY).toBe(term.buffer.active.baseY);
  fireEvent.mouseUp(document);
});

it("sizes the first thumb from the unscaled parent in a mirror pane", async () => {
  const term = await terminal();
  registerTerminal("scroll-test", term);
  // Mirror transforms shrink the terminal's visible rectangle but do not shrink the sibling track.
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ ...rect, height: 60 });
  const view = mount();
  await waitFor(() => expect(view.track()).not.toBeNull());
  const thumb = view.track()!.firstElementChild as HTMLElement;
  expect(Number.parseFloat(thumb.style.height)).toBeCloseTo(294 * term.rows / (term.buffer.active.baseY + term.rows));
});

it("rebinds after restart and ignores disposal of the previous terminal", async () => {
  const first = await terminal();
  registerTerminal("scroll-test", first);
  const view = mount();
  await waitFor(() => expect(view.track()).not.toBeNull());
  const second = await terminal();
  act(() => { registerTerminal("scroll-test", second); unregisterTerminal("scroll-test", first); });
  fireEvent.click(view.track()!, { clientY: 0 });
  expect(second.buffer.active.viewportY).toBe(0);
  expect(first.buffer.active.viewportY).toBe(first.buffer.active.baseY);
  act(() => unregisterTerminal("scroll-test", second));
  await waitFor(() => expect(view.track()).toBeNull());
});

it("hides for alternate screens and releases an active drag on unmount", async () => {
  const term = await terminal();
  registerTerminal("scroll-test", term);
  const view = mount();
  await waitFor(() => expect(view.track()).not.toBeNull());
  await act(async () => new Promise<void>((resolve) => term.write("\x1b[?1049h", resolve)));
  await waitFor(() => expect(view.track()).toBeNull());
  await act(async () => new Promise<void>((resolve) => term.write("\x1b[?1049l", resolve)));
  await waitFor(() => expect(view.track()).not.toBeNull());
  fireEvent.mouseDown(view.track()!.firstElementChild!, { clientY: 260 });
  view.unmount();
  const before = term.buffer.active.viewportY;
  fireEvent.mouseMove(document, { clientY: 0 });
  expect(term.buffer.active.viewportY).toBe(before);
});
