import { invoke } from "@tauri-apps/api/core";

export async function getDataDir(): Promise<string> {
  return invoke<string>("get_data_dir");
}

export async function openDataDir(): Promise<void> {
  return invoke<void>("open_data_dir");
}
