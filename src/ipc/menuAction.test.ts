//! Covers which window acts on a native menu action.
//!
//! The backend addresses `menu://action` to the focused window with `emit_to(label, …)`. The frontend used
//! a bare Tauri `listen`, whose default target is Any, so every window accepted every action: one Cmd+D in
//! the main window also split the focused pane of each open remote window, and mirror mode then carried
//! those splits to the host. These tests replay Tauri's delivery rule against the registered listener.

import { beforeEach, describe, expect, it, vi } from "vitest";

type Target = { target?: string } | undefined;
type Handler = { cb: (e: { payload: unknown }) => void; target?: string };

const { handlers, listen } = vi.hoisted(() => {
  const handlers: Handler[] = [];
  const listen = vi.fn(
    (_name: string, cb: Handler["cb"], options?: Target) => {
      handlers.push({ cb, target: options?.target });
      return Promise.resolve(() => {});
    },
  );
  return { handlers, listen };
});
vi.mock("@tauri-apps/api/event", () => ({ listen }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "remote-1a2b3c4d" }),
}));
vi.mock("../i18n", () => ({ t: (key: string) => key }));

/** Tauri's rule: a listener without a target takes everything; a labelled one takes its label or a broadcast. */
function deliver(label: string | null, payload: string) {
  for (const h of handlers)
    if (label === null || h.target === undefined || h.target === label)
      h.cb({ payload });
}

beforeEach(() => {
  handlers.length = 0;
  listen.mockClear();
  vi.resetModules();
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("native menu actions", () => {
  it("are acted on only by the window they were addressed to", async () => {
    const { onMenuAction } = await import("./events");
    const seen: string[] = [];
    await onMenuAction((action) => seen.push(action));

    deliver("main", "split-right");
    expect(seen).toEqual([]);

    deliver("remote-1a2b3c4d", "split-right");
    expect(seen).toEqual(["split-right"]);
  });

  it("are scoped the same way in remote windows", async () => {
    (window as unknown as Record<string, unknown>).__VLX_REMOTE__ = { session: null };
    (window as unknown as Record<string, unknown>).__VLX_FORCE_BROWSER__ = true;
    try {
      const { onMenuAction } = await import("./events");
      const seen: string[] = [];
      await onMenuAction((action) => seen.push(action));

      deliver("main", "split-down");
      expect(seen).toEqual([]);
      expect(listen).toHaveBeenCalledWith("menu://action", expect.any(Function), {
        target: "remote-1a2b3c4d",
      });
    } finally {
      delete (window as unknown as Record<string, unknown>).__VLX_REMOTE__;
      delete (window as unknown as Record<string, unknown>).__VLX_FORCE_BROWSER__;
    }
  });

  it("still arrive when broadcast to every window", async () => {
    const { onMenuAction } = await import("./events");
    const seen: string[] = [];
    await onMenuAction((action) => seen.push(action));

    deliver(null, "settings");
    expect(seen).toEqual(["settings"]);
  });
});
