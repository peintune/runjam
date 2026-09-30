import { invoke } from "@tauri-apps/api/core";

export async function getProxyPort(): Promise<number> {
  return invoke<number>("get_proxy_port");
}

export async function getProxyUrl(): Promise<string> {
  return invoke<string>("get_proxy_url");
}

/** Globally suppress reasoning/thinking output on the proxy. This is a
 *  process-wide proxy setting (not per-session), matching the main window's
 *  "no thinking" toggle. */
export async function setReasoningDisabled(disabled: boolean): Promise<void> {
  return invoke("set_reasoning_disabled", { disabled });
}
