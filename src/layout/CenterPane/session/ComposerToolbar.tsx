import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import Icons from "../../../components/Icons";
import { useT } from "../../../i18n";

/** Common choices stay beside the message; occasional settings expand into their own row. */
export function ComposerToolbar({ primary, secondary, actions, status, mobile }: {
  primary: ReactNode;
  secondary: ReactNode;
  actions: ReactNode;
  status: ReactNode;
  mobile: boolean;
}) {
  const t = useT();
  const id = useId();
  const [expanded, setExpanded] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const toggle = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!expanded || mobile) return;
    const outside = (event: PointerEvent) => {
      if (!box.current?.contains(event.target as Node)) setExpanded(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || !box.current?.closest(".sv-composer")?.contains(event.target as Node)) return;
      // An open selector handles its own Escape first. A later Escape folds the settings row.
      if (box.current.querySelector('[role="option"], .sv-popover')) return;
      event.preventDefault();
      event.stopPropagation();
      setExpanded(false);
      toggle.current?.focus();
    };
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("keydown", escape, true);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("keydown", escape, true);
    };
  }, [expanded, mobile]);

  return <div className="sv-controls sv-toolbar" ref={box}>
    <div className="sv-controls-main">
      <div className="sv-controls-primary">
        {primary}
        {!mobile && <button type="button" className="sv-chip sv-more-toggle" ref={toggle}
          title={t("chat.moreOptions")} aria-label={t("chat.moreOptions")}
          aria-expanded={expanded} aria-controls={id} onClick={() => setExpanded(value => !value)}>
          <Icons.sliders size={14} />
          <span>{t("chat.moreOptions")}</span>
          <Icons.chevD size={12} />
        </button>}
      </div>
      <div className="sv-controls-actions">{actions}</div>
    </div>
    <div className="sv-controls-secondary" id={id} hidden={!mobile && !expanded}>
      {secondary}
    </div>
    <div className="sv-controls-status">{status}</div>
  </div>;
}
