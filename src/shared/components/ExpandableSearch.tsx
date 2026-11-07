import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Search, X } from "lucide-react";

interface ExpandableSearchProps {
  value: string;
  onChange: (value: string) => void;
  /** Placeholder, tooltip and accessible name of the field. */
  label: string;
  /** Accessible name of the clear/close button. */
  clearLabel: string;
  /** Keep the field expanded instead of collapsing to the search icon. */
  alwaysOpen?: boolean;
  /** Runs before the field takes focus, e.g. to activate a non-focusable window. */
  prepareFocus?: () => Promise<unknown>;
  /** Enter outside an IME composition. */
  onSubmit?: (value: string) => void;
  className?: string;
  /** Rendered inside the control, e.g. an anchored results popover. */
  children?: ReactNode;
}

/**
 * Search field that rests as an icon button and expands on click. Escape or
 * the clear button empties it and collapses it again.
 */
const ExpandableSearch = ({
  value,
  onChange,
  label,
  clearLabel,
  alwaysOpen = false,
  prepareFocus,
  onSubmit,
  className,
  children
}: ExpandableSearchProps) => {
  const [isOpen, setIsOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const inputId = useId();
  const expanded = alwaysOpen || isOpen || value.length > 0;

  useEffect(() => {
    if (!isOpen) return;
    let disposed = false;
    const focus = () => {
      if (!disposed) inputRef.current?.focus({ preventScroll: true });
    };
    if (prepareFocus) void prepareFocus().catch(console.error).finally(focus);
    else focus();
    return () => { disposed = true; };
  }, [isOpen, prepareFocus]);

  const close = () => {
    onChange("");
    setIsOpen(false);
    if (alwaysOpen) inputRef.current?.focus({ preventScroll: true });
    else triggerRef.current?.focus({ preventScroll: true });
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
      return;
    }
    if (e.nativeEvent.isComposing) return;
    if (e.key === "Enter" && value.trim() && onSubmit) onSubmit(value);
  };

  return (
    <div
      className={`expandable-search${expanded ? " is-open" : ""}${className ? ` ${className}` : ""}`}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null) && !value.trim()) {
          setIsOpen(false);
        }
      }}
    >
      <button
        ref={triggerRef}
        type="button"
        className="expandable-search-trigger"
        title={label}
        aria-label={label}
        aria-expanded={expanded}
        aria-controls={inputId}
        onClick={() => {
          if (isOpen) inputRef.current?.focus();
          else setIsOpen(true);
        }}
      >
        <Search size={14} aria-hidden="true" />
      </button>
      <input
        id={inputId}
        ref={inputRef}
        className="expandable-search-input"
        placeholder={label}
        aria-label={label}
        tabIndex={expanded ? 0 : -1}
        aria-hidden={!expanded}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={handleKeyDown}
      />
      <button
        type="button"
        className="expandable-search-clear"
        title={clearLabel}
        aria-label={clearLabel}
        tabIndex={expanded ? 0 : -1}
        aria-hidden={!expanded}
        onClick={close}
      >
        <X size={12} aria-hidden="true" />
      </button>
      {children}
    </div>
  );
};

export default ExpandableSearch;
