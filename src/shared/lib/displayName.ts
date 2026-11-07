/** What fits the title bar: 16 Latin characters, 8 Chinese. */
export const DISPLAY_NAME_MAX_WIDTH = 16;
export const DEFAULT_DISPLAY_NAME = "Copy & Paste";

const charWidth = (char: string) => (/[\u0000-\u00ff]/.test(char) ? 1 : 2);

export const displayNameWidth = (name: string): number =>
  [...name].reduce((width, char) => width + charWidth(char), 0);

/** Cuts a name to what fits, for a field that refuses more typing. */
export const clampDisplayName = (name: string): string => {
  let width = 0;
  let clamped = "";
  for (const char of name) {
    width += charWidth(char);
    if (width > DISPLAY_NAME_MAX_WIDTH) break;
    clamped += char;
  }
  return clamped;
};
