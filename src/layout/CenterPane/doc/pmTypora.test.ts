import { afterEach, describe, expect, it } from "vitest";
import { Schema } from "@milkdown/kit/prose/model";
import { EditorState, TextSelection } from "@milkdown/kit/prose/state";
import { EditorView } from "@milkdown/kit/prose/view";
import { autoPairPlugin, focusBlockPlugin, liftListBlock, liftListItemAtStart } from "./pmTypora";

const schema = new Schema({
  nodes: {
    doc: { content: "block+" },
    paragraph: { content: "text*", group: "block", toDOM: () => ["p", 0] },
    code_block: { content: "text*", group: "block", code: true, toDOM: () => ["pre", 0] },
    text: { group: "inline" },
  },
  marks: { inlineCode: { toDOM: () => ["code", 0] } },
});

let view: EditorView | null = null;
afterEach(() => { view?.destroy(); view = null; });

function mount(text: string, caret = text.length, node = "paragraph") {
  const doc = schema.node("doc", null, [schema.node(node, null, text ? schema.text(text) : undefined)]);
  const state = EditorState.create({ doc, plugins: [autoPairPlugin(), focusBlockPlugin(() => true)] });
  view = new EditorView(document.createElement("div"), {
    state: state.apply(state.tr.setSelection(TextSelection.create(doc, caret + 1))),
  });
  return view;
}
const type = (v: EditorView, text: string) => {
  const { from, to } = v.state.selection;
  const handled = v.someProp("handleTextInput", f => f(v, from, to, text, () => v.state.tr.insertText(text, from, to)));
  if (!handled) v.dispatch(v.state.tr.insertText(text, from, to));
};
const backspace = (v: EditorView) =>
  v.someProp("handleKeyDown", f => f(v, new KeyboardEvent("keydown", { key: "Backspace" })));
const text = (v: EditorView) => v.state.doc.textContent;
const caret = (v: EditorView) => v.state.selection.head - 1;

describe("auto-pairing", () => {
  it("pairs brackets, steps over the inserted closer, and deletes an empty pair", () => {
    const v = mount("");
    type(v, "(");
    expect([text(v), caret(v)]).toEqual(["()", 1]);
    type(v, "x");
    type(v, ")");
    expect([text(v), caret(v)]).toEqual(["(x)", 3]);
    type(v, "[");
    expect(backspace(v)).toBe(true);
    expect(text(v)).toBe("(x)");
  });

  it("never swallows a closer the user typed", () => {
    const v = mount("f)", 1);
    type(v, ")");
    expect(text(v)).toBe("f))");
  });

  it("leaves apostrophes and brackets before words alone", () => {
    const v = mount("don", 3);
    type(v, "'");
    expect(text(v)).toBe("don'");
    const w = mount("word", 0);
    type(w, "(");
    expect(text(w)).toBe("(word");
  });

  it("wraps a selection", () => {
    const v = mount("hello");
    v.dispatch(v.state.tr.setSelection(TextSelection.create(v.state.doc, 1, 6)));
    type(v, '"');
    expect(text(v)).toBe('"hello"');
    expect([v.state.selection.from, v.state.selection.to]).toEqual([2, 7]);
  });

  it("does not pair inside code", () => {
    const v = mount("", 0, "code_block");
    type(v, "(");
    expect(text(v)).toBe("(");
  });
});

describe("focus mode", () => {
  it("marks only the top-level block that holds the caret", () => {
    const doc = schema.node("doc", null, [
      schema.node("paragraph", null, schema.text("one")),
      schema.node("paragraph", null, schema.text("two")),
    ]);
    const state = EditorState.create({ doc, plugins: [focusBlockPlugin(() => true)] });
    view = new EditorView(document.createElement("div"), {
      state: state.apply(state.tr.setSelection(TextSelection.create(doc, 7))),
    });
    const blocks = view.dom.querySelectorAll("p");
    expect(blocks[0].classList.contains("vlx-current-block")).toBe(false);
    expect(blocks[1].classList.contains("vlx-current-block")).toBe(true);
  });
});

describe("list backspace", () => {
  const listSchema = new Schema({
    nodes: {
      doc: { content: "block+" },
      paragraph: { content: "text*", group: "block", toDOM: () => ["p", 0] },
      ordered_list: { content: "list_item+", group: "block", toDOM: () => ["ol", 0] },
      list_item: { content: "paragraph block*", defining: true, toDOM: () => ["li", 0] },
      text: { group: "inline" },
    },
  });
  const p = (text = "") => listSchema.node("paragraph", null, text ? listSchema.text(text) : undefined);
  const li = (...content: ReturnType<typeof p>[]) => listSchema.node("list_item", null, content);
  const ol = (...items: ReturnType<typeof li>[]) => listSchema.node("ordered_list", null, items);
  const lift = liftListItemAtStart(listSchema.nodes.list_item);
  // Run the command with the caret at `pos` and describe the resulting block structure.
  const run = (doc: ReturnType<typeof p>, pos: number, command = lift) => {
    let state = EditorState.create({ doc });
    state = state.apply(state.tr.setSelection(TextSelection.create(doc, pos)));
    const handled = command(state, tr => { state = state.apply(tr); });
    return { handled, doc: state.doc.toString() };
  };

  it("turns an empty last item into a paragraph after the list", () => {
    const doc = listSchema.node("doc", null, [ol(li(p("one")), li(p("two")), li(p()))]);
    // doc > ol > li(3rd) > p: the empty paragraph opens at 1 + 7 + 7 + 2.
    expect(run(doc, 17)).toEqual({
      handled: true,
      doc: 'doc(ordered_list(list_item(paragraph("one")), list_item(paragraph("two"))), paragraph)',
    });
  });

  it("splits the list around a non-empty middle item", () => {
    const doc = listSchema.node("doc", null, [ol(li(p("one")), li(p("two")), li(p("three")))]);
    expect(run(doc, 10).doc).toBe(
      'doc(ordered_list(list_item(paragraph("one"))), paragraph("two"), ordered_list(list_item(paragraph("three"))))',
    );
  });

  it("outdents a nested item by one level", () => {
    const doc = listSchema.node("doc", null, [ol(li(p("one"), ol(li(p("inner")))))]);
    expect(run(doc, 10).doc).toBe(
      'doc(ordered_list(list_item(paragraph("one")), list_item(paragraph("inner"))))',
    );
  });

  it("moves an unnumbered paragraph under an item out of the list", () => {
    const doc = listSchema.node("doc", null, [ol(li(p("one")), li(p("two"), p("tail"), p("more")), li(p("three")))]);
    // Caret at the start of "tail", the second paragraph of the second item.
    expect(run(doc, 15).doc).toBe(
      'doc(ordered_list(list_item(paragraph("one")), list_item(paragraph("two"))), paragraph("tail"), paragraph("more"), '
        + 'ordered_list(list_item(paragraph("three"))))',
    );
  });

  it("moves an unnumbered paragraph under a nested item into the enclosing item", () => {
    const doc = listSchema.node("doc", null, [ol(li(p("one"), ol(li(p("inner"), p("tail")))))]);
    expect(run(doc, 17).doc).toBe(
      'doc(ordered_list(list_item(paragraph("one"), ordered_list(list_item(paragraph("inner"))), paragraph("tail"))))',
    );
  });

  it("leaves mid-text carets to the default behavior", () => {
    const doc = listSchema.node("doc", null, [ol(li(p("one"), p("two")))]);
    expect(run(doc, 4).handled).toBe(false);
    expect(run(doc, 9).handled).toBe(false);
  });

  it("outdents an unnumbered paragraph from anywhere in it but leaves numbered ones to Milkdown", () => {
    const doc = listSchema.node("doc", null, [ol(li(p("one"), p("two")))]);
    const outdent = liftListBlock(listSchema.nodes.list_item);
    expect(run(doc, 10, outdent).doc).toBe('doc(ordered_list(list_item(paragraph("one"))), paragraph("two"))');
    expect(run(doc, 4, outdent).handled).toBe(false);
  });
});
