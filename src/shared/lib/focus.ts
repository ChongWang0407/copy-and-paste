import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

let focusSessionSequence = 0;

const nextFocusOwner = (scope: string) => {
  focusSessionSequence += 1;
  return `${scope}:${Date.now()}:${focusSessionSequence}`;
};

export async function acquireBlurGuard(owner: string): Promise<void> {
  await invoke("acquire_blur_guard", { owner });
}

export async function releaseBlurGuard(owner: string): Promise<void> {
  await invoke("release_blur_guard", { owner });
}

export async function activateForInput(): Promise<void> {
  await invoke("activate_window_focus");
}

export async function activateForNativeDialog(): Promise<void> {
  await invoke("activate_app_focus");
}

export async function focusClipboardWindow(): Promise<void> {
  await invoke("focus_clipboard_window");
}

export async function focusWindowImmediately(): Promise<void> {
  await focusClipboardWindow();
}

export async function withNativeDialog<T>(
  action: () => Promise<T>,
  scope = "native-dialog"
): Promise<T> {
  const owner = nextFocusOwner(scope);
  const currentWindow = getCurrentWindow();
  let wasAlwaysOnTop = false;

  await acquireBlurGuard(owner);
  try {
    wasAlwaysOnTop = await currentWindow.isAlwaysOnTop().catch(() => false);
    if (wasAlwaysOnTop) {
      await currentWindow.setAlwaysOnTop(false).catch(() => {});
    }

    await activateForNativeDialog().catch(() => {});
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    return await action();
  } finally {
    if (wasAlwaysOnTop) {
      await currentWindow.setAlwaysOnTop(true).catch(() => {});
    }
    await focusClipboardWindow().catch(() => {});
    await releaseBlurGuard(owner).catch(() => {});
  }
}
