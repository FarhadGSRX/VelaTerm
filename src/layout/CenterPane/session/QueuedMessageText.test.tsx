//! The queued-message text above the composer: what is drawn while it waits, and what the view button
//! opens. jsdom reports every box as zero-sized, so the clamp is simulated through the two properties
//! the component measures.

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
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

/** jsdom implements neither method of the native dialog, so both are recorded here. */
function stubDialog() {
  const showModal = vi.fn();
  const close = vi.fn();
  const previous = {
    showModal: Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal"),
    close: Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "close"),
  };
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value: showModal });
  Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value: close });
  return {
    showModal,
    restore: () => {
      for (const [method, descriptor] of Object.entries(previous)) {
        if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, method, descriptor);
        else Reflect.deleteProperty(HTMLDialogElement.prototype, method);
      }
    },
  };
}

afterEach(cleanup);

it("opens a long queued message in a dialog instead of drawing all of it", () => {
  const restoreLines = stubLinesBox(true);
  const dialog = stubDialog();
  const text = "An instruction long enough that it cannot be drawn in the composer";
  const view = render(<QueuedMessageText text={text} disabled={false} onEdit={() => {}} />);
  try {
    fireEvent.click(screen.getByRole("button", { name: "View full message" }));
    expect(dialog.showModal).toHaveBeenCalledTimes(1);
    expect(document.querySelector(".sv-queue-preview-body")?.textContent).toBe(text);

    fireEvent.click(document.querySelector<HTMLButtonElement>(".sv-queue-preview-close")!);
    expect(document.querySelector(".sv-queue-preview")).toBeNull();
  } finally {
    // Unmount while the dialog stubs are still installed: the component closes its dialog on cleanup.
    view.unmount();
    dialog.restore();
    restoreLines();
  }
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
