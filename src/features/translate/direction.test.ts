import { describe, expect, it } from "vitest";
import { detectDirection } from "./direction";

const zhToEn = { source: "zh-Hans", target: "en" };
const enToZh = { source: "en", target: "zh-Hans" };

describe("translate direction", () => {
  it("translates Chinese to English", () => {
    expect(detectDirection("系统会自己弹出下载确认框")).toEqual(zhToEn);
  });

  it("translates English to Chinese", () => {
    expect(detectDirection("The system asks before it downloads.")).toEqual(enToZh);
  });

  it("treats Chinese with product names as Chinese", () => {
    expect(detectDirection("macOS 15/26 以上")).toEqual(zhToEn);
    expect(detectDirection("用 Translation 框架翻译")).toEqual(zhToEn);
  });

  it("treats English with a stray Chinese word as English", () => {
    expect(detectDirection("Open the settings page and choose 中文 as the language")).toEqual(enToZh);
  });

  it("defaults to English → Chinese without letters", () => {
    expect(detectDirection("12345 !!")).toEqual(enToZh);
  });
});
