import { useT } from "../../../i18n";
import type { MessageOrigin } from "../../../ipc/chat";
import Icons from "../../../components/Icons";
import { kindIconEl } from "../../sessionViewers/sessionMeta";

/** Keep sender names and workflow roles consistent before and after delivery. */
export function messageSenderName(origin: MessageOrigin, t: ReturnType<typeof useT>): string {
  const role = origin.role === "plan" ? t("chat.origin.plan")
    : origin.role === "exec" ? t("chat.origin.exec")
    : origin.role === "review" ? t("chat.origin.review") : null;
  return role ? `${origin.name} · ${role}` : origin.name;
}

/** A sender's agent icon; a helper has no agent, so it gets the generic bot icon. */
export function originIconEl(origin: MessageOrigin, size = 14) {
  return origin.agent ? kindIconEl(origin.agent, size) : <Icons.bot size={size} />;
}

export function MessageSender({ origin }: { origin: MessageOrigin }) {
  const t = useT();
  const name = messageSenderName(origin, t);
  return (
    <span className="sv-message-sender" title={name}>
      <span className="sv-message-sender-icon" aria-hidden>{originIconEl(origin, 14)}</span>
      <span className="sv-message-sender-name">{name}</span>
    </span>
  );
}
