import { expect, it } from "vitest";
import { loadTranslationSource } from "./TranslatePanel";

it("rejects translation source loading when the full-content command is unavailable", async () => {
  await expect(loadTranslationSource(1, 10)).rejects.toThrow();
});
