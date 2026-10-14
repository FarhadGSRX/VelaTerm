//! "Ignore whitespace" for the diff viewer: a line-level diff in which lines differing only in
//! whitespace count as equal. CodeMirror's merge view accepts a replacement diff function; this
//! one turns every distinct normalized line into a single private-use character, lets the
//! library's own algorithm diff those strings, and maps the result back to character offsets.
//! Changes are then whole lines, which is what someone ignoring indentation wants to see.

/** Minimal shape of `@codemirror/merge`'s Change, constructed by the caller's class. */
export interface ChangeLike {
  fromA: number;
  toA: number;
  fromB: number;
  toB: number;
}

type DiffFn<C> = (a: string, b: string) => readonly C[];
type ChangeCtor<C> = new (fromA: number, toA: number, fromB: number, toB: number) => C;

const FIRST = 0xe000;
const LAST = 0xf8ff;

const normalize = (line: string) => line.replace(/\s+/g, "");

/** Start offset of every line, plus the document length at the end. */
function lineStarts(text: string): number[] {
  const starts = [0];
  for (let i = 0; i < text.length; i++) if (text[i] === "\n") starts.push(i + 1);
  starts.push(text.length);
  return starts;
}

/**
 * Diff `a` and `b` ignoring whitespace, or return null when the texts hold more distinct lines
 * than there are token characters or differ in their final newline; the caller then falls back
 * to the ordinary diff.
 */
export function diffIgnoringWhitespace<C extends ChangeLike>(a: string, b: string, diff: DiffFn<ChangeLike>, Change: ChangeCtor<C>): C[] | null {
  // A final newline present on one side only is a character-level change inside the last line;
  // whole-line tokens cannot express it, so let the ordinary diff show it.
  if (a.length && b.length && a.endsWith("\n") !== b.endsWith("\n")) return null;
  const tokens = new Map<string, string>();
  const encode = (text: string) => {
    let out = "";
    for (const line of text.split("\n")) {
      const key = normalize(line);
      let token = tokens.get(key);
      if (token === undefined) {
        if (FIRST + tokens.size > LAST) return null;
        token = String.fromCharCode(FIRST + tokens.size);
        tokens.set(key, token);
      }
      out += token;
    }
    return out;
  };
  const ta = encode(a);
  const tb = encode(b);
  if (ta === null || tb === null) return null;
  const sa = lineStarts(a);
  const sb = lineStarts(b);
  // Token index i is line i; a change covering lines [from, to) spans from that line's start to the
  // next line's start, so the newline travels with the line it ends.
  const offset = (starts: number[], line: number) => starts[Math.min(line, starts.length - 1)];
  return diff(ta, tb).map((c) => new Change(
    offset(sa, c.fromA), offset(sa, c.toA), offset(sb, c.fromB), offset(sb, c.toB),
  ));
}
