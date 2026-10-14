import { useEffect, useMemo, useState } from "react";
import { getLocale, useT } from "../i18n";
import type { TranslationTiming } from "../ipc/translation";

/** Interpolate the host's active timer with a monotonic clock; client wall-clock skew is irrelevant. */
export function TranslationTime({ timing, serverTime, running = false, compact = false }: {
  timing?: TranslationTiming; serverTime?: number; running?: boolean; compact?: boolean;
}) {
  const t = useT();
  const [now, setNow] = useState(() => performance.now());
  const startedAt = timing?.startedAt;
  const durationMs = timing?.durationMs;
  const active = running && startedAt != null && serverTime != null;
  const snapshot = useMemo(() => ({ at: performance.now(), elapsed: (durationMs ?? 0) +
    (active ? Math.max(0, serverTime! - startedAt!) : 0) }), [durationMs, startedAt, serverTime, active]);
  useEffect(() => {
    if (!active) return;
    setNow(performance.now());
    const timer = window.setInterval(() => setNow(performance.now()), 200);
    return () => window.clearInterval(timer);
  }, [active, snapshot]);
  if (durationMs == null || !Number.isFinite(durationMs)) return null;
  const elapsed = Math.max(0, snapshot.elapsed + (active ? Math.max(0, now - snapshot.at) : 0));
  const value = new Intl.NumberFormat(getLocale(), { style: "unit", unit: "second", unitDisplay: "short",
    minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(elapsed / 1000);
  return <span className={compact ? "translation-time" : "translation-time translation-time-summary"} aria-live="off">
    {compact ? `· ${value}` : t("translation.duration", value)}
  </span>;
}
