import { useEffect, useState } from "react";

const TICK_MS = 30_000;
const listeners = new Set<(now: number) => void>();
let timer: number | null = null;

/** One shared clock for every card: the time labels move on together. */
export const useNow = () => {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    listeners.add(setNow);
    if (timer === null) {
      timer = window.setInterval(() => {
        const tick = Date.now();
        listeners.forEach((listener) => listener(tick));
      }, TICK_MS);
    }
    return () => {
      listeners.delete(setNow);
      if (listeners.size === 0 && timer !== null) {
        window.clearInterval(timer);
        timer = null;
      }
    };
  }, []);
  return now;
};
