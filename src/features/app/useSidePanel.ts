import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { SidePanelKind } from "../../shared/types";
import { isTauriRuntime } from "../../shared/lib/tauriRuntime";

/** 批量粘贴 widens the list a little, for the tick circles. */
const BATCH_LIST_GROWTH = 40;
/** Wider than the list: a preview has to show more than the card did. */
const PREVIEW_WIDTH_RATIO = 1.5;

export interface SidePanelState {
  kind: SidePanelKind | null;
  /** Width of the list column while a panel is open, in px. */
  listWidth: number | null;
}

const CLOSED: SidePanelState = { kind: null, listWidth: null };
/** Fallback if no resize arrives: longer than the native shrink animation. */
const CLOSE_SETTLE_MAX_MS = 600;

/**
 * 常用, 编辑, 批量粘贴 and 预览 share the area the window grows into on the right.
 * Only one is open at a time. The native side (set_side_panel) keeps the
 * window on screen and restores it, and collapses it whenever it hides.
 */
export const useSidePanel = () => {
  const [panel, setPanel] = useState<SidePanelState>(CLOSED);
  const baseWidthRef = useRef<number | null>(null);
  /** Ends a close that is still waiting for the window to shrink. */
  const settleRef = useRef<(() => void) | null>(null);
  const closingWidthRef = useRef<number | null>(null);

  const open = useCallback(async (kind: SidePanelKind) => {
    // Reopened mid-shrink: the window's width right now is neither size.
    const shrinkingFrom = settleRef.current ? closingWidthRef.current : null;
    settleRef.current?.();
    const baseWidth = baseWidthRef.current ?? shrinkingFrom ?? window.innerWidth;
    baseWidthRef.current = baseWidth;
    const growth = kind === "batch" ? BATCH_LIST_GROWTH : 0;
    const extra = kind === "preview" ? Math.round(baseWidth * PREVIEW_WIDTH_RATIO) : baseWidth + growth;
    setPanel({ kind, listWidth: baseWidth + growth });
    if (isTauriRuntime()) {
      await invoke<number>("set_side_panel", { extra }).catch(console.error);
    }
  }, []);

  const close = useCallback(async () => {
    settleRef.current?.();
    const baseWidth = baseWidthRef.current;
    baseWidthRef.current = null;
    if (!isTauriRuntime() || baseWidth === null) {
      setPanel(CLOSED);
      if (isTauriRuntime()) await invoke("set_side_panel", { extra: null }).catch(console.error);
      return;
    }
    // The window shrinks back with an animation. Letting the list fill the
    // still-wide window first made it stretch and snap back (the flash), so
    // it keeps its width until the window is narrow again.
    setPanel({ kind: null, listWidth: baseWidth });
    closingWidthRef.current = baseWidth;
    const finish = () => {
      window.removeEventListener("resize", onResize);
      window.clearTimeout(timer);
      settleRef.current = null;
      setPanel((current) => (current.kind === null ? CLOSED : current));
    };
    const onResize = () => {
      if (window.innerWidth <= baseWidth + 1) finish();
    };
    const timer = window.setTimeout(finish, CLOSE_SETTLE_MAX_MS);
    window.addEventListener("resize", onResize);
    settleRef.current = finish;
    await invoke("set_side_panel", { extra: null }).catch(console.error);
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const subscription = listen("side-panel-collapsed", () => {
      settleRef.current?.();
      baseWidthRef.current = null;
      setPanel(CLOSED);
    });
    return () => void subscription.then((off) => off());
  }, []);

  return { panel, open, close };
};
