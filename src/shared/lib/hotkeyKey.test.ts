import { describe, expect, test } from "vitest";
import { hotkeyKeyFromEvent, isTypingHotkey } from "./hotkeyKey";

const ev = (code: string, key: string, mods: Partial<KeyboardEvent> = {}) =>
  ({ code, key, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...mods }) as KeyboardEvent;

describe("hotkeyKeyFromEvent", () => {
  test("uses the physical key, not the character Option or Shift types", () => {
    expect(hotkeyKeyFromEvent(ev("KeyC", "ç", { altKey: true }))).toBe("C");
    expect(hotkeyKeyFromEvent(ev("Digit1", "!", { shiftKey: true }))).toBe("1");
    expect(hotkeyKeyFromEvent(ev("Space", " "))).toBe("Space");
    expect(hotkeyKeyFromEvent(ev("ArrowUp", "ArrowUp"))).toBe("Up");
    expect(hotkeyKeyFromEvent(ev("Comma", "≤", { altKey: true }))).toBe("Comma");
    expect(hotkeyKeyFromEvent(ev("F5", "F5"))).toBe("F5");
  });

  test("ignores bare modifiers", () => {
    for (const code of ["AltLeft", "ShiftRight", "MetaLeft", "ControlLeft"]) {
      expect(hotkeyKeyFromEvent(ev(code, "Alt"))).toBeNull();
    }
  });
});

describe("isTypingHotkey", () => {
  test("refuses shortcuts that would swallow typed characters", () => {
    for (const h of ["Shift+S", "Shift+1", "C", "Space", "Shift+Comma"]) expect(isTypingHotkey(h)).toBe(true);
  });
  test("allows real modifiers, function keys and clearing", () => {
    for (const h of ["Alt+C", "Command+Shift+V", "Ctrl+1", "F5", "Shift+F5", "MouseMiddle", ""]) {
      expect(isTypingHotkey(h)).toBe(false);
    }
  });
});
