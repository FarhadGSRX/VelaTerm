//! Coverage for the terminal image-paste default.

import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/settingsSync", () => ({ pushSetting: vi.fn() }));

import { loadSettings, saveSettings } from "./settings";

beforeEach(() => {
  localStorage.clear();
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
