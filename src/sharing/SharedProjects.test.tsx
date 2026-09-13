import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

vi.mock("../ipc/transport", () => ({ invoke: vi.fn() }));
vi.mock("../platform/env", () => ({ env: { isBrowser: false } }));
vi.mock("../platform", () => ({
  platform: { opener: { openExternal: vi.fn().mockResolvedValue(undefined) } },
}));

import { invoke } from "../ipc/transport";
import { setLang } from "../i18n";
import { SharedProjectsRoute } from "./SharedProjects";

let linked = false;
let displayName = "Signed-in user";
const accountChanged = vi.fn();
async function advance(ms: number) {
  for (let elapsed = 0; elapsed < ms; elapsed += 10) {
    await act(() => vi.advanceTimersByTimeAsync(Math.min(10, ms - elapsed)));
  }
}
const statusCalls = () => vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "public_account_status").length;

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  setLang("en");
  linked = false;
  displayName = "Signed-in user";
  window.history.replaceState(null, "", "/?publicAccount=1");
  window.addEventListener("public-account-changed", accountChanged);
  vi.mocked(invoke).mockImplementation(async (cmd) => {
    if (cmd === "public_account_status") {
      // A delayed response lets the test observe repeated requests without an unbounded microtask loop.
      await new Promise((resolve) => setTimeout(resolve, 10));
      return { linked, account: linked ? { id: "account", displayName } : undefined } as never;
    }
    if (cmd === "public_remote_options") return {
      options: { scopes: ["project"] },
      projects: [{ id: "project-1", name: "Test project" }],
      sessions: [],
    } as never;
    if (cmd === "public_account_remote_devices") return [] as never;
    if (cmd === "public_account_link") return { url: "https://velaterm.com/account/device?code=fixture", publicKey: "fixture" } as never;
    if (cmd === "public_account_poll") { linked = true; return { linked: true } as never; }
    if (cmd === "public_account_logout") { linked = false; return undefined as never; }
    throw new Error(`Unexpected command: ${cmd}`);
  });
});

afterEach(() => {
  cleanup();
  window.removeEventListener("public-account-changed", accountChanged);
  vi.useRealTimers();
});

it.each([false, true])("keeps the account panel mounted while open (linked=%s)", async (initialLinked) => {
  linked = initialLinked;
  const { container } = render(<SharedProjectsRoute />);
  await advance(100);

  expect(statusCalls()).toBe(2);
  expect(accountChanged).not.toHaveBeenCalled();
  const panel = container.querySelector(".public-sharing");
  expect(panel).not.toBeNull();
  expect(screen.queryByRole("status")).toBeNull();

  await advance(45000);
  expect(statusCalls()).toBe(5);
  expect(container.querySelector(".public-sharing")).toBe(panel);
  expect(screen.queryByRole("status")).toBeNull();
});

it("preserves the panel and selected scope while an account change refreshes the header", async () => {
  linked = true;
  const { container } = render(<SharedProjectsRoute />);
  await advance(100);
  const panel = container.querySelector(".public-sharing");
  const scope = screen.getByRole("radio", { name: "Project" }) as HTMLInputElement;
  fireEvent.click(scope);
  const project = screen.getByRole("combobox", { name: "Project" }) as HTMLSelectElement;
  fireEvent.change(project, { target: { value: "project-1" } });

  displayName = "Updated name";
  act(() => { window.dispatchEvent(new Event("public-account-changed")); });
  expect(container.querySelector(".public-sharing")).toBe(panel);
  expect(screen.queryByRole("status")).toBeNull();
  await advance(100);

  expect(screen.getByRole("link", { name: "Updated name" })).toBeTruthy();
  expect(container.querySelector(".public-sharing")).toBe(panel);
  expect(scope.checked).toBe(true);
  expect(project.value).toBe("project-1");
});

it("updates the account header after login and logout without remounting the panel", async () => {
  const { container } = render(<SharedProjectsRoute />);
  await advance(100);
  const panel = container.querySelector(".public-sharing");
  fireEvent.click(screen.getByRole("button", { name: "Sign in to VelaTerm" }));
  await advance(100);

  expect(screen.getByRole("link", { name: "Signed-in user" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Sign out" })).toBeTruthy();
  expect(container.querySelector(".public-sharing")).toBe(panel);
  expect(accountChanged).toHaveBeenCalledTimes(1);

  fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
  await advance(100);
  expect(screen.getByRole("link", { name: "Account" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Sign in to VelaTerm" })).toBeTruthy();
  expect(container.querySelector(".public-sharing")).toBe(panel);
  expect(accountChanged).toHaveBeenCalledTimes(2);
});
