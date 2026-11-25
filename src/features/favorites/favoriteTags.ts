import type { FavoriteItem } from "../../shared/types";

// Same limits as the backend (favorites_repo.rs), which has the last word.
export const MAX_TITLE_LENGTH = 80;
export const MAX_TAG_LENGTH = 24;
export const MAX_TAGS_PER_ITEM = 8;
export const MAX_CARD_VISIBLE_TAGS = 3;

/** Trim, then cut to `max` code points, so emoji and CJK are never split. */
const clip = (raw: string, max: number) => Array.from(raw.trim()).slice(0, max).join("").trimEnd();

export const normalizeTitle = (raw: string): string => clip(raw, MAX_TITLE_LENGTH);

export const normalizeTag = (raw: string): string => clip(raw, MAX_TAG_LENGTH);

export type AppendStatus = "added" | "empty" | "duplicate" | "limit";

/**
 * Add one tag. Repeats are found ignoring case, and a tag already in use
 * elsewhere keeps that spelling ("prompt" becomes "Prompt").
 */
export const appendTag = (
  current: string[],
  raw: string,
  knownPool: string[]
): { next: string[]; status: AppendStatus } => {
  const cleaned = normalizeTag(raw);
  if (!cleaned) return { next: current, status: "empty" };
  const lower = cleaned.toLowerCase();
  if (current.some((tag) => tag.toLowerCase() === lower)) return { next: current, status: "duplicate" };
  if (current.length >= MAX_TAGS_PER_ITEM) return { next: current, status: "limit" };
  const canonical = knownPool.find((tag) => tag.toLowerCase() === lower) ?? cleaned;
  return { next: [...current, canonical], status: "added" };
};

/** Tag input text split at commas: the finished parts, and what is still being typed. */
export const splitTagDraft = (value: string): { complete: string[]; rest: string } => {
  const parts = value.split(/[,，]/);
  const rest = parts.pop() ?? "";
  return { complete: parts, rest };
};

/** Tags in use, most used first, then alphabetical. */
export const collectSortedTags = (items: FavoriteItem[]): string[] => {
  const counts = new Map<string, number>();
  for (const item of items) {
    for (const tag of item.tags ?? []) {
      const cleaned = normalizeTag(tag);
      if (cleaned) counts.set(cleaned, (counts.get(cleaned) ?? 0) + 1);
    }
  }
  return Array.from(counts.entries())
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([tag]) => tag);
};
