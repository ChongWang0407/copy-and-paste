import type { Translate } from "../../shared/types";
import { errorToText, formatError } from "../../shared/lib/errorMessage";

export type TranslationProvider = "apple" | "deepl" | "openai";

export interface TranslationConfig {
  provider: TranslationProvider;
  openaiBaseUrl: string;
  openaiModel: string;
  deeplKeySet: boolean;
  openaiKeySet: boolean;
  appleSupported: boolean;
  available: boolean;
}

export interface TranslationDraft {
  provider: TranslationProvider;
  openaiBaseUrl: string;
  openaiModel: string;
  deeplKey?: string;
  openaiKey?: string;
  clearDeeplKey?: boolean;
  clearOpenaiKey?: boolean;
}

export function canTestTranslation(draft: TranslationDraft, deeplKeySet: boolean): boolean {
  switch (draft.provider) {
    case "apple":
      return false;
    case "deepl":
      return !draft.clearDeeplKey && Boolean(draft.deeplKey?.trim() || deeplKeySet);
    case "openai":
      return Boolean(draft.openaiBaseUrl.trim() && draft.openaiModel.trim());
  }
}

const errorKeys = new Map([
  ["missing_config", "translate_err_not_configured"],
  ["invalid_endpoint", "translate_err_invalid_url"],
  ["auth_failed", "translate_err_invalid_key"],
  ["quota_exceeded", "translate_err_rate_limited"],
  ["timeout", "translate_err_timeout"],
  ["network_error", "translate_err_network"],
  ["too_large", "translate_err_too_large"],
  ["unsupported_os", "translate_unsupported_os"],
  ["cancelled", "translate_cancelled"],
  ["unsupported_pair", "translate_unavailable"],
  ["config_unavailable", "translate_err_config_unavailable"],
  ["secret_storage_unavailable", "translate_err_secret_storage"],
  ["invalid_response", "translate_err_invalid_response"],
  ["no_window", "translate_err_no_window"],
]);

export function translationErrorMessage(t: Translate, error: unknown): string {
  const text = errorToText(error);
  const key = errorKeys.get(text);
  if (key) return t(key);
  const detail = formatError(text.replace(/^upstream_error: /, ""), t);
  return detail ? t("translate_failed") + detail : t("translate_err_generic");
}
