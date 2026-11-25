import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ClipboardEntry, Translate } from "../../shared/types";
import { entryKey } from "../../shared/lib/clipboardTypes";
import { startWindowDrag } from "../../shared/lib/windowDrag";

interface EditPanelProps {
  t: Translate;
  entry: ClipboardEntry;
  deleteAfterPaste: boolean;
  moveToTopAfterPaste: boolean;
  pushToast: (message: string, duration?: number) => void;
  onPasteFailed: (error: unknown) => void;
  onClose: () => void;
}

/** Rich text is edited as plain text; 另存为 keeps the formatted original. */
const EditPanel = ({
  t,
  entry,
  deleteAfterPaste,
  moveToTopAfterPaste,
  pushToast,
  onPasteFailed,
  onClose,
}: EditPanelProps) => {
  const [text, setText] = useState<string | null>(null);
  const { id, createdAt } = entryKey(entry);

  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const toastRef = useRef(pushToast);
  toastRef.current = pushToast;
  const tRef = useRef(t);
  tRef.current = t;

  useEffect(() => {
    let alive = true;
    // The list holds a shortened copy of long entries. Without the full text
    // there is nothing to edit: saving the copy would cut the entry short.
    invoke<string>("get_clipboard_content", { id, createdAt })
      .then((full) => {
        if (alive) setText(full);
      })
      .catch((error) => {
        if (!alive) return;
        console.error("Full content failed to load", error);
        toastRef.current(tRef.current("content_load_failed"), 3000);
        closeRef.current();
      });
    return () => {
      alive = false;
    };
  }, [id, createdAt]);

  const fail = (error: unknown) => pushToast(t("save_failed") + String(error), 3000);
  const save = () => invoke("update_item_content", { id, createdAt, newContent: text ?? "" });

  const empty = !text || !text.trim();

  return (
    <div className="side-panel-body">
      <div className="panel-title" onMouseDown={startWindowDrag}>
        <h2>{t("edit_title")}</h2>
      </div>
      <div className="panel-fill">
        <textarea
          className={`glass-field edit-textarea${entry.content_type === "code" ? " is-code" : ""}`}
          value={text ?? ""}
          disabled={text === null}
          spellCheck={false}
          onChange={(event) => setText(event.target.value)}
        />
      </div>
      <div className="panel-actions">
        <button
          type="button"
          className="glass-button is-primary"
          disabled={empty}
          onClick={() => {
            save()
              .then(() =>
                invoke("paste_text", {
                  text,
                  source: { id, createdAt },
                  deleteAfterUse: deleteAfterPaste && !entry.is_pinned,
                  moveToTop: moveToTopAfterPaste,
                })
                  .catch(onPasteFailed)
                  .finally(onClose)
              )
              .catch(fail);
          }}
        >
          {t("edit_save_and_paste")}
        </button>
        <button
          type="button"
          className="glass-button"
          disabled={empty}
          onClick={() => {
            save().then(onClose).catch(fail);
          }}
        >
          {t("edit_save")}
        </button>
        <button
          type="button"
          className="glass-button"
          disabled={empty}
          onClick={() => {
            const contentType = entry.content_type === "rich_text" ? "text" : entry.content_type;
            invoke("add_manual_item", { content: text, contentType, tags: [] })
              .then(() => {
                pushToast(t("edit_saved_as"));
                onClose();
              })
              .catch(fail);
          }}
        >
          {t("edit_save_as")}
        </button>
      </div>
    </div>
  );
};

export default EditPanel;
