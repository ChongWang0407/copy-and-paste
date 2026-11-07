import { useEffect, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { toLocalFilePath, toTauriLocalImageSrc } from "../lib/localImageSrc";
import { isTauriRuntime } from "../lib/tauriRuntime";

// Originals are often 4K screenshots that take tens of ms to decode on every
// mount. get_image_thumbnail returns a cached copy sharp enough for a card on a
// Retina screen (or the original when it is already small). Results are kept in
// memory so a card scrolled back into view gets its src synchronously.
const resolved = new Map<string, string>();
const inflight = new Map<string, Promise<string>>();
const MAX_RESOLVED = 600;

const loadThumbnail = (path: string, key: string): Promise<string> => {
  let pending = inflight.get(key);
  if (!pending) {
    pending = invoke<string>("get_image_thumbnail", { path })
      .then((thumbPath) => convertFileSrc(thumbPath))
      .catch(() => convertFileSrc(path))
      .then((src) => {
        if (resolved.size >= MAX_RESOLVED) {
          const oldest = resolved.keys().next().value;
          if (oldest !== undefined) resolved.delete(oldest);
        }
        resolved.set(key, src);
        inflight.delete(key);
        return src;
      });
    inflight.set(key, pending);
  }
  return pending;
};

export const originalImageSrc = (content: string, isExternal?: boolean) =>
  content.startsWith("data:")
    ? content
    : toTauriLocalImageSrc(content) || (isExternal ? convertFileSrc(content) : content);

/**
 * Image src for a list card. Returns null only while a local file's thumbnail
 * is looked up for the first time; callers should keep the card's space reserved.
 */
export const useImageThumbnail = (
  content: string | null,
  isExternal?: boolean,
  /** Changes when the entry changes (e.g. its timestamp), so an updated image
   * isn't served from this session cache. */
  version?: number
): string | null => {
  const path = content && isTauriRuntime() ? toLocalFilePath(content) : null;
  const key = path ? `${path}|${version ?? 0}` : null;
  const fallback = content ? originalImageSrc(content, isExternal) : null;
  const [src, setSrc] = useState<string | null>(() => (key ? resolved.get(key) ?? null : fallback));

  useEffect(() => {
    if (!path || !key) {
      setSrc(fallback);
      return;
    }
    const cached = resolved.get(key);
    if (cached) {
      setSrc(cached);
      return;
    }
    let alive = true;
    setSrc(null);
    void loadThumbnail(path, key).then((next) => {
      if (alive) setSrc(next);
    });
    return () => {
      alive = false;
    };
  }, [path, key, fallback]);

  return src;
};

// Video covers come from the system thumbnailer (get_video_thumbnail). "" marks
// a video the system couldn't preview, so the card falls back to <video>.
const videoResolved = new Map<string, string>();
const videoInflight = new Map<string, Promise<string>>();

const loadVideoThumbnail = (path: string, key: string): Promise<string> => {
  let pending = videoInflight.get(key);
  if (!pending) {
    pending = invoke<string>("get_video_thumbnail", { path })
      .then((thumbPath) => convertFileSrc(thumbPath))
      .catch(() => "")
      .then((src) => {
        if (videoResolved.size >= MAX_RESOLVED) {
          const oldest = videoResolved.keys().next().value;
          if (oldest !== undefined) videoResolved.delete(oldest);
        }
        videoResolved.set(key, src);
        videoInflight.delete(key);
        return src;
      });
    videoInflight.set(key, pending);
  }
  return pending;
};

/**
 * Cover image for a video card: a src once the system made one, `""` when it
 * couldn't (show the video element instead), `null` while still looking.
 */
export const useVideoThumbnail = (content: string | null, version?: number): string | null => {
  const path = content && isTauriRuntime() && !content.startsWith("data:") ? toLocalFilePath(content) : null;
  const key = path ? `${path}|${version ?? 0}` : null;
  const [src, setSrc] = useState<string | null>(() => (key ? videoResolved.get(key) ?? null : ""));

  useEffect(() => {
    if (!path || !key) {
      setSrc("");
      return;
    }
    const cached = videoResolved.get(key);
    if (cached !== undefined) {
      setSrc(cached);
      return;
    }
    let alive = true;
    setSrc(null);
    void loadVideoThumbnail(path, key).then((next) => {
      if (alive) setSrc(next);
    });
    return () => {
      alive = false;
    };
  }, [path, key]);

  return src;
};
