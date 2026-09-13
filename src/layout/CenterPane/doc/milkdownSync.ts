import type { Node as ProseNode } from "@milkdown/kit/prose/model";
import type { Transaction } from "@milkdown/kit/prose/state";
import { Transform } from "@milkdown/kit/prose/transform";

/** Match Milkdown's heading normalization before comparing parsed and live documents. */
export function withHeadingIds(doc: ProseNode, getId: (node: ProseNode) => string): ProseNode {
  const transform = new Transform(doc);
  const counts = new Map<string, number>();
  doc.descendants((node, pos) => {
    if (node.type.name !== "heading" || !node.textContent.trim()) return;
    const base = getId(node);
    const count = (counts.get(base) ?? 0) + 1;
    counts.set(base, count);
    const id = count === 1 ? base : `${base}-#${count}`;
    if (node.attrs.id !== id) transform.setNodeMarkup(pos, undefined, { ...node.attrs, id });
  });
  return transform.doc;
}

/** Preserve unchanged nodes, selections, and node views when Markdown arrives from the code pane. */
export function replaceChangedContent(tr: Transaction, next: ProseNode): Transaction | null {
  const start = tr.doc.content.findDiffStart(next.content);
  if (start == null) return null;
  const end = tr.doc.content.findDiffEnd(next.content) ?? { a: start, b: start };
  const overlap = start - Math.min(end.a, end.b);
  if (overlap > 0) { end.a += overlap; end.b += overlap; }
  return tr.replace(start, end.a, next.slice(start, end.b)).setMeta("addToHistory", false);
}
