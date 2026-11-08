import { afterEach, expect, it } from "vitest";
import { Crepe } from "@milkdown/crepe";
import { editorViewCtx } from "@milkdown/kit/core";
import { $prose } from "@milkdown/kit/utils";
import { TextSelection } from "@milkdown/kit/prose/state";
import type { EditorView } from "@milkdown/kit/prose/view";
import { typoraKeymap } from "./pmTypora";

// Runs the real Crepe keymap chain, so this catches a Milkdown default that outranks our binding.
let crepe: Crepe | null = null;
afterEach(async () => { await crepe?.destroy(); crepe = null; });

async function mount(markdown: string): Promise<EditorView> {
  const root = document.body.appendChild(document.createElement("div"));
  crepe = new Crepe({
    root,
    defaultValue: markdown,
    features: { [Crepe.Feature.CodeMirror]: false, [Crepe.Feature.ImageBlock]: false, [Crepe.Feature.Latex]: false },
  });
  crepe.editor.use($prose(ctx => typoraKeymap(ctx)));
  await crepe.create();
  let view!: EditorView;
  crepe.editor.action(ctx => { view = ctx.get(editorViewCtx); });
  return view;
}
const press = (view: EditorView, key: string, shiftKey = false) =>
  view.someProp("handleKeyDown", f => f(view, new KeyboardEvent("keydown", { key, shiftKey })));

it("Backspace on a new empty list item removes its number instead of merging it upward", async () => {
  const view = await mount("1. one\n2. two\n");
  // End of "two": doc > ordered_list > second list_item > paragraph.
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 13)));
  expect(view.state.selection.$from.parent.textContent).toBe("two");
  press(view, "Enter");
  press(view, "Backspace");
  const [list, paragraph] = [view.state.doc.child(0), view.state.doc.child(1)];
  expect(list.childCount).toBe(2);
  expect(list.lastChild?.childCount).toBe(1);
  expect(paragraph.type.name).toBe("paragraph");
  expect(view.state.selection.$from.parent).toBe(paragraph);
});

for (const key of ["Backspace", "Shift-Tab"]) {
  it(`${key} moves an unnumbered paragraph under an item out of the list`, async () => {
    const view = await mount("1. one\n2. two\n\n   tail\n");
    const doc = view.state.doc;
    let start = 0;
    doc.descendants((node, pos) => { if (node.isTextblock && node.textContent === "tail") start = pos + 1; });
    expect(view.state.doc.resolve(start).node(-1).type.name).toBe("list_item");
    view.dispatch(view.state.tr.setSelection(TextSelection.create(doc, start)));
    press(view, key === "Shift-Tab" ? "Tab" : key, key === "Shift-Tab");
    const [list, paragraph] = [view.state.doc.child(0), view.state.doc.child(1)];
    expect(list.childCount).toBe(2);
    expect(list.lastChild?.textContent).toBe("two");
    expect([paragraph.type.name, paragraph.textContent]).toEqual(["paragraph", "tail"]);
  });
}
