import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { LaunchModelCatalog, LaunchOption } from "../ipc/launch";

const lookup = vi.hoisted(() => vi.fn());
vi.mock("../ipc/launch", () => ({ launchModels: lookup }));
vi.mock("../i18n", () => ({ useT: () => (key: string) => key }));
import { ModelEffortFields } from "./LaunchFields";

const catalog: LaunchModelCatalog = {
  models: [
    { id: "provider-a/shared-model", label: "Shared model", effortLevels: ["low", "xhigh"] },
    { id: "provider-b/shared-model", label: "Shared model", effortLevels: ["off"] },
  ], effortLevels: ["low", "xhigh", "off"],
};

function Harness({ kind = "codex", cwd = "/repo" }: { kind?: string; cwd?: string }) {
  const [draft, setDraft] = useState({ model: "provider-a/shared-model", effort: "xhigh" });
  return <><ModelEffortFields spec={{ id: kind, effortFlag: "effort", effortLevels: ["invented"] } as LaunchOption}
    {...draft} context={{ parentSessionId: "parent", cwd }} onChange={(model, effort) => setDraft({ model, effort })} />
    <output>{JSON.stringify(draft)}</output></>;
}

beforeEach(() => { lookup.mockReset().mockResolvedValue(catalog); });
afterEach(cleanup);

it.each(["claude", "codex", "opencode", "pi", "omp"])("reads %s model-specific effort levels and retains the selected provider", async kind => {
  render(<Harness kind={kind} />);
  const effort = screen.getByLabelText("spawn.effortLabel");
  fireEvent.click(effort);
  await screen.findByRole("option", { name: "low" });
  expect(screen.queryByRole("option", { name: "invented" })).toBeNull();
  expect(screen.queryByRole("option", { name: "off" })).toBeNull();
  expect(lookup).toHaveBeenCalledWith(kind, { parentSessionId: "parent", cwd: "/repo", inheritArgs: false });
  fireEvent.keyDown(effort, { key: "Escape" });
  fireEvent.click(screen.getByLabelText("spawn.modelLabel"));
  fireEvent.click(screen.getByRole("option", { name: /provider-b\/shared-model/ }));
  expect(screen.getByRole("status").textContent).toBe(JSON.stringify({ model: "provider-b/shared-model", effort: "" }));
  fireEvent.click(effort);
  expect(screen.getAllByRole("option").map(option => option.textContent)).toEqual(["spawn.modelDefault", "off"]);
  fireEvent.click(screen.getByRole("option", { name: "off" }));
  expect(screen.getByRole("status").textContent).toBe(JSON.stringify({ model: "provider-b/shared-model", effort: "off" }));
});

it("shows the lookup failure, preserves the draft and retries both model and effort choices", async () => {
  lookup.mockRejectedValueOnce(new Error("Provider catalogue unavailable"));
  render(<Harness />);
  await screen.findByText("Provider catalogue unavailable");
  expect((screen.getByLabelText("spawn.modelLabel") as HTMLInputElement).value).toBe("provider-a/shared-model");
  expect((screen.getByLabelText("spawn.effortLabel") as HTMLInputElement).value).toBe("xhigh");
  fireEvent.click(screen.getByRole("button", { name: "common.retry" }));
  fireEvent.click(screen.getByLabelText("spawn.effortLabel"));
  await screen.findByRole("option", { name: "low" });
  expect(lookup).toHaveBeenCalledTimes(2);
});

it("ignores a previous directory's late catalogue", async () => {
  let resolveOld!: (catalog: LaunchModelCatalog) => void;
  lookup.mockImplementationOnce(() => new Promise<LaunchModelCatalog>(resolve => { resolveOld = resolve; }));
  const view = render(<Harness cwd="/old" />);
  await waitFor(() => expect(lookup).toHaveBeenCalledOnce());
  view.rerender(<Harness cwd="/new" />);
  fireEvent.click(screen.getByLabelText("spawn.effortLabel"));
  await screen.findByRole("option", { name: "low" });
  await act(async () => resolveOld({ models: [], effortLevels: ["wrong-directory"] }));
  expect(screen.queryByRole("option", { name: "wrong-directory" })).toBeNull();
  expect(screen.getByRole("option", { name: "low" })).toBeTruthy();
});
