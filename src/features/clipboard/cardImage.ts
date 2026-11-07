import type { CSSProperties } from "react";

export interface ImageSize {
  width: number;
  height: number;
}

/** Height limit of a picture in a card, in px. */
const CARD_IMAGE_HEIGHT = 128;
const TALL_IMAGE_HEIGHT_LIMIT = 280;

/**
 * Pictures show whole. Scaled to the usual height, a portrait screenshot ends
 * up a thin strip with nothing readable in it, so the card grows with the
 * picture's proportions, up to a limit.
 */
export const cardImageMaxHeight = (width: number, height: number): number => {
  if (width <= 0 || height <= width) return CARD_IMAGE_HEIGHT;
  return Math.min(TALL_IMAGE_HEIGHT_LIMIT, Math.round((CARD_IMAGE_HEIGHT * height) / width));
};

/**
 * The size a picture ends up at, known before it loads. Without it a card scrolled
 * back into view starts short and grows when the picture arrives, and the list
 * jumps. Matches what `max-width: 100%` and cardImageMaxHeight give a loaded one.
 */
export const cardImageBox = ({ width, height }: ImageSize): CSSProperties => {
  const fitted = Math.min(width, Math.round((cardImageMaxHeight(width, height) * width) / height));
  return { width: `min(100%, ${fitted}px)`, aspectRatio: `${width} / ${height}`, maxHeight: "none" };
};
