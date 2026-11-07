import { describe, expect, it } from "vitest";
import { capsuleLocale, capsuleSubtitle, capsuleTitle, knownAppName } from "./capsuleText";

describe("capsule text", () => {
  it("names what was copied", () => {
    expect(capsuleTitle({ kind: "image", fileCount: 0 }, "zh")).toBe("已复制图片");
    expect(capsuleTitle({ kind: "url", fileCount: 0 }, "en")).toBe("Link copied");
    expect(capsuleTitle({ kind: "file", fileCount: 3 }, "zh")).toBe("已复制 3 个文件");
    expect(capsuleTitle({ kind: "file", fileCount: 1 }, "en")).toBe("File copied");
    expect(capsuleTitle({ kind: "something-new", fileCount: 0 }, "en")).toBe("Copied");
  });

  it("shows the preview, else where it came from", () => {
    expect(capsuleSubtitle({ kind: "text", preview: " hello ", appName: "Safari" }, "en")).toBe("hello");
    expect(capsuleSubtitle({ kind: "image", preview: "", appName: "Safari" }, "zh")).toBe("来自 Safari");
    expect(capsuleSubtitle({ kind: "image", preview: "", appName: "macOS App" }, "zh")).toBe("");
  });

  it("drops the capture's placeholder app names", () => {
    expect(knownAppName("Unknown")).toBe("");
    expect(knownAppName(" Notes ")).toBe("Notes");
  });

  it("uses the saved language when there is one", () => {
    expect(capsuleLocale("zh")).toBe("zh");
    expect(capsuleLocale("en")).toBe("en");
  });
});
