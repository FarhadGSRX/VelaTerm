import { describe, expect, it, vi } from "vitest";
import { safeCommand, safeError, beginDiagnosticOperation, diagnosticOperation } from "./diagnosticSafety";
import { clearRequestErrors, getRequestErrors, recordRequestError } from "./reqLog";

describe("diagnostic privacy", () => {
  it("never retains private exception text, nested objects or caller command names", () => {
    const spy=vi.spyOn(console,"error").mockImplementation(() => {});
    clearRequestErrors();
    const secret="Bearer synthetic-test-token user@example.test /private/home/document";
    recordRequestError(secret,new Error(secret));
    recordRequestError("pty_kill",{password:secret});
    expect(getRequestErrors().map(e=>[e.cmd,e.message])).toEqual([["pty_kill","operation_failed"],["unknown","operation_failed"]]);
    expect(JSON.stringify(spy.mock.calls)).not.toContain(secret);
    spy.mockRestore();
  });
  it("keeps useful fixed error categories without their original messages", () => {
    expect(safeError(new Error("permission denied: private path"))).toBe("permission_denied");
    expect(safeError("timed out https://example.test/?token=synthetic")).toBe("timeout");
    expect(safeCommand("pty_spawn")).toBe("pty_spawn");
    for (const command of ["launch_models", "plan_execute_defaults", "plan_execute_prepare", "plan_execute_start", "plan_execute_create"]) expect(safeCommand(command)).toBe(command);
    expect(safeError("Executor: Model and effort must be identifiers without spaces or shell operators")).toBe("invalid_launch_identifier");
    expect(safeCommand("pty_spawn\nforged")).toBe("unknown");
  });
  it("correlates a remount only within the bounded operation lifetime", () => {
    vi.useFakeTimers();
    const id=beginDiagnosticOperation("session-test");
    expect(diagnosticOperation("session-test")).toBe(id);
    vi.advanceTimersByTime(120_001);
    expect(diagnosticOperation("session-test")).toBeUndefined();
    vi.useRealTimers();
  });
});
