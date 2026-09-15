import { expect, it } from "vitest";
import { knowledgeParent } from "./KnowledgeBack";

const parent = (search: string) => {
  const found = knowledgeParent(search);
  return found && { route: found.route, ...Object.fromEntries(Object.entries(found.values).filter(([, value]) => value != null)) };
};

it("has no parent on the knowledge-base home", () => {
  expect(parent("")).toBeNull();
  expect(parent("?memory=notebooks")).toBeNull();
});

it("returns every top-level view to the home page", () => {
  for (const route of ["library", "jobs", "collections", "notebook/v", "notebooks/copy"]) expect(parent(`?memory=${route}`)).toEqual({ route: "notebooks" });
});

it("keeps the session scope when leaving an entry or its sub-pages", () => {
  expect(parent("?memory=entry/e&memorySession=s")).toEqual({ route: "library", memorySession: "s" });
  expect(parent("?memory=history/e/2&memoryProject=p")).toEqual({ route: "entry/e", memoryProject: "p" });
  expect(parent("?memory=edit/e")).toEqual({ route: "entry/e" });
  expect(parent("?memory=job/j&memoryJobPage=2")).toEqual({ route: "jobs" });
});

it("climbs the archived-session hierarchy", () => {
  expect(parent("?memory=collection/s&memoryCollectionProject=p&memoryCollectionTab=entries")).toEqual({ route: "collections", memoryCollectionProject: "p" });
  expect(parent("?memory=collections&memoryCollectionProject=p")).toEqual({ route: "collections" });
});

it("closes vault views before leaving the vault", () => {
  expect(parent("?memory=notebook/v/new&memoryFolder=Tea")).toEqual({ route: "notebook/v", memoryFolder: "Tea" });
  expect(parent("?memory=notebook/v&memoryFilter=trash")).toEqual({ route: "notebook/v" });
  expect(parent("?memory=notebook/v&memoryQuery=tea&memoryScope=all")).toEqual({ route: "notebook/v" });
  expect(parent("?memory=notebook/v&memoryFolder=Tea")).toEqual({ route: "notebook/v" });
});
