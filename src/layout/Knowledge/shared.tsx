import { useCallback, useEffect, useRef, useState } from "react";
import { t, type I18nKey } from "../../i18n";
import { listen } from "../../ipc/transport";
export function knowledgeError(error: unknown) {
  const code = String(error).match(/knowledge_[a-z_]+/)?.[0] ?? "";
  const keys: Record<string, I18nKey> = {
    knowledge_query_failed: "knowledge.queryFailed", knowledge_result_large: "knowledge.resultLarge",
    knowledge_busy: "knowledge.busy", knowledge_runtime_missing: "knowledge.setup", knowledge_disabled: "knowledge.disabledHelp",
    knowledge_conflict: "knowledge.conflict", knowledge_symbol_missing: "knowledge.symbolMissing",
    knowledge_missing_directory: "knowledge.directoryMissing", knowledge_directory_changed: "knowledge.directoryMissing",
    knowledge_partial: "knowledge.partial", knowledge_interrupted: "knowledge.interrupted",
    knowledge_checksum: "knowledge.checksum", knowledge_download_failed: "knowledge.downloadFailed",
    knowledge_source_missing: "knowledge.symbolMissing", knowledge_timeout: "knowledge.timeout",
  };
  return t(keys[code] ?? "knowledge.error");
}
export function useKnowledgeLoad<T>(loader: () => Promise<T>, deps: unknown[], retainOnRefresh = false) {
  const [loading, setLoading] = useState(true);
  const previousDeps = useRef<unknown[] | null>(null);
  const [data,setData] = useState<T | null>(null); const [error,setError] = useState("");
  const [revision,setRevision] = useState(0);
  useEffect(() => {
    let active = true; let stop: (() => void) | undefined;
    void listen("knowledge://changed", () => { if (active) setRevision((v) => v+1); }).then((off) => { if (active) stop = off; else off(); }).catch(() => {});
    return () => { active = false; stop?.(); };
  }, []);
  useEffect(() => {
    let alive = true; setLoading(true);
    const sameResource = previousDeps.current !== null && previousDeps.current.length === deps.length
      && deps.every((value, i) => Object.is(value, previousDeps.current![i]));
    previousDeps.current = [...deps];
    if (!retainOnRefresh || !sameResource) setData(null);
    setError("");
    loader().then((v) => { if (alive) setData(v); }).catch((e) => { if (alive) { setData(null); setError(knowledgeError(e)); } }).finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [...deps,revision]);
  const reload = useCallback(() => setRevision(v => v + 1), []);
  return { data,error,loading,reload };
}
export function KnowledgeState({ value }: { value: string }) {
  const keys: Record<string,I18nKey> = { ready:"knowledge.ready", disabled:"knowledge.disabled", indexing:"knowledge.indexing", syncing:"knowledge.syncing", failed:"knowledge.failed", current:"knowledge.current", review:"knowledge.review", unavailable:"knowledge.unavailable" };
  return <span className={`knowledge-state ${value}`}>{t(keys[value] ?? "knowledge.unavailable")}</span>;
}
