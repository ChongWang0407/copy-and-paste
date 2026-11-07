import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask, message, open as openDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, ChevronRight, RotateCcw } from "lucide-react";
import { FaviconGithub } from "./FaviconGithub";
import appLogo from "../../../assets/logo.png";
import AppSelectorModal from "./AppSelectorModal";
import { HotkeyField, Row, Section, Segmented, Toggle } from "./controls";
import { chosenApp, unchosenFallback } from "../utils/defaultApps";
import type { InstalledAppOption } from "../types";
import type { SettingsStore } from "../../app/useSettings";
import { displayNameOf } from "../../app/useSettings";
import type { Locale } from "../../../shared/types";
import { withNativeDialog } from "../../../shared/lib/focus";
import { formatError } from "../../../shared/lib/errorMessage";
import { requestMacAccessibility } from "../../../shared/lib/macAccessibility";
import { clampDisplayName } from "../../../shared/lib/displayName";
import { isTypingHotkey } from "../../../shared/lib/hotkeyKey";
import { GITHUB_URL, WEBSITE_URL } from "../../../shared/config";
import { ACCENTS, ACCENT_NAMES, ACCENT_SETTING_KEY, normalizeAccent } from "../../../shared/lib/accent";

const APP_TYPES = ["text", "rich_text", "image", "video", "code", "url"] as const;
const PERSISTENT_LIMIT_MIN = 50;
const PERSISTENT_LIMIT_MAX = 99999;
const GLASS_SAVE_THROTTLE_MS = 100;
// 复制提示弹窗停留时间 (ms); the default is the slider's middle. Matches capsule.rs.
const CAPSULE_DURATION_MIN = 1000;
const CAPSULE_DURATION_MAX = 3400;
const CAPSULE_DURATION_DEFAULT = 2200;

const appDisplayName = (path: string) => (path.split(/[/\\]/).pop() || path).replace(/\.(exe|app)$/i, "");


interface SettingsPageProps {
  settings: SettingsStore;
  pushToast: (message: string, duration?: number) => void;
  /** The window stays open while the user drags it to its fixed position. */
  onSetFixedPosition: () => void;
  onResetSettings: () => void;
  onGlassLevel: (level: number) => void;
}

const SettingsPage = ({
  settings,
  pushToast,
  onSetFixedPosition,
  onResetSettings,
  onGlassLevel,
}: SettingsPageProps) => {
  const { t, values, flag, save, remember, language } = settings;
  const [openSections, setOpenSections] = useState<Record<string, boolean>>({ common: true });
  const toggleSection = (id: string) => setOpenSections((prev) => ({ ...prev, [id]: !prev[id] }));

  const [autostart, setAutostart] = useState(false);
  const [accessibilityGranted, setAccessibilityGranted] = useState(true);
  const [dataPath, setDataPath] = useState("");
  const [installedApps, setInstalledApps] = useState<InstalledAppOption[]>([]);
  const [defaultApps, setDefaultApps] = useState<Record<string, string>>({});
  const [appSelector, setAppSelector] = useState<string | null>(null);
  const [hasFixedPosition, setHasFixedPosition] = useState(false);

  const [nameDraft, setNameDraft] = useState(displayNameOf(values));
  const [limitDraft, setLimitDraft] = useState(values["app.persistent_limit"] ?? "1000");

  useEffect(() => {
    invoke<boolean>("is_autostart_enabled").then(setAutostart).catch(console.error);
    invoke<string>("get_data_path").then(setDataPath).catch(console.error);
    invoke<boolean>("has_fixed_window_position").then(setHasFixedPosition).catch(console.error);
    invoke<{ name: string; path: string }[]>("scan_installed_apps")
      .then((apps) =>
        setInstalledApps(
          apps.map((app) => ({ label: app.name, value: app.path })).sort((a, b) => a.label.localeCompare(b.label))
        )
      )
      .catch(console.error);
    APP_TYPES.forEach((type) => {
      invoke<string>("get_system_default_app", { contentType: type })
        .then((name) => setDefaultApps((prev) => ({ ...prev, [type]: name })))
        .catch(console.error);
    });
  }, []);

  // Only shown while the permission is missing.
  useEffect(() => {
    const check = () => {
      invoke<boolean>("check_macos_permissions").then(setAccessibilityGranted).catch(console.error);
    };
    check();
    const interval = window.setInterval(check, 3000);
    return () => window.clearInterval(interval);
  }, []);

  const hotkey = values["app.hotkey"] ?? "";

  const hotkeyIsFree = useCallback(
    async (next: string, allowTyping: boolean, other: { hotkey: string; name: string }) => {
      if (!next) return true;
      if (!allowTyping && isTypingHotkey(next)) {
        pushToast(t("hotkey_needs_modifier"), 4000);
        return false;
      }
      if (next === other.hotkey) {
        pushToast(t("hotkey_conflict").replace("{name}", other.name), 4000);
        return false;
      }
      try {
        await invoke("test_hotkey_available", { hotkey: next, allowTyping });
        return true;
      } catch (error) {
        pushToast(`${next}: ${formatError(error, t) || t("hotkey_occupied")}`, 4000);
        return false;
      }
    },
    [pushToast, t]
  );

  const recordWakeHotkey = async (next: string) => {
    if (!(await hotkeyIsFree(next, true, { hotkey: "", name: "" }))) return;
    save("app.hotkey", next);
    invoke("register_hotkey", { hotkey: next }).catch((error) => {
      if (next) pushToast(t("hotkey_register_failed") + String(error), 3000);
    });
  };

  const applyToggle = (key: string, value: boolean, command: string, args: Record<string, unknown>) => {
    remember(key, String(value));
    invoke(command, args).catch((error) => {
      remember(key, String(!value));
      console.error(`${command} failed`, error);
    });
  };

  const setPositionMode = (mode: string) => {
    save("app.window_position_mode", mode);
    if (mode === "fixed" && !hasFixedPosition) onSetFixedPosition();
  };

  const commitLimit = () => {
    const parsed = parseInt(limitDraft, 10);
    const previous = values["app.persistent_limit"] ?? "1000";
    if (!Number.isFinite(parsed)) {
      setLimitDraft(previous);
      return;
    }
    const clamped = String(Math.min(PERSISTENT_LIMIT_MAX, Math.max(PERSISTENT_LIMIT_MIN, parsed)));
    setLimitDraft(clamped);
    save("app.persistent_limit", clamped);
  };

  // Each change rebuilds the native glass, so a drag is throttled.
  const glassLevel = Number(values["app.glass_level"] ?? 35);
  const glassTimer = useRef<number | null>(null);
  const pendingGlass = useRef<number | null>(null);
  const setGlassLevel = (level: number) => {
    remember("app.glass_level", String(level));
    onGlassLevel(level);
    pendingGlass.current = level;
    if (glassTimer.current !== null) return;
    glassTimer.current = window.setTimeout(() => {
      glassTimer.current = null;
      invoke("set_glass_level", { level: pendingGlass.current }).catch(console.error);
    }, GLASS_SAVE_THROTTLE_MS);
  };

  const accent = normalizeAccent(values[ACCENT_SETTING_KEY]);

  const capsuleEnabled = flag("app.capsule_enabled", true);
  const capsuleStyle = ["island", "black"].includes(values["app.capsule_style"] ?? "") ? values["app.capsule_style"]! : "glass";
  // The style goes along: save_setting may not have landed yet.
  const previewCapsule = (style: string) => {
    invoke("preview_capsule", { style }).catch(console.error);
  };
  const capsuleDuration = Math.min(
    CAPSULE_DURATION_MAX,
    Math.max(CAPSULE_DURATION_MIN, Number(values["app.capsule_duration_ms"]) || CAPSULE_DURATION_DEFAULT)
  );
  const durationProgress = ((capsuleDuration - CAPSULE_DURATION_MIN) / (CAPSULE_DURATION_MAX - CAPSULE_DURATION_MIN)) * 100;

  const changeDataPath = () =>
    withNativeDialog(async () => {
      const selected = await openDialog({ directory: true, multiple: false, title: t("change_data_path") });
      if (!selected) return;
      const newPath = selected as string;
      const confirmed = await ask(t("data_move_confirm").replace("{path}", newPath), {
        title: t("change_data_path"),
        kind: "warning",
        okLabel: t("confirm"),
        cancelLabel: t("cancel"),
      });
      if (!confirmed) return;
      try {
        await invoke("set_data_path", { newPath });
        await message(t("data_move_success"), { title: t("notice"), kind: "info" });
        await invoke("relaunch");
      } catch (error) {
        await message(t("data_move_failed") + formatError(error, t), { title: t("error"), kind: "error" });
      }
    }, "settings:change-data-path");

  const describeDefaultApp = (type: string) => {
    const appName = (path: string) => installedApps.find((app) => app.value === path)?.label ?? appDisplayName(path);
    const chosen = chosenApp(values, type);
    if (chosen) return appName(chosen);
    if (unchosenFallback(values, type) === "text") {
      return `${t("same_as_text_app")} (${appName(chosenApp(values, "text"))})`;
    }
    return defaultApps[type] ? `${t("system_default")} (${appDisplayName(defaultApps[type])})` : t("not_configured");
  };

  const positionMode = values["app.window_position_mode"] ?? "follow_mouse";
  const nameChanged = nameDraft.trim() !== displayNameOf(values);

  return (
    <div className="settings-page">
      <Section title={t("section_common")} open={!!openSections.common} onToggle={() => toggleSection("common")}>
        <Row label={t("wake_hotkey")} subtitle={t("hotkey_click_hint")}>
          <HotkeyField t={t} value={hotkey} label={t("wake_hotkey")} onRecorded={(next) => void recordWakeHotkey(next)} />
        </Row>
        <Row label={t("autostart")} hint={t("autostart_hint")}>
          <Toggle
            label={t("autostart")}
            checked={autostart}
            onChange={(enabled) => {
              setAutostart(enabled);
              invoke("toggle_autostart", { enabled }).catch(() => setAutostart(!enabled));
            }}
          />
        </Row>
        <Row label={t("silent_start")} hint={t("silent_start_hint")}>
          <Toggle
            label={t("silent_start")}
            checked={flag("app.silent_start")}
            onChange={(enabled) => applyToggle("app.silent_start", enabled, "set_silent_start", { enabled })}
          />
        </Row>
        <Row label={t("window_position")} hint={t("window_position_hint")} column>
          <Segmented
            value={positionMode}
            onChange={setPositionMode}
            options={[
              { value: "follow_mouse", label: t("window_position_follow") },
              { value: "last", label: t("window_position_last") },
              { value: "fixed", label: t("window_position_fixed") },
            ]}
          />
          {positionMode === "fixed" && hasFixedPosition && (
            <button type="button" className="link-button" onClick={onSetFixedPosition}>
              {t("window_position_reset")}
            </button>
          )}
        </Row>
        <Row label={t("hide_tray_icon")} hint={t("hide_tray_icon_hint")}>
          <Toggle
            label={t("hide_tray_icon")}
            checked={flag("app.hide_tray_icon")}
            onChange={(hide) => applyToggle("app.hide_tray_icon", hide, "set_tray_visible", { visible: !hide })}
          />
        </Row>
        <Row label={t("hide_dock_icon")} hint={t("hide_dock_icon_hint")}>
          <Toggle
            label={t("hide_dock_icon")}
            checked={flag("app.hide_dock_icon")}
            onChange={(hide) => applyToggle("app.hide_dock_icon", hide, "set_dock_visible", { visible: !hide })}
          />
        </Row>
        <Row label={t("show_search_box")}>
          <Toggle
            label={t("show_search_box")}
            checked={flag("app.show_search_box", true)}
            onChange={(show) => save("app.show_search_box", String(show))}
          />
        </Row>
        <Row label={t("show_type_tabs")} hint={t("show_type_tabs_hint")}>
          <Toggle
            label={t("show_type_tabs")}
            checked={flag("app.show_type_tabs", true)}
            onChange={(show) => save("app.show_type_tabs", String(show))}
          />
        </Row>
        <Row label={t("show_source_app")} hint={t("show_source_app_hint")}>
          <Toggle
            label={t("show_source_app")}
            checked={flag("app.show_source_app_icon", true)}
            onChange={(show) => save("app.show_source_app_icon", String(show))}
          />
        </Row>
        <Row label={t("language")} column>
          <Segmented<Locale>
            value={language}
            onChange={(next) => save("app.language", next)}
            options={[
              { value: "zh", label: t("language_zh") },
              { value: "en", label: t("language_en") },
            ]}
          />
        </Row>
        <Row label={t("display_name")} hint={t("display_name_hint")} column>
          <div className="inline-field">
            <input
              className="glass-field"
              value={nameDraft}
              onChange={(event) => setNameDraft(clampDisplayName(event.target.value))}
            />
            <button
              type="button"
              className="glass-button"
              disabled={!nameChanged}
              onClick={() => {
                const name = nameDraft.trim();
                save("app.display_name", name);
                setNameDraft(name || displayNameOf({}));
                pushToast(t("display_name_saved"));
              }}
            >
              {t("save")}
            </button>
          </div>
        </Row>
        <Row label={t("arrow_key_selection")} hint={t("arrow_key_selection_hint")}>
          <Toggle
            label={t("arrow_key_selection")}
            checked={flag("app.arrow_key_selection", true)}
            onChange={(enabled) => save("app.arrow_key_selection", String(enabled))}
          />
        </Row>
        {!accessibilityGranted && (
          <Row label={t("accessibility_permission")} hint={t("accessibility_permission_hint")}>
            <button
              type="button"
              className="glass-button is-compact"
              onClick={() => {
                requestMacAccessibility().catch(() => pushToast(t("accessibility_settings_failed"), 4000));
              }}
            >
              {t("accessibility_go_authorize")}
            </button>
          </Row>
        )}
      </Section>

      <Section title={t("section_interface")} open={!!openSections.interface} onToggle={() => toggleSection("interface")}>
        <Row label={t("accent_color")} column>
          <div className="accent-swatches" role="radiogroup" aria-label={t("accent_color")}>
            {ACCENT_NAMES.map((name) => (
              <button
                key={name}
                type="button"
                role="radio"
                aria-checked={accent === name}
                aria-label={t(`accent_${name}`)}
                title={t(`accent_${name}`)}
                className={`accent-swatch${accent === name ? " is-active" : ""}`}
                style={{ "--swatch": ACCENTS[name].base, "--swatch-light": ACCENTS[name].light } as CSSProperties}
                onClick={() => save(ACCENT_SETTING_KEY, name)}
              >
                {accent === name && <Check size={12} strokeWidth={3} />}
              </button>
            ))}
          </div>
        </Row>
        <Row label={t("glass_transparency")} column>
          <div className="glass-slider">
            <input
              type="range"
              min={0}
              max={100}
              value={glassLevel}
              aria-label={t("glass_transparency")}
              style={{ "--range-progress": `${glassLevel}%` } as CSSProperties}
              onChange={(event) => setGlassLevel(Number(event.target.value))}
            />
            <div className="glass-slider-ends">
              <span>{t("glass_clear")}</span>
              <span>{t("glass_tinted")}</span>
            </div>
          </div>
        </Row>
        <Row label={t("capsule")} hint={t("capsule_hint")}>
          <Toggle
            label={t("capsule")}
            checked={capsuleEnabled}
            onChange={(enabled) => {
              save("app.capsule_enabled", String(enabled));
              if (enabled) previewCapsule(capsuleStyle);
            }}
          />
        </Row>
        {capsuleEnabled && (
          <Row label={t("capsule_style")} column child>
            <Segmented
              value={capsuleStyle}
              onChange={(style) => {
                save("app.capsule_style", style);
                previewCapsule(style);
              }}
              options={[
                { value: "glass", label: t("capsule_style_glass") },
                { value: "island", label: t("capsule_style_island") },
                { value: "black", label: t("capsule_style_black") },
              ]}
            />
          </Row>
        )}
        {capsuleEnabled && (
          <Row
            label={`${t("capsule_duration")} · ${(capsuleDuration / 1000).toFixed(1)}${t("seconds_short")}`}
            column
            child
          >
            <div className="glass-slider">
              <input
                type="range"
                min={CAPSULE_DURATION_MIN}
                max={CAPSULE_DURATION_MAX}
                step={100}
                value={capsuleDuration}
                aria-label={t("capsule_duration")}
                style={{ "--range-progress": `${durationProgress}%` } as CSSProperties}
                onChange={(event) => save("app.capsule_duration_ms", event.target.value)}
                // Show it once the slider is let go, to feel the new time.
                onPointerUp={() => previewCapsule(capsuleStyle)}
                onKeyUp={() => previewCapsule(capsuleStyle)}
              />
              <div className="glass-slider-ends">
                <span>{t("capsule_duration_short")}</span>
                <span>{t("capsule_duration_long")}</span>
              </div>
            </div>
          </Row>
        )}
      </Section>

      <Section title={t("section_paste")} open={!!openSections.paste} onToggle={() => toggleSection("paste")}>
        <Row label={t("persistent_storage")} hint={t("persistent_hint")}>
          <Toggle
            label={t("persistent_storage")}
            checked={flag("app.persistent", true)}
            onChange={(enabled) => applyToggle("app.persistent", enabled, "set_persistence", { enabled })}
          />
        </Row>
        {flag("app.persistent", true) && (
          <Row label={t("persistent_limit_enabled")} hint={t("persistent_limit_enabled_hint")}>
            <Toggle
              label={t("persistent_limit_enabled")}
              checked={flag("app.persistent_limit_enabled")}
              onChange={(enabled) => save("app.persistent_limit_enabled", String(enabled))}
            />
          </Row>
        )}
        {flag("app.persistent", true) && flag("app.persistent_limit_enabled") && (
          <Row label={t("persistent_limit")} hint={t("persistent_limit_hint")} child>
            <input
              className="glass-field is-number"
              inputMode="numeric"
              value={limitDraft}
              onChange={(event) => setLimitDraft(event.target.value.replace(/\D/g, ""))}
              onBlur={commitLimit}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
              }}
            />
          </Row>
        )}
        <Row label={t("merge_duplicates")} hint={t("merge_duplicates_hint")}>
          <Toggle
            label={t("merge_duplicates")}
            checked={flag("app.deduplicate", true)}
            onChange={(enabled) => applyToggle("app.deduplicate", enabled, "set_deduplication", { enabled })}
          />
        </Row>
        <Row label={t("delete_after_paste")} hint={t("delete_after_paste_hint")}>
          <Toggle
            label={t("delete_after_paste")}
            checked={flag("app.delete_after_paste")}
            onChange={(enabled) => applyToggle("app.delete_after_paste", enabled, "set_delete_after_paste", { enabled })}
          />
        </Row>
        <Row label={t("move_to_top_after_paste")} hint={t("move_to_top_after_paste_hint")}>
          <Toggle
            label={t("move_to_top_after_paste")}
            checked={flag("app.move_to_top_after_paste", true)}
            onChange={(enabled) => save("app.move_to_top_after_paste", String(enabled))}
          />
        </Row>
      </Section>

      <Section title={t("section_data")} open={!!openSections.data} onToggle={() => toggleSection("data")}>
        <Row label={t("data_path")} column>
          <code className="path-text">{dataPath}</code>
          <div className="inline-actions">
            <button type="button" className="glass-button is-compact" onClick={() => void changeDataPath()}>
              {t("change_data_path")}
            </button>
            <button
              type="button"
              className="glass-button is-compact"
              onClick={() => {
                invoke("open_folder", { path: dataPath }).catch((error) =>
                  pushToast(t("open_failed") + String(error), 3000)
                );
              }}
            >
              {t("open_folder")}
            </button>
          </div>
        </Row>
      </Section>

      <Section title={t("section_default_apps")} open={!!openSections.apps} onToggle={() => toggleSection("apps")}>
        {APP_TYPES.map((type) => (
          <Row key={type} label={t(`type_${type === "rich_text" ? "rich_text" : type}`)}>
            <button
              type="button"
              className="picker-button"
              title={t("change_app")}
              onClick={() => setAppSelector(type)}
            >
              <span>{describeDefaultApp(type)}</span>
              <ChevronRight size={13} />
            </button>
          </Row>
        ))}
      </Section>

      <footer className="settings-footer">
        <button type="button" className="glass-button" onClick={onResetSettings}>
          <RotateCcw size={13} />
          {t("reset_defaults")}
        </button>
        <div className="brand-title">
          <img className="brand-logo" src={appLogo} alt="" draggable={false} />
          <span className="brand-name">
            Copy &amp; Paste
          </span>
        </div>
        <div className="brand-slogan">{t("slogan")}</div>
        <div className="brand-author">{t("author")}</div>
        <div className="brand-links">
          <button
            type="button"
            className="link-button"
            onClick={() => {
              if (WEBSITE_URL) void openUrl(WEBSITE_URL);
            }}
          >
            {t("official_website")}
          </button>
          <span className="brand-links-sep" aria-hidden="true" />
          <button
            type="button"
            className="link-button"
            onClick={() => {
              if (GITHUB_URL) void openUrl(GITHUB_URL);
            }}
          >
            <FaviconGithub />
            {t("github")}
          </button>
        </div>
      </footer>

      <AppSelectorModal
        show={appSelector}
        installedApps={installedApps}
        theme=""
        colorMode="light"
        t={t}
        onClose={() => setAppSelector(null)}
        onSave={(type, path) => save(`app.${type}`, path)}
        resetLabel={
          appSelector && chosenApp(values, appSelector)
            ? t(unchosenFallback(values, appSelector) === "text" ? "same_as_text_app" : "system_default")
            : undefined
        }
      />
    </div>
  );
};

export default SettingsPage;
