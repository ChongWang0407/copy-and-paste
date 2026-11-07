/**
 * Block Chromium / WebView browser accelerators that collide with desktop-app UX
 * or OS / third-party global hotkeys while this app is focused.
 *
 * Complements Windows WebView2 `AreBrowserAcceleratorKeysEnabled=false`.
 * Editing shortcuts (Ctrl/Cmd+C/V/X/A/Z) are intentionally left alone.
 */

export type BrowserAcceleratorBlockerOptions = {
  /** Called for Ctrl/Cmd+F (and F3) after the browser find UI is suppressed. */
  onFind?: () => void;
  /** When true, keep F5 / Ctrl+R / F12 available for local development. */
  allowDevReload?: boolean;
};

const isMod = (event: KeyboardEvent) => event.ctrlKey || event.metaKey;

const isFindShortcut = (event: KeyboardEvent) => {
  const key = event.key.toLowerCase();
  if (key === "f3") return true;
  if (isMod(event) && !event.altKey && !event.shiftKey && key === "f") return true;
  if (isMod(event) && !event.altKey && key === "g") return true; // find next / prev
  return false;
};

/** Browser-only accelerators that should never win inside this app. */
export const isBrowserAcceleratorShortcut = (
  event: KeyboardEvent,
  options?: Pick<BrowserAcceleratorBlockerOptions, "allowDevReload">
): boolean => {
  const key = event.key.toLowerCase();
  const mod = isMod(event);
  const allowDev = options?.allowDevReload === true;

  // F1 → browser Help (steals focus from other tools while this app is focused)
  if (key === "f1" && !mod && !event.altKey) return true;

  // Find-in-page
  if (isFindShortcut(event)) return true;

  // Print / save / view-source
  if (mod && !event.altKey && !event.shiftKey && (key === "p" || key === "s" || key === "u")) {
    return true;
  }

  // Reload
  if (!allowDev) {
    if (key === "f5" && !mod && !event.altKey) return true;
    if (mod && !event.altKey && !event.shiftKey && key === "r") return true;
  }

  // Zoom
  if (mod && !event.altKey && (key === "=" || key === "+" || key === "-" || key === "_" || key === "0")) {
    return true;
  }
  if (mod && !event.altKey && (event.key === "Add" || event.key === "Subtract")) return true;

  // DevTools (keep in Vite/dev builds)
  if (!allowDev) {
    if (key === "f12" && !mod && !event.altKey) return true;
    if (mod && event.shiftKey && (key === "i" || key === "j" || key === "c")) return true;
  }

  return false;
};

export const installBrowserAcceleratorBlocker = (
  options: BrowserAcceleratorBlockerOptions = {}
): (() => void) => {
  const allowDevReload =
    options.allowDevReload ??
    (typeof import.meta !== "undefined" && Boolean((import.meta as ImportMeta & { env?: { DEV?: boolean } }).env?.DEV));

  const onKeyDown = (event: KeyboardEvent) => {
    if (!isBrowserAcceleratorShortcut(event, { allowDevReload })) return;

    // Find shortcuts: suppress browser UI but allow app listeners (e.g. focus the search box).
    if (isFindShortcut(event)) {
      options.onFind?.();
      event.preventDefault();
      return;
    }

    event.preventDefault();
    event.stopPropagation();
  };

  window.addEventListener("keydown", onKeyDown, true);
  return () => window.removeEventListener("keydown", onKeyDown, true);
};
