import { useEffect, useReducer, useState } from "react";
import { chatModels, type ChatModel } from "../../../ipc/chat";
import { onTransportReconnect } from "../../../ipc/transport";

// Bound the entire discovery request, including transport and backend work before the agent timeout.
const MODEL_LOAD_TIMEOUT_MS = 30_000;

/** Keep discovery failures separate from the model reported by the conversation itself. */
export function useChatModels(sessionId: string, version: number, enabled = true) {
  const [attempt, retry] = useReducer((n: number) => n + 1, 0);
  const [state, setState] = useState<{
    sessionId: string; models: ChatModel[]; loading: boolean; failed: boolean;
  }>({ sessionId, models: [], loading: true, failed: false });

  useEffect(() => onTransportReconnect(retry), []);
  useEffect(() => {
    if (!enabled) {setState({sessionId,models:[],loading:false,failed:false});return;}
    let disposed = false;
    setState(previous => ({ sessionId, models: previous.sessionId === sessionId ? previous.models : [], loading: true, failed: previous.sessionId === sessionId && previous.failed }));
    const timeout = window.setTimeout(() => {
      disposed = true;
      setState(previous => ({ ...previous, loading: false, failed: true }));
    }, MODEL_LOAD_TIMEOUT_MS);
    void chatModels(sessionId).then(models => {
      if (disposed) return;
      window.clearTimeout(timeout);
      setState({ sessionId, models, loading: false, failed: false });
    }).catch(() => {
      if (disposed) return;
      window.clearTimeout(timeout);
      setState(previous => ({ ...previous, loading: false, failed: true }));
    });
    return () => { disposed = true; window.clearTimeout(timeout); };
  }, [sessionId, version, attempt, enabled]);

  // A pane can be reused for another session before its effect runs; never offer the old catalogue.
  const current = state.sessionId === sessionId ? state : { models: [], loading: true, failed: false };
  return { ...current, retry };
}
