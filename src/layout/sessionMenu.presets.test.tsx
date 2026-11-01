import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentPreset, Session } from "../types";
import type { MenuItem } from "../components/ContextMenu";

const mocks = vi.hoisted(() => ({
  setSessionPreset: vi.fn(), updateSession: vi.fn(), addSession: vi.fn(), t: (key: string) => key,
}));
const state = vi.hoisted(() => new Proxy({
  sessions: [] as Session[], projects: [], groups: [], agentPresets: [] as AgentPreset[], agentDefaults: {},
  runtimes: {}, paneTrees: {}, sidebarTreeViews: [], ephemeralSessions: {}, openTabs: [], liveTabs: [],
  sessionTitlePrefs: {}, addSession: mocks.addSession, updateSession: mocks.updateSession,
}, { get: (target, key) => key in target ? target[key as keyof typeof target] : vi.fn() }));
vi.mock("../store/termStore", () => ({
  useTermStore: Object.assign((select: (s: typeof state) => unknown) => select(state), { getState: () => state }),
}));
vi.mock("../ipc/presets", async importOriginal => ({
  ...await importOriginal<object>(), setSessionPreset: mocks.setSessionPreset,
}));
vi.mock("../ipc/commands", async importOriginal => ({
  ...await importOriginal<object>(), listShells: vi.fn().mockResolvedValue([]), gitbashStatus: vi.fn().mockResolvedValue(null),
}));
vi.mock("../hooks/useGitBranch", () => ({
  isWorktreeGone: () => false, peekGitBranchInfo: () => null, prefetchGitBranchInfo: vi.fn(), invalidateGitBranch: vi.fn(),
}));
vi.mock("../ipc/events", async importOriginal => ({
  ...await importOriginal<object>(), onGitbashDownloadDone: () => Promise.resolve(() => {}),
}));
vi.mock("../i18n", () => ({ t: mocks.t, useT: () => mocks.t }));

import { useSessionMenu } from "./sessionMenu";

function session(agentPresetId: string | null = null, kind: Session["kind"] = "claude"): Session {
  return { id: "session", projectId: "project", groupId: null, name: "velaterm-lead", kind,
    agentSessionId: "native-id", shell: null, cwd: null, envJson: null, initCmd: null, hotkey: null,
    parentSessionId: null, collapsed: false, worktreePath: null, sortOrder: 0, createdAt: 0, agentPresetId };
}
function preset(id: string, source: string | null, extra: Partial<AgentPreset> = {}): AgentPreset {
  return { id, name: id, baseKind: "claude", agentArgs: "--model opus", permissionMode: "auto", sortOrder: 0,
    createdAt: 0, source, ...extra };
}
const node = { kind: "session" as const, id: "session", name: "velaterm-lead", projectId: "project", groupId: null };

function openEdit() {
  const { result } = renderHook(useSessionMenu);
  const edit = result.current.buildSessionItems(node).find(item => item.label === "common.edit");
  act(() => edit?.onClick?.({} as React.MouseEvent));
  render(<>{result.current.dialogs}</>);
}
const presetSelect = () => screen.queryByRole("combobox", { name: "tree.presetLink" });
function pick(label: string) {
  fireEvent.click(presetSelect()!);
  fireEvent.click(screen.getByRole("option", { name: label }));
}

beforeEach(() => {
  state.agentPresets = [preset("f-co/velaterm-lead", "file", { cwd: "C:/work" }), preset("mine", null)];
  state.sessions = [session()];
  mocks.setSessionPreset.mockReset().mockResolvedValue("f-co/velaterm-lead");
  mocks.updateSession.mockReset().mockResolvedValue(undefined);
  mocks.addSession.mockReset().mockResolvedValue(null);
});
afterEach(cleanup);

describe("presets from presets.json", () => {
  it("Edit Session links a Claude session to a file preset", async () => {
    openEdit();
    const select = presetSelect();
    expect(select).not.toBeNull();
    expect(select!.textContent).toBe("tree.presetNone");
    fireEvent.click(select!);
    expect(screen.getAllByRole("option").map(o => o.textContent)).toEqual(["tree.presetNone", "f-co/velaterm-lead"]);
    fireEvent.click(screen.getByRole("option", { name: "f-co/velaterm-lead" }));
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));
    await waitFor(() => expect(mocks.setSessionPreset).toHaveBeenCalledWith("session", "f-co/velaterm-lead"));
    expect(mocks.updateSession).toHaveBeenCalled();
  });

  it("Edit Session unlinks, and leaves an unchanged link alone", async () => {
    state.sessions = [session("f-co/velaterm-lead")];
    openEdit();
    expect(presetSelect()!.textContent).toBe("f-co/velaterm-lead");
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));
    await waitFor(() => expect(mocks.updateSession).toHaveBeenCalled());
    expect(mocks.setSessionPreset).not.toHaveBeenCalled();
    cleanup();
    openEdit();
    pick("tree.presetNone");
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));
    await waitFor(() => expect(mocks.setSessionPreset).toHaveBeenCalledWith("session", null));
  });

  it("shows no picker without file presets or for other agents", () => {
    state.agentPresets = [preset("mine", null)];
    openEdit();
    expect(presetSelect()).toBeNull();
    cleanup();
    state.agentPresets = [preset("f-co/velaterm-lead", "file")];
    state.sessions = [session(null, "codex")];
    openEdit();
    expect(presetSelect()).toBeNull();
  });

  it("the new-session menu badges file presets and starts their sessions in the preset's folder", async () => {
    const { result } = renderHook(useSessionMenu);
    const items = result.current.newSessionItems("project", null);
    const agents = items.find(item => item.submenu?.some(sub => sub.label === "agentPicker.title"));
    const presets = (agents?.submenu ?? []).filter((item: MenuItem) => item.label === "tree.newAgentSession" && item.icon
      && (item.shortcut === "presets.json" || item.shortcut === undefined));
    const file = (agents?.submenu ?? []).find((item: MenuItem) => item.shortcut === "presets.json");
    expect(file).toBeDefined();
    expect(presets.length).toBeGreaterThan(1);
    act(() => file?.onClick?.({} as React.MouseEvent));
    await waitFor(() => expect(mocks.addSession).toHaveBeenCalled());
    expect(mocks.addSession.mock.calls[0][0]).toMatchObject({
      name: "f-co/velaterm-lead", agentPresetId: "f-co/velaterm-lead", agentArgs: "--model opus", cwd: "C:/work",
    });
  });
});
