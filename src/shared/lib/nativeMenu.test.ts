import { afterEach, describe, expect, it, vi } from "vitest";

const state = vi.hoisted(() => ({ log: [] as string[], dismiss: [] as (() => void)[], menus: 0 }));

vi.mock("./focus", () => ({
  acquireBlurGuard: vi.fn(async () => {}),
  releaseBlurGuard: vi.fn(async () => {}),
}));

vi.mock("@tauri-apps/api/dpi", () => ({ LogicalPosition: class {} }));

vi.mock("@tauri-apps/api/menu", () => {
  const item = { new: async () => ({}) };
  return {
    MenuItem: item,
    CheckMenuItem: item,
    PredefinedMenuItem: item,
    Submenu: item,
    Menu: {
      new: async () => {
        const id = ++state.menus;
        state.log.push(`new ${id}`);
        return {
          popup: () => {
            state.log.push(`popup ${id}`);
            return new Promise<void>((resolve) => state.dismiss.push(resolve));
          },
          close: async () => {
            state.log.push(`close ${id}`);
          },
        };
      },
    },
  };
});

import { showNativeMenu } from "./nativeMenu";

const dismiss = () => state.dismiss.shift()?.();

describe("showNativeMenu", () => {
  afterEach(() => vi.useRealTimers());

  it("leaves menu resources alone while a menu is on screen", async () => {
    vi.useFakeTimers();

    const first = showNativeMenu([{ text: "a" }]);
    await vi.advanceTimersByTimeAsync(0);
    dismiss();
    await first;

    // Opened right after the first one, and left open for a while.
    const second = showNativeMenu([{ text: "b" }]);
    await vi.advanceTimersByTimeAsync(5000);
    expect(state.log).toEqual(["new 1", "popup 1", "close 1", "new 2", "popup 2"]);

    // A third one waits for the second to go.
    const third = showNativeMenu([{ text: "c" }]);
    await vi.advanceTimersByTimeAsync(5000);
    expect(state.log).toHaveLength(5);

    dismiss();
    await second;
    await vi.advanceTimersByTimeAsync(0);
    expect(state.log.slice(5)).toEqual(["close 2", "new 3", "popup 3"]);

    dismiss();
    await third;
  });
});
