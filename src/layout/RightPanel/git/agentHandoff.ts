//! Handing a Git task to the agent of the session the panel is showing, instead of starting a
//! separate one: that agent already knows what it was working on, which is most of what resolving a
//! conflict or writing a commit message needs.
//!
//! A conversation-view session receives the request as an ordinary message through the same outbox
//! the composer uses, so it queues behind a running turn and shows up in the conversation. A
//! terminal session gets the text pasted into its input as a bracketed paste, without Enter: a TUI
//! has no message queue, so the user decides when to send it.

import { chatStart } from "../../../ipc/chat";
import { ptyWrite } from "../../../ipc/commands";
import { kindLabel } from "../../sessionMenuShared";
import { createSubmission, deliverSubmission } from "../../CenterPane/session/outbox";
import { useTermStore } from "../../../store/termStore";
import type { Session } from "../../../types";

export interface AgentHandoff {
  /** Display name of the agent, for button labels. */
  label: string;
  /** True when the text is sent; false when it is only pasted into a terminal for the user to send. */
  sends: boolean;
  send: (text: string) => Promise<void>;
}

/** The handoff for a session, or null when it has no agent that can take a request right now. */
export function useAgentHandoff(session: Session | undefined): AgentHandoff | null {
  const running = useTermStore((s) => (session ? s.runtimes[session.id]?.status === "running" : false));
  if (!session || session.kind === "terminal" || session.kind === "browser") return null;
  const id = session.id;
  const label = kindLabel(session.kind);
  const reveal = () => useTermStore.getState().openSession(id);
  if (session.engine === "chat") {
    return {
      label,
      sends: true,
      send: async (text) => {
        reveal();
        const item = createSubmission(id, text, [], "queue");
        // Starting is a no-op for a running agent; a stopped one resumes with its saved settings.
        await deliverSubmission(id, item, () => chatStart(id));
      },
    };
  }
  if (!running) return null;
  return {
    label,
    sends: false,
    send: async (text) => {
      reveal();
      await ptyWrite(id, `\x1b[200~${text}\x1b[201~`);
    },
  };
}
