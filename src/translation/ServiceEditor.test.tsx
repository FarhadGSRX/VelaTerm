import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { TranslationService } from "../ipc/translation";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), copy: vi.fn() }));
vi.mock("../ipc/transport", async original => ({ ...await original<typeof import("../ipc/transport")>(), invoke: mocks.invoke }));
vi.mock("../platform", () => ({ platform: { clipboard: { writeText: mocks.copy } } }));
import { setLang } from "../i18n";
import { ServiceEditor } from "./ServiceEditor";

const service: TranslationService = { id: "fixture", name: "Fixture", baseUrl: "https://fixture.example/v1", model: "fixture-model", hasKey: true };
const savedKey = "synthetic-saved-key";
const props = () => ({ service, isNew: false, editable: true, onSave: vi.fn().mockResolvedValue(undefined), onDelete: vi.fn().mockResolvedValue(undefined), onCancel: vi.fn(), formatError: () => "Request failed" });
beforeEach(() => {
  setLang("en");
  mocks.invoke.mockReset().mockImplementation(async command => command === "translation_service_key" ? savedKey : "fixture");
  mocks.copy.mockReset().mockResolvedValue(undefined);
});
afterEach(cleanup);

it("loads a saved key masked, copies its full value without revealing it, and resets visibility on reopen", async () => {
  const options = props();
  const view = render(<ServiceEditor {...options} />);
  const key = await screen.findByLabelText<HTMLInputElement>("API key");
  expect(key.value).toBe(savedKey);
  expect(key.type).toBe("password");
  expect(view.container.querySelector('[role="dialog"]')).toBeNull();
  expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("translation_service_key", { id: "fixture" });
  fireEvent.click(screen.getByRole("button", { name: "Copy API key" }));
  expect((await screen.findByRole("status")).textContent).toBe("API key copied");
  expect(mocks.copy).toHaveBeenCalledExactlyOnceWith(savedKey, { reportFailure: true });
  expect(key.type).toBe("password");
  fireEvent.click(screen.getByRole("button", { name: "Show API key" }));
  expect(key.type).toBe("text");
  view.unmount(); render(<ServiceEditor {...options} />);
  expect((await screen.findByLabelText<HTMLInputElement>("API key")).type).toBe("password");
});

it("preserves edits on save failure, supports replacing and clearing the key, and reports failed copies", async () => {
  const options = props();
  render(<ServiceEditor {...options} />);
  const key = await screen.findByLabelText<HTMLInputElement>("API key");
  fireEvent.change(key, { target: { value: "replacement-key" } });
  mocks.copy.mockRejectedValueOnce(new Error("clipboard unavailable"));
  fireEvent.click(screen.getByRole("button", { name: "Copy API key" }));
  expect((await screen.findByRole("status")).textContent).toBe("Could not copy. Please try again.");
  mocks.invoke.mockRejectedValueOnce(new Error("storage unavailable"));
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect((await screen.findByRole("alert")).textContent).toBe("Request failed");
  expect(key.value).toBe("replacement-key");
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(options.onSave).toHaveBeenCalledWith("fixture"));
  expect(mocks.invoke).toHaveBeenLastCalledWith("translation_service_save", { service: { id: service.id, name: service.name, baseUrl: service.baseUrl, model: service.model, apiKey: "replacement-key", clearKey: false } });
  fireEvent.change(key, { target: { value: "" } });
  expect(screen.getByRole<HTMLButtonElement>("button", { name: "Copy API key" }).disabled).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(mocks.invoke).toHaveBeenLastCalledWith("translation_service_save", { service: expect.objectContaining({ apiKey: "", clearKey: true }) }));
});

it("can cancel credential loading and does not allow saving after a failed read until retry succeeds", async () => {
  const options = props();
  let reject!: (error: Error) => void;
  mocks.invoke.mockImplementationOnce(() => new Promise((_resolve, fail) => { reject = fail; }));
  render(<ServiceEditor {...options} />);
  expect(screen.queryByLabelText("API key")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(options.onCancel).toHaveBeenCalledOnce();
  await act(async () => { reject(new Error("connection lost")); });
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect((await screen.findByLabelText<HTMLInputElement>("API key")).value).toBe(savedKey);
  expect(mocks.invoke.mock.calls.every(([command]) => command === "translation_service_key")).toBe(true);
});

it("keeps Escape inside the editor and does not submit when keyboard focus is on reveal or copy", async () => {
  const options = props(); const parentEscape = vi.fn();
  render(<div onKeyDown={parentEscape}><ServiceEditor {...options} /></div>);
  await screen.findByLabelText("API key");
  fireEvent.keyDown(screen.getByRole("button", { name: "Show API key" }), { key: "Enter" });
  fireEvent.keyDown(screen.getByRole("button", { name: "Copy API key" }), { key: "Enter" });
  expect(mocks.invoke.mock.calls.some(([command]) => command === "translation_service_save")).toBe(false);
  parentEscape.mockClear();
  fireEvent.keyDown(screen.getByLabelText("API key"), { key: "Escape" });
  expect(options.onCancel).toHaveBeenCalledOnce();
  expect(parentEscape).not.toHaveBeenCalled();
});

it("retains the service draft when canceling deletion and requires confirmation before deleting", async () => {
  const options = props(); render(<ServiceEditor {...options} />);
  await screen.findByLabelText("API key");
  fireEvent.change(screen.getByLabelText(/Service name/), { target: { value: "Renamed fixture" } });
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  expect(screen.getByRole("dialog").getAttribute("aria-label")).toBe("Delete: Fixture");
  expect(options.onDelete).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.getByLabelText<HTMLInputElement>(/Service name/).value).toBe("Renamed fixture");
  expect(screen.getByLabelText<HTMLInputElement>("API key").type).toBe("password");
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  await waitFor(() => expect(options.onDelete).toHaveBeenCalledExactlyOnceWith("fixture"));
});

it("never reads stored credentials in an uneditable remote view", () => {
  render(<ServiceEditor {...props()} editable={false} />);
  expect(mocks.invoke).not.toHaveBeenCalled();
  expect(screen.queryByLabelText("API key")).toBeNull();
});
