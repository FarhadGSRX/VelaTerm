//! Shared right-panel components: the KV key-value row used by the Info tab, Git tab, and project-group
//! ScopeInfo, and the collapsible section wrapper that lets users fold away the Info panel regions they
//! do not need to watch.

import Icons from "../../components/Icons";
import { useT } from "../../i18n";
import { useTermStore } from "../../store/termStore";

export function KV({ k, v, accent }: { k: string; v: React.ReactNode; accent?: boolean }) {
  return (
    <div className="kv">
      <span className="k">{k}</span>
      <span className={"v" + (accent ? " accent" : "")}>{v}</span>
    </div>
  );
}

/**
 * One Info panel section: a caption row that doubles as a disclosure control, an optional right-aligned
 * qualifier (the agent name, the pid), optional header controls, and the section body.
 *
 * The caption and the plus/minus button both toggle, because the caption is the obvious target while the
 * button is what makes the affordance visible at all. Collapsed sections keep their header, including its
 * controls, so a live toggle such as Resources' "system" checkbox stays reachable while the rows are hidden.
 *
 * Collapse state is stored by id rather than per session: the panel shows the same regions for every
 * session, so a user who folds away Usage means it everywhere. State lives in the settings store so it
 * survives restarts and follows the user to other shells.
 */
export function Section({
  id,
  title,
  tag,
  actions,
  className,
  ariaLabel,
  busy,
  children,
}: {
  /** Stable section id, also the key its collapsed state is persisted under. */
  id: string;
  /** Caption text. It is uppercase in CSS, so write it in sentence case. */
  title: string;
  /** Right-aligned qualifier rendered in the header's tag styling. */
  tag?: React.ReactNode;
  /** Header controls such as the usage refresh button, shown before the collapse button. */
  actions?: React.ReactNode;
  className?: string;
  /** Accessible name for the section, when callers or tests rely on one. */
  ariaLabel?: string;
  /** Marks a section whose body is still loading, so assistive tech reports the section as busy. */
  busy?: boolean;
  children: React.ReactNode;
}) {
  const t = useT();
  const collapsed = useTermStore((s) => !!s.infoCollapsed[id]);
  const toggleInfoSection = useTermStore((s) => s.toggleInfoSection);
  const tip = collapsed ? t("panel.expandSection") : t("panel.collapseSection");
  return (
    <section
      className={"insp-section" + (collapsed ? " collapsed" : "") + (className ? ` ${className}` : "")}
      aria-label={ariaLabel}
      aria-busy={busy}
    >
      <h4 className="insp-head">
        <button
          type="button"
          className="insp-head-toggle"
          onClick={() => toggleInfoSection(id)}
          aria-expanded={!collapsed}
          title={tip}
        >
          <span className="ttl">{title}</span>
        </button>
        {tag != null && <span className="insp-head-tag">{tag}</span>}
        {actions}
        <button
          type="button"
          className="insp-collapse"
          onClick={() => toggleInfoSection(id)}
          aria-expanded={!collapsed}
          aria-label={tip}
          title={tip}
        >
          {collapsed ? <Icons.plus size={12} /> : <Icons.minus size={12} />}
        </button>
      </h4>
      {!collapsed && children}
    </section>
  );
}
