// The key of a shortcut, named by its physical position (`event.code`), not
// the character it types. On macOS, Option changes the character (Option+C
// types "ç", Option+F "ƒ"), and Shift turns digits into symbols, so
// `event.key` gave hotkeys the backend could not parse ("参数无效") and in-app
// matches that never fired. The names are ones the Rust parser accepts
// (global-hotkey `parse_key`).
const CODE_NAMES: Record<string, string> = {
  Space: "Space",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
};

const MODIFIER_CODE = /^(Shift|Control|Alt|Meta|OS)(Left|Right)?$|^(CapsLock|Fn|FnLock)$/;

/** Key name for a shortcut string, or null for a bare modifier or an unknown key. */
export const hotkeyKeyFromCode = (code: string): string | null => {
  if (!code || MODIFIER_CODE.test(code)) return null;
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) return digit[1];
  return CODE_NAMES[code] ?? code;
};

/** Key name for a keyboard event; falls back to `event.key` when there is no code. */
export const hotkeyKeyFromEvent = (event: { code: string; key: string }): string | null => {
  const fromCode = hotkeyKeyFromCode(event.code);
  if (fromCode) return fromCode;
  if (event.code) return null;
  const key = event.key.toUpperCase();
  return ["CONTROL", "SHIFT", "ALT", "META"].includes(key) ? null : key;
};

const MODIFIER_NAMES = new Set(["CTRL", "CONTROL", "ALT", "OPTION", "MENU", "WIN", "SUPER", "COMMAND", "CMD", "META"]);

/**
 * A global shortcut with no modifier but Shift on a key that types text
 * (Shift+S, Shift+1, plain C). Registered globally it swallows that
 * character in every app, so such shortcuts are refused.
 */
export const isTypingHotkey = (hotkey: string): boolean => {
  const parts = hotkey.split("+").map((p) => p.trim().toUpperCase()).filter(Boolean);
  if (parts.length === 0 || parts[0] === "MOUSEMIDDLE" || parts[0] === "MBUTTON") return false;
  if (parts.some((p) => MODIFIER_NAMES.has(p))) return false;
  const key = parts.filter((p) => p !== "SHIFT").pop();
  if (!key) return false;
  // Function keys, navigation and the like type nothing.
  return !/^(F([1-9]|1[0-9]|2[0-4])|ESC(APE)?|INSERT|DELETE|HOME|END|PAGEUP|PAGEDOWN|UP|DOWN|LEFT|RIGHT|ARROW(UP|DOWN|LEFT|RIGHT)|PRINTSCREEN|SCROLLLOCK|PAUSE|NUMLOCK)$/.test(key);
};
