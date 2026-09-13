import { useEffect, useRef, useState } from "react";
import Icons from "../../../components/Icons";
import { useT } from "../../../i18n";

/** Keep controls open while picking options, including options that do not take focus. */
export function useComposerOptions(enabled: boolean) {
  const container = useRef<HTMLElement | null>(null);
  const [expanded, setExpanded] = useState(false);
  useEffect(() => {
    if (!enabled || !expanded) return;
    const outside = (event: PointerEvent) => {
      if (!container.current?.contains(event.target as Node)) setExpanded(false);
    };
    document.addEventListener("pointerdown", outside, true);
    return () => document.removeEventListener("pointerdown", outside, true);
  }, [enabled, expanded]);
  return {
    expanded,
    ref: (node: HTMLElement | null) => { container.current = node; },
    toggle: () => {
      const active = document.activeElement;
      if (active instanceof HTMLElement && container.current?.contains(active)) active.blur();
      setExpanded(value => !value);
    },
  };
}

export function ComposerOptionsButton({ expanded, onToggle }: { expanded: boolean; onToggle: () => void }) {
  const t = useT();
  return <button type="button" className="sv-options-toggle" aria-label={t("mobile.inputOptions")}
    title={t("mobile.inputOptions")} aria-expanded={expanded}
    onPointerDown={event => event.preventDefault()} onClick={onToggle}>
    <Icons.sliders size={20} />
  </button>;
}
