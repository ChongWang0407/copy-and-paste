import { describe, expect, it } from "vitest";
import type { ClipboardEntry } from "../../shared/types";
import { canPreview, entryIdFromCard } from "./useSpacePreview";

const entry = (content_type: string) => ({ content_type }) as ClipboardEntry;

describe("space preview", () => {
  it("previews everything but files", () => {
    for (const type of ["text", "rich_text", "code", "url", "image", "video"]) {
      expect(canPreview(entry(type))).toBe(true);
    }
    expect(canPreview(entry("file"))).toBe(false);
  });

  it("reads the entry id from a card", () => {
    expect(entryIdFromCard("clipboard-item-42")).toBe(42);
    expect(entryIdFromCard("clipboard-item-")).toBeNull();
    expect(entryIdFromCard("clipboard-item-x")).toBeNull();
    expect(entryIdFromCard("something-else")).toBeNull();
    expect(entryIdFromCard(undefined)).toBeNull();
  });
});
