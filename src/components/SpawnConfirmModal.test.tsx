//! Child-session review preserves launch context and edits across asynchronous capability loading.
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SpawnRequest } from "../ipc/events";
import type { PlanExecuteRolePrefs } from "../store/settings";

const mocks = vi.hoisted(() => ({
  launchOptions: vi.fn(),
  launchModels: vi.fn(),
  launchSelection: vi.fn(),
  planExecuteDefaults: vi.fn(),
  preparePlanExecute: vi.fn(),
  createPlanExecute: vi.fn(),
  applyLaunchArgs: vi.fn(),
  nativeImage: vi.fn(),
  env: { isTauri: false },
  store: {
    pendingSpawns: [] as SpawnRequest[],
    sessions: [] as {
      id: string;
      name: string;
      kind: string;
      agentArgs: string | null;
      cwd?: string;
    }[],
    projects: [],
    agentDefaults: {},
    planExecutePrefs: { plan: {}, exec: {} } as {
      plan: PlanExecuteRolePrefs;
      exec: PlanExecuteRolePrefs;
    },
    setPlanExecuteRolePrefs: vi.fn(),
    confirmSpawn: vi.fn(),
    cancelSpawn: vi.fn(),
    loadTree: vi.fn(),
    openSession: vi.fn(),
  },
}));
vi.mock("../platform/env", () => ({ env: mocks.env }));
vi.mock("../terminal/imageInput", async (original) => ({
  ...await original<typeof import("../terminal/imageInput")>(),
  imageFromNativeClipboard: mocks.nativeImage,
}));
vi.mock("../i18n", () => ({ useT: () => (key: string) => key }));
vi.mock("../hooks/nativeViewSuspend", () => ({
  useSuspendNativeViews: () => {},
}));
vi.mock("../ipc/commands", () => ({
  agentListModels: vi.fn().mockResolvedValue(["opus", "sonnet"]),
}));
vi.mock("../ipc/launch", () => ({
  launchOptions: mocks.launchOptions,
  launchModels: mocks.launchModels,
  launchSelection: mocks.launchSelection,
  planExecuteDefaults: mocks.planExecuteDefaults,
  preparePlanExecute: mocks.preparePlanExecute,
  createPlanExecute: mocks.createPlanExecute,
  applyLaunchArgs: mocks.applyLaunchArgs,
}));
vi.mock("../store/termStore", () => ({
  useTermStore: Object.assign((selector: (s: typeof mocks.store) => unknown) =>
    selector(mocks.store), { getState: () => mocks.store }),
}));
import { SpawnConfirmModal } from "./SpawnConfirmModal";

beforeEach(() => {
  vi.clearAllMocks();
  mocks.launchModels.mockReset().mockResolvedValue({ models: [
    { id: "opus", label: "opus", effortLevels: ["low", "high"] },
    { id: "sonnet", label: "sonnet", effortLevels: ["low", "high"] },
    { id: "gpt-5.6-sol", label: "gpt-5.6-sol", effortLevels: ["low", "medium", "high", "xhigh", "max", "ultra"] },
  ], effortLevels: ["low", "medium", "high", "xhigh", "max", "ultra"] });
  mocks.env.isTauri = false;
  mocks.nativeImage.mockReset();
  window.history.replaceState(null, "", "/");
  mocks.store.pendingSpawns = [];
  mocks.store.loadTree.mockResolvedValue(undefined);
  mocks.launchOptions.mockResolvedValue([
    {
      supportsPlanExecute: true,
      id: "claude",
      label: "Claude",
      acceptsTask: true,
      effortFlag: "--effort",
      effortLevels: ["low", "high"],
    },
    {
      supportsPlanExecute: true,
      id: "codex",
      label: "Codex",
      acceptsTask: true,
      effortFlag: "model_reasoning_effort",
      effortLevels: ["low", "medium", "high", "xhigh", "max", "ultra"],
    },
    {
      supportsPlanExecute: false,
      id: "terminal",
      label: "Terminal",
      acceptsTask: false,
      effortFlag: null,
      effortLevels: [],
    },
  ]);
  mocks.launchSelection.mockResolvedValue({ model: "sonnet", effort: "low" });
  mocks.applyLaunchArgs.mockResolvedValue(null);
  mocks.store.confirmSpawn.mockResolvedValue(undefined);
  mocks.store.agentDefaults = {};
  mocks.store.planExecutePrefs = { plan: {}, exec: {} };
});
afterEach(() => { cleanup(); window.history.replaceState(null, "", "/"); });
async function open(req: Partial<SpawnRequest> = {}, waitForOptions = true) {
  mocks.store.sessions = [
    {
      id: "p1",
      name: "Parent session",
      kind: "claude",
      agentArgs: "--model sonnet",
      cwd: "/repo",
    },
  ];
  mocks.store.pendingSpawns = [
    { parentSessionId: "p1", prompt: "Investigate the parser", ...req },
  ];
  render(<SpawnConfirmModal />);
  if (waitForOptions) await screen.findByLabelText("spawn.modelLabel");
}
async function launch() {
  await waitFor(() => expect((screen.getByRole("button", { name: "spawn.launch" }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole("button", { name: "spawn.launch" }));
  await waitFor(() => expect(mocks.store.confirmSpawn).toHaveBeenCalledOnce());
  return mocks.store.confirmSpawn.mock.calls[0][0] as SpawnRequest;
}
describe("single child-session review", () => {
  it("preserves task, invocation directory and explicitly requested choices", async () => {
    await open({
      cwd: "/other repo",
      model: "opus",
      effort: "high",
      worktree: false,
    });
    expect(screen.getByText("/other repo")).toBeTruthy();
    expect(
      (screen.getByLabelText("spawn.modelLabel") as HTMLInputElement).value,
    ).toBe("opus");
    expect(await launch()).toMatchObject({
      cwd: "/other repo",
      model: "opus",
      effort: "high",
      worktree: false,
    });
  });
  it("uses backend-resolved inherited settings and can explicitly clear a model", async () => {
    await open();
    expect(mocks.launchSelection).toHaveBeenCalledWith(
      "claude",
      "--model sonnet",
    );
    fireEvent.change(screen.getByLabelText("spawn.modelLabel"), {
      target: { value: "" },
    });
    expect((await launch()).model).toBe("");
  });
  it("supports Codex effort without silently dropping the request", async () => {
    await open({ kind: "codex", model: "gpt-5.5", effort: "xhigh" });
    expect((await launch()).effort).toBe("xhigh");
  });
  it("keeps plain terminal launches available without agent-only controls", async () => {
    await open({ kind: "terminal" }, false);
    await screen.findByText("launch.terminalHint");
    await waitFor(() =>
      expect(
        (
          screen.getByRole("button", {
            name: "spawn.launch",
          }) as HTMLButtonElement
        ).disabled,
      ).toBe(false),
    );
    expect(screen.queryByLabelText("spawn.promptLabel")).toBeNull();
    expect(screen.queryByLabelText("spawn.modelLabel")).toBeNull();
    expect(await launch()).toMatchObject({
      kind: "terminal",
      model: null,
      effort: null,
    });
  });
  it("does not submit an empty task", async () => {
    await open();
    fireEvent.change(screen.getByLabelText("spawn.promptLabel"), {
      target: { value: " \n " },
    });
    expect(
      (
        screen.getByRole("button", {
          name: "spawn.launch",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
  });
  it("keeps the request reviewable when backend validation rejects a choice", async () => {
    await open();
    mocks.applyLaunchArgs.mockRejectedValueOnce(
      new Error("invalid identifier"),
    );
    fireEvent.click(screen.getByRole("button", { name: "spawn.launch" }));
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "launch.startError invalid identifier",
    );
    expect(mocks.store.confirmSpawn).not.toHaveBeenCalled();
  });
  it("blocks launch while the capability service is unavailable and allows retry", async () => {
    mocks.launchOptions.mockRejectedValueOnce(new Error("offline"));
    await open({}, false);
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(
      (
        screen.getByRole("button", {
          name: "spawn.launch",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "common.retry" }));
    await screen.findByLabelText("spawn.modelLabel");
    await launch();
  });
});

it.each(["none", "shared", "each"] as const)("keeps both roles editable and submits the %s worktree mode", async mode => {
  const config = {
    plan: { agent: "codex" as const, model: "planner", effort: "high" },
    exec: { agent: "codex" as const, model: "executor", effort: "medium" },
  };
  mocks.planExecuteDefaults.mockResolvedValue(config);
  await open({ planExecute: config }, false);
  const planner = within(await screen.findByRole("region", { name: "launch.planTitle" }));
  const executor = within(screen.getByRole("region", { name: "launch.execTitle" }));
  expect(screen.getAllByRole("radio")).toHaveLength(3);
  const labels = { none: "orch.worktreeNone", shared: "orch.worktreeShared", each: "orch.worktreeEach" };
  expect((screen.getByRole("radio", { name: /orch.worktreeShared/ }) as HTMLInputElement).checked).toBe(true);
  fireEvent.click(screen.getByRole("radio", { name: new RegExp(labels[mode]) }));
  expect((planner.getByLabelText("spawn.modelLabel") as HTMLInputElement).value).toBe("planner");
  expect((executor.getByLabelText("spawn.effortLabel") as HTMLInputElement).value).toBe("medium");
  fireEvent.change(planner.getByLabelText("spawn.modelLabel"), { target: { value: "final-planner" } });
  fireEvent.change(planner.getByLabelText("spawn.effortLabel"), { target: { value: "xhigh" } });
  fireEvent.change(executor.getByLabelText("spawn.modelLabel"), { target: { value: "final-executor" } });
  fireEvent.change(executor.getByLabelText("spawn.effortLabel"), { target: { value: "low" } });
  const submitted = await launch();
  expect(submitted.worktree).toBe(mode !== "none");
  expect(submitted.planExecute).toEqual({
    worktreeMode: mode,
    plan: { agent: "codex", model: "final-planner", effort: "xhigh" },
    exec: { agent: "codex", model: "final-executor", effort: "low" },
  });
});

it("does not launch a workflow using invented settings when defaults cannot be loaded", async () => {
  mocks.planExecuteDefaults.mockRejectedValue(new Error("offline"));
  await open({ planExecute: { plan: {}, exec: {} } }, false);
  await screen.findByRole("alert");
  expect((screen.getByRole("button", { name: "spawn.launch" }) as HTMLButtonElement).disabled).toBe(true);
  expect(mocks.store.confirmSpawn).not.toHaveBeenCalled();
});

it("ignores obsolete terminal role preferences and lists only workflow-capable agents", async () => {
  mocks.store.planExecutePrefs = { plan: { agent: "terminal", model: "obsolete", effort: "obsolete" }, exec: {} };
  const config = { plan: { agent: "codex" as const, model: "planner" }, exec: { agent: "claude" as const } };
  mocks.planExecuteDefaults.mockResolvedValue(config);
  await open({ planExecute: { plan: {}, exec: {} } }, false);
  const planner = within(await screen.findByRole("region", { name: "launch.planTitle" }));
  fireEvent.click(planner.getByLabelText("spawn.agentLabel"));
  expect(screen.getAllByRole("option").map(option => option.textContent)).toEqual(["Claude", "Codex"]);
  fireEvent.keyDown(planner.getByLabelText("spawn.agentLabel"), { key: "Escape" });
  expect((await launch()).planExecute?.plan).toEqual(config.plan);
});

it("disables workflow launch for a stale unsupported role until the user selects an available agent", async () => {
  const config = { plan: { agent: "terminal" as const }, exec: { agent: "codex" as const } };
  mocks.planExecuteDefaults.mockResolvedValue(config);
  await open({ planExecute: config }, false);
  const planner = within(await screen.findByRole("region", { name: "launch.planTitle" }));
  expect((screen.getByRole("button", { name: "spawn.launch" }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(planner.getByLabelText("spawn.agentLabel"));
  fireEvent.click(screen.getByRole("option", { name: "Claude" }));
  expect((await launch()).planExecute?.plan.agent).toBe("claude");
});

it("resets only the selected role when its agent changes", async () => {
  const config = { plan: { agent: "codex" as const, model: "planner", effort: "high" }, exec: { agent: "codex" as const, model: "executor", effort: "medium" } };
  mocks.planExecuteDefaults.mockResolvedValue(config);
  await open({ planExecute: config }, false);
  const planner = within(await screen.findByRole("region", { name: "launch.planTitle" }));
  fireEvent.click(planner.getByLabelText("spawn.agentLabel"));
  fireEvent.click(await screen.findByRole("option", { name: "Claude" }));
  expect((await launch()).planExecute).toEqual({
    worktreeMode: "shared",
    plan: { agent: "claude", model: "", effort: "" },
    exec: config.exec,
  });
  expect(mocks.store.setPlanExecuteRolePrefs).toHaveBeenCalledWith("plan", { agent: "claude", model: null, effort: null });
});

it("opens the workflow on the choices remembered from the previous one", async () => {
  mocks.store.planExecutePrefs = {
    plan: { agent: "claude", model: "opus", effort: "high" },
    exec: {},
  };
  mocks.planExecuteDefaults.mockResolvedValue({
    plan: { agent: "codex", model: "planner", effort: "medium" },
    exec: { agent: "codex", model: "executor", effort: "medium" },
  });
  await open({ planExecute: { plan: {}, exec: {} } }, false);
  const planner = within(await screen.findByRole("region", { name: "launch.planTitle" }));
  const executor = within(screen.getByRole("region", { name: "launch.execTitle" }));
  expect((planner.getByLabelText("spawn.agentLabel") as HTMLInputElement).value).toBe("Claude");
  expect((planner.getByLabelText("spawn.modelLabel") as HTMLInputElement).value).toBe("opus");
  expect((planner.getByLabelText("spawn.effortLabel") as HTMLInputElement).value).toBe("high");
  expect((executor.getByLabelText("spawn.agentLabel") as HTMLInputElement).value).toBe("Codex");
  expect((executor.getByLabelText("spawn.modelLabel") as HTMLInputElement).value).toBe("executor");
});

it("lets an explicit request value win over the remembered choice", async () => {
  mocks.store.planExecutePrefs = { plan: { agent: "claude", model: "opus", effort: "high" }, exec: {} };
  const config = { plan: { agent: "codex" as const, model: "requested", effort: "low" }, exec: { agent: "codex" as const, model: "executor", effort: "medium" } };
  mocks.planExecuteDefaults.mockResolvedValue(config);
  await open({ planExecute: config }, false);
  const planner = within(await screen.findByRole("region", { name: "launch.planTitle" }));
  expect((planner.getByLabelText("spawn.agentLabel") as HTMLInputElement).value).toBe("Codex");
  expect((planner.getByLabelText("spawn.modelLabel") as HTMLInputElement).value).toBe("requested");
  expect((planner.getByLabelText("spawn.effortLabel") as HTMLInputElement).value).toBe("low");
});


describe("new planning workflow from the session menu", () => {
  const context = { projectId: "p", groupId: "g", parentSessionId: "parent" };
  const config = { plan: { agent: "codex", model: "planner", effort: "high" }, exec: { agent: "codex", model: "executor", effort: "medium" } };
  const route = "/?planExecute=creation-id&planProject=p&planGroup=g&planParent=parent";
  beforeEach(() => {
    window.history.replaceState(null, "", route);
    mocks.preparePlanExecute.mockResolvedValue({ config, cwd: "/repo", worktree: false, locationNames: ["Project", "Group", "Parent"] });
    mocks.createPlanExecute.mockResolvedValue({ planner: { id: "new-planner" }, run: { state: "planning" } });
  });
  it.each(["none", "shared", "each"] as const)("loads backend defaults and creates both roles with %s worktree mode", async mode => {
    render(<SpawnConfirmModal />);
    const planner = within(await screen.findByRole("region", { name: "launch.planTitle" }));
    const executor = within(screen.getByRole("region", { name: "launch.execTitle" }));
    expect(mocks.preparePlanExecute).toHaveBeenCalledWith(context);
    expect(mocks.planExecuteDefaults).not.toHaveBeenCalled();
    expect(mocks.createPlanExecute).not.toHaveBeenCalled();
    expect(screen.getByText("Project / Group / Parent")).toBeTruthy();
    expect((screen.getByRole("button", { name: "launch.createAndStart" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("spawn.promptLabel"), { target: { value: "Implement the requested feature" } });
    fireEvent.change(screen.getByLabelText("launch.workingDirectory"), { target: { value: "/repo/selected" } });
    fireEvent.change(planner.getByLabelText("spawn.modelLabel"), { target: { value: "review-model" } });
    fireEvent.change(executor.getByLabelText("spawn.effortLabel"), { target: { value: "low" } });
    expect(screen.getAllByRole("radio")).toHaveLength(3);
    const labels = { none: "orch.worktreeNone", shared: "orch.worktreeShared", each: "orch.worktreeEach" };
    expect((screen.getByRole("radio", { name: /orch.worktreeNone/ }) as HTMLInputElement).checked).toBe(true);
    fireEvent.click(screen.getByRole("radio", { name: new RegExp(labels[mode]) }));
    fireEvent.click(screen.getByRole("button", { name: "launch.createAndStart" }));
    await waitFor(() => expect(mocks.store.openSession).toHaveBeenCalledWith("new-planner"));
    expect(mocks.createPlanExecute).toHaveBeenCalledWith({ requestId: "creation-id", context, prompt: "Implement the requested feature", cwd: "/repo/selected", worktree: mode !== "none",
      config: { worktreeMode: mode, plan: { ...config.plan, model: "review-model" }, exec: { ...config.exec, effort: "low" } } });
    expect(window.location.search).toBe("");
    expect(mocks.store.confirmSpawn).not.toHaveBeenCalled();
  });
  it("cancels without touching queued requests and responds to browser navigation", async () => {
    render(<SpawnConfirmModal />);
    await screen.findByRole("region", { name: "launch.planTitle" });
    fireEvent.change(screen.getByLabelText("spawn.promptLabel"), { target: { value: "Keep my draft" } });
    act(() => { window.history.replaceState(null, "", route + "&session=another"); window.dispatchEvent(new PopStateEvent("popstate")); });
    expect((screen.getByLabelText("spawn.promptLabel") as HTMLTextAreaElement).value).toBe("Keep my draft");
    fireEvent.click(screen.getByRole("button", { name: "common.cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(mocks.store.cancelSpawn).not.toHaveBeenCalled();
    expect(mocks.createPlanExecute).not.toHaveBeenCalled();
    act(() => { window.history.replaceState(null, "", route); window.dispatchEvent(new PopStateEvent("popstate")); });
    await screen.findByRole("region", { name: "launch.planTitle" });
    expect(screen.getByRole("dialog")).toBeTruthy();
  });
  it("retains the same request and editable prompt after a failed launch", async () => {
    mocks.createPlanExecute.mockResolvedValueOnce({ planner: { id: "retained-planner" }, run: { state: "blocked", summary: "Agent unavailable" } });
    render(<SpawnConfirmModal />);
    await screen.findByRole("region", { name: "launch.planTitle" });
    fireEvent.change(screen.getByLabelText("spawn.promptLabel"), { target: { value: "Task" } });
    pasteImage(screen.getByLabelText("spawn.promptLabel"));
    await screen.findByAltText("screenshot.png");
    fireEvent.click(screen.getByRole("button", { name: "launch.createAndStart" }));
    expect((await screen.findByRole("alert")).textContent).toContain("Agent unavailable");
    expect(screen.getByAltText("screenshot.png")).toBeTruthy();
    expect(new URLSearchParams(window.location.search).get("planExecute")).toBe("creation-id");
    fireEvent.click(screen.getByRole("button", { name: "launch.createAndStart" }));
    await waitFor(() => expect(mocks.store.openSession).toHaveBeenCalledWith("new-planner"));
    expect(mocks.createPlanExecute.mock.calls[0]).toEqual(mocks.createPlanExecute.mock.calls[1]);
  });
  it("blocks creation when placement cannot be resolved", async () => {
    mocks.preparePlanExecute.mockRejectedValueOnce(new Error("Project removed"));
    render(<SpawnConfirmModal />);
    await screen.findByRole("alert");
    fireEvent.change(screen.getByLabelText("spawn.promptLabel"), { target: { value: "Task" } });
    expect((screen.getByRole("button", { name: "launch.createAndStart" }) as HTMLButtonElement).disabled).toBe(true);
    expect(mocks.createPlanExecute).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "common.retry" }));
    await screen.findByRole("region", { name: "launch.planTitle" });
    expect((screen.getByRole("button", { name: "launch.createAndStart" }) as HTMLButtonElement).disabled).toBe(false);
  });
});


function pasteImage(input: HTMLElement, name = "screenshot.png") {
  const file = new File([new Uint8Array([1, 2, 3])], name, { type: "image/png" });
  fireEvent.paste(input, { clipboardData: { items: [{ kind: "file", type: file.type, getAsFile: () => file }], getData: () => "" } });
}

it("renders invalid caller model values for correction and gives the executor's validation error", async () => {
  const config = { plan: { agent: "codex" as const, model: "6-astrak", effort: "xhigh" }, exec: { agent: "codex" as const, model: "gpt 5.6 sol", effort: "xhigh" } };
  mocks.planExecuteDefaults.mockResolvedValue(config);
  mocks.applyLaunchArgs.mockImplementation(async (_kind, _args, model) => {
    if (model === "gpt 5.6 sol") throw new Error("Model and effort must be identifiers without spaces or shell operators");
    return null;
  });
  await open({ planExecute: config }, false);
  const executor = within(await screen.findByRole("region", { name: "launch.execTitle" }));
  expect((executor.getByLabelText("spawn.modelLabel") as HTMLInputElement).value).toBe("gpt 5.6 sol");
  pasteImage(screen.getByLabelText("spawn.promptLabel"));
  await screen.findByAltText("screenshot.png");
  fireEvent.click(screen.getByRole("button", { name: "spawn.launch" }));
  expect((await screen.findByRole("alert")).textContent).toContain("launch.execTitle: Model and effort must be identifiers");
  expect(mocks.store.confirmSpawn).not.toHaveBeenCalled();
  expect(screen.getByAltText("screenshot.png")).toBeTruthy();
  fireEvent.change(executor.getByLabelText("spawn.modelLabel"), { target: { value: "gpt-5.6-sol" } });
  expect((await launch()).images).toEqual([{ mimeType: "image/png", data: "AQID" }]);
});

it("keeps loading failures specific and preserves the prompt while retrying", async () => {
  mocks.planExecuteDefaults.mockRejectedValueOnce(new Error("The workflow session is missing or archived"));
  mocks.planExecuteDefaults.mockResolvedValue({ plan: { agent: "codex" }, exec: { agent: "codex" } });
  await open({ planExecute: { plan: {}, exec: {} } }, false);
  expect((await screen.findByRole("alert")).textContent).toContain("The workflow session is missing or archived");
  fireEvent.change(screen.getByLabelText("spawn.promptLabel"), { target: { value: "Keep this edited task" } });
  pasteImage(screen.getByLabelText("spawn.promptLabel"));
  await screen.findByAltText("screenshot.png");
  fireEvent.click(screen.getByRole("button", { name: "common.retry" }));
  await screen.findByRole("region", { name: "launch.execTitle" });
  expect((await launch()).prompt).toBe("Keep this edited task");
});

it("serializes consecutive image pastes, supports removal, and carries images into ordinary spawn requests", async () => {
  await open();
  const input = screen.getByLabelText("spawn.promptLabel");
  pasteImage(input, "first.png"); pasteImage(input, "second.png");
  await screen.findByAltText("second.png");
  expect(screen.getByAltText("first.png")).toBeTruthy();
  fireEvent.click(screen.getAllByRole("button", { name: "chat.attach.remove" })[0]);
  expect(screen.queryByAltText("first.png")).toBeNull();
  expect((await launch()).images).toEqual([{ mimeType: "image/png", data: "AQID" }]);
});

it("uses the native image clipboard for an empty WebView paste without intercepting plain text", async () => {
  mocks.env.isTauri = true;
  mocks.nativeImage.mockResolvedValue(new File([new Uint8Array([4, 5, 6])], "native.png", { type: "image/png" }));
  await open();
  const input = screen.getByLabelText("spawn.promptLabel");
  fireEvent.paste(input, { clipboardData: { items: [], getData: () => "plain text" } });
  expect(mocks.nativeImage).not.toHaveBeenCalled();
  fireEvent.paste(input, { clipboardData: { items: [], getData: () => "" } });
  await screen.findByAltText("native.png");
  expect((await launch()).images).toEqual([{ mimeType: "image/png", data: "BAUG" }]);
});

it("does not attach an image whose read finishes after the request was cancelled", async () => {
  mocks.env.isTauri = true;
  let finish!: (file: File) => void;
  mocks.nativeImage.mockReturnValue(new Promise<File>(resolve => { finish = resolve; }));
  await open();
  fireEvent.paste(screen.getByLabelText("spawn.promptLabel"), { clipboardData: { items: [], getData: () => "" } });
  await waitFor(() => expect(mocks.nativeImage).toHaveBeenCalled());
  cleanup();
  await act(async () => { finish(new File(["image"], "cancelled.png", { type: "image/png" })); });
  await open({ requestId: "next-request" });
  expect(screen.queryByAltText("cancelled.png")).toBeNull();
});


it("keeps the error visible when a failed workflow is returned to the queue", async () => {
  const config = { plan: { agent: "codex" as const }, exec: { agent: "codex" as const } };
  const req = { requestId: "retry-visible", parentSessionId: "p1", prompt: "Task", planExecute: config };
  mocks.planExecuteDefaults.mockResolvedValue(config);
  mocks.store.pendingSpawns = [req];
  const view = render(<SpawnConfirmModal />);
  await screen.findByRole("region", { name: "launch.execTitle" });
  pasteImage(screen.getByLabelText("spawn.promptLabel"));
  await screen.findByAltText("screenshot.png");
  mocks.store.confirmSpawn.mockImplementationOnce(async (submitted) => {
    mocks.store.pendingSpawns = [];
    view.rerender(<SpawnConfirmModal />);
    await new Promise(resolve => setTimeout(resolve, 0));
    mocks.store.pendingSpawns = [submitted];
    view.rerender(<SpawnConfirmModal />);
    throw new Error("Agent executable not found");
  });
  fireEvent.click(screen.getByRole("button", { name: "spawn.launch" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Agent executable not found");
  await screen.findByAltText("image-1.png");
});


it.each(["spawn", "menu"])("selects independent Codex effort levels from both %s workflow dropdowns", async entry => {
  const config = {
    plan: { agent: "codex" as const, model: "", effort: "" },
    exec: { agent: "codex" as const, model: "gpt-5.6-sol", effort: "" },
  };
  if (entry === "menu") {
    window.history.replaceState(null, "", "/?planExecute=effort-choice&planProject=p");
    mocks.preparePlanExecute.mockResolvedValue({ config, cwd: "/repo", worktree: false, locationNames: ["Project"] });
    mocks.createPlanExecute.mockResolvedValue({ planner: { id: "planner" }, run: { state: "planning" } });
    render(<SpawnConfirmModal />);
    await screen.findByRole("region", { name: "launch.planTitle" });
    fireEvent.change(screen.getByLabelText("spawn.promptLabel"), { target: { value: "Task" } });
  } else {
    mocks.planExecuteDefaults.mockResolvedValue(config);
    await open({ planExecute: config }, false);
    await screen.findByRole("region", { name: "launch.planTitle" });
  }
  const planner = within(screen.getByRole("region", { name: "launch.planTitle" }));
  const executor = within(screen.getByRole("region", { name: "launch.execTitle" }));
  const choose = async (field: HTMLElement, level: string) => {
    fireEvent.click(field);
    await screen.findByRole("option", { name: "ultra" });
    expect(screen.getAllByRole("option").map(option => option.textContent)).toEqual([
      "spawn.modelDefault", "low", "medium", "high", "xhigh", "max", "ultra",
    ]);
    fireEvent.click(screen.getByRole("option", { name: level }));
    expect((field as HTMLInputElement).value).toBe(level);
  };
  await choose(planner.getByLabelText("spawn.effortLabel"), "xhigh");
  await choose(executor.getByLabelText("spawn.effortLabel"), "high");
  expect((planner.getByLabelText("spawn.modelLabel") as HTMLInputElement).value).toBe("");
  expect((executor.getByLabelText("spawn.modelLabel") as HTMLInputElement).value).toBe("gpt-5.6-sol");
  expect(mocks.store.setPlanExecuteRolePrefs).toHaveBeenCalledWith("plan", { effort: "xhigh" });
  expect(mocks.store.setPlanExecuteRolePrefs).toHaveBeenCalledWith("exec", { effort: "high" });
  // Returning one role to its native default must leave the other role's explicit choice intact.
  fireEvent.click(planner.getByLabelText("spawn.effortLabel"));
  fireEvent.click(screen.getByRole("option", { name: "spawn.modelDefault" }));
  expect(mocks.store.setPlanExecuteRolePrefs).toHaveBeenCalledWith("plan", { effort: "" });
  const expected = { worktreeMode: entry === "menu" ? "none" : "shared", plan: config.plan, exec: { ...config.exec, effort: "high" } };
  if (entry === "menu") {
    fireEvent.click(screen.getByRole("button", { name: "launch.createAndStart" }));
    await waitFor(() => expect(mocks.createPlanExecute).toHaveBeenCalledWith(expect.objectContaining({ config: expected })));
  } else {
    expect((await launch()).planExecute).toEqual(expected);
  }
});

it("carries automatic splitting from the command request and lets the user turn it off", async () => {
  const config = { splitTasks: true, plan: { agent: "claude" as const }, exec: { agent: "codex" as const } };
  mocks.planExecuteDefaults.mockResolvedValue(config);
  await open({ planExecute: config }, false);
  const toggle = await screen.findByRole("checkbox", { name: /launch.splitTasks/ });
  expect((toggle as HTMLInputElement).checked).toBe(true);
  fireEvent.click(toggle);
  expect((await launch()).planExecute?.splitTasks).toBe(false);
});

it("offers automatic task splitting in a new planning and execution session", async () => {
  window.history.replaceState(null, "", "/?planExecute=create-split&planProject=project");
  mocks.preparePlanExecute.mockResolvedValue({ config: { plan: { agent: "claude" }, exec: { agent: "codex" } }, cwd: "/repo", worktree: false, locationNames: ["Project"] });
  mocks.createPlanExecute.mockResolvedValue({ planner: { id: "planner" }, run: { id: "flow", state: "planning", summary: "" } });
  render(<SpawnConfirmModal />);
  fireEvent.change(await screen.findByLabelText("spawn.promptLabel"), { target: { value: "Split this task" } });
  fireEvent.click(await screen.findByRole("checkbox", { name: /launch.splitTasks/ }));
  fireEvent.click(screen.getByRole("button", { name: "launch.createAndStart" }));
  await waitFor(() => expect(mocks.createPlanExecute).toHaveBeenCalledOnce());
  expect(mocks.createPlanExecute.mock.calls[0][0].config.splitTasks).toBe(true);
});
