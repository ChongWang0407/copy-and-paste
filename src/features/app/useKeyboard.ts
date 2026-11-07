import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ClipboardEntry } from "../../shared/types";
import { cycleTypeFilter, type TypeFilter } from "../../shared/lib/clipboardTypes";

interface KeyboardOptions {
  items: ClipboardEntry[];
  enabled: boolean;
  arrowKeys: boolean;
  selectedIndex: number | null;
  setSelectedIndex: (index: number | null) => void;
  typeFilter: TypeFilter | null;
  setTypeFilter: (filter: TypeFilter | null) => void;
  paste: (entry: ClipboardEntry) => void;
  remove: (entry: ClipboardEntry) => void;
  focusSearch: () => void;
  /** Esc: closes what's open. False when nothing was. */
  dismiss: () => boolean;
}

export const isField = (target: EventTarget | null) =>
  target instanceof HTMLElement &&
  (target.matches("input, textarea, select") || target.isContentEditable);

export const hasOpenDialog = () => document.querySelector("[aria-modal='true']") !== null;

export const useKeyboard = (options: KeyboardOptions) => {
  const ref = useRef(options);
  ref.current = options;

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const current = ref.current;
      if (event.defaultPrevented || event.isComposing || event.key === "Process") return;
      if (hasOpenDialog()) return;

      if (event.key === "Escape") {
        event.preventDefault();
        if (isField(event.target)) (event.target as HTMLElement).blur();
        // escape_window_cmd also unpins: Esc is the only way out of a pinned window.
        if (!current.dismiss()) invoke("escape_window_cmd").catch(console.error);
        return;
      }
      if (!current.enabled) return;

      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        current.focusSearch();
        return;
      }

      const inSearch = event.target instanceof HTMLElement && event.target.classList.contains("searchbox-input");
      const inOtherField = isField(event.target) && !inSearch;
      if (inOtherField) return;

      if (event.key === "Tab" && !event.metaKey && !event.ctrlKey && !event.altKey) {
        event.preventDefault();
        current.setTypeFilter(cycleTypeFilter(current.typeFilter, event.shiftKey ? -1 : 1));
        current.setSelectedIndex(null);
        return;
      }

      if (current.arrowKeys && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
        event.preventDefault();
        const last = current.items.length - 1;
        if (last < 0) return;
        const position = current.selectedIndex;
        const next =
          event.key === "ArrowDown"
            ? position === null ? 0 : Math.min(position + 1, last)
            : position === null ? 0 : Math.max(position - 1, 0);
        current.setSelectedIndex(next);
        return;
      }

      const selected = current.selectedIndex === null ? null : current.items[current.selectedIndex];
      if (!selected) return;

      if (event.key === "Enter") {
        event.preventDefault();
        event.stopPropagation();
        current.paste(selected);
        return;
      }

      const forwardDelete = event.key === "Delete";
      const commandDelete = event.metaKey && event.key === "Backspace";
      if ((forwardDelete || commandDelete) && !inSearch) {
        event.preventDefault();
        current.remove(selected);
      }
    };

    // Before the search box sees Return: with an item picked, it pastes that item.
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);
};
