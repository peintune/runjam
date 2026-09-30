import { invoke } from "@tauri-apps/api/core";

export async function getDataDir(): Promise<string> {
  return invoke<string>("get_data_dir");
}

export async function openDataDir(): Promise<void> {
  return invoke<void>("open_data_dir");
}

export async function openInFinder(path: string): Promise<void> {
  return invoke<void>("open_in_finder", { path });
}

export async function revealPath(path: string): Promise<void> {
  return invoke<void>("reveal_path", { path });
}

/** Desktop pet: whether the floating icon is currently shown. */
export async function petEnabled(): Promise<boolean> {
  return invoke<boolean>("pet_enabled");
}

/** Desktop pet: show or hide the floating icon (persisted). */
export async function setPetEnabled(enabled: boolean): Promise<void> {
  return invoke<void>("set_pet_enabled", { enabled });
}
