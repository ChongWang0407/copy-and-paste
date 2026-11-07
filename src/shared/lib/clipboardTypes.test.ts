import { describe, expect, it } from "vitest";
import { cycleTypeFilter, entryMatchesType, isEditableEntry, toggleTypeFilter, TYPE_FILTERS } from "./clipboardTypes";

describe("type tabs", () => {
  it("keeps the order of the tabs", () => {
    expect(TYPE_FILTERS).toEqual(["text", "image", "video", "file", "url", "code"]);
  });

  it("turns off when the active tab is clicked again", () => {
    expect(toggleTypeFilter(null, "image")).toBe("image");
    expect(toggleTypeFilter("image", "image")).toBeNull();
    expect(toggleTypeFilter("image", "code")).toBe("code");
  });

  it("cycles through no filter", () => {
    expect(cycleTypeFilter(null, 1)).toBe("text");
    expect(cycleTypeFilter("code", 1)).toBeNull();
    expect(cycleTypeFilter(null, -1)).toBe("code");
  });

  it("counts rich text as text", () => {
    expect(entryMatchesType("rich_text", "text")).toBe(true);
    expect(entryMatchesType("rich_text", "code")).toBe(false);
    expect(entryMatchesType("image", null)).toBe(true);
  });

  it("offers editing for text, code and links only", () => {
    expect(["text", "rich_text", "code", "url"].every(isEditableEntry)).toBe(true);
    expect(["image", "video", "file"].some(isEditableEntry)).toBe(false);
  });
});
