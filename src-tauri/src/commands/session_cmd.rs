use crate::models::session::Session;
use crate::session::runner::SessionManager;
use crate::search;
use tauri::State;
use std::sync::Mutex;
use std::path::PathBuf;

fn default_session_dir() -> PathBuf {
    let home = directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".runjam").join("session")
}

#[tauri::command]
pub async fn start_session(
    app: tauri::AppHandle,
    manager: State<'_, Mutex<SessionManager>>,
    cli: String,
    cli_display_name: String,
    directory: Option<String>,
    session_id: Option<String>,
    model: Option<String>,
    mode: Option<String>,
    permission_mode: Option<String>,
) -> Result<Session, String> {
    let id = session_id.unwrap_or_else(|| format!(
        "{}-{}",
        chrono::Utc::now().timestamp_millis(),
        &cli[..4.min(cli.len())]
    ));

    // Default to ~/.runjam/session/{id} when no directory selected
    let dir = directory.unwrap_or_else(|| {
        let path = default_session_dir().join(&id);
        std::fs::create_dir_all(&path).ok();
        path.to_string_lossy().to_string()
    });

    let session = {
        let mut mgr = manager.lock().map_err(|e| e.to_string())?;
        mgr.start(
            &app,
            id.clone(),
            &cli,
            &cli_display_name,
            Some(&dir),
            model.as_deref(),
            mode.as_deref().unwrap_or("assistant"),
            permission_mode.as_deref().unwrap_or("ask_approval"),
        )?
    }; // lock released here — before SQLite write

    search::save_session(
        &id,
        &cli,
        &cli_display_name,
        &cli_display_name,
        &dir,
        "running",
        None,
        0,
        0,
    );

    Ok(session)
}

#[tauri::command]
pub fn stop_session(
    manager: State<'_, Mutex<SessionManager>>,
    id: String,
) -> Result<(), String> {
    let mut mgr = manager.lock().map_err(|e| e.to_string())?;
    mgr.stop(&id)
}

#[tauri::command]
pub async fn send_input(
    app: tauri::AppHandle,
    manager: State<'_, Mutex<SessionManager>>,
    id: String,
    text: String,
    history: Option<Vec<String>>,
) -> Result<(), String> {
    let mgr = manager.lock().map_err(|e| e.to_string())?;
    mgr.send_input(&app, &id, &text, history.as_deref())
}

#[tauri::command]
pub fn respond_interaction(
    manager: State<'_, Mutex<SessionManager>>,
    id: String,
    response: String,
) -> Result<(), String> {
    let mgr = manager.lock().map_err(|e| e.to_string())?;
    mgr.respond(&id, &response)
}

#[tauri::command]
pub fn respond_permission(
    manager: State<'_, Mutex<SessionManager>>,
    id: String,
    request_id: String,
    response: String,
) -> Result<(), String> {
    let mgr = manager.lock().map_err(|e| e.to_string())?;
    mgr.respond_permission(&id, &request_id, &response)
}

#[tauri::command]
pub fn list_sessions() -> Vec<Session> {
    // In the future, read from SQLite.
    // For now, this is managed in-memory via the frontend workspace store.
    vec![]
}

#[tauri::command]
pub fn get_session_logs(id: String) -> Vec<String> {
    let log_path = dirs_log_dir().join(format!("{}.log", id));
    if log_path.exists() {
        std::fs::read_to_string(&log_path)
            .unwrap_or_default()
            .lines()
            .map(|l| l.to_string())
            .collect()
    } else {
        vec![]
    }
}

fn dirs_log_dir() -> std::path::PathBuf {
    let base = if let Some(dir) = directories::ProjectDirs::from("com", "runjam", "RunJam") {
        dir.data_local_dir().to_path_buf()
    } else {
        std::path::PathBuf::from(".")
    };
    let log_dir = base.join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    log_dir
}
