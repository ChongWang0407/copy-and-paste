import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { activateForInput } from "../lib/focus";
import { isTauriRuntime } from "../lib/tauriRuntime";

const isEditableTarget = (target: EventTarget | null): target is HTMLElement => {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.matches("input, textarea, select") ||
    target.isContentEditable ||
    Boolean(target.closest("[contenteditable='true']"))
  );
};

/**
 * Activates the non-activating clipboard panel before the browser assigns
 * caret focus. A single capture listener replaces per-input focus commands,
 * which otherwise run too late and can bounce the caret or interrupt IME.
 */
export function useFocusCoordinator() {
  useEffect(() => {
    if (!isTauriRuntime()) return;

    let activationInFlight: Promise<void> | null = null;
    let pendingTarget: HTMLElement | null = null;
    let lastActivationAt = 0;

    const restoreEditableFocus = (target: HTMLElement | null) => {
      if (!target?.isConnected || document.activeElement === target) return;
      requestAnimationFrame(() => {
        if (target.isConnected) target.focus({ preventScroll: true });
      });
    };

    const activate = (target: HTMLElement) => {
      pendingTarget = target;
      if (activationInFlight) return;

      // A refocus caused by the activation itself must not start another IPC
      // round trip. This also keeps quick input-to-input tabbing stable.
      if (performance.now() - lastActivationAt < 250) return;

      activationInFlight = getCurrentWindow()
        .isFocused()
        .catch(() => false)
        .then((focused) => (focused ? undefined : activateForInput()))
        .then(() => {
          lastActivationAt = performance.now();
          const targetToRestore = pendingTarget;
          pendingTarget = null;
          restoreEditableFocus(targetToRestore);
        })
        .catch(() => {})
        .finally(() => {
          activationInFlight = null;
        });
    };

    const handlePointerDown = (event: PointerEvent) => {
      if (isEditableTarget(event.target)) activate(event.target);
    };

    const handleFocusIn = (event: FocusEvent) => {
      if (isEditableTarget(event.target)) activate(event.target);
    };

    document.addEventListener("pointerdown", handlePointerDown, true);
    document.addEventListener("focusin", handleFocusIn, true);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown, true);
      document.removeEventListener("focusin", handleFocusIn, true);
    };
  }, []);
}
