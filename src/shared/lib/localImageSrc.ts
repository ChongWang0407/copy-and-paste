import { convertFileSrc } from "@tauri-apps/api/core";

const decodeUriComponentSafely = (value: string, rounds = 2): string => {
  let out = value;
  for (let i = 0; i < rounds; i++) {
    if (!/%[0-9a-fA-F]{2}/.test(out)) break;
    try {
      const decoded = decodeURIComponent(out);
      if (decoded === out) break;
      out = decoded;
    } catch {
      break;
    }
  }
  return out;
};

/** Absolute filesystem path for a local image reference, or null for data:, http:, etc. */
export const toLocalFilePath = (rawSrc: string): string | null => {
  let value = (rawSrc || "").trim();
  if (!value) return null;

  // Non-local sources should not be converted.
  if (/^(data:|https?:|blob:|asset:|tauri:)/i.test(value)) {
    return null;
  }

  value = decodeUriComponentSafely(value, 2);

  if (/^file:/i.test(value)) {
    value = value.replace(/^file:\/+/i, "");
    if (/^\/[a-zA-Z]:/.test(value)) {
      value = value.slice(1);
    }
  }

  if (!/^[a-zA-Z]:[\\/]/.test(value) && !value.startsWith("/")) {
    return null;
  }

  return value;
};

/**
 * Inverse of `convertFileSrc`: the filesystem path behind an asset URL
 * (`asset://localhost/<enc>` on macOS/Linux, `http(s)://asset.localhost/<enc>`
 * on Windows), or a plain local path as-is. Null for anything else.
 */
export const assetUrlToFilePath = (src: string): string | null => {
  const match = /^(?:asset:\/\/localhost|https?:\/\/asset\.localhost)\/(.+)$/i.exec((src || "").trim());
  if (!match) return toLocalFilePath(src);
  const path = decodeUriComponentSafely(match[1].split(/[?#]/)[0], 2);
  // Unix paths are encoded with their leading "/" ("%2FUsers/..."), Windows
  // paths start with the drive letter.
  return /^[a-zA-Z]:[\\/]/.test(path) || path.startsWith("/") ? path : `/${path}`;
};

export const toTauriLocalImageSrc = (rawSrc: string): string | null => {
  const path = toLocalFilePath(rawSrc);
  return path ? convertFileSrc(path) : null;
};

export const withImageCacheBust = (
  src: string | null | undefined,
  revision?: number | null
): string | null => {
  if (!src) return null;
  if (/^(data:|blob:|https?:|asset:|tauri:)/i.test(src)) return src;
  const token = revision ?? Date.now();
  const joiner = src.includes("?") ? "&" : "?";
  return `${src}${joiner}rev=${token}`;
};

