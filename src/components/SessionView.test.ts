import source from "./SessionView.vue?raw";
import { describe, expect, it } from "vitest";

describe("model picker copy", () => {
  it("does not show the next-message hint in either composer", () => {
    expect(source.includes("models.switchNextTurn")).toBe(false);
  });
});
