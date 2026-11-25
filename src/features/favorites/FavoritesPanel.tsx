import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ChangeEvent, KeyboardEvent, MutableRefObject, WheelEvent } from "react";
import { Reorder, useDragControls } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FileText, GripVertical, Pencil, Play, Plus, Trash2, X } from "lucide-react";
import type { FavoriteItem, Translate } from "../../shared/types";
import { isEditableEntry } from "../../shared/lib/clipboardTypes";
import { useImageThumbnail, useVideoThumbnail } from "../../shared/hooks/useImageThumbnail";
import { startWindowDrag } from "../../shared/lib/windowDrag";
import { FAVORITE_TABS, inTab, matchesVisible, mergeOrder, type FavoriteTab } from "./favoriteList";
import {
  appendTag,
  collectSortedTags,
  MAX_CARD_VISIBLE_TAGS,
  MAX_TAGS_PER_ITEM,
  MAX_TITLE_LENGTH,
  normalizeTitle,
  splitTagDraft,
} from "./favoriteTags";

interface FavoritesPanelProps {
  t: Translate;
  pushToast: (message: string, duration?: number) => void;
  onPasteFailed: (error: unknown) => void;
  /** Tells the app an editor is open, so the list's keys stay off. */
  onEditingChange?: (editing: boolean) => void;
  /** Esc: closes the open editor first. False when none was open. */
  cancelEditRef?: MutableRefObject<(() => boolean) | null>;
}

type EditorMode = "text" | "media";

interface EditorValue {
  title: string;
  tags: string[];
  text: string;
}

const fileName = (path: string) => path.split(/[\\/]/).pop() || path;

/** Pinyin being picked: Return and commas belong to the input method. */
const composing = (event: KeyboardEvent) => event.nativeEvent.isComposing || event.keyCode === 229;

const moreFiles = (t: Translate, item: FavoriteItem) =>
  item.content_type === "file" && item.paths.length > 1
    ? t("more_files").replace("{count}", String(item.paths.length - 1))
    : null;

const FavoriteMedia = ({ item, t, hasTitle }: { item: FavoriteItem; t: Translate; hasTitle: boolean }) => {
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
  const more = moreFiles(t, item);
  return (
    <span className={`favorite-media${hasTitle ? " has-title" : ""}`}>
      {item.content_type === "video" && (
        <span className="favorite-thumb is-video">
          {video && <img src={video} alt="" draggable={false} />}
          <Play size={12} />
        </span>
      )}
      {item.content_type === "file" && <FileText size={20} className="favorite-file-icon" />}
      <span className="favorite-name" title={fileName(path)}>{fileName(path)}</span>
      {more && <span className="card-file-more">{more}</span>}
    </span>
  );
};

/** The media being edited, small, above the title. */
const FavoriteEditorPreview = ({ item, t }: { item: FavoriteItem; t: Translate }) => {
  const path = item.paths[0] ?? "";
  const image = useImageThumbnail(item.content_type === "image" ? path : null, true, item.created_at);
  const video = useVideoThumbnail(item.content_type === "video" ? path : null, item.created_at);
  const hint = <span className="favorite-editor-media-hint">{t("favorites_edit_media_hint")}</span>;
  if (item.content_type === "image") {
    return (
      <div className="favorite-editor-media">
        {image ? (
          <img className="favorite-editor-thumb" src={image} alt="" draggable={false} />
        ) : (
          <span className="favorite-editor-thumb" />
        )}
        {hint}
      </div>
    );
  }
  const more = moreFiles(t, item);
  return (
    <div className="favorite-editor-media">
      {item.content_type === "video" ? (
        <span className="favorite-thumb is-video">
          {video && <img src={video} alt="" draggable={false} />}
          <Play size={12} />
        </span>
      ) : (
        <FileText size={18} className="favorite-file-icon" />
      )}
      <span className="favorite-editor-media-meta">
        <span className="favorite-editor-media-name" title={fileName(path)}>
          {fileName(path)}
          {more && ` ${more}`}
        </span>
        {hint}
      </span>
    </div>
  );
};

/** Add or edit a favorite in place: title, tags and, for text, the text. */
const FavoriteEditor = ({
  t,
  mode,
  item,
  initialTitle,
  initialTags,
  initialText,
  initialFocus,
  existingTags,
  saving,
  pushToast,
  onSave,
  onCancel,
}: {
  t: Translate;
  mode: EditorMode;
  item?: FavoriteItem;
  initialTitle: string;
  initialTags: string[];
  initialText: string;
  initialFocus: "title" | "content";
  existingTags: string[];
  saving: boolean;
  pushToast: (message: string, duration?: number) => void;
  onSave: (value: EditorValue) => void;
  onCancel: () => void;
}) => {
  const [title, setTitle] = useState(initialTitle);
  const [tags, setTags] = useState(initialTags);
  const [tagDraft, setTagDraft] = useState("");
  const [text, setText] = useState(initialText);
  const tagInputRef = useRef<HTMLInputElement>(null);
  const textAreaRef = useRef<HTMLTextAreaElement>(null);
  const atLimit = tags.length >= MAX_TAGS_PER_ITEM;

  const suggestions = useMemo(() => {
    const chosen = new Set(tags.map((tag) => tag.toLowerCase()));
    return existingTags.filter((tag) => !chosen.has(tag.toLowerCase()));
  }, [existingTags, tags]);

  /** Adds each tag in turn; null when the limit stopped one. */
  const addTags = (current: string[], values: string[]): string[] | null => {
    let next = current;
    for (const value of values) {
      const result = appendTag(next, value, existingTags);
      if (result.status === "limit") {
        pushToast(t("favorites_tag_limit"), 2000);
        setTags(next);
        return null;
      }
      next = result.next;
    }
    setTags(next);
    return next;
  };

  /** The tag still in the input counts too: saving never drops it. */
  const commitDraft = (): string[] | null => {
    if (!tagDraft.trim()) return tags;
    const next = addTags(tags, [tagDraft]);
    if (next) setTagDraft("");
    return next;
  };

  const canSave = !saving && (mode === "media" || text.trim().length > 0);

  const submit = () => {
    if (!canSave) return;
    const finalTags = commitDraft();
    if (!finalTags) return;
    onSave({ title: normalizeTitle(title), tags: finalTags, text });
  };

  const focusAfterTags = () => {
    if (mode === "text") textAreaRef.current?.focus();
  };

  const handleTitleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (composing(event) || event.key !== "Enter" || event.metaKey || event.ctrlKey) return;
    event.preventDefault();
    if (atLimit) focusAfterTags();
    else tagInputRef.current?.focus();
  };

  const handleTagKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (composing(event) || event.metaKey || event.ctrlKey) return;
    if (event.key === "Enter" || event.key === "," || event.key === "，") {
      event.preventDefault();
      if (tagDraft.trim()) commitDraft();
      else if (event.key === "Enter") focusAfterTags();
      return;
    }
    if (event.key === "Backspace" && tagDraft === "" && tags.length > 0) {
      event.preventDefault();
      setTags(tags.slice(0, -1));
    }
  };

  // A comma typed through an input method, or pasted, also ends a tag.
  const handleTagChange = (event: ChangeEvent<HTMLInputElement>) => {
    const value = event.target.value;
    if ((event.nativeEvent as InputEvent).isComposing) {
      setTagDraft(value);
      return;
    }
    const { complete, rest } = splitTagDraft(value);
    if (complete.length === 0) {
      setTagDraft(value);
      return;
    }
    if (addTags(tags, complete)) setTagDraft(rest);
  };

  return (
    <div
      className="favorite-editor"
      onClick={(event) => event.stopPropagation()}
      onKeyDown={(event) => {
        if (composing(event)) return;
        if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
          event.preventDefault();
          submit();
        }
      }}
    >
      {mode === "media" && item && <FavoriteEditorPreview item={item} t={t} />}

      <input
        type="text"
        autoFocus={initialFocus === "title"}
        className="glass-field favorite-title-input"
        placeholder={t("favorites_title_placeholder")}
        aria-label={t("favorites_title_label")}
        maxLength={MAX_TITLE_LENGTH}
        value={title}
        disabled={saving}
        onChange={(event) => setTitle(event.target.value)}
        onKeyDown={handleTitleKeyDown}
      />

      <div className="favorite-tag-box" onClick={() => tagInputRef.current?.focus()}>
        {tags.map((tag) => (
          <span key={tag} className="favorite-tag-chip" title={tag}>
            <span className="favorite-tag-chip-text">{tag}</span>
            <button
              type="button"
              className="favorite-tag-chip-remove"
              aria-label={t("favorites_tag_remove").replace("{tag}", tag)}
              disabled={saving}
              onClick={(event) => {
                event.stopPropagation();
                setTags((current) => current.filter((value) => value !== tag));
              }}
            >
              <X size={10} />
            </button>
          </span>
        ))}
        <input
          ref={tagInputRef}
          type="text"
          className="favorite-tag-input"
          placeholder={tags.length === 0 ? t("favorites_tags_placeholder") : t("favorites_tags_placeholder_more")}
          aria-label={t("favorites_tags_label")}
          value={tagDraft}
          disabled={saving || atLimit}
          onChange={handleTagChange}
          onKeyDown={handleTagKeyDown}
        />
      </div>

      {suggestions.length > 0 && (
        <div className="favorite-tag-suggestions" role="group" aria-label={t("favorites_tags_existing")}>
          {suggestions.map((tag) => (
            <button
              key={tag}
              type="button"
              className="favorite-tag-suggestion"
              title={tag}
              aria-label={t("favorites_tag_add").replace("{tag}", tag)}
              disabled={saving || atLimit}
              onClick={() => addTags(tags, [tag])}
            >
              + {tag}
            </button>
          ))}
        </div>
      )}

      {mode === "text" && (
        <textarea
          ref={textAreaRef}
          autoFocus={initialFocus === "content"}
          className="glass-field favorite-textarea"
          placeholder={t("favorites_placeholder")}
          aria-label={t("favorites_content_label")}
          value={text}
          disabled={saving}
          onChange={(event) => setText(event.target.value)}
        />
      )}

      <div className="favorite-editor-actions">
        <button type="button" className="glass-button is-compact" disabled={saving} onClick={onCancel}>
          {t("cancel")}
        </button>
        <button type="button" className="glass-button is-compact is-primary" disabled={!canSave} onClick={submit}>
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
  existingTags,
  saving,
  pushToast,
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
  existingTags: string[];
  saving: boolean;
  pushToast: (message: string, duration?: number) => void;
  onPaste: () => void;
  onEdit: () => void;
  onSaveEdit: (mode: EditorMode, value: EditorValue) => void;
  onCancelEdit: () => void;
  onDelete: () => void;
  onDragEnd: () => void;
}) => {
  const dragControls = useDragControls();
  const isText = isEditableEntry(item.content_type);
  const mode: EditorMode = isText ? "text" : "media";
  const title = item.title?.trim() ?? "";
  const tags = item.tags ?? [];
  const hasTitle = title.length > 0;
  const hasTags = tags.length > 0;
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
        <FavoriteEditor
          t={t}
          mode={mode}
          item={item}
          initialTitle={title}
          initialTags={tags}
          initialText={item.content}
          initialFocus="title"
          existingTags={existingTags}
          saving={saving}
          pushToast={pushToast}
          onSave={(value) => onSaveEdit(mode, value)}
          onCancel={onCancelEdit}
        />
      ) : (
        <div
          className={`favorite-item${item.content_type === "image" ? " is-image" : ""}${hasTitle ? " has-title" : ""}${hasTags ? " has-tags" : ""}`}
          onClick={onPaste}
        >
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
          <div className="favorite-body">
            {hasTitle && (
              <span className="favorite-item-title" title={title}>
                {title}
              </span>
            )}
            {isText ? (
              <span
                className={`favorite-text${item.content_type === "code" ? " is-code" : ""}${hasTitle ? " has-title" : ""}`}
              >
                {item.preview || item.content}
              </span>
            ) : (
              <FavoriteMedia item={item} t={t} hasTitle={hasTitle} />
            )}
            {hasTags && (
              <span className="favorite-card-tags" aria-label={tags.join(", ")}>
                {tags.slice(0, MAX_CARD_VISIBLE_TAGS).map((tag) => (
                  <span key={tag} className="favorite-card-tag" title={tag}>
                    {tag}
                  </span>
                ))}
                {tags.length > MAX_CARD_VISIBLE_TAGS && (
                  <span className="favorite-card-tag is-more" title={tags.slice(MAX_CARD_VISIBLE_TAGS).join(", ")}>
                    +{tags.length - MAX_CARD_VISIBLE_TAGS}
                  </span>
                )}
              </span>
            )}
          </div>
          <span className="favorite-actions">
            <button
              type="button"
              className="card-action"
              title={t("edit")}
              aria-label={t("edit")}
              data-favorite-edit={item.id}
              onClick={(event) => {
                event.stopPropagation();
                onEdit();
              }}
            >
              <Pencil size={12} />
            </button>
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

const FavoritesPanel = ({ t, pushToast, onPasteFailed, onEditingChange, cancelEditRef }: FavoritesPanelProps) => {
  const [items, setItems] = useState<FavoriteItem[]>([]);
  const [adding, setAdding] = useState(false);
  const [tab, setTab] = useState<FavoriteTab>("text");
  const [editingId, setEditingId] = useState<number | null>(null);
  const [selectedTag, setSelectedTag] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [overflow, setOverflow] = useState({ left: false, right: false });
  const panelRef = useRef<HTMLDivElement>(null);
  const addButtonRef = useRef<HTMLButtonElement>(null);
  const tagBarRef = useRef<HTMLDivElement>(null);
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

  // Suggestions in the editor come from both tabs; the filter bar from this one.
  const allTags = useMemo(() => collectSortedTags(items), [items]);
  const tabTags = useMemo(() => collectSortedTags(items.filter((item) => inTab(item, tab))), [items, tab]);
  // The row being edited stays visible while its editor is open, even when a
  // filter change would hide it, so unsaved edits are never lost.
  const visible = items.filter(
    (item) => matchesVisible(item, tab, selectedTag) || (item.id === editingId && inTab(item, tab))
  );
  const isEditing = adding || editingId !== null;

  // A tag with nothing left under it in this tab falls back to 全部.
  useEffect(() => {
    if (selectedTag !== null && !tabTags.includes(selectedTag)) setSelectedTag(null);
  }, [selectedTag, tabTags]);

  // The row being edited was deleted elsewhere (or its tab was left).
  useEffect(() => {
    if (editingId !== null && !visible.some((item) => item.id === editingId)) setEditingId(null);
  }, [editingId, visible]);

  useEffect(() => {
    onEditingChange?.(isEditing);
  }, [isEditing, onEditingChange]);

  useEffect(() => () => onEditingChange?.(false), [onEditingChange]);

  /** Closes the editor and gives focus back to what opened it. */
  const closeEditor = useCallback(() => {
    const editedId = editingId;
    setAdding(false);
    setEditingId(null);
    requestAnimationFrame(() => {
      const target =
        editedId === null
          ? addButtonRef.current
          : panelRef.current?.querySelector<HTMLElement>(`[data-favorite-edit="${editedId}"]`);
      target?.focus({ preventScroll: true });
    });
  }, [editingId]);

  useEffect(() => {
    if (!cancelEditRef) return;
    cancelEditRef.current = () => {
      if (!isEditing) return false;
      if (!saving) closeEditor();
      return true;
    };
    return () => {
      cancelEditRef.current = null;
    };
  }, [cancelEditRef, closeEditor, isEditing, saving]);

  const syncTagBarOverflow = useCallback(() => {
    const bar = tagBarRef.current;
    const left = bar !== null && bar.scrollLeft > 1;
    const right = bar !== null && bar.scrollLeft + bar.clientWidth < bar.scrollWidth - 1;
    setOverflow((current) => (current.left === left && current.right === right ? current : { left, right }));
  }, []);

  const hasTagBar = tabTags.length > 0;
  useLayoutEffect(() => syncTagBarOverflow(), [syncTagBarOverflow, tabTags]);

  useEffect(() => {
    const bar = tagBarRef.current;
    if (!hasTagBar || !bar || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(syncTagBarOverflow);
    observer.observe(bar);
    return () => observer.disconnect();
  }, [hasTagBar, syncTagBarOverflow]);

  const handleTagBarWheel = (event: WheelEvent<HTMLDivElement>) => {
    if (Math.abs(event.deltaY) > Math.abs(event.deltaX)) event.currentTarget.scrollLeft += event.deltaY;
  };

  const pickTag = (tag: string | null, button: HTMLElement) => {
    setSelectedTag(tag);
    button.scrollIntoView({ inline: "nearest", block: "nearest" });
  };

  const switchTab = (id: FavoriteTab) => {
    setTab(id);
    setEditingId(null);
    if (id !== "text") setAdding(false);
  };

  const fail = (error: unknown) => pushToast(t("save_failed") + String(error), 3000);

  const saveNew = ({ title, tags, text }: EditorValue) => {
    setSaving(true);
    invoke("add_favorite_text", { text, title, tags })
      .then(() => {
        // Keep the new favorite in sight when the filter no longer covers it.
        if (selectedTag !== null && !tags.includes(selectedTag)) setSelectedTag(null);
        closeEditor();
      })
      .catch((error) => {
        if (String(error).includes("favorite_duplicate")) pushToast(t("favorites_duplicate"), 3000);
        else fail(error);
      })
      .finally(() => setSaving(false));
  };

  const saveEdit = (item: FavoriteItem, mode: EditorMode, { title, tags, text }: EditorValue) => {
    setSaving(true);
    const saved =
      mode === "text"
        ? invoke("update_favorite_text", { id: item.id, text, title, tags })
        : invoke("update_favorite_meta", { id: item.id, title, tags });
    saved
      .then(closeEditor)
      .catch(fail)
      .finally(() => setSaving(false));
  };

  return (
    <div className="side-panel-body" ref={panelRef}>
      <div className="panel-title" onMouseDown={startWindowDrag}>
        <h2>{t("favorites_title")}</h2>
        <button
          ref={addButtonRef}
          type="button"
          className="glass-button is-compact"
          onClick={() => {
            // Typed favorites are text, so they belong to 常用语.
            setTab("text");
            setEditingId(null);
            setAdding(true);
          }}
        >
          <Plus size={13} />
          {t("favorites_add")}
        </button>
      </div>

      <div className="favorite-toolbar">
        <div className="favorite-tabs" role="tablist">
          {FAVORITE_TABS.map((id) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={tab === id}
              className={`type-tab${tab === id ? " is-active" : ""}`}
              onClick={() => switchTab(id)}
            >
              {t(`favorites_tab_${id}`)}
            </button>
          ))}
        </div>

        {hasTagBar && (
          <div
            ref={tagBarRef}
            className={`favorite-tag-bar${overflow.left ? " has-overflow-left" : ""}${overflow.right ? " has-overflow-right" : ""}`}
            role="group"
            aria-label={t("favorites_tag_filter")}
            onWheel={handleTagBarWheel}
            onScroll={syncTagBarOverflow}
          >
            <button
              type="button"
              className={`favorite-tag-pill${selectedTag === null ? " is-active" : ""}`}
              aria-pressed={selectedTag === null}
              onClick={(event) => pickTag(null, event.currentTarget)}
            >
              {t("favorites_tags_all")}
            </button>
            {tabTags.map((tag) => {
              const active = selectedTag === tag;
              return (
                <button
                  key={tag}
                  type="button"
                  className={`favorite-tag-pill${active ? " is-active" : ""}`}
                  aria-pressed={active}
                  title={tag}
                  onClick={(event) => pickTag(active ? null : tag, event.currentTarget)}
                >
                  {tag}
                </button>
              );
            })}
          </div>
        )}
      </div>

      <div className="panel-scroll">
        {adding && (
          <FavoriteEditor
            t={t}
            mode="text"
            initialTitle=""
            initialTags={selectedTag !== null && tabTags.includes(selectedTag) ? [selectedTag] : []}
            initialText=""
            initialFocus="content"
            existingTags={allTags}
            saving={saving}
            pushToast={pushToast}
            onCancel={closeEditor}
            onSave={saveNew}
          />
        )}

        {visible.length === 0 && !adding ? (
          <div className="empty-state">
            <p className="empty-title">{selectedTag ? t("favorites_empty_tag") : t(`favorites_empty_${tab}`)}</p>
            {selectedTag ? (
              <button type="button" className="link-button" onClick={() => setSelectedTag(null)}>
                {t("favorites_show_all")}
              </button>
            ) : (
              <p className="empty-hint">{t(`favorites_empty_hint_${tab}`)}</p>
            )}
          </div>
        ) : (
          <Reorder.Group
            axis="y"
            values={visible}
            onReorder={(reordered) => {
              // While an editor is open the list may show a row outside the
              // filter; reordering then would misplace slots, so it waits.
              if (isEditing) return;
              setItems((all) => mergeOrder(all, tab, selectedTag, reordered));
            }}
            className={`favorite-list${tab === "media" ? " is-media" : ""}`}
          >
            {visible.map((item) => (
              <FavoriteRow
                key={item.id}
                item={item}
                t={t}
                editing={editingId === item.id}
                existingTags={allTags}
                saving={saving}
                pushToast={pushToast}
                onPaste={() => {
                  invoke("paste_favorite", { id: item.id }).catch(onPasteFailed);
                }}
                onEdit={() => {
                  setAdding(false);
                  setEditingId(item.id);
                }}
                onCancelEdit={closeEditor}
                onSaveEdit={(mode, value) => saveEdit(item, mode, value)}
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
