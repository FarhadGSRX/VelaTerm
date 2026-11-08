import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import en from "../../i18n/locales/en";
import { useTermStore } from "../../store/termStore";
import type { AgentPreset, Session } from "../../types";
import { AgentPresetsSection } from "./AgentPresetsSection";

const api = vi.hoisted(() => ({ list: vi.fn(), remove: vi.fn() }));
vi.mock("../../ipc/presets", () => ({ listAgentPresets: api.list, deleteAgentPreset: api.remove }));
vi.mock("../../store/termStore", async () => {
  const { create } = await import("zustand");
  return { useTermStore: create(() => ({ agentPresets: [], sessions: [] })) };
});
vi.mock("../../i18n", () => ({ useT: () => (key: keyof typeof en, ...args: unknown[]) => {
  const value = en[key];
  return typeof value === "function" ? Reflect.apply(value, null, args) : value;
} }));
vi.mock("../agentPresetIcon", () => ({ PresetIcon: () => <svg /> }));

const preset = { id: "saved", name: "Custom Codex", baseKind: "codex", agentArgs: "--model custom" } as AgentPreset;
const session = { id: "session", kind: "codex", agentPresetId: preset.id, agentArgs: preset.agentArgs } as Session;
const button = () => screen.getByRole("button", { name: "Delete preset “Custom Codex”" });

beforeEach(() => {
  vi.clearAllMocks();
  useTermStore.setState({ agentPresets: [], sessions: [session] });
  api.list.mockResolvedValue([preset]);
  api.remove.mockResolvedValue(undefined);
});
afterEach(cleanup);

it("confirms deletion, updates the shared menu cache, and preserves existing sessions", async () => {
  render(<AgentPresetsSection />);
  await screen.findByText(preset.name);
  fireEvent.click(button());
  expect(api.remove).not.toHaveBeenCalled();
  expect(screen.getByText("Delete preset “Custom Codex”?")).toBeTruthy();
  fireEvent.click(button());
  await screen.findByText("No saved presets.");
  expect(api.remove).toHaveBeenCalledExactlyOnceWith(preset.id);
  expect(useTermStore.getState().agentPresets).toEqual([]);
  expect(useTermStore.getState().sessions).toEqual([session]);
});

it("cancels confirmation without deleting the preset", async () => {
  render(<AgentPresetsSection />);
  await screen.findByText(preset.name);
  fireEvent.click(button());
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.queryByText("Delete preset “Custom Codex”?")).toBeNull();
  expect(api.remove).not.toHaveBeenCalled();
  expect(useTermStore.getState().agentPresets).toEqual([preset]);
});

it("keeps a failed deletion visible and lets the user retry", async () => {
  api.remove.mockRejectedValueOnce(new Error("Preset deletion failed"));
  render(<AgentPresetsSection />);
  await screen.findByText(preset.name);
  fireEvent.click(button());
  fireEvent.click(button());
  expect((await screen.findByRole("alert")).textContent).toContain("Preset deletion failed");
  expect(useTermStore.getState().agentPresets).toEqual([preset]);
  await waitFor(() => expect(button().hasAttribute("disabled")).toBe(false));
  fireEvent.click(button());
  await screen.findByText("No saved presets.");
  expect(api.remove).toHaveBeenCalledTimes(2);
});

it("blocks repeated deletion while a request is in progress", async () => {
  let resolve!: () => void;
  api.remove.mockImplementationOnce(() => new Promise<void>(done => { resolve = done; }));
  render(<AgentPresetsSection />);
  await screen.findByText(preset.name);
  fireEvent.click(button());
  fireEvent.click(button());
  fireEvent.click(button());
  expect(api.remove).toHaveBeenCalledOnce();
  expect(button().hasAttribute("disabled")).toBe(true);
  expect(useTermStore.getState().agentPresets).toEqual([preset]);
  await act(async () => resolve());
  await screen.findByText("No saved presets.");
});

it("reports an unavailable list and retries without claiming there are no presets", async () => {
  api.list.mockRejectedValueOnce(new Error("Preset list unavailable"));
  render(<AgentPresetsSection />);
  expect((await screen.findByRole("alert")).textContent).toContain("Preset list unavailable");
  expect(screen.queryByText("No saved presets.")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await screen.findByText(preset.name);
  expect(screen.queryByRole("alert")).toBeNull();
});
