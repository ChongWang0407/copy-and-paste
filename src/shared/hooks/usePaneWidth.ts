import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type RefObject
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { isTauriRuntime } from "../lib/tauriRuntime";

interface UsePaneWidthOptions {
  /** Setting key the width is remembered under. */
  storageKey: string;
  /** Element whose content box the divider moves across. */
  containerRef: RefObject<HTMLElement | null>;
  /** The resizable (left) pane, measured when there is no stored width yet. */
  paneRef: RefObject<HTMLElement | null>;
  /** Narrowest the resizable pane may get, in px. */
  min: number;
  /** Space the other pane always keeps, in px. */
  minRest: number;
  max?: number;
  /** "ratio" stores a share of the container, so it survives window resizes. */
  unit?: "px" | "ratio";
}

const KEY_STEP_PX = 16;

// Plain numbers; the tag manager used to store {"width": n}.
const parseStored = (raw: string | undefined): number | null => {
  if (!raw) return null;
  const value = Number(raw);
  if (Number.isFinite(value) && value > 0) return value;
  try {
    const parsed = JSON.parse(raw) as { width?: unknown };
    return typeof parsed.width === "number" && parsed.width > 0 ? parsed.width : null;
  } catch {
    return null;
  }
};

const contentBox = (el: HTMLElement) => {
  const rect = el.getBoundingClientRect();
  const style = getComputedStyle(el);
  const left = rect.left + parseFloat(style.paddingLeft || "0");
  const width = rect.width - parseFloat(style.paddingLeft || "0") - parseFloat(style.paddingRight || "0");
  return { left, width };
};

/**
 * Drag-to-resize for a two-pane layout. Returns the stored width (null means
 * "use the layout's default"), whether a drag is in progress, and the props for
 * a PaneDivider. Double-click resets; arrow keys nudge.
 */
export const usePaneWidth = ({
  storageKey,
  containerRef,
  paneRef,
  min,
  minRest,
  max = Number.POSITIVE_INFINITY,
  unit = "px"
}: UsePaneWidthOptions) => {
  const [value, setValue] = useState<number | null>(null);
  const [resizing, setResizing] = useState(false);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let alive = true;
    invoke<Record<string, string>>("get_settings")
      .then((settings) => {
        const stored = parseStored(settings[storageKey]);
        if (!alive || stored == null) return;
        // Older builds allowed narrower sidebars; keep stored widths in range.
        setValue(unit === "ratio" ? Math.min(0.9, Math.max(0.1, stored)) : Math.min(max, Math.max(min, stored)));
      })
      .catch(() => { /* keep the default width */ });
    return () => {
      alive = false;
    };
  }, [storageKey, unit, min, max]);

  // The cursor and text selection must not flicker while the pointer
  // outruns the 8px divider.
  useEffect(() => {
    document.body.classList.toggle("is-pane-resizing", resizing);
    return () => document.body.classList.remove("is-pane-resizing");
  }, [resizing]);

  const persist = useCallback((next: number | null) => {
    if (!isTauriRuntime()) return;
    invoke("save_setting", {
      key: storageKey,
      value: next == null ? "" : String(Math.round(next * 1000) / 1000)
    }).catch(console.error);
  }, [storageKey]);

  const fromPx = useCallback((px: number, width: number) => {
    const upper = Math.max(min, Math.min(max, width - minRest));
    const clamped = Math.max(min, Math.min(px, upper));
    return unit === "ratio" ? clamped / width : clamped;
  }, [min, max, minRest, unit]);

  const valueAt = useCallback((clientX: number) => {
    const container = containerRef.current;
    if (!container) return null;
    const box = contentBox(container);
    return box.width > 0 ? fromPx(clientX - box.left, box.width) : null;
  }, [containerRef, fromPx]);

  const latestRef = useRef<number | null>(null);

  const onPointerDown = useCallback((event: PointerEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    latestRef.current = null;
    setResizing(true);
  }, []);

  const onPointerMove = useCallback((event: PointerEvent<HTMLElement>) => {
    if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
    const next = valueAt(event.clientX);
    if (next == null) return;
    latestRef.current = next;
    setValue(next);
  }, [valueAt]);

  const onPointerUp = useCallback((event: PointerEvent<HTMLElement>) => {
    if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
    event.currentTarget.releasePointerCapture(event.pointerId);
    setResizing(false);
    // A plain click (no movement) must not pin the default width.
    if (latestRef.current != null) persist(latestRef.current);
  }, [persist]);

  const onDoubleClick = useCallback(() => {
    setValue(null);
    persist(null);
  }, [persist]);

  const onKeyDown = useCallback((event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    const container = containerRef.current;
    const pane = paneRef.current;
    if (!container || !pane) return;
    event.preventDefault();
    const box = contentBox(container);
    const currentPx = pane.getBoundingClientRect().width;
    const next = fromPx(currentPx + (event.key === "ArrowRight" ? KEY_STEP_PX : -KEY_STEP_PX), box.width);
    setValue(next);
    persist(next);
  }, [containerRef, paneRef, fromPx, persist]);

  return {
    value,
    resizing,
    dividerProps: {
      onPointerDown,
      onPointerMove,
      onPointerUp,
      onPointerCancel: onPointerUp,
      onDoubleClick,
      onKeyDown
    }
  };
};
