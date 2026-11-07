import { describe, expect, it } from "vitest";
import { buildBatchSegments, type BatchItem } from "./joinBatch";

const text = (id: number, value: string): BatchItem => ({ id, contentType: "text", text: value });
const image = (id: number): BatchItem => ({ id, contentType: "image", text: "" });

describe("buildBatchSegments", () => {
  it("merges pure text into one paste", () => {
    expect(buildBatchSegments([text(1, "a"), text(2, "b"), text(3, "c")], "newline")).toEqual([
      { kind: "text", text: "a\nb\nc" },
    ]);
    expect(buildBatchSegments([text(1, "a"), text(2, "b")], "caesura")).toEqual([
      { kind: "text", text: "a、b" },
    ]);
  });

  it("gives an image its own step, between the text around it", () => {
    expect(buildBatchSegments([text(1, "a"), image(2), text(3, "b")], "newline")).toEqual([
      { kind: "text", text: "a\n" },
      { kind: "entry", id: 2 },
      { kind: "text", text: "\nb" },
    ]);
  });

  it("keeps the connector between two images", () => {
    expect(buildBatchSegments([image(1), image(2)], "newline")).toEqual([
      { kind: "entry", id: 1 },
      { kind: "text", text: "\n" },
      { kind: "entry", id: 2 },
    ]);
  });

  it("does nothing for an empty selection", () => {
    expect(buildBatchSegments([], "newline")).toEqual([]);
  });
});
