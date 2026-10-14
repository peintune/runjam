import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Native pickers temporarily transfer focus away from the pet popup. */
export async function withPetWindowDialog<T>(action: () => Promise<T>): Promise<T> {
  let pet = false;
  try { pet = getCurrentWindow().label === "pet-chat"; } catch { /* not a Tauri webview */ }
  if (!pet) return action();
  await invoke("suspend_pet_chat_autohide", {suspended: true});
  try { return await action(); }
  finally { await invoke("suspend_pet_chat_autohide", {suspended: false}); }
}
