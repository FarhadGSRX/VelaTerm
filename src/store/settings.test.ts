//! Coverage for the per-agent default view: the built-in fallback, the saved per-agent choice, and the
//! migration from the retired app-wide setting. Every chat-capable agent shares the conversation fallback;
//! saved per-agent choices can still override it.

import { beforeEach, describe, expect, it, vi } from "vitest";
import { SETTINGS_KEY, defaultEngineFor, loadSettings, saveSettings } from "./settings";

vi.mock("../ipc/settingsSync", () => ({ pushSetting: vi.fn() }));

describe("terminal renderer migration", () => {
  beforeEach(() => localStorage.clear());

  it.each(["canvas", "invalid", null])("opens legacy renderer %s with DOM", (termRenderer) => {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify({ termRenderer, gpuRender: true, termFontSize: 18 }));
    const settings = loadSettings();
    expect(settings.termRenderer).toBe("dom");
    expect(settings.termFontSize).toBe(18);
  });

  it.each(["dom", "webgl"])("preserves a supported %s preference", (termRenderer) => {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify({ termRenderer, gpuRender: false }));
    expect(loadSettings().termRenderer).toBe(termRenderer);
  });

  it.each([true, false])("preserves the older GPU preference %s when no renderer was saved", (gpuRender) => {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify({ gpuRender }));
    expect(loadSettings().termRenderer).toBe(gpuRender ? "webgl" : "dom");
  });
});

describe("defaultEngineFor", () => {
  beforeEach(() => localStorage.clear());

  it("opens every chat-capable agent in the conversation", () => {
    expect(defaultEngineFor("claude", {})).toBe("chat");
    expect(defaultEngineFor("codex", {})).toBe("chat");
    expect(defaultEngineFor("opencode", {})).toBe("chat");
    expect(defaultEngineFor("pi", {})).toBe("chat");
    expect(defaultEngineFor("omp", {})).toBe("chat");
  });

  it("prefers the saved per-agent choice over the built-in default", () => {
    expect(defaultEngineFor("omp", { omp: { engine: "tui" } })).toBe("tui");
    expect(defaultEngineFor("pi", { pi: { engine: "tui" } })).toBe("tui");
  });
});

describe("planExecutePrefs sanitization", () => {
  beforeEach(() => localStorage.clear());

  it("keeps only well-formed role choices", () => {
    localStorage.setItem(
      SETTINGS_KEY,
      JSON.stringify({ planExecutePrefs: { plan: { agent: "codex", model: 7, effort: "high" }, exec: "nonsense" } }),
    );
    expect(loadSettings().planExecutePrefs).toEqual({ plan: { agent: "codex", effort: "high" }, exec: {} });
  });

  it("defaults to empty roles when nothing was saved", () => {
    expect(loadSettings().planExecutePrefs).toEqual({ plan: {}, exec: {} });
  });
});

describe("memoryPrefs sanitization", () => {
  beforeEach(() => localStorage.clear());

  it("keeps only a well-formed agent, model and effort choice", () => {
    localStorage.setItem(
      SETTINGS_KEY,
      JSON.stringify({ memoryPrefs: { agent: "codex", model: 7, effort: "high" } }),
    );
    expect(loadSettings().memoryPrefs).toEqual({ agent: "codex", effort: "high" });
  });

  it("defaults to an empty choice when nothing was saved", () => {
    expect(loadSettings().memoryPrefs).toEqual({});
  });
});

describe("reference summary settings", () => {
  beforeEach(() => localStorage.clear());

  it("keeps pre-summary disabled by default", () => {
    expect(loadSettings().referSummary).toEqual({
      enabled: false,
      agent: "claude",
      model: "",
      effort: "",
    });
  });

  it("sanitizes the one global Agent, model, and effort selection", () => {
    localStorage.setItem(
      SETTINGS_KEY,
      JSON.stringify({ referSummary: { enabled: true, agent: "codex", model: 7, effort: "high" } }),
    );
    expect(loadSettings().referSummary).toEqual({
      enabled: true,
      agent: "codex",
      model: "",
      effort: "high",
    });
  });
});

describe("legacy defaultSessionEngine migration", () => {
  beforeEach(() => localStorage.clear());

  it("folds the retired app-wide choice into Claude, Codex and OpenCode", () => {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify({ defaultSessionEngine: "tui" }));
    const s = loadSettings();
    expect(s.agentDefaults.claude?.engine).toBe("tui");
    expect(s.agentDefaults.codex?.engine).toBe("tui");
    expect(s.agentDefaults.opencode?.engine).toBe("tui");
    // Pi and OMP were never covered by the app-wide setting, so the unified fallback still applies.
    expect(s.agentDefaults.pi?.engine).toBeUndefined();
    expect(defaultEngineFor("pi", s.agentDefaults)).toBe("chat");
    expect(defaultEngineFor("omp", s.agentDefaults)).toBe("chat");
  });

  it("keeps an explicit per-agent choice while migrating the rest", () => {
    localStorage.setItem(
      SETTINGS_KEY,
      JSON.stringify({
        defaultSessionEngine: "tui",
        agentDefaults: { claude: { engine: "chat", args: "--model opus" } },
      }),
    );
    const s = loadSettings();
    expect(s.agentDefaults.claude).toEqual({ engine: "chat", args: "--model opus" });
    expect(s.agentDefaults.codex?.engine).toBe("tui");
  });

  it("does not leak migrated values into the built-in defaults", () => {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify({ defaultSessionEngine: "tui" }));
    loadSettings();
    localStorage.clear();
    expect(loadSettings().agentDefaults).toEqual({});
  });
});

// fg: the persisted settings-modal section must round-trip and fall back for blobs written before it existed,
// and image paste defaults to the agent-native path rather than upload.
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
