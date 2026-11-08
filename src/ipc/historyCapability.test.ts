import { afterEach, beforeEach, expect, it, vi } from "vitest";

const { nativeCall } = vi.hoisted(() => ({ nativeCall: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: nativeCall, Channel: class {} }));
vi.mock("../i18n", () => ({ t: (key: string) => key }));
vi.mock("./reqLog", () => ({ recordRequestError: vi.fn() }));

beforeEach(() => { vi.resetModules(); nativeCall.mockReset(); (window as any).__TAURI_INTERNALS__ = {}; });
afterEach(() => { delete (window as any).__TAURI_INTERNALS__; });

it("probes a separate command and never calls an older desktop history reader", async () => {
  nativeCall.mockRejectedValue("Unknown command: chat_capabilities");
  const { invoke } = await import("./transport");
  await expect(invoke("chat_snapshot", { sessionId: "s", preferBounded: true })).rejects.toThrow("history_protocol_unsupported:");
  await expect(invoke("read_agent_transcript", { sessionId: "s" })).rejects.toThrow("history_protocol_unsupported:");
  const calls = nativeCall.mock.calls.map(([, args]) => args?.cmd);
  expect(calls.filter(command => command === "chat_capabilities")).toHaveLength(1);
  expect(calls).not.toContain("chat_snapshot");
  expect(calls).not.toContain("read_agent_transcript");
});

it("shares the successful capability check without blocking agent controls", async () => {
  nativeCall.mockImplementation((_, args) => Promise.resolve(args.cmd === "chat_capabilities" ? { historyProtocol: 2 } : { rows: [] }));
  const { invoke } = await import("./transport");
  await Promise.all([invoke("chat_snapshot", { sessionId: "s" }), invoke("chat_row", { sessionId: "s", rowId: "r" })]);
  await invoke("chat_interrupt", { sessionId: "s" });
  const calls = nativeCall.mock.calls.map(([, args]) => args?.cmd);
  expect(calls.filter(command => command === "chat_capabilities")).toHaveLength(1);
  expect(calls).toContain("chat_snapshot");
  expect(calls).toContain("chat_row");
  expect(calls).toContain("chat_interrupt");
});

it("retries a transient probe failure without attempting an unverified history read", async () => {
  nativeCall.mockImplementation((_, args) => Promise.resolve(args.cmd === "chat_capabilities" ? { historyProtocol: 2 } : { rows: [] }));
  nativeCall.mockRejectedValueOnce("Backend temporarily unavailable");
  const { invoke } = await import("./transport");
  await expect(invoke("chat_snapshot", { sessionId: "s" })).rejects.toBe("Backend temporarily unavailable");
  expect(nativeCall.mock.calls.some(([, args]) => args?.cmd === "chat_snapshot")).toBe(false);
  await expect(invoke("chat_snapshot", { sessionId: "s" })).resolves.toEqual({ rows: [] });
});
