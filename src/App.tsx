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
import BatchPanel from "./features/batch/BatchPanel";
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
import type { ClipboardEntry } from "./shared/types";

/** How much the CSS tint shows on systems with frosted glass only. */
const FROSTED_TINT_MAX = 0.72;
/** Longer than the side panel's animation, so its in-between widths aren't saved. */
const WINDOW_SIZE_SAVE_DELAY_MS = 500;

const App = () => {
  useFocusCoordinator();
  useContextMenuBlock();

  const settings = useSettings();
  const { t, flag, values, ready, save } = settings;
  const { toasts, pushToast, confirmDialog, openConfirm, closeConfirm } = useOverlays(t);

  const [showSettings, setShowSettings] = useState(false);
  const [draft, setDraft] = useState("");
  const [search, setSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState<TypeFilter | null>(null);
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [isWindowPinned, setIsWindowPinned] = useState(false);
  const [settingFixedPosition, setSettingFixedPosition] = useState(false);
  const [editing, setEditing] = useState<ClipboardEntry | null>(null);
  const [batchItems, setBatchItems] = useState<ClipboardEntry[]>([]);
  const [previewEntry, setPreviewEntry] = useState<ClipboardEntry | null>(null);
  const [hasNativeGlass, setHasNativeGlass] = useState(true);
  const [canTranslate, setCanTranslate] = useState(false);

  const pushToastRef = useRef(pushToast);
  pushToastRef.current = pushToast;
  const tRef = useRef(t);
  tRef.current = t;

  const searchRef = useRef<SearchBoxHandle | null>(null);
  const listRef = useRef<ClipboardListHandle | null>(null);

  const { items, setItems, hasMore, loading, loaded, loadMore, refetch } = useHistory(search, typeFilter, ready);
  const { panel, open: openPanel, close: closePanel } = useSidePanel();

  const deleteAfterPaste = flag("app.delete_after_paste");
  const moveToTopAfterPaste = flag("app.move_to_top_after_paste", true);

  const afterPaste = useCallback(() => {
    setDraft("");
    setSearch("");
    setSelectedIndex(null);
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

  const startBatch = useCallback(
    (item: ClipboardEntry) => {
      setEditing(null);
      setBatchItems([item]);
      void openPanel("batch");
    },
    [openPanel]
  );

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
      setItems((prev) => prev.filter((entry) => entry.id !== item.id));
      invoke("delete_clipboard_entry", { id: item.id }).catch((error) => {
        pushToast(t("delete_failed") + String(error), 3000);
        void refetch(true);
      });
    },
    [pushToast, refetch, setItems, t]
  );

  const closeSidePanel = useCallback(() => {
    setEditing(null);
    setBatchItems([]);
    setPreviewEntry(null);
    void closePanel();
  }, [closePanel]);

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

  const dismiss = useCallback(() => {
    if (settingFixedPosition) return true;
    if (panel.kind) {
      closeSidePanel();
      return true;
    }
    if (showSettings) {
      leaveSettings();
      return true;
    }
    if (selectedIndex !== null) {
      setSelectedIndex(null);
      return true;
    }
    if (draft || search) {
      setDraft("");
      setSearch("");
      return true;
    }
    return false;
  }, [closeSidePanel, draft, leaveSettings, panel.kind, search, selectedIndex, settingFixedPosition, showSettings]);

  // 编辑 and 翻译 take typing: the list's keys stay off while they are open.
  const typingInPanel = panel.kind === "edit" || panel.kind === "translate";

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
  });

  useSpacePreview({
    enabled: !showSettings && !settingFixedPosition && !typingInPanel && !batchMode,
    items,
    selectedIndex,
    onOpen: showPreview,
    onClose: hidePreview,
  });

  // The window opens at the size it was left at. While a side panel is open
  // the width isn't the user's (the native panel swallows Tauri's resize events,
  // so this listens to the page).
  const sidePanelOpen = panel.kind !== null;
  useEffect(() => {
    if (!isTauriRuntime() || !ready) return;
    let timer: number | null = null;
    const onResize = () => {
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        if (sidePanelOpen) return;
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
  }, [ready, sidePanelOpen]);

  // The selection follows the list, not a stale index.
  useEffect(() => {
    setSelectedIndex(null);
  }, [search, typeFilter]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const subscriptions = [
      // Each opening starts unfiltered: a filter is for digging one item out.
      listen("clipboard-shown", () => {
        setTypeFilter(null);
        setSelectedIndex(null);
        listRef.current?.scrollToTop();
      }),
      listen<boolean>("window-pinned-changed", ({ payload }) => setIsWindowPinned(payload)),
      // A close (hotkey, red button) was refused because the window is pinned.
      listen("window-pinned-close-blocked", () => pushToastRef.current(tRef.current("pinned_esc_hint"))),
      listen("side-panel-collapsed", () => {
        setEditing(null);
        setBatchItems([]);
        setPreviewEntry(null);
      }),
    ];
    return () => {
      subscriptions.forEach((subscription) => void subscription.then((off) => off()));
    };
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    invoke<boolean>("supports_liquid_glass").then(setHasNativeGlass).catch(() => setHasNativeGlass(true));
    invoke<boolean>("translation_supported").then(setCanTranslate).catch(() => setCanTranslate(false));
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

  const togglePinned = () => {
    const next = !isWindowPinned;
    setIsWindowPinned(next);
    invoke("set_window_pinned", { pinned: next }).catch(console.error);
    if (next) pushToast(t("pinned_esc_hint"));
  };

  const clearHistory = () => {
    openConfirm({
      title: t("clear_confirm_title"),
      message: t("clear_confirm_message"),
      danger: true,
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

  const renderItem = (item: ClipboardEntry) => (
    <ClipboardCard
      item={item}
      t={t}
      selected={selectedIndex !== null && items[selectedIndex]?.id === item.id}
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
  );

  if (!ready) return <div className="app-shell" />;

  return (
    <div className="app-shell" style={shellStyle}>
      <div className="app-main" style={panel.listWidth ? { flex: `0 0 ${panel.listWidth}px` } : undefined}>
        <AppHeader
          t={t}
          displayName={displayNameOf(values)}
          showSettings={showSettings}
          isWindowPinned={isWindowPinned}
          favoritesOpen={panel.kind === "favorites"}
          onBack={leaveSettings}
          onToggleFavorites={toggleFavorites}
          onTogglePinned={togglePinned}
          onPinnedCloseAttempt={() => pushToast(t("pinned_esc_hint"))}
          onClearHistory={clearHistory}
          onOpenSettings={() => {
            closeSidePanel();
            setShowSettings(true);
          }}
        />

        {showSettings ? (
          <SettingsPage
            settings={settings}
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
                onLoadMore={() => void loadMore()}
                renderItem={renderItem}
              />
            </div>
          </>
        )}
      </div>

      {panel.kind && (
        <aside className="side-panel">
          {panel.kind === "favorites" && (
            <FavoritesPanel t={t} pushToast={pushToast} onPasteFailed={failed} />
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
          {panel.kind === "translate" && editing && (
            <TranslatePanel
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
          {panel.kind === "preview" && previewEntry && (
            <PreviewPanel key={previewEntry.id} t={t} entry={previewEntry} />
          )}
          {panel.kind === "batch" && (
            <BatchPanel
              t={t}
              items={batchItems}
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
        confirmLabel={confirmDialog.danger ? t("clear_confirm_ok") : t("confirm")}
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
