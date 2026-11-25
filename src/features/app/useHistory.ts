import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ClipboardEntry } from "../../shared/types";
import { entryMatchesType, isEntry, type EntryKey, type TypeFilter } from "../../shared/lib/clipboardTypes";
import { isTauriRuntime } from "../../shared/lib/tauriRuntime";
import { appendPage, compareEntries, insertEntry, refreshCount, removeEntries } from "./historyList";

const PAGE_SIZE = 80;
const SEARCH_LIMIT = 300;
/** A page whose offset went stale while loading is asked for again, this often at most. */
const MAX_PAGE_ATTEMPTS = 3;

/**
 * The list on screen. `search` is what was submitted (Search button / Return),
 * not what is being typed: the list doesn't move while typing.
 */
export const useHistory = (search: string, typeFilter: TypeFilter | null, ready: boolean) => {
  const [list, setList] = useState<ClipboardEntry[]>([]);
  // Mirrors `list` synchronously: its length is the offset of the next page.
  const listRef = useRef<ClipboardEntry[]>([]);
  /** Bumped by every change to the loaded rows, so a page loaded across one is retried. */
  const mutationRef = useRef(0);
  const [hasMore, setHasMore] = useState(false);
  const hasMoreRef = useRef(false);
  const [loading, setLoading] = useState(false);
  const loadingRef = useRef(false);
  const [loaded, setLoaded] = useState(false);
  const sequenceRef = useRef(0);
  const term = search.trim();
  const query = `${typeFilter ?? ""}\u0000${term}`;
  /** The query whose first page has arrived; until then the list shows the previous one. */
  const [settledQuery, setSettledQuery] = useState<string | null>(null);
  const settledRef = useRef<string | null>(null);

  const commit = useCallback((next: ClipboardEntry[]) => {
    if (next === listRef.current) return;
    listRef.current = next;
    mutationRef.current += 1;
    setList(next);
  }, []);

  const setMore = useCallback((more: boolean) => {
    hasMoreRef.current = more;
    setHasMore(more);
  }, []);

  const fetchFirstPage = useCallback(
    async (keepLoaded = false) => {
      if (!isTauriRuntime()) return;
      const sequence = ++sequenceRef.current;
      try {
        if (term) {
          // One more than shown tells whether the results were cut off.
          const found = await invoke<ClipboardEntry[]>("search_clipboard_history", {
            searchTerm: term,
            limit: SEARCH_LIMIT + 1,
            contentType: typeFilter ?? undefined,
          });
          if (sequence !== sequenceRef.current) return;
          commit(found.slice(0, SEARCH_LIMIT).sort(compareEntries));
          setMore(found.length > SEARCH_LIMIT);
        } else {
          const count = keepLoaded ? refreshCount(listRef.current.length, PAGE_SIZE) : PAGE_SIZE;
          const rows = await invoke<ClipboardEntry[]>("get_clipboard_history", {
            limit: count + 1,
            offset: 0,
            contentType: typeFilter ?? undefined,
          });
          if (sequence !== sequenceRef.current) return;
          const more = rows.length > count;
          commit(more ? rows.slice(0, count) : rows);
          setMore(more);
        }
        settledRef.current = query;
        setSettledQuery(query);
        setLoaded(true);
      } catch (error) {
        console.error("History failed to load", error);
      }
    },
    [commit, query, setMore, term, typeFilter]
  );

  const loadMore = useCallback(async () => {
    if (term || loadingRef.current || !hasMoreRef.current || settledRef.current !== query) return;
    loadingRef.current = true;
    setLoading(true);
    try {
      for (let attempt = 0; attempt < MAX_PAGE_ATTEMPTS; attempt++) {
        const sequence = sequenceRef.current;
        const mutation = mutationRef.current;
        const rows = await invoke<ClipboardEntry[]>("get_clipboard_history", {
          limit: PAGE_SIZE + 1,
          offset: listRef.current.length,
          contentType: typeFilter ?? undefined,
        });
        if (sequence !== sequenceRef.current) return;
        // A copy or deletion landed while the page loaded: the backend may
        // have counted it or not, so the offset is asked for again.
        if (mutation !== mutationRef.current) continue;
        const more = rows.length > PAGE_SIZE;
        commit(appendPage(listRef.current, more ? rows.slice(0, PAGE_SIZE) : rows));
        setMore(more);
        return;
      }
    } catch (error) {
      console.error("History failed to load", error);
    } finally {
      loadingRef.current = false;
      setLoading(false);
    }
  }, [commit, query, setMore, term, typeFilter]);

  /** Rows removed here (an optimistic delete) leave the next page's offset right. */
  const removeItems = useCallback(
    (entries: EntryKey[]) => {
      const byId = new Map(entries.map((entry) => [entry.id, entry]));
      commit(listRef.current.filter((entry) => {
        const removed = byId.get(entry.id);
        return !removed || !isEntry(entry, removed);
      }));
    },
    [commit]
  );

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
        commit(insertEntry(listRef.current, payload, filterRef.current, hasMoreRef.current));
      }),
      listen<number>("clipboard-removed", ({ payload }) => {
        commit(removeEntries(listRef.current, [payload]));
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
  }, [commit]);

  // Already in display order; this only hides the previous tab's rows until
  // the new tab's first page arrives.
  const visible = useMemo(
    () => (typeFilter ? list.filter((item) => entryMatchesType(item.content_type, typeFilter)) : list),
    [list, typeFilter]
  );

  return {
    items: visible,
    hasMore,
    loading,
    loaded,
    /** The list shows the finished result of the current search and tab. */
    settled: settledQuery === query,
    loadMore,
    refetch: fetchFirstPage,
    removeItems,
  };
};
