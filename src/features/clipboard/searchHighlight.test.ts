import { describe, expect, it } from "vitest";
import { hasMatch, splitMatches } from "./searchHighlight";

describe("splitMatches", () => {
  it("marks every occurrence", () => {
    expect(splitMatches("固定在这里，再固定", "固定")).toEqual([
      { text: "固定", match: true },
      { text: "在这里，再", match: false },
      { text: "固定", match: true },
    ]);
  });

  it("ignores case and keeps the original letters", () => {
    expect(splitMatches("Open the PDF", "pdf")).toEqual([
      { text: "Open the ", match: false },
      { text: "PDF", match: true },
    ]);
  });

  it("treats the term literally", () => {
    expect(hasMatch(splitMatches("a+b=c", "a+b"))).toBe(true);
    expect(hasMatch(splitMatches("aab", "a+b"))).toBe(false);
  });

  it("leaves text alone without a term or a match", () => {
    expect(splitMatches("固定", "  ")).toEqual([{ text: "固定", match: false }]);
    expect(hasMatch(splitMatches("固定", "移动"))).toBe(false);
  });
});
