import { describe, expect, it } from "vitest";
import type { FavoriteItem } from "../../shared/types";
import { inTab, matchesVisible, mergeOrder } from "./favoriteList";

const item = (id: number, content_type: string, tags: string[] = []): FavoriteItem => ({
  id, content_type, content: "", preview: "", source_app: "",
  sort_order: id, created_at: 0, paths: [], title: "", tags,
});

describe("favorite tabs", () => {
  it("puts every type in exactly one tab, video and file under media", () => {
    for (const type of ["text", "rich_text", "code", "url"]) {
      expect(inTab(item(1, type), "text")).toBe(true);
      expect(inTab(item(1, type), "media")).toBe(false);
    }
    for (const type of ["image", "video", "file"]) {
      expect(inTab(item(1, type), "media")).toBe(true);
      expect(inTab(item(1, type), "text")).toBe(false);
    }
  });

  it("filters by tag within the tab", () => {
    expect(matchesVisible(item(1, "text", ["a"]), "text", "a")).toBe(true);
    expect(matchesVisible(item(1, "text", ["b"]), "text", "a")).toBe(false);
    expect(matchesVisible(item(1, "image", ["a"]), "text", "a")).toBe(false);
    expect(matchesVisible(item(1, "text"), "text", null)).toBe(true);
  });
});

describe("mergeOrder", () => {
  const ids = (list: FavoriteItem[]) => list.map((favorite) => favorite.id);

  it("reorders the filtered items inside their own slots", () => {
    const all = [
      item(1, "text", ["p"]),
      item(2, "text"),
      item(3, "image", ["p"]),
      item(4, "text", ["p"]),
      item(5, "text"),
      item(6, "text", ["p"]),
    ];
    const [one, , , four, , six] = all;
    // Under "p" in 常用语, drag the last visible item to the top.
    const merged = mergeOrder(all, "text", "p", [six, one, four]);
    expect(ids(merged)).toEqual([6, 2, 3, 1, 5, 4]);
  });

  it("keeps the other tab's slots when nothing is filtered", () => {
    const all = [item(1, "text"), item(2, "video"), item(3, "text"), item(4, "file")];
    expect(ids(mergeOrder(all, "media", null, [all[3], all[1]]))).toEqual([1, 4, 3, 2]);
  });
});
