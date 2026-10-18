//! Knowledge-base Collections: archived sessions kept alongside knowledge entries. Both the right tree
//! and the main area arrange archived sessions as they sat in the session tree — project, group chain,
//! then parent and child sessions. The main area also searches their full content and reads one
//! conversation together with the entries generated from it.
//!
//! The archive overlay that used to host this functionality was removed; restore, export, delete and
//! organize actions all live here now.

import { Fragment, useEffect, useMemo, useState, type ReactNode } from "react";
import Icons from "../../components/Icons";
import { canExportContext, exportSessionToFile } from "../../exportSession";
import { dateLocale, useT } from "../../i18n";
import { memoryCollections, memoryList, type MemoryCollectionProject, type MemoryCollectionSession } from "../../ipc/memory";
import { useTermStore } from "../../store/termStore";
import type { Session } from "../../types";
import { ContentSearchControls, SearchConsole, useContentSearch, useSearchNav } from "../GlobalSearch/searchKit";
import { KnowledgeTreeRow, knowledgeSelection } from "../Notebook/KnowledgeTreeRow";
import { SessionContentViewer } from "../sessionViewers/SessionContentViewer";
import { fmtArchivedAt, KindIcon, locationOf } from "../sessionViewers/sessionMeta";
import { MemoryLink, memoryNavigate, memoryUrl, useMemoryLocation } from "./navigation";
import { LoadState, MemoryIcon, memoryTime, useMemoryLoad } from "./shared";
import "./memory.css";

/** Window event that tells every collections surface to reload after a restore or permanent delete. */
const COLLECTIONS_CHANGED = "memory:collections";

function projectName(project: MemoryCollectionProject, fallback: string): string {
  return project.name || fallback;
}

/** One node of a project's archive tree: a group from the original container chain, or a session. */
export type CollectionNode =
  | { kind: "group"; key: string; name: string; children: CollectionNode[] }
  | { kind: "session"; key: string; session: MemoryCollectionSession; children: CollectionNode[] };

const groupKey = (id: string) => `g:${id}`;
const sessionKey = (id: string) => `s:${id}`;

/** Arrange one project's archived sessions as they sat in the session tree: roots under their group
 *  chain, descendants under their archived parent. Groups lead each level by name; sessions keep the
 *  backend's newest-archived-first order. */
export function collectionTree(sessions: MemoryCollectionSession[]): CollectionNode[] {
  const roots: CollectionNode[] = [];
  const groups = new Map<string, CollectionNode>();
  const nodes = new Map<string, CollectionNode>(sessions.map((session) => [session.id, { kind: "session", key: sessionKey(session.id), session, children: [] }]));
  for (const session of sessions) {
    const node = nodes.get(session.id)!;
    const parent = session.parentId ? nodes.get(session.parentId) : undefined;
    if (parent) { parent.children.push(node); continue; }
    let level = roots;
    // An older remote backend omits group IDs; its roots then sit directly under the project.
    (session.groupIds ?? []).forEach((id, index) => {
      let group = groups.get(id);
      if (!group) {
        group = { kind: "group", key: groupKey(id), name: session.groupPath[index] ?? "", children: [] };
        groups.set(id, group);
        level.push(group);
      }
      level = group.children;
    });
    level.push(node);
  }
  const rank = (node: CollectionNode) => (node.kind === "group" ? 0 : 1);
  const order = (level: CollectionNode[]) => {
    level.sort((a, b) => rank(a) - rank(b) || (a.kind === "group" && b.kind === "group" ? a.name.localeCompare(b.name) : 0));
    for (const node of level) order(node.children);
  };
  order(roots);
  return roots;
}

/** Groups and parent sessions above one archived session, top-down, so a tree can expand the path
 *  to it and labels can show where it was stored. */
export function collectionLineage(byId: Map<string, MemoryCollectionSession>, id: string): { key: string; name: string; sessionId?: string }[] {
  const lineage: { key: string; name: string; sessionId?: string }[] = [];
  let current = byId.get(id);
  const seen = new Set<string>();
  while (current?.parentId && !seen.has(current.id)) {
    seen.add(current.id);
    const parent = byId.get(current.parentId);
    if (!parent) break;
    lineage.unshift({ key: sessionKey(parent.id), name: parent.name, sessionId: parent.id });
    current = parent;
  }
  if (current) lineage.unshift(...(current.groupIds ?? []).map((group, index) => ({ key: groupKey(group), name: current!.groupPath[index] ?? "" })));
  return lineage;
}

/** Compact archive time for a single-line row; the year appears only when it is not the current one.
 *  The full localized time stays available in the row's tooltip. */
function archivedShort(ts?: number | null): string {
  if (!ts) return "";
  const date = new Date(ts * 1000);
  const year = date.getFullYear() === new Date().getFullYear() ? undefined : "numeric";
  return date.toLocaleString(dateLocale(), { year, month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

const projectKey = (id: string) => `p:${id}`;

function sessionCount(nodes: CollectionNode[]): number {
  return nodes.reduce((sum, node) => sum + (node.kind === "session" ? 1 : 0) + sessionCount(node.children), 0);
}

/** Every archived session row by ID, across projects. */
function useCollectionRows(data: { projects: MemoryCollectionProject[] } | null) {
  return useMemo(() => new Map((data?.projects ?? []).flatMap((project) => project.sessions.map((session) => [session.id, session] as const))), [data]);
}

/** Right-panel tree: archived sessions under the project, groups and parent sessions they were archived from. */
export function MemoryCollectionsDirectory() {
  const t = useT();
  const location = useMemoryLocation();
  const params = new URLSearchParams(location);
  const [page, id = ""] = (params.get("memory") ?? "").split("/");
  const projectId = params.get("memoryCollectionProject") ?? "";
  const { data, error, reload } = useMemoryLoad(memoryCollections, []);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const open = (...keys: string[]) => setExpanded((old) => (keys.every((key) => old.has(key)) ? old : new Set([...old, ...keys])));
  const toggle = (key: string) =>
    setExpanded((old) => {
      const next = new Set(old);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  const rowById = useCollectionRows(data);
  const trees = useMemo(() => new Map((data?.projects ?? []).map((project) => [project.id, collectionTree(project.sessions)])), [data]);
  useEffect(() => {
    const project = projectId || data?.projects.find((p) => p.sessions.some((s) => s.id === id))?.id;
    if (project) open(project, ...(id ? collectionLineage(rowById, id).map((node) => node.key) : []));
  }, [data, projectId, id, rowById]);
  useEffect(() => {
    window.addEventListener(COLLECTIONS_CHANGED, reload);
    return () => window.removeEventListener(COLLECTIONS_CHANGED, reload);
  }, [reload]);
  const sessionById = useMemo(() => new Map((data?.sessions ?? []).map((s) => [s.id, s])), [data]);
  const branches = (project: MemoryCollectionProject, nodes: CollectionNode[], depth: number) => <ul className="nb-tree-branches">
    {nodes.map((node) => {
      if (node.kind === "group") {
        return <KnowledgeTreeRow key={node.key} name={node.name} depth={depth} icon={<Icons.folder size={15} />}
          expanded={expanded.has(node.key)} onToggle={() => toggle(node.key)} onActivate={() => toggle(node.key)}>
          {branches(project, node.children, depth + 1)}
        </KnowledgeTreeRow>;
      }
      const session = node.session;
      const live = sessionById.get(session.id);
      const nested = node.children.length > 0;
      return <KnowledgeTreeRow key={node.key} name={session.name} route={`collection/${session.id}`}
        values={{ ...knowledgeSelection, memoryCollectionProject: project.id, memoryCollectionQuery: null }} depth={depth}
        icon={live ? <KindIcon session={live} size={15} /> : <Icons.bot size={15} />} count={session.count || undefined}
        title={[projectName(project, t("memory.unknownProject")), ...collectionLineage(rowById, session.id).map((item) => item.name), session.name].filter(Boolean).join(" › ")}
        selected={page === "collection" && id === session.id}
        expanded={nested ? expanded.has(node.key) : undefined} onToggle={nested ? () => toggle(node.key) : undefined}>
        {nested && branches(project, node.children, depth + 1)}
      </KnowledgeTreeRow>;
    })}
  </ul>;
  return <ul className="nb-tree-branches memory-directory">
    {(!data || error) && <li><LoadState error={error} reload={reload} /></li>}
    {data && !data.projects.length && <li className="nb-tree-empty">{t("archive.empty1")}</li>}
    {data?.projects.map((project) => <KnowledgeTreeRow key={project.id} name={projectName(project, t("memory.unknownProject"))}
      route="collections" values={{ ...knowledgeSelection, memoryCollectionProject: project.id, memoryCollectionQuery: null }} depth={1}
      icon={<Icons.folder size={15} />} count={project.count || undefined}
      selected={page === "collections" && projectId === project.id && !id}
      expanded={expanded.has(project.id)} onToggle={() => toggle(project.id)} onOpen={() => open(project.id)}>
      {branches(project, trees.get(project.id) ?? [], 2)}
    </KnowledgeTreeRow>)}
  </ul>;
}

/** Main area: browse, archive-wide search, and one archived conversation with its knowledge entries. */
export function MemoryCollections({ sessionId }: { sessionId: string }) {
  const t = useT();
  const location = useMemoryLocation();
  const params = new URLSearchParams(location);
  const query = params.get("memoryCollectionQuery") ?? "";
  const projectId = params.get("memoryCollectionProject") ?? "";
  const { data, error, reload } = useMemoryLoad(memoryCollections, []);
  const loadArchived = useTermStore((s) => s.loadArchived);
  const restoreSession = useTermStore((s) => s.restoreSession);
  const deleteNode = useTermStore((s) => s.deleteNode);
  const liveProjects = useTermStore((s) => s.projects);
  const groups = useTermStore((s) => s.groups);
  const liveSessions = useTermStore((s) => s.sessions);
  useEffect(() => { void loadArchived(); }, [loadArchived]);
  useEffect(() => {
    window.addEventListener(COLLECTIONS_CHANGED, reload);
    return () => window.removeEventListener(COLLECTIONS_CHANGED, reload);
  }, [reload]);
  const search = useContentSearch("archived", query);
  const nav = useSearchNav(search.results);
  const totalMatches=search.page?.totalMatches ?? 0;const matchedSessions=search.page?.totalSessions ?? 0;
  const sessionById = useMemo(() => new Map((data?.sessions ?? []).map((s) => [s.id, s])), [data]);
  const rowById = useCollectionRows(data);
  const trees = useMemo(() => new Map((data?.projects ?? []).map((item) => [item.id, collectionTree(item.sessions)])), [data]);
  // Every branch starts expanded; the set records what the reader folded away.
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const fold = (key: string) =>
    setCollapsed((old) => {
      const next = new Set(old);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  const session = sessionId ? sessionById.get(sessionId) : undefined;
  const project = data?.projects.find((p) => p.sessions.some((s) => s.id === sessionId)) ?? data?.projects.find((p) => p.id === projectId);
  const row = project?.sessions.find((s) => s.id === sessionId);
  const lineage = sessionId ? collectionLineage(rowById, sessionId) : [];
  const projectLabel = project ? projectName(project, t("memory.unknownProject")) : "";
  // Tombstoned projects and groups are not in the live tree, so search results use the stored path.
  const storedPaths = useMemo(() => {
    const paths = new Map<string, string>();
    for (const item of data?.projects ?? []) {
      const name = projectName(item, t("memory.unknownProject"));
      for (const archived of item.sessions) paths.set(archived.id, [name, ...collectionLineage(rowById, archived.id).map((node) => node.name)].filter(Boolean).join(" › "));
    }
    return paths;
  }, [data, rowById, t]);
  // The default tab follows the stored entry count; an explicit choice is carried in the URL.
  const storedTab = params.get("memoryCollectionTab");
  const tab = storedTab === "conversation" || storedTab === "entries" ? storedTab : row?.count ? "entries" : "conversation";
  const refresh = () => { reload(); void loadArchived(); };
  const changed = () => { window.dispatchEvent(new Event(COLLECTIONS_CHANGED)); void loadArchived(); };
  // Restoring or deleting a session takes its whole subtree, so the open session may go with an ancestor.
  const leaves = (id: string) => id === sessionId || lineage.some((node) => node.sessionId === id);
  const restore = async (id: string) => {
    await restoreSession(id);
    changed();
    if (leaves(id)) memoryNavigate(memoryUrl("collections", { memoryCollectionProject: project?.id ?? null }));
  };
  const remove = async (id: string) => {
    await deleteNode("session", id);
    await loadArchived();
    window.dispatchEvent(new Event(COLLECTIONS_CHANGED));
    if (leaves(id)) memoryNavigate(memoryUrl("collections", { memoryCollectionProject: project?.id ?? null }));
  };
  // A nested session is restored with its archived parent, never on its own below an archived parent.
  const actions = (target: Session) => <>
    {!rowById.get(target.id)?.parentId && <button type="button" className="memory-toolbar-button" title={t("archive.restore")} aria-label={t("archive.restore")} onClick={() => void restore(target.id)}><Icons.restart size={14} aria-hidden="true" /></button>}
    <MemoryLink route={`compile/${target.id}`} className="memory-toolbar-button" title={t("memory.add")} aria-label={t("memory.add")}><MemoryIcon size={14} /></MemoryLink>
    {canExportContext(target) && <button type="button" className="memory-toolbar-button" title={t("archive.export")} aria-label={t("archive.export")} onClick={() => void exportSessionToFile(target)}><Icons.download size={14} aria-hidden="true" /></button>}
    <button type="button" className="memory-toolbar-button" title={t("archive.deleteForever")} aria-label={t("archive.deleteForever")} style={{ color: "var(--status-error)" }} onClick={() => void remove(target.id)}><Icons.trash size={14} aria-hidden="true" /></button>
  </>;
  const totalSessions = data?.projects.reduce((sum, item) => sum + item.sessions.length, 0) ?? 0;
  const visibleProjects = data ? (projectId && project ? [project] : data.projects) : [];
  const allCollapsed = visibleProjects.length > 0 && visibleProjects.every((item) => collapsed.has(projectKey(item.id)));
  const foldAll = () =>
    setCollapsed((old) => {
      const next = new Set(old);
      for (const item of visibleProjects) {
        if (allCollapsed) next.delete(projectKey(item.id));
        else next.add(projectKey(item.id));
      }
      return next;
    });
  const branches = (item: MemoryCollectionProject, nodes: CollectionNode[]): ReactNode => nodes.map((node) => {
    const open = !collapsed.has(node.key);
    const children = node.children.length > 0 && open && <div className="memory-collection-children">{branches(item, node.children)}</div>;
    const chevron = open ? <Icons.chevD size={12} aria-hidden="true" /> : <Icons.chevR size={12} aria-hidden="true" />;
    if (node.kind === "group") {
      return <div key={node.key} className="memory-collection-branch">
        <button type="button" className="memory-collection-folder" aria-expanded={open} onClick={() => fold(node.key)}>
          <span className="memory-collection-toggle" aria-hidden="true">{chevron}</span><Icons.folder size={14} aria-hidden="true" /><span>{node.name}</span><small>{sessionCount(node.children)}</small>
        </button>
        {children}
      </div>;
    }
    const archived = node.session;
    const live = sessionById.get(archived.id);
    return <div key={node.key} className="memory-collection-branch">
      <div className="memory-collection-row" title={`${archived.name}\n${fmtArchivedAt(archived.archivedAt)}`}>
        {node.children.length > 0
          ? <button type="button" className="memory-collection-toggle" aria-label={archived.name} aria-expanded={open} onClick={() => fold(node.key)}>{chevron}</button>
          : <span className="memory-collection-toggle" aria-hidden="true" />}
        <MemoryLink route={`collection/${archived.id}`} values={{ memoryCollectionProject: item.id, memoryCollectionTab: null }} className="memory-collection-row-link">
          <span className="memory-collection-icon">{live ? <KindIcon session={live} size={14} /> : <Icons.bot size={14} />}</span>
          <span className="memory-collection-title">{archived.name}</span>
          {archived.count > 0 && <span className="memory-collection-count" title={t("memory.entries")}>{archived.count}</span>}
        </MemoryLink>
        <span className="memory-collection-tail">
          <time className="memory-collection-time" dateTime={archived.archivedAt ? new Date(archived.archivedAt * 1000).toISOString() : undefined}>{archivedShort(archived.archivedAt)}</time>
          {live && <span className="memory-collection-actions">{actions(live)}</span>}
        </span>
      </div>
      {children}
    </div>;
  });
  return <main className="memory-browser-main">
    <div className="memory-browser-toolbar" data-reading={!!sessionId}>
      <div className="memory-browser-location">
        <nav aria-label={t("memory.hierarchy")}>
          <MemoryLink route="collections" values={{ memoryCollectionProject: null, memoryCollectionQuery: null, memoryCollectionTab: null }}>{t("memory.collections")}</MemoryLink>
          {project && <><span aria-hidden="true">/</span><MemoryLink route="collections" values={{ memoryCollectionProject: project.id, memoryCollectionQuery: null, memoryCollectionTab: null }}>{projectLabel}</MemoryLink></>}
          {lineage.filter((node) => node.sessionId).map((node) => <Fragment key={node.key}><span aria-hidden="true">/</span><MemoryLink route={`collection/${node.sessionId}`} values={{ memoryCollectionProject: project?.id ?? null, memoryCollectionQuery: null, memoryCollectionTab: null }}>{node.name}</MemoryLink></Fragment>)}
          {session && <><span aria-hidden="true">/</span><MemoryLink route={`collection/${session.id}`} values={{ memoryCollectionProject: project?.id ?? null, memoryCollectionQuery: null, memoryCollectionTab: null }}>{session.name}</MemoryLink></>}
        </nav>
        {!sessionId && data && <span className="memory-browser-count" title={t("memory.collections")}>{totalSessions}</span>}
      </div>
      {sessionId ? <div className="memory-browser-controls memory-collection-actions">
        {session && actions(session)}
      </div> : <div className="memory-browser-controls">
        <label className="memory-browser-search">
          <Icons.search size={15} aria-hidden="true" />
          <input aria-label={t("archive.searchPlaceholder")} placeholder={t("archive.searchPlaceholder")} value={query}
            onChange={(e) => memoryNavigate(memoryUrl("collections", { memoryCollectionProject: null, memoryCollectionQuery: e.target.value || null, memoryCollectionTab: null }), true)} />
        </label>
        <div className="memory-browser-filters">
          {query.trim() && <span className="memory-browser-count">{search.loading ? t("search.searching") : t("search.summary", totalMatches, matchedSessions)}</span>}
          {!query.trim() && visibleProjects.length > 1 && <button type="button" className="memory-toolbar-button"
            aria-label={t(allCollapsed ? "memory.expandAllProjects" : "memory.collapseAllProjects")}
            title={t(allCollapsed ? "memory.expandAllProjects" : "memory.collapseAllProjects")} onClick={foldAll}>
            {allCollapsed ? <Icons.expand size={15} aria-hidden="true" /> : <Icons.collapse size={15} aria-hidden="true" />}
          </button>}
          <button type="button" className="memory-toolbar-button" aria-label={t("common.refresh")} title={t("common.refresh")} onClick={refresh}><Icons.restart size={15} aria-hidden="true" /></button>
        </div>
      </div>}
    </div>
    <div className="memory-browser-content" data-collection={sessionId ? "session" : query.trim() ? "search" : "browse"}>
      {!data ? <LoadState error={error} reload={refresh} /> : sessionId ? (
        (!session || !row) ? <p className="memory-empty">{t("memory.notFound")}</p> : <>
          <div className="memory-collection-tabs" role="tablist">
            <MemoryLink role="tab" aria-selected={tab === "conversation"} className={`memory-collection-tab${tab === "conversation" ? " active" : ""}`}
              route={`collection/${session.id}`} values={{ memoryCollectionProject: project?.id ?? null, memoryCollectionTab: "conversation", memoryCollectionQuery: null }}>{t("memory.collectionConversation")}</MemoryLink>
            <MemoryLink role="tab" aria-selected={tab === "entries"} className={`memory-collection-tab${tab === "entries" ? " active" : ""}`}
              route={`collection/${session.id}`} values={{ memoryCollectionProject: project?.id ?? null, memoryCollectionTab: "entries", memoryCollectionQuery: null }}>{t("memory.entries")}{row.count ? ` · ${row.count}` : ""}</MemoryLink>
          </div>
          {tab === "conversation"
            ? <div className="memory-collection-viewer"><SessionContentViewer key={session.id} session={session} /></div>
            : <CollectionEntries sessionId={session.id} />}
        </>) : query.trim() ? (
        <><ContentSearchControls search={search} /><SearchConsole nav={nav} results={search.results} sessionById={sessionById} projects={liveProjects} groups={groups} liveSessions={liveSessions}
          sessionLocation={(s) => storedPaths.get(s.id) ?? locationOf(s, liveProjects, groups, liveSessions)}
          renderGroupActions={(_hit, target) => (target ? actions(target) : null)}
          emptyHint={search.error ?? (search.loading ? t("search.searching") : t("search.noResults"))} /></>
      ) : (
        <div className="memory-collection-groups">
          {!data.projects.length ? <p className="memory-empty">{t("archive.empty1")}<br />{t("archive.empty2")}</p>
            : visibleProjects.map((item) => {
              const open = !collapsed.has(projectKey(item.id));
              const name = projectName(item, t("memory.unknownProject"));
              return <section key={item.id} className="memory-collection-group">
                <h3>
                  <button type="button" className="memory-collection-project" aria-expanded={open} title={name} onClick={() => fold(projectKey(item.id))}>
                    <span className="memory-collection-toggle" aria-hidden="true">{open ? <Icons.chevD size={12} /> : <Icons.chevR size={12} />}</span>
                    <Icons.folder size={14} aria-hidden="true" />
                    <span className="memory-collection-project-name">{name}</span>
                    <small>{item.sessions.length}</small>
                  </button>
                </h3>
                {open && <div className="memory-collection-rows">
                  {branches(item, trees.get(item.id) ?? [])}
                </div>}
              </section>;
            })}
        </div>
      )}
    </div>
  </main>;
}

/** Knowledge entries generated from one archived session, with the organize entry when empty. */
function CollectionEntries({ sessionId }: { sessionId: string }) {
  const t = useT();
  const { data, error, reload } = useMemoryLoad(() => memoryList({ query: "", tag: "", sort: "updated", page: 0, sessionId }), [sessionId]);
  if (!data) return <LoadState error={error} reload={reload} />;
  if (!data.entries.length) return <p className="memory-empty">{t("memory.collectionEmptyEntries")} <MemoryLink route={`compile/${sessionId}`}>{t("memory.add")}</MemoryLink></p>;
  return <div className="memory-collection-entries">
    <div className="memory-entry-list">
      {data.entries.map((entry) => <div key={entry.id} className="memory-entry-card">
        <MemoryLink route={`entry/${entry.id}`} values={{ memoryProject: null, memorySession: sessionId }} className="memory-entry-card-link">
          <div className="memory-entry-card-heading"><strong>{entry.title}</strong><span aria-hidden="true">↗</span></div>
          {entry.summary && <p>{entry.summary}</p>}
          <div className="memory-entry-card-footer">
            <div className="memory-tags">{entry.tags.map((value) => <span key={value}>{value}</span>)}</div>
            <small>{memoryTime(entry.updatedAt)}</small>
          </div>
        </MemoryLink>
      </div>)}
    </div>
  </div>;
}
