import { describe, expect, it } from "vitest";
import type { FavoriteItem } from "../../shared/types";
import {
  appendTag,
  collectSortedTags,
  MAX_TAGS_PER_ITEM,
  normalizeTag,
  normalizeTitle,
  splitTagDraft,
} from "./favoriteTags";

const item = (tags: string[]): FavoriteItem => ({
  id: 0, content_type: "text", content: "", preview: "", source_app: "",
  sort_order: 0, created_at: 0, paths: [], title: "", tags,
});

describe("favorite tags", () => {
  it("trims and cuts by code point", () => {
    expect(normalizeTag("  work  ")).toBe("work");
    expect(normalizeTag("   ")).toBe("");
    expect(normalizeTag("😀".repeat(30))).toBe("😀".repeat(24));
    expect(normalizeTitle(` ${"长".repeat(100)} `)).toBe("长".repeat(80));
    expect(normalizeTitle(`${"a".repeat(79)}  b`)).toBe("a".repeat(79));
  });

  it("adds tags once, ignoring case, in the spelling already in use", () => {
    expect(appendTag([], " prompt ", ["Prompt"])).toEqual({ next: ["Prompt"], status: "added" });
    expect(appendTag(["Prompt"], "PROMPT", [])).toEqual({ next: ["Prompt"], status: "duplicate" });
    expect(appendTag(["a"], "  ", [])).toEqual({ next: ["a"], status: "empty" });
    const full = Array.from({ length: MAX_TAGS_PER_ITEM }, (_, n) => `t${n}`);
    expect(appendTag(full, "one more", []).status).toBe("limit");
  });

  it("splits typed text at ASCII and full-width commas", () => {
    expect(splitTagDraft("a,b，c")).toEqual({ complete: ["a", "b"], rest: "c" });
    expect(splitTagDraft("abc")).toEqual({ complete: [], rest: "abc" });
    expect(splitTagDraft("abc,")).toEqual({ complete: ["abc"], rest: "" });
  });

  it("sorts by use, then alphabetically", () => {
    const items = [item(["b", "a"]), item(["b", "c"]), item([" a "]), item(["z"])];
    expect(collectSortedTags(items)).toEqual(["a", "b", "c", "z"]);
    expect(collectSortedTags([item(["x"]), item(["y", "x"])])).toEqual(["x", "y"]);
    expect(collectSortedTags([{ ...item([]), tags: undefined as unknown as string[] }])).toEqual([]);
  });
});
