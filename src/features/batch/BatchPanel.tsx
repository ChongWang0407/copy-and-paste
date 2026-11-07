import { useEffect, useMemo, useRef, useState } from "react";
import { Reorder, useDragControls } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { ChevronUp, FileText, GripVertical, Play, X } from "lucide-react";
import type { ClipboardEntry, Translate } from "../../shared/types";
import { isTextEntry } from "../../shared/lib/clipboardTypes";
import { useImageThumbnail, useVideoThumbnail } from "../../shared/hooks/useImageThumbnail";
import { showNativeMenu, type NativeMenuItem } from "../../shared/lib/nativeMenu";
import { startWindowDrag } from "../../shared/lib/windowDrag";
import { buildBatchSegments, DEFAULT_CONNECTOR, CONNECTORS, type ConnectorId } from "./joinBatch";

interface BatchPanelProps {
  t: Translate;
  items: ClipboardEntry[];
  onReorder: (items: ClipboardEntry[]) => void;
  onRemove: (item: ClipboardEntry) => void;
  deleteAfterPaste: boolean;
  moveToTopAfterPaste: boolean;
  onPasteFailed: (error: unknown) => void;
  onClose: () => void;
}

const PUNCTUATION: ConnectorId[] = ["comma", "period", "semicolon", "caesura"];

const fileName = (path: string) => path.split(/[\\/]/).pop() || path;

/** The connector as it will look between two items. */
const connectorGlyph = (connector: ConnectorId) => {
  if (connector === "newline") return "↵";
  if (connector === "space") return "␣";
  return CONNECTORS[connector];
};

const BatchMedia = ({ item }: { item: ClipboardEntry }) => {
  const image = useImageThumbnail(item.content_type === "image" ? item.content : null, item.is_external, item.timestamp);
  const video = useVideoThumbnail(item.content_type === "video" ? item.content : null, item.timestamp);
  if (item.content_type === "image") {
    return image ? <img className="batch-image" src={image} alt="" draggable={false} /> : null;
  }
  if (item.content_type === "video") {
    return (
      <span className="batch-file">
        <span className="favorite-thumb is-video">
          {video && <img src={video} alt="" draggable={false} />}
          <Play size={12} />
        </span>
        {fileName(item.content)}
      </span>
    );
  }
  return (
    <span className="batch-file">
      <FileText size={18} />
      {fileName(item.content.split("\n")[0] ?? "")}
    </span>
  );
};

const BatchRow = ({
  item,
  t,
  connector,
  onRemove,
  onDragEnd,
}: {
  item: ClipboardEntry;
  t: Translate;
  /** Shown after the item; null after the last one. */
  connector: string | null;
  onRemove: () => void;
  onDragEnd: () => void;
}) => {
  const dragControls = useDragControls();
  return (
    <Reorder.Item value={item} dragListener={false} dragControls={dragControls} onDragEnd={onDragEnd} className="batch-row">
      <div className="batch-item">
        <span
          className="favorite-grip"
          title={t("favorites_drag")}
          onPointerDown={(event) => {
            event.preventDefault();
            dragControls.start(event);
          }}
        >
          <GripVertical size={14} />
        </span>
        <span className="batch-content">
          {isTextEntry(item.content_type) ? (
            <span className="batch-text">{item.preview || item.content}</span>
          ) : (
            <BatchMedia item={item} />
          )}
        </span>
        <button type="button" className="card-action" title={t("batch_remove")} aria-label={t("batch_remove")} onClick={onRemove}>
          <X size={12} />
        </button>
      </div>
      {connector && <div className="batch-connector">{connector}</div>}
    </Reorder.Item>
  );
};

const BatchPanel = ({
  t,
  items,
  onReorder,
  onRemove,
  deleteAfterPaste,
  moveToTopAfterPaste,
  onPasteFailed,
  onClose,
}: BatchPanelProps) => {
  const [connector, setConnector] = useState<ConnectorId>(DEFAULT_CONNECTOR);
  const [pasting, setPasting] = useState(false);
  const connectorButton = useRef<HTMLButtonElement | null>(null);
  const [localOrder, setLocalOrder] = useState(items);
  useEffect(() => setLocalOrder(items), [items]);

  const glyph = useMemo(() => connectorGlyph(connector), [connector]);

  const openConnectorMenu = () => {
    const option = (id: ConnectorId, text: string): NativeMenuItem => ({
      text,
      checked: connector === id,
      action: () => setConnector(id),
    });
    const box = connectorButton.current?.getBoundingClientRect();
    // Opens upwards: the button is at the bottom of the window.
    const menuHeight = 3 * 24 + 12;
    void showNativeMenu(
      [
        option("newline", t("batch_connector_newline")),
        option("space", t("batch_connector_space")),
        {
          text: t("batch_connector_punctuation"),
          children: PUNCTUATION.map((id) => option(id, CONNECTORS[id])),
        },
      ],
      box ? { x: box.left, y: Math.max(0, box.top - menuHeight) } : undefined
    );
  };

  const paste = async () => {
    setPasting(true);
    try {
      // The list holds a shortened copy of long entries.
      const full = await Promise.all(
        localOrder.map(async (item) => ({
          id: item.id,
          contentType: item.content_type,
          text: isTextEntry(item.content_type)
            ? await invoke<string>("get_clipboard_content", { id: item.id }).catch(() => item.content)
            : "",
        }))
      );
      await invoke("paste_batch", {
        segments: buildBatchSegments(full, connector),
        sourceIds: localOrder.map((item) => item.id),
        deleteAfterUse: deleteAfterPaste,
        moveToTop: moveToTopAfterPaste,
      });
      onClose();
    } catch (error) {
      onPasteFailed(error);
    } finally {
      setPasting(false);
    }
  };

  return (
    <div className="side-panel-body">
      <div className="panel-title" onMouseDown={startWindowDrag}>
        <h2>{t("batch_title")}</h2>
      </div>
      <div className="panel-fill">
        <div className="batch-preview">
          {localOrder.length === 0 ? (
            <div className="empty-state">
              <p className="empty-hint">{t("batch_empty")}</p>
            </div>
          ) : (
            <Reorder.Group axis="y" values={localOrder} onReorder={setLocalOrder} className="batch-list">
              {localOrder.map((item, index) => (
                <BatchRow
                  key={item.id}
                  item={item}
                  t={t}
                  connector={index < localOrder.length - 1 ? glyph : null}
                  onRemove={() => onRemove(item)}
                  onDragEnd={() => onReorder(localOrder)}
                />
              ))}
            </Reorder.Group>
          )}
        </div>
      </div>
      <div className="panel-actions">
        <button ref={connectorButton} type="button" className="glass-button" onClick={openConnectorMenu}>
          {t("batch_connector")}
          <ChevronUp size={13} />
        </button>
        <button
          type="button"
          className="glass-button is-primary"
          disabled={localOrder.length === 0 || pasting}
          onClick={() => void paste()}
        >
          {t("batch_paste_action")}
        </button>
      </div>
    </div>
  );
};

export default BatchPanel;
