import { describe, expect, it } from "vitest";
import { directoriesToRefresh, reconcileChildren, toNode, updateFileNode, visibleDirectories } from "./fileTree";

const dir = (name: string, badge: string | null = null) => ({ name, isDir: true, isHidden: false, gitBadge: badge });
const file = (name: string, badge: string | null = null) => ({ ...dir(name, badge), isDir: false });

describe("Files-tree reconciliation", () => {
  it("updates backend facts while retaining expanded descendants", () => {
    const child = { ...toNode(dir("src"), "/repo"), open: true, loaded: true, children: [toNode(file("文档.txt", "M"), "/repo/src")] };
    const root = { ...toNode(dir("repo"), ""), path: "/repo", open: true, loaded: true, children: [child] };
    const next = reconcileChildren(root, [dir("src", "M"), file("new.txt", "U")]);
    expect(next.children?.[0].open).toBe(true);
    expect(next.children?.[0].children).toBe(child.children);
    expect(next.children?.[0].badge).toBe("M");
    const updated = updateFileNode(next, "/repo/src", (node) => reconcileChildren(node, [file("文档.txt")]));
    expect(updated.children?.[0].children?.[0].badge).toBeNull();
    expect(updated.children?.[0].open).toBe(true);
    expect(visibleDirectories(updated)).toEqual(["/repo", "/repo/src"]);
  });

  it("removes disappeared nodes and resets a node when its type changes", () => {
    const root = { ...toNode(dir("repo"), ""), path: "/repo", children: [{ ...toNode(dir("same"), "/repo"), open: true }, toNode(file("gone"), "/repo")] };
    const next = reconcileChildren(root, [file("same", "M")]);
    expect(next.children).toHaveLength(1);
    expect(next.children?.[0].isDir).toBe(false);
    expect(next.children?.[0].open).toBe(false);
  });
});

describe("directoriesToRefresh", () => {
  const tree = {
    name: "r", path: "/r", isDir: true, isHidden: false, open: true, loaded: true,
    children: [
      { name: "src", path: "/r/src", isDir: true, isHidden: false, open: true, loaded: true, children: [
        { name: "app", path: "/r/src/app", isDir: true, isHidden: false, open: false, loaded: true, children: [] },
      ] },
      { name: "docs", path: "/r/docs", isDir: true, isHidden: false, open: false, loaded: false },
    ],
  };

  it("re-reads only the shown directories an event names", () => {
    expect(directoriesToRefresh(tree, ["", "src", "src/app", "docs", "node_modules/x"], false)).toEqual(["/r", "/r/src"]);
    expect(directoriesToRefresh(tree, ["src/app"], false)).toEqual([]);
  });

  it("re-reads everything shown for an unlisted burst or a git change", () => {
    expect(directoriesToRefresh(tree, null, false)).toEqual(["/r", "/r/src"]);
    expect(directoriesToRefresh(tree, [], true)).toEqual(["/r", "/r/src"]);
  });
});
