import type { ClipboardEntry } from "../../shared/types";
import { entryMatchesType, type TypeFilter } from "../../shared/lib/clipboardTypes";

/*
 * The loaded part of the history, always in display order and holding only
 * rows of the current tab. Its length is therefore the backend offset of the
 * next page: every insert and removal below keeps that true.
 */

const activityAt = (item: ClipboardEntry) => item.sort_at || item.timestamp;

/** The backend's default (manual) order: pins by their order, then newest activity. */
export const compareEntries = (a: ClipboardEntry, b: ClipboardEntry) => {
  if (a.is_pinned !== b.is_pinned) return a.is_pinned ? -1 : 1;
  if (a.is_pinned) {
    const byOrder = (b.pinned_order || 0) - (a.pinned_order || 0);
    if (byOrder) return byOrder;
  }
  return activityAt(b) - activityAt(a) || b.id - a.id;
};

/**
 * A new or changed entry, put where the backend would list it. One that sorts
 * after every loaded row while more pages exist belongs to a later page: it
 * is left for that page, or the next page's offset would skip a row.
 */
export const insertEntry = (
  list: ClipboardEntry[],
  entry: ClipboardEntry,
  filter: TypeFilter | null,
  hasMore: boolean
): ClipboardEntry[] => {
  const rest = list.filter((item) => item.id !== entry.id);
  if (!entryMatchesType(entry.content_type, filter)) return rest;
  let at = rest.findIndex((item) => compareEntries(entry, item) < 0);
  if (at < 0) {
    if (hasMore) return rest;
    at = rest.length;
  }
  return [...rest.slice(0, at), entry, ...rest.slice(at)];
};

export const removeEntries = (list: ClipboardEntry[], ids: Iterable<number>): ClipboardEntry[] => {
  const gone = new Set(ids);
  const next = list.filter((item) => !gone.has(item.id));
  return next.length === list.length ? list : next;
};

/** The next page, minus rows already shown. */
export const appendPage = (list: ClipboardEntry[], page: ClipboardEntry[]): ClipboardEntry[] => {
  const known = new Set(list.map((item) => item.id));
  return [...list, ...page.filter((item) => !known.has(item.id))];
};

/** A refresh reloads everything shown, however long, not just the first page. */
export const refreshCount = (loaded: number, pageSize: number) => Math.max(pageSize, loaded);
