import { afterEach, describe, expect, it, vi } from "vitest";
const write = vi.hoisted(() => vi.fn());
vi.mock("../ipc/commands", () => ({ ptyWrite: write }));
import { SUBMIT_ENTER_DELAY_MS, submitToTerminal, terminalSubmitText } from "./terminalSubmit";

afterEach(() => { vi.useRealTimers(); vi.resetAllMocks(); });

describe("terminal message submission", () => {
  it("wraps multiline text in bracketed paste and drops control characters", () => {
    expect(terminalSubmitText("one\r\ntwo\x1b[201~\x03\tend", true)).toBe("\x1b[200~one\ntwo[201~\tend\x1b[201~");
  });

  it("types newlines as Returns without bracketed paste", () => {
    expect(terminalSubmitText("ls\npwd", false)).toBe("ls\rpwd");
  });

  it("sends Enter as a separate write after the delay", async () => {
    vi.useFakeTimers();
    write.mockResolvedValue(undefined);
    const done = submitToTerminal("s1", "hello");
    await vi.advanceTimersByTimeAsync(SUBMIT_ENTER_DELAY_MS - 1);
    expect(write.mock.calls).toEqual([["s1", "\x1b[200~hello\x1b[201~"]]);
    await vi.advanceTimersByTimeAsync(1);
    await done;
    expect(write.mock.calls).toEqual([["s1", "\x1b[200~hello\x1b[201~"], ["s1", "\r"]]);
  });

  it("does not press Enter when the text fails to send", async () => {
    write.mockRejectedValue(new Error("offline"));
    await expect(submitToTerminal("s1", "hello")).rejects.toThrow("offline");
    expect(write).toHaveBeenCalledTimes(1);
  });
});
