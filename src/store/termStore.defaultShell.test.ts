//! Changing the default shell must refresh the cached shell list. The backend marks `isDefault` from the saved
//! preference, and the terminal header names a session without an explicit shell after that entry, so a list
//! cached at startup would keep naming the old default for a terminal that launched the new one.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { listShells, flushNow } = vi.hoisted(() => ({
  listShells: vi.fn(),
  flushNow: vi.fn(),
}));

vi.mock("../ipc/commands", () => ({
  createWorktree: vi.fn(),
  getSessionCwd: vi.fn().mockResolvedValue(null),
  ptyKill: vi.fn().mockResolvedValue(undefined),
  ptyWrite: vi.fn().mockResolvedValue(undefined),
  listShells,
}));
vi.mock("../ipc/settingsSync", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../ipc/settingsSync")>()),
  flushNow,
}));
vi.mock("../ipc/tree", () => ({
  listTree: vi.fn().mockResolvedValue({ projects: [], groups: [], sessions: [] }),
}));
vi.mock("../notify", () => ({
  notify: vi.fn(),
  getNotifyPermission: vi.fn().mockResolvedValue("granted"),
  requestNotifyPermission: vi.fn().mockResolvedValue("granted"),
  getEffectiveNotifyPermission: vi.fn().mockResolvedValue("granted"),
  requestEffectiveNotifyPermission: vi.fn().mockResolvedValue("granted"),
}));

import { shellDisplayName, useTermStore } from "./termStore";
import type { ShellOption } from "../ipc/commands";

const shell = (id: string, label: string, isDefault: boolean): ShellOption => ({
  id,
  label,
  path: `/shells/${id}`,
  isDefault,
});

beforeEach(() => {
  // jsdom has no matchMedia, and persisting settings re-resolves the theme.
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
  );
  listShells.mockReset();
  flushNow.mockReset();
  useTermStore.setState({
    defaultShell: "",
    shells: [shell("powershell", "Windows PowerShell", true), shell("pwsh", "PowerShell 7", false)],
  });
});

afterEach(() => vi.unstubAllGlobals());

describe("setDefaultShell", () => {
  it("re-reads the shell list once the preference is saved, so new terminals are labelled correctly", async () => {
    let saved!: () => void;
    flushNow.mockReturnValue(new Promise<void>((resolve) => (saved = resolve)));
    listShells.mockResolvedValue([
      shell("powershell", "Windows PowerShell", false),
      shell("pwsh", "PowerShell 7", true),
    ]);

    useTermStore.getState().setDefaultShell("/shells/pwsh");
    // The backend computes `isDefault` from the stored value, so asking before the write lands would return
    // the old marking.
    expect(listShells).not.toHaveBeenCalled();

    saved();
    await vi.waitFor(() => expect(shellDisplayName(useTermStore.getState().shells, null)).toBe("PowerShell 7"));
    expect(listShells).toHaveBeenCalledTimes(1);
  });

  it("keeps the cached list when detection fails", async () => {
    flushNow.mockResolvedValue(undefined);
    listShells.mockRejectedValue(new Error("offline"));
    const before = useTermStore.getState().shells;

    useTermStore.getState().setDefaultShell("/shells/pwsh");
    await vi.waitFor(() => expect(listShells).toHaveBeenCalled());
    await Promise.resolve();
    expect(useTermStore.getState().shells).toBe(before);
  });
});
