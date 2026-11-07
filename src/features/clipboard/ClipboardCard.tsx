import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { MouseEvent } from "react";
import { ArrowUpToLine, Check, FileText, ImageOff, Pencil, Play, Trash2 } from "lucide-react";
import type { ClipboardEntry, Translate } from "../../shared/types";
import { isEditableEntry, isTextEntry } from "../../shared/lib/clipboardTypes";
import { hasMatch, splitMatches, type TextPart } from "./searchHighlight";
import { formatAge } from "../../shared/lib/formatAge";
import { getSourceAppIcon, peekSourceAppIcon } from "../../shared/lib/sourceAppIcon";
import { getFileIcon, peekFileIcon } from "../../shared/lib/fileIcon";
import { useImageThumbnail, useVideoThumbnail } from "../../shared/hooks/useImageThumbnail";
import { cardImageBox, type ImageSize } from "./cardImage";
import { useNow } from "./useNow";

interface ClipboardCardProps {
  item: ClipboardEntry;
  t: Translate;
  selected: boolean;
  showSourceIcon: boolean;
  /** 批量粘贴: a tick circle replaces the actions and clicks select. */
  batchMode: boolean;
  batchChecked: boolean;
  onPaste: (item: ClipboardEntry) => void;
  onToggleBatch: (item: ClipboardEntry) => void;
  onContextMenu: (item: ClipboardEntry) => void;
  onEdit: (item: ClipboardEntry) => void;
  onTogglePin: (item: ClipboardEntry) => void;
  onDelete: (item: ClipboardEntry) => void;
  /** Images only: OCR the picture and paste the text. */
  onPasteAsText: (item: ClipboardEntry) => void;
  /** 翻译 (macOS 26+, 文本 cards): opens the translate panel. Absent: no button. */
  onTranslate?: (item: ClipboardEntry) => void;
  /** The search the list is showing: its words are lit in the card's text. */
  searchTerm: string;
}

/** 粘贴为文字: the character 文 drawn as lines, matching the lucide icons beside it. */
const WenIcon = () => (
  <svg
    width={12}
    height={12}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth={2}
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <path d="M12 2.5v3.5" />
    <path d="M3.5 7h17" />
    <path d="M17 7c-1.6 6-6.4 11.4-13 14" />
    <path d="M7 7c1.6 6 6.4 11.4 13 14" />
  </svg>
);

/** 翻译: 文 at the top left, A at the bottom right, as translation apps draw it. */
const TranslateIcon = () => (
  <svg
    width={12}
    height={12}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth={2}
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <path d="M7 2v2" />
    <path d="M2 5h10" />
    <path d="M9.6 5C8.8 8.4 6.4 11.4 2.5 13" />
    <path d="M4.4 5c.8 3.4 3.2 6.4 7.1 8" />
    <path d="m13 22 4.5-11 4.5 11" />
    <path d="M14.6 18h5.8" />
  </svg>
);

const SourceIcon = ({ path }: { path?: string }) => {
  const [icon, setIcon] = useState(() => peekSourceAppIcon(path) ?? null);
  useEffect(() => {
    let alive = true;
    if (path) {
      void getSourceAppIcon(path).then((loaded) => {
        if (alive) setIcon(loaded);
      });
    }
    return () => {
      alive = false;
    };
  }, [path]);
  return icon ? <img className="card-app-icon" src={icon} alt="" draggable={false} /> : null;
};

/** Per entry, so a card scrolled back into view has its height right away. */
const imageSizes = new Map<number, ImageSize>();

const ImagePreview = ({ item, t }: { item: ClipboardEntry; t: Translate }) => {
  const missing = item.is_external && item.file_preview_exists === false;
  const src = useImageThumbnail(missing ? null : item.content, item.is_external, item.timestamp);
  const [failed, setFailed] = useState(false);
  const [size, setSize] = useState(() => imageSizes.get(item.id));
  if (missing || failed) {
    return (
      <div className="card-missing">
        <ImageOff size={18} />
        <span>{t("image_missing")}</span>
      </div>
    );
  }
  return (
    <div className="card-image-frame">
      {src && (
        <img
          src={src}
          alt=""
          className="card-image"
          style={size ? cardImageBox(size) : undefined}
          decoding="async"
          draggable={false}
          onLoad={(event) => {
            const { naturalWidth: width, naturalHeight: height } = event.currentTarget;
            if (!width || !height || (size?.width === width && size.height === height)) return;
            const loaded = { width, height };
            imageSizes.set(item.id, loaded);
            setSize(loaded);
          }}
          onError={() => setFailed(true)}
        />
      )}
    </div>
  );
};

const fileName = (path: string) => path.split(/[\\/]/).pop() || path;

const VideoPreview = ({ item }: { item: ClipboardEntry }) => {
  const cover = useVideoThumbnail(item.content, item.timestamp);
  return (
    <div className="card-file">
      <div className="card-video-cover">
        {cover && <img src={cover} alt="" draggable={false} />}
        <Play size={14} />
      </div>
      <span className="card-file-name">{fileName(item.content)}</span>
    </div>
  );
};

const FilePreview = ({ item, t }: { item: ClipboardEntry; t: Translate }) => {
  const paths = item.content.split("\n").filter(Boolean);
  const first = paths[0] ?? "";
  const [icon, setIcon] = useState(() => peekFileIcon(first) ?? null);
  useEffect(() => {
    let alive = true;
    void getFileIcon(first).then((loaded) => {
      if (alive) setIcon(loaded);
    });
    return () => {
      alive = false;
    };
  }, [first]);
  return (
    <div className="card-file">
      {icon ? <img className="card-file-icon" src={icon} alt="" draggable={false} /> : <FileText size={26} />}
      <span className="card-file-name">{fileName(first)}</span>
      {paths.length > 1 && (
        <span className="card-file-more">{t("more_files").replace("{count}", String(paths.length - 1))}</span>
      )}
    </div>
  );
};

/** The words found by a search, and the text they were found in. */
interface TextMatch {
  parts: TextPart[];
}

/** Text a search found a word in: the card's own preview, or else the fuller content. */
const findTextMatch = (item: ClipboardEntry, term: string): TextMatch | null => {
  if (!term.trim() || !isTextEntry(item.content_type)) return null;
  for (const text of [item.preview, item.content]) {
    if (!text) continue;
    const parts = splitMatches(text, term);
    if (hasMatch(parts)) return { parts };
  }
  return null;
};

/**
 * Card text with the searched words lit. The card shows three lines; when the
 * first word found is below them, the text scrolls up so it shows, one line of
 * context above it where there is room.
 */
const HighlightedText = ({ match, isCode }: { match: TextMatch; isCode: boolean }) => {
  const boxRef = useRef<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    const box = boxRef.current;
    const mark = box?.querySelector("mark");
    if (!box || !(mark instanceof HTMLElement)) return;
    const lineHeight = parseFloat(getComputedStyle(box).lineHeight) || 19;
    // Measured against the text box itself (offsetTop counts from the card).
    box.scrollTop = 0;
    const markTop = mark.getBoundingClientRect().top - box.getBoundingClientRect().top;
    const line = Math.floor(markTop / lineHeight + 0.01);
    const shownLines = Math.round(box.clientHeight / lineHeight);
    const middle = Math.floor((shownLines - 1) / 2);
    // Already showing: stay put. Otherwise the word's line goes to the middle,
    // except near the end, where the window stops at the last line.
    const maxLine = Math.max(0, Math.ceil((box.scrollHeight - box.clientHeight) / lineHeight));
    const firstLine = line < shownLines ? 0 : Math.min(line - middle, maxLine);
    box.scrollTop = firstLine * lineHeight;
    // Fades at the cut edges say there is more text above or below.
    box.classList.toggle("is-cut-top", box.scrollTop > 0);
    box.classList.toggle("is-cut-bottom", box.scrollTop + box.clientHeight < box.scrollHeight - 1);
  }, [match]);

  return (
    <div ref={boxRef} className={`card-text is-searched${isCode ? " is-code" : ""}`}>
      {match.parts.map((part, index) =>
        part.match ? (
          <mark key={index} className="search-hit">
            {part.text}
          </mark>
        ) : (
          part.text
        )
      )}
    </div>
  );
};

const CardBody = ({ item, t, match }: { item: ClipboardEntry; t: Translate; match: TextMatch | null }) => {
  switch (item.content_type) {
    case "image":
      return <ImagePreview item={item} t={t} />;
    case "video":
      return <VideoPreview item={item} />;
    case "file":
      return <FilePreview item={item} t={t} />;
    default:
      if (match) return <HighlightedText match={match} isCode={item.content_type === "code"} />;
      return (
        <div className={`card-text${item.content_type === "code" ? " is-code" : ""}`}>
          {item.preview || item.content}
        </div>
      );
  }
};

const ClipboardCard = ({
  item,
  t,
  selected,
  showSourceIcon,
  batchMode,
  batchChecked,
  onPaste,
  onToggleBatch,
  onContextMenu,
  onEdit,
  onTogglePin,
  onDelete,
  onPasteAsText,
  onTranslate,
  searchTerm,
}: ClipboardCardProps) => {
  const now = useNow();
  const match = useMemo(() => findTextMatch(item, searchTerm), [item, searchTerm]);
  // Images found by the text in them get the rim too, without any lit words.
  const searchHit = match !== null || (searchTerm.trim() !== "" && item.ocr_match === true);

  const action = (handler: (entry: ClipboardEntry) => void) => (event: MouseEvent) => {
    event.stopPropagation();
    handler(item);
  };

  return (
    <div
      id={`clipboard-item-${item.id}`}
      className={`card${selected ? " is-selected" : ""}${batchChecked ? " is-checked" : ""}${searchHit ? " is-search-hit" : ""}`}
      onClick={() => (batchMode ? onToggleBatch(item) : onPaste(item))}
      onContextMenu={(event) => {
        event.preventDefault();
        if (!batchMode) onContextMenu(item);
      }}
    >
      <div className="card-main">
        <div className="card-meta">
          <span className="card-app">
            {showSourceIcon && <SourceIcon path={item.source_app_path} />}
            <span className="card-app-name">{item.source_app}</span>
          </span>
          {!batchMode && (
            <span className="card-actions">
              {onTranslate && (item.content_type === "text" || item.content_type === "rich_text") && (
                <button
                  type="button"
                  className="card-action"
                  title={t("translate")}
                  aria-label={t("translate")}
                  onClick={action(onTranslate)}
                >
                  <TranslateIcon />
                </button>
              )}
              {item.content_type === "image" && (
                <button
                  type="button"
                  className="card-action"
                  title={t("paste_as_text")}
                  aria-label={t("paste_as_text")}
                  onClick={action(onPasteAsText)}
                >
                  <WenIcon />
                </button>
              )}
              {isEditableEntry(item.content_type) && (
                <button type="button" className="card-action" title={t("edit")} aria-label={t("edit")} onClick={action(onEdit)}>
                  <Pencil size={12} />
                </button>
              )}
              <button
                type="button"
                className={`card-action${item.is_pinned ? " is-active" : ""}`}
                title={item.is_pinned ? t("unpin_top") : t("pin_top")}
                aria-label={item.is_pinned ? t("unpin_top") : t("pin_top")}
                aria-pressed={item.is_pinned}
                onClick={action(onTogglePin)}
              >
                <ArrowUpToLine size={12} />
              </button>
              <button type="button" className="card-action is-danger" title={t("delete")} aria-label={t("delete")} onClick={action(onDelete)}>
                <Trash2 size={12} />
              </button>
            </span>
          )}
          <time className="card-time">{formatAge(item.created_at || item.timestamp, now)}</time>
        </div>
        <div className="card-rule" />
        <CardBody item={item} t={t} match={match} />
      </div>
      {batchMode && (
        <span className={`card-check${batchChecked ? " is-on" : ""}`} aria-hidden="true">
          {batchChecked && <Check size={12} strokeWidth={3} />}
        </span>
      )}
    </div>
  );
};

export default memo(ClipboardCard);
