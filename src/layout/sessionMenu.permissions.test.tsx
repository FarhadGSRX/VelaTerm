import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { NewAgentSession, ResumeSession } from "./sessionMenuDialogs";
import { useTermStore } from "../store/termStore";
import { invoke } from "../ipc/transport";

vi.mock("../ipc/transport", async original => ({
  ...await original<typeof import("../ipc/transport")>(),
  invoke: vi.fn(), onTransportReconnect: () => () => {},
}));

const cases = [
  ["claude", "plan"], ["claude", "default"], ["claude", "acceptEdits"],
  ["claude", "auto"], ["claude", "skip"], ["opencode", "default"], ["opencode", "skip"],
] as const;

beforeEach(() => {
  useTermStore.setState({ projects: [], groups: [], sessions: [], agentDefaults: {}, agentPresets: [] });
  vi.mocked(invoke).mockImplementation((command, args) => {
    if (command === "agent_permission_catalog") {
      const { agent, stored } = args as { agent: string; stored: string };
      return Promise.resolve({ modes: agent === "opencode" ? ["default", "bypassPermissions"]
        : ["plan", "default", "acceptEdits", "auto", "bypassPermissions"],
        selected: stored === "skip" ? "bypassPermissions" : stored || "default" }) as Promise<never>;
    }
    return Promise.resolve(command === "home_dir" ? "/tmp" : []) as Promise<never>;
  });
});
afterEach(cleanup);

for (const restoring of [false, true]) {
  it.each(cases)(`${restoring ? "restoring" : "creating"} %s retains the saved %s permission`, async (kind, mode) => {
    useTermStore.setState({ agentDefaults: { [kind]: { permissionMode: mode } } });
    const confirm = vi.fn().mockResolvedValue(undefined);
    render(restoring ? <ResumeSession onConfirm={confirm} onCancel={() => {}} />
      : <NewAgentSession projectId="p" parentSessionId={null} onConfirm={confirm} onCancel={() => {}} />);
    if (kind === "opencode") {
      fireEvent.click(screen.getByRole("combobox", { name: "Agent type" }));
      fireEvent.click(screen.getByRole("option", { name: "OpenCode" }));
    }
    await waitFor(() => expect((screen.getByRole("combobox", { name: "Permission" }) as HTMLButtonElement).disabled).toBe(false));
    if (restoring) fireEvent.change(screen.getByRole("textbox", { name: /Session ID/ }), { target: { value: "qa-native-session" } });
    fireEvent.click(screen.getByRole("button", { name: restoring ? "Resume & Open" : "Create" }));
    await waitFor(() => expect(confirm).toHaveBeenCalled());
    expect(confirm.mock.calls[0][0]).toMatchObject({ kind, permissionMode: mode });
  });
}


it("selects permissions with Enter without submitting the surrounding form", async () => {
  const confirm = vi.fn().mockResolvedValue(undefined);
  render(<NewAgentSession projectId="p" parentSessionId={null} onConfirm={confirm} onCancel={() => {}} />);
  const permission = screen.getByRole("combobox", { name: "Permission" });
  await waitFor(() => expect((permission as HTMLButtonElement).disabled).toBe(false));
  fireEvent.keyDown(permission, { key: "Enter" });
  fireEvent.keyDown(permission, { key: "ArrowDown" });
  fireEvent.keyDown(permission, { key: "Enter" });
  await waitFor(() => expect(permission.textContent).toBe("Accept File Edits"));
  expect(confirm).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Create" }));
  await waitFor(() => expect(confirm).toHaveBeenCalled());
  expect(confirm.mock.calls[0][0].permissionMode).toBe("acceptEdits");
});
