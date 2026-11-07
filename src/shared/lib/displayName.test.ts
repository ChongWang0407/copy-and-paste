import { describe, expect, it } from "vitest";
import { clampDisplayName, displayNameWidth } from "./displayName";

describe("display name", () => {
  it("counts Chinese as two", () => {
    expect(displayNameWidth("Copy & Paste")).toBe(12);
    expect(displayNameWidth("剪贴板")).toBe(6);
  });

  it("cuts what does not fit", () => {
    expect(clampDisplayName("abcdefghijklmnopqrst")).toBe("abcdefghijklmnop");
    expect(clampDisplayName("一二三四五六七八九")).toBe("一二三四五六七八");
    expect(clampDisplayName("Copy & Paste")).toBe("Copy & Paste");
  });
});
