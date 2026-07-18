import { invoke } from "@tauri-apps/api/core";

export interface SessionInfo {
  id: string;
  cli: string;
  cli_display_name: string;
  directory: string | null;
  pid: number | null;
  status: string;
  created_at: string;
}

export async function startSession(
  cli: string,
  cliDisplayName: string,
  directory?: string,
  sessionId?: string,
  model?: string,
  mode?: string,
  permissionMode?: string,
): Promise<SessionInfo> {
  return invoke<SessionInfo>("start_session", {
    cli,
    cliDisplayName,
    directory: directory ?? null,
    sessionId: sessionId ?? null,
    model: model ?? null,
    mode: mode ?? "assistant",
    permissionMode: permissionMode ?? null,
  });
}

export async function stopSession(id: string): Promise<void> {
  return invoke("stop_session", { id });
}

export async function listSessions(): Promise<SessionInfo[]> {
  return invoke<SessionInfo[]>("list_sessions");
}

export async function getSessionLogs(id: string): Promise<string[]> {
  return invoke<string[]>("get_session_logs", { id });
}

export async function sendInput(id: string, text: string, history?: string[]): Promise<void> {
  return invoke("send_input", { id, text, history: history ?? null });
}

export async function respondInteraction(id: string, response: string): Promise<void> {
  return invoke("respond_interaction", { id, response });
}

export async function respondPermission(id: string, requestId: string, response: string): Promise<void> {
  return invoke("respond_permission", { id, requestId, response });
}
