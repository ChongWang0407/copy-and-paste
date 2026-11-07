import { isTextEntry } from "../../shared/lib/clipboardTypes";

export type BatchSegment = { kind: "text"; text: string } | { kind: "entry"; id: number };

export interface BatchItem {
  id: number;
  contentType: string;
  /** Full text of a text entry. */
  text: string;
}

export const CONNECTORS = {
  space: " ",
  newline: "\n",
  comma: "，",
  period: "。",
  semicolon: "；",
  caesura: "、",
} as const;

export type ConnectorId = keyof typeof CONNECTORS;
export const DEFAULT_CONNECTOR: ConnectorId = "newline";

/**
 * What a batch pastes, step by step. Text and connectors merge into one step
 * (pure text is a single paste); images, videos and files are a step each.
 */
export const buildBatchSegments = (items: BatchItem[], connector: ConnectorId): BatchSegment[] => {
  const segments: BatchSegment[] = [];
  let text = "";
  items.forEach((item, index) => {
    if (index > 0) text += CONNECTORS[connector];
    if (isTextEntry(item.contentType)) {
      text += item.text;
      return;
    }
    if (text) segments.push({ kind: "text", text });
    text = "";
    segments.push({ kind: "entry", id: item.id });
  });
  if (text) segments.push({ kind: "text", text });
  return segments;
};
