/** The tabs under the search box, in display order. */
export const TYPE_FILTERS = ["text", "image", "video", "file", "url", "code"] as const;
export type TypeFilter = (typeof TYPE_FILTERS)[number];

/** Clicking the tab that is on turns the filter off: there is no 全部 tab. */
export const toggleTypeFilter = (current: TypeFilter | null, clicked: TypeFilter): TypeFilter | null =>
  current === clicked ? null : clicked;

/** Tab / Shift+Tab, passing through "no filter" at the ends. */
export const cycleTypeFilter = (current: TypeFilter | null, step: 1 | -1): TypeFilter | null => {
  const stops: (TypeFilter | null)[] = [null, ...TYPE_FILTERS];
  const index = Math.max(0, stops.indexOf(current));
  return stops[(index + step + stops.length) % stops.length];
};

/** Rich text has no tab of its own: it counts as 文本. */
export const entryMatchesType = (contentType: string, filter: TypeFilter | null): boolean => {
  if (!filter) return true;
  return filter === "text" ? contentType === "text" || contentType === "rich_text" : contentType === filter;
};

/** Text entries are the ones that can be edited (文本 / 代码 / 网址). */
export const isTextEntry = (contentType: string): boolean =>
  ["text", "rich_text", "code", "url"].includes(contentType);

export const isEditableEntry = isTextEntry;
