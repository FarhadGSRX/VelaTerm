import { t } from "../i18n";

/** These legacy names may materialize an entire conversation on an older backend. */
const HISTORY_READS = new Set([
  "chat_snapshot", "chat_row", "chat_subagent_rows", "chat_rewind_candidates", "read_agent_chat", "read_agent_transcript",
  "export_session_context", "search_session_content",
  "search_content_start", "search_content_page", "search_content_cancel",
]);

export function requiresHistoryProtocol(command: string): boolean {
  return HISTORY_READS.has(command);
}

export function historyProtocolError(): Error {
  return new Error(`history_protocol_unsupported: ${t("chat.history.updateServer")}`);
}

export function acceptsHistoryProtocol(version: unknown): boolean {
  return version === 2;
}
