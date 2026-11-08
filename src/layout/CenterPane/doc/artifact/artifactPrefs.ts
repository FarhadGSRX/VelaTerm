//! Per-file preferences for artifact previews, kept in this shell's localStorage.
//!
//! Network access is remembered per absolute path: allowing one page online says nothing about the next.

const NET_KEY = "vlx-artifact-net-allow";

/** Most paths remembered; the oldest allowance is forgotten first. */
const MAX_PATHS = 200;

function readNet(): string[] {
  try {
    const raw = JSON.parse(localStorage.getItem(NET_KEY) ?? "[]");
    return Array.isArray(raw) ? raw.filter((p): p is string => typeof p === "string") : [];
  } catch {
    return [];
  }
}

export function isNetAllowed(path: string): boolean {
  return readNet().includes(path);
}

export function setNetAllowed(path: string, allowed: boolean): void {
  const rest = readNet().filter((p) => p !== path);
  const next = allowed ? [...rest, path].slice(-MAX_PATHS) : rest;
  try {
    localStorage.setItem(NET_KEY, JSON.stringify(next));
  } catch {
    /* Storage full or unavailable: the choice lasts for this session only. */
  }
}
