import { expect, test } from "vitest";
import { parseRules, serializeRules } from "./cleanupRules";

test("a replacement may contain =>", () => {
    expect(parseRules("a => b => c")).toEqual([
        { match: "a", replace: "b => c", label: undefined, action: "clean" }
    ]);
});

test("a line break typed in a rule stays in that rule", () => {
    const stored = serializeRules([{ match: "line1", replace: "first\nline2", action: "clean" }]);

    expect(stored).toBe("line1 => first\\nline2");
    expect(parseRules(stored)).toHaveLength(1);
});
