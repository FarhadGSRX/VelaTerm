import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import { TurnSection, TurnStatsView } from "./TurnSection";
import { agentTurnStats, type AgentTurnStats } from "../../ipc/commands";
import type { Session } from "../../types";
vi.mock("../../ipc/commands", () => ({ agentTurnStats: vi.fn() }));
const stats: AgentTurnStats = {
  model: "gpt-5.2-codex", tokens: 1600, inputTokens: 12000, totalTokens: 13600, cachedTokens: 10560,
  cacheHitPercent: 88, sessionTokens: 320000, toolsUsed: 3, filesTouched: 0,
  contextTokens: 73000, contextLimit: 258400, contextPercent: 28.25, generationTokensPerSecond: 32,
};
const session = (id: string, agentSessionId: string | null = "native") => ({ id, kind: "codex", agentSessionId } as Session);
afterEach(() => { cleanup(); vi.resetAllMocks(); });
describe("This turn", () => {
  it("separates context, turn totals, cached input and session usage", () => {
    render(<TurnStatsView stats={stats} />);
    const panel = screen.getByRole("region", { name: "This turn" });
    expect(within(panel).getByText("gpt-5.2-codex")).toBeTruthy();
    expect(within(panel).getByRole("progressbar").getAttribute("aria-valuenow")).toBe("28.25");
    expect(within(panel).getByText("13.6k")).toBeTruthy();
    expect(within(panel).getByText("320k")).toBeTruthy();
    expect(within(panel).getByText("88.0%")).toBeTruthy();
    expect(within(panel).getByText("32.0 tok/s")).toBeTruthy();
    expect(within(panel).getByText("0")).toBeTruthy();
  });
  it("shows absent metrics as unknown and caps only the visual context meter", () => {
    const { rerender } = render(<TurnStatsView stats={null} state="error" />);
    expect(screen.getByRole("status").textContent).toContain("temporarily unavailable");
    expect(screen.queryByText("0")).toBeNull();
    expect(screen.queryByRole("progressbar")).toBeNull();
    rerender(<TurnStatsView stats={{ ...stats, contextPercent: 105, generationTokensPerSecond: null }} />);
    expect(screen.getByText("105%")).toBeTruthy();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("100");
    expect(screen.queryByText(/tok\/s/)).toBeNull();
  });
  it("shows context used without a meter when the model's limit is unknown", () => {
    render(<TurnStatsView stats={{ ...stats, contextPercent: null, contextLimit: null }} />);
    const panel = screen.getByRole("region", { name: "This turn" });
    expect(within(panel).queryByRole("progressbar")).toBeNull();
    expect(within(panel).getByText("73.0k / —")).toBeTruthy();
  });
  it("rejects a late response from a previously selected session", async () => {
    let resolveOld!: (value: AgentTurnStats) => void;
    vi.mocked(agentTurnStats).mockImplementation(id => id === "old" ? new Promise(resolve => { resolveOld = resolve; }) : Promise.resolve({ ...stats, totalTokens: 9000 }));
    const { rerender } = render(<TurnSection session={session("old")} />);
    rerender(<TurnSection session={session("new")} />);
    await screen.findByText("9k");
    await act(async () => { resolveOld(stats); });
    expect(screen.queryByText("13.6k")).toBeNull();
    expect(screen.getByText("9k")).toBeTruthy();
  });
  it("clears stale statistics when a read fails and waits for an uncaptured session", async () => {
    vi.mocked(agentTurnStats).mockResolvedValueOnce(stats).mockRejectedValue(new Error("missing"));
    const { rerender } = render(<TurnSection session={session("a")} />);
    await screen.findByText("13.6k");
    rerender(<TurnSection session={session("a")} currentTool="Read" />);
    await waitFor(() => expect(screen.getByRole("status").textContent).toContain("temporarily unavailable"));
    expect(screen.queryByText("13.6k")).toBeNull();
    const calls = vi.mocked(agentTurnStats).mock.calls.length;
    rerender(<TurnSection session={session("b", null)} />);
    expect(screen.getByRole("status").textContent).toContain("first turn");
    expect(vi.mocked(agentTurnStats).mock.calls.length).toBe(calls);
  });
});
