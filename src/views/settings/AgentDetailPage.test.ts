import { describe, expect, it } from "vitest";
import source from "./AgentDetailPage.vue?raw";

describe("agent config mode feedback", () => {
  it("does not reuse the permanent hint as a second success message", () => {
    expect(source.includes('nativeMsg.value[id] = t("models.codexModeHint")')).toBe(false);
  });
});
