import { useCallback, useEffect, useRef, useState } from "react";
import { Reorder, useDragControls } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FileText, GripVertical, Pencil, Play, Plus, Trash2 } from "lucide-react";
import type { FavoriteItem, Translate } from "../../shared/types";
import { isEditableEntry } from "../../shared/lib/clipboardTypes";
import { useImageThumbnail, useVideoThumbnail } from "../../shared/hooks/useImageThumbnail";
import { startWindowDrag } from "../../shared/lib/windowDrag";

interface FavoritesPanelProps {
  t: Translate;
  pushToast: (message: string, duration?: number) => void;
  onPasteFailed: (error: unknown) => void;
}

const fileName = (path: string) => path.split(/[\\/]/).pop() || path;

type FavoriteTab = "text" | "image";
const FAVORITE_TABS: FavoriteTab[] = ["text", "image"];
const inTab = (item: FavoriteItem, tab: FavoriteTab) =>
  tab === "image" ? item.content_type === "image" : isEditableEntry(item.content_type);

/** Put a reordered tab back into the full list, keeping the other tab's slots. */
const mergeOrder = (all: FavoriteItem[], tab: FavoriteTab, reordered: FavoriteItem[]) => {
  let next = 0;
  return all.map((item) => (inTab(item, tab) ? reordered[next++] ?? item : item));
};

const FavoriteMedia = ({ item }: { item: FavoriteItem }) => {
  const path = item.paths[0] ?? "";
  const image = useImageThumbnail(item.content_type === "image" ? path : null, true, item.created_at);
  const video = useVideoThumbnail(item.content_type === "video" ? path : null, item.created_at);
  if (item.content_type === "image") {
    // 常用图 shows the picture alone, never the attachment's file name.
    return (
      <span className="favorite-media is-image">
        {image && <img className="favorite-image" src={image} alt="" draggable={false} />}
      </span>
    );
  }
  return (
    <span className="favorite-media">
      {item.content_type === "video" && (
        <span className="favorite-thumb is-video">
          {video && <img src={video} alt="" draggable={false} />}
          <Play size={12} />
        </span>
      )}
      {item.content_type === "file" && <FileText size={20} className="favorite-file-icon" />}
      <span className="favorite-name">{fileName(path)}</span>
    </span>
  );
};

/** Add or edit a text favorite, in place. */
const FavoriteEditor = ({
  t,
  initial,
  onSave,
  onCancel,
}: {
  t: Translate;
  initial: string;
  onSave: (text: string) => void;
  onCancel: () => void;
}) => {
  const [text, setText] = useState(initial);
  return (
    <div className="favorite-editor">
      <textarea
        autoFocus
        className="glass-field favorite-textarea"
        placeholder={t("favorites_placeholder")}
        value={text}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            onCancel();
          }
        }}
      />
      <div className="favorite-editor-actions">
        <button type="button" className="glass-button" onClick={onCancel}>
          {t("cancel")}
        </button>
        <button
          type="button"
          className="glass-button is-primary"
          disabled={!text.trim()}
          onClick={() => onSave(text)}
        >
          {t("save")}
        </button>
      </div>
    </div>
  );
};

const FavoriteRow = ({
  item,
  t,
  editing,
  onPaste,
  onEdit,
  onSaveEdit,
  onCancelEdit,
  onDelete,
  onDragEnd,
}: {
  item: FavoriteItem;
  t: Translate;
  editing: boolean;
  onPaste: () => void;
  onEdit: () => void;
  onSaveEdit: (text: string) => void;
  onCancelEdit: () => void;
  onDelete: () => void;
  onDragEnd: () => void;
}) => {
  const dragControls = useDragControls();
  return (
    <Reorder.Item
      value={item}
      dragListener={false}
      dragControls={dragControls}
      onDragEnd={onDragEnd}
      className="favorite-row"
      whileDrag={{ scale: 1.02 }}
    >
      {editing ? (
        <FavoriteEditor t={t} initial={item.content} onSave={onSaveEdit} onCancel={onCancelEdit} />
      ) : (
        <div className="favorite-item" onClick={onPaste}>
          <span
            className="favorite-grip"
            title={t("favorites_drag")}
            onClick={(event) => event.stopPropagation()}
            onPointerDown={(event) => {
              event.preventDefault();
              dragControls.start(event);
            }}
          >
            <GripVertical size={14} />
          </span>
          {isEditableEntry(item.content_type) ? (
            <span className={`favorite-text${item.content_type === "code" ? " is-code" : ""}`}>
              {item.preview || item.content}
            </span>
          ) : (
            <FavoriteMedia item={item} />
          )}
          <span className="favorite-actions">
            {isEditableEntry(item.content_type) && (
              <button
                type="button"
                className="card-action"
                title={t("edit")}
                aria-label={t("edit")}
                onClick={(event) => {
                  event.stopPropagation();
                  onEdit();
                }}
              >
                <Pencil size={12} />
              </button>
            )}
            <button
              type="button"
              className="card-action is-danger"
              title={t("delete")}
              aria-label={t("delete")}
              onClick={(event) => {
                event.stopPropagation();
                onDelete();
              }}
            >
              <Trash2 size={12} />
            </button>
          </span>
        </div>
      )}
    </Reorder.Item>
  );
};

const FavoritesPanel = ({ t, pushToast, onPasteFailed }: FavoritesPanelProps) => {
  const [items, setItems] = useState<FavoriteItem[]>([]);
  const [adding, setAdding] = useState(false);
  const [tab, setTab] = useState<FavoriteTab>("text");
  const [editingId, setEditingId] = useState<number | null>(null);
  const orderRef = useRef(items);
  orderRef.current = items;

  const refresh = useCallback(() => {
    invoke<FavoriteItem[]>("list_favorites").then(setItems).catch(console.error);
  }, []);

  useEffect(() => {
    refresh();
    const subscription = listen("favorites-changed", refresh);
    return () => void subscription.then((off) => off());
  }, [refresh]);

  const visible = items.filter((item) => inTab(item, tab));

  const fail = (error: unknown) => pushToast(t("save_failed") + String(error), 3000);

  return (
    <div className="side-panel-body">
      <div className="panel-title" onMouseDown={startWindowDrag}>
        <h2>{t("favorites_title")}</h2>
        <button
          type="button"
          className="glass-button is-compact"
          onClick={() => {
            // Typed favorites are text, so they belong to 常用语.
            setTab("text");
            setAdding(true);
          }}
        >
          <Plus size={13} />
          {t("favorites_add")}
        </button>
      </div>

      <div className="favorite-tabs" role="tablist">
        {FAVORITE_TABS.map((id) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={tab === id}
            className={`type-tab${tab === id ? " is-active" : ""}`}
            onClick={() => {
              setTab(id);
              setEditingId(null);
              if (id !== "text") setAdding(false);
            }}
          >
            {t(`favorites_tab_${id}`)}
          </button>
        ))}
      </div>

      <div className="panel-scroll">
        {adding && (
          <FavoriteEditor
            t={t}
            initial=""
            onCancel={() => setAdding(false)}
            onSave={(text) => {
              invoke("add_favorite_text", { text })
                .then(() => setAdding(false))
                .catch(fail);
            }}
          />
        )}

        {visible.length === 0 && !adding ? (
          <div className="empty-state">
            <p className="empty-title">{t(`favorites_empty_${tab}`)}</p>
            <p className="empty-hint">{t(`favorites_empty_hint_${tab}`)}</p>
          </div>
        ) : (
          <Reorder.Group
            axis="y"
            values={visible}
            onReorder={(reordered) => setItems((all) => mergeOrder(all, tab, reordered))}
            className={`favorite-list${tab === "image" ? " is-images" : ""}`}
          >
            {visible.map((item) => (
              <FavoriteRow
                key={item.id}
                item={item}
                t={t}
                editing={editingId === item.id}
                onPaste={() => {
                  invoke("paste_favorite", { id: item.id }).catch(onPasteFailed);
                }}
                onEdit={() => setEditingId(item.id)}
                onCancelEdit={() => setEditingId(null)}
                onSaveEdit={(text) => {
                  invoke("update_favorite_text", { id: item.id, text })
                    .then(() => setEditingId(null))
                    .catch(fail);
                }}
                onDelete={() => {
                  invoke("delete_favorite", { id: item.id }).catch(fail);
                }}
                onDragEnd={() => {
                  invoke("reorder_favorites", { ids: orderRef.current.map((favorite) => favorite.id) }).catch(fail);
                }}
              />
            ))}
          </Reorder.Group>
        )}
      </div>
    </div>
  );
};

export default FavoritesPanel;
