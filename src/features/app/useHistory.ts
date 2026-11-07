import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ClipboardEntry } from "../../shared/types";
import { entryMatchesType, type TypeFilter } from "../../shared/lib/clipboardTypes";
import { isTauriRuntime } from "../../shared/lib/tauriRuntime";

const PAGE_SIZE = 80;
const SEARCH_LIMIT = 300;

const activityAt = (item: ClipboardEntry) => item.sort_at || item.timestamp;

export const compareEntries = (a: ClipboardEntry, b: ClipboardEntry) => {
  if (a.is_pinned !== b.is_pinned) return a.is_pinned ? -1 : 1;
  if (a.is_pinned) {
    const byOrder = (b.pinned_order || 0) - (a.pinned_order || 0);
    if (byOrder) return byOrder;
  }
  return activityAt(b) - activityAt(a) || b.id - a.id;
};

/**
 * The list on screen. `search` is what was submitted (Search button / Return),
 * not what is being typed: the list doesn't move while typing.
 */
export const useHistory = (search: string, typeFilter: TypeFilter | null, ready: boolean) => {
  const [items, setItems] = useState<ClipboardEntry[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const offsetRef = useRef(0);
  const sequenceRef = useRef(0);
  const term = search.trim();

  const fetchFirstPage = useCallback(
    async (keepLoaded = false) => {
      if (!isTauriRuntime()) return;
      const sequence = ++sequenceRef.current;
      try {
        if (term) {
          const found = await invoke<ClipboardEntry[]>("search_clipboard_history", {
            searchTerm: term,
            limit: SEARCH_LIMIT,
          });
          if (sequence !== sequenceRef.current) return;
          setItems(found);
          setHasMore(false);
          offsetRef.current = found.length;
        } else {
          const count = keepLoaded ? Math.max(PAGE_SIZE, offsetRef.current) : PAGE_SIZE;
          const rows = await invoke<ClipboardEntry[]>("get_clipboard_history", {
            limit: count + 1,
            offset: 0,
            contentType: typeFilter ?? undefined,
          });
          if (sequence !== sequenceRef.current) return;
          const more = rows.length > count;
          const page = more ? rows.slice(0, count) : rows;
          setItems(page);
          setHasMore(more);
          offsetRef.current = page.length;
        }
        setLoaded(true);
      } catch (error) {
        console.error("History failed to load", error);
      }
    },
    [term, typeFilter]
  );

  const loadMore = useCallback(async () => {
    if (term || loading || !hasMore) return;
    const sequence = sequenceRef.current;
    setLoading(true);
    try {
      const rows = await invoke<ClipboardEntry[]>("get_clipboard_history", {
        limit: PAGE_SIZE + 1,
        offset: offsetRef.current,
        contentType: typeFilter ?? undefined,
      });
      if (sequence !== sequenceRef.current) return;
      const more = rows.length > PAGE_SIZE;
      const page = more ? rows.slice(0, PAGE_SIZE) : rows;
      offsetRef.current += page.length;
      setItems((prev) => {
        const known = new Set(prev.map((item) => item.id));
        return [...prev, ...page.filter((item) => !known.has(item.id))];
      });
      setHasMore(more);
    } catch (error) {
      console.error("History failed to load", error);
    } finally {
      setLoading(false);
    }
  }, [hasMore, loading, term, typeFilter]);

  useEffect(() => {
    if (ready) void fetchFirstPage();
  }, [fetchFirstPage, ready]);

  const fetchRef = useRef(fetchFirstPage);
  fetchRef.current = fetchFirstPage;
  const termRef = useRef(term);
  termRef.current = term;
  const filterRef = useRef(typeFilter);
  filterRef.current = typeFilter;

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const subscriptions = [
      listen<ClipboardEntry>("clipboard-updated", ({ payload }) => {
        if (termRef.current) {
          void fetchRef.current(true);
          return;
        }
        setItems((prev) => {
          const rest = prev.filter((item) => item.id !== payload.id);
          return entryMatchesType(payload.content_type, filterRef.current) ? [payload, ...rest] : rest;
        });
      }),
      listen<number>("clipboard-removed", ({ payload }) => {
        setItems((prev) => prev.filter((item) => item.id !== payload));
      }),
      listen("clipboard-changed", () => void fetchRef.current(true)),
      // A searched word may be in an image that has only now been read.
      listen("clipboard-ocr-ready", () => {
        if (termRef.current) void fetchRef.current(true);
      }),
    ];
    return () => {
      subscriptions.forEach((subscription) => void subscription.then((off) => off()));
    };
  }, []);

  const visible = useMemo(
    () => items.filter((item) => entryMatchesType(item.content_type, typeFilter)).sort(compareEntries),
    [items, typeFilter]
  );

  return { items: visible, setItems, hasMore, loading, loaded, loadMore, refetch: fetchFirstPage };
};
