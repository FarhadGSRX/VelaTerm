import { useEffect, useState } from "react";
import type { PlanExecuteContext } from "../ipc/launch";

export interface PlanExecuteMenuRoute { requestId: string; context: PlanExecuteContext }

export function planExecuteUrl(context: PlanExecuteContext | null): string {
  const url = new URL(window.location.href);
  for (const key of ["planExecute", "planProject", "planGroup", "planParent"]) url.searchParams.delete(key);
  if (context) {
    url.searchParams.set("planExecute", crypto.randomUUID());
    url.searchParams.set("planProject", context.projectId);
    if (context.groupId) url.searchParams.set("planGroup", context.groupId);
    if (context.parentSessionId) url.searchParams.set("planParent", context.parentSessionId);
  }
  return url.href;
}

export function navigatePlanExecute(url: string) {
  window.history.pushState(null, "", url);
  window.dispatchEvent(new PopStateEvent("popstate"));
}

function readRoute(): PlanExecuteMenuRoute | null {
  const params = new URLSearchParams(window.location.search);
  const requestId = params.get("planExecute");
  if (!requestId) return null;
  return { requestId, context: { projectId: params.get("planProject") ?? "", groupId: params.get("planGroup"), parentSessionId: params.get("planParent") } };
}

export function usePlanExecuteMenuRoute() {
  const [route, setRoute] = useState(readRoute);
  useEffect(() => {
    const update = () => setRoute(previous => {
      const next = readRoute();
      return JSON.stringify(previous) === JSON.stringify(next) ? previous : next;
    });
    window.addEventListener("popstate", update);
    return () => window.removeEventListener("popstate", update);
  }, []);
  return route;
}
