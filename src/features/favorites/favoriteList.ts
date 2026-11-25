import type { FavoriteItem } from "../../shared/types";
import { isEditableEntry } from "../../shared/lib/clipboardTypes";

/** 常用语 holds text; 图片与文件 holds everything else (image, video, file). */
export type FavoriteTab = "text" | "media";
export const FAVORITE_TABS: FavoriteTab[] = ["text", "media"];

export const inTab = (item: FavoriteItem, tab: FavoriteTab) =>
  tab === "media" ? !isEditableEntry(item.content_type) : isEditableEntry(item.content_type);

export const matchesVisible = (item: FavoriteItem, tab: FavoriteTab, tag: string | null) =>
  inTab(item, tab) && (tag === null || (item.tags ?? []).includes(tag));

/**
 * Put a reordered visible list back into the full one. The visible items
 * trade the slots they already held; every other item keeps its place.
 */
export const mergeOrder = (
  all: FavoriteItem[],
  tab: FavoriteTab,
  tag: string | null,
  reordered: FavoriteItem[]
) => {
  let next = 0;
  return all.map((item) => (matchesVisible(item, tab, tag) ? reordered[next++] ?? item : item));
};
