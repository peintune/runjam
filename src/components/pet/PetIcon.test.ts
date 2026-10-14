import { describe, it, expect } from "vitest";
import { createSSRApp, h } from "vue";
import { renderToString } from "@vue/server-renderer";
import PetIcon from "./PetIcon.vue";
import nativeWindowSource from "../../../src-tauri/src/commands/pet_cmd.rs?raw";

describe("pet launcher size", () => {
  it("uses a 44px image and click target", async () => {
    const html = await renderToString(createSSRApp({render: () => h(PetIcon)}));
    expect(html.match(/w-11 h-11/g) ?? []).toHaveLength(2);
  });
  it("keeps an 8px transparent drag rim around the compact image", () => {
    const size = Number(nativeWindowSource.match(/const ICON_SIZE: f64 = ([\d.]+);/)?.[1]);
    expect(size).toBe(60);
    expect((size - 44) / 2).toBe(8);
  });
});
