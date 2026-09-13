import { useEffect, useRef, useState } from "react";

import Icons from "../../../components/Icons";
import { useT } from "../../../i18n";
import { platform } from "../../../platform";
import { copyAttributes } from "./selectionCopy";

/** Copy the source text, preserving whitespace and excluding the block's presentation controls. */
export function CodeBlockHeader({ code, lang }: { code: string; lang?: string }) {
  const t = useT();
  const [status, setStatus] = useState<"idle" | "copying" | "copied" | "failed">("idle");
  const request = useRef(0);

  useEffect(() => {
    setStatus("idle");
    return () => { request.current += 1; };
  }, [code]);

  useEffect(() => {
    if (status !== "copied") return;
    const timer = window.setTimeout(() => setStatus("idle"), 2000);
    return () => window.clearTimeout(timer);
  }, [status]);

  const copy = async () => {
    if (status === "copying") return;
    const id = ++request.current;
    setStatus("copying");
    try {
      await platform.clipboard.writeText(code, { reportFailure: true });
      if (request.current === id) setStatus("copied");
    } catch {
      if (request.current === id) setStatus("failed");
    }
  };

  return (
    <div className="sv-code-header" {...copyAttributes.ignore}>
      {lang ? <span className="sv-code-lang">{lang}</span> : null}
      <button type="button" className="sv-code-copy" onClick={() => void copy()}
        aria-disabled={status === "copying"} aria-busy={status === "copying"} title={t("common.copy")}>
        {status === "copied" ? <Icons.check /> : <Icons.copy />}
        <span aria-live="polite">{t(status === "copied" ? "common.copied" : "common.copy")}</span>
      </button>
      {status === "failed" ? <span className="sv-code-copy-error" role="alert">{t("common.copyFailed")}</span> : null}
    </div>
  );
}
