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

/// Agent startup holds session/client locks while waiting on ACP responses.
/// Commands that need those locks must yield IPC dispatch and do their waiting
/// on the blocking pool, keeping both the UI and async workers available.
pub(crate) async fn run_session_task<R: tauri::Runtime, T: Send + 'static>(
    app: tauri::AppHandle<R>,
    action: impl FnOnce(&mut SessionManager) -> T + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<T, String> {
        let state = app.state::<Mutex<SessionManager>>();
        let mut manager = state.lock().map_err(|e| e.to_string())?;
        Ok(action(&mut manager))
    })
    .await
    .map_err(|e| format!("Session worker failed: {e}"))?
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
    config_mode: Option<String>,
    reasoning_disabled: Option<bool>,
) -> Result<Session, String> {
    let id = session_id.unwrap_or_else(|| format!(
        "{}-{}",
        chrono::Utc::now().timestamp_millis(),
        &cli[..4.min(cli.len())]
    ));

    let qa = mode.as_deref() == Some(crate::pet_qa::MODE);
    // Q&A never runs inside a user project, even if a stale frontend sends one.
    let dir = directory.filter(|_| !qa).unwrap_or_else(|| {
        default_session_dir().join(&id).to_string_lossy().to_string()
    });
    // Always ensure the session working directory exists — the frontend may
    // pass a path that hasn't been created yet, and Command::current_dir fails
    // with ENOENT if the directory is missing.
    std::fs::create_dir_all(&dir).ok();

    let mut launch_plan = if crate::agent_launch::supports_native(&cli) {
        let default = {
            let db = app.state::<Mutex<crate::db::connection::Database>>();
            let guard = db.lock().map_err(|e| e.to_string())?;
            let conn = guard.conn.lock().map_err(|e| e.to_string())?;
            crate::commands::models_cmd::config_mode_from_db(&conn, &cli)
        };
        // Legacy Claude/Gemini can run their managed CLI default without a
        // selected RunJam model. Codex managed mode requires an exact model.
        let selection = if cli != "codex-cli" && model.as_deref().unwrap_or("").is_empty() && config_mode.is_none() {
            default
        } else { crate::agent_launch::resolve_mode(config_mode.as_deref(), model.as_deref(), default)? };
        if cli == "codex-cli" {
            Some(crate::agent_launch::prepare_codex(&app, selection, model.as_deref(), reasoning_disabled.unwrap_or(false))?)
        } else if selection == crate::agent_launch::ConfigMode::Native {
            Some(crate::agent_launch::prepare_native(&cli, reasoning_disabled.unwrap_or(false))?)
        } else { None }
    } else { None };
    let native = launch_plan.as_ref().map(|p| p.info.config_mode == crate::agent_launch::ConfigMode::Native).unwrap_or(false);
    let saved_permission = {
        let db = app.state::<Mutex<crate::db::connection::Database>>();
        let guard = db.lock().map_err(|e| e.to_string())?;
        let conn = guard.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row("SELECT value FROM app_settings WHERE key=?1", [format!("agent_permission_{cli}")], |r| r.get::<_, String>(0)).unwrap_or_default()
    };
    let permission = if qa { "read_only" } else { permission_mode.as_deref().filter(|p| !p.is_empty())
        .or_else(|| if saved_permission.is_empty() { None } else { Some(saved_permission.as_str()) })
        .unwrap_or(if native { "inherit" } else { "ask_approval" }) };
    if let Some(plan) = &mut launch_plan { plan.info.permission_mode = permission.into(); }
    if let Some(plan) = &launch_plan {
        manager.lock().map_err(|e| e.to_string())?.reserve_planned_start(&id, &plan.info.run_id);
    }

    // Deploy built-in skills into the session's per-agent skills directory.
    // This happens BEFORE the agent process starts so the agent picks them up
    // natively via its own skill discovery mechanism — no ACP protocol changes.
    let agent_type = match cli.as_str() {
        "claude-code" => "claude",
        "codex-cli" => "codex",
        "gemini-cli" => "gemini",
        _ => "",
    };
    if !agent_type.is_empty() && !native && !qa {
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

    // Managed Claude/Gemini model overrides go only to RunJam's isolated config.
    // Apply them synchronously before spawn so a new chat cannot race the
    // frontend's setAgentModel and start with the previous model. Native
    // launches never enter this branch.
    if let Some(model_id) = model.as_deref().filter(|_| cli != "codex-cli" && !native) {
        let db = app.state::<std::sync::Mutex<crate::db::connection::Database>>();
        let guard = db.lock().ok();
        if let Some(guard) = guard {
            let conn = guard.conn.lock().ok();
            if let Some(conn) = conn {
                match conn.query_row(
                    "SELECT name, api_key FROM models WHERE id = ?1",
                    [model_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                ) {
                    Ok((model_name, api_key)) => {
                        crate::rjlog!("[SESSION] Applying model '{}' (id={}) to agent '{}' before process start", model_name, model_id, cli);
                        if let Err(e) = crate::models_config::set_agent_model(&cli, &model_name, &api_key) {
                            crate::rjlog!("[SESSION] Warning: failed to write model to agent config: {}", e);
                        }
                    }
                    Err(e) => {
                        crate::rjlog!("[SESSION] Model id '{}' not found in DB ({}), agent will use its current config", model_id, e);
                    }
                }
            }
        }
    }

    let session = if let Some(plan) = launch_plan {
        let run_id = plan.info.run_id.clone();
        let prepared = (|| {
            let mut client = crate::acp_client::AcpClient::start(&app, &id, agent_type, Some(&dir), model.as_deref(),
                mode.as_deref().unwrap_or("assistant"), permission, Some(plan))?;
            manager.lock().map_err(|e| e.to_string())?.track_pending_pid(&id, &run_id, client.pid())?;
            client.initialize_session(&app, &id)?;
            if !client.is_alive() { return Err(format!("{cli} exited during initialization")); }
            Ok(client)
        })();
        match prepared {
            Ok(client) => manager.lock().map_err(|e| e.to_string())?.attach_planned(&app, id.clone(), &cli, &cli_display_name, &dir, client)?,
            Err(error) => {
                manager.lock().map_err(|e| e.to_string())?.clear_pending(&id, &run_id);
                return Err(error);
            }
        }
    } else {
        let mut mgr = manager.lock().map_err(|e| e.to_string())?;
        mgr.start(
            &app,
            id.clone(),
            &cli,
            &cli_display_name,
            Some(&dir),
            model.as_deref(),
            mode.as_deref().unwrap_or("assistant"),
            permission,
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
pub async fn get_session_runtime<R: tauri::Runtime>(app: tauri::AppHandle<R>, id: String) -> Option<crate::agent_launch::RuntimeInfo> {
    run_session_task(app, move |manager| manager.runtime_info(&id)).await.ok().flatten()
}

#[cfg(test)]
mod responsiveness_tests {
    use super::*;
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};
    use tauri::test::{mock_builder, mock_context, noop_assets, INVOKE_KEY};

    /// Dispatch runs on the UI thread in production. Holding a session lock
    /// models the Agent startup/handshake without launching a real process.
    fn assert_dispatch_remains_responsive(command: &str, body: serde_json::Value) {
        let app = mock_builder()
            .manage(Mutex::new(SessionManager::new()))
            .manage(Arc::new(Mutex::new(crate::proxy::ProxyState::new())))
            .invoke_handler(tauri::generate_handler![
                get_session_runtime, respond_interaction, respond_permission,
                crate::commands::proxy_cmd::set_reasoning_disabled,
            ])
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build().unwrap();
        let handle = app.handle().clone();
        let (locked_tx, locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let state = handle.state::<Mutex<SessionManager>>();
            let _guard = state.lock().unwrap();
            locked_tx.send(()).unwrap();
            // A broken synchronous handler must fail rather than hang the test.
            let _ = release_rx.recv_timeout(Duration::from_secs(2));
        });
        locked_rx.recv().unwrap();
        let (response_tx, response_rx) = mpsc::channel();
        let started = Instant::now();
        window.as_ref().clone().on_message(
            tauri::webview::InvokeRequest {
                cmd: command.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: if cfg!(windows) { "http://tauri.localhost" } else { "tauri://localhost" }.parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.into(),
            },
            Box::new(move |_, _, response, _, _| { let _ = response_tx.send(response); }),
        );
        let elapsed = started.elapsed();
        let _ = release_tx.send(());
        worker.join().unwrap();
        let response = response_rx.recv_timeout(Duration::from_secs(2)).expect("command must finish after unlock");
        let response = match response {
            tauri::ipc::InvokeResponse::Ok(body) => Ok(body.deserialize::<serde_json::Value>().unwrap()),
            tauri::ipc::InvokeResponse::Err(error) => Err(error.0),
        };
        let expected = if command.starts_with("respond_") {
            Err(serde_json::json!("No client for session: starting"))
        } else { Ok(serde_json::Value::Null) };
        assert_eq!(response, expected, "the request must reach the real {command} handler");
        assert!(elapsed < Duration::from_millis(500),
            "{command} blocked UI dispatch for {elapsed:?} while the session lock was busy");
    }

    #[test]
    fn runtime_lookup_does_not_block_ui_dispatch() {
        assert_dispatch_remains_responsive("get_session_runtime", serde_json::json!({"id": "starting"}));
    }

    #[test]
    fn interaction_response_does_not_block_ui_dispatch() {
        assert_dispatch_remains_responsive("respond_interaction", serde_json::json!({"id": "starting", "response": "yes"}));
    }

    #[test]
    fn permission_response_does_not_block_ui_dispatch() {
        assert_dispatch_remains_responsive("respond_permission", serde_json::json!({"id": "starting", "requestId": "request", "response": "yes"}));
    }

    #[test]
    fn reasoning_change_does_not_block_ui_dispatch() {
        assert_dispatch_remains_responsive("set_reasoning_disabled", serde_json::json!({"sessionId": "starting", "disabled": true}));
    }
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
pub async fn respond_interaction<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    response: String,
) -> Result<(), String> {
    run_session_task(app, move |manager| manager.respond(&id, &response)).await?
}

#[tauri::command]
pub async fn respond_permission<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    request_id: String,
    response: String,
) -> Result<(), String> {
    run_session_task(app, move |manager| manager.respond_permission(&id, &request_id, &response)).await?
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
