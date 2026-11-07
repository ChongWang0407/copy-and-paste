import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso";
import type { ReactNode } from "react";
import type { ClipboardEntry, Translate } from "../../shared/types";

/** Rows get ready this far outside the view, in px, so pictures are in before they show. */
const LIST_RENDER_AHEAD = 800;

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
        scrollerRef={(element) => {
          scrollerRef.current = element instanceof HTMLElement ? element : null;
        }}
        // Set directly: a render per scroll start would cost more than it saves.
        isScrolling={(scrolling) => scrollerRef.current?.classList.toggle("is-scrolling", scrolling)}
        className="clipboard-list"
        data={items}
        // By entry, not index: a new copy shifts every index.
        computeItemKey={(_index, item) => item.id}
        itemContent={(_index, item) => <div className="list-row">{renderItem(item)}</div>}
        endReached={() => {
          if (hasMore && !loading) onLoadMore();
        }}
        increaseViewportBy={LIST_RENDER_AHEAD}
        components={{
          // Room above the first card for its search glow, which would be cut off.
          Header: () => <div className="list-start" />,
          Footer: () => (loading ? <div className="list-footer">{t("loading")}</div> : <div className="list-end" />),
        }}
      />
    );
  }
);

ClipboardList.displayName = "ClipboardList";

export default ClipboardList;
