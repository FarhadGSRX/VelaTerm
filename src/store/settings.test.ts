//! Coverage for the two settings added alongside the persisted settings-modal section: `settingsTab` must
//! round-trip and must fall back for blobs written before it existed, and image paste now defaults to native.

import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/settingsSync", () => ({ pushSetting: vi.fn() }));

import { loadSettings, saveSettings, SETTINGS_KEY } from "./settings";

beforeEach(() => {
  localStorage.clear();
});

describe("settingsTab persistence", () => {
  it("defaults to appearance with nothing stored", () => {
    expect(loadSettings().settingsTab).toBe("appearance");
  });

  it("falls back to appearance for a blob written before the field existed", () => {
    // A real pre-upgrade payload: valid settings, no settingsTab key.
    localStorage.setItem(SETTINGS_KEY, JSON.stringify({ accent: "blue", inspectorTab: "git" }));
    const s = loadSettings();
    expect(s.settingsTab).toBe("appearance");
    expect(s.accent).toBe("blue"); // merge must not clobber what was stored
  });

  it("round-trips a chosen section", () => {
    saveSettings({ ...loadSettings(), settingsTab: "behavior" });
    expect(loadSettings().settingsTab).toBe("behavior");
  });
});

describe("image paste default", () => {
  it("defaults to native", () => {
    expect(loadSettings().imagePasteMode).toBe("agent");
  });

  it("still honours an explicit stored upload preference", () => {
    saveSettings({ ...loadSettings(), imagePasteMode: "upload" });
    expect(loadSettings().imagePasteMode).toBe("upload");
  });
});
