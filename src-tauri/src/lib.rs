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
pub mod log_util;

use commands::term_cmd::TerminalState;
use db::connection::Database;
use session::runner::SessionManager;
use state::AppState;
use proxy::ProxyState;
use std::sync::{Arc, Mutex};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    search::init_db();
    let app_dir = directories::ProjectDirs::from("com", "runjam", "RunJam")
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    
    let db = Database::new(&app_dir).expect("Failed to create database");
    db::migrations::run_migrations(&db.conn.lock().unwrap());
    
    // Ensure default session working directory exists
    if let Some(home) = directories::UserDirs::new() {
        std::fs::create_dir_all(home.home_dir().join(".runjam").join("session")).ok();
    }

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
        .manage(Mutex::new(SessionManager::new()))
        .manage(Mutex::new(AppState::load()))
        .manage(Mutex::new(db))
        .manage(proxy_state)
        .manage(Mutex::new(TerminalState::new()))
        .invoke_handler(tauri::generate_handler![
            commands::agent_cmd::detect_agents,
            commands::agent_cmd::check_agent,
            commands::agent_cmd::install_agent,
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
            commands::search_cmd::delete_session,
            commands::search_cmd::archive_session,
            commands::search_cmd::unarchive_session,
            commands::search_cmd::delete_archived_sessions,
            commands::session_cmd::start_session,
            commands::session_cmd::stop_session,
            commands::session_cmd::send_input,
            commands::session_cmd::respond_interaction,
            commands::session_cmd::respond_permission,
            commands::session_cmd::list_sessions,
            commands::session_cmd::get_session_logs,
            commands::project_cmd::list_projects,
            commands::fs_cmd::list_dir,
            commands::fs_cmd::read_file_text,
            commands::fs_cmd::write_file,
            commands::fs_cmd::read_file_bytes,
            commands::cost_cmd::get_cost_summary,
            commands::cost_cmd::get_cost_by_agent,
            commands::cost_cmd::get_cost_by_day,
            commands::cost_cmd::get_cost_by_session,
            commands::cost_cmd::get_cost_by_directory,
            commands::app_cmd::get_data_dir,
            commands::app_cmd::open_data_dir,
            commands::app_cmd::open_in_finder,
            commands::proxy_cmd::get_proxy_port,
            commands::proxy_cmd::get_proxy_url,
            commands::ollama_cmd::check_ollama_installed,
            commands::ollama_cmd::get_ollama_status,
            commands::ollama_cmd::list_ollama_models,
            commands::ollama_cmd::pull_ollama_model,
            commands::ollama_cmd::create_ollama_model,
            commands::term_cmd::spawn_terminal,
            commands::term_cmd::write_terminal,
            commands::term_cmd::kill_terminal,
            commands::term_cmd::resize_terminal,
            commands::term_cmd::get_terminal_cwd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
