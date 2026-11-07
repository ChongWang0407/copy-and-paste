import { useEffect, useRef } from "react";
import type { ClipboardEntry } from "../../shared/types";
import { hasOpenDialog, isField } from "../app/useKeyboard";

/** A shorter press is just a key tap. */
const HOLD_MS = 250;
const CARD_ID_PREFIX = "clipboard-item-";

/** Files have no preview yet. */
export const canPreview = (entry: ClipboardEntry) => entry.content_type !== "file";

export const entryIdFromCard = (elementId: string | undefined): number | null => {
  if (!elementId?.startsWith(CARD_ID_PREFIX)) return null;
  const raw = elementId.slice(CARD_ID_PREFIX.length);
  const id = Number(raw);
  return raw && Number.isInteger(id) ? id : null;
};

interface SpacePreviewOptions {
  enabled: boolean;
  items: ClipboardEntry[];
  selectedIndex: number | null;
  onOpen: (entry: ClipboardEntry) => void;
  onClose: () => void;
}

/** The card under the mouse, or else the one picked with the arrow keys. */
const pickEntry = ({ items, selectedIndex }: SpacePreviewOptions) => {
  const hovered = entryIdFromCard(document.querySelector<HTMLElement>(".card:hover")?.id);
  if (hovered !== null) return items.find((item) => item.id === hovered) ?? null;
  return selectedIndex === null ? null : items[selectedIndex] ?? null;
};

/** Holding Space previews an entry; letting go puts it away. */
export const useSpacePreview = (options: SpacePreviewOptions) => {
  const ref = useRef(options);
  ref.current = options;

  useEffect(() => {
    let timer: number | null = null;
    let pressed = false;
    let previewing = false;

    const release = () => {
      pressed = false;
      if (timer !== null) {
        window.clearTimeout(timer);
        timer = null;
      }
      if (previewing) {
        previewing = false;
        ref.current.onClose();
      }
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.code !== "Space" || event.defaultPrevented || event.isComposing) return;
      if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return;
      if (!ref.current.enabled || isField(event.target) || hasOpenDialog()) return;

      // Space would also scroll the list, or click a focused button.
      event.preventDefault();
      if (pressed) return;
      pressed = true;
      timer = window.setTimeout(() => {
        timer = null;
        const entry = pickEntry(ref.current);
        if (!entry || !canPreview(entry)) return;
        previewing = true;
        ref.current.onOpen(entry);
      }, HOLD_MS);
    };

    const onKeyUp = (event: KeyboardEvent) => {
      if (event.code !== "Space" || !pressed) return;
      event.preventDefault();
      release();
    };

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    // The key-up goes to whatever took the focus.
    window.addEventListener("blur", release);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("blur", release);
      release();
    };
  }, []);
};
