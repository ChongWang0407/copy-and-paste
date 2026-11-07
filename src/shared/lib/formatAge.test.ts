import { describe, expect, it } from "vitest";
import { formatAge } from "./formatAge";

const NOW = 1_800_000_000_000;
const MIN = 60_000;

describe("formatAge", () => {
  it("is empty without a time", () => {
    expect(formatAge(0, NOW)).toBe("");
    expect(formatAge(NaN, NOW)).toBe("");
  });

  it("uses min, h and d", () => {
    expect(formatAge(NOW - 10_000, NOW)).toBe("< 1 min");
    expect(formatAge(NOW - MIN, NOW)).toBe("1 min");
    expect(formatAge(NOW - 9 * MIN, NOW)).toBe("9 min");
    expect(formatAge(NOW - 59 * MIN, NOW)).toBe("59 min");
    expect(formatAge(NOW - 60 * MIN, NOW)).toBe("1 h");
    expect(formatAge(NOW - 23 * 60 * MIN, NOW)).toBe("23 h");
    expect(formatAge(NOW - 24 * 60 * MIN, NOW)).toBe("1 d");
    expect(formatAge(NOW - 40 * 24 * 60 * MIN, NOW)).toBe("40 d");
  });

  it("accepts seconds and a clock that ran backwards", () => {
    expect(formatAge(Math.floor((NOW - 2 * MIN) / 1000), NOW)).toBe("2 min");
    expect(formatAge(NOW + 5000, NOW)).toBe("< 1 min");
  });
});
