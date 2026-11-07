import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { translations, type TranslationKey } from "../../locales";
import type { Locale, Translate } from "../../shared/types";
import { isTauriRuntime } from "../../shared/lib/tauriRuntime";
import { DEFAULT_DISPLAY_NAME } from "../../shared/lib/displayName";
import { ACCENT_SETTING_KEY, applyAccent, mirrorAccent, normalizeAccent } from "../../shared/lib/accent";

const LANGUAGE_MIRROR_KEY = "copy_paste_language";

const systemLanguage = (): Locale => (navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en");

export interface SettingsStore {
  ready: boolean;
  values: Record<string, string>;
  language: Locale;
  t: Translate;
  flag: (key: string, fallback?: boolean) => boolean;
  /** Saves a setting and keeps the window in step with it. */
  save: (key: string, value: string) => void;
  /** For settings saved by a dedicated backend command. */
  remember: (key: string, value: string) => void;
}

export const useSettings = (): SettingsStore => {
  const [values, setValues] = useState<Record<string, string>>({});
  const [ready, setReady] = useState(false);

  useEffect(() => {
    if (!isTauriRuntime()) {
      setReady(true);
      return;
    }
    invoke<Record<string, string>>("get_settings")
      .then((loaded) => {
        setValues(loaded);
        setReady(true);
      })
      .catch((error) => console.error("Settings failed to load", error));
  }, []);

  const language: Locale = useMemo(() => {
    const saved = values["app.language"];
    return saved === "zh" || saved === "en" ? saved : systemLanguage();
  }, [values]);

  useEffect(() => {
    try {
      localStorage.setItem(LANGUAGE_MIRROR_KEY, language);
    } catch {
      // Only the crash screen reads this.
    }
    document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
  }, [language]);

  // Until settings load, the colour mirrored last time (applied in main.tsx) stays.
  const accent = ready ? normalizeAccent(values[ACCENT_SETTING_KEY]) : null;
  useEffect(() => {
    if (!accent) return;
    applyAccent(accent);
    mirrorAccent(accent);
  }, [accent]);

  const t: Translate = useCallback(
    (key) => translations[language][key as TranslationKey] ?? translations.en[key as TranslationKey] ?? key,
    [language]
  );

  const flag = useCallback(
    (key: string, fallback = false) => {
      const value = values[key];
      return value === undefined ? fallback : value === "true";
    },
    [values]
  );

  const remember = useCallback((key: string, value: string) => {
    setValues((prev) => ({ ...prev, [key]: value }));
  }, []);

  const save = useCallback(
    (key: string, value: string) => {
      remember(key, value);
      invoke("save_setting", { key, value }).catch((error) => console.error(`Saving ${key} failed`, error));
    },
    [remember]
  );

  return { ready, values, language, t, flag, save, remember };
};

export const displayNameOf = (values: Record<string, string>) =>
  values["app.display_name"]?.trim() || DEFAULT_DISPLAY_NAME;
