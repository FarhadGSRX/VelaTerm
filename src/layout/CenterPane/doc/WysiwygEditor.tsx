//! Thin wrapper around Milkdown Crepe: lifecycle, edit notifications, and Markdown access.
//!
//! defaultValue initializes the document; companion source updates use minimal ProseMirror transactions.

import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useRef,
} from "react";
import { Crepe } from "@milkdown/crepe";
import "@milkdown/crepe/theme/common/style.css";
import { editorViewCtx, parserCtx } from "@milkdown/kit/core";
import { headingIdGenerator } from "@milkdown/kit/preset/commonmark";
import { $prose } from "@milkdown/kit/utils";
import type { EditorView } from "@milkdown/kit/prose/view";
import { Slice } from "@milkdown/kit/prose/model";
import { Plugin, TextSelection } from "@milkdown/kit/prose/state";
import { syntaxHighlighting } from "@codemirror/language";
import { vlxHighlight } from "./docHighlight";
import { t } from "../../../i18n";
import { EMPTY_STATUS, type DocSearchControl } from "./docSearch";
import {
  pmApply,
  pmClear,
  pmNext,
  pmPrev,
  pmReplace,
  pmReplaceAll,
  pmSearchPlugin,
} from "./pmSearch";
import { pmMermaidPlugin } from "./pmMermaid";
import { onUploadDocImage, proxyDocImageURL } from "./docImageIO";
import { stripImageRatioAlt } from "./docImage";
import { replaceChangedContent, withHeadingIds } from "./milkdownSync";

/** Localize Crepe's slash menu, placeholders, and link/image/code widgets. Capture the current locale
 * at mount; DocView rebuilds the editor by key instead of hot-switching it. docPath locates pasted
 * image storage and resolves rendering paths. */
const crepeFeatureConfigs = (docPath: string, textOnly: boolean) => ({
  [Crepe.Feature.Placeholder]: {
    text: t("crepe.placeholder"),
    mode: "block" as const,
  },
  [Crepe.Feature.BlockEdit]: {
    textGroup: {
      label: t("crepe.textGroup"),
      text: { label: t("crepe.paragraph") },
      h1: { label: t("crepe.h1") },
      h2: { label: t("crepe.h2") },
      h3: { label: t("crepe.h3") },
      h4: { label: t("crepe.h4") },
      h5: { label: t("crepe.h5") },
      h6: { label: t("crepe.h6") },
      quote: { label: t("crepe.quote") },
      divider: { label: t("crepe.divider") },
    },
    listGroup: {
      label: t("crepe.listGroup"),
      bulletList: { label: t("crepe.bulletList") },
      orderedList: { label: t("crepe.orderedList") },
      taskList: { label: t("crepe.taskList") },
    },
    advancedGroup: {
      label: t("crepe.advancedGroup"),
      image: textOnly ? null : { label: t("crepe.image") },
      codeBlock: { label: t("crepe.codeBlock") },
      table: { label: t("crepe.table") },
      math: { label: t("crepe.math") },
    },
  },
  [Crepe.Feature.LinkTooltip]: {
    inputPlaceholder: t("crepe.linkPlaceholder"),
  },
  [Crepe.Feature.ImageBlock]: {
    inlineUploadButton: t("crepe.upload"),
    inlineUploadPlaceholderText: t("crepe.orPasteImageLink"),
    blockUploadButton: t("crepe.uploadImage"),
    blockUploadPlaceholderText: t("crepe.orPasteImageLink"),
    blockCaptionPlaceholderText: t("crepe.imageCaption"),
    blockConfirmButton: t("crepe.confirm"),
    // Top-level onUpload/proxyDomURL covers both inline and block images through Crepe's fallback.
    onUpload: (file: File) => textOnly ? Promise.reject(new Error(t("memory.invalid"))) : onUploadDocImage(file, docPath),
    proxyDomURL: (url: string) => textOnly ? "data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=" : proxyDocImageURL(url, docPath),
  },
  [Crepe.Feature.CodeMirror]: {
    theme: syntaxHighlighting(vlxHighlight),
    searchPlaceholder: t("crepe.searchLanguage"),
    noResultText: t("crepe.noResult"),
    copyText: t("common.copy"),
    previewToggleText: (previewOnly: boolean) =>
      previewOnly ? t("crepe.edit") : t("crepe.collapse"),
  },
});

export interface WysiwygHandle {
  /** Restore keyboard focus without changing the current selection. */
  focus: () => void;
  /** Return the current Markdown serialization, or null before the editor is ready. */
  getMarkdown: () => string | null;
  /** Apply companion Markdown with a minimal ProseMirror transaction. */
  setMarkdown: (markdown: string) => void;
  /** Insert Markdown at the current selection, parsing it into ProseMirror nodes. */
  insertText: (markdown: string) => void;
  scrollToHeading: (index: number) => void;
  /** Return current HTML for printing/PDF export, or null before the editor is ready. */
  getHtml: () => string | null;
  /** Shared find/replace controls, effective after the ProseMirror view is ready. */
  search: DocSearchControl;
  /** Position the caret from an editor-gutter click, preserving the anchor for Shift-click. */
  placeCaret: (x: number, y: number, extend: boolean) => void;
}

export const WysiwygEditor = forwardRef<
  WysiwygHandle,
  {
    defaultValue: string;
    /** Current absolute document path, empty for drafts; used for sibling assets and relative images. */
    docPath: string;
    /** Called after genuine user edits so DocView can mark dirty and decide persistence. */
    onEdited: () => void;
    /** Prevent document image reads and uploads for text-only knowledge entries. */
    textOnly?: boolean;
    onReady?: () => void;
    onError?: (error: unknown) => void;
  }
>(function WysiwygEditor({ defaultValue, docPath, onEdited, textOnly = false, onReady, onError }, ref) {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const crepeRef = useRef<Crepe | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  // markdownUpdated before create() completes is parser/normalization initialization noise, not user editing.
  const readyRef = useRef(false);
  const syncingRef = useRef(false);
  const lifecycleRef = useRef({ onReady, onError });
  lifecycleRef.current = { onReady, onError };
  const onEditedRef = useRef(onEdited);
  onEditedRef.current = onEdited;

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const crepe = new Crepe({
      root,
      defaultValue,
      featureConfigs: crepeFeatureConfigs(docPath, textOnly),
    });
    // Register find/replace before create(), the stable Milkdown pattern versus mutating state afterward.
    crepe.editor.use($prose(() => pmSearchPlugin()));
    // Mermaid plugin renders decorations below fenced blocks without changing document serialization.
    crepe.editor.use($prose(() => pmMermaidPlugin()));
    // Observe transactions immediately without serializing the document on every keystroke.
    crepe.editor.use($prose(() => new Plugin({
      view: () => ({
        update(view, previous) {
          if (!readyRef.current || syncingRef.current || view.state.doc.eq(previous.doc)) return;
          if (root.contains(document.activeElement)) onEditedRef.current();
        },
      }),
    })));
    crepeRef.current = crepe;
    let disposed = false;
    void crepe.create().then(() => {
      if (disposed) return;
      readyRef.current = true;
      // Retain the active view for search controls; the plugin was registered before create().
      crepe.editor.action((ctx) => {
        viewRef.current = ctx.get(editorViewCtx);
      });
      lifecycleRef.current.onReady?.();
    }).catch(error => { if (!disposed) lifecycleRef.current.onError?.(error); });
    return () => {
      disposed = true;
      readyRef.current = false;
      viewRef.current = null;
      crepeRef.current = null;
      // Guard destruction because occasional ProseMirror teardown races must not interrupt React unmount.
      try {
        void crepe.destroy();
      } catch {
        /* Defensive fallback. */
      }
    };
    // Create once per mount; the parent changes key only for reloads or initialization retries.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useImperativeHandle(ref, () => ({
    focus: () => viewRef.current?.focus(),
    setMarkdown: (markdown) => {
      const crepe = crepeRef.current;
      const view = viewRef.current;
      if (!crepe || !view || !readyRef.current) return;
      crepe.editor.action(ctx => {
        const next = ctx.get(parserCtx)(markdown);
        if (!next) return;
        const normalized = withHeadingIds(next, ctx.get(headingIdGenerator.key));
        const transaction = replaceChangedContent(view.state.tr, normalized);
        if (!transaction) return;
        syncingRef.current = true;
        try { view.dispatch(transaction); } finally { syncingRef.current = false; }
      });
    },
    scrollToHeading: index => {
      rootRef.current?.querySelectorAll<HTMLElement>(".ProseMirror :is(h1,h2,h3,h4,h5,h6)")[index]?.scrollIntoView({ block: "start" });
    },
    insertText: markdown => {
      const crepe = crepeRef.current;
      const view = viewRef.current;
      if (!crepe || !view || !readyRef.current) return;
      crepe.editor.action(ctx => {
        const parsed = ctx.get(parserCtx)(markdown);
        if (!parsed) return;
        const transaction = view.state.tr.replaceSelection(new Slice(parsed.content, 0, 0)).scrollIntoView();
        view.dispatch(transaction);
      });
    },
    placeCaret: (x, y, extend) => {
      const view = viewRef.current;
      if (!view) return;
      const rect = view.dom.getBoundingClientRect();
      const hit = view.posAtCoords({
        left: Math.min(Math.max(x, rect.left + 1), rect.right - 1),
        top: Math.min(Math.max(y, rect.top + 1), rect.bottom - 1),
      });
      const doc = view.state.doc;
      const pos = hit?.pos ?? (y < rect.top ? 0 : doc.content.size);
      const near = TextSelection.near(doc.resolve(pos));
      const selection = extend
        ? TextSelection.between(doc.resolve(view.state.selection.anchor), near.$head)
        : near;
      view.dispatch(view.state.tr.setSelection(selection).scrollIntoView());
      view.focus();
    },
    getMarkdown: () => {
      const crepe = crepeRef.current;
      if (!crepe || !readyRef.current) return null;
      try {
        return stripImageRatioAlt(crepe.getMarkdown());
      } catch {
        return null;
      }
    },
    getHtml: () => {
      if (!viewRef.current || !readyRef.current) return null;
      // The `.ProseMirror` editor node's innerHTML is the rendered HTML.
      return viewRef.current.dom.innerHTML;
    },
    search: {
      apply: (o) =>
        viewRef.current ? pmApply(viewRef.current, o.query, o.caseSensitive) : EMPTY_STATUS,
      next: () => (viewRef.current ? pmNext(viewRef.current) : EMPTY_STATUS),
      prev: () => (viewRef.current ? pmPrev(viewRef.current) : EMPTY_STATUS),
      replace: (txt) => (viewRef.current ? pmReplace(viewRef.current, txt) : EMPTY_STATUS),
      replaceAll: (txt) => (viewRef.current ? pmReplaceAll(viewRef.current, txt) : EMPTY_STATUS),
      clear: () => {
        if (viewRef.current) pmClear(viewRef.current);
      },
    },
  }));

  return <div className="docview-wysiwyg" ref={rootRef} />;
});
