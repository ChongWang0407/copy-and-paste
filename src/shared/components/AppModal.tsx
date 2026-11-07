import { useEffect, useId, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { acquireBlurGuard, releaseBlurGuard } from "../lib/focus";

export interface AppModalProps {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
  theme?: string;
  panelClassName?: string;
  overlayClassName?: string;
  closeOnOverlayClick?: boolean;
  closeOnEscape?: boolean;
  ariaLabel?: string;
}

const FOCUSABLE_SELECTOR = [
  "button:not([disabled])",
  "[href]",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[tabindex]:not([tabindex='-1'])"
].join(",");

const AppModal = ({
  open,
  onClose,
  children,
  theme,
  panelClassName = "modal-panel",
  overlayClassName = "",
  closeOnOverlayClick = true,
  closeOnEscape = closeOnOverlayClick,
  ariaLabel = "Dialog"
}: AppModalProps) => {
  const modalId = useId();
  const panelRef = useRef<HTMLDivElement | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!open) return;
    const owner = `app-modal:${modalId}`;
    const acquired = acquireBlurGuard(owner).catch(() => {});
    return () => {
      void acquired.finally(() => releaseBlurGuard(owner).catch(() => {}));
    };
  }, [modalId, open]);

  useEffect(() => {
    if (!open) return;

    const previouslyFocused = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;
    const focusInitialControl = requestAnimationFrame(() => {
      const panel = panelRef.current;
      if (!panel) return;
      const autofocusTarget = panel.querySelector<HTMLElement>("[autofocus]");
      const firstFocusable = panel.querySelector<HTMLElement>(FOCUSABLE_SELECTOR);
      (autofocusTarget || firstFocusable || panel).focus({ preventScroll: true });
    });

    const handleKeyDown = (event: KeyboardEvent) => {
      const panel = panelRef.current;
      if (!panel) return;

      if (event.key === "Escape" && closeOnEscape) {
        event.preventDefault();
        event.stopPropagation();
        onCloseRef.current();
        return;
      }

      if (event.key !== "Tab") return;
      const focusable = Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR))
        .filter((element) => element.offsetParent !== null);
      if (focusable.length === 0) {
        event.preventDefault();
        panel.focus({ preventScroll: true });
        return;
      }

      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", handleKeyDown, true);
    return () => {
      cancelAnimationFrame(focusInitialControl);
      document.removeEventListener("keydown", handleKeyDown, true);
      if (previouslyFocused?.isConnected) {
        previouslyFocused.focus({ preventScroll: true });
      }
    };
  }, [closeOnEscape, open]);

  if (!open || typeof document === "undefined") return null;

  const themeClass = theme ? `theme-${theme}` : "";
  const panelClasses = [panelClassName, themeClass].filter(Boolean).join(" ");
  const overlayClasses = ["modal-overlay", overlayClassName].filter(Boolean).join(" ");

  return createPortal(
    <div
      className={overlayClasses}
      onClick={closeOnOverlayClick ? onClose : undefined}
    >
      <div
        ref={panelRef}
        className={panelClasses}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={ariaLabel}
        tabIndex={-1}
      >
        {children}
      </div>
    </div>,
    document.body
  );
};

export default AppModal;
