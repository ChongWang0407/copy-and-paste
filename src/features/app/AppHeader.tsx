import { ArrowLeft, Pin, PinOff, Settings as SettingsIcon, Star, Trash2 } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { startWindowDrag } from "../../shared/lib/windowDrag";
import type { Translate } from "../../shared/types";

interface AppHeaderProps {
  t: Translate;
  displayName: string;
  showSettings: boolean;
  isWindowPinned: boolean;
  favoritesOpen: boolean;
  onBack: () => void;
  onToggleFavorites: () => void;
  onTogglePinned: () => void;
  /** The red button while pinned: only Esc closes, so it just reminds. */
  onPinnedCloseAttempt: () => void;
  onClearHistory: () => void;
  onOpenSettings: () => void;
}

const AppHeader = ({
  t,
  displayName,
  showSettings,
  isWindowPinned,
  favoritesOpen,
  onBack,
  onToggleFavorites,
  onTogglePinned,
  onPinnedCloseAttempt,
  onClearHistory,
  onOpenSettings,
}: AppHeaderProps) => (
  <header className="app-header" onMouseDown={startWindowDrag}>
    <button
      type="button"
      className="traffic-close"
      tabIndex={-1}
      title={t("hide")}
      aria-label={t("hide")}
      onMouseDown={(event) => {
        // Not a drag start, and no focus (focus is what left the × and a ring showing).
        event.stopPropagation();
        event.preventDefault();
      }}
      onClick={(event) => {
        event.stopPropagation();
        if (isWindowPinned) {
          onPinnedCloseAttempt();
          return;
        }
        invoke("hide_window_cmd").catch(console.error);
      }}
    >
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <path d="M5 5l6 6M11 5l-6 6" />
      </svg>
    </button>

    {showSettings ? (
      <button type="button" className="icon-button back-button" title={t("back")} aria-label={t("back")} onClick={onBack}>
        <ArrowLeft size={16} />
      </button>
    ) : (
      <>
        <span className="app-name" title={displayName}>
          {displayName}
        </span>
        <div className="header-actions">
          <button
            type="button"
            className={`favorites-button${favoritesOpen ? " is-active" : ""}`}
            title={t("favorites")}
            aria-label={t("favorites")}
            aria-pressed={favoritesOpen}
            onClick={onToggleFavorites}
          >
            <Star size={15} />
          </button>
          <button
            type="button"
            className={`icon-button${isWindowPinned ? " is-active" : ""}`}
            title={isWindowPinned ? t("unpin_window") : t("pin_window")}
            aria-label={isWindowPinned ? t("unpin_window") : t("pin_window")}
            aria-pressed={isWindowPinned}
            onClick={onTogglePinned}
          >
            {isWindowPinned ? <PinOff size={15} /> : <Pin size={15} />}
          </button>
          <button
            type="button"
            className="icon-button"
            title={t("clear_history")}
            aria-label={t("clear_history")}
            onClick={onClearHistory}
          >
            <Trash2 size={15} />
          </button>
          <button
            type="button"
            className="icon-button"
            title={t("settings")}
            aria-label={t("settings")}
            onClick={onOpenSettings}
          >
            <SettingsIcon size={15} />
          </button>
        </div>
      </>
    )}
  </header>
);

export default AppHeader;
