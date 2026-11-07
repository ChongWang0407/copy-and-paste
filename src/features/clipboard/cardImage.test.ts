import { describe, expect, it } from "vitest";
import { cardImageBox, cardImageMaxHeight } from "./cardImage";

describe("cardImageMaxHeight", () => {
  it("keeps the usual height for landscape and square pictures", () => {
    expect(cardImageMaxHeight(1280, 720)).toBe(128);
    expect(cardImageMaxHeight(500, 500)).toBe(128);
  });

  it("grows with a portrait picture's proportions", () => {
    expect(cardImageMaxHeight(684, 1230)).toBe(230);
  });

  it("stops at the limit for very long pictures", () => {
    expect(cardImageMaxHeight(400, 8000)).toBe(280);
  });

  it("falls back to the usual height without a size", () => {
    expect(cardImageMaxHeight(0, 0)).toBe(128);
  });
});

describe("cardImageBox", () => {
  it("reserves the size a loaded picture would take", () => {
    expect(cardImageBox({ width: 800, height: 400 }).width).toBe("min(100%, 256px)");
    expect(cardImageBox({ width: 445, height: 800 }).width).toBe("min(100%, 128px)");
    expect(cardImageBox({ width: 800, height: 400 }).aspectRatio).toBe("800 / 400");
  });

  it("never enlarges a small picture", () => {
    expect(cardImageBox({ width: 60, height: 40 }).width).toBe("min(100%, 60px)");
  });
});
