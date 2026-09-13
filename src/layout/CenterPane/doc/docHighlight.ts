import { HighlightStyle } from "@codemirror/language";
import { tags } from "@lezer/highlight";

/** Shared Markdown/code highlight palette mapped to Vlinx semantic colors and text levels. */
export const vlxHighlight = HighlightStyle.define([
  // ── Markdown ──
  { tag: tags.heading, color: "var(--accent)", fontWeight: "600" },
  { tag: tags.strong, fontWeight: "700" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.strikethrough, textDecoration: "line-through" },
  { tag: tags.monospace, color: "var(--mag)" },
  { tag: tags.link, color: "var(--cyan)" },
  { tag: tags.url, color: "var(--cyan)", textDecoration: "underline" },
  { tag: tags.quote, color: "var(--text-mid)", fontStyle: "italic" },
  { tag: tags.contentSeparator, color: "var(--text-dim)" },
  // ── General code ──
  { tag: tags.comment, color: "var(--text-dim)", fontStyle: "italic" },
  { tag: tags.meta, color: "var(--text-dim)" },
  { tag: tags.processingInstruction, color: "var(--text-dim)" },
  { tag: tags.keyword, color: "var(--mag)" },
  { tag: tags.string, color: "var(--green)" },
  { tag: tags.number, color: "var(--yellow)" },
  { tag: tags.typeName, color: "var(--yellow)" },
  { tag: tags.className, color: "var(--yellow)" },
  { tag: tags.bool, color: "var(--yellow)" },
  { tag: tags.atom, color: "var(--yellow)" },
  { tag: tags.null, color: "var(--yellow)" },
  { tag: tags.attributeName, color: "var(--yellow)" },
  { tag: tags.function(tags.variableName), color: "var(--cyan)" },
  { tag: tags.function(tags.propertyName), color: "var(--cyan)" },
  { tag: tags.tagName, color: "var(--red)" },
  { tag: tags.regexp, color: "var(--red)" },
  { tag: tags.escape, color: "var(--red)" },
  { tag: tags.definition(tags.variableName), color: "var(--text)" },
  { tag: tags.operator, color: "var(--text-mid)" },
  { tag: tags.punctuation, color: "var(--text-mid)" },
  { tag: tags.bracket, color: "var(--text-mid)" },
]);

