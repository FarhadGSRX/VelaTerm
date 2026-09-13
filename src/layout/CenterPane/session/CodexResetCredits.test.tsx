import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
vi.mock("../../../ipc/commands", () => ({ codexResetCreditConsume: vi.fn(), usageRefresh: vi.fn() }));
import { codexResetCreditConsume, usageRefresh, type UsageSnapshot } from "../../../ipc/commands";
import { setLang } from "../../../i18n";
import { useTermStore } from "../../../store/termStore";
import { CodexResetCredits } from "./CodexResetCredits";

function snapshot(count: number | null = 2): UsageSnapshot {
  const empty = { data: null, error: null, fetchedAt: null, errorAt: null };
  return { claude: empty, grok: empty, codex: { ...empty, data: { primary: null, secondary: null, planType: "pro", resetCredits: count } }, auto: true, intervalSec: 300 };
}
async function open(count: number | null = 2) {
  useTermStore.setState({ usage: snapshot(count) });
  vi.mocked(usageRefresh).mockResolvedValue(snapshot(count));
  const view = render(<CodexResetCredits />);
  fireEvent.click(screen.getByRole("button", { name: `Reset credits: ${count ?? "—"}` }));
  await waitFor(() => expect(screen.queryByText("Processing…")).toBeNull());
  return view;
}
beforeEach(() => { vi.resetAllMocks(); sessionStorage.clear(); setLang("en"); });
afterEach(cleanup);

it.each([null, 0])("disables redemption for unknown or zero balance (%s)", async (count) => {
  await open(count);
  expect((screen.getByRole("button", { name: "Use one credit" }) as HTMLButtonElement).disabled).toBe(true);
  if (count === null) expect(screen.getByText("Reset credit count is unavailable.")).toBeTruthy();
  expect(codexResetCreditConsume).not.toHaveBeenCalled();
});

it("sends one request on repeated clicks and publishes the refreshed balance", async () => {
  let resolve!: (value: Awaited<ReturnType<typeof codexResetCreditConsume>>) => void;
  vi.mocked(codexResetCreditConsume).mockReturnValue(new Promise((r) => { resolve = r; }));
  await open();
  const button = screen.getByRole("button", { name: "Use one credit" });
  fireEvent.click(button); fireEvent.click(button);
  expect(codexResetCreditConsume).toHaveBeenCalledTimes(1);
  await act(async () => resolve({ outcome: "reset", usage: snapshot(1) }));
  expect(screen.getByRole("status").textContent).toBe("Usage limits reset.");
  expect(useTermStore.getState().usage?.codex.data?.resetCredits).toBe(1);
  expect(sessionStorage.getItem("vlx-codex-reset-attempt")).toBeNull();
});

it("retains the UUID after a timeout and remount, including when the refreshed balance is zero", async () => {
  vi.mocked(codexResetCreditConsume).mockRejectedValueOnce(new Error("timeout"));
  const view = await open();
  fireEvent.click(screen.getByRole("button", { name: "Use one credit" }));
  await screen.findByRole("alert");
  const key = vi.mocked(codexResetCreditConsume).mock.calls[0][0];
  view.unmount();
  vi.mocked(codexResetCreditConsume).mockResolvedValueOnce({ outcome: "alreadyRedeemed", usage: snapshot(0) });
  await open(0);
  fireEvent.click(screen.getByRole("button", { name: "Retry reset" }));
  await screen.findByRole("status");
  expect(codexResetCreditConsume).toHaveBeenLastCalledWith(key);
  expect(screen.getByRole("status").textContent).toBe("This request already succeeded.");
});

it.each(["nothingToReset", "noCredit"] as const)("reports %s without pretending a reset happened", async (outcome) => {
  vi.mocked(codexResetCreditConsume).mockResolvedValue({ outcome, usage: snapshot(0) });
  await open();
  fireEvent.click(screen.getByRole("button", { name: "Use one credit" }));
  await screen.findByRole("status");
  expect(screen.queryByText("Usage limits reset.")).toBeNull();
  expect((screen.getByRole("button", { name: "Use one credit" }) as HTMLButtonElement).disabled).toBe(true);
});

it("keeps confirmed success visible when the follow-up read failed", async () => {
  const usage = snapshot(null); usage.codex.error = "read failed";
  vi.mocked(codexResetCreditConsume).mockResolvedValue({ outcome: "reset", usage });
  await open(); fireEvent.click(screen.getByRole("button", { name: "Use one credit" }));
  await screen.findByRole("status");
  expect(screen.getByRole("alert")).toBeTruthy();
  expect(screen.getByText("Reset credit count is unavailable.")).toBeTruthy();
  expect(sessionStorage.getItem("vlx-codex-reset-attempt")).toBeNull();
});

it("keeps the redemption panel inside a narrow viewport near its right edge", async () => {
  const previousWidth = window.innerWidth;
  Object.defineProperty(window, "innerWidth", { value: 360, configurable: true });
  const bounds = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ left: 300 } as DOMRect);
  try {
    const view = await open();
    const panel = view.container.querySelector(".sv-popover") as HTMLElement;
    expect(300 + Number.parseFloat(panel.style.left) + Number.parseFloat(panel.style.width)).toBeLessThanOrEqual(352);
  } finally {
    bounds.mockRestore();
    Object.defineProperty(window, "innerWidth", { value: previousWidth, configurable: true });
  }
});
