import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { diagnosticEvent, store } = vi.hoisted(() => ({
  diagnosticEvent: vi.fn(),
  store: { inputLatencyThresholdMs: 50 },
}));
vi.mock("../../../ipc/transport", () => ({ diagnosticEvent }));
vi.mock("../../../store/termStore", () => ({ useTermStore: { getState: () => store } }));

import { attachInputLatencyLog } from "./inputLatency";

let now = 0;
let input: HTMLTextAreaElement;
let detach: () => void;

function keydown(key: string, at: number, init: KeyboardEventInit = {}) {
  now = at;
  input.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, ...init }));
}
function text(at: number, type: "input" | "compositionupdate" = "input") {
  now = at;
  input.dispatchEvent(new Event(type, { bubbles: true }));
}

beforeEach(() => {
  now = 0;
  vi.spyOn(performance, "now").mockImplementation(() => now);
  // The frame after the text paints immediately in these tests, so render time is zero.
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => { cb(now); return 0; });
  store.inputLatencyThresholdMs = 50;
  input = document.createElement("textarea");
  document.body.appendChild(input);
  detach = attachInputLatencyLog(() => input, "session-1");
});

afterEach(() => {
  detach();
  input.remove();
  diagnosticEvent.mockReset();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("composer input latency log", () => {
  it("logs a keystroke whose text arrives later than the threshold, without the typed text", () => {
    keydown("~", 1000);
    text(1120);
    expect(diagnosticEvent).toHaveBeenCalledTimes(1);
    const [event, fields] = diagnosticEvent.mock.calls[0];
    expect(event).toBe("input_latency");
    expect(fields).toMatchObject({ sessionId: "session-1", keyKind: "symbol", keyToTextMs: 120, renderMs: 0, totalMs: 120, thresholdMs: 50 });
    expect(JSON.stringify(fields)).not.toContain("~");
  });

  it("ignores keystrokes faster than the threshold", () => {
    keydown("a", 1000);
    text(1010);
    expect(diagnosticEvent).not.toHaveBeenCalled();
  });

  it("follows the configured threshold", () => {
    store.inputLatencyThresholdMs = 200;
    keydown("a", 1000);
    text(1120);
    expect(diagnosticEvent).not.toHaveBeenCalled();
  });

  it("measures a symbol held back until the next key from its own keypress", () => {
    keydown("!", 1000);
    keydown("a", 1250);
    text(1252);
    expect(diagnosticEvent).toHaveBeenCalledTimes(1);
    expect(diagnosticEvent.mock.calls[0][1]).toMatchObject({ keyKind: "symbol", keyToTextMs: 252 });
  });

  it("skips the late keydown WebKit sends after a punctuation mark's text", () => {
    text(1000);
    keydown("！", 1002, { keyCode: 229 });
    keydown("a", 1250);
    text(1255);
    expect(diagnosticEvent).not.toHaveBeenCalled();
  });

  it("treats IME composition updates as the text arriving", () => {
    keydown("q", 1000, { keyCode: 229 });
    text(1090, "compositionupdate");
    expect(diagnosticEvent.mock.calls[0][1]).toMatchObject({ keyKind: "composition", keyToTextMs: 90 });
  });

  it("ignores shortcuts, keys without text and other fields", () => {
    keydown("a", 1000, { metaKey: true });
    keydown("Shift", 1000);
    text(1200);
    const other = document.createElement("textarea");
    document.body.appendChild(other);
    now = 2000;
    other.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }));
    now = 2300;
    other.dispatchEvent(new Event("input", { bubbles: true }));
    other.remove();
    expect(diagnosticEvent).not.toHaveBeenCalled();
  });

  it("drops a keydown that never produced text", () => {
    keydown("a", 1000);
    text(7000);
    expect(diagnosticEvent).not.toHaveBeenCalled();
  });
});
