import { describe, expect, test } from "vitest";
import type { ClipboardEntry } from "../../shared/types";
import { appendPage, compareEntries, insertEntry, refreshCount, removeEntries } from "./historyList";

const entry = (id: number, extra: Partial<ClipboardEntry> = {}): ClipboardEntry => ({
  id,
  content_type: "text",
  content: `text ${id}`,
  source_app: "Test",
  timestamp: id,
  sort_at: id,
  created_at: id,
  preview: `text ${id}`,
  is_pinned: false,
  ...extra,
});

const ids = (items: ClipboardEntry[]) => items.map((item) => item.id);

/** The backend: the stored rows in display order, paged by offset. */
class Store {
  rows: ClipboardEntry[];
  constructor(count: number) {
    this.rows = Array.from({ length: count }, (_, index) => entry(count - index));
  }
  page(offset: number, limit: number) {
    return this.rows.slice(offset, offset + limit);
  }
  add(row: ClipboardEntry) {
    this.rows = [...this.rows.filter((item) => item.id !== row.id), row].sort(compareEntries);
  }
  remove(id: number) {
    this.rows = this.rows.filter((item) => item.id !== id);
  }
  hasMoreThan(loaded: ClipboardEntry[]) {
    return loaded.length < this.rows.length;
  }
}

const PAGE = 10;

/** Loads pages until the end, the way scrolling does: offset = rows shown. */
const loadAll = (store: Store, shown: ClipboardEntry[]) => {
  let list = shown;
  for (let guard = 0; guard < 100 && store.hasMoreThan(list); guard++) {
    list = appendPage(list, store.page(list.length, PAGE));
  }
  return list;
};

describe("pagination offset", () => {
  test("new copies shown at the top keep the next page from repeating a row", () => {
    const store = new Store(30);
    let list = store.page(0, PAGE);
    for (const id of [31, 32]) {
      store.add(entry(id));
      list = insertEntry(list, entry(id), null, true);
    }
    expect(ids(loadAll(store, list))).toEqual(ids(store.rows));
  });

  test("auto-deleted rows keep the next page from skipping one", () => {
    const store = new Store(30);
    let list = store.page(0, PAGE);
    for (const id of [30, 25, 21]) {
      store.remove(id);
      list = removeEntries(list, [id]);
    }
    expect(list).toHaveLength(PAGE - 3);
    expect(ids(loadAll(store, list))).toEqual(ids(store.rows));
  });

  test("an entry changed beyond the loaded rows waits for its page", () => {
    const store = new Store(30);
    let list = store.page(0, PAGE);
    // Row 3 (page 3) got its picture updated: still sorts after everything loaded.
    const changed = entry(3, { preview: "updated" });
    store.add(changed);
    list = insertEntry(list, changed, null, true);
    expect(list).toHaveLength(PAGE);
    expect(ids(loadAll(store, list))).toEqual(ids(store.rows));
    // With everything loaded there is no later page: it goes at the end.
    expect(ids(insertEntry(store.page(0, 3), entry(1), null, false))).toEqual([30, 29, 28, 1]);
  });

  test("a loaded entry that now sorts past the loaded rows leaves them", () => {
    const store = new Store(30);
    let list = store.page(0, PAGE);
    const unpinnedOld = entry(25, { sort_at: 2 });
    store.add(unpinnedOld);
    list = insertEntry(list, unpinnedOld, null, true);
    expect(ids(loadAll(store, list))).toEqual(ids(store.rows));
  });

  test("an entry outside the tab only leaves the list", () => {
    const list = [entry(3), entry(2), entry(1)];
    expect(ids(insertEntry(list, entry(4, { content_type: "image" }), "text", true))).toEqual([3, 2, 1]);
    expect(ids(insertEntry(list, entry(2, { content_type: "image" }), "text", true))).toEqual([3, 1]);
    // Rich text belongs to the 文本 tab.
    expect(ids(insertEntry(list, entry(4, { content_type: "rich_text" }), "text", true))).toEqual([4, 3, 2, 1]);
  });

  test("new copies go below the pins", () => {
    const pinned = entry(1, { is_pinned: true, pinned_order: 1 });
    expect(ids(insertEntry([pinned, entry(5)], entry(9), null, true))).toEqual([1, 9, 5]);
  });

  test("a refresh reloads everything shown, past 1000 rows too", () => {
    expect(refreshCount(1040, 80)).toBe(1040);
    expect(refreshCount(12, 80)).toBe(80);
  });

  test("removing nothing keeps the same list", () => {
    const list = [entry(1)];
    expect(removeEntries(list, [7])).toBe(list);
  });
});
