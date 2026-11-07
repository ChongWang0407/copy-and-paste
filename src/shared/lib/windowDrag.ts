import type { MouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Left-press on empty title areas moves the window. */
export const startWindowDrag = (event: MouseEvent<HTMLElement>) => {
  if (event.button !== 0) return;
  const target = event.target as HTMLElement;
  if (target.closest('button, input, select, textarea, a, [role="button"]')) return;
  event.preventDefault();
  void getCurrentWindow().startDragging();
};
