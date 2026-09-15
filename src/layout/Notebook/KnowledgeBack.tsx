//! "Up one level" for every knowledge-base page. The parent comes from the route hierarchy rather than
//! browser history, so the button never leaves the knowledge base or bounces between sibling views.
import Icons from "../../components/Icons";
import { useT } from "../../i18n";
import { MemoryLink, useMemoryLocation } from "../Memory/navigation";
import { knowledgeSelection } from "./KnowledgeTreeRow";

type ParentLocation = { route: string; values: Record<string, string | null> };

/** The page one level above the current knowledge-base location, or null on the knowledge-base home. */
export function knowledgeParent(search: string): ParentLocation | null {
  const params = new URLSearchParams(search);
  const [page = "", id = "", action = ""] = (params.get("memory") ?? "").split("/");
  const at = (route: string, values: Record<string, string | null> = {}): ParentLocation =>
    ({ route, values: { ...knowledgeSelection, memoryScope: null, memoryJobPage: null, ...values } });
  const home = at("notebooks");
  const project = params.get("memoryProject"), session = params.get("memorySession");
  switch (page) {
    case "":
      return null;
    case "notebooks":
      return id ? home : null;
    case "library":
      return session ? at("library", { memoryProject: project }) : project ? at("library") : home;
    case "entry":
    case "new":
    case "source":
    case "compile":
      return at("library", { memoryProject: project, memorySession: session });
    case "history":
    case "edit":
      return at(`entry/${id}`, { memoryProject: project, memorySession: session });
    case "job":
      return at("jobs");
    case "collection":
      return at("collections", { memoryCollectionProject: params.get("memoryCollectionProject") });
    case "collections":
      return params.has("memoryCollectionProject") ? at("collections") : home;
    case "notebook": {
      const vault = `notebook/${id}`;
      const folder = params.get("memoryFolder") ?? "";
      if (action) return at(vault, { memoryFolder: folder || null, memoryFilter: params.get("memoryFilter"), memoryTag: params.get("memoryTag") });
      if (params.get("memoryQuery") || params.get("memoryFilter") || params.get("memoryTag")) return at(vault, { memoryFolder: folder || null });
      if (folder) return at(vault, { memoryFolder: folder.includes("/") ? folder.slice(0, folder.lastIndexOf("/")) : null });
      return home;
    }
    default:
      return home;
  }
}

export function KnowledgeBackLink({ className }: { className: string }) {
  const t = useT();
  const parent = knowledgeParent(useMemoryLocation());
  if (!parent) return null;
  return <MemoryLink className={className} route={parent.route} values={parent.values} title={t("memory.up")} aria-label={t("memory.up")}>
    <Icons.arrowLeft size={15} aria-hidden="true" />
  </MemoryLink>;
}
