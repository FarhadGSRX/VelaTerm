//! Submit a composed message to a PTY as one unit: the text first, then Enter as a separate write.
//!
//! Agents such as Claude Code can leave the text as an editable draft when the paste end and Enter
//! arrive together, so Enter waits until the TUI has taken the paste. Bracketed paste keeps a
//! multiline message as text instead of submitting each line.

import { ptyWrite } from "../ipc/commands";

/** Delay between the text and Enter. */
export const SUBMIT_ENTER_DELAY_MS = 500;

/** Bytes for the text part. Control characters are removed so the message cannot inject keys or end
 * the paste early; tabs and newlines stay. Without bracketed paste, newlines become Returns, as typed. */
export function terminalSubmitText(text: string, bracketed: boolean): string {
  const clean = text.replace(/\r\n?/g, "\n").replace(/[\x00-\x08\x0b-\x1f\x7f]/g, "");
  return bracketed ? `\x1b[200~${clean}\x1b[201~` : clean.replace(/\n/g, "\r");
}

export async function submitToTerminal(sessionId: string, text: string, bracketed = true) {
  await ptyWrite(sessionId, terminalSubmitText(text, bracketed));
  await new Promise((resolve) => setTimeout(resolve, SUBMIT_ENTER_DELAY_MS));
  await ptyWrite(sessionId, "\r");
}
