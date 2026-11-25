import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertCircle, CheckCircle2, Eye, EyeOff, Loader2 } from "lucide-react";
import { withNativeDialog } from "../../../../shared/lib/focus";
import type { Translate } from "../../../../shared/types";
import { canTestTranslation, translationErrorMessage } from "../../../translate/config";
import type { TranslationConfig, TranslationDraft, TranslationProvider } from "../../../translate/config";
import { Row, Segmented } from "../controls";

interface TranslationSettingsProps {
  config: TranslationConfig;
  onSaved: (config: TranslationConfig) => void;
  pushToast: (message: string, duration?: number) => void;
  t: Translate;
}

const savedDraft = (config: TranslationConfig): TranslationDraft => ({
  provider: config.provider,
  openaiBaseUrl: config.openaiBaseUrl,
  openaiModel: config.openaiModel,
});

const TranslationSettings = ({ config, onSaved, pushToast, t }: TranslationSettingsProps) => {
  const [draft, setDraft] = useState(() => savedDraft(config));
  const [showKey, setShowKey] = useState(false);
  const [busy, setBusy] = useState<"saving" | "testing" | null>(null);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const { provider } = config;
  const deepl = provider === "deepl";
  const keyField = deepl ? "deeplKey" : "openaiKey";
  const key = draft[keyField] ?? "";
  const keySet = deepl ? config.deeplKeySet : config.openaiKeySet;
  const workingDraft: TranslationDraft = deepl
    ? { ...savedDraft(config), deeplKey: key }
    : { provider, openaiBaseUrl: draft.openaiBaseUrl, openaiModel: draft.openaiModel, openaiKey: key };
  const dirty = Boolean(key.trim()) || (!deepl && (
    draft.openaiBaseUrl.trim() !== config.openaiBaseUrl || draft.openaiModel.trim() !== config.openaiModel
  ));

  const updateDraft = (field: "openaiBaseUrl" | "openaiModel" | "deeplKey" | "openaiKey", value: string) => {
    setDraft((previous) => ({ ...previous, [field]: value }));
    if (field === keyField && !value) setShowKey(false);
    setTestResult(null);
  };

  const save = async (next: TranslationDraft, update: (saved: TranslationConfig) => void, toastKey?: string) => {
    if (busy) return;
    setBusy("saving");
    setTestResult(null);
    try {
      const request = () => invoke<TranslationConfig>("save_translation_config", { draft: next });
      const saved = next.deeplKey?.trim() || next.openaiKey?.trim()
        ? await withNativeDialog(request, "translation-settings")
        : await request();
      onSaved(saved);
      update(saved);
      setShowKey(false);
      if (toastKey) pushToast(t(toastKey));
    } catch (error) {
      pushToast(t("save_failed") + translationErrorMessage(t, error), 4000);
    } finally {
      setBusy(null);
    }
  };

  const changeProvider = (next: TranslationProvider) => {
    if (next === provider) return;
    void save({ ...savedDraft(config), provider: next }, () => {});
  };

  const clearKey = () => {
    const next = { ...savedDraft(config), [deepl ? "clearDeeplKey" : "clearOpenaiKey"]: true };
    void save(next, () => setDraft((previous) => ({ ...previous, [keyField]: "" })), "translate_key_cleared");
  };

  const saveForm = () => void save(workingDraft, (saved) => {
    setDraft((previous) => ({
      ...previous,
      ...(deepl ? {} : { openaiBaseUrl: saved.openaiBaseUrl, openaiModel: saved.openaiModel }),
      [keyField]: "",
    }));
  }, "translate_saved");

  const testConnection = async () => {
    if (busy) return;
    setBusy("testing");
    setTestResult(null);
    try {
      const request = () => invoke<string>("test_translation_config", { draft: workingDraft });
      const result = keySet && !key.trim() ? await withNativeDialog(request, "translation-test") : await request();
      setTestResult({ ok: true, message: `${t("translate_test_ok")} · ${result}` });
    } catch (error) {
      setTestResult({ ok: false, message: translationErrorMessage(t, error) });
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      <Row label={t("translate_provider")} column>
        <Segmented
          value={provider}
          disabled={busy !== null}
          onChange={changeProvider}
          options={[
            { value: "apple", label: t("translate_provider_apple") },
            { value: "deepl", label: t("translate_provider_deepl") },
            { value: "openai", label: t("translate_provider_openai") },
          ]}
        />
      </Row>
      {provider === "apple" ? (
        <div className="setting-row is-column">
          <p className={`translate-note${config.appleSupported ? "" : " is-warning"}`}>
            {t(config.appleSupported ? "translate_apple_supported_note" : "translate_apple_unsupported_note")}
          </p>
        </div>
      ) : (
        <>
          {!deepl && (
            <>
              <Row label={t("translate_base_url")} hint={t("translate_base_url_hint")} column>
                <input
                  className="glass-field"
                  type="url"
                  aria-label={t("translate_base_url")}
                  autoComplete="off"
                  spellCheck={false}
                  disabled={busy !== null}
                  value={draft.openaiBaseUrl}
                  placeholder="https://api.openai.com/v1"
                  onChange={(event) => updateDraft("openaiBaseUrl", event.target.value)}
                />
              </Row>
              <Row label={t("translate_model")} column>
                <input
                  className="glass-field"
                  aria-label={t("translate_model")}
                  autoComplete="off"
                  spellCheck={false}
                  disabled={busy !== null}
                  value={draft.openaiModel}
                  placeholder={t("translate_model_placeholder")}
                  onChange={(event) => updateDraft("openaiModel", event.target.value)}
                />
              </Row>
            </>
          )}
          <Row label={t("translate_api_key")} hint={deepl ? undefined : t("translate_openai_key_hint")} column>
            {keySet && (
              <div className="secret-field-meta">
                <span className="setting-status-badge">{t("translate_key_saved")}</span>
                <button type="button" className="link-button is-danger-link" disabled={busy !== null} onClick={clearKey}>
                  {t("translate_key_clear")}
                </button>
              </div>
            )}
            <div className="secret-field">
              <input
                type={showKey ? "text" : "password"}
                className="glass-field"
                aria-label={t("translate_api_key")}
                autoComplete="off"
                spellCheck={false}
                disabled={busy !== null}
                value={key}
                placeholder={t(keySet ? "translate_key_saved_placeholder"
                  : deepl ? "translate_deepl_key_placeholder" : "translate_openai_key_placeholder")}
                onChange={(event) => updateDraft(keyField, event.target.value)}
              />
              <button
                type="button"
                className="secret-field-toggle"
                disabled={!key || busy !== null}
                aria-label={t(showKey ? "translate_hide_key" : "translate_show_key")}
                title={t(showKey ? "translate_hide_key" : "translate_show_key")}
                onClick={() => setShowKey(!showKey)}
              >
                {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
            </div>
          </Row>
          <div className="setting-row is-column">
            <p className="translate-note">{t("translate_privacy_note")}</p>
            <div className="inline-actions">
              <button type="button" className="glass-button is-compact is-primary" disabled={!dirty || busy !== null}
                onClick={saveForm}>
                {t("save")}
              </button>
              <button type="button" className="glass-button is-compact"
                disabled={busy !== null || !canTestTranslation(workingDraft, config.deeplKeySet)}
                onClick={() => void testConnection()}>
                {busy === "testing" && <Loader2 size={12} className="is-spinning" />}
                {t(busy === "testing" ? "translate_testing" : "translate_test")}
              </button>
            </div>
            {testResult && (
              <div className={`translate-test-result ${testResult.ok ? "is-ok" : "is-error"}`} role="status">
                {testResult.ok ? <CheckCircle2 size={13} /> : <AlertCircle size={13} />}
                <span>{testResult.message}</span>
              </div>
            )}
          </div>
        </>
      )}
    </>
  );
};

export default TranslationSettings;
