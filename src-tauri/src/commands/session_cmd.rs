use crate::models::session::Session;
use crate::session::runner::SessionManager;
use crate::search;
use crate::skill;
use tauri::Manager;
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
    // Skill names to deploy into the session's working directory before the
    // agent starts. Each agent discovers skills from its own per-session
    // directory (.claude/skills/, .codex/skills/, .gemini/skills/).
    // None / empty list = deploy nothing (user opted out of skills).
    skills: Option<Vec<String>>,
) -> Result<Session, String> {
    let id = session_id.unwrap_or_else(|| format!(
        "{}-{}",
        chrono::Utc::now().timestamp_millis(),
        &cli[..4.min(cli.len())]
    ));

    // Default to ~/.runjam/session/{id} when no directory selected
    let dir = directory.unwrap_or_else(|| {
        default_session_dir().join(&id).to_string_lossy().to_string()
    });
    // Always ensure the session working directory exists — the frontend may
    // pass a path that hasn't been created yet, and Command::current_dir fails
    // with ENOENT if the directory is missing.
    std::fs::create_dir_all(&dir).ok();

    // Deploy built-in skills into the session's per-agent skills directory.
    // This happens BEFORE the agent process starts so the agent picks them up
    // natively via its own skill discovery mechanism — no ACP protocol changes.
    let agent_type = match cli.as_str() {
        "claude-code" => "claude",
        "codex-cli" => "codex",
        "gemini-cli" => "gemini",
        _ => "",
    };
    if !agent_type.is_empty() {
        let mut skill_names = skills.unwrap_or_default();
        // Always inject the default constraints skill so every session has
        // guardrails: output path conventions, dependency checks, fallback
        // strategies, and file management discipline. Users cannot opt out
        // of this via the UI — it is added server-side.
        if !skill_names.iter().any(|n| n == "runjam-defaults") {
            skill_names.push("runjam-defaults".to_string());
        }
        if let Err(e) = skill::deploy_skills_to_session(&app, &dir, agent_type, &skill_names) {
            crate::rjlog!("[SESSION] Warning: skill deployment failed: {}", e);
            // Non-fatal — session should still start without skills.
        }
    }

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

    // Telemetry: record the feature usage (queued locally, sent in batch).
    {
        let db = app.state::<std::sync::Mutex<crate::db::connection::Database>>();
        let guard = db.lock().ok();
        if let Some(guard) = guard {
            crate::telemetry::track(&guard, "session_started", serde_json::json!({ "cli": cli }));
        }
    }

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
        "",
    );

    Ok(session)
}

#[tauri::command]
pub async fn stop_session(
    app: tauri::AppHandle,
    manager: State<'_, Mutex<SessionManager>>,
    id: String,
) -> Result<(), String> {
    let mut mgr = manager.lock().map_err(|e| e.to_string())?;
    let res = mgr.stop(&id);
    // Telemetry: record session stop (non-fatal).
    let db = app.state::<std::sync::Mutex<crate::db::connection::Database>>();
    let guard = db.lock().ok();
    if let Some(guard) = guard {
        crate::telemetry::track(&guard, "session_stopped", serde_json::json!({ "session_id": id }));
    }
    res
}

/// Whether a backend agent process still exists for the session. After a
/// webview reload (dev HMR full-reload from agent files landing in the project
/// tree) the backend process usually survives — the frontend uses this to avoid
/// killing it and losing the in-process conversation context.
#[tauri::command]
pub async fn session_alive(
    manager: State<'_, Mutex<SessionManager>>,
    id: String,
) -> Result<bool, String> {
    let mgr = manager.lock().map_err(|e| e.to_string())?;
    Ok(mgr.has_client(&id))
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
pub async fn set_session_permission_mode(
    manager: State<'_, Mutex<SessionManager>>,
    id: String,
    mode: String,
) -> Result<(), String> {
    let mgr = manager.lock().map_err(|e| e.to_string())?;
    mgr.set_permission_mode(&id, &mode)
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
