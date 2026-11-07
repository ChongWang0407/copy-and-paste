import { describe, expect, it } from "vitest";
import { ACCENT_NAMES, applyAccent, DEFAULT_ACCENT, normalizeAccent } from "./accent";

describe("accent", () => {
  it("offers six colours, blue first and default", () => {
    expect(ACCENT_NAMES).toHaveLength(6);
    expect(ACCENT_NAMES[0]).toBe("blue");
    expect(DEFAULT_ACCENT).toBe("blue");
  });

  it("falls back to blue for unknown or missing values", () => {
    expect(normalizeAccent(undefined)).toBe("blue");
    expect(normalizeAccent("pink")).toBe("blue");
    expect(normalizeAccent("toString")).toBe("blue");
    expect(normalizeAccent("green")).toBe("green");
  });

  it("writes the CSS variables, with rgb triplets", () => {
    const style = new Map<string, string>();
    const root = { style: { setProperty: (k: string, v: string) => style.set(k, v) } } as unknown as HTMLElement;
    applyAccent("blue", root);
    expect(style.get("--accent")).toBe("#0a84ff");
    expect(style.get("--accent-rgb")).toBe("10, 132, 255");
    expect(style.get("--accent-strong-rgb")).toBe("0, 102, 224");
  });
});
