//! Compatibility for saved translations produced before Markdown text-node extraction.
import type { TranslationResult } from "../ipc/translation";

interface Marker { start: number; end: number; text: string; tightLeft: boolean; tightRight: boolean }

function markers(text: string): Marker[] {
  const result: Marker[] = [];
  let offset = 0; let fence: string | null = null;
  for (const line of text.split("\n")) {
    const boundary = /^ {0,3}(`{3,}|~{3,})/.exec(line)?.[1];
    if (fence || boundary) {
      if (fence) {
        if (boundary && boundary[0] === fence[0] && boundary.length >= fence.length) fence = null;
      } else fence = boundary!;
      offset += line.length + 1; continue;
    }
    if (/^(?: {4}|\t|\s*<)/.test(line)) { offset += line.length + 1; continue; }
    for (let i = 0; i < line.length;) {
      if (line[i] === "\\") { i += 2; continue; }
      if (line[i] === "`") {
        let end = i + 1; while (line[end] === "`") end++;
        const close = line.indexOf(line.slice(i, end), end);
        const length = end - i;
        i = close < 0 ? end : close + length; continue;
      }
      if (line.startsWith("](", i)) {
        let end = i + 2; let depth = 1;
        while (end < line.length && depth) {
          if (line[end] === "\\") { end += 2; continue; }
          if (line[end] === "(") depth++;
          if (line[end] === ")") depth--;
          end++;
        }
        if (!depth) { i = end; continue; }
      }
      if (!"*_~".includes(line[i])) { i++; continue; }
      let end = i + 1; while (line[end] === line[i]) end++;
      result.push({ start: offset + i, end: offset + end, text: line.slice(i, end),
        tightLeft: i > 0 && !/\s/.test(line[i - 1]), tightRight: end < line.length && !/\s/.test(line[end]) });
      i = end;
    }
    offset += line.length + 1;
  }
  return result;
}

function restoreSpacing(original: string, translation: string): string {
  if (original === translation) return translation;
  const source = markers(original); const target = markers(translation);
  // Only repair a matching delimiter sequence; never invent emphasis for translated prose.
  if (!source.length || source.length !== target.length || source.some((marker, i) => marker.text !== target[i].text)) return translation;
  let output = ""; let cursor = 0;
  for (let i = 0; i < target.length; i++) {
    const marker = target[i]; let gap = translation.slice(cursor, marker.start);
    if (i > 0 && source[i - 1].tightRight) gap = gap.replace(/^[^\S\r\n]+/, "");
    if (source[i].tightLeft) gap = gap.replace(/[^\S\r\n]+$/, "");
    output += gap + marker.text; cursor = marker.end;
  }
  let tail = translation.slice(cursor);
  if (source[source.length - 1].tightRight) tail = tail.replace(/^[^\S\r\n]+/, "");
  return output + tail;
}

export function formattedTranslationPairs(pairs: TranslationResult["pairs"], formatVersion = 0): TranslationResult["pairs"] {
  if (formatVersion >= 2) return pairs;
  return pairs.map(pair => ({ ...pair, translation: restoreSpacing(pair.original, pair.translation) }));
}
