import { afterEach, beforeEach, expect, it, vi } from "vitest";

const { nativeInvoke, remoteInvoke, env } = vi.hoisted(() => ({
  nativeInvoke: vi.fn(), remoteInvoke: vi.fn(),
  env: { hasNativeHost: true, isTauri: true, isRemoteWindow: false },
}));
vi.mock("./env", () => ({ env }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: nativeInvoke }));
vi.mock("../ipc/transport", () => ({ invoke: remoteInvoke, listen: vi.fn(), copyText: vi.fn(), openPath: vi.fn(), pickDirectory: vi.fn(), revealPath: vi.fn() }));
vi.mock("../notify", () => ({ getEffectiveNotifyPermission: vi.fn(), getNotifyPermission: vi.fn(), notify: vi.fn(), requestEffectiveNotifyPermission: vi.fn(), requestNotifyPermission: vi.fn() }));

import { tauriPlatform } from "./tauri";
import { electronPlatform } from "./electron";

beforeEach(() => {
  nativeInvoke.mockReset(); remoteInvoke.mockReset();
  Object.assign(env, { hasNativeHost: true, isTauri: true, isRemoteWindow: false });
});
afterEach(() => { delete (window as any).vlxNative; });

it("queries the local native host even in a remote Tauri window", async () => {
  Object.assign(env, { isTauri: false, isRemoteWindow: true });
  const catalog = { families: ["Local Font"], systemFontsAvailable: true };
  nativeInvoke.mockResolvedValue(catalog);
  expect(await tauriPlatform.fonts.catalog()).toBe(catalog);
  expect(nativeInvoke).toHaveBeenCalledExactlyOnceWith("plugin:local-fonts|catalog");
  expect(remoteInvoke).not.toHaveBeenCalled();
});

it("requests only bundled fonts in a plain browser", async () => {
  Object.assign(env, { hasNativeHost: false, isTauri: false });
  remoteInvoke.mockResolvedValue({ families: ["JetBrains Mono"], systemFontsAvailable: false });
  await tauriPlatform.fonts.catalog();
  expect(remoteInvoke).toHaveBeenCalledExactlyOnceWith("bundled_font_catalog");
  expect(nativeInvoke).not.toHaveBeenCalled();
});

it("uses Electron's local bridge rather than its remote transport", async () => {
  const fontCatalog = vi.fn().mockResolvedValue({ families: ["Electron Local Font"], systemFontsAvailable: true });
  (window as any).vlxNative = { fontCatalog };
  await electronPlatform.fonts.catalog();
  expect(fontCatalog).toHaveBeenCalledOnce();
  expect(remoteInvoke).not.toHaveBeenCalled();
});

it("falls back only to bundled fonts when native enumeration fails", async () => {
  nativeInvoke.mockRejectedValue(new Error("Unavailable"));
  const bundled = { families: ["JetBrains Mono"], systemFontsAvailable: false };
  remoteInvoke.mockResolvedValue(bundled);
  expect(await tauriPlatform.fonts.catalog()).toBe(bundled);
  expect(await electronPlatform.fonts.catalog()).toBe(bundled);
  expect(remoteInvoke.mock.calls.map(([command]) => command)).toEqual(["bundled_font_catalog", "bundled_font_catalog"]);
});
