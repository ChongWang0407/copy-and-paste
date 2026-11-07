import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowDown } from "lucide-react";
import type { ClipboardEntry, Translate } from "../../shared/types";
import { startWindowDrag } from "../../shared/lib/windowDrag";
import { withNativeDialog } from "../../shared/lib/focus";
import { detectDirection, type TranslateDirection, type TranslateLanguage } from "./direction";

interface TranslatePanelProps {
  t: Translate;
  entry: ClipboardEntry;
  deleteAfterPaste: boolean;
  moveToTopAfterPaste: boolean;
  pushToast: (message: string, duration?: number) => void;
  onPasteFailed: (error: unknown) => void;
  onClose: () => void;
}

const languageName = (t: Translate, language: TranslateLanguage) =>
  t(language === "en" ? "translate_lang_en" : "translate_lang_zh");

const errorMessage = (t: Translate, error: unknown) => {
  const text = String(error);
  if (text.includes("unsupported_os")) return t("translate_unsupported_os");
  if (text.includes("cancelled")) return t("translate_cancelled");
  if (text.includes("unsupported_pair")) return t("translate_unavailable");
  return t("translate_failed") + text;
};

/** 翻译: 原文 above, 译文 below, both editable. Chinese ↔ English only. */
const TranslatePanel = ({
  t,
  entry,
  deleteAfterPaste,
  moveToTopAfterPaste,
  pushToast,
  onPasteFailed,
  onClose,
}: TranslatePanelProps) => {
  const [source, setSource] = useState<string | null>(null);
  const [result, setResult] = useState("");
  const [direction, setDirection] = useState<TranslateDirection | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Only the latest request may fill in 译文.
  const requestRef = useRef(0);

  const run = useCallback(
    async (text: string) => {
      if (!text.trim()) return;
      const request = ++requestRef.current;
      const next = detectDirection(text);
      setDirection(next);
      setBusy(true);
      setError(null);
      const translate = () => invoke<string>("translate_text", { text, ...next });
      try {
        const status = await invoke<string>("translation_status", { ...next }).catch(() => "installed");
        // A missing language pack brings up the system's download prompt:
        // the window must not hide while it is open.
        const translated = status === "download" ? await withNativeDialog(translate, "translate") : await translate();
        if (request === requestRef.current) setResult(translated);
      } catch (failure) {
        if (request === requestRef.current) {
          setResult("");
          setError(errorMessage(t, failure));
        }
      } finally {
        if (request === requestRef.current) setBusy(false);
      }
    },
    [t]
  );

  useEffect(() => {
    let alive = true;
    // The list holds a shortened copy of long entries.
    invoke<string>("get_clipboard_content", { id: entry.id })
      .catch(() => entry.content)
      .then((full) => {
        if (!alive) return;
        setSource(full);
        void run(full);
      });
    return () => {
      alive = false;
      requestRef.current += 1;
    };
  }, [entry.content, entry.id, run]);

  const pasteText = (text: string) =>
    invoke("paste_text", {
      text,
      sourceId: entry.id,
      deleteAfterUse: deleteAfterPaste && !entry.is_pinned,
      moveToTop: moveToTopAfterPaste,
    })
      .catch(onPasteFailed)
      .finally(onClose);

  const sourceEmpty = !source || !source.trim();
  const resultEmpty = busy || !result.trim();

  return (
    <div className="side-panel-body">
      <div className="panel-title" onMouseDown={startWindowDrag}>
        <h2>{t("translate_title")}</h2>
      </div>
      <div className="translate-body">
        <label className="translate-label">
          <span>{t("translate_source")}</span>
          {direction && <span className="translate-lang">{languageName(t, direction.source)}</span>}
        </label>
        <textarea
          className="glass-field edit-textarea translate-textarea"
          value={source ?? ""}
          disabled={source === null}
          spellCheck={false}
          onChange={(event) => setSource(event.target.value)}
        />
        <div className="translate-run">
          <button
            type="button"
            className="glass-button is-compact"
            disabled={sourceEmpty || busy}
            onClick={() => void run(source ?? "")}
          >
            <ArrowDown size={13} />
            {busy ? t("translate_running") : t("translate_run")}
          </button>
        </div>
        <label className="translate-label">
          <span>{t("translate_target")}</span>
          {direction && <span className="translate-lang">{languageName(t, direction.target)}</span>}
        </label>
        <textarea
          className={`glass-field edit-textarea translate-textarea${error ? " has-error" : ""}`}
          value={busy ? "" : result}
          placeholder={busy ? t("translate_running") : error ?? ""}
          disabled={busy}
          spellCheck={false}
          onChange={(event) => setResult(event.target.value)}
        />
      </div>
      <div className="panel-actions">
        <button type="button" className="glass-button is-primary" disabled={resultEmpty} onClick={() => void pasteText(result)}>
          {t("translate_paste_result")}
        </button>
        <button type="button" className="glass-button" disabled={sourceEmpty} onClick={() => void pasteText(source ?? "")}>
          {t("translate_paste_source")}
        </button>
        <button
          type="button"
          className="glass-button"
          disabled={resultEmpty}
          onClick={() => {
            invoke("add_manual_item", { content: result, contentType: "text", tags: [] })
              .then(() => {
                pushToast(t("edit_saved_as"));
                onClose();
              })
              .catch((failure) => pushToast(t("save_failed") + String(failure), 3000));
          }}
        >
          {t("translate_save_result")}
        </button>
      </div>
    </div>
  );
};

export default TranslatePanel;
