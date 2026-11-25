import { describe, expect, test } from "vitest";
import { assetUrlToFilePath, toLocalFilePath } from "./localImageSrc";

describe("toLocalFilePath", () => {
  test("file URLs keep the root of a macOS path", () => {
    expect(toLocalFilePath("file:///Users/me/a%20b.png")).toBe("/Users/me/a b.png");
    expect(toLocalFilePath("file://localhost/Users/me/a.png")).toBe("/Users/me/a.png");
    expect(toLocalFilePath("file:/Users/me/a.png")).toBe("/Users/me/a.png");
  });

  test("Windows file URLs drop the slash before the drive", () => {
    expect(toLocalFilePath("file:///C:/Users/me/a.png")).toBe("C:/Users/me/a.png");
  });

  test("relative file URLs are not local paths", () => {
    expect(toLocalFilePath("file://server/share/a.png")).toBeNull();
  });
});

describe("assetUrlToFilePath", () => {
  test("macOS / Linux asset URL", () => {
    expect(assetUrlToFilePath(`asset://localhost/${encodeURIComponent("/Users/me/Library/CopyPaste/attachments/a b.png")}`))
      .toBe("/Users/me/Library/CopyPaste/attachments/a b.png");
  });

  test("Windows asset URL (http and https schemes)", () => {
    const encoded = encodeURIComponent("C:\\Users\\me\\AppData\\Roaming\\com.wangchong.copypaste\\attachments\\图片.png");
    expect(assetUrlToFilePath(`http://asset.localhost/${encoded}`))
      .toBe("C:\\Users\\me\\AppData\\Roaming\\com.wangchong.copypaste\\attachments\\图片.png");
    expect(assetUrlToFilePath(`https://asset.localhost/${encoded}`))
      .toBe("C:\\Users\\me\\AppData\\Roaming\\com.wangchong.copypaste\\attachments\\图片.png");
  });

  test("drops a cache-busting query", () => {
    expect(assetUrlToFilePath(`asset://localhost/${encodeURIComponent("/tmp/x.png")}?rev=3`)).toBe("/tmp/x.png");
  });

  test("plain local paths pass through; remote and data sources are not files", () => {
    expect(assetUrlToFilePath("/tmp/x.png")).toBe("/tmp/x.png");
    expect(assetUrlToFilePath("C:\\x.png")).toBe("C:\\x.png");
    expect(assetUrlToFilePath("https://example.com/x.png")).toBeNull();
    expect(assetUrlToFilePath("data:image/png;base64,AAAA")).toBeNull();
  });
});
