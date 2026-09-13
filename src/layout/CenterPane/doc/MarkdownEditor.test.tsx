import * as React from "react";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MarkdownEditor } from "./MarkdownEditor";

const calls = vi.hoisted(() => ({
  visualFocus: vi.fn(), sourceFocus: vi.fn(), setMarkdown: vi.fn(), setText: vi.fn(), placeCaret: vi.fn(),
}));
vi.mock("./WysiwygEditor", () => ({
  WysiwygEditor: React.forwardRef(function Visual(props: { defaultValue: string; onReady(): void; onEdited(): void }, ref) {
    const input = React.useRef<HTMLTextAreaElement>(null);
    React.useImperativeHandle(ref, () => ({
      getMarkdown: () => input.current!.value,
      setMarkdown: (text: string) => { calls.setMarkdown(text); input.current!.value = text; },
      placeCaret: calls.placeCaret,
      focus: () => { calls.visualFocus(); input.current?.focus(); },
    }));
    // The real editor initializes once, independently of callback identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    React.useEffect(() => props.onReady(), []);
    return <div className="docview-wysiwyg"><textarea data-testid="visual" ref={input}
      defaultValue={props.defaultValue} onChange={props.onEdited} /></div>;
  }),
}));
vi.mock("./SourceEditor", () => ({
  SourceEditor: React.forwardRef(function Source(props: { defaultValue: string; onEdited(): void }, ref) {
    const input = React.useRef<HTMLTextAreaElement>(null);
    React.useImperativeHandle(ref, () => ({
      getText: () => input.current!.value,
      setText: (text: string) => { calls.setText(text); input.current!.value = text; },
      focus: () => { calls.sourceFocus(); input.current?.focus(); },
    }));
    return <textarea data-testid="source" ref={input} defaultValue={props.defaultValue} onChange={props.onEdited} />;
  }),
}));

afterEach(() => { cleanup(); vi.useRealTimers(); vi.clearAllMocks(); });
const props = { defaultValue: "original", docPath: "/tmp/example.md", onEdited: vi.fn(), onImageError: vi.fn() };

describe("Markdown writing interactions", () => {
  it("does not steal initial focus and restores the selected editor's focus after a mode switch", () => {
    const view = render(<MarkdownEditor {...props} mode="visual" />);
    expect(calls.visualFocus).not.toHaveBeenCalled();
    view.rerender(<MarkdownEditor {...props} mode="source" />);
    expect(document.activeElement).toBe(view.getByTestId("source"));
    view.rerender(<MarkdownEditor {...props} mode="compare" />);
    expect(document.activeElement).toBe(view.getByTestId("visual"));
  });

  it.each(["visual", "source"] as const)("waits for %s composition to finish before updating the companion", side => {
    vi.useFakeTimers();
    const view = render(<MarkdownEditor {...props} mode="compare" />);
    const update = side === "visual" ? calls.setText : calls.setMarkdown;
    update.mockClear();
    const input = view.getByTestId(side);
    fireEvent.compositionStart(input);
    fireEvent.change(input, { target: { value: "输入中" } });
    act(() => vi.advanceTimersByTime(500));
    expect(update).not.toHaveBeenCalled();
    fireEvent.change(input, { target: { value: "输入完成" } });
    fireEvent.compositionEnd(input);
    act(() => vi.advanceTimersByTime(200));
    expect(update).toHaveBeenLastCalledWith("输入完成");
  });

  it("places a caret from paper clicks while preserving Shift and leaving editor controls alone", () => {
    const view = render(<MarkdownEditor {...props} mode="visual" />);
    const pane = view.container.querySelector<HTMLElement>(".doc-markdown-visual")!;
    Object.defineProperty(pane, "clientWidth", { value: 600 });
    fireEvent.mouseDown(pane, { button: 0, clientX: 20, clientY: 240, shiftKey: true });
    expect(calls.placeCaret).toHaveBeenCalledWith(20, 240, true);
    calls.placeCaret.mockClear();
    fireEvent.mouseDown(view.getByTestId("visual"), { button: 0, clientX: 20 });
    fireEvent.mouseDown(pane, { button: 2, clientX: 20 });
    fireEvent.mouseDown(pane, { button: 0, clientX: 601 });
    expect(calls.placeCaret).not.toHaveBeenCalled();
  });
});
