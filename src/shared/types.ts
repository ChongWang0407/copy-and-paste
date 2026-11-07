export interface ClipboardEntry {
  id: number;
  /** text / rich_text / code / url / image / video / file */
  content_type: string;
  content: string;
  html_content?: string;
  source_app: string;
  source_app_path?: string;
  /** Ordering timestamp; mirrors sort_at. */
  timestamp: number;
  /** Immutable first-capture time. */
  created_at?: number;
  last_used_at?: number;
  sort_at?: number;
  preview: string;
  is_pinned: boolean;
  is_external?: boolean;
  pinned_order?: number;
  file_preview_exists?: boolean;
  /** Search results only: an image found by the text in it. */
  ocr_match?: boolean;
}

export interface FavoriteItem {
  id: number;
  content_type: string;
  content: string;
  html_content?: string | null;
  preview: string;
  source_app: string;
  sort_order: number;
  created_at: number;
  /** Absolute paths of a media favorite. */
  paths: string[];
}

export type Locale = "zh" | "en";

export interface ToastItem {
  id: number;
  msg: string;
}

export interface ConfirmDialogState {
  show: boolean;
  title: string;
  message: string;
  danger?: boolean;
  onConfirm: () => void;
}

export type Translate = (key: string) => string;

export type SidePanelKind = "favorites" | "edit" | "translate" | "batch" | "preview";
