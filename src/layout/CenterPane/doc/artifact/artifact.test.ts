//! Pure logic behind artifact previews: message validation, viewport fitting, change filtering, prefs.

import { beforeEach, describe, expect, it } from "vitest";

import { docKindOf, makeDocTab } from "../../../../store/docTab";
import { artifactPaths } from "../../session/toolCards";
import { consoleReport, MAX_CONSOLE, parseBridgeMessage, pushConsole, type ConsoleEntry } from "./artifactBridge";
import { isNetAllowed, setNetAllowed } from "./artifactPrefs";
import { frameLayout, viewportReadout } from "./artifactViewport";
import { changeAffectsPage } from "./useArtifact";

describe("parseBridgeMessage", () => {
  it("accepts the injected script's messages", () => {
    expect(parseBridgeMessage({ source: "vlx-artifact", type: "ready" })).toEqual({ type: "ready" });
    expect(parseBridgeMessage({ source: "vlx-artifact", type: "console", level: "warn", text: "w" })).toEqual({ type: "console", level: "warn", text: "w" });
    expect(parseBridgeMessage({ source: "vlx-artifact", type: "error", text: "boom", line: 42 })).toEqual({ type: "error", text: "boom", line: 42 });
    expect(parseBridgeMessage({ source: "vlx-artifact", type: "scroll", y: 120.7 })).toEqual({ type: "scroll", y: 120 });
  });

  it("rejects other sources, unknown types and malformed fields", () => {
    expect(parseBridgeMessage({ type: "ready" })).toBeNull();
    expect(parseBridgeMessage({ source: "other", type: "ready" })).toBeNull();
    expect(parseBridgeMessage({ source: "vlx-artifact", type: "open", url: "https://x" })).toBeNull();
    expect(parseBridgeMessage({ source: "vlx-artifact", type: "console", level: "info", text: "x" })).toBeNull();
    expect(parseBridgeMessage({ source: "vlx-artifact", type: "scroll", y: -1 })).toBeNull();
    expect(parseBridgeMessage({ source: "vlx-artifact", type: "scroll", y: "10" })).toBeNull();
    expect(parseBridgeMessage("vlx-artifact")).toBeNull();
    expect(parseBridgeMessage(null)).toBeNull();
  });

  it("clips oversized text a page might post itself", () => {
    const msg = parseBridgeMessage({ source: "vlx-artifact", type: "error", text: "x".repeat(10_000) });
    expect(msg?.type === "error" && msg.text.length).toBe(2000);
  });
});

describe("console", () => {
  it("keeps the newest entries", () => {
    let list: ConsoleEntry[] = [];
    for (let i = 0; i < MAX_CONSOLE + 5; i++) list = pushConsole(list, { id: i, level: "error", text: String(i) });
    expect(list).toHaveLength(MAX_CONSOLE);
    expect(list[0].id).toBe(5);
    expect(list.at(-1)!.id).toBe(MAX_CONSOLE + 4);
  });

  it("reports only errors to the agent", () => {
    const report = consoleReport("/p/a.html", [
      { id: 1, level: "warn", text: "slow" },
      { id: 2, level: "error", text: "TypeError: x", line: 9 },
    ]);
    expect(report).toContain("/p/a.html");
    expect(report).toContain("- TypeError: x (line 9)");
    expect(report).not.toContain("slow");
  });
});

describe("frameLayout", () => {
  it("fills the area for fit and scales devices down, never up", () => {
    expect(frameLayout("fit", { width: 800, height: 600 })).toEqual({ width: 800, height: 600, scale: 1 });
    const tablet = frameLayout("tablet", { width: 600, height: 600 });
    expect(tablet.width).toBe(834);
    expect(tablet.scale).toBeCloseTo((600 - 32) / 1112);
    expect(frameLayout("phone", { width: 2000, height: 2000 }).scale).toBe(1);
    expect(viewportReadout("phone", { width: 390, height: 844, scale: 0.5 })).toBe("390 \u00d7 844 \u00b7 50%");
    expect(viewportReadout("fit", { width: 1, height: 1, scale: 1 })).toBeNull();
  });
});

describe("changeAffectsPage", () => {
  it("reloads for the page's folder and shallow asset folders, not build or dependency churn", () => {
    expect(changeAffectsPage(null)).toBe(true);
    expect(changeAffectsPage([""])).toBe(true);
    expect(changeAffectsPage(["css"])).toBe(true);
    expect(changeAffectsPage(["assets/img"])).toBe(true);
    expect(changeAffectsPage(["node_modules/react"])).toBe(false);
    expect(changeAffectsPage([".git"])).toBe(false);
    expect(changeAffectsPage(["target/debug"])).toBe(false);
    expect(changeAffectsPage(["a/b/c"])).toBe(false);
  });
});

describe("artifactPaths", () => {
  it("finds HTML pages written by Claude Code and Codex calls", () => {
    expect(artifactPaths("Write", { file_path: "/p/index.html" })).toEqual(["/p/index.html"]);
    expect(artifactPaths("Edit", { file_path: "/p/main.rs" })).toEqual([]);
    expect(artifactPaths("FileChange", { changes: [{ path: "/p/a.HTM" }, { path: "/p/b.css" }] })).toEqual(["/p/a.HTM"]);
    expect(artifactPaths("Diff", { diff: "+++ b/site/page.html\n+++ b/x.ts\n" }, "/repo/")).toEqual(["/repo/site/page.html"]);
    expect(artifactPaths("Diff", { diff: "+++ b/site/page.html\n" })).toEqual([]);
    expect(artifactPaths("Write", { file_path: "C:\\p\\index.html" })).toEqual(["C:\\p\\index.html"]);
    expect(artifactPaths("Read", { file_path: "/p/index.html" })).toEqual([]);
  });
});

describe("doc kind", () => {
  it("opens HTML pages as artifacts in preview mode", () => {
    expect(docKindOf("/p/index.html")).toBe("artifact");
    expect(docKindOf("/p/Design.dc.html")).toBe("artifact");
    expect(docKindOf("/p/old.HTM")).toBe("artifact");
    expect(docKindOf("/p/page.xhtml")).toBe("code");
    expect(makeDocTab("/p/index.html").mode).toBe("preview");
  });
});

describe("network preference", () => {
  beforeEach(() => localStorage.clear());

  it("is remembered per file", () => {
    expect(isNetAllowed("/a.html")).toBe(false);
    setNetAllowed("/a.html", true);
    expect(isNetAllowed("/a.html")).toBe(true);
    expect(isNetAllowed("/b.html")).toBe(false);
    setNetAllowed("/a.html", false);
    expect(isNetAllowed("/a.html")).toBe(false);
  });
});
