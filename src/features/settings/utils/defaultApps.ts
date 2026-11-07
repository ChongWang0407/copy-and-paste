/** The app chosen in 打开方式 for a content type, or "" for none. */
export const chosenApp = (appSettings: Record<string, string>, type: string): string =>
  (appSettings[`app.${type}`] ?? "").trim();

/**
 * What opens a type with no app chosen. Code falls back to the text app when
 * one is set (services/content_handler.rs), otherwise the system default.
 */
export const unchosenFallback = (appSettings: Record<string, string>, type: string): "text" | "system" =>
  type === "code" && chosenApp(appSettings, "text") ? "text" : "system";
