//! Turns a command's raw output, ANSI escape sequences included, into styled runs of text.
//!
//! The log dialog shows output that was written for a terminal: colours, bold, and progress lines
//! redrawn in place with `\r`. Only text styling (SGR) is kept. Every other control sequence is dropped,
//! and a line rewritten with `\r` keeps only its last version, so the dialog reads like the final screen.

export interface AnsiStyle {
  fg?: string;
  bg?: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
  strike?: boolean;
  inverse?: boolean;
}

export interface AnsiSpan {
  text: string;
  style: AnsiStyle;
}

/** The 16 basic colours, mapped onto the theme so they follow light and dark mode. */
const BASIC = [
  "var(--text-faint)",
  "var(--red)",
  "var(--green)",
  "var(--yellow)",
  "var(--blue)",
  "var(--mag)",
  "var(--cyan)",
  "var(--text)",
];

function basic(index: number): string {
  return BASIC[index % 8];
}

/** A colour from the xterm 256-colour palette. */
function palette(index: number): string | undefined {
  if (!Number.isInteger(index) || index < 0 || index > 255) return undefined;
  if (index < 16) return basic(index);
  if (index < 232) {
    const n = index - 16;
    const level = (v: number) => (v === 0 ? 0 : 55 + v * 40);
    return `rgb(${level(Math.floor(n / 36))}, ${level(Math.floor(n / 6) % 6)}, ${level(n % 6)})`;
  }
  const gray = 8 + (index - 232) * 10;
  return `rgb(${gray}, ${gray}, ${gray})`;
}

function rgb(r: number, g: number, b: number): string | undefined {
  const ok = (v: number) => Number.isInteger(v) && v >= 0 && v <= 255;
  return ok(r) && ok(g) && ok(b) ? `rgb(${r}, ${g}, ${b})` : undefined;
}

/**
 * Reads an extended colour (`38;5;n`, `38;2;r;g;b`, or the colon forms) starting at `params[i]`.
 * Returns the colour and how many extra parameters the semicolon form consumed.
 */
function extendedColor(params: string[], i: number): [string | undefined, number] {
  const head = params[i];
  if (head.includes(":")) {
    const sub = head.split(":");
    if (sub[1] === "5") return [palette(Number(sub[2])), 0];
    if (sub[1] === "2") {
      const [r, g, b] = sub.slice(-3).map(Number);
      return [rgb(r, g, b), 0];
    }
    return [undefined, 0];
  }
  if (params[i + 1] === "5") return [palette(Number(params[i + 2])), 2];
  if (params[i + 1] === "2") {
    return [rgb(Number(params[i + 2]), Number(params[i + 3]), Number(params[i + 4])), 4];
  }
  return [undefined, 0];
}

function applySgr(style: AnsiStyle, body: string): AnsiStyle {
  const params = body === "" ? ["0"] : body.split(";");
  let next: AnsiStyle = { ...style };
  for (let i = 0; i < params.length; i++) {
    const code = Number(params[i].split(":")[0] || "0");
    if (code === 0) next = {};
    else if (code === 1) next.bold = true;
    else if (code === 2) next.dim = true;
    else if (code === 3) next.italic = true;
    else if (code === 4) next.underline = true;
    else if (code === 7) next.inverse = true;
    else if (code === 9) next.strike = true;
    else if (code === 22) next.bold = next.dim = false;
    else if (code === 23) next.italic = false;
    else if (code === 24) next.underline = false;
    else if (code === 27) next.inverse = false;
    else if (code === 29) next.strike = false;
    else if (code >= 30 && code <= 37) next.fg = basic(code - 30);
    else if (code === 39) next.fg = undefined;
    else if (code >= 40 && code <= 47) next.bg = basic(code - 40);
    else if (code === 49) next.bg = undefined;
    else if (code >= 90 && code <= 97) next.fg = basic(code - 90);
    else if (code >= 100 && code <= 107) next.bg = basic(code - 100);
    else if (code === 38 || code === 48) {
      const [color, used] = extendedColor(params, i);
      if (code === 38) next.fg = color;
      else next.bg = color;
      i += used;
    }
  }
  return next;
}

function sameStyle(a: AnsiStyle, b: AnsiStyle): boolean {
  return (
    a.fg === b.fg &&
    a.bg === b.bg &&
    !!a.bold === !!b.bold &&
    !!a.dim === !!b.dim &&
    !!a.italic === !!b.italic &&
    !!a.underline === !!b.underline &&
    !!a.strike === !!b.strike &&
    !!a.inverse === !!b.inverse
  );
}

export function parseAnsi(input: string): AnsiSpan[] {
  const done: AnsiSpan[] = [];
  // Spans of the line being written. A `\r` marks it for replacement by whatever text comes next.
  let line: AnsiSpan[] = [];
  let rewind = false;
  let style: AnsiStyle = {};

  const push = (text: string) => {
    if (rewind) {
      line = [];
      rewind = false;
    }
    const last = line[line.length - 1];
    if (last && sameStyle(last.style, style)) last.text += text;
    else line.push({ text, style });
  };
  const endLine = () => {
    rewind = false;
    line.push({ text: "\n", style: {} });
    done.push(...line);
    line = [];
  };

  let i = 0;
  let plain = "";
  const flush = () => {
    if (plain) push(plain);
    plain = "";
  };
  while (i < input.length) {
    const ch = input[i];
    const code = ch.charCodeAt(0);
    if (ch === "\x1b") {
      flush();
      const kind = input[i + 1];
      if (kind === "[") {
        // CSI: parameters and intermediates, then a final byte in 0x40–0x7E.
        let j = i + 2;
        while (j < input.length && !/[\x40-\x7e]/.test(input[j])) j++;
        if (j >= input.length) break;
        if (input[j] === "m") style = applySgr(style, input.slice(i + 2, j));
        i = j + 1;
      } else if (kind === "]" || kind === "P" || kind === "_" || kind === "^") {
        // OSC and other strings run until BEL or ST (`ESC \`).
        let j = i + 2;
        while (j < input.length && input[j] !== "\x07" && !(input[j] === "\x1b" && input[j + 1] === "\\")) j++;
        if (j >= input.length) break;
        i = input[j] === "\x07" ? j + 1 : j + 2;
      } else if (kind === "(" || kind === ")" || kind === "#") {
        i += 3;
      } else {
        i += 2;
      }
    } else if (ch === "\n") {
      flush();
      endLine();
      i++;
    } else if (ch === "\r") {
      flush();
      if (input[i + 1] !== "\n") rewind = true;
      i++;
    } else if (ch === "\t" || code >= 0x20) {
      plain += ch;
      i++;
    } else {
      i++;
    }
  }
  flush();
  done.push(...line);

  // Merge runs that ended up adjacent with the same style.
  const merged: AnsiSpan[] = [];
  for (const span of done) {
    const last = merged[merged.length - 1];
    if (last && (span.text === "\n" || sameStyle(last.style, span.style))) last.text += span.text;
    else merged.push({ ...span });
  }
  return merged;
}

/** Inline CSS for a span's style; empty for plain text. */
export function spanCss(style: AnsiStyle): Record<string, string> {
  const css: Record<string, string> = {};
  let fg = style.fg;
  let bg = style.bg;
  if (style.inverse) {
    [fg, bg] = [bg ?? "var(--bg-term)", fg ?? "var(--text)"];
  }
  if (fg) css.color = fg;
  if (bg) css.backgroundColor = bg;
  if (style.bold) css.fontWeight = "600";
  if (style.dim) css.opacity = "0.6";
  if (style.italic) css.fontStyle = "italic";
  const lines = [style.underline && "underline", style.strike && "line-through"].filter(Boolean);
  if (lines.length) css.textDecoration = lines.join(" ");
  return css;
}
