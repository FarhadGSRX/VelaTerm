import { afterEach, describe, expect, it, vi } from "vitest";
import { installTauriLinkHandler } from "./tauriLinkHandler";

const nativeWindow = window as unknown as Record<string, unknown>;
let remove = () => {};
afterEach(() => {
  remove();
  delete nativeWindow.__TAURI_INTERNALS__;
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

function setup(native = true) {
  if (native) nativeWindow.__TAURI_INTERNALS__ = {};
  const open = vi.fn().mockResolvedValue(undefined);
  remove = installTauriLinkHandler(open);
  return open;
}

function clickLink(href = "https://velaterm.com/game-center", target = "_blank", init: MouseEventInit = {}, prevented = false) {
  const anchor = document.createElement("a");
  anchor.href = href;
  anchor.target = target;
  const child = document.createElement("span");
  anchor.append(child);
  document.body.append(anchor);
  const event = new MouseEvent("click", { bubbles: true, cancelable: true, composed: true, ...init });
  if (prevented) event.preventDefault();
  // Observe our handler first, then suppress jsdom's unimplemented native navigation.
  let defaultPrevented = false;
  window.addEventListener("click", (observed) => {
    defaultPrevented = observed.defaultPrevented;
    observed.preventDefault();
  }, { once: true });
  child.dispatchEvent(event);
  return { defaultPrevented };
}

describe("application-scoped Tauri external links", () => {
  it("leaves normal browser clicks to native navigation", () => {
    const open = setup(false);
    expect(clickLink().defaultPrevented).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });

  it.each(["https://velaterm.com/game-center", "http://localhost/game", "mailto:hello@example.com", "tel:+15555550123"])("opens a supported application link: %s", (href) => {
    const open = setup();
    expect(clickLink(href).defaultPrevented).toBe(true);
    expect(open).toHaveBeenCalledExactlyOnceWith(href);
  });

  it.each([{ ctrlKey: true }, { shiftKey: true }])("preserves modifier opening for ordinary application links: %o", (modifier) => {
    const open = setup();
    expect(clickLink("https://velaterm.com/", "", modifier).defaultPrevented).toBe(true);
    expect(open).toHaveBeenCalledExactlyOnceWith("https://velaterm.com/");
  });

  it.each([{ metaKey: true }, { altKey: true }, { button: 1 }, { button: 2 }])("does not claim other mouse gestures: %o", (modifier) => {
    const open = setup();
    expect(clickLink(undefined, undefined, modifier).defaultPrevented).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });

  it("respects application navigation that already handled the click", () => {
    const open = setup();
    clickLink(undefined, undefined, {}, true);
    expect(open).not.toHaveBeenCalled();
  });

  it("leaves same-tab navigation unchanged", () => {
    const open = setup();
    expect(clickLink("https://velaterm.com/", "").defaultPrevented).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });

  it.each(["file:///tmp/example", "data:text/plain,example", "about:blank"])("does not send unsupported protocols to the system opener: %s", (href) => {
    const open = setup();
    expect(clickLink(href).defaultPrevented).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });

  it("removes its listener when the application module is replaced", () => {
    const open = setup();
    remove();
    expect(clickLink().defaultPrevented).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });

  it("handles rejected opener calls without an unhandled promise", async () => {
    const open = setup();
    open.mockRejectedValueOnce(new Error("opening failed"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    clickLink();
    await Promise.resolve();
    expect(warn).toHaveBeenCalledOnce();
  });
});
