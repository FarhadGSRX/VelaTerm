import { describe, expect, it } from "vitest";
import { Schema } from "@milkdown/kit/prose/model";
import { EditorState } from "@milkdown/kit/prose/state";
import { replaceChangedContent, withHeadingIds } from "./milkdownSync";

const schema = new Schema({
  nodes: {
    doc: { content: "block+" },
    paragraph: { content: "text*", group: "block" },
    heading: { content: "text*", group: "block", attrs: { level: { default: 1 }, id: { default: "" } } },
    text: { group: "inline" },
  },
  marks: { strong: {} },
});
const paragraph = (text: string) => schema.node("paragraph", null, text ? schema.text(text) : undefined);
const doc = (...texts: string[]) => schema.node("doc", null, texts.map(paragraph));

describe("Milkdown companion synchronization", () => {
  it("normalizes generated and duplicate heading IDs without replacing unchanged headings", () => {
    const heading = (id: string) => schema.node("heading", { id }, schema.text("Title"));
    const before = schema.node("doc", null, [heading("title"), heading("title-#2"), paragraph("old")]);
    const parsed = schema.node("doc", null, [heading(""), heading(""), paragraph("new")]);
    const next = withHeadingIds(parsed, node => node.textContent.toLowerCase());
    const tr = replaceChangedContent(EditorState.create({ doc: before }).tr, next)!;
    expect(tr.doc.firstChild).toBe(before.firstChild);
    expect(tr.doc.child(1)).toBe(before.child(1));
    expect(tr.doc.eq(next)).toBe(true);
  });
  it("retains unchanged nodes while updating a later paragraph", () => {
    const before = doc("Keep this heading", "change here", "Keep this ending");
    const next = doc("Keep this heading", "new content here", "Keep this ending");
    const tr = replaceChangedContent(EditorState.create({ doc: before }).tr, next)!;
    expect(tr.doc.eq(next)).toBe(true);
    expect(tr.doc.firstChild).toBe(before.firstChild);
    expect(tr.doc.lastChild).toBe(before.lastChild);
    expect(tr.getMeta("addToHistory")).toBe(false);
  });

  it.each([
    ["abc", "ababc"], ["ababc", "abc"], ["", "insert"], ["remove", ""],
    ["same repeated repeated", "same repeated"], ["line  ", "line   "],
  ])("preserves text when changing %j to %j", (before, after) => {
    const tr = replaceChangedContent(EditorState.create({ doc: doc(before) }).tr, doc(after))!;
    expect(tr.doc.eq(doc(after))).toBe(true);
  });

  it("handles block insertion, deletion, heading conversion, and marks", () => {
    const cases = [
      [doc("one", "two"), doc("one", "middle", "two")],
      [doc("one", "middle", "two"), doc("one", "two")],
      [doc("title"), schema.node("doc", null, schema.node("heading", { level: 2 }, schema.text("title")))],
      [doc("bold"), schema.node("doc", null, schema.node("paragraph", null, schema.text("bold", [schema.mark("strong")])))],
    ];
    for (const [before, after] of cases) {
      const tr = replaceChangedContent(EditorState.create({ doc: before }).tr, after)!;
      expect(tr.doc.eq(after)).toBe(true);
    }
  });

  it("does not create a transaction for unchanged content", () => {
    expect(replaceChangedContent(EditorState.create({ doc: doc("same") }).tr, doc("same"))).toBeNull();
  });
});
