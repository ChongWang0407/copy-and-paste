import { forwardRef, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties, FormEvent } from "react";
import { ChevronLeft, Search, X } from "lucide-react";
import type { Translate } from "../../shared/types";

const LETTERS = "Search".split("");
/** Where the letters rest as a placeholder, from the box's left edge. */
const LETTERS_REST_X = 38;
/** Width of the blue button, and of the letters inside it. */
const BUTTON_WIDTH = 66;
const BUTTON_INSET = 3;
const LETTERS_WIDTH = 44;
/** Extra button width for the ‹ in front of the letters. */
const ARROW_ROOM = 12;

export interface SearchBoxHandle {
  focus: () => void;
}

interface SearchBoxProps {
  t: Translate;
  value: string;
  onChange: (value: string) => void;
  /** Search button or Return. */
  onSubmit: (value: string) => void;
  /** The search the list is showing now. */
  appliedValue: string;
}

/**
 * At rest: a magnifier and a box reading "Search". On focus the box shortens,
 * the six letters slide right one after another, and a blue button grows to
 * catch them.
 */
const SearchBox = forwardRef<SearchBoxHandle, SearchBoxProps>(({ t, value, onChange, onSubmit, appliedValue }, ref) => {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const formRef = useRef<HTMLFormElement | null>(null);
  const [focused, setFocused] = useState(false);
  const [boxWidth, setBoxWidth] = useState(0);
  const composingRef = useRef(false);
  const active = focused || value.length > 0;

  useImperativeHandle(ref, () => ({ focus: () => inputRef.current?.focus() }), []);

  useLayoutEffect(() => {
    const form = formRef.current;
    if (!form) return;
    const measure = () => setBoxWidth(form.clientWidth);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(form);
    return () => observer.disconnect();
  }, []);

  // Emptying the box brings the whole list back at once.
  const clearedRef = useRef(false);
  useEffect(() => {
    if (value === "" && clearedRef.current) onSubmit("");
    clearedRef.current = value !== "";
  }, [onSubmit, value]);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!composingRef.current) onSubmit(value);
  };

  // Nothing new to search for: the button gets a ‹ and takes the box back to rest.
  const canCollapse = active && value.trim() === appliedValue;
  const collapse = () => {
    onChange("");
    inputRef.current?.blur();
  };

  const buttonWidth = BUTTON_WIDTH + (canCollapse ? ARROW_ROOM : 0);
  const buttonLeft = boxWidth - buttonWidth - BUTTON_INSET;
  const lettersLeft = buttonLeft + (canCollapse ? ARROW_ROOM : 0) + (BUTTON_WIDTH - LETTERS_WIDTH) / 2;
  const shift = Math.max(0, lettersLeft - LETTERS_REST_X);

  return (
    <form
      ref={formRef}
      className={`searchbox${active ? " is-active" : ""}${canCollapse ? " can-collapse" : ""}`}
      style={{ "--search-shift": `${shift}px`, "--search-button-width": `${buttonWidth}px` } as CSSProperties}
      onSubmit={submit}
    >
      <span className="searchbox-icon" aria-hidden="true">
        <Search size={15} />
      </span>
      <input
        ref={inputRef}
        className="searchbox-input"
        type="text"
        aria-label="Search"
        autoComplete="off"
        autoCorrect="off"
        spellCheck={false}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onCompositionStart={() => {
          composingRef.current = true;
        }}
        onCompositionEnd={() => {
          composingRef.current = false;
        }}
      />
      {value && (
        <button
          type="button"
          className="searchbox-clear"
          title={t("search_clear")}
          aria-label={t("search_clear")}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => {
            onChange("");
            inputRef.current?.focus();
          }}
        >
          <X size={12} />
        </button>
      )}
      <button
        type={canCollapse ? "button" : "submit"}
        className="searchbox-button"
        tabIndex={active ? 0 : -1}
        aria-label={canCollapse ? t("search_close") : "Search"}
        title={canCollapse ? t("search_close") : undefined}
        // Keeps the input focused, so the box doesn't start closing mid-click.
        onMouseDown={(event) => event.preventDefault()}
        onClick={canCollapse ? collapse : undefined}
      >
        <ChevronLeft className="searchbox-arrow" size={13} strokeWidth={2.6} aria-hidden="true" />
      </button>
      <span className="searchbox-letters" aria-hidden="true">
        {LETTERS.map((letter, index) => (
          <span
            key={index}
            style={{ transitionDelay: `${(active ? LETTERS.length - 1 - index : index) * 45}ms` }}
          >
            {letter}
          </span>
        ))}
      </span>
    </form>
  );
});

SearchBox.displayName = "SearchBox";

export default SearchBox;
