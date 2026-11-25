import { useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ClipboardEntry, Translate } from "../../shared/types";
import { entryKey, isTextEntry } from "../../shared/lib/clipboardTypes";

/** Files and media are read by id on the backend; sending their content is waste. */
const contentFor = (entry: ClipboardEntry) => (isTextEntry(entry.content_type) ? entry.content : "");

export type PasteFormat = "original" | "plain" | "markdown";

/** Thrown when an entry's full text can't be read: the list only has a shortened copy. */
export const CONTENT_UNAVAILABLE = "content_unavailable";

/** Error codes from a paste that stopped before anything was deleted, and their messages. */
const STOPPED_PASTE_MESSAGES: Record<string, string> = {
  [CONTENT_UNAVAILABLE]: "content_load_failed",
  paste_target_unavailable: "paste_target_unavailable",
  paste_keystroke_failed: "paste_keystroke_failed",
  clipboard_snapshot_failed: "clipboard_snapshot_failed",
  clipboard_changed_during_paste: "clipboard_changed_during_paste",
};

export const stoppedPasteMessage = (error: unknown): string | null => {
  const text = String(error);
  const code = Object.keys(STOPPED_PASTE_MESSAGES).find((key) => text.includes(key));
  return code ? STOPPED_PASTE_MESSAGES[code] : null;
};

interface PasteOptions {
  t: Translate;
  pushToast: (message: string, duration?: number) => void;
  deleteAfterPaste: boolean;
  moveToTopAfterPaste: boolean;
  /** The list goes back to normal after a paste. */
  afterPaste: () => void;
}

export const usePasteActions = ({ t, pushToast, deleteAfterPaste, moveToTopAfterPaste, afterPaste }: PasteOptions) => {
  const failed = useCallback(
    (error: unknown) => {
      const text = String(error);
      // The backend event opens the permission prompt.
      if (text.includes("accessibility_permission_required")) return;
      if (text.includes("ocr_no_text")) return pushToast(t("ocr_no_text"));
      if (text.includes("file_missing")) return pushToast(t("file_missing"));
      const stopped = stoppedPasteMessage(text);
      if (stopped) return pushToast(t(stopped), 3000);
      pushToast(t("copy_failed") + text, 3000);
    },
    [pushToast, t]
  );

  const afterUse = useCallback(
    (entry: ClipboardEntry) => ({
      deleteAfterUse: deleteAfterPaste && !entry.is_pinned,
      moveToTop: moveToTopAfterPaste,
    }),
    [deleteAfterPaste, moveToTopAfterPaste]
  );

  const paste = useCallback(
    async (entry: ClipboardEntry, format: PasteFormat = "original") => {
      const { deleteAfterUse, moveToTop } = afterUse(entry);
      try {
        if (format === "markdown") {
          await invoke("paste_entry_as_markdown", { ...entryKey(entry), deleteAfterUse, moveToTop });
        } else {
          await invoke("copy_to_clipboard", {
            content: contentFor(entry),
            contentType: entry.content_type,
            paste: true,
            ...entryKey(entry),
            deleteAfterUse,
            pasteWithFormat: format === "original",
            moveToTop,
          });
        }
        afterPaste();
      } catch (error) {
        failed(error);
      }
    },
    [afterPaste, afterUse, failed]
  );

  const pasteImageText = useCallback(
    async (entry: ClipboardEntry) => {
      try {
      await invoke("paste_image_text", { ...entryKey(entry), ...afterUse(entry) });
        afterPaste();
      } catch (error) {
        failed(error);
      }
    },
    [afterPaste, afterUse, failed]
  );

  return { paste, pasteImageText, afterUse, failed };
};
