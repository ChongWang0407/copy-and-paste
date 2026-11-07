import { useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ClipboardEntry, Translate } from "../../shared/types";
import { isTextEntry } from "../../shared/lib/clipboardTypes";

/** Files and media are read by id on the backend; sending their content is waste. */
const contentFor = (entry: ClipboardEntry) => (isTextEntry(entry.content_type) ? entry.content : "");

export type PasteFormat = "original" | "plain" | "markdown";

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
          await invoke("paste_entry_as_markdown", { id: entry.id, deleteAfterUse, moveToTop });
        } else {
          await invoke("copy_to_clipboard", {
            content: contentFor(entry),
            contentType: entry.content_type,
            paste: true,
            id: entry.id,
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
        await invoke("paste_image_text", { id: entry.id, ...afterUse(entry) });
        afterPaste();
      } catch (error) {
        failed(error);
      }
    },
    [afterPaste, afterUse, failed]
  );

  return { paste, pasteImageText, afterUse, failed };
};
