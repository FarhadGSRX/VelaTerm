//! Which switches ask first, and what they say.

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { t } from "../../../i18n";
import { useTermStore } from "../../../store/termStore";
import type { AgentState, Session } from "../../../types";
import { useEngineSwitch } from "./engineSwitch";

const session = { id: "s1", kind: "claude", engine: "tui" } as Session;

function Harness() {
  const { switchTo, confirm } = useEngineSwitch(session);
  return (
    <>
      <button onClick={() => switchTo("chat")}>go</button>
      {confirm}
    </>
  );
}

describe("useEngineSwitch", () => {
  const setSessionEngineMode = vi.fn(() => Promise.resolve());
  const original = useTermStore.getState();

  const withState = (agentState: AgentState | null) => {
    useTermStore.setState({
      runtimes: { s1: { status: "running", agent: "claude", agentState } },
      setSessionEngineMode,
    });
  };

  beforeEach(() => setSessionEngineMode.mockClear());
  afterEach(() => {
    cleanup();
    useTermStore.setState(original, true);
  });

  it("moves an idle agent without asking", async () => {
    withState("waiting");
    render(<Harness />);
    await act(async () => fireEvent.click(screen.getByText("go")));
    expect(setSessionEngineMode).toHaveBeenCalledWith("s1", "chat");
  });

  it("asks before stopping a turn in progress", () => {
    withState("working");
    render(<Harness />);
    fireEvent.click(screen.getByText("go"));
    expect(screen.getByText(t("session.switchBody"))).toBeTruthy();
    expect(setSessionEngineMode).not.toHaveBeenCalled();
  });

  it("asks before the agent loses track of its background tasks", async () => {
    withState("background");
    render(<Harness />);
    fireEvent.click(screen.getByText("go"));
    expect(screen.getByText(t("session.switchBodyBackground"))).toBeTruthy();
    expect(setSessionEngineMode).not.toHaveBeenCalled();
    await act(async () => fireEvent.click(screen.getByText(t("session.switchConfirm"))));
    expect(setSessionEngineMode).toHaveBeenCalledWith("s1", "chat");
  });
});
