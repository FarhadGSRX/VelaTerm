//! A translated display never becomes the identity or value of a question form control.
import { useEffect, useState } from "react";
import Select from "../components/Select";
import { StatusDot } from "../components/StatusDot";
import { useT } from "../i18n";
import { translateQuestion, type QuestionTranslation, type TranslationDisplay } from "../ipc/translation";
import type { ChatPermission } from "../ipc/chat";
import { parseQuestionFormQuestions } from "../layout/CenterPane/session/questionForm";
import { useConversationTranslation } from "./state";
import { translationError } from "./SettingsSection";
import { TranslationTime } from "./Time";

export function useQuestionTranslation(request: ChatPermission) {
  const state = useConversationTranslation();
  const t = useT();
  const [result, setResult] = useState<{ key: string; value?: QuestionTranslation; error?: string } | null>(null);
  const [retry, setRetry] = useState(0);
  const [override, setOverride] = useState<TranslationDisplay | null>(null);
  const prefs = state?.value?.preferences;
  const enabled = !!(state?.settings?.config.enabled && state.value?.enabled && prefs?.replies);
  const sessionId = state?.sessionId ?? "";
  const key = JSON.stringify([sessionId, request.id, request.input, enabled, state?.value?.version,
    state?.value?.generation, prefs?.replySource, prefs?.replyTarget]);
  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    setResult(null);
    void translateQuestion(sessionId, request.id).then(value => {
      if (!disposed) setResult({ key, value });
    }, error => { if (!disposed) setResult({ key, error: String(error) }); });
    return () => { disposed = true; };
  }, [enabled, sessionId, request.id, key, retry]);
  const current = enabled && result?.key === key ? result : null;
  const value = current?.value;
  const valid = value && value.version === state?.value?.version && value.generation === state?.value?.generation
    && value.source === prefs?.replySource && value.target === prefs?.replyTarget;
  const questions = valid ? parseQuestionFormQuestions(value.input) : null;
  const error = current?.error || (value && !questions ? "translation:structure" : "");
  const mode = override ?? prefs?.display ?? "original";
  return {
    questions, mode, target: prefs?.replyTarget,
    outgoing: !!(state?.settings?.config.enabled && state.value?.enabled && prefs?.outgoing),
    control: enabled ? <div className="translation-question-controls">
      {!current && <span className="translation-message-status" role="status"><StatusDot status="working" size={6} />{t("translation.translating")}</span>}
      {questions && <><Select value={mode} width="auto" size="sm" menuPortal
        options={(state?.settings?.displays ?? []).map(display => ({ value: display, label: t(`translation.${display}`) }))}
        onChange={setOverride} ariaLabel={t("translation.replyDisplay")} /><TranslationTime timing={value} /></>}
      {error && <><span className="translation-hint" role="alert">{translationError(error, t)}</span>
        <button className="vlx-btn" onClick={() => setRetry(value => value + 1)}>{t("translation.retry")}</button></>}
    </div> : null,
  };
}

export function QuestionText({ original, translated, mode, target }: {
  original: string; translated?: string; mode: TranslationDisplay; target?: string;
}) {
  if (!translated || translated === original || mode === "original") return <>{original}</>;
  if (mode === "translation") return <span lang={target}>{translated}</span>;
  return <span className="translation-question-pair"><span>{original}</span><span lang={target}>{translated}</span></span>;
}
