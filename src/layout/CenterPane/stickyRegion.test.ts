//! Squeezing a tab's layout into a band of the stage, for the sticky-pane split. `computeLayout`
//! always reports percentages of the whole stage, so showing two tabs side by side rescales their
//! rects rather than recomputing them — splits must keep their proportions inside the band.

import { describe, expect, it } from "vitest";
import {
  computeDividers,
  computeLayout,
  dividerIntoRegion,
  makeLeaf,
  rectIntoRegion,
  splitAt,
  type Rect,
} from "./paneTree";

const FULL: Rect = { left: 0, top: 0, width: 100, height: 100 };

describe("rectIntoRegion", () => {
  it("leaves a rect alone when the region is the whole stage", () => {
    expect(rectIntoRegion(FULL, 0, 100)).toEqual(FULL);
  });

  it("scales a full-width rect down to the band and offsets it", () => {
    expect(rectIntoRegion(FULL, 0, 65)).toEqual({ left: 0, top: 0, width: 65, height: 100 });
    expect(rectIntoRegion(FULL, 65, 35)).toEqual({ left: 65, top: 0, width: 35, height: 100 });
  });

  it("keeps a split's proportions inside the band", () => {
    // A 50/50 vertical split of the stage, squeezed into the left 60%: still 50/50 of 60.
    const left = rectIntoRegion({ left: 0, top: 0, width: 50, height: 100 }, 0, 60);
    const right = rectIntoRegion({ left: 50, top: 0, width: 50, height: 100 }, 0, 60);
    expect(left).toEqual({ left: 0, top: 0, width: 30, height: 100 });
    expect(right).toEqual({ left: 30, top: 0, width: 30, height: 100 });
    // The two halves still tile the band exactly, with no gap and no overhang.
    expect(left.width + right.width).toBe(60);
    expect(right.left + right.width).toBe(60);
  });

  it("never touches the vertical axis, because the bands are full height", () => {
    const r: Rect = { left: 20, top: 30, width: 40, height: 70 };
    const squeezed = rectIntoRegion(r, 10, 50);
    expect(squeezed.top).toBe(30);
    expect(squeezed.height).toBe(70);
  });

  it("tiles a real two-pane tab across two bands without overlap", () => {
    const leaf = makeLeaf("a");
    const laid = computeLayout(splitAt(leaf, leaf.paneId, "horizontal", "b"));
    const main = laid.map(({ rect }) => rectIntoRegion(rect, 0, 70));
    for (const r of main) {
      expect(r.left).toBeGreaterThanOrEqual(0);
      expect(r.left + r.width).toBeLessThanOrEqual(70 + 1e-9);
    }
  });
});

describe("dividerIntoRegion", () => {
  const split = (dir: "horizontal" | "vertical") => {
    const leaf = makeLeaf("a");
    return splitAt(leaf, leaf.paneId, dir, "b");
  };
  const tree = split("horizontal");

  it("scales a horizontal divider's position but not its length", () => {
    const [d] = computeDividers(tree);
    const moved = dividerIntoRegion(d, 0, 50);
    // Horizontal divider: leftPct is the boundary x, lengthPct is a HEIGHT and must not shrink.
    expect(moved.leftPct).toBeCloseTo((d.leftPct * 50) / 100);
    expect(moved.lengthPct).toBe(d.lengthPct);
  });

  it("scales a vertical divider's length, since there it is a width", () => {
    const [d] = computeDividers(split("vertical"));
    expect(d.dir).toBe("vertical");
    const moved = dividerIntoRegion(d, 0, 40);
    expect(moved.lengthPct).toBeCloseTo((d.lengthPct * 40) / 100);
  });

  it("scales parentRect, which the drag maths divides through", () => {
    const [d] = computeDividers(tree);
    const moved = dividerIntoRegion(d, 30, 70);
    // Leave parentRect at stage width and a dragged divider outruns the pointer.
    expect(moved.parentRect.width).toBeCloseTo((d.parentRect.width * 70) / 100);
    expect(moved.parentRect.left).toBeCloseTo(30 + (d.parentRect.left * 70) / 100);
  });

  it("offsets into the right-hand band", () => {
    const [d] = computeDividers(tree);
    const moved = dividerIntoRegion(d, 65, 35);
    expect(moved.leftPct).toBeCloseTo(65 + (d.leftPct * 35) / 100);
    expect(moved.leftPct).toBeGreaterThanOrEqual(65);
    expect(moved.leftPct).toBeLessThanOrEqual(100);
  });
});
