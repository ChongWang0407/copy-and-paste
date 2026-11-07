// 个性化界面 → 主题色. The theme colour replaces every blue in the UI
// (buttons, highlights, active icons); 删除 keeps its red.
//
// The capsule is a separate window, so the choice is mirrored to
// localStorage, which both pages share and read before settings load.

export const ACCENT_SETTING_KEY = "app.accent_color";
const ACCENT_MIRROR_KEY = "copy_paste_accent";

export interface AccentPalette {
  /** Main colour, also the swatch. */
  base: string;
  /** Text and icons on a soft fill. */
  strong: string;
  /** Top and bottom of filled buttons. */
  light: string;
  mid: string;
}

export const ACCENTS = {
  blue: { base: "#0a84ff", strong: "#0066e0", light: "#2f97ff", mid: "#0a7bff" },
  green: { base: "#22b05a", strong: "#178a45", light: "#3cc46e", mid: "#22b05a" },
  orange: { base: "#ff8a00", strong: "#d96a00", light: "#ffa12e", mid: "#ff8a00" },
  red: { base: "#f0444e", strong: "#cc2a35", light: "#ff5f68", mid: "#f0444e" },
  purple: { base: "#8e5cf0", strong: "#6e3fd1", light: "#a277f5", mid: "#8e5cf0" },
  teal: { base: "#12a7b8", strong: "#0b8494", light: "#2fbccb", mid: "#12a7b8" },
} satisfies Record<string, AccentPalette>;

export type AccentName = keyof typeof ACCENTS;
export const ACCENT_NAMES = Object.keys(ACCENTS) as AccentName[];
export const DEFAULT_ACCENT: AccentName = "blue";

export const normalizeAccent = (value: string | null | undefined): AccentName =>
  value && Object.prototype.hasOwnProperty.call(ACCENTS, value) ?(value as AccentName) : DEFAULT_ACCENT;

const rgbOf = (hex: string) => {
  const n = parseInt(hex.slice(1), 16);
  return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`;
};

/** Sets the accent variables on the page (defaults live in the CSS). */
export const applyAccent = (name: AccentName, root: HTMLElement = document.documentElement) => {
  const p = ACCENTS[name];
  const vars: Record<string, string> = {
    "--accent": p.base,
    "--accent-strong": p.strong,
    "--accent-light": p.light,
    "--accent-mid": p.mid,
    "--accent-rgb": rgbOf(p.base),
    "--accent-strong-rgb": rgbOf(p.strong),
    "--accent-soft": `rgba(${rgbOf(p.base)}, 0.16)`,
  };
  for (const [key, value] of Object.entries(vars)) root.style.setProperty(key, value);
};

export const mirrorAccent = (name: AccentName) => {
  try {
    localStorage.setItem(ACCENT_MIRROR_KEY, name);
  } catch {
    // The capsule then stays on the default colour.
  }
};

export const mirroredAccent = (): AccentName => {
  try {
    return normalizeAccent(localStorage.getItem(ACCENT_MIRROR_KEY));
  } catch {
    return DEFAULT_ACCENT;
  }
};
