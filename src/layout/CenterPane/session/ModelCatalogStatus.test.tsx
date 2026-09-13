import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
vi.mock("../../../ipc/transport", () => ({ invoke: vi.fn(), listen: vi.fn() }));
import { invoke, listen } from "../../../ipc/transport";
import { setLang } from "../../../i18n";
import { ModelCatalogStatus } from "./ModelCatalogStatus";

const cached = { source: "cache", revision: 2, checkedAt: null, error: "downloadFailed", refreshing: false };
let receive: (value: unknown) => void;
const stop = vi.fn();
beforeEach(() => {
  vi.resetAllMocks(); setLang("en");
  vi.mocked(invoke).mockResolvedValue(cached);
  vi.mocked(listen).mockImplementation((_name, callback) => { receive = callback; return Promise.resolve(stop); });
});
afterEach(cleanup);

it("shows restored cache and switches to the new website revision on a backend event", async () => {
  const changed = vi.fn();
  const view = render(<ModelCatalogStatus onChanged={changed} />);
  await screen.findByText("Cached model catalog");
  expect(screen.getByText("Update failed. The previous catalog is still available.")).toBeTruthy();
  await act(async () => receive({ ...cached, source: "website", revision: 3, error: null }));
  expect(screen.getByText("Website model catalog")).toBeTruthy();
  expect(screen.getByText("· v3")).toBeTruthy();
  expect(screen.queryByText("Update failed. The previous catalog is still available.")).toBeNull();
  expect(changed).toHaveBeenCalledTimes(2);
  view.unmount();
  await waitFor(() => expect(stop).toHaveBeenCalledOnce());
});

it("keeps the cached revision visible when manual refresh cannot reach the backend", async () => {
  render(<ModelCatalogStatus onChanged={vi.fn()} />);
  await screen.findByText("Cached model catalog");
  vi.mocked(invoke).mockRejectedValueOnce(new Error("disconnected"));
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await waitFor(() => expect((screen.getByRole("button", { name: "Refresh" }) as HTMLButtonElement).disabled).toBe(false));
  expect(invoke).toHaveBeenCalledWith("model_catalog_refresh");
  expect(screen.getByText("· v2")).toBeTruthy();
  expect(screen.getByText("Update failed. The previous catalog is still available.")).toBeTruthy();
});
