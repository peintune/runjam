import { invoke } from "@tauri-apps/api/core";

export async function getProxyPort(): Promise<number> {
  return invoke<number>("get_proxy_port");
}

export async function getProxyUrl(): Promise<string> {
  return invoke<string>("get_proxy_url");
}
