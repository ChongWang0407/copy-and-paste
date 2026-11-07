import { useCallback, useEffect, useRef, useState } from "react";
import type { ConfirmDialogState, ToastItem, Translate } from "../../shared/types";
import { formatError } from "../../shared/lib/errorMessage";

const CLOSED: ConfirmDialogState = { show: false, title: "", message: "", onConfirm: () => {} };

let nextToastId = 1;
/** Every toast stays half as long as its caller asks: they lingered. */
const TOAST_DURATION_SCALE = 0.5;

export const useOverlays = (t: Translate) => {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [confirmDialog, setConfirmDialog] = useState<ConfirmDialogState>(CLOSED);
  const tRef = useRef(t);
  tRef.current = t;

  // Every toast goes through formatError, so raw backend errors are translated,
  // stripped and shortened here.
  const pushToast = useCallback((message: string, duration = 2400) => {
    const id = nextToastId++;
    setToasts((prev) => [...prev, { id, msg: formatError(message, tRef.current) }]);
    window.setTimeout(() => {
      setToasts((prev) => prev.filter((toast) => toast.id !== id));
    }, duration * TOAST_DURATION_SCALE);
  }, []);

  // Any click dismisses what is showing. It runs on press, before the click
  // itself, so a toast that click raises (pin, a refused close) still shows.
  const hasToasts = toasts.length > 0;
  useEffect(() => {
    if (!hasToasts) return;
    const dismiss = () => setToasts([]);
    window.addEventListener("pointerdown", dismiss, true);
    return () => window.removeEventListener("pointerdown", dismiss, true);
  }, [hasToasts]);

  const openConfirm = useCallback((options: Omit<ConfirmDialogState, "show">) => {
    setConfirmDialog({ ...options, show: true });
  }, []);

  const closeConfirm = useCallback(() => {
    setConfirmDialog((prev) => ({ ...prev, show: false }));
  }, []);

  return { toasts, pushToast, confirmDialog, openConfirm, closeConfirm };
};
