import { describe, expect, it } from "vitest";
import { chosenApp, unchosenFallback } from "./defaultApps";

describe("default apps", () => {
  it("treats an empty or blank value as nothing chosen", () => {
    expect(chosenApp({ "app.image": "" }, "image")).toBe("");
    expect(chosenApp({ "app.image": "  " }, "image")).toBe("");
    expect(chosenApp({}, "image")).toBe("");
  });

  it("code without its own app follows the text app, like the backend", () => {
    expect(unchosenFallback({ "app.text": "C:\\npp.exe" }, "code")).toBe("text");
    expect(unchosenFallback({}, "code")).toBe("system");
    expect(unchosenFallback({ "app.text": "C:\\npp.exe" }, "image")).toBe("system");
  });
});
