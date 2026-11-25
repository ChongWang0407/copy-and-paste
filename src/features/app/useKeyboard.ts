import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ClipboardEntry, SidePanelKind } from "../../shared/types";
import { cycleTypeFilter, type TypeFilter } from "../../shared/lib/clipboardTypes";
import { resolveKey } from "./keyboardRules";

interface KeyboardOptions {
  items: ClipboardEntry[];
  /** The list's own keys apply (not while settings or an editing panel is open). */
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
  panel: SidePanelKind | null;
  /** Settings or setting the fixed position. */
  blocked: boolean;
  batchCount: number;
  canSelectAll: boolean;
  /** Cmd+A on the list: batch mode with everything shown ticked. */
  startBatchSelectAll: () => void;
  toggleSelectAll: () => void;
  requestBatchDelete: () => void;
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
      if (event.defaultPrevented) return;
      const inSearch = event.target instanceof HTMLElement && event.target.classList.contains("searchbox-input");
      const action = resolveKey(event, {
        dialogOpen: hasOpenDialog(),
        inSearch,
        inField: isField(event.target),
        panel: current.panel,
        blocked: current.blocked,
        listKeys: current.enabled,
        arrowKeys: current.arrowKeys,
        itemCount: current.items.length,
        selectedIndex: current.selectedIndex,
        batchCount: current.batchCount,
        canSelectAll: current.canSelectAll,
      });
      if (action.type === "ignore") return;
      event.preventDefault();
      const selected = current.selectedIndex === null ? undefined : current.items[current.selectedIndex];

      switch (action.type) {
        case "escape":
          if (isField(event.target)) (event.target as HTMLElement).blur();
          // escape_window_cmd also unpins: Esc is the only way out of a pinned window.
          if (!current.dismiss()) invoke("escape_window_cmd").catch(console.error);
          return;
        case "focusSearch":
          current.focusSearch();
          return;
        case "cycleType":
          current.setTypeFilter(cycleTypeFilter(current.typeFilter, action.step));
          current.setSelectedIndex(null);
          return;
        case "select":
          current.setSelectedIndex(action.index);
          return;
        case "paste":
          event.stopPropagation();
          if (selected) current.paste(selected);
          return;
        case "remove":
          if (selected) current.remove(selected);
          return;
        case "startBatchSelectAll":
          current.startBatchSelectAll();
          return;
        case "toggleSelectAll":
          current.toggleSelectAll();
          return;
        case "batchDelete":
          current.requestBatchDelete();
          return;
        case "consume":
          return;
      }
    };

    // Before the search box sees Return: with an item picked, it pastes that item.
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);
};
