import type { SidePanelKind } from "../../shared/types";

export interface KeyInput {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  isComposing: boolean;
}

export interface KeyContext {
  dialogOpen: boolean;
  /** Focus is in the search box. */
  inSearch: boolean;
  /** Focus is in any text field, the search box included. */
  inField: boolean;
  panel: SidePanelKind | null;
  /** Settings, or setting the fixed position: the list is not in use. */
  blocked: boolean;
  /** The list's own keys (arrows, Return, Delete, Tab, Cmd+F) apply. */
  listKeys: boolean;
  arrowKeys: boolean;
  itemCount: number;
  selectedIndex: number | null;
  batchCount: number;
  /** The list shows the finished result of the current search and tab. */
  canSelectAll: boolean;
}

export type KeyAction =
  /** Not ours: the key does what it normally does. */
  | { type: "ignore" }
  /** Swallowed, nothing else happens. */
  | { type: "consume" }
  | { type: "escape" }
  | { type: "focusSearch" }
  | { type: "cycleType"; step: 1 | -1 }
  | { type: "select"; index: number }
  | { type: "paste" }
  | { type: "remove" }
  | { type: "startBatchSelectAll" }
  | { type: "toggleSelectAll" }
  | { type: "batchDelete" };

const IGNORE: KeyAction = { type: "ignore" };
const CONSUME: KeyAction = { type: "consume" };

/** Cmd+A exactly: Ctrl+A moves to the line start on macOS and stays untouched. */
const isCommandA = (input: KeyInput) =>
  input.metaKey && !input.ctrlKey && !input.altKey && !input.shiftKey && input.key.toLowerCase() === "a";

const isFind = (input: KeyInput) => (input.metaKey || input.ctrlKey) && input.key.toLowerCase() === "f";

const isDelete = (input: KeyInput) => input.key === "Delete" || (input.metaKey && input.key === "Backspace");

/** What a key press does in the main window. */
export const resolveKey = (input: KeyInput, ctx: KeyContext): KeyAction => {
  if (input.isComposing || input.key === "Process" || ctx.dialogOpen) return IGNORE;
  if (input.key === "Escape") return { type: "escape" };

  if (isCommandA(input)) {
    // A text field keeps its own select-all.
    if (ctx.inField || ctx.blocked) return IGNORE;
    const ready = ctx.canSelectAll && ctx.itemCount > 0;
    if (ctx.panel === "batch") return ready ? { type: "toggleSelectAll" } : CONSUME;
    // Editing, translating, a preview or 常用: Cmd+A must not swap the panel out.
    if (ctx.panel !== null) return IGNORE;
    return ready ? { type: "startBatchSelectAll" } : CONSUME;
  }

  // Batch mode: no single-item paste or delete, so a stray Return or Delete
  // can't act on one entry.
  if (ctx.panel === "batch") {
    if (ctx.blocked) return IGNORE;
    if (isFind(input)) return { type: "focusSearch" };
    if (ctx.inField) return IGNORE;
    if (isDelete(input)) return ctx.batchCount > 0 ? { type: "batchDelete" } : IGNORE;
    return IGNORE;
  }

  if (!ctx.listKeys) return IGNORE;
  if (isFind(input)) return { type: "focusSearch" };
  if (ctx.inField && !ctx.inSearch) return IGNORE;

  if (input.key === "Tab" && !input.metaKey && !input.ctrlKey && !input.altKey) {
    return { type: "cycleType", step: input.shiftKey ? -1 : 1 };
  }

  if (ctx.arrowKeys && (input.key === "ArrowDown" || input.key === "ArrowUp")) {
    const last = ctx.itemCount - 1;
    if (last < 0) return CONSUME;
    const position = ctx.selectedIndex;
    const index =
      input.key === "ArrowDown"
        ? position === null ? 0 : Math.min(position + 1, last)
        : position === null ? 0 : Math.max(position - 1, 0);
    return { type: "select", index };
  }

  const selected = ctx.selectedIndex !== null && ctx.selectedIndex < ctx.itemCount;
  if (!selected) return IGNORE;
  if (input.key === "Enter") return { type: "paste" };
  if (isDelete(input) && !ctx.inSearch) return { type: "remove" };
  return IGNORE;
};
