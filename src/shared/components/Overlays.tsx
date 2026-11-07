import { AnimatePresence, motion } from "framer-motion";
import AppModal from "./AppModal";
import type { ConfirmDialogState, ToastItem, Translate } from "../types";

export const ToastContainer = ({ toasts }: { toasts: ToastItem[] }) => (
  <div className="toast-container">
    <AnimatePresence>
      {toasts.map((toast) => (
        <motion.div
          key={toast.id}
          initial={{ opacity: 0, y: -10, scale: 0.96 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, scale: 0.96, transition: { duration: 0.16 } }}
          transition={{ type: "spring", stiffness: 420, damping: 30 }}
          className="toast glass-popover"
        >
          {toast.msg}
        </motion.div>
      ))}
    </AnimatePresence>
  </div>
);

export const ConfirmDialog = ({
  state,
  confirmLabel,
  cancelLabel,
  onClose,
}: {
  state: ConfirmDialogState;
  confirmLabel: string;
  cancelLabel: string;
  onClose: () => void;
}) => (
  <AppModal open={state.show} onClose={onClose} panelClassName="modal-panel glass-popover" ariaLabel={state.title}>
    <h3 className="modal-title">{state.title}</h3>
    <p className="modal-message">{state.message}</p>
    <div className="modal-actions">
      <button type="button" className="glass-button" onClick={onClose}>
        {cancelLabel}
      </button>
      <button
        type="button"
        className={`glass-button ${state.danger ? "is-danger" : "is-primary"}`}
        onClick={state.onConfirm}
      >
        {confirmLabel}
      </button>
    </div>
  </AppModal>
);

export type { Translate };
