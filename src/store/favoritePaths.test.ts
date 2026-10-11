//! Per-project file favourites: pinning is scoped to the project the tree belongs to, toggling is
//! symmetric, and a project that ends up with none leaves no key behind in the persisted blob.

import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  createWorktree: vi.fn(),
  getSessionCwd: vi.fn().mockResolvedValue(null),
  ptyKill: vi.fn().mockResolvedValue(undefined),
  ptyWrite: vi.fn().mockResolvedValue(undefined),
  listShells: vi.fn().mockResolvedValue([]),
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

// Toggling runs the shared persist-and-apply path, which re-applies the visual theme and reads the
// prefers-color-scheme query. jsdom has no matchMedia, so stand one in rather than mocking the store's
// own persistence away -- the point is to exercise the real action.
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    onchange: null,
    dispatchEvent: () => false,
  }),
});

import { useTermStore } from "./termStore";

const favs = () => useTermStore.getState().favoritePaths;
const toggle = (project: string, path: string) =>
  useTermStore.getState().toggleFavoritePath(project, path);

beforeEach(() => {
  useTermStore.setState({ favoritePaths: {} });
});

describe("toggleFavoritePath", () => {
  it("pins a file under its own project", () => {
    toggle("proj-a", "/repo/src/main.ts");
    expect(favs()).toEqual({ "proj-a": ["/repo/src/main.ts"] });
  });

  it("keeps each project's pins separate, including the same path in two projects", () => {
    toggle("proj-a", "/shared/notes.md");
    toggle("proj-b", "/shared/notes.md");
    toggle("proj-b", "/repo-b/index.ts");
    expect(favs()["proj-a"]).toEqual(["/shared/notes.md"]);
    expect(favs()["proj-b"]).toEqual(["/shared/notes.md", "/repo-b/index.ts"]);
  });

  it("unpins on a second toggle of the same path", () => {
    toggle("proj-a", "/repo/a.ts");
    toggle("proj-a", "/repo/b.ts");
    toggle("proj-a", "/repo/a.ts");
    expect(favs()["proj-a"]).toEqual(["/repo/b.ts"]);
  });

  it("removes the project key entirely once its last pin goes", () => {
    toggle("proj-a", "/repo/only.ts");
    toggle("proj-a", "/repo/only.ts");
    // Not `{"proj-a": []}` -- an emptied project should leave nothing behind in the synced blob.
    expect("proj-a" in favs()).toBe(false);
    expect(favs()).toEqual({});
  });

  it("preserves insertion order so the pinned list does not reshuffle under the user", () => {
    toggle("p", "/z.ts");
    toggle("p", "/a.ts");
    toggle("p", "/m.ts");
    expect(favs()["p"]).toEqual(["/z.ts", "/a.ts", "/m.ts"]);
  });

  it("leaves other projects untouched when one is emptied", () => {
    toggle("keep", "/kept.ts");
    toggle("drop", "/gone.ts");
    toggle("drop", "/gone.ts");
    expect(favs()).toEqual({ keep: ["/kept.ts"] });
  });
});
