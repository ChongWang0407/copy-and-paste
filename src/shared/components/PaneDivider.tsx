import type { HTMLAttributes } from "react";

interface PaneDividerProps extends HTMLAttributes<HTMLDivElement> {
  /** True while dragging; keeps the handle visible. */
  active: boolean;
  /** Accessible name and tooltip, e.g. "Drag to resize, double-click to reset". */
  label: string;
  /** Sits inside the right pane's edge instead of between two flex items. */
  overlay?: boolean;
}

/**
 * Draggable boundary between two panes. The line and handle only show on
 * hover, focus or while dragging; the pane border is the resting divider.
 */
const PaneDivider = ({ active, label, overlay = false, className, ...rest }: PaneDividerProps) => (
  <div
    role="separator"
    aria-orientation="vertical"
    aria-label={label}
    title={label}
    tabIndex={0}
    className={`pane-divider${overlay ? " is-overlay" : ""}${active ? " active" : ""}${className ? ` ${className}` : ""}`}
    {...rest}
  >
    <span className="pane-divider-handle" aria-hidden="true" />
  </div>
);

export default PaneDivider;
