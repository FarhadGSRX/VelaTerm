//! Typora-style editing behavior for the visual editor: formatting shortcuts, bracket auto-pairing,
//! Cmd/Ctrl+click on links, and the decorations behind focus and typewriter modes.
//!
//! These plugins are registered through `$prose`, which places them ahead of Milkdown's own keymap,
//! so a binding here takes precedence over a Milkdown default with the same keys.

import type { Ctx } from "@milkdown/kit/ctx";
import { commandsCtx } from "@milkdown/kit/core";
import {
  createCodeBlockCommand,
  headingSchema,
  listItemSchema,
  paragraphSchema,
  toggleInlineCodeCommand,
  wrapInBlockTypeCommand,
  wrapInBlockquoteCommand,
  wrapInBulletListCommand,
  wrapInOrderedListCommand,
} from "@milkdown/kit/preset/commonmark";
import { insertTableCommand, toggleStrikethroughCommand } from "@milkdown/kit/preset/gfm";
import { toggleLinkCommand } from "@milkdown/kit/component/link-tooltip";
import { chainCommands } from "@milkdown/kit/prose/commands";
import { undoInputRule } from "@milkdown/kit/prose/inputrules";
import { keymap } from "@milkdown/kit/prose/keymap";
import { liftListItem } from "@milkdown/kit/prose/schema-list";
import { Plugin, PluginKey, TextSelection, type Command, type EditorState } from "@milkdown/kit/prose/state";
import { Decoration, DecorationSet, type EditorView } from "@milkdown/kit/prose/view";
import type { NodeType } from "@milkdown/kit/prose/model";
import { liftTarget } from "@milkdown/kit/prose/transform";

const IS_MAC = typeof navigator !== "undefined" && /mac|iphone|ipad|ipod/i.test(navigator.platform || navigator.userAgent || "");

/** The platform's link-follow modifier: Cmd on macOS (Ctrl+click is a right click there), Ctrl elsewhere. */
export const isLinkModifier = (event: MouseEvent | KeyboardEvent) => (IS_MAC ? event.metaKey : event.ctrlKey);

/** Nearest list node around the selection, with its type. */
function currentList(state: EditorState): NodeType | null {
  const { $from } = state.selection;
  for (let depth = $from.depth; depth > 0; depth--) {
    const node = $from.node(depth);
    if (node.type.name === "bullet_list" || node.type.name === "ordered_list") return node.type;
  }
  return null;
}

/** Heading level of the textblock around the selection; 0 for a paragraph, null for anything else. */
function headingLevel(state: EditorState): number | null {
  const parent = state.selection.$from.parent;
  if (parent.type.name === "heading") return parent.attrs.level as number;
  return parent.type.name === "paragraph" ? 0 : null;
}

/**
 * Move a later block of a list item (an unnumbered paragraph under the marker), together with the
 * blocks after it in the same item, out of the item: to the top level for a top-level list, or into
 * the enclosing item for a nested one. Returns false in an item's first block, so Milkdown's own
 * lift handles the marker there.
 */
export const liftListBlock = (itemType: NodeType): Command => (state, dispatch) => {
  const { $from } = state.selection;
  if ($from.depth < 2 || $from.node(-1).type !== itemType || $from.index(-1) === 0) return false;
  const { doc } = state;
  const range = doc.resolve($from.before()).blockRange(doc.resolve($from.end(-1)));
  const target = range && liftTarget(range);
  if (!range || target == null) return false;
  dispatch?.(state.tr.lift(range, target).scrollIntoView());
  return true;
};

/**
 * Backspace at the very start of a list item's block removes the list structure before joining text,
 * as in Typora: a numbered paragraph loses its marker (a nested item moves out one level), and an
 * unnumbered paragraph under the marker moves out of the item. Milkdown's default joins the block into
 * the text above instead, so the number could never be removed.
 */
export const liftListItemAtStart = (itemType: NodeType): Command => (state, dispatch, view) => {
  const { selection } = state;
  if (!(selection instanceof TextSelection) || !selection.empty) return false;
  const { $from } = selection;
  if ($from.parentOffset !== 0 || $from.depth < 2 || $from.node(-1).type !== itemType) return false;
  if ($from.index(-1) !== 0) return liftListBlock(itemType)(state, dispatch, view);
  return liftListItem(itemType)(state, dispatch, view);
};

/** Formatting shortcuts following Typora's defaults, adjusted where the app already owns a chord. */
export function typoraKeymap(ctx: Ctx): Plugin {
  const commands = () => ctx.get(commandsCtx);
  const run = (fn: () => boolean): Command => () => fn();
  const setHeading = (level: number): Command => (state, dispatch) => {
    const { from, to } = state.selection;
    const tr = level === 0
      ? state.tr.setBlockType(from, to, paragraphSchema.type(ctx))
      : state.tr.setBlockType(from, to, headingSchema.type(ctx), { level });
    dispatch?.(tr.scrollIntoView());
    return true;
  };
  // Cmd/Ctrl+= promotes paragraph → H6 → … → H1; Cmd/Ctrl+- demotes back down to a paragraph.
  const promote: Command = (state, dispatch) => {
    const level = headingLevel(state);
    if (level == null || level === 1) return level === 1;
    return setHeading(level === 0 ? 6 : level - 1)(state, dispatch);
  };
  const demote: Command = (state, dispatch) => {
    const level = headingLevel(state);
    if (!level) return level === 0;
    return setHeading(level === 6 ? 0 : level + 1)(state, dispatch);
  };
  // A list shortcut inside a list of the same kind lifts the items out, mirroring Typora's toggle.
  const list = (name: "bullet_list" | "ordered_list", wrap: Command): Command => (state, dispatch, view) => {
    if (currentList(state)?.name === name) return liftListItem(listItemSchema.type(ctx))(state, dispatch, view);
    return wrap(state, dispatch, view);
  };
  const clearFormat: Command = (state, dispatch) => {
    const { from, to, empty } = state.selection;
    if (empty) {
      dispatch?.(state.tr.setStoredMarks([]));
      return true;
    }
    dispatch?.(state.tr.removeMark(from, to));
    return true;
  };
  const bullet = list("bullet_list", run(() => commands().call(wrapInBulletListCommand.key)));
  const ordered = list("ordered_list", run(() => commands().call(wrapInOrderedListCommand.key)));
  const task = run(() => commands().call(wrapInBlockTypeCommand.key, { nodeType: listItemSchema.type(ctx), attrs: { checked: false } }));
  const quote = run(() => commands().call(wrapInBlockquoteCommand.key));
  const code = run(() => commands().call(createCodeBlockCommand.key));
  const math = run(() => commands().call(createCodeBlockCommand.key, "LaTeX"));
  const table = run(() => commands().call(insertTableCommand.key, { row: 3, col: 3 }));
  const inlineCode = run(() => commands().call(toggleInlineCodeCommand.key));
  const strike = run(() => commands().call(toggleStrikethroughCommand.key));
  const common: Record<string, Command> = {
    // Undoing a just-applied input rule (typing "1. " then Backspace) still takes precedence.
    Backspace: chainCommands(undoInputRule, liftListItemAtStart(listItemSchema.type(ctx))),
    // Outdenting an unnumbered paragraph moves only it out; Milkdown's lift would unnumber the whole item.
    "Shift-Tab": liftListBlock(listItemSchema.type(ctx)),
    "Mod-[": liftListBlock(listItemSchema.type(ctx)),
    "Mod-=": promote,
    "Mod-Shift-=": promote,
    "Mod--": demote,
    "Mod-k": run(() => commands().call(toggleLinkCommand.key)),
    "Mod-\\": clearFormat,
  };
  const platform: Record<string, Command> = IS_MAC
    ? {
        "Mod-Alt-q": quote,
        "Mod-Alt-o": ordered,
        "Mod-Alt-u": bullet,
        "Mod-Alt-x": task,
        "Mod-Alt-c": code,
        "Mod-Alt-b": math,
        "Mod-Alt-t": table,
        "Ctrl-`": inlineCode,
        "Ctrl-Shift-`": strike,
      }
    : {
        "Ctrl-Shift-q": quote,
        "Ctrl-Shift-[": ordered,
        "Ctrl-Shift-]": bullet,
        "Ctrl-Shift-k": code,
        "Ctrl-Shift-m": math,
        "Ctrl-t": table,
        "Ctrl-Shift-`": inlineCode,
        "Alt-Shift-5": strike,
      };
  return keymap({ ...common, ...platform });
}

// ── Bracket and quote auto-pairing ──

const PAIRS: Record<string, string> = { "(": ")", "[": "]", "{": "}", '"': '"', "'": "'" };
const WORD = /[\p{L}\p{N}_]/u;
/** Characters after the caret that still allow an opening bracket to pair. */
const PAIR_BEFORE = /[\s)\]}.,;:!?'"，。；：！？、）】」』]/u;

type PairMeta = { add: number } | { remove: number };
const pairKey = new PluginKey<number[]>("vlxAutoPair");

function inCode(state: EditorState, pos: number): boolean {
  const $pos = state.doc.resolve(pos);
  if ($pos.parent.type.spec.code) return true;
  return ($pos.marks()).some(mark => mark.type.name === "inlineCode");
}

/**
 * Auto-pair brackets and quotes as they are typed. Only closers this plugin inserted are tracked, so
 * typing a closer steps over an inserted one but never swallows a character the user typed.
 */
export function autoPairPlugin(): Plugin<number[]> {
  return new Plugin<number[]>({
    key: pairKey,
    state: {
      init: () => [],
      apply(tr, tracked) {
        let next = tr.docChanged
          ? tracked.flatMap(pos => {
            const mapped = tr.mapping.mapResult(pos, 1);
            return mapped.deleted ? [] : [mapped.pos];
          })
          : tracked;
        const meta = tr.getMeta(pairKey) as PairMeta | undefined;
        if (meta && "add" in meta) next = [...next, meta.add];
        if (meta && "remove" in meta) next = next.filter(pos => pos !== meta.remove);
        return next;
      },
    },
    props: {
      handleTextInput(view, from, to, text) {
        if (view.composing || text.length !== 1) return false;
        const { state } = view;
        if (inCode(state, from)) return false;
        const tracked = pairKey.getState(state) ?? [];
        if (from === to && tracked.includes(from) && state.doc.textBetween(from, Math.min(from + 1, state.doc.content.size)) === text) {
          view.dispatch(state.tr.setSelection(TextSelection.create(state.doc, from + 1)).setMeta(pairKey, { remove: from }));
          return true;
        }
        const close = PAIRS[text];
        if (!close) return false;
        const $from = state.doc.resolve(from);
        if (from !== to) {
          if (!$from.sameParent(state.doc.resolve(to))) return false;
          const tr = state.tr.insertText(close, to).insertText(text, from);
          view.dispatch(tr.setSelection(TextSelection.create(tr.doc, from + 1, to + 1)));
          return true;
        }
        const before = from > $from.start() ? state.doc.textBetween(from - 1, from) : "";
        const after = from < $from.end() ? state.doc.textBetween(from, from + 1) : "";
        if (text === close ? WORD.test(before) || WORD.test(after) : after !== "" && !PAIR_BEFORE.test(after)) return false;
        const tr = state.tr.insertText(text + close, from, to);
        view.dispatch(tr.setSelection(TextSelection.create(tr.doc, from + 1)).setMeta(pairKey, { add: from + 1 }));
        return true;
      },
      handleKeyDown(view, event) {
        if (event.key !== "Backspace" || event.metaKey || event.ctrlKey || event.altKey || view.composing) return false;
        const { state } = view;
        const { empty, head } = state.selection;
        if (!empty || !(pairKey.getState(state) ?? []).includes(head)) return false;
        const $head = state.selection.$head;
        if (head <= $head.start() || head >= $head.end()) return false;
        const pair = state.doc.textBetween(head - 1, head + 1);
        if (PAIRS[pair[0]] !== pair[1]) return false;
        view.dispatch(state.tr.delete(head - 1, head + 1).setMeta(pairKey, { remove: head }));
        return true;
      },
    },
  });
}

// ── Links ──

/**
 * Follow links on Cmd/Ctrl+click. While the modifier is held, links show a pointer cursor through the
 * `vlx-link-modifier` class, the same affordance Typora gives.
 */
export function linkClickPlugin(open: (href: string) => void): Plugin {
  return new Plugin({
    view(view) {
      const toggle = (event: KeyboardEvent) => view.dom.classList.toggle("vlx-link-modifier", isLinkModifier(event));
      const clear = () => view.dom.classList.remove("vlx-link-modifier");
      window.addEventListener("keydown", toggle);
      window.addEventListener("keyup", toggle);
      window.addEventListener("blur", clear);
      return {
        destroy() {
          window.removeEventListener("keydown", toggle);
          window.removeEventListener("keyup", toggle);
          window.removeEventListener("blur", clear);
        },
      };
    },
    props: {
      handleDOMEvents: {
        mousedown(_view, event) {
          // Keep the caret where it is; the click below performs the navigation.
          if (event.button !== 0 || !isLinkModifier(event)) return false;
          const anchor = (event.target as HTMLElement | null)?.closest?.("a[href]");
          if (!anchor) return false;
          event.preventDefault();
          return true;
        },
        click(view, event) {
          if (event.button !== 0 || !isLinkModifier(event)) return false;
          const anchor = (event.target as HTMLElement | null)?.closest?.("a[href]");
          if (!anchor || !view.dom.contains(anchor)) return false;
          event.preventDefault();
          open(anchor.getAttribute("href") ?? "");
          return true;
        },
      },
    },
  });
}

// ── Focus and typewriter modes ──

const refreshKey = new PluginKey("vlxWritingModes");

/** Ask the writing-mode plugins to re-read their flags. */
export function refreshWritingModes(view: EditorView) {
  view.dispatch(view.state.tr.setMeta(refreshKey, true));
}

/** Mark the top-level block holding the caret; CSS dims the rest while focus mode is on. */
export function focusBlockPlugin(active: () => boolean): Plugin {
  return new Plugin({
    props: {
      decorations(state) {
        if (!active()) return null;
        const { selection, doc } = state;
        const start = selection.$head.depth > 0 ? selection.$head.before(1) : selection.from;
        const node = doc.nodeAt(start);
        if (!node) return null;
        return DecorationSet.create(doc, [Decoration.node(start, start + node.nodeSize, { class: "vlx-current-block" })]);
      },
    },
  });
}

const pointerKey = new PluginKey<boolean>("vlxTypewriterPointer");

/**
 * Keep the caret line vertically centered in `scroller` after typing or keyboard navigation. Mouse
 * selections are left alone so a click or drag never scrolls the text away from the pointer.
 */
export function typewriterPlugin(active: () => boolean, scroller: () => HTMLElement | null): Plugin<boolean> {
  return new Plugin<boolean>({
    key: pointerKey,
    state: {
      init: () => false,
      apply: (tr, pointer) => (tr.getMeta("pointer") ? true : tr.docChanged || tr.selectionSet ? false : pointer),
    },
    view: () => {
      let frame = 0;
      return {
        update(view, previous) {
          if (!active() || view.composing || !view.hasFocus() || pointerKey.getState(view.state)) return;
          if (view.state.doc.eq(previous.doc) && view.state.selection.eq(previous.selection)) return;
          cancelAnimationFrame(frame);
          // Run after ProseMirror's own scroll-into-view so the final position is ours.
          frame = requestAnimationFrame(() => {
            const box = scroller();
            if (!box || !view.dom.isConnected) return;
            const caret = view.coordsAtPos(view.state.selection.head);
            const rect = box.getBoundingClientRect();
            box.scrollTop += (caret.top + caret.bottom) / 2 - (rect.top + rect.height / 2);
          });
        },
        destroy: () => cancelAnimationFrame(frame),
      };
    },
  });
}
