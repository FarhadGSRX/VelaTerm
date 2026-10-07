import { expect, it } from "vitest";
import { parseAnsi, spanCss } from "./ansiLog";

it("keeps plain text unchanged", () => {
  expect(parseAnsi("compiled\nall tests passed")).toEqual([{ text: "compiled\nall tests passed", style: {} }]);
});

it("turns SGR colours and bold into styles and drops the escape codes", () => {
  const spans = parseAnsi("\x1b[1;36m==> build\x1b[0m\ndone");
  expect(spans.map((s) => s.text).join("")).toBe("==> build\ndone");
  expect(spans[0]).toEqual({ text: "==> build\n", style: { bold: true, fg: "var(--cyan)" } });
  expect(spans[1]).toEqual({ text: "done", style: {} });
});

it("reads 256-colour and true-colour forms", () => {
  expect(parseAnsi("\x1b[38;5;196mx")[0].style.fg).toBe("rgb(255, 0, 0)");
  expect(parseAnsi("\x1b[38;2;10;20;30mx")[0].style.fg).toBe("rgb(10, 20, 30)");
  expect(parseAnsi("\x1b[48:2::1:2:3mx")[0].style.bg).toBe("rgb(1, 2, 3)");
});

it("keeps only the last version of a line redrawn with a carriage return", () => {
  const text = parseAnsi("progress 10%\rprogress 50%\r\x1b[Kprogress 100%\nnext\r\n").map((s) => s.text).join("");
  expect(text).toBe("progress 100%\nnext\n");
});

it("keeps a line that ends with a carriage return when nothing replaces it", () => {
  expect(parseAnsi("downloading\r").map((s) => s.text).join("")).toBe("downloading");
});

it("drops cursor movement, OSC strings and other control characters", () => {
  const text = parseAnsi("\x1b]0;title\x07a\x1b[2Ab\x1b(Bc\x07d\x1b[1").map((s) => s.text).join("");
  expect(text).toBe("abcd");
});

it("maps styles to inline CSS, swapping colours when inverted", () => {
  expect(spanCss({})).toEqual({});
  expect(spanCss({ fg: "var(--red)", inverse: true })).toEqual({ color: "var(--bg-term)", backgroundColor: "var(--red)" });
  expect(spanCss({ underline: true, strike: true, dim: true })).toEqual({
    opacity: "0.6",
    textDecoration: "underline line-through",
  });
});
