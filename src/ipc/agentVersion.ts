import { invoke } from "./transport";

export interface AgentVersion {
  version: string | null;
  running: boolean;
  latestVersion: string | null;
  updateAvailable: boolean;
  checkStatus: "unchecked" | "checked" | "failed";
  docsUrl: string | null;
}

export function agentVersion(sessionId: string, checkUpdates = false): Promise<AgentVersion> {
  return invoke<AgentVersion>("agent_version", { sessionId, checkUpdates });
}
