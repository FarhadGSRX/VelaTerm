//! Load the optional WebGL renderer only when selected. DOM is the default, and WebGL may lose
//! contexts in hidden views. Cache its constructor for synchronous context recovery on tab reveal.

import type { WebglAddon } from "@xterm/addon-webgl";

type WebglCtor = typeof WebglAddon;

let webglCtor: WebglCtor | null = null;

/**
 * Load the WebGL addon constructor, returning null when the chunk cannot be fetched. Null is not
 * exceptional: every caller already treats an unavailable WebGL renderer as "stay on DOM".
 */
export async function loadWebglCtor(): Promise<WebglCtor | null> {
  if (webglCtor) return webglCtor;
  try {
    webglCtor = (await import("@xterm/addon-webgl")).WebglAddon;
    return webglCtor;
  } catch {
    return null;
  }
}

/**
 * The WebGL constructor if a previous load already resolved it, otherwise null. For synchronous call
 * sites that only run after WebGL was successfully attached once, where awaiting would restructure
 * surrounding logic for no benefit.
 */
export function peekWebglCtor(): WebglCtor | null {
  return webglCtor;
}
