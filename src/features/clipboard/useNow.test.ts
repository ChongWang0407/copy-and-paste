import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { isClockRunning, setClockVisible, subscribeClock } from "./useNow";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

test("the card clock stops while the window is hidden and catches up when shown", () => {
  const ticks: number[] = [];
  const unsubscribe = subscribeClock((now) => ticks.push(now));
  expect(isClockRunning()).toBe(true);

  vi.advanceTimersByTime(30_000);
  expect(ticks).toHaveLength(1);

  setClockVisible(false);
  expect(isClockRunning()).toBe(false);
  vi.advanceTimersByTime(120_000);
  expect(ticks).toHaveLength(1);

  // Shown again: the labels update at once, then every 30 s as before.
  setClockVisible(true);
  expect(ticks).toHaveLength(2);
  vi.advanceTimersByTime(30_000);
  expect(ticks).toHaveLength(3);

  unsubscribe();
  expect(isClockRunning()).toBe(false);
});

test("no timer runs without a card", () => {
  setClockVisible(true);
  expect(isClockRunning()).toBe(false);
});
