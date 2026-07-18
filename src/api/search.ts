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
  status: string;
  pid: number | null;
  pinned: number;
  archived: number;
  created_at: string;
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
): Promise<void> {
  return invoke("save_session", { id, cli, cli_display_name, title, directory, status, pid, pinned, archived });
}

export async function getSessions(): Promise<SessionRecord[]> {
  return invoke<SessionRecord[]>("get_sessions");
}

export async function updateSessionTitle(id: string, title: string): Promise<void> {
  return invoke("update_session_title", { id, title });
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
