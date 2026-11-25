import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import AppHeader from "./features/app/AppHeader";
import SearchBox, { type SearchBoxHandle } from "./features/app/SearchBox";
import TypeTabs from "./features/app/TypeTabs";
import FixedPositionBanner from "./features/app/FixedPositionBanner";
import { displayNameOf, useSettings } from "./features/app/useSettings";
import { useOverlays } from "./features/app/useOverlays";
import { useHistory } from "./features/app/useHistory";
import { useSidePanel } from "./features/app/useSidePanel";
import { usePasteActions } from "./features/app/usePasteActions";
import { useKeyboard } from "./features/app/useKeyboard";
import ClipboardCard from "./features/clipboard/ClipboardCard";
import ClipboardList, { type ClipboardListHandle } from "./features/clipboard/ClipboardList";
import FavoritesPanel from "./features/favorites/FavoritesPanel";
import EditPanel from "./features/edit/EditPanel";
import TranslatePanel from "./features/translate/TranslatePanel";
import type { TranslationConfig } from "./features/translate/config";
import BatchPanel from "./features/batch/BatchPanel";
import {
  allVisibleSelected,
  applyEntryStates,
  batchDeleteMessage,
  deletionRequest,
  entryKey,
  indexOfEntry,
  refreshSelection,
  selectAllVisible,
  selectionCounts,
  toggleSelectAllVisible,
  type EntryKey,
  type EntryState,
} from "./features/batch/batchSelection";
import PreviewPanel from "./features/preview/PreviewPanel";
import { useSpacePreview } from "./features/preview/useSpacePreview";
import SettingsPage from "./features/settings/components/SettingsPage";
import PermissionOnboarding from "./shared/components/PermissionOnboarding";
import { ConfirmDialog, ToastContainer } from "./shared/components/Overlays";
import { useContextMenuBlock } from "./shared/hooks/useContextMenuBlock";
import { useFocusCoordinator } from "./shared/hooks/useFocusCoordinator";
import { isTextEntry, type TypeFilter } from "./shared/lib/clipboardTypes";
import { showNativeMenu, type NativeMenuItem } from "./shared/lib/nativeMenu";
import { isTauriRuntime } from "./shared/lib/tauriRuntime";
import type { ClipboardEntry, WindowPinMode } from "./shared/types";

/** How much the CSS tint shows on systems with frosted glass only. */
const FROSTED_TINT_MAX = 0.72;
/** Longer than the side panel's animation, so its in-between widths aren't saved. */
const WINDOW_SIZE_SAVE_DELAY_MS = 500;
const PIN_MODE_KEY = "app.window_pin_mode";

const normalizePinMode = (value?: string): WindowPinMode => value === "normal" ? "normal" : "always_on_top";

const App = () => {
  useFocusCoordinator();
  useContextMenuBlock();

  const settings = useSettings();
  const { t, flag, values, ready, save, remember } = settings;
  const { toasts, pushToast, confirmDialog, openConfirm, closeConfirm } = useOverlays(t);

  const [showSettings, setShowSettings] = useState(false);
  const [draft, setDraft] = useState("");
  const [search, setSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState<TypeFilter | null>(null);
  // The entry picked with the arrow keys, not its row: rows shift as copies come and go.
  const [selectedKey, setSelectedKey] = useState<EntryKey | null>(null);
  const [isWindowPinned, setIsWindowPinned] = useState(false);
  const pinMode = normalizePinMode(values[PIN_MODE_KEY]);
  const pinModeRef = useRef(pinMode);
  useEffect(() => { pinModeRef.current = pinMode; }, [pinMode]);
  const windowPinnedRef = useRef(isWindowPinned);
  const pinModeChange = useRef(Promise.resolve());
  const updateWindowPinned = useCallback((pinned: boolean) => {
    windowPinnedRef.current = pinned;
    setIsWindowPinned(pinned);
  }, []);
  const [settingFixedPosition, setSettingFixedPosition] = useState(false);
  const [editing, setEditing] = useState<ClipboardEntry | null>(null);
  const [batchItems, setBatchItems] = useState<ClipboardEntry[]>([]);
  const [batchDeleting, setBatchDeleting] = useState(false);
  const [previewEntry, setPreviewEntry] = useState<ClipboardEntry | null>(null);
  const [hasNativeGlass, setHasNativeGlass] = useState(true);
  const [translationConfig, setTranslationConfig] = useState<TranslationConfig | null>(null);
  const canTranslate = translationConfig?.available ?? false;

  const pushToastRef = useRef(pushToast);
  pushToastRef.current = pushToast;
  const tRef = useRef(t);
  tRef.current = t;

  const searchRef = useRef<SearchBoxHandle | null>(null);
  const listRef = useRef<ClipboardListHandle | null>(null);

  const { items, hasMore, loading, loaded, settled, loadMore, refetch, removeItems } = useHistory(
    search,
    typeFilter,
    ready
  );
  const { panel, open: openPanel, close: closePanel } = useSidePanel();
  const itemsRef = useRef(items);
  itemsRef.current = items;
  const batchItemsRef = useRef(batchItems);
  batchItemsRef.current = batchItems;

  const selectedIndex = useMemo(() => indexOfEntry(items, selectedKey), [items, selectedKey]);
  const setSelectedIndex = useCallback((index: number | null) => {
    const entry = index === null ? undefined : itemsRef.current[index];
    setSelectedKey(entry ? entryKey(entry) : null);
  }, []);
  // The picked entry is gone (deleted, or not in this list): nothing stays picked.
  useEffect(() => {
    if (selectedKey && selectedIndex === null) setSelectedKey(null);
  }, [selectedIndex, selectedKey]);

  const deleteAfterPaste = flag("app.delete_after_paste");
  const moveToTopAfterPaste = flag("app.move_to_top_after_paste", true);

  const afterPaste = useCallback(() => {
    setDraft("");
    setSearch("");
    setSelectedKey(null);
  }, []);

  const { paste, pasteImageText, failed } = usePasteActions({
    t,
    pushToast,
    deleteAfterPaste,
    moveToTopAfterPaste,
    afterPaste,
  });

  const batchMode = panel.kind === "batch";

  const toggleBatch = useCallback((item: ClipboardEntry) => {
    setBatchItems((prev) =>
      prev.some((entry) => entry.id === item.id) ? prev.filter((entry) => entry.id !== item.id) : [...prev, item]
    );
  }, []);

  const addFavorite = useCallback(
    (item: ClipboardEntry) => {
      invoke("add_favorite_from_entry", { id: item.id })
        .then(() => pushToast(t("add_favorite_ok")))
        .catch((error) => {
          const text = String(error);
          if (text.includes("favorite_too_large")) pushToast(t("favorite_too_large"), 3000);
          else if (text.includes("file_missing")) pushToast(t("file_missing"), 3000);
          else pushToast(t("save_failed") + text, 3000);
        });
    },
    [pushToast, t]
  );

  // Batch mode has no keyboard pick: a highlighted card would look ticked.
  const startBatch = useCallback(
    (item: ClipboardEntry) => {
      setEditing(null);
      setSelectedKey(null);
      setBatchItems([item]);
      void openPanel("batch");
    },
    [openPanel]
  );

  // Cmd+A on the list.
  const startBatchSelectAll = useCallback(() => {
    setEditing(null);
    setSelectedKey(null);
    setBatchItems(selectAllVisible([], itemsRef.current));
    void openPanel("batch");
  }, [openPanel]);

  const toggleSelectAll = useCallback(() => {
    setBatchItems((prev) => toggleSelectAllVisible(prev, itemsRef.current));
  }, []);

  // The ticked entries follow the list (pin state), so the confirmation counts right.
  useEffect(() => {
    setBatchItems((prev) => refreshSelection(prev, items));
  }, [items]);

  // 分享: the system share menu, anchored to the card (right-click menu's last item).
  const shareEntry = useCallback(
    (item: ClipboardEntry) => {
      const card = document.getElementById(`clipboard-item-${item.id}`);
      const anchor = card?.getBoundingClientRect() ?? new DOMRect(0, 0, 0, 0);
      invoke("share_content", {
        id: item.id,
        content: item.content,
        contentType: item.content_type,
        anchor: { x: anchor.left, y: anchor.top, width: anchor.width, height: anchor.height },
      }).catch((error) => {
        const text = String(error);
        pushToast(text.includes("file_missing") ? t("file_missing") : t("share_failed") + text, 3000);
      });
    },
    [pushToast, t]
  );

  const showCardMenu = useCallback(
    (item: ClipboardEntry) => {
      // 打开: the entry in its default app (set per type in Settings), first for every type.
      const open: NativeMenuItem = {
        text: t("open"),
        action: () =>
          void invoke("open_content", { id: item.id, content: item.content, contentType: item.content_type }).catch(
            (error) => {
              const text = String(error);
              pushToast(text.includes("file_missing") ? t("file_missing") : t("open_failed") + text, 3000);
            }
          ),
      };
      const batch: NativeMenuItem = { text: t("batch_paste"), action: () => startBatch(item) };
      const favorite = (key: string): NativeMenuItem => ({ text: t(key), action: () => addFavorite(item) });
      let menu: NativeMenuItem[];
      if (isTextEntry(item.content_type)) {
        menu = [
          {
            text: t("paste_format"),
            children: [
              { text: t("paste_format_original"), action: () => void paste(item, "original") },
              { text: t("paste_format_plain"), action: () => void paste(item, "plain") },
              { text: t("paste_format_markdown"), action: () => void paste(item, "markdown") },
            ],
          },
          favorite("add_favorite_text"),
          batch,
        ];
      } else if (item.content_type === "image") {
        // 粘贴为文字 lives on the card's own 文 button now.
        menu = [favorite("add_favorite_image"), batch];
      } else if (item.content_type === "video") {
        menu = [favorite("add_favorite_video"), batch];
      } else {
        menu = [favorite("add_favorite_file"), batch];
      }
      const share: NativeMenuItem = { text: t("share"), action: () => shareEntry(item) };
      void showNativeMenu([open, ...menu, share]);
    },
    [addFavorite, paste, pushToast, shareEntry, startBatch, t]
  );

  const pasteImageTextEntry = useCallback(
    (item: ClipboardEntry) => void pasteImageText(item),
    [pasteImageText]
  );

  const editEntry = useCallback(
    (item: ClipboardEntry) => {
      setEditing(item);
      void openPanel("edit");
    },
    [openPanel]
  );

  // 翻译 shares 编辑's entry slot: only one panel is open at a time.
  const translateEntry = useCallback(
    (item: ClipboardEntry) => {
      setEditing(item);
      void openPanel("translate");
    },
    [openPanel]
  );

  const togglePin = useCallback(
    (item: ClipboardEntry) => {
      invoke("toggle_clipboard_pin", { id: item.id, isPinned: !item.is_pinned }).catch((error) =>
        pushToast(t("pin_failed") + String(error), 3000)
      );
    },
    [pushToast, t]
  );

  const deleteEntry = useCallback(
    (item: ClipboardEntry) => {
      removeItems([item.id]);
      invoke("delete_clipboard_entry", { id: item.id }).catch((error) => {
        pushToast(t("delete_failed") + String(error), 3000);
        void refetch(true);
      });
    },
    [pushToast, refetch, removeItems, t]
  );

  const closeSidePanel = useCallback(() => {
    setEditing(null);
    setBatchItems([]);
    setPreviewEntry(null);
    void closePanel();
  }, [closePanel]);

  const batchDeletingRef = useRef(false);
  const deleteBatch = useCallback(
    async (frozen: EntryState[]) => {
      batchDeletingRef.current = true;
      setBatchDeleting(true);
      try {
        const count = await invoke<number>("delete_clipboard_entries", { items: frozen });
        removeItems(frozen.map((item) => item.id));
        closeSidePanel();
        pushToast(t("batch_deleted").replace("{count}", String(count)));
      } catch (error) {
        if (String(error).includes("selection_changed")) {
          // Something ticked was deleted, replaced or (un)pinned meanwhile:
          // bring the ticks up to date and let the user confirm again.
          const states = await invoke<EntryState[]>("get_clipboard_entry_states", {
            ids: frozen.map((item) => item.id),
          }).catch(() => null);
          if (states) setBatchItems((prev) => applyEntryStates(prev, states));
          pushToast(t("batch_selection_changed"), 3000);
        } else {
          pushToast(t("delete_failed") + String(error), 3000);
        }
        void refetch(true);
      } finally {
        batchDeletingRef.current = false;
        setBatchDeleting(false);
      }
    },
    [closeSidePanel, pushToast, refetch, removeItems, t]
  );

  const requestBatchDelete = useCallback(() => {
    const selection = batchItemsRef.current;
    if (selection.length === 0 || batchDeletingRef.current) return;
    // Frozen now: what the dialog counts is exactly what gets deleted.
    const frozen = deletionRequest(selection);
    openConfirm({
      title: t("batch_delete_title"),
      message: batchDeleteMessage(t, selectionCounts(selection, itemsRef.current)),
      danger: true,
      confirmLabel: t("delete"),
      onConfirm: () => {
        closeConfirm();
        void deleteBatch(frozen);
      },
    });
  }, [closeConfirm, deleteBatch, openConfirm, t]);

  // A preview borrows the panel area: 常用 comes back when it ends.
  const favoritesUnderPreview = useRef(false);

  const showPreview = useCallback(
    (entry: ClipboardEntry) => {
      favoritesUnderPreview.current = panel.kind === "favorites";
      setPreviewEntry(entry);
      void openPanel("preview");
    },
    [openPanel, panel.kind]
  );

  const hidePreview = useCallback(() => {
    setPreviewEntry(null);
    void (favoritesUnderPreview.current ? openPanel("favorites") : closePanel());
  }, [closePanel, openPanel]);

  const toggleFavorites = useCallback(() => {
    if (panel.kind === "favorites") {
      closeSidePanel();
      return;
    }
    setEditing(null);
    setBatchItems([]);
    void openPanel("favorites");
  }, [closeSidePanel, openPanel, panel.kind]);

  const leaveSettings = useCallback(() => setShowSettings(false), []);

  // 常用's editor: Esc closes it before the panel, so unsaved text isn't lost.
  const [favoriteEditing, setFavoriteEditing] = useState(false);
  const favoriteCancelEditRef = useRef<(() => boolean) | null>(null);

  const dismiss = useCallback(() => {
    if (settingFixedPosition) return true;
    if (panel.kind === "favorites" && favoriteCancelEditRef.current?.()) return true;
    if (panel.kind) {
      closeSidePanel();
      return true;
    }
    if (showSettings) {
      leaveSettings();
      return true;
    }
    if (selectedKey !== null) {
      setSelectedKey(null);
      return true;
    }
    if (draft || search) {
      setDraft("");
      setSearch("");
      return true;
    }
    return false;
  }, [closeSidePanel, draft, leaveSettings, panel.kind, search, selectedKey, settingFixedPosition, showSettings]);

  // 编辑, 翻译 and 常用's editor take typing: the list's keys stay off while they are open.
  const typingInPanel =
    panel.kind === "edit" || panel.kind === "translate" || (panel.kind === "favorites" && favoriteEditing);

  useKeyboard({
    items,
    enabled: !showSettings && !batchMode && !typingInPanel,
    arrowKeys: flag("app.arrow_key_selection", true),
    selectedIndex,
    setSelectedIndex,
    typeFilter,
    setTypeFilter,
    paste: (entry) => void paste(entry),
    remove: deleteEntry,
    focusSearch: () => searchRef.current?.focus(),
    dismiss,
    panel: panel.kind,
    blocked: showSettings || settingFixedPosition,
    batchCount: batchItems.length,
    canSelectAll: settled,
    startBatchSelectAll,
    toggleSelectAll,
    requestBatchDelete,
  });

  useSpacePreview({
    enabled: !showSettings && !settingFixedPosition && !typingInPanel && !batchMode,
    items,
    selectedIndex,
    onOpen: showPreview,
    onClose: hidePreview,
  });

  // The list keeps its own width while the side panel opens or shrinks away.
  const sidePanelResizingWindow = panel.listWidth !== null;
  useEffect(() => {
    if (!isTauriRuntime() || !ready) return;
    let timer: number | null = null;
    const onResize = () => {
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        if (sidePanelResizingWindow) return;
        const width = String(Math.round(window.innerWidth));
        const height = String(Math.round(window.innerHeight));
        invoke("save_setting", { key: "app.window_logical_width", value: width }).catch(console.error);
        invoke("save_setting", { key: "app.window_logical_height", value: height }).catch(console.error);
      }, WINDOW_SIZE_SAVE_DELAY_MS);
    };
    window.addEventListener("resize", onResize);
    return () => {
      if (timer !== null) window.clearTimeout(timer);
      window.removeEventListener("resize", onResize);
    };
  }, [ready, sidePanelResizingWindow]);

  // A new search or tab starts with nothing picked.
  useEffect(() => {
    setSelectedKey(null);
  }, [search, typeFilter]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const subscriptions = [
      // Each opening starts unfiltered: a filter is for digging one item out.
      listen("clipboard-shown", () => {
        setTypeFilter(null);
        setSelectedKey(null);
        listRef.current?.scrollToTop();
      }),
      listen<boolean>("window-pinned-changed", ({ payload }) => updateWindowPinned(payload)),
      // A close (hotkey, red button) was refused because the window is pinned.
      listen("window-pinned-close-blocked", () => {
        if (pinModeRef.current === "always_on_top") pushToastRef.current(tRef.current("pinned_esc_hint"));
      }),
      listen("side-panel-collapsed", () => {
        setEditing(null);
        setBatchItems([]);
        setPreviewEntry(null);
      }),
    ];
    return () => {
      subscriptions.forEach((subscription) => void subscription.then((off) => off()));
    };
  }, [updateWindowPinned]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    invoke<boolean>("supports_liquid_glass").then(setHasNativeGlass).catch(() => setHasNativeGlass(true));
    let alive = true;
    invoke<TranslationConfig>("get_translation_config").then((config) => {
      if (alive) setTranslationConfig(config);
    }).catch(() => {
      if (alive) setTranslationConfig(null);
    });
    return () => { alive = false; };
  }, []);

  const [glassLevelLive, setGlassLevelLive] = useState<number | null>(null);
  const glassLevel = glassLevelLive ?? Number(values["app.glass_level"] ?? 35);

  // macOS 26+ tints the native glass; before that this layer does.
  const shellStyle = useMemo(
    () =>
      ({
        "--frosted-tint": hasNativeGlass ? 0 : (glassLevel / 100) * FROSTED_TINT_MAX,
      }) as CSSProperties,
    [glassLevel, hasNativeGlass]
  );

  const showPinnedToast = (mode: WindowPinMode) =>
    pushToast(t(mode === "normal" ? "pinned_normal_hint" : "pinned_esc_hint"));

  const togglePinned = () => {
    const pinned = !windowPinnedRef.current;
    updateWindowPinned(pinned);
    invoke("set_window_pinned", { pinned }).catch(console.error);
    if (pinned) showPinnedToast(pinModeRef.current);
  };

  const selectPinMode = (mode: WindowPinMode, fromHeader: boolean) => {
    pinModeChange.current = pinModeChange.current.then(async () => {
      const changed = mode !== pinModeRef.current;
      if (changed) {
        await invoke("set_window_pin_mode", { mode });
        pinModeRef.current = mode;
        remember(PIN_MODE_KEY, mode);
      }
      if (!fromHeader) return;
      const shouldPin = !windowPinnedRef.current;
      if (shouldPin) {
        await invoke("set_window_pinned", { pinned: true });
        updateWindowPinned(true);
      }
      if (shouldPin || changed) showPinnedToast(mode);
    }).catch((error) => pushToast(t("save_failed") + String(error), 3000));
  };

  const clearHistory = () => {
    openConfirm({
      title: t("clear_confirm_title"),
      message: t("clear_confirm_message"),
      danger: true,
      confirmLabel: t("clear_confirm_ok"),
      onConfirm: () => {
        closeConfirm();
        invoke("clear_clipboard_history").catch((error) => pushToast(t("clear_failed") + String(error), 3000));
      },
    });
  };

  const resetSettings = () => {
    openConfirm({
      title: t("reset_defaults"),
      message: t("reset_confirm"),
      danger: true,
      confirmLabel: t("reset_ok"),
      onConfirm: () => {
        closeConfirm();
        invoke("reset_settings")
          .then(() => window.location.reload())
          .catch((error) => pushToast(t("reset_failed") + String(error), 3000));
      },
    });
  };

  const beginFixedPosition = () => {
    invoke("begin_fixed_position_setup").catch(console.error);
    setSettingFixedPosition(true);
  };

  const batchIds = useMemo(() => new Set(batchItems.map((item) => item.id)), [batchItems]);
  const showSourceIcon = flag("app.show_source_app_icon", true);

  // No highlighted card in batch mode: it would look ticked.
  const selectedId = !batchMode && selectedIndex !== null ? items[selectedIndex]?.id : undefined;
  const allSelected = useMemo(() => allVisibleSelected(batchItems, items), [batchItems, items]);

  // Stable between keystrokes in the search box, so typing doesn't re-render the list.
  const renderItem = useCallback(
    (item: ClipboardEntry) => (
      <ClipboardCard
        item={item}
        t={t}
        selected={item.id === selectedId}
        showSourceIcon={showSourceIcon}
        batchMode={batchMode}
        batchChecked={batchIds.has(item.id)}
        onPaste={paste as (entry: ClipboardEntry) => void}
        onToggleBatch={toggleBatch}
        onContextMenu={showCardMenu}
        onEdit={editEntry}
        onTogglePin={togglePin}
        onDelete={deleteEntry}
        onPasteAsText={pasteImageTextEntry}
        onTranslate={canTranslate ? translateEntry : undefined}
        searchTerm={search}
      />
    ),
    [
      batchIds,
      batchMode,
      canTranslate,
      deleteEntry,
      editEntry,
      paste,
      pasteImageTextEntry,
      search,
      selectedId,
      showCardMenu,
      showSourceIcon,
      t,
      toggleBatch,
      togglePin,
      translateEntry,
    ]
  );
  const loadMoreItems = useCallback(() => void loadMore(), [loadMore]);

  if (!ready) return <div className="app-shell" />;

  return (
    <div className="app-shell" style={shellStyle}>
      <div className="app-main" style={panel.listWidth ? { flex: `0 0 ${panel.listWidth}px` } : undefined}>
        <AppHeader
          t={t}
          displayName={displayNameOf(values)}
          showSettings={showSettings}
          isWindowPinned={isWindowPinned}
          pinMode={pinMode}
          favoritesOpen={panel.kind === "favorites"}
          onBack={leaveSettings}
          onToggleFavorites={toggleFavorites}
          onTogglePinned={togglePinned}
          onSelectPinMode={(mode) => selectPinMode(mode, true)}
          onCloseWindow={() => void invoke("hide_window_cmd").catch(console.error)}
          onClearHistory={clearHistory}
          onOpenSettings={() => {
            closeSidePanel();
            setShowSettings(true);
          }}
        />

        {showSettings ? (
          <SettingsPage
            settings={settings}
            pinMode={pinMode}
            onSelectPinMode={(mode) => selectPinMode(mode, false)}
            translationConfig={translationConfig}
            onTranslationSaved={setTranslationConfig}
            pushToast={pushToast}
            onSetFixedPosition={beginFixedPosition}
            onResetSettings={resetSettings}
            onGlassLevel={setGlassLevelLive}
          />
        ) : (
          <>
            <div className="list-toolbar">
              {flag("app.show_search_box", true) && (
                <SearchBox
                  ref={searchRef}
                  t={t}
                  value={draft}
                  onChange={setDraft}
                  onSubmit={(value) => setSearch(value.trim())}
                  appliedValue={search}
                />
              )}
              {flag("app.show_type_tabs", true) && <TypeTabs t={t} value={typeFilter} onChange={setTypeFilter} />}
            </div>
            <div className="list-area">
              <ClipboardList
                ref={listRef}
                t={t}
                items={items}
                loaded={loaded}
                searching={search.length > 0 || typeFilter !== null}
                hasMore={hasMore}
                loading={loading}
                selectedIndex={selectedIndex}
                onLoadMore={loadMoreItems}
                renderItem={renderItem}
              />
            </div>
          </>
        )}
      </div>

      {panel.kind && (
        <aside className="side-panel">
          {panel.kind === "favorites" && (
            <FavoritesPanel
              t={t}
              pushToast={pushToast}
              onPasteFailed={failed}
              onEditingChange={setFavoriteEditing}
              cancelEditRef={favoriteCancelEditRef}
            />
          )}
          {panel.kind === "edit" && editing && (
            <EditPanel
              key={editing.id}
              t={t}
              entry={editing}
              deleteAfterPaste={deleteAfterPaste}
              moveToTopAfterPaste={moveToTopAfterPaste}
              pushToast={pushToast}
              onPasteFailed={failed}
              onClose={closeSidePanel}
            />
          )}
          {panel.kind === "translate" && editing && translationConfig && (
            <TranslatePanel
              key={editing.id}
              t={t}
              entry={editing}
              config={translationConfig}
              deleteAfterPaste={deleteAfterPaste}
              moveToTopAfterPaste={moveToTopAfterPaste}
              pushToast={pushToast}
              onPasteFailed={failed}
              onClose={closeSidePanel}
            />
          )}
          {panel.kind === "preview" && previewEntry && (
            <PreviewPanel key={previewEntry.id} t={t} entry={previewEntry} />
          )}
          {panel.kind === "batch" && (
            <BatchPanel
              t={t}
              items={batchItems}
              allVisibleSelected={allSelected}
              canSelectAll={settled && items.length > 0}
              hasMore={hasMore}
              deleting={batchDeleting}
              onToggleSelectAll={toggleSelectAll}
              onDeleteSelected={requestBatchDelete}
              onReorder={setBatchItems}
              onRemove={toggleBatch}
              deleteAfterPaste={deleteAfterPaste}
              moveToTopAfterPaste={moveToTopAfterPaste}
              onPasteFailed={failed}
              onClose={closeSidePanel}
            />
          )}
        </aside>
      )}

      {settingFixedPosition && (
        <FixedPositionBanner
          t={t}
          onDone={(saved) => {
            setSettingFixedPosition(false);
            if (saved) pushToast(t("fixed_banner_saved"));
          }}
        />
      )}

      <ToastContainer toasts={toasts} />
      <ConfirmDialog
        state={confirmDialog}
        confirmLabel={confirmDialog.confirmLabel ?? t("confirm")}
        cancelLabel={t("cancel")}
        onClose={closeConfirm}
      />
      <PermissionOnboarding
        ready={ready}
        t={t}
        onboardingDone={flag("app.onboarding_done")}
        onDone={() => save("app.onboarding_done", "true")}
        pushToast={pushToast}
      />
    </div>
  );
};

export default App;
