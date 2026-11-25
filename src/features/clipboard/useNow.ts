import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { isTauriRuntime } from "../../shared/lib/tauriRuntime";

const TICK_MS = 30_000;
const listeners = new Set<(now: number) => void>();
let timer: ReturnType<typeof setInterval> | null = null;
let hidden = false;
let watching = false;

const tick = () => {
  const now = Date.now();
  listeners.forEach((listener) => listener(now));
};

const startTimer = () => {
  if (timer === null && !hidden && listeners.size > 0) timer = setInterval(tick, TICK_MS);
};

const stopTimer = () => {
  if (timer !== null) {
    clearInterval(timer);
    timer = null;
  }
};

/** A hidden window has no labels to move on: the clock stops, and catches up when shown. */
export const setClockVisible = (visible: boolean) => {
  if (hidden === !visible) return;
  hidden = !visible;
  if (hidden) {
    stopTimer();
  } else {
    tick();
    startTimer();
  }
};

export const isClockRunning = () => timer !== null;

const watchWindow = () => {
  if (watching) return;
  watching = true;
  if (typeof document !== "undefined") {
    document.addEventListener("visibilitychange", () => setClockVisible(document.visibilityState !== "hidden"));
  }
  if (isTauriRuntime()) {
    void listen("clipboard-hidden", () => setClockVisible(false));
    void listen("clipboard-shown", () => setClockVisible(true));
  }
};

/** Shared by every card; returns the unsubscribe. */
export const subscribeClock = (listener: (now: number) => void) => {
  listeners.add(listener);
  watchWindow();
  startTimer();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) stopTimer();
  };
};

/** One shared clock for every card: the time labels move on together. */
export const useNow = () => {
  const [now, setNow] = useState(Date.now);
  useEffect(() => subscribeClock(setNow), []);
  return now;
};
