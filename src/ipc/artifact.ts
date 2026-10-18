//! Artifact preview backend calls (see src-tauri/src/artifact.rs).
//!
//! A preview asks for a grant: a random ID bound to the folder holding the page. The sandboxed iframe then
//! loads `<base><grantId>/<entry>`, where the base depends on the shell. The desktop app serves its own
//! `vlxart` scheme, which Windows exposes as `http(s)://vlxart.localhost`. Electron, browsers and remote
//! clients go through the backend's web server under `/api/artifact/`.

import { isTauri } from "./transport";
import { invoke } from "./transport";
import { apiUrl } from "./shareBase";

export interface ArtifactGrant {
  grantId: string;
  /** Entry page relative to the root, already percent-encoded. */
  entry: string;
  /** Folder the grant serves. */
  root: string;
}

export interface ArtifactVersion {
  version: number;
  mtimeMs: number;
  size: number;
}

/** URL prefix grants are reached through in this shell; always ends with `/`. */
export function artifactBase(): string {
  if (isTauri) {
    return /Windows/i.test(navigator.userAgent) ? `${location.protocol}//vlxart.localhost/` : "vlxart://localhost/";
  }
  return `${location.origin}${apiUrl("/api/artifact/")}`;
}

export function artifactGrant(path: string, opts: { net: boolean; version?: number }): Promise<ArtifactGrant> {
  return invoke<ArtifactGrant>("artifact_grant", { path, base: artifactBase(), net: opts.net, version: opts.version });
}

/** Extends a grant's lease; false means it lapsed and a new grant is needed. */
export function artifactRenew(grantId: string): Promise<boolean> {
  return invoke<boolean>("artifact_renew", { grantId });
}

export function artifactRevoke(grantId: string): Promise<void> {
  return invoke<void>("artifact_revoke", { grantId });
}

export function artifactVersions(path: string): Promise<ArtifactVersion[]> {
  return invoke<ArtifactVersion[]>("artifact_versions", { path });
}

/** Saves the page as a new version when it changed since the last one; returns the version list. */
export function artifactSnapshot(path: string): Promise<ArtifactVersion[]> {
  return invoke<ArtifactVersion[]>("artifact_snapshot", { path });
}

export function artifactRestore(path: string, version: number): Promise<void> {
  return invoke<void>("artifact_restore", { path, version });
}

/** The iframe URL for a grant. `nonce` forces a reload; `scrollY` is restored by the injected script. */
export function artifactUrl(grant: ArtifactGrant, nonce: number, scrollY: number): string {
  const query = `?__vlx_r=${nonce}` + (scrollY > 0 ? `&__vlx_scroll=${Math.round(scrollY)}` : "");
  return `${artifactBase()}${grant.grantId}/${grant.entry}${query}`;
}
