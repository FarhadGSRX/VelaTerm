//! The rules for a page's `compose`: which page may fill which message box, with what text, how often.

import { beforeEach, describe, expect, it, vi } from "vitest";

import { parseBridgeMessage } from "./artifactBridge";
import { COMPOSE_MAX, composeIntoDraft, composeRefusal, resetComposeHistory, runCompose } from "./artifactCompose";

const ok = { text: "/thoughts ok 3", panelSession: "s", beside: "s", now: 10_000 };

describe("composeRefusal", () => {
  it("lets a page in its own session's panel folder compose beside that session", () => {
    expect(composeRefusal(ok)).toBeNull();
    expect(composeRefusal({ ...ok, text: 'Re "item": ' })).toBeNull();
  });

  it("refuses a tab, a page outside any panel folder, and another session's page", () => {
    expect(composeRefusal({ ...ok, beside: null })).toBe("not-beside");
    expect(composeRefusal({ ...ok, panelSession: null })).toBe("not-allowed");
    expect(composeRefusal({ ...ok, panelSession: undefined })).toBe("not-allowed");
    expect(composeRefusal({ ...ok, panelSession: "other" })).toBe("other-session");
  });

  it("refuses empty and over-long text rather than trimming it", () => {
    expect(composeRefusal({ ...ok, text: "" })).toBe("empty");
    expect(composeRefusal({ ...ok, text: "   " })).toBe("empty");
    expect(composeRefusal({ ...ok, text: "x".repeat(COMPOSE_MAX) })).toBeNull();
    expect(composeRefusal({ ...ok, text: "x".repeat(COMPOSE_MAX + 1) })).toBe("too-long");
  });

  it("refuses every control character and line separator, so a paste can't act as Enter", () => {
    for (const c of ["\n", "\r", "\t", "\u0000", "\u001b", "\u007f", "\u0085", "\u009f", "\u2028", "\u2029"]) {
      expect(composeRefusal({ ...ok, text: `/thoughts ok 3${c}` })).toBe("control");
    }
  });

  it("allows one compose a second into a conversation", () => {
    expect(composeRefusal({ ...ok, last: ok.now - 999 })).toBe("too-soon");
    expect(composeRefusal({ ...ok, last: ok.now - 1000 })).toBeNull();
  });
});

describe("runCompose", () => {
  beforeEach(() => resetComposeHistory());

  it("hands passing text to the message box, and counts only what went in toward the rate limit", () => {
    const fill = vi.fn().mockReturnValueOnce("absent").mockReturnValue("inserted");
    expect(runCompose(ok, fill)).toBe("not-beside");
    // The compose that found no message box didn't start the one-second wait.
    expect(runCompose({ ...ok, now: ok.now + 10 }, fill)).toBeNull();
    expect(fill).toHaveBeenLastCalledWith("s", "/thoughts ok 3");
    expect(runCompose({ ...ok, now: ok.now + 500 }, fill)).toBe("too-soon");
    expect(runCompose({ ...ok, now: ok.now + 1010 }, fill)).toBeNull();
    // The wait is per conversation.
    expect(runCompose({ ...ok, panelSession: "t", beside: "t", now: ok.now + 1020 }, fill)).toBeNull();
  });

  it("never calls the message box for a refused compose, and reports a missing view as not beside", () => {
    const fill = vi.fn().mockReturnValue("absent");
    expect(runCompose({ ...ok, text: "a\nb" }, fill)).toBe("control");
    expect(fill).not.toHaveBeenCalled();
    expect(runCompose(ok, fill)).toBe("not-beside");
  });
});

describe("the message box and the bridge", () => {
  it("puts the line alone in an empty box, else on its own line after what is there", () => {
    expect(composeIntoDraft("", "/thoughts ok 3")).toBe("/thoughts ok 3");
    expect(composeIntoDraft("  \n", "/thoughts ok 3")).toBe("/thoughts ok 3");
    expect(composeIntoDraft("half a sentence", "/thoughts ok 3")).toBe("half a sentence\n/thoughts ok 3");
    expect(composeIntoDraft("/thoughts ok 3\n\n", "/thoughts no 4")).toBe("/thoughts ok 3\n/thoughts no 4");
  });

  it("parses compose with string text only, keeping one character past the limit so it is refused", () => {
    const msg = (text: unknown) => parseBridgeMessage({ source: "vlx-artifact", type: "compose", text });
    expect(msg("/thoughts ok 3")).toEqual({ type: "compose", text: "/thoughts ok 3" });
    expect(msg(3)).toBeNull();
    expect(msg(undefined)).toBeNull();
    const long = msg("x".repeat(5000));
    expect(long && long.type === "compose" && long.text.length).toBe(COMPOSE_MAX + 1);
    expect(parseBridgeMessage({ source: "elsewhere", type: "compose", text: "x" })).toBeNull();
  });
});
