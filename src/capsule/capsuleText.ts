// Words for the copy capsule. Kept apart from locales.ts so the capsule page
// stays small.

export type CapsuleLocale = "zh" | "en";

export interface CapsulePayload {
  kind: string;
  preview: string;
  appName: string;
  appIcon: string | null;
  imagePath: string | null;
  fileCount: number;
  style: "glass" | "island" | "black";
  language: string;
  count: number;
  fresh: boolean;
  seq: number;
  width: number;
  height: number;
  topInset: number;
}

const TITLES: Record<CapsuleLocale, Record<string, string>> = {
  zh: {
    text: "已复制文本",
    rich_text: "已复制文本",
    code: "已复制代码",
    url: "已复制链接",
    image: "已复制图片",
    video: "已复制视频",
    file: "已复制文件",
    files: "已复制 {n} 个文件",
    sample: "复制提示弹窗",
    other: "已复制",
  },
  en: {
    text: "Text copied",
    rich_text: "Text copied",
    code: "Code copied",
    url: "Link copied",
    image: "Image copied",
    video: "Video copied",
    file: "File copied",
    files: "{n} files copied",
    sample: "Copy Notice",
    other: "Copied",
  },
};

const SAMPLE_SUBTITLE: Record<CapsuleLocale, string> = {
  zh: "每次复制，都会在这里出现",
  en: "Shows up here every time you copy",
};

const FROM: Record<CapsuleLocale, string> = { zh: "来自 {app}", en: "From {app}" };

export const capsuleLocale = (language: string): CapsuleLocale => {
  if (language === "zh" || language === "en") return language;
  return navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
};

export const capsuleTitle = (payload: Pick<CapsulePayload, "kind" | "fileCount">, locale: CapsuleLocale) => {
  const titles = TITLES[locale];
  if (payload.kind === "file" && payload.fileCount > 1) {
    return titles.files.replace("{n}", String(payload.fileCount));
  }
  return titles[payload.kind] ?? titles.other;
};

/** Generic names the capture falls back to when it can't tell the app. */
const UNKNOWN_APPS = new Set(["", "unknown", "macos app"]);

export const knownAppName = (name: string) => (UNKNOWN_APPS.has(name.trim().toLowerCase()) ? "" : name.trim());

export const capsuleSubtitle = (
  payload: Pick<CapsulePayload, "kind" | "preview" | "appName">,
  locale: CapsuleLocale
) => {
  if (payload.kind === "sample") return SAMPLE_SUBTITLE[locale];
  if (payload.preview.trim()) return payload.preview.trim();
  const app = knownAppName(payload.appName);
  return app ? FROM[locale].replace("{app}", app) : "";
};
