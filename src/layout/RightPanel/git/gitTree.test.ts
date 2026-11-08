import { expect, it } from "vitest";
import type { ChangedFile } from "../../../ipc/commands";
import { baseName, buildGitRows } from "./gitTree";

const f = (path: string) => ({ path, status: "modified", index: "", worktree: "modified" }) as ChangedFile;

it("lists every file flat in list mode", () => {
  const rows = buildGitRows([f("a/b.txt"), f("c.txt")], "list", new Set());
  expect(rows.map((r) => r.kind === "file" && r.file.path)).toEqual(["a/b.txt", "c.txt"]);
});

it("nests files under folders, compacting single-folder chains", () => {
  const rows = buildGitRows([f("src/app/ui/a.ts"), f("src/app/ui/b.ts"), f("src/main.ts"), f("README.md")], "tree", new Set());
  expect(rows.map((r) => (r.kind === "dir" ? `${r.depth}:${r.name}/(${r.files.length})` : `${r.depth}:${r.file.path}`))).toEqual([
    "0:src/(3)",
    "1:app/ui/(2)",
    "2:src/app/ui/a.ts",
    "2:src/app/ui/b.ts",
    "1:src/main.ts",
    "0:README.md",
  ]);
});

it("hides the contents of collapsed folders but keeps their file count", () => {
  const rows = buildGitRows([f("src/a.ts"), f("src/b.ts"), f("x.ts")], "tree", new Set(["src"]));
  expect(rows).toHaveLength(2);
  expect(rows[0]).toMatchObject({ kind: "dir", path: "src", open: false });
  expect(rows[0].kind === "dir" && rows[0].files).toHaveLength(2);
});

it("keeps an untracked directory as a single row", () => {
  const rows = buildGitRows([f("build/")], "tree", new Set());
  expect(rows).toEqual([{ kind: "file", file: f("build/"), depth: 0 }]);
  expect(baseName("a/build/")).toBe("build/");
  expect(baseName("a/b.txt")).toBe("b.txt");
});
