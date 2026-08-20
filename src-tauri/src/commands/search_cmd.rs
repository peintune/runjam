use crate::search::{self, SearchResult, SessionRecord};

#[tauri::command]
pub fn search_conversations(query: String) -> Vec<SearchResult> {
    search::search_messages(&query, 30)
}

/// Saves a conversation message and returns the session's updated
/// `context_chars` so the frontend can refresh the sidebar's context size.
#[tauri::command]
pub fn save_conversation_message(session_id: String, role: String, content: String) -> i64 {
    search::save_message(&session_id, &role, &content)
}

#[tauri::command]
pub fn get_conversation_messages(session_id: String) -> Vec<SearchResult> {
    search::get_messages_by_session(&session_id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn save_session(
    id: String,
    cli: String,
    cli_display_name: String,
    title: String,
    directory: String,
    status: String,
    pid: Option<i64>,
    pinned: i64,
    archived: i64,
    acp_session_id: String,
) {
    search::save_session(&id, &cli, &cli_display_name, &title, &directory, &status, pid, pinned, archived, &acp_session_id);
}

#[tauri::command]
pub fn get_sessions() -> Vec<SessionRecord> {
    search::get_sessions()
}

#[tauri::command]
pub fn update_session_title(id: String, title: String) {
    search::update_session_title(&id, &title);
}

#[tauri::command]
pub fn update_session_model(id: String, model: String) {
    search::set_session_model(&id, &model);
}

#[tauri::command]
pub fn delete_session(id: String) -> Result<(), String> {
    search::delete_session(&id).map_err(|e| format!("Failed to delete session: {}", e))
}

#[tauri::command]
pub fn archive_session(id: String) {
    search::set_session_archived(&id, true);
}

#[tauri::command]
pub fn unarchive_session(id: String) {
    search::set_session_archived(&id, false);
}

#[tauri::command]
pub fn delete_archived_sessions() {
    search::delete_archived_sessions();
}

#[tauri::command]
pub fn touch_session(id: String) {
    search::touch_session(&id);
}
