import { Change, diff } from "@codemirror/merge";
import { expect, it } from "vitest";
import { diffIgnoringWhitespace } from "./whitespaceDiff";

const slices = (a: string, b: string) =>
  diffIgnoringWhitespace(a, b, diff, Change)!.map((c) => [a.slice(c.fromA, c.toA), b.slice(c.fromB, c.toB)]);

it("treats re-indented and re-spaced lines as unchanged", () => {
  const a = "function f() {\n  return 1;\n}\n";
  const b = "function f() {\n    return  1;\n}\n";
  expect(slices(a, b)).toEqual([]);
});

it("reports real edits as whole lines next to whitespace-only ones", () => {
  const a = "one\n  two\nthree\n";
  const b = "one\n\ttwo\nTHREE\nfour\n";
  expect(slices(a, b)).toEqual([["three\n", "THREE\nfour\n"]]);
});

it("falls back when only one side ends with a newline, and handles empty sides", () => {
  expect(diffIgnoringWhitespace("a\nb", "a\nb\n", diff, Change)).toBeNull();
  // Neither side ends with a newline: the added last line is reported, the re-indented one is not.
  expect(slices("a\nb", "a\n  b\nc")).toEqual([["", "c"]]);
  expect(slices("", "x\n")).toEqual([["", "x\n"]]);
});
