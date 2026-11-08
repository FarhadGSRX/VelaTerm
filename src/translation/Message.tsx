//! Read-only translations never replace the original conversation row or its default copy text.
import { useMemo, useState, type ReactNode } from "react";
import Icons from "../components/Icons";
import Select from "../components/Select";
import { StatusDot } from "../components/StatusDot";
import { useT } from "../i18n";
import { CopyBtn } from "../layout/sessionMenuShared";
import { translateReply, type TranslationDisplay, type TranslationResult } from "../ipc/translation";
import { Markdown, MarkdownBlocks } from "../layout/CenterPane/session/markdown";
import { md } from "../markdownEngine";
import { useConversationTranslation } from "./state";
import { TranslationTime } from "./Time";
import { translationError } from "./SettingsSection";
import { formattedTranslationPairs } from "./formatting";

export function useOriginalMessage(rowId: string | undefined, text: string) {
  const state = useConversationTranslation();
  return state?.originals.find(message => message.id === rowId)?.original ?? text;
}

function TranslationPairs({ pairs, target, tail = "" }: { pairs: TranslationResult["pairs"]; target: string; tail?: string }) {
  const t = useT();
  const original = pairs.map(pair => pair.original).join("\n") + tail;
  const translation = pairs.map(pair => pair.translation).join("\n") + tail;
  const blocks = useMemo(() => {
    if (original.length > 200_000 || translation.length > 200_000) return null;
    try {
      // Resolve references and all container syntax against complete documents before pairing blocks.
      const source = md.lexer(original).filter(token => token.type !== "space");
      const translated = md.lexer(translation).filter(token => token.type !== "space");
      if (source.length !== translated.length || source.some((token, i) => token.type !== translated[i].type)) return null;
      return source.map((token, i) => ({ original: token, translation: translated[i] }));
    } catch { return null; }
  }, [original, translation]);
  const pair = (key: number, source: ReactNode, translated: ReactNode, same: boolean) => <div className="translation-pair" key={key}>
    <div className="translation-pair-original" aria-label={t("translation.original")}>{source}</div>
    {!same && <div className="translation-pair-target" lang={target} aria-label={t("translation.translation")}>{translated}</div>}
  </div>;
  return <div className="translation-pairs">{blocks
    ? blocks.map((block, i) => pair(i, <MarkdownBlocks tokens={[block.original]} />, <MarkdownBlocks tokens={[block.translation]} />, block.original.raw === block.translation.raw))
    : pair(0, <Markdown text={original} />, <Markdown text={translation} />, original === translation)}</div>;
}

export function SentTranslation({ rowId, original }: { rowId: string; original: ReactNode }) {
  const state = useConversationTranslation();
  const record = state?.value?.messages.find(message => message.id === rowId);
  if (!state?.settings?.config.enabled || !state.value?.enabled || !record) return original;
  return <div className="translation-sent">
    <TranslationPairs pairs={formattedTranslationPairs(record.result.pairs, record.result.formatVersion)} target={record.target} />
    <TranslationTime timing={record} />
  </div>;
}

export function ReplyTranslation({ rowId, text, original, streaming = false }: { rowId: string; text: string; original: ReactNode; streaming?: boolean }) {
  const t = useT(); const state = useConversationTranslation(); const [override, setOverride] = useState<TranslationDisplay | null>(null);
  const [busy, setBusy] = useState(false); const [error, setError] = useState("");
  if (!state?.settings?.config.enabled || !state.value?.enabled) return original;
  const record = state.value.replies.find(record => record.id === rowId);
  const matches = !!record && record.source === state.value.preferences.replySource && record.target === state.value.preferences.replyTarget && text.startsWith(record.original);
  const current = matches && record.version === state.value.version && record.generation === state.value.generation;
  // A translation from an earlier engine configuration still matches this text and language pair, so keep it visible.
  const ready = !!record?.result && matches;
  const pairs = ready ? formattedTranslationPairs(record.result!.pairs, record.result!.formatVersion) : [];
  const translated = pairs.map(pair => pair.translation).join("\n");
  const translatedSource = pairs.map(pair => pair.original).join("\n");
  const complete = ready && translatedSource === text && record.state === "ready";
  const tail = ready && text.startsWith(translatedSource) ? text.slice(translatedSource.length) : text;
  const pending = !complete && (record?.state === "pending" || busy || (streaming && state.value.preferences.replies));
  const mode = override ?? state.value.preferences.display;
  const run = async () => { setBusy(true); setError(""); try { await translateReply(state.sessionId, rowId); await state.refresh(); } catch (e) { setError(translationError(e, t)); } finally { setBusy(false); } };
  let body: ReactNode = original;
  if (ready && mode === "translation") body = <Markdown text={translated + tail} />;
  if (ready && mode === "both") body = <div className="translation-reply-blocks"><div><p className="translation-hint">{t("translation.original")}</p>{original}</div><div><p className="translation-hint">{t("translation.translation")} · {record.target}</p><Markdown text={translated + tail} /></div></div>;
  if (ready && mode === "immersive") body = <TranslationPairs pairs={pairs} target={record.target} tail={tail} />;
  return <>
    {ready && <div className="translation-reply-actions">
      <Select value={mode} width="auto" size="sm" menuPortal options={state.settings.displays.map(display => ({ value: display, label: t(`translation.${display}`) }))} onChange={setOverride} ariaLabel={t("translation.replyDisplay")} />
      <div className="translation-copy-actions">
        {(mode === "both" || mode === "immersive") && <CopyBtn tooltip disabled={!complete} label={t("translation.copyBoth")} copiedLabel={t("translation.copiedBoth")} icon={<Icons.clipboard size={14} />}
          value={text === translated ? text : `${text}\n\n${translated}`} />}
        <CopyBtn tooltip disabled={!complete} label={t("translation.copyTranslation")} copiedLabel={t("translation.copiedTranslation")} icon={<Icons.copy size={14} />} value={translated} />
      </div>
    </div>}
    {body}
    {pending && <div className="translation-message-status" role="status"><StatusDot status="working" size={6} />{t("translation.translating")}<TranslationTime timing={record} serverTime={state.value.serverTime} running compact /></div>}
    {!pending && current && <TranslationTime timing={record} />}
    {!pending && (!ready || !current || record?.state === "failed" || (record?.result?.formatVersion ?? 0) < 2) && <div className="translation-message-status"><button className="vlx-btn" onClick={() => void run()}>{t(record?.state === "failed" || ready ? "translation.retry" : "translation.title")}</button></div>}
    {record && !current && <p className="translation-hint">{t("translation.stale")}</p>}
    {(error || record?.error) && <p role="alert" className="translation-hint">{error || (record?.error === "translation:canceled" ? t("translation.unavailable") : translationError(record?.error, t))}</p>}
  </>;
}
