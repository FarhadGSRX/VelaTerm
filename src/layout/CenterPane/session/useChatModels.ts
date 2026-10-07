import { useEffect, useReducer, useState } from "react";
import { chatModels, type ChatModel } from "../../../ipc/chat";
import { onTransportReconnect } from "../../../ipc/transport";

/** Keep discovery failures separate from the model reported by the conversation itself. */
export function useChatModels(sessionId: string, version: number) {
  const [attempt, retry] = useReducer((n: number) => n + 1, 0);
  const [state, setState] = useState<{
    sessionId: string; models: ChatModel[]; loading: boolean; failed: boolean;
  }>({ sessionId, models: [], loading: true, failed: false });

  useEffect(() => onTransportReconnect(retry), []);
  useEffect(() => {
    let disposed = false;
    setState(previous => ({ sessionId, models: previous.sessionId === sessionId ? previous.models : [], loading: true, failed: previous.sessionId === sessionId && previous.failed }));
    void chatModels(sessionId).then(models => {
      if (!disposed) setState({ sessionId, models, loading: false, failed: false });
    }).catch(() => {
      if (!disposed) setState(previous => ({ ...previous, loading: false, failed: true }));
    });
    return () => { disposed = true; };
  }, [sessionId, version, attempt]);

  // A pane can be reused for another session before its effect runs; never offer the old catalogue.
  const current = state.sessionId === sessionId ? state : { models: [], loading: true, failed: false };
  return { ...current, retry };
}
