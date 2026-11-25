import { expect, test } from "vitest";
import { CONTENT_UNAVAILABLE, stoppedPasteMessage } from "./usePasteActions";

test("a paste stopped before anything was deleted says why", () => {
  expect(stoppedPasteMessage(new Error(`${CONTENT_UNAVAILABLE}: [internal] decrypt failed`))).toBe(
    "content_load_failed"
  );
  expect(stoppedPasteMessage("[validation] paste_target_unavailable")).toBe("paste_target_unavailable");
  expect(stoppedPasteMessage("[internal] paste_keystroke_failed")).toBe("paste_keystroke_failed");
  expect(stoppedPasteMessage("[validation] clipboard_snapshot_failed")).toBe("clipboard_snapshot_failed");
  expect(stoppedPasteMessage("[validation] clipboard_changed_during_paste")).toBe("clipboard_changed_during_paste");
  expect(stoppedPasteMessage("[io] disk full")).toBeNull();
});
