//! The queued-message text above the composer: what is drawn while it waits, and what the view button
//! opens. jsdom reports every box as zero-sized, so the clamp is simulated through the two properties
//! the component measures.

import { cleanup, fireEvent, render, screen,waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import { QueuedMessageText } from "./QueuedMessageText";

/** Stand in for a two-line clamp that overflows, or for a box the text fits inside. */
function stubLinesBox(overflow: boolean) {
  const scrollHeight = Object.getOwnPropertyDescriptor(Element.prototype, "scrollHeight")!;
  const clientHeight = Object.getOwnPropertyDescriptor(Element.prototype, "clientHeight")!;
  Object.defineProperty(Element.prototype, "scrollHeight", {
    configurable: true,
    get(this: Element) {
      return overflow && this.classList.contains("sv-queue-text-lines") ? 120 : scrollHeight.get!.call(this);
    },
  });
  Object.defineProperty(Element.prototype, "clientHeight", {
    configurable: true,
    get(this: Element) {
      return overflow && this.classList.contains("sv-queue-text-lines") ? 40 : clientHeight.get!.call(this);
    },
  });
  return () => {
    Object.defineProperty(Element.prototype, "scrollHeight", scrollHeight);
    Object.defineProperty(Element.prototype, "clientHeight", clientHeight);
  };
}

afterEach(cleanup);

it("opens a long queued message in a dialog instead of drawing all of it", () => {
  const restoreLines = stubLinesBox(true);
  const text = "An instruction long enough that it cannot be drawn in the composer";
  const view = render(<QueuedMessageText text={text} disabled={false} onEdit={() => {}} />);
  try {
    fireEvent.click(screen.getByRole("button", { name: "View full message" }));
    expect(screen.getByRole("dialog",{name:"View full message"}).getAttribute("aria-modal")).toBe("true");
    expect(document.querySelector(".sv-queue-preview-body")?.textContent).toBe(text);

    fireEvent.click(document.querySelector<HTMLButtonElement>(".sv-queue-preview-close")!);
    expect(document.querySelector(".sv-queue-preview")).toBeNull();
  } finally {
    view.unmount();
    restoreLines();
  }
});

it("reads only an explicitly opened queued part and replaces it when paging",async()=>{
  const first="雪😀".repeat(4000);
  const loadPart=vi.fn(async(sequence:number)=>({text:sequence===0?first:"Complete final part",chunkSequence:sequence,contentRevision:"saved",eof:sequence===1,sizeBytes:28_019}));
  render(<QueuedMessageText text="Short original preview" disabled={false} onEdit={()=>{}} loadPart={loadPart}/>);
  expect(loadPart).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button",{name:"View full message"}));
  await waitFor(()=>expect(document.querySelector(".sv-queue-preview-body")?.textContent).toBe(first));
  expect(loadPart).toHaveBeenCalledTimes(1);
  const nodes=[...document.querySelectorAll(".sv-queue-preview-body span")];
  expect(nodes.length).toBeGreaterThan(1);expect(nodes.every(node=>new TextEncoder().encode(node.textContent??"").byteLength<=4096)).toBe(true);
  fireEvent.click(screen.getByRole("button",{name:"Next"}));
  await waitFor(()=>expect(document.querySelector(".sv-queue-preview-body")?.textContent).toBe("Complete final part"));
  expect(document.querySelector(".sv-queue-preview-body")?.textContent).not.toContain(first);
  expect(loadPart).toHaveBeenCalledTimes(2);
  expect(screen.queryByRole("button",{name:"Next"})).toBeNull();
});

it("aborts a queued read when its preview closes",async()=>{
  let signal:AbortSignal|undefined;
  const loadPart=vi.fn((_sequence:number,current:AbortSignal)=>{signal=current;return new Promise<never>(()=>{});});
  render(<QueuedMessageText text="Preview" disabled={false} onEdit={()=>{}} loadPart={loadPart}/>);
  fireEvent.click(screen.getByRole("button",{name:"View full message"}));
  expect(signal?.aborted).toBe(false);
  fireEvent.click(screen.getByRole("button",{name:"Close"}));
  expect(signal?.aborted).toBe(true);expect(screen.queryByRole("dialog")).toBeNull();
});

it("leaves a short queued message without a view button and still opens its editor", () => {
  const restoreLines = stubLinesBox(false);
  try {
    const onEdit = vi.fn();
    render(<QueuedMessageText text="Short instruction" disabled={false} onEdit={onEdit} />);

    expect(screen.queryByRole("button", { name: "View full message" })).toBeNull();
    fireEvent.click(screen.getByText("Short instruction"));
    expect(onEdit).toHaveBeenCalledTimes(1);
  } finally {
    restoreLines();
  }
});
