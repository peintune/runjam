import { describe, it, expect, vi } from "vitest";
const native = vi.hoisted(() => vi.fn());
vi.mock("../api/models", () => ({getNativeModels: native}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ask: vi.fn()}));
import { withNativeChoice, resolveDefaultPermission } from "./agentLaunch";
import { NATIVE_MODEL_ID } from "../api/sessions";
import type { ModelEntry } from "../api/models";

describe("native model choice", () => {
  it("keeps native selectable when its display metadata cannot be loaded", async () => {
    native.mockRejectedValueOnce(new Error("temporary read failure"));
    const result = await withNativeChoice("codex-cli", [{id: "managed"}] as ModelEntry[]);
    expect(result.map(m => m.id)).toEqual([NATIVE_MODEL_ID, "managed"]);
  });
  it.each(["claude-code", "gemini-cli"])("offers actual native mode for %s even without an explicit model", async agent => {
    native.mockResolvedValueOnce([]);
    expect((await withNativeChoice(agent, [])).map(m => m.id)).toEqual([NATIVE_MODEL_ID]);
  });
  it("does not add native mode to unsupported agents", async () => {
    expect(await withNativeChoice("ollama-cli", [])).toEqual([]);
  });
  it("honors saved first-launch full access before native inheritance", () => {
    expect(resolveDefaultPermission("full_access", true)).toBe("full_access");
    expect(resolveDefaultPermission("ask_approval", true)).toBe("ask_approval");
    expect(resolveDefaultPermission("", true)).toBe("inherit");
    expect(resolveDefaultPermission("", false)).toBe("approve_for_me");
  });
});
