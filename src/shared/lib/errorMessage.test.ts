import { describe, expect, test } from "vitest";
import { errorToText, formatError, localizeErrorTags, tidyMessage } from "./errorMessage";

const t = (key: string) => ({ error_kind_io: "文件错误", error_kind_internal: "内部错误" } as Record<string, string>)[key] ?? key;

describe("formatError", () => {
  test("translates the backend tag", () => {
    expect(formatError("[io] No such file or directory (os error 2)", t)).toBe("文件错误: No such file or directory (os error 2)");
  });

  test("translates legacy Chinese prefixes too", () => {
    expect(formatError("内部系统错误: boom", t)).toBe("内部错误: boom");
  });

  test("drops the label when no translation exists", () => {
    expect(formatError("[network] timed out", t)).toBe("timed out");
  });

  test("strips an HTML error page and caps the length", () => {
    const page = "<!DOCTYPE html><html><head><title>502 Bad Gateway</title></head><body><h1>502 Bad Gateway</h1>" +
      "<!-- padding -->".repeat(50) + "x".repeat(500) + "</body></html>";
    const out = formatError(`[internal] AI API error - ${page}`, t);
    expect(out).not.toContain("<");
    expect(out.length).toBeLessThanOrEqual(200);
    expect(out.endsWith("…")).toBe(true);
  });

  test("tags in the middle of a composed message are translated", () => {
    expect(localizeErrorTags("打开失败: [io] gone", t)).toBe("打开失败: 文件错误: gone");
  });
});

describe("errorToText", () => {
  test("handles Error, objects and nullish values", () => {
    expect(errorToText(new Error("bad"))).toBe("bad");
    expect(errorToText({ message: "obj" })).toBe("obj");
    expect(errorToText({ code: 1 })).toBe('{"code":1}');
    expect(errorToText(undefined)).toBe("");
  });

  test("tidyMessage collapses whitespace", () => {
    expect(tidyMessage("a\n\n   b\tc")).toBe("a b c");
  });
});
