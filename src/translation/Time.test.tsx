import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setLang } from "../i18n";
import { TranslationTime } from "./Time";

beforeEach(() => { setLang("en"); vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "performance"] }); });
afterEach(() => { cleanup(); vi.useRealTimers(); });

it("interpolates server elapsed time and freezes the authoritative completion duration", () => {
  const { rerender } = render(<TranslationTime timing={{ startedAt: 10000, durationMs: 400 }} serverTime={11500} running compact />);
  expect(screen.getByText("· 1.9 sec")).toBeTruthy();
  act(() => { vi.advanceTimersByTime(600); });
  expect(screen.getByText("· 2.5 sec")).toBeTruthy();
  rerender(<TranslationTime timing={{ startedAt: null, durationMs: 2100 }} serverTime={13000} />);
  expect(screen.getByText("Translation time: 2.1 sec")).toBeTruthy();
  act(() => { vi.advanceTimersByTime(3000); });
  expect(screen.getByText("Translation time: 2.1 sec")).toBeTruthy();
  expect(vi.getTimerCount()).toBe(0);
});

it("refreshes cumulative batch timing without counting the idle interval twice", () => {
  const { rerender } = render(<TranslationTime timing={{ startedAt: null, durationMs: 1200 }} serverTime={15000} running compact />);
  act(() => { vi.advanceTimersByTime(10000); });
  expect(screen.getByText("· 1.2 sec")).toBeTruthy();
  rerender(<TranslationTime timing={{ startedAt: 25000, durationMs: 1200 }} serverTime={25300} running compact />);
  expect(screen.getByText("· 1.5 sec")).toBeTruthy();
  act(() => { vi.advanceTimersByTime(400); });
  expect(screen.getByText("· 1.9 sec")).toBeTruthy();
});

it("hides unknown legacy timings but retains measured zero and restored results", () => {
  const { container, rerender } = render(<TranslationTime timing={{ durationMs: null }} />);
  expect(container.childElementCount).toBe(0);
  rerender(<TranslationTime timing={{ durationMs: 0 }} />);
  expect(screen.getByText("Translation time: 0.0 sec")).toBeTruthy();
  rerender(<TranslationTime timing={{ durationMs: 1834 }} />);
  expect(screen.getByText("Translation time: 1.8 sec")).toBeTruthy();
});
