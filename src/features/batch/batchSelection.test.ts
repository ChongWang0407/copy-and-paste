import { describe, expect, test } from "vitest";
import type { ClipboardEntry } from "../../shared/types";
import {
  allVisibleSelected,
  applyEntryStates,
  batchDeleteMessage,
  deletionRequest,
  deselectAllVisible,
  entryKey,
  indexOfEntry,
  refreshSelection,
  selectAllVisible,
  selectionCountText,
  selectionCounts,
  showsLoadedSuffix,
  toggleSelectAllVisible,
} from "./batchSelection";

const entry = (id: number, extra: Partial<ClipboardEntry> = {}): ClipboardEntry => ({
  id,
  content_type: "text",
  content: `text ${id}`,
  source_app: "Test",
  timestamp: id,
  created_at: id * 10,
  preview: `text ${id}`,
  is_pinned: false,
  ...extra,
});

const ids = (items: ClipboardEntry[]) => items.map((item) => item.id);
// Echoes keys and placeholders, so the tests read the template choice.
const t = (key: string) => `${key}:{count}:{pinned}`;

describe("select all", () => {
  const [a, b, c, d] = [entry(1), entry(2), entry(3), entry(4)];

  test("adds the shown entries not ticked yet, in list order, after the earlier ticks", () => {
    expect(ids(selectAllVisible([c, entry(9)], [a, b, c, d]))).toEqual([3, 9, 1, 2, 4]);
  });

  test("deselect removes only the shown entries", () => {
    // 9 was ticked under another search: it stays.
    expect(ids(deselectAllVisible([a, entry(9), c], [a, b, c, d]))).toEqual([9]);
  });

  test("toggle selects until everything shown is ticked, then deselects", () => {
    const once = toggleSelectAllVisible([b], [a, b, c]);
    expect(ids(once)).toEqual([2, 1, 3]);
    expect(allVisibleSelected(once, [a, b, c])).toBe(true);
    expect(ids(toggleSelectAllVisible(once, [a, b, c]))).toEqual([]);
  });

  test("an empty list is never all selected", () => {
    expect(allVisibleSelected([a], [])).toBe(false);
    expect(toggleSelectAllVisible([a], [])).toEqual([a]);
  });
});

describe("counts", () => {
  test("pinned and outside-the-current-view entries are counted", () => {
    const pinned = entry(1, { is_pinned: true });
    const elsewhere = entry(7, { is_pinned: true });
    expect(selectionCounts([pinned, entry(2), elsewhere], [pinned, entry(2), entry(3)])).toEqual({
      total: 3,
      pinned: 2,
      outsideView: 1,
    });
  });

  test("(loaded) only when everything shown is ticked and more is not loaded", () => {
    expect(showsLoadedSuffix(true, true)).toBe(true);
    expect(showsLoadedSuffix(true, false)).toBe(false);
    expect(showsLoadedSuffix(false, true)).toBe(false);
    expect(selectionCountText(t, 80, true, true)).toBe("batch_selected_loaded:80:{pinned}");
    expect(selectionCountText(t, 2, true, false)).toBe("batch_selected_count:2:{pinned}");
  });

  test("the confirmation names pinned and out-of-view entries", () => {
    expect(batchDeleteMessage(t, { total: 3, pinned: 0, outsideView: 0 }).split("\n")).toEqual([
      "batch_delete_confirm:3:0",
      "batch_delete_irreversible:{count}:{pinned}",
    ]);
    expect(batchDeleteMessage(t, { total: 5, pinned: 2, outsideView: 1 }).split("\n")).toEqual([
      "batch_delete_confirm_pinned:5:2",
      "batch_delete_outside:1:{pinned}",
      "batch_delete_irreversible:{count}:{pinned}",
    ]);
  });
});

describe("entry identity", () => {
  test("a pick follows its entry and is lost when the ID is reused", () => {
    const picked = entry(5);
    const key = entryKey(picked);
    // New copies above shift the row.
    expect(indexOfEntry([entry(8), entry(7), entry(6), picked], key)).toBe(3);
    // Deleted, then the next copy got ID 5.
    expect(indexOfEntry([entry(6), entry(5, { created_at: 999 })], key)).toBeNull();
    expect(indexOfEntry([entry(6)], key)).toBeNull();
    expect(indexOfEntry([picked], null)).toBeNull();
  });

  test("ticks take the list's pin state and drop entries whose ID was reused", () => {
    const selected = [entry(1), entry(2), entry(3)];
    const list = [entry(1, { is_pinned: true }), entry(2, { created_at: 999 })];
    const refreshed = refreshSelection(selected, list);
    // 3 is not in this list (another tab): kept as it was.
    expect(ids(refreshed)).toEqual([1, 3]);
    expect(refreshed[0].is_pinned).toBe(true);
  });

  test("a refresh with nothing new keeps the same array", () => {
    const selected = [entry(1), entry(2)];
    expect(refreshSelection(selected, [entry(1), entry(2), entry(3)])).toBe(selected);
  });

  test("after selection_changed only entries still the same stay, with their pin state", () => {
    const selected = [entry(1), entry(2), entry(3), entry(-4)];
    const states = [
      { id: 1, createdAt: 10, isPinned: true },
      { id: 2, createdAt: 555, isPinned: false },
      { id: -4, createdAt: -40, isPinned: false },
    ];
    const next = applyEntryStates(selected, states);
    expect(ids(next)).toEqual([1, -4]);
    expect(next[0].is_pinned).toBe(true);
  });

  test("the deletion request carries ID, capture time and pin state", () => {
    expect(deletionRequest([entry(1, { is_pinned: true }), entry(2, { created_at: undefined })])).toEqual([
      { id: 1, createdAt: 10, isPinned: true },
      { id: 2, createdAt: 0, isPinned: false },
    ]);
  });
});
