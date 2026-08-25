mod commands;
mod models;
mod db;
mod agent;
mod session;
mod git;
mod cost;
mod state;
mod models_config;
mod acp;
mod search;
mod acp_client;
mod proxy;
mod node_util;
mod skill;
pub mod log_util;
mod util;
mod telemetry;
mod updates;

use commands::term_cmd::TerminalState;
use db::connection::Database;
use session::runner::SessionManager;
use state::AppState;
use proxy::ProxyState;
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// Copy the database from the legacy platform app-data dir (e.g.
/// ~/Library/Application Support/RunJam/runjam.db on macOS) into ~/.runjam/
/// so existing users don't lose their session history after the migration.
/// Only runs once — once the file exists in the new location it's left alone.
fn migrate_legacy_db(new_dir: &std::path::Path) {
    let new_db = new_dir.join("runjam.db");
    if new_db.exists() {
        return; // already migrated (or fresh install)
    }
    let legacy_dir = directories::ProjectDirs::from("com", "runjam", "RunJam")
        .map(|d| d.data_local_dir().to_path_buf());
    if let Some(legacy) = legacy_dir {
        let legacy_db = legacy.join("runjam.db");
        if legacy_db.exists() {
            if let Err(e) = std::fs::copy(&legacy_db, &new_db) {
                eprintln!("[migrate] Failed to copy legacy db: {}", e);
            } else {
                println!("[migrate] Database migrated from {:?} to {:?}", legacy_db, new_db);
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Report panics to the backend (sanitized) before the app dies.
    telemetry::init_panic_hook();

    // RunJam stores all user data (db, logs, sessions, ACP packages) under
    // ~/.runjam/ — a single, predictable, user-visible location instead of the
    // platform-specific app-data dir (~/Library/Application Support/RunJam on
    // macOS, %LOCALAPPDATA%/RunJam on Windows). This keeps everything in one
    // place: easy to back up, inspect, and clean up.
    let app_dir = directories::UserDirs::new()
        .map(|d| d.home_dir().join(".runjam"))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    std::fs::create_dir_all(&app_dir).ok();

    // Migrate database from the legacy platform app-data dir if present.
    migrate_legacy_db(&app_dir);

    search::init_db();
    let db = Database::new(&app_dir).expect("Failed to create database");
    db::migrations::run_migrations(&db.conn.lock().unwrap());

    // Ensure default session working directory exists
    std::fs::create_dir_all(app_dir.join("session")).ok();

    let proxy_state = Arc::new(Mutex::new(ProxyState::new()));
    // Load agent→model mapping so the proxy can resolve same-named models by id
    {
        let conn = db.conn.lock().unwrap();
        let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        if let Ok(mut stmt) = conn.prepare("SELECT agent_id, model_id FROM agent_models") {
            if let Ok(iter) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                for r in iter.flatten() {
                    map.entry(r.0).or_insert_with(Vec::new).push(r.1);
                }
            }
        }
        proxy_state.lock().unwrap().agent_models = map;
    }
    commands::proxy_cmd::init_proxy(proxy_state.clone());
    
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Mutex::new(SessionManager::new()))
        .manage(Mutex::new(AppState::load()))
        .manage(Mutex::new(db))
        .manage(proxy_state)
        .manage(Mutex::new(TerminalState::new()))
        .invoke_handler(tauri::generate_handler![
            commands::agent_cmd::detect_agents,
            commands::agent_cmd::check_agent,
            commands::agent_cmd::install_agent,
            commands::skill_cmd::list_skills,
            commands::skill_cmd::list_user_skills,
            commands::skill_cmd::install_skill_zip,
            commands::skill_cmd::remove_user_skill,
            commands::skill_cmd::list_session_skills,
            commands::skill_cmd::deploy_session_skill,
            commands::skill_cmd::remove_session_skill,
            commands::agent_cmd::uninstall_agent,
            commands::agent_cmd::set_agent_enabled,
            commands::agent_cmd::check_nodejs,
            commands::agent_cmd::get_nodejs_install_guide,
            commands::agent_cmd::open_nodejs_download,
            commands::agent_cmd::read_agent_config,
            commands::agent_cmd::write_agent_config,
            commands::agent_cmd::get_agent_dir_info,
            commands::agent_cmd::get_agent_statuses,
            commands::agent_cmd::test_agent,
            commands::models_cmd::get_models,
            commands::models_cmd::save_models,
            commands::models_cmd::get_last_agent,
            commands::models_cmd::set_last_agent,
            commands::models_cmd::get_agent_models,
            commands::models_cmd::get_agent_model_map,
            commands::models_cmd::assign_model_to_agent,
            commands::models_cmd::remove_model_from_agent,
            commands::models_cmd::read_agent_config_models,
            commands::models_cmd::get_model_aliases,
            commands::models_cmd::add_model_alias,
            commands::models_cmd::remove_model_alias,
            commands::models_cmd::get_model_by_alias,
            commands::models_cmd::sync_model_to_all_agents,
            commands::models_cmd::set_default_model,
            commands::models_cmd::get_default_model,
            commands::models_cmd::set_agent_default_model,
            commands::models_cmd::get_agent_default_model,
            commands::models_cmd::get_session_model,
            commands::models_cmd::set_session_model,
            commands::models_cmd::get_agent_permission_mode,
            commands::models_cmd::set_agent_permission_mode,
            commands::models_cmd::configure_agent_to_proxy,
            commands::models_cmd::set_agent_model_cmd,
            commands::search_cmd::search_conversations,
            commands::search_cmd::save_conversation_message,
            commands::search_cmd::get_conversation_messages,
            commands::search_cmd::save_session,
            commands::search_cmd::get_sessions,
            commands::search_cmd::update_session_title,
            commands::search_cmd::update_session_model,
            commands::search_cmd::delete_session,
            commands::search_cmd::archive_session,
            commands::search_cmd::unarchive_session,
            commands::search_cmd::delete_archived_sessions,
            commands::search_cmd::touch_session,
            commands::session_cmd::start_session,
            commands::session_cmd::stop_session,
            commands::session_cmd::session_alive,
            commands::session_cmd::send_input,
            commands::session_cmd::respond_interaction,
            commands::session_cmd::set_session_permission_mode,
            commands::session_cmd::respond_permission,
            commands::session_cmd::list_sessions,
            commands::session_cmd::get_session_logs,
            commands::project_cmd::list_projects,
            commands::fs_cmd::list_dir,
            commands::fs_cmd::read_file_text,
            commands::fs_cmd::write_file,
            commands::fs_cmd::read_file_bytes,
            commands::fs_cmd::get_file_size,
            commands::fs_cmd::search_files,
            commands::fs_cmd::list_mention_entries,
            commands::fs_cmd::search_mention_files,
            commands::fs_cmd::parse_file,
            commands::fs_cmd::create_dir,
            commands::fs_cmd::create_file,
            commands::fs_cmd::rename_path,
            commands::fs_cmd::delete_path,
            commands::cost_cmd::get_cost_summary,
            commands::cost_cmd::get_cost_by_agent,
            commands::cost_cmd::get_cost_by_day,
            commands::cost_cmd::get_cost_by_session,
            commands::cost_cmd::get_cost_by_directory,
            commands::app_cmd::get_data_dir,
            commands::app_cmd::open_data_dir,
            commands::app_cmd::open_in_finder,
            commands::app_cmd::reveal_path,
            commands::app_cmd::open_app_tab,
            commands::app_cmd::close_app_tab,
            commands::app_cmd::set_app_tab_visible,
            commands::app_cmd::layout_app_tabs,
            commands::app_cmd::app_tab_navigate,
            commands::proxy_cmd::get_proxy_port,
            commands::proxy_cmd::get_proxy_url,
            commands::proxy_cmd::set_reasoning_disabled,
            commands::llama_cmd::check_llama_server_available,
            commands::llama_cmd::get_llama_server_status,
            commands::llama_cmd::get_server_status,
            commands::llama_cmd::list_llama_models,
            commands::llama_cmd::start_llama_server,
            commands::llama_cmd::stop_llama_server,
            commands::llama_cmd::download_llama_model,
            commands::llama_cmd::get_download_status,
            commands::llama_cmd::open_llama_models_dir,
            commands::llama_cmd::create_llama_model,
            commands::term_cmd::spawn_terminal,
            commands::term_cmd::write_terminal,
            commands::term_cmd::kill_terminal,
            commands::term_cmd::resize_terminal,
            commands::term_cmd::take_terminal_pending,
            commands::term_cmd::get_terminal_shell_mode,
            commands::term_cmd::get_terminal_cwd,
            commands::telemetry_cmd::get_telemetry_status,
            commands::telemetry_cmd::set_telemetry_enabled,
            commands::telemetry_cmd::track_event,
            commands::telemetry_cmd::submit_feedback,
            commands::telemetry_cmd::check_for_updates,
            commands::telemetry_cmd::get_proxy_config,
            commands::telemetry_cmd::set_proxy_config,
            commands::telemetry_cmd::test_proxy,
            commands::telemetry_cmd::check_update_ui,
            commands::telemetry_cmd::install_update,
            commands::telemetry_cmd::get_announcements,
            commands::telemetry_cmd::mark_announcement_read,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let _ = commands::llama_cmd::stop_llama_server();
                // Terminate terminal shells — otherwise they leak as orphaned
                // processes after the app exits (each idle interactive shell
                // can keep polling prompt state and burning CPU).
                commands::term_cmd::kill_all_terminals(window.app_handle());
            }
        })
        .setup(|app| {
            telemetry::set_app_handle(app.handle().clone());

            // Register the device and record app start (enqueued locally,
            // flushed in the background — never blocks startup).
            // NOTE: the Mutex guard must be dropped before calling
            // telemetry::register/track, because those functions lock
            // Database.conn themselves (std::sync::Mutex is not reentrant —
            // holding the lock here would deadlock and stall startup).
            let enabled = {
                let db = app.state::<Mutex<Database>>();
                let guard = db.lock().unwrap();
                let conn = guard.conn.lock().unwrap();
                telemetry::is_enabled(&conn)
            };
            if enabled {
                let version = app.package_info().version.to_string();
                let platform = commands::telemetry_cmd::platform_name();
                let db = app.state::<Mutex<Database>>();
                let guard = db.lock().unwrap();
                telemetry::register(&guard, &version, platform, std::env::consts::ARCH, "", true);
                telemetry::track(&guard, "app_started", serde_json::json!({ "version": version }));
            }
            telemetry::flush_async(app.handle());

            // Background worker: drain the queue periodically.
            let worker_handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(600));
                telemetry::flush_async(&worker_handle);
            });

            // Warm the terminal's cached user PATH in the background: probing the
            // interactive shell's PATH (nvm/conda/etc.) takes seconds on the
            // first run, so start it now so the first terminal spawn never blocks.
            std::thread::spawn(|| commands::term_cmd::prefetch_user_path());

            // `titleBarStyle: Overlay` + `hiddenTitle` in tauri.conf.json are
            // macOS-only. On Windows they are ignored, so the app would show a
            // native title bar on top of our custom in-app nav bar (the h-8
            // `data-tauri-drag-region` strip), leaving a big empty gap between
            // the title bar and the content. Remove the window decorations on
            // non-macOS so the custom title bar takes over instead.
            #[cfg(not(target_os = "macos"))]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_decorations(false);
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
