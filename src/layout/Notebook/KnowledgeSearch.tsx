//! Knowledge-base lookup shared by the center workspace and the right inspector: one debounced backend
//! query, rendered as a hit list with match snippets. Each host keeps its own search field and layout;
//! this module owns the query lifecycle, the keyboard contract, and the result rows.

import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import Icons from "../../components/Icons";
import { useT } from "../../i18n";
import { notebookSearch, type NotebookSearchHit, type NotebookSearchRelated, type NotebookSearchResult } from "../../ipc/notebook";
import { memoryTime } from "../Memory/shared";
import { highlightMatches } from "../sessionViewers/highlight";
import { notebookError } from "./shared";
import "./notebook.css";

/** Hits requested per query. The figure shown to the user stays `total`, which counts every match. */
const SEARCH_LIMIT = 100;
/** Typing pause before a query runs, matching the notebook list's existing debounce. */
const SEARCH_DELAY = 250;

/** Literals to highlight: what the backend actually matched, falling back to the typed words. */
export function searchTerms(query: string, matched?: string[]) {
  return matched?.length ? matched : query.trim().split(/\s+/).filter(Boolean);
}

/** Debounced search state. A newer query supersedes an in-flight one, so a slow answer never wins. */
export function useKnowledgeSearch(query: string, vaultId = "") {
  const [state, setState] = useState<{ result: NotebookSearchResult | null; error: string }>({
    result: null,
    error: "",
  });
  const generation = useRef(0);
  useEffect(() => {
    const text = query.trim();
    const id = ++generation.current;
    if (!text) {
      setState({ result: null, error: "" });
      return;
    }
    // Keep the previous hits on screen while the next query runs, so the list does not flash empty.
    setState((old) => ({ result: old.result, error: "" }));
    const timer = setTimeout(() => {
      notebookSearch(text, vaultId, SEARCH_LIMIT)
        .then((result) => {
          if (generation.current === id) setState({ result, error: "" });
        })
        .catch((error) => {
          if (generation.current === id) setState({ result: null, error: notebookError(error) });
        });
    }, SEARCH_DELAY);
    return () => clearTimeout(timer);
  }, [query, vaultId]);
  return state;
}

/** Keyboard contract shared by both hosts: arrows move the highlight, Enter opens it, Escape clears. */
export function useSearchKeys(count: number, onOpen: (index: number) => void, onClear: () => void) {
  const [active, setActive] = useState(0);
  useEffect(() => {
    setActive((index) => Math.min(index, Math.max(0, count - 1)));
  }, [count]);
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      if (!count) return;
      event.preventDefault();
      setActive((index) => Math.max(0, Math.min(count - 1, index + (event.key === "ArrowDown" ? 1 : -1))));
    } else if (event.key === "Enter") {
      if (!count) return;
      event.preventDefault();
      onOpen(active);
    } else if (event.key === "Escape") {
      event.preventDefault();
      onClear();
    }
  };
  return { active, setActive, onKeyDown };
}

/**
 * The body of a note hit, shared by the workspace and the home page. `compact` drops the per-hit
 * line and time the inspector's narrow column cannot show; the snippet stays, because it is what
 * tells two notes with similar names apart.
 */
export function KnowledgeNoteRow({
  hit,
  query,
  compact = false,
  onOpenRelated,
}: {
  hit: NotebookSearchHit;
  query: string;
  compact?: boolean;
  onOpenRelated: (related: NotebookSearchRelated) => void;
}) {
  const t = useT();
  const terms = searchTerms(query, hit.matched);
  return <>
    <Icons.docLines size={16} aria-hidden="true" />
    <div>
      <strong>{highlightMatches(hit.name.replace(/\.md$/i, ""), terms)}</strong>
      <small>{hit.vaultName} · {hit.path}</small>
      {hit.summary && <p>{highlightMatches(hit.summary, terms)}</p>}
      {!compact && <span className="nb-search-meta">
        {hit.line > 0 && `${t("nb.searchLine", String(hit.line))} · `}
        {hit.matches > 0 && `${t("nb.searchMatches", String(hit.matches))} · `}
        {memoryTime(hit.updatedAt)}
      </span>}
      {hit.related.length > 0 && <span className="nb-search-related">
        {t("nb.searchRelated")}
        {hit.related.map((related) => <button
          key={`${related.path}`}
          type="button"
          title={related.path}
          onClick={(event) => { event.stopPropagation(); onOpenRelated(related); }}
        >{related.name.replace(/\.md$/i, "")}</button>)}
      </span>}
    </div>
  </>;
}

/** The note hit list. A fuzzy page is prefixed with a hint so loosened matches are never mistaken for exact ones. */
export function KnowledgeSearchResults({
  query,
  result,
  error,
  active,
  onActive,
  onOpen,
  onOpenRelated,
  compact = false,
}: {
  query: string;
  result: NotebookSearchResult | null;
  error: string;
  active: number;
  onActive: (index: number) => void;
  onOpen: (index: number) => void;
  onOpenRelated: (related: NotebookSearchRelated) => void;
  compact?: boolean;
}) {
  const t = useT();
  if (error) return <p className="nb-message" role="alert">{error}</p>;
  if (!result) return <p className="nb-search-hint" role="status">{t("common.loading")}</p>;
  if (!result.entries.length) return <p className="nb-search-hint" role="status">{t("nb.searchEmpty")}</p>;
  return <div className="nb-search-results" role="listbox" aria-label={t("nb.searchResults")}>
    {result.fuzzy && <p className="nb-search-hint" role="status">{t("nb.searchFuzzy")}</p>}
    {result.entries.map((hit: NotebookSearchHit, index) => (
      <div
        key={`${hit.vaultId}:${hit.path}`}
        role="option"
        aria-selected={index === active}
        className={"nb-search-row" + (index === active ? " active" : "")}
        onMouseEnter={() => onActive(index)}
        onClick={() => onOpen(index)}
      >
        <KnowledgeNoteRow hit={hit} query={query} compact={compact} onOpenRelated={onOpenRelated} />
      </div>
    ))}
    {result.hasMore && <p className="nb-search-hint">{t("nb.searchMore")}</p>}
  </div>;
}
