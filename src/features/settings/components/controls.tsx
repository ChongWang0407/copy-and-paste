import { useCallback, useEffect, useRef, useState } from "react";
import type { KeyboardEvent, ReactNode } from "react";
import { createPortal } from "react-dom";
import { ChevronDown, ChevronRight } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { getHotkeyDisplayTokens } from "../../../shared/lib/hotkeyDisplay";
import { hotkeyKeyFromEvent } from "../../../shared/lib/hotkeyKey";
import type { Translate } from "../../../shared/types";

export const Hint = ({ text }: { text: string }) => {
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  const ref = useRef<HTMLSpanElement | null>(null);

  const show = () => {
    const box = ref.current?.getBoundingClientRect();
    if (!box) return;
    const width = 220;
    const left = Math.max(10, Math.min(box.left + box.width / 2 - width / 2, window.innerWidth - width - 10));
    setPosition({ left, top: box.bottom + 8 });
  };

  return (
    <>
      <span
        ref={ref}
        className="hint-dot"
        role="img"
        aria-label={text}
        tabIndex={0}
        onMouseEnter={show}
        onMouseLeave={() => setPosition(null)}
        onFocus={show}
        onBlur={() => setPosition(null)}
      >
        ?
      </span>
      {position &&
        createPortal(
          <div className="hint-tooltip glass-popover" style={position}>
            {text}
          </div>,
          document.body
        )}
    </>
  );
};

export const Row = ({
  label,
  hint,
  subtitle,
  children,
  child = false,
  column = false,
}: {
  label: string;
  hint?: string;
  subtitle?: string;
  children?: ReactNode;
  child?: boolean;
  column?: boolean;
}) => (
  <div className={`setting-row${child ? " is-child" : ""}${column ? " is-column" : ""}`}>
    <div className="setting-label">
      <span className="setting-label-line">
        <span className="setting-name">{label}</span>
        {hint && <Hint text={hint} />}
      </span>
      {subtitle && <span className="setting-subtitle">{subtitle}</span>}
    </div>
    {children}
  </div>
);

export const Toggle = ({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
}) => (
  <label className="switch">
    <input type="checkbox" aria-label={label} checked={checked} onChange={(event) => onChange(event.target.checked)} />
    <span className="switch-track">
      <span className="switch-thumb" />
    </span>
  </label>
);

export const Segmented = <T extends string>({
  value,
  options,
  onChange,
  disabled = false,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  disabled?: boolean;
}) => (
  <div className="segmented" role="radiogroup">
    {options.map((option) => (
      <button
        key={option.value}
        type="button"
        role="radio"
        aria-checked={value === option.value}
        disabled={disabled}
        className={`segment${value === option.value ? " is-active" : ""}`}
        onClick={() => onChange(option.value)}
      >
        {option.label}
      </button>
    ))}
  </div>
);

export const Section = ({
  title,
  open,
  onToggle,
  children,
}: {
  title: string;
  open: boolean;
  onToggle: () => void;
  children: ReactNode;
}) => (
  <section className={`settings-section${open ? " is-open" : ""}`}>
    <button type="button" className="settings-section-header" aria-expanded={open} onClick={onToggle}>
      <h3>{title}</h3>
      {open ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
    </button>
    {open && <div className="settings-section-body">{children}</div>}
  </section>
);

/** Records a shortcut: click, then press it. Esc cancels, Delete clears. */
export const HotkeyField = ({
  t,
  value,
  label,
  onRecorded,
}: {
  t: Translate;
  value: string;
  label: string;
  onRecorded: (hotkey: string) => Promise<void>;
}) => {
  const [recording, setRecording] = useState(false);
  const session = useRef<{ started: Promise<void>; finishing: boolean } | null>(null);

  const finish = useCallback(async (commit?: () => Promise<void>) => {
    const current = session.current;
    if (!current || current.finishing) return;
    current.finishing = true;
    try {
      await current.started;
      await commit?.();
    } catch (error) {
      console.error(error);
    } finally {
      await invoke("set_recording_mode", { enabled: false }).catch(console.error);
      hotkeyCaptureActive = false;
      session.current = null;
      setRecording(false);
    }
  }, []);

  useEffect(() => () => { void finish(); }, [finish]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!session.current) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat || session.current.finishing) return;
    if (event.key === "Escape") {
      void finish();
      return;
    }
    if (event.key === "Backspace" || event.key === "Delete") {
      void finish(() => onRecorded(""));
      return;
    }
    const key = hotkeyKeyFromEvent(event);
    if (!key) return;
    const modifiers: string[] = [];
    if (event.ctrlKey) modifiers.push("Ctrl");
    if (event.shiftKey) modifiers.push("Shift");
    if (event.altKey) modifiers.push("Alt");
    if (event.metaKey) modifiers.push("Command");
    void finish(() => onRecorded([...modifiers, key].join("+")));
  };

  const shown = getHotkeyDisplayTokens(value, { preferMacSymbols: true })
    .map((token) => token.label)
    .join("");

  return (
    <div
      className={`hotkey-field${recording ? " is-recording" : ""}`}
      role="button"
      tabIndex={0}
      aria-label={label}
      onClick={(event) => {
        if (hotkeyCaptureActive) return;
        hotkeyCaptureActive = true;
        const started = invoke<void>("set_recording_mode", { enabled: true });
        session.current = { started, finishing: false };
        void started.catch(() => finish());
        setRecording(true);
        event.currentTarget.focus();
      }}
      onKeyDown={onKeyDown}
      onBlur={() => void finish()}
    >
      {recording ? t("hotkey_waiting") : shown || t("hotkey_not_set")}
    </div>
  );
};

// A capture can still be finishing after Settings has been closed and reopened.
let hotkeyCaptureActive = false;
