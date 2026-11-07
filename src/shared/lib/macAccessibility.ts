import { invoke } from "@tauri-apps/api/core";

type PermissionRequest = "granted" | "prompted" | "failed";

let systemPromptRequested = false;

/**
 * "Authorize" for the macOS Accessibility permission.
 *
 * The first time, only macOS's own dialog is shown: it puts this app in the
 * Accessibility list, and its own button opens System Settings. Opening
 * Settings as well put two things on screen at once. Later clicks go straight
 * to Settings, where the app is listed by then.
 */
export const requestMacAccessibility = async (): Promise<void> => {
  if (!systemPromptRequested) {
    systemPromptRequested = true;
    const outcome = await invoke<PermissionRequest>("request_macos_permissions");
    if (outcome !== "failed") return;
  }
  await invoke("open_macos_accessibility_settings");
};
