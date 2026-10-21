import { describe, expect, it } from "vitest";
import type { Project } from "../../types";
import { arrangeProjectsInCollections } from "./projectCollectionLayout";
import { alphabeticalOrders, resolveProjectDrop, resolveProjectPlacement } from "./projectCollectionDrop";
import { moveToCollectionItem } from "./projectCollectionMenu";
import { t } from "../../i18n";

const project = (id: string, collectionId: string | null = null, rootPath = `/tmp/${id}`): Project => ({
  id, name: id, collectionId, rootPath, collapsed: false, sortOrder: 0, createdAt: 0,
});

describe("collection membership projection", () => {
  const work = project("work", null, "");
  const notes = project("notes", "work", "");
  const repo = project("repo", "notes");
  const loose = project("loose");
  const projects = [loose, repo, work, notes];

  it("preserves nested membership, sibling order, counts and indentation", () => {
    expect(arrangeProjectsInCollections(projects, () => true).map(e => [e.project.id, e.indent, e.projectCount]))
      .toEqual([["work", 0, 1], ["notes", 1, 1], ["repo", 2, 0], ["loose", 0, 0]]);
  });
  it("hides an entire collapsed branch while keeping loose projects", () => {
    expect(arrangeProjectsInCollections(projects, p => p.id !== "work").map(e => e.project.id)).toEqual(["work", "loose"]);
  });
  it("renders orphaned projects and malformed cycles without dropping or repeating rows", () => {
    const bad = [project("a", "b", ""), project("b", "a", ""), project("orphan", "gone")];
    expect(new Set(arrangeProjectsInCollections(bad, () => true).map(e => e.project.id))).toEqual(new Set(["a", "b", "orphan"]));
  });
  it("rejects self-membership and cycles in both drag and menu choices", () => {
    expect(resolveProjectDrop({ kind: "project", id: "work" }, notes, projects)).toBeNull();
    expect(resolveProjectDrop({ kind: "project", id: "work" }, work, projects)).toBeNull();
    const menu = moveToCollectionItem(t, work, projects, () => {});
    expect(menu?.submenu?.filter(i => i.label === "work" || i.label === "notes").every(i => i.disabled)).toBe(true);
  });
  it("joins a collection, joins another project's collection, or detaches beside a loose project", () => {
    expect(resolveProjectDrop({ kind: "project", id: "loose" }, notes, projects)).toEqual({ projectId: "loose", collectionId: "notes" });
    expect(resolveProjectDrop({ kind: "project", id: "loose" }, repo, projects)).toEqual({ projectId: "loose", collectionId: "notes" });
    expect(resolveProjectDrop({ kind: "project", id: "repo" }, loose, projects)).toEqual({ projectId: "repo", collectionId: null });
    expect(resolveProjectDrop({ kind: "session", id: "repo" }, work, projects)).toBeNull();
  });
});

describe("hand ordering within a list", () => {
  const at = (p: Project, sortOrder: number, name = p.name): Project => ({ ...p, sortOrder, name });
  const work = at(project("work", null, ""), 100);
  const home = at(project("home", null, ""), 200);
  const a = at(project("a", "work"), 10);
  const b = at(project("b", "work"), 20);
  const c = at(project("c", "work"), 30);
  const loose = at(project("loose"), 300);
  const projects = [work, home, a, b, c, loose];
  const drag = (id: string) => ({ kind: "project", id });

  it("moves a project above or below a sibling in its own collection", () => {
    expect(resolveProjectPlacement(drag("c"), a, "top", projects, 9999))
      .toEqual({ projectId: "c", collectionId: "work", changesCollection: false, sortOrder: -990 });
    expect(resolveProjectPlacement(drag("a"), b, "bottom", projects, 9999))
      .toEqual({ projectId: "a", collectionId: "work", changesCollection: false, sortOrder: 25 });
    expect(resolveProjectPlacement(drag("a"), c, "bottom", projects, 9999)?.sortOrder).toBe(9999);
  });
  it("reorders collections among themselves", () => {
    expect(resolveProjectPlacement(drag("home"), work, "top", projects, 9999))
      .toEqual({ projectId: "home", collectionId: null, changesCollection: false, sortOrder: -900 });
  });
  it("joins the target's collection at an edge, and keeps the old center behavior", () => {
    expect(resolveProjectPlacement(drag("loose"), b, "top", projects, 9999))
      .toEqual({ projectId: "loose", collectionId: "work", changesCollection: true, sortOrder: 15 });
    expect(resolveProjectPlacement(drag("loose"), work, "center", projects))
      .toEqual({ projectId: "loose", collectionId: "work", changesCollection: true, sortOrder: null });
  });
  it("lands at the near end of its own kind beside a row of the other kind", () => {
    // Collections list first, so a project dropped beside one becomes the first project.
    expect(resolveProjectPlacement(drag("a"), home, "top", projects, 9999))
      .toEqual({ projectId: "a", collectionId: null, changesCollection: true, sortOrder: -700 });
    expect(resolveProjectPlacement(drag("home"), loose, "top", projects, 9999)?.sortOrder).toBe(9999);
  });
  it("rejects self-drops and a collection placed inside itself", () => {
    expect(resolveProjectPlacement(drag("a"), a, "top", projects)).toBeNull();
    expect(resolveProjectPlacement(drag("work"), a, "top", projects)).toBeNull();
  });
  it("sorts a list by name, collections first, numbers in numeric order", () => {
    const list = [at(project("z", null, ""), 1, "Zeta"), at(project("p10"), 2, "proj 10"),
      at(project("p9"), 3, "Proj 9"), at(project("y", null, ""), 4, "alpha"), at(project("in", "z"), 5, "Inside")];
    expect(alphabeticalOrders(list, null)).toEqual([
      { id: "y", sortOrder: 1000 }, { id: "z", sortOrder: 2000 }, { id: "p9", sortOrder: 3000 }, { id: "p10", sortOrder: 4000 },
    ]);
    expect(alphabeticalOrders(list, "z")).toEqual([{ id: "in", sortOrder: 1000 }]);
  });
});
