//! Regression coverage for the spawn confirmation claim.
//!
//! The confirmation card is shown on every connected client, so the same request can be answered twice
//! within the same second — one person on a desktop and a phone, or two people. The backend hands the
//! request to exactly one answer; a client that loses must drop its card and do nothing, or the task runs
//! twice with two worktrees and two agents.

import { beforeEach, describe, expect, it, vi } from "vitest";

const resolveSpawn = vi.hoisted(() => vi.fn());
const uploadImage = vi.hoisted(() => vi.fn());
const startPlanExecute = vi.hoisted(() => vi.fn());
vi.mock("../ipc/launch", async importOriginal => ({ ...await importOriginal<typeof import("../ipc/launch")>(), startPlanExecute }));
vi.mock("../ipc/transport", async importOriginal => ({ ...await importOriginal<typeof import("../ipc/transport")>(), uploadImage }));

vi.mock("../ipc/commands", () => ({
  createWorktree: vi.fn(),
  getSessionCwd: vi.fn().mockResolvedValue(null),
  ptyKill: vi.fn().mockResolvedValue(undefined),
  ptyWrite: vi.fn().mockResolvedValue(undefined),
  listShells: vi.fn().mockResolvedValue([]),
  resolveSpawn,
}));
vi.mock("../ipc/tree", () => ({
  listTree: vi.fn().mockResolvedValue({ projects: [], groups: [], sessions: [] }),
}));
vi.mock("../notify", () => ({
  notify: vi.fn(),
  getNotifyPermission: vi.fn().mockResolvedValue("granted"),
  requestNotifyPermission: vi.fn().mockResolvedValue("granted"),
  getEffectiveNotifyPermission: vi.fn().mockResolvedValue("granted"),
  requestEffectiveNotifyPermission: vi.fn().mockResolvedValue("granted"),
}));

import { useTermStore } from "./termStore";

const realExecuteSpawn = useTermStore.getState().executeSpawn;

const request = { parentSessionId: "parent-1", prompt: "build the thing" };

/** Queue one card and run confirm against a stubbed executeSpawn, returning that stub. */
async function confirmWith(claim: () => Promise<boolean>) {
  const executeSpawn = vi.fn().mockResolvedValue(undefined);
  resolveSpawn.mockImplementation(claim);
  useTermStore.setState({ pendingSpawns: [{ ...request }], executeSpawn });
  await useTermStore.getState().confirmSpawn({ ...request });
  return executeSpawn;
}

describe("spawn confirmation claim", () => {
  beforeEach(() => {
    resolveSpawn.mockReset();
  });

  it("executes the task when this client wins the claim", async () => {
    const executeSpawn = await confirmWith(() => Promise.resolve(true));

    expect(resolveSpawn).toHaveBeenCalledWith("parent-1", "build the thing", true);
    expect(executeSpawn).toHaveBeenCalledTimes(1);
    // The card leaves the queue either way; only the work is conditional.
    expect(useTermStore.getState().pendingSpawns).toHaveLength(0);
  });

  it("does nothing but drop the card when another client answered first", async () => {
    const executeSpawn = await confirmWith(() => Promise.resolve(false));

    expect(executeSpawn).not.toHaveBeenCalled();
    expect(useTermStore.getState().pendingSpawns).toHaveLength(0);
  });

  it("still executes when the backend cannot answer", async () => {
    // An older backend or a transport error must not strand the task: fall back to the previous
    // behavior of executing, which risks a duplicate only in the case that was already possible.
    const executeSpawn = await confirmWith(() => Promise.reject(new Error("unknown command")));

    expect(executeSpawn).toHaveBeenCalledTimes(1);
  });

  it("claims a cancelled card too, so a cancel racing a confirm settles on one answer", () => {
    resolveSpawn.mockResolvedValue(true);
    useTermStore.setState({ pendingSpawns: [{ ...request }] });

    useTermStore.getState().cancelSpawn();

    expect(resolveSpawn).toHaveBeenCalledWith("parent-1", "build the thing", false);
    expect(useTermStore.getState().pendingSpawns).toHaveLength(0);
  });
});

/** Run one request through handleSpawnRequest against a stubbed executeSpawn, returning that stub. */
async function handleWith(
  claim: () => Promise<boolean>,
  patch: Partial<{ spawnConfirm: boolean }> = {},
) {
  const executeSpawn = vi.fn().mockResolvedValue(undefined);
  resolveSpawn.mockImplementation(claim);
  useTermStore.setState({
    spawnConfirm: false,
    notifyEnabled: false,
    pendingSpawns: [],
    executeSpawn,
    ...patch,
  });
  await useTermStore.getState().handleSpawnRequest({ ...request });
  return executeSpawn;
}

describe("spawn claim on the immediate-start path", () => {
  beforeEach(() => {
    resolveSpawn.mockReset();
  });

  it("claims before running when confirmation is off", async () => {
    const executeSpawn = await handleWith(() => Promise.resolve(true));

    expect(resolveSpawn).toHaveBeenCalledWith("parent-1", "build the thing", true);
    expect(executeSpawn).toHaveBeenCalledTimes(1);
  });

  it("does not run when another client already claimed the request", async () => {
    // Two clients with confirmation off both receive the same spawn event. Without a claim each one
    // starts the task, producing two worktrees and two agents for one request.
    const executeSpawn = await handleWith(() => Promise.resolve(false));

    expect(executeSpawn).not.toHaveBeenCalled();
  });

  it("still runs when the backend cannot answer the claim", async () => {
    const executeSpawn = await handleWith(() =>
      Promise.reject(new Error("unknown command")),
    );

    expect(executeSpawn).toHaveBeenCalledTimes(1);
  });

  it("queues a card without claiming when confirmation is on", async () => {
    // The claim belongs to the answer, not to the arrival: claiming here would settle the request
    // before anyone looked at it and dismiss the card on every other client.
    const executeSpawn = await handleWith(() => Promise.resolve(true), {
      spawnConfirm: true,
    });

    expect(resolveSpawn).not.toHaveBeenCalled();
    expect(executeSpawn).not.toHaveBeenCalled();
    expect(useTermStore.getState().pendingSpawns).toHaveLength(1);
  });
});

it("retains a failed workflow request and retries its original claim without creating a new request", async () => {
  const workflow = { ...request, requestId: "workflow-retry", planExecute: { plan: {}, exec: {} } };
  const executeSpawn = vi.fn().mockRejectedValueOnce(new Error("agent missing")).mockResolvedValueOnce(undefined);
  resolveSpawn.mockReset().mockResolvedValue(true);
  useTermStore.setState({ pendingSpawns: [workflow], executeSpawn });
  await expect(useTermStore.getState().confirmSpawn(workflow)).rejects.toThrow("agent missing");
  expect(useTermStore.getState().pendingSpawns).toEqual([workflow]);
  useTermStore.getState().handleSpawnResolved(workflow.parentSessionId, workflow.prompt);
  expect(useTermStore.getState().pendingSpawns).toEqual([workflow]);
  await useTermStore.getState().confirmSpawn(workflow);
  expect(resolveSpawn).toHaveBeenCalledOnce();
  expect(executeSpawn).toHaveBeenNthCalledWith(2, workflow);
  expect(useTermStore.getState().pendingSpawns).toHaveLength(0);
});

it.each(["none", "shared", "each"] as const)("starts only planning with %s mode when a split skill skips launch configuration", async worktreeMode => {
  const workflow = { ...request, requestId: `skill-${worktreeMode}`, noConfirm: true,
    planExecute: { splitTasks: true, worktreeMode, plan: {}, exec: {} } };
  const openSession = vi.fn();
  const addSession = vi.fn();
  const loadTree = vi.fn().mockResolvedValue(undefined);
  startPlanExecute.mockReset().mockResolvedValue({ planner: { id: "skill-planner" }, run: { state: "planning" } });
  resolveSpawn.mockReset().mockResolvedValue(true);
  useTermStore.setState({ spawnConfirm: true, notifyEnabled: false, pendingSpawns: [],
    executeSpawn: realExecuteSpawn, loadTree, openSession, addSession });
  await useTermStore.getState().handleSpawnRequest(workflow);
  expect(startPlanExecute).toHaveBeenCalledExactlyOnceWith(workflow);
  expect(useTermStore.getState().pendingSpawns).toEqual([]);
  expect(openSession).toHaveBeenCalledExactlyOnceWith("skill-planner", expect.any(Object));
  expect(addSession).not.toHaveBeenCalled();
});

it("retains a failed immediate workflow and retries the same claim and directory choice", async () => {
  const workflow = { ...request, requestId: "skill-failure-retry", noConfirm: true,
    planExecute: { splitTasks: true, worktreeMode: "each" as const, plan: {}, exec: {} } };
  const executeSpawn = vi.fn().mockRejectedValueOnce(new Error("worktree unavailable")).mockResolvedValueOnce(undefined);
  resolveSpawn.mockReset().mockResolvedValue(true);
  useTermStore.setState({ spawnConfirm: true, pendingSpawns: [], executeSpawn });
  await expect(useTermStore.getState().handleSpawnRequest(workflow)).rejects.toThrow("worktree unavailable");
  expect(useTermStore.getState().pendingSpawns).toEqual([workflow]);
  useTermStore.getState().handleSpawnResolved(workflow.parentSessionId, workflow.prompt);
  expect(useTermStore.getState().pendingSpawns).toEqual([workflow]);
  await useTermStore.getState().confirmSpawn(workflow);
  expect(resolveSpawn).toHaveBeenCalledOnce();
  expect(executeSpawn).toHaveBeenNthCalledWith(2, workflow);
  expect(useTermStore.getState().pendingSpawns).toEqual([]);
});

it("does not start a split skill on a client that loses the initial claim", async () => {
  const workflow = { ...request, requestId: "skill-lost-claim", noConfirm: true,
    planExecute: { splitTasks: true, plan: {}, exec: {} } };
  const executeSpawn = vi.fn();
  resolveSpawn.mockReset().mockResolvedValue(false);
  useTermStore.setState({ spawnConfirm: true, pendingSpawns: [], executeSpawn });
  await useTermStore.getState().handleSpawnRequest(workflow);
  expect(executeSpawn).not.toHaveBeenCalled();
  expect(useTermStore.getState().pendingSpawns).toEqual([]);
});


it("retains image drafts and the claim when upload fails, then retries without losing the card", async () => {
  const draft = { ...request, requestId: "image-retry", images: [{ mimeType: "image/png", data: "AQID" }] };
  const executeSpawn = vi.fn().mockRejectedValueOnce(new Error("upload failed")).mockResolvedValueOnce(undefined);
  resolveSpawn.mockReset().mockResolvedValue(true);
  useTermStore.setState({ pendingSpawns: [draft], executeSpawn });
  await expect(useTermStore.getState().confirmSpawn(draft)).rejects.toThrow("upload failed");
  useTermStore.getState().handleSpawnResolved(draft.parentSessionId, draft.prompt);
  expect(useTermStore.getState().pendingSpawns).toEqual([draft]);
  await useTermStore.getState().confirmSpawn(draft);
  expect(resolveSpawn).toHaveBeenCalledOnce();
  expect(executeSpawn).toHaveBeenNthCalledWith(2, draft);
  expect(useTermStore.getState().pendingSpawns).toEqual([]);
});

it.each(["codex", "claude"] as const)("uploads ordinary %s task images before creating a child and includes their paths", async kind => {
  const addSession = vi.fn().mockResolvedValue({ id: "image-child" });
  const openSession = vi.fn();
  useTermStore.setState({
    sessions: [{ id: "parent-1", projectId: "project-1", kind, name: "Parent", cwd: "/project" } as never],
    projects: [], agentDefaults: {}, pendingPrompts: {}, addSession, openSession,
  });
  uploadImage.mockReset().mockRejectedValueOnce(new Error("upload failed"));
  const draft = { ...request, kind, worktree: false, images: [{ mimeType: "image/png", data: "AQID" }] };
  await expect(realExecuteSpawn(draft)).rejects.toThrow("upload failed");
  expect(addSession).not.toHaveBeenCalled();
  uploadImage.mockResolvedValue("/tmp/pasted.png");
  await realExecuteSpawn(draft);
  expect(uploadImage).toHaveBeenLastCalledWith(new Uint8Array([1, 2, 3]), "png");
  expect(addSession).toHaveBeenCalledOnce();
  expect(useTermStore.getState().takePendingPrompt("image-child")).toBe(
    request.prompt + (kind === "codex" ? "\nimage_path: /tmp/pasted.png" : "\n/tmp/pasted.png"),
  );
  expect(openSession).toHaveBeenCalledWith("image-child", expect.any(Object));
});
