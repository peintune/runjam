import { describe, it, expect, vi, beforeEach } from "vitest";
const mocked = vi.hoisted(() => ({label: "pet-chat", invoke: vi.fn(async () => {})}));
vi.mock("@tauri-apps/api/core", () => ({invoke: mocked.invoke}));
vi.mock("@tauri-apps/api/window", () => ({getCurrentWindow: () => ({label: mocked.label})}));
import { withPetWindowDialog } from "./petWindowDialog";

describe("native pet dialogs", () => {
  beforeEach(() => {mocked.label = "pet-chat"; mocked.invoke.mockClear();});
  it("suspends auto-hide until the native dialog finishes", async () => {
    expect(await withPetWindowDialog(async () => "selected")).toBe("selected");
    expect(mocked.invoke.mock.calls).toEqual([
      ["suspend_pet_chat_autohide", {suspended: true}],
      ["suspend_pet_chat_autohide", {suspended: false}],
    ]);
  });
  it("resumes auto-hide even if the dialog fails", async () => {
    await expect(withPetWindowDialog(async () => {throw new Error("cancelled");})).rejects.toThrow("cancelled");
    expect(mocked.invoke).toHaveBeenLastCalledWith("suspend_pet_chat_autohide", {suspended: false});
  });
  it("leaves main-window dialogs alone", async () => {
    mocked.label = "main";
    await withPetWindowDialog(async () => {});
    expect(mocked.invoke).not.toHaveBeenCalled();
  });
});
