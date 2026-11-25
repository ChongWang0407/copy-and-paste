import { forwardRef, memo, useCallback, useEffect, useImperativeHandle, useMemo, useRef } from "react";
import { Virtuoso, type Components, type VirtuosoHandle } from "react-virtuoso";
import type { ReactNode } from "react";
import type { ClipboardEntry, Translate } from "../../shared/types";

/**
 * Rows get ready this far outside the view, in px, so pictures are in before
 * they show. More below: that is the way the list is usually scrolled.
 */
const LIST_RENDER_AHEAD = { top: 300, bottom: 600 };

export interface ClipboardListHandle {
  scrollToTop: () => void;
}

interface ClipboardListProps {
  t: Translate;
  items: ClipboardEntry[];
  loaded: boolean;
  searching: boolean;
  hasMore: boolean;
  loading: boolean;
  selectedIndex: number | null;
  onLoadMore: () => void;
  renderItem: (item: ClipboardEntry) => ReactNode;
}

interface ListContext {
  t: Translate;
  loading: boolean;
}

// Defined once: components created during render are remounted on every render.
const LIST_COMPONENTS: Components<ClipboardEntry, ListContext> = {
  // Room above the first card for its search glow, which would be cut off.
  Header: () => <div className="list-start" />,
  Footer: ({ context }) =>
    context?.loading ? <div className="list-footer">{context.t("loading")}</div> : <div className="list-end" />,
};

// By entry, not index: a new copy shifts every index.
const itemKey = (_index: number, item: ClipboardEntry) => item.id;

const ClipboardList = forwardRef<ClipboardListHandle, ClipboardListProps>(
  ({ t, items, loaded, searching, hasMore, loading, selectedIndex, onLoadMore, renderItem }, ref) => {
    const virtuosoRef = useRef<VirtuosoHandle>(null);
    const scrollerRef = useRef<HTMLElement | null>(null);

    useImperativeHandle(ref, () => ({
      scrollToTop: () => virtuosoRef.current?.scrollTo({ top: 0, behavior: "auto" }),
    }));

    useEffect(() => {
      if (selectedIndex !== null) {
        virtuosoRef.current?.scrollIntoView({ index: selectedIndex, behavior: "auto" });
      }
    }, [selectedIndex]);

    const setScroller = useCallback((element: HTMLElement | Window | null) => {
      scrollerRef.current = element instanceof HTMLElement ? element : null;
    }, []);
    // Set directly: a render per scroll start would cost more than it saves.
    const markScrolling = useCallback(
      (scrolling: boolean) => scrollerRef.current?.classList.toggle("is-scrolling", scrolling),
      []
    );
    const itemContent = useCallback(
      (_index: number, item: ClipboardEntry) => <div className="list-row">{renderItem(item)}</div>,
      [renderItem]
    );
    const endReached = useCallback(() => {
      if (hasMore && !loading) onLoadMore();
    }, [hasMore, loading, onLoadMore]);
    const context = useMemo(() => ({ t, loading }), [loading, t]);

    if (loaded && items.length === 0) {
      return (
        <div className="empty-state">
          <p className="empty-title">{searching ? t("empty_search") : t("empty_title")}</p>
          {!searching && <p className="empty-hint">{t("empty_hint")}</p>}
        </div>
      );
    }

    return (
      <Virtuoso
        ref={virtuosoRef}
        scrollerRef={setScroller}
        isScrolling={markScrolling}
        className="clipboard-list"
        data={items}
        context={context}
        computeItemKey={itemKey}
        itemContent={itemContent}
        endReached={endReached}
        increaseViewportBy={LIST_RENDER_AHEAD}
        components={LIST_COMPONENTS}
      />
    );
  }
);

ClipboardList.displayName = "ClipboardList";

export default memo(ClipboardList);
