import { invoke } from "@tauri-apps/api/core";

export interface SearchResult {
  session_id: string;
  role: string;
  content: string;
  created_at: string;
}

export interface SessionRecord {
  id: string;
  cli: string;
  cli_display_name: string;
  title: string;
  directory: string;
  model: string | null;
  status: string;
  pid: number | null;
  pinned: number;
  archived: number;
  created_at: string;
  last_active_at: string | null;
  acp_session_id: string;
}

export async function searchConversations(query: string): Promise<SearchResult[]> {
  return invoke<SearchResult[]>("search_conversations", { query });
}

export async function saveConversationMessage(sessionId: string, role: string, content: string): Promise<void> {
  return invoke("save_conversation_message", { sessionId, role, content });
}

export async function getConversationMessages(sessionId: string): Promise<SearchResult[]> {
  return invoke<SearchResult[]>("get_conversation_messages", { sessionId });
}

export async function saveSession(
  id: string,
  cli: string,
  cli_display_name: string,
  title: string,
  directory: string,
  status: string,
  pid: number | null,
  pinned: number,
  archived: number,
  acp_session_id?: string,
): Promise<void> {
  return invoke("save_session", { id, cli, cli_display_name, title, directory, status, pid, pinned, archived, acp_session_id: acp_session_id ?? "" });
}

export async function getSessions(): Promise<SessionRecord[]> {
  return invoke<SessionRecord[]>("get_sessions");
}

export async function updateSessionTitle(id: string, title: string): Promise<void> {
  return invoke("update_session_title", { id, title });
}

export async function updateSessionModel(id: string, model: string): Promise<void> {
  return invoke("update_session_model", { id, model });
}

export async function deleteSession(id: string): Promise<void> {
  return invoke("delete_session", { id });
}

export async function archiveSession(id: string): Promise<void> {
  return invoke("archive_session", { id });
}

export async function unarchiveSession(id: string): Promise<void> {
  return invoke("unarchive_session", { id });
}

export async function deleteArchivedSessions(): Promise<void> {
  return invoke("delete_archived_sessions");
}

/** Bump the session's last_active_at timestamp on the backend so the
 *  sidebar's time and ordering reflect the freshest activity, even after a
 *  page reload. Cheap — single UPDATE, no network. */
export async function touchSession(id: string): Promise<void> {
  return invoke("touch_session", { id });
}
