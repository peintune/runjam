import { invoke } from "@tauri-apps/api/core";

/**
 * Hold (or release) a system idle-sleep assertion.
 *
 * Used to keep the machine awake while an agent session is running, so closing
 * the lid does not freeze a long task. macOS-only; a no-op error elsewhere.
 */
export async function setPreventSleep(enabled: boolean): Promise<void> {
  return invoke<void>("set_prevent_sleep", { enabled });
}

/** Whether the assertion is currently held. */
export async function preventSleepActive(): Promise<boolean> {
  return invoke<boolean>("prevent_sleep_active");
}