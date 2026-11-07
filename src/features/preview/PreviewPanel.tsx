import { useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { ImageOff } from "lucide-react";
import type { ClipboardEntry, Translate } from "../../shared/types";
import { originalImageSrc } from "../../shared/hooks/useImageThumbnail";
import { toLocalFilePath } from "../../shared/lib/localImageSrc";
import { startWindowDrag } from "../../shared/lib/windowDrag";

interface PreviewPanelProps {
  t: Translate;
  entry: ClipboardEntry;
}

const ImagePreview = ({ t, entry }: PreviewPanelProps) => {
  const [failed, setFailed] = useState(false);
  if (failed) {
    return (
      <div className="preview-message">
        <ImageOff size={20} />
        <span>{t("image_missing")}</span>
      </div>
    );
  }
  return (
    <img
      className="preview-image"
      src={originalImageSrc(entry.content, entry.is_external)}
      alt=""
      draggable={false}
      onError={() => setFailed(true)}
    />
  );
};

const VideoPreview = ({ t, entry }: PreviewPanelProps) => {
  const video = useRef<HTMLVideoElement | null>(null);
  const [failed, setFailed] = useState(false);
  const path = toLocalFilePath(entry.content.split("\n")[0] ?? "");

  useEffect(() => {
    const element = video.current;
    if (!element) return;
    // A key press may not count as permission to start with sound.
    element.play().catch(() => {
      element.muted = true;
      element.play().catch(() => {});
    });
  }, [path]);

  if (!path || failed) {
    return <div className="preview-message">{t("preview_video_failed")}</div>;
  }
  return (
    <video
      ref={video}
      className="preview-video"
      src={convertFileSrc(path)}
      controls
      playsInline
      onError={() => setFailed(true)}
    />
  );
};

const TextPreview = ({ entry }: { entry: ClipboardEntry }) => {
  const [text, setText] = useState(entry.content || entry.preview);

  // The list holds a shortened copy of long entries.
  useEffect(() => {
    let alive = true;
    invoke<string>("get_clipboard_content", { id: entry.id })
      .then((full) => {
        if (alive) setText(full);
      })
      .catch(console.error);
    return () => {
      alive = false;
    };
  }, [entry.id]);

  return <div className={`preview-text${entry.content_type === "code" ? " is-code" : ""}`}>{text}</div>;
};

const PreviewPanel = ({ t, entry }: PreviewPanelProps) => (
  <div className="side-panel-body">
    <div className="panel-title" onMouseDown={startWindowDrag}>
      <h2>{t("preview_title")}</h2>
    </div>
    <div className="panel-scroll">
      {entry.content_type === "image" && <ImagePreview t={t} entry={entry} />}
      {entry.content_type === "video" && <VideoPreview t={t} entry={entry} />}
      {entry.content_type !== "image" && entry.content_type !== "video" && <TextPreview entry={entry} />}
    </div>
  </div>
);

export default PreviewPanel;
