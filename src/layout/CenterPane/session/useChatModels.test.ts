import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { chatModels, type ChatModel } from "../../../ipc/chat";
import { onTransportReconnect } from "../../../ipc/transport";
import { useChatModels } from "./useChatModels";

vi.mock("../../../ipc/chat", () => ({ chatModels: vi.fn() }));
vi.mock("../../../ipc/transport", () => ({ onTransportReconnect: vi.fn(() => () => {}) }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });

const models: ChatModel[] = [{ id: "reported", label: "Reported", description: "", effortLevels: [], curated: true, largeContext: false }];

it("ignores a discovery response from a previous session", async () => {
  let finish!: (value: ChatModel[]) => void;
  vi.mocked(chatModels).mockImplementation(id => id === "old"
    ? new Promise(resolve => { finish = resolve; }) : Promise.resolve(models));
  const { result, rerender } = renderHook(({ id }) => useChatModels(id, 0), { initialProps: { id: "old" } });
  rerender({ id: "new" });
  await waitFor(() => expect(result.current.models).toEqual(models));
  await act(async () => finish([{ ...models[0], id: "obsolete" }]));
  expect(result.current.models).toEqual(models);
});

it("retries after reconnect and ignores an older request that fails late", async () => {
  let reject!: (error: Error) => void;
  vi.mocked(chatModels).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }))
    .mockResolvedValue(models);
  const { result } = renderHook(() => useChatModels("s", 0));
  act(() => vi.mocked(onTransportReconnect).mock.calls[0][0]());
  await waitFor(() => expect(result.current.models).toEqual(models));
  await act(async () => reject(new Error("late failure")));
  expect(result.current.failed).toBe(false);
  expect(result.current.loading).toBe(false);
  expect(chatModels).toHaveBeenCalledTimes(2);
});
