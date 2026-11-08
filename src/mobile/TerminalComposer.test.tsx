import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createRef } from "react";
const submit = vi.hoisted(() => vi.fn());
vi.mock("../i18n", () => ({ useT: () => (key: string) => key }));
vi.mock("./terminalSubmit", () => ({ submitToTerminal: submit }));
import { TerminalComposer } from "./TerminalComposer";

afterEach(() => { cleanup(); vi.resetAllMocks(); });

const mount = (sessionId: string, props: { bracketed?: () => boolean; onClose?: () => void } = {}) => render(
  <TerminalComposer sessionId={sessionId} inputRef={createRef<HTMLTextAreaElement>()}
    bracketed={props.bracketed ?? (() => true)} onClose={props.onClose ?? (() => {})} onImages={() => {}} />,
);
const box = () => screen.getByRole("textbox") as HTMLTextAreaElement;

describe("terminal message box", () => {
  it("sends the whole message and clears it", async () => {
    submit.mockResolvedValue(undefined);
    mount("send", { bracketed: () => false });
    const send = screen.getByRole("button", { name: "session.send" }) as HTMLButtonElement;
    expect(send.disabled).toBe(true);
    fireEvent.change(box(), { target: { value: "第一行\n第二行" } });
    fireEvent.click(send);
    await waitFor(() => expect(box().value).toBe(""));
    expect(submit).toHaveBeenCalledWith("send", "第一行\n第二行", false);
  });

  it("keeps the text and reports a failed send", async () => {
    submit.mockRejectedValue(new Error("offline"));
    vi.spyOn(console, "warn").mockImplementation(() => {});
    mount("fail");
    fireEvent.change(box(), { target: { value: "keep me" } });
    fireEvent.keyDown(box(), { key: "Enter", ctrlKey: true });
    expect((await screen.findByRole("alert")).textContent).toBe("chat.submission.failed");
    expect(box().value).toBe("keep me");
  });

  it("keeps the draft across closing and reopening", () => {
    const close = vi.fn();
    mount("draft", { onClose: close });
    fireEvent.change(box(), { target: { value: "unsent" } });
    fireEvent.click(screen.getByRole("button", { name: "common.close" }));
    expect(close).toHaveBeenCalledOnce();
    cleanup();
    mount("draft");
    expect(box().value).toBe("unsent");
  });

  it("inserts a newline on a plain Return instead of sending", () => {
    mount("newline");
    fireEvent.change(box(), { target: { value: "text" } });
    fireEvent.keyDown(box(), { key: "Enter" });
    expect(submit).not.toHaveBeenCalled();
  });
});
