import { describe, expect, it } from "vitest";
import { translations } from "../../locales";
import { canTestTranslation, translationErrorMessage } from "./config";
import type { TranslationDraft } from "./config";

const t = (key: string) => translations.zh[key as keyof typeof translations.zh];
const draft: TranslationDraft = {
  provider: "openai",
  openaiBaseUrl: "http://localhost:11434/v1",
  openaiModel: "llama3.2",
};

describe("translation settings", () => {
  it("enables DeepL testing with a saved or newly entered key", () => {
    const deepl = { ...draft, provider: "deepl" } as const;
    expect(canTestTranslation(deepl, false)).toBe(false);
    expect(canTestTranslation(deepl, true)).toBe(true);
    expect(canTestTranslation({ ...deepl, clearDeeplKey: true }, true)).toBe(false);
    expect(canTestTranslation({ ...deepl, deeplKey: "  " }, false)).toBe(false);
    expect(canTestTranslation({ ...deepl, deeplKey: "example:fx" }, false)).toBe(true);
  });

  it("enables OpenAI-compatible testing without a key when the address and model are present", () => {
    expect(canTestTranslation(draft, false)).toBe(true);
    expect(canTestTranslation({ ...draft, openaiBaseUrl: "" }, false)).toBe(false);
    expect(canTestTranslation({ ...draft, openaiModel: " " }, false)).toBe(false);
    expect(canTestTranslation({ ...draft, provider: "apple" }, false)).toBe(false);
  });

  it.each([
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
    ["secret_storage_unavailable", "translate_err_secret_storage"],
    ["no_window", "translate_err_no_window"],
  ])("localizes %s", (code, key) => {
    expect(translationErrorMessage(t, code)).toBe(t(key));
  });

  it("shows a safe upstream status and shortens unexpected error detail", () => {
    expect(translationErrorMessage(t, "upstream_error: HTTP 502 Bad Gateway")).toBe(
      t("translate_failed") + "HTTP 502 Bad Gateway"
    );
    expect(translationErrorMessage(t, "<p>Unexpected response</p>" + "x".repeat(500))).toBe(
      t("translate_failed") + "Unexpected response " + "x".repeat(179) + "…"
    );
  });
});
