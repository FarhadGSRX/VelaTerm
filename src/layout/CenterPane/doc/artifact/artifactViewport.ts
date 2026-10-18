//! Device viewports for artifact previews and the scale that fits one into the available area.

import type { ArtifactViewport } from "../../../../store/docTab";

export interface ViewportSize {
  width: number;
  height: number;
}

/** Fixed device sizes; fit has none and fills the area. */
export const VIEWPORTS: Record<Exclude<ArtifactViewport, "fit">, ViewportSize> = {
  desktop: { width: 1280, height: 800 },
  tablet: { width: 834, height: 1112 },
  phone: { width: 390, height: 844 },
};

export const VIEWPORT_ORDER: ArtifactViewport[] = ["fit", "desktop", "tablet", "phone"];

/** Space kept around a device frame on the stage. */
export const STAGE_PADDING = 16;

export interface FrameLayout {
  /** The iframe's own size, in CSS pixels before scaling. */
  width: number;
  height: number;
  /** Scale applied so the frame fits; 1 for fit and when the device already fits. */
  scale: number;
}

/**
 * Lay out a viewport inside an area. A device renders at its real size and scales down to fit with padding;
 * it never scales up, since a phone layout blown up to fill a desktop pane misrepresents the page.
 */
export function frameLayout(viewport: ArtifactViewport, area: ViewportSize): FrameLayout {
  if (viewport === "fit") return { width: area.width, height: area.height, scale: 1 };
  const size = VIEWPORTS[viewport];
  const availW = Math.max(1, area.width - STAGE_PADDING * 2);
  const availH = Math.max(1, area.height - STAGE_PADDING * 2);
  const scale = Math.min(1, availW / size.width, availH / size.height);
  return { width: size.width, height: size.height, scale };
}

/** Header readout such as "834 × 1112 · 52%". */
export function viewportReadout(viewport: ArtifactViewport, layout: FrameLayout): string | null {
  if (viewport === "fit") return null;
  const size = VIEWPORTS[viewport];
  return `${size.width} × ${size.height} · ${Math.round(layout.scale * 100)}%`;
}
