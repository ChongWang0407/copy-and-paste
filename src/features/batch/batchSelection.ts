import type { ClipboardEntry, Translate } from "../../shared/types";
import { entryKey, isEntry, type EntryKey } from "../../shared/lib/clipboardTypes";
export { entryKey, isEntry, type EntryKey } from "../../shared/lib/clipboardTypes";

/** Where the entry is in the list now, or null once it is gone. */
export const indexOfEntry = (items: ClipboardEntry[], key: EntryKey | null): number | null => {
  if (!key) return null;
  const index = items.findIndex((item) => isEntry(item, key));
  return index < 0 ? null : index;
};

const idsOf = (items: ClipboardEntry[]) => new Set(items.map((item) => item.id));

/** Every entry the list shows is ticked (and it shows any). */
export const allVisibleSelected = (selected: ClipboardEntry[], visible: ClipboardEntry[]) => {
  if (visible.length === 0) return false;
  const ids = idsOf(selected);
  return visible.every((item) => ids.has(item.id));
};

/** 全选: the shown entries not ticked yet, added after the others in list order. */
export const selectAllVisible = (selected: ClipboardEntry[], visible: ClipboardEntry[]) => {
  const ids = idsOf(selected);
  const added = visible.filter((item) => !ids.has(item.id));
  return added.length ? [...selected, ...added] : selected;
};

/** 取消全选: only the shown entries; ticks made under another search or tab stay. */
export const deselectAllVisible = (selected: ClipboardEntry[], visible: ClipboardEntry[]) => {
  const ids = idsOf(visible);
  return selected.filter((item) => !ids.has(item.id));
};

export const toggleSelectAllVisible = (selected: ClipboardEntry[], visible: ClipboardEntry[]) =>
  allVisibleSelected(selected, visible) ? deselectAllVisible(selected, visible) : selectAllVisible(selected, visible);

export interface SelectionCounts {
  total: number;
  pinned: number;
  /** Ticked under another search or tab, not in the list shown now. */
  outsideView: number;
}

export const selectionCounts = (selected: ClipboardEntry[], visible: ClipboardEntry[]): SelectionCounts => {
  const ids = idsOf(visible);
  return {
    total: selected.length,
    pinned: selected.filter((item) => item.is_pinned).length,
    outsideView: selected.filter((item) => !ids.has(item.id)).length,
  };
};

/** 「（已加载）」: everything shown is ticked, but more of the list is not loaded yet. */
export const showsLoadedSuffix = (hasMore: boolean, allSelected: boolean) => hasMore && allSelected;

export const selectionCountText = (t: Translate, count: number, hasMore: boolean, allSelected: boolean) =>
  t(showsLoadedSuffix(hasMore, allSelected) ? "batch_selected_loaded" : "batch_selected_count").replace(
    "{count}",
    String(count)
  );

export const batchDeleteMessage = (t: Translate, counts: SelectionCounts) => {
  const lines = [
    (counts.pinned > 0 ? t("batch_delete_confirm_pinned") : t("batch_delete_confirm"))
      .replace("{count}", String(counts.total))
      .replace("{pinned}", String(counts.pinned)),
  ];
  if (counts.outsideView > 0) lines.push(t("batch_delete_outside").replace("{count}", String(counts.outsideView)));
  lines.push(t("batch_delete_irreversible"));
  return lines.join("\n");
};

const shownDifferently = (a: ClipboardEntry, b: ClipboardEntry) =>
  a.is_pinned !== b.is_pinned || a.preview !== b.preview || a.content !== b.content;

/**
 * The ticked entries as the list has them now (pin state, content), so the
 * confirmation counts what will be deleted. One whose ID now names another
 * entry is dropped. Unchanged: the same array (a refresh must not reset the
 * panel's drag order).
 */
export const refreshSelection = (selected: ClipboardEntry[], list: ClipboardEntry[]) => {
  if (selected.length === 0) return selected;
  const byId = new Map(list.map((item) => [item.id, item]));
  let changed = false;
  const next: ClipboardEntry[] = [];
  for (const item of selected) {
    const current = byId.get(item.id);
    if (!current) {
      next.push(item);
    } else if (!isEntry(current, entryKey(item))) {
      changed = true;
    } else if (shownDifferently(current, item)) {
      changed = true;
      next.push(current);
    } else {
      next.push(item);
    }
  }
  return changed ? next : selected;
};

/** What `delete_clipboard_entries` and `get_clipboard_entry_states` exchange. */
export interface EntryState {
  id: number;
  createdAt: number;
  isPinned: boolean;
}

export const deletionRequest = (selected: ClipboardEntry[]): EntryState[] =>
  selected.map((item) => ({ id: item.id, createdAt: item.created_at ?? 0, isPinned: item.is_pinned }));

/** After `selection_changed`: drops entries that are gone and takes the current pin state. */
export const applyEntryStates = (selected: ClipboardEntry[], states: EntryState[]) => {
  const byId = new Map(states.map((state) => [state.id, state]));
  return selected.flatMap((item) => {
    const state = byId.get(item.id);
    if (!state || state.createdAt !== (item.created_at ?? 0)) return [];
    return state.isPinned === item.is_pinned ? [item] : [{ ...item, is_pinned: state.isPinned }];
  });
};
