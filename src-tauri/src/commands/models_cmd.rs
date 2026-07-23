use crate::models_config::{ModelEntry, ModelAlias, ModelConfig, sync_to_agent, backup_agent_config, read_models_from_agent_config, configure_agent_proxy, restore_agent_config, set_agent_model, detect_model_protocol};
use crate::db::connection::Database;
use crate::proxy::ProxyState;
use tauri::State;
use std::sync::{Arc, Mutex};

pub fn get_models_from_conn(conn: &rusqlite::Connection) -> Vec<ModelEntry> {
    let mut stmt = conn.prepare("SELECT id, name, alias, provider, provider_name, provider_icon, api_base, api_key, protocol, context_window, support_reasoning, support_tools, tags FROM models ORDER BY created_at").unwrap();
    let models_iter = stmt.query_map([], |row| {
        let tags_str: String = row.get(12)?;
        let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
        Ok(ModelEntry {
            id: row.get(0)?,
            name: row.get(1)?,
            alias: row.get(2)?,
            provider: row.get(3)?,
            provider_name: row.get(4)?,
            provider_icon: row.get(5)?,
            api_base: row.get(6)?,
            api_key: row.get(7)?,
            protocol: row.get(8)?,
            context_window: row.get(9)?,
            support_reasoning: row.get(10)?,
            support_tools: row.get(11)?,
            tags,
            use_proxy: false,
        })
    }).unwrap();
    
    let mut models = Vec::new();
    for model in models_iter {
        if let Ok(m) = model {
            models.push(m);
        }
    }
    models
}

#[tauri::command]
pub fn get_models(db: State<'_, Mutex<Database>>) -> Vec<ModelEntry> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    get_models_from_conn(&conn)
}

#[tauri::command]
pub fn save_models(models: Vec<ModelEntry>, db: State<'_, Mutex<Database>>) -> Result<(), String> {
    let db_guard = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let conn = db_guard.conn.lock().map_err(|e| format!("DB conn error: {}", e))?;
    
    // Temporarily disable foreign keys so we can do a full replace without
    // being blocked by agent_models / model_aliases referencing existing models.
    conn.execute("PRAGMA foreign_keys=OFF", [])
        .map_err(|e| format!("Failed to disable FK: {}", e))?;
    
    conn.execute("DELETE FROM models", []).map_err(|e| format!("Failed to clear models: {}", e))?;
    
    for model in &models {
        let tags_json = serde_json::to_string(&model.tags).unwrap_or_default();
        let protocol = detect_model_protocol(&model.name, Some(&model.api_base)).as_str().to_string();
        conn.execute(
            "INSERT INTO models (id, name, alias, provider, provider_name, provider_icon, api_base, api_key, protocol, context_window, support_reasoning, support_tools, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, CURRENT_TIMESTAMP)",
            rusqlite::params![
                &model.id, &model.name, &model.alias, &model.provider, &model.provider_name,
                &model.provider_icon, &model.api_base, &model.api_key, &protocol,
                model.context_window as i64, model.support_reasoning as i64, model.support_tools as i64, &tags_json,
            ],
        ).map_err(|e| format!("Failed to insert model {}: {}", model.name, e))?;
    }
    
    // Clean up orphaned child rows for models that were removed
    // (models whose id no longer exists in the models table).
    conn.execute(
        "DELETE FROM agent_models WHERE model_id NOT IN (SELECT id FROM models)",
        [],
    ).ok();
    conn.execute(
        "DELETE FROM model_aliases WHERE model_id NOT IN (SELECT id FROM models)",
        [],
    ).ok();
    
    // Re-enable foreign keys
    conn.execute("PRAGMA foreign_keys=ON", [])
        .map_err(|e| format!("Failed to re-enable FK: {}", e))?;
    
    // Save to models.json for proxy server
    ModelConfig { models: models.clone() }.save();
    
    Ok(())
}

#[tauri::command]
pub fn get_last_agent(db: State<'_, Mutex<Database>>) -> String {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    let result: Result<String, _> = conn.query_row(
        "SELECT value FROM app_settings WHERE key = 'last_agent'", [],
        |row| row.get(0)
    );
    result.unwrap_or_else(|_| "claude-code".to_string())
}

#[tauri::command]
pub fn set_last_agent(agent_id: String, db: State<'_, Mutex<Database>>) -> Result<(), String> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)",
        (&"last_agent", &agent_id),
    ).map_err(|e| format!("Failed to save last agent: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn get_agent_models(agent_id: String, db: State<'_, Mutex<Database>>) -> Vec<ModelEntry> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    
    let mut stmt = conn.prepare(
        "SELECT m.id, m.name, m.alias, m.provider, m.provider_name, m.provider_icon, m.api_base, m.api_key, m.protocol, m.context_window, m.support_reasoning, m.tags, am.use_proxy
         FROM models m 
         JOIN agent_models am ON m.id = am.model_id 
         WHERE am.agent_id = ? 
         ORDER BY am.is_default DESC, am.created_at ASC"
    ).unwrap();
    
    let models_iter = stmt.query_map([&agent_id], |row| {
        let tags_str: String = row.get(11)?;
        let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
        Ok(ModelEntry {
            id: row.get(0)?, name: row.get(1)?, alias: row.get(2)?,
            provider: row.get(3)?, provider_name: row.get(4)?, provider_icon: row.get(5)?,
            api_base: row.get(6)?, api_key: row.get(7)?, protocol: row.get(8)?,
            context_window: row.get(9)?, support_reasoning: row.get(10)?, support_tools: true, tags,
            use_proxy: row.get::<_, i32>(12)? != 0,
        })
    }).unwrap();
    
    let mut models = Vec::new();
    for model in models_iter { if let Ok(m) = model { models.push(m); } }
    models
}

#[derive(Debug, serde::Serialize)]
pub struct AgentModelInfo {
    pub agent_id: String,
    pub model_id: String,
    pub use_proxy: bool,
    pub is_default: bool,
}

#[tauri::command]
pub fn get_agent_model_map(db: State<'_, Mutex<Database>>) -> Vec<AgentModelInfo> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    
    let mut stmt = conn.prepare(
        "SELECT agent_id, model_id, use_proxy, is_default FROM agent_models"
    ).unwrap();
    
    let iter = stmt.query_map([], |row| {
        Ok(AgentModelInfo {
            agent_id: row.get(0)?,
            model_id: row.get(1)?,
            use_proxy: row.get::<_, i32>(2)? != 0,
            is_default: row.get::<_, i32>(3)? != 0,
        })
    }).unwrap();
    
    iter.filter_map(|r| r.ok()).collect()
}

#[tauri::command]
pub fn assign_model_to_agent(agent_id: String, model_id: String, use_proxy: bool, db: State<'_, Mutex<Database>>, proxy_state: State<'_, Arc<Mutex<ProxyState>>>) -> Result<(), String> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    
    // Check if any models already assigned — if none, this is the first → make it default
    let existing_count: i32 = conn.query_row(
        "SELECT COUNT(*) FROM agent_models WHERE agent_id = ?1",
        [&agent_id], |r| r.get(0)
    ).unwrap_or(0);
    let is_default = if existing_count == 0 { 1 } else { 0 };
    
    conn.execute(
        "INSERT OR IGNORE INTO agent_models (agent_id, model_id, use_proxy, is_default, created_at) VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP)",
        (&agent_id, &model_id, &use_proxy, &is_default),
    ).map_err(|e| format!("Failed to assign model: {}", e))?;
    
    backup_agent_config(&agent_id).ok();
    
    // Sync only the DEFAULT model to the agent's native config
    let default_models = get_default_models_for_sync(&conn, &agent_id);
    if !default_models.is_empty() {
        sync_to_agent(&agent_id, &default_models).map_err(|e| format!("Failed to sync to agent config: {}", e))?;
    }
    
    // Save ALL models to models.json for proxy server
    let all_models = get_all_models_from_db(&conn);
    ModelConfig { models: all_models }.save();
    {
        let mut ps = proxy_state.lock().unwrap();
        ps.models = ModelConfig::load().models;
        let ids: Vec<String> = get_agent_models_for_sync(&conn, &agent_id).iter().map(|m| m.id.clone()).collect();
        ps.agent_models.insert(agent_id.clone(), ids);
    }
    
    // Always configure agent to use the proxy — all model traffic goes through proxy
    let port = proxy_state.lock().unwrap().port;
    if port > 0 {
        let proxy_url = format!("http://127.0.0.1:{}", port);
        configure_agent_proxy(&agent_id, &proxy_url).ok();
    }
    
    Ok(())
}

#[tauri::command]
pub fn remove_model_from_agent(agent_id: String, model_id: String, db: State<'_, Mutex<Database>>, proxy_state: State<'_, Arc<Mutex<ProxyState>>>) -> Result<(), String> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    
    let was_default: bool = conn.query_row(
        "SELECT is_default FROM agent_models WHERE agent_id=?1 AND model_id=?2",
        (&agent_id, &model_id), |r| r.get::<_, i32>(0)
    ).map(|v| v != 0).unwrap_or(false);
    
    conn.execute(
        "DELETE FROM agent_models WHERE agent_id = ?1 AND model_id = ?2",
        (&agent_id, &model_id),
    ).map_err(|e| format!("Failed to remove model: {}", e))?;
    
    // If we removed the default, promote the first remaining model to default
    if was_default {
        conn.execute(
            "UPDATE agent_models SET is_default = 1 WHERE agent_id = ?1 AND model_id = (SELECT model_id FROM agent_models WHERE agent_id = ?1 ORDER BY created_at ASC LIMIT 1)",
            [&agent_id],
        ).ok();
    }
    
    let models = get_agent_models_for_sync(&conn, &agent_id);
    if models.is_empty() {
        restore_agent_config(&agent_id).ok();
    } else {
        let default_models = get_default_models_for_sync(&conn, &agent_id);
        if !default_models.is_empty() {
            sync_to_agent(&agent_id, &default_models).map_err(|e| format!("Failed to sync to agent config: {}", e))?;
        }
    }
    
    {
        let mut ps = proxy_state.lock().unwrap();
        let ids: Vec<String> = models.iter().map(|m| m.id.clone()).collect();
        if ids.is_empty() {
            ps.agent_models.remove(&agent_id);
        } else {
            ps.agent_models.insert(agent_id.clone(), ids);
        }
    }
    
    Ok(())
}

fn get_all_models_from_db(conn: &rusqlite::Connection) -> Vec<ModelEntry> {
    let mut stmt = conn.prepare(
        "SELECT id, name, alias, provider, provider_name, provider_icon, api_base, api_key, protocol, context_window, support_reasoning, tags FROM models ORDER BY created_at"
    ).unwrap();
    let models_iter = stmt.query_map([], |row| {
        let tags_str: String = row.get(11)?;
        let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
        Ok(ModelEntry {
            id: row.get(0)?, name: row.get(1)?, alias: row.get(2)?,
            provider: row.get(3)?, provider_name: row.get(4)?, provider_icon: row.get(5)?,
            api_base: row.get(6)?, api_key: row.get(7)?, protocol: row.get(8)?,
            context_window: row.get(9)?, support_reasoning: row.get(10)?, support_tools: true, tags,
            use_proxy: false,
        })
    }).unwrap();
    models_iter.filter_map(|m| m.ok()).collect()
}

fn get_agent_models_for_sync(conn: &rusqlite::Connection, agent_id: &str) -> Vec<ModelEntry> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.name, m.alias, m.provider, m.provider_name, m.provider_icon, m.api_base, m.api_key, m.protocol, m.context_window, m.support_reasoning, m.tags 
         FROM models m JOIN agent_models am ON m.id = am.model_id 
         WHERE am.agent_id = ? ORDER BY am.is_default DESC, am.created_at ASC"
    ).unwrap();
    let models_iter = stmt.query_map([agent_id], |row| {
        let tags_str: String = row.get(11)?;
        let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
        Ok(ModelEntry {
            id: row.get(0)?, name: row.get(1)?, alias: row.get(2)?,
            provider: row.get(3)?, provider_name: row.get(4)?, provider_icon: row.get(5)?,
            api_base: row.get(6)?, api_key: row.get(7)?, protocol: row.get(8)?,
            context_window: row.get(9)?, support_reasoning: row.get(10)?, support_tools: true, tags,
            use_proxy: false,
        })
    }).unwrap();
    models_iter.filter_map(|m| m.ok()).collect()
}

/// Get only the default model for sync — this is what gets written to the agent config.
fn get_default_models_for_sync(conn: &rusqlite::Connection, agent_id: &str) -> Vec<ModelEntry> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.name, m.alias, m.provider, m.provider_name, m.provider_icon, m.api_base, m.api_key, m.protocol, m.context_window, m.support_reasoning, m.tags 
         FROM models m JOIN agent_models am ON m.id = am.model_id 
         WHERE am.agent_id = ? AND am.is_default = 1
         ORDER BY am.created_at ASC LIMIT 1"
    ).unwrap();
    let models_iter = stmt.query_map([agent_id], |row| {
        let tags_str: String = row.get(11)?;
        let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
        Ok(ModelEntry {
            id: row.get(0)?, name: row.get(1)?, alias: row.get(2)?,
            provider: row.get(3)?, provider_name: row.get(4)?, provider_icon: row.get(5)?,
            api_base: row.get(6)?, api_key: row.get(7)?, protocol: row.get(8)?,
            context_window: row.get(9)?, support_reasoning: row.get(10)?, support_tools: true, tags,
            use_proxy: false,
        })
    }).unwrap();
    models_iter.filter_map(|m| m.ok()).collect()
}

#[tauri::command]
pub fn set_agent_default_model(agent_id: String, model_id: String, db: State<'_, Mutex<Database>>, proxy_state: State<'_, Arc<Mutex<ProxyState>>>) -> Result<(), String> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    
    // Unset current default
    conn.execute("UPDATE agent_models SET is_default = 0 WHERE agent_id = ?1", [&agent_id])
        .map_err(|e| format!("Failed to unset default: {}", e))?;
    // Set new default
    conn.execute("UPDATE agent_models SET is_default = 1 WHERE agent_id = ?1 AND model_id = ?2", (&agent_id, &model_id))
        .map_err(|e| format!("Failed to set default: {}", e))?;
    
    // Re-sync the new default model to agent config
    backup_agent_config(&agent_id).ok();
    let default_models = get_default_models_for_sync(&conn, &agent_id);
    if !default_models.is_empty() {
        sync_to_agent(&agent_id, &default_models).map_err(|e| format!("Failed to sync to agent config: {}", e))?;
        
        let mapping = get_agent_model_map_generic(&conn);
        if let Some(info) = mapping.iter().find(|m| m.agent_id == agent_id && m.is_default) {
            if info.use_proxy {
                let port = proxy_state.lock().unwrap().port;
                if port > 0 {
                    configure_agent_proxy(&agent_id, &format!("http://127.0.0.1:{}", port)).ok();
                }
            }
        }
    }
    
    Ok(())
}

#[tauri::command]
pub fn get_agent_default_model(agent_id: String, db: State<'_, Mutex<Database>>) -> String {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    let result: Result<String, _> = conn.query_row(
        "SELECT model_id FROM agent_models WHERE agent_id = ?1 AND is_default = 1 LIMIT 1",
        [&agent_id], |row| row.get(0)
    );
    result.unwrap_or_default()
}

#[tauri::command]
pub fn get_session_model(agent_id: String, db: State<'_, Mutex<Database>>) -> String {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    let result: Result<String, _> = conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?",
        [&format!("session_model_{}", agent_id)], |row| row.get(0)
    );
    result.unwrap_or_default()
}

#[tauri::command]
pub fn set_session_model(agent_id: String, model_id: String, db: State<'_, Mutex<Database>>) -> Result<(), String> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)",
        (&format!("session_model_{}", agent_id), &model_id),
    ).map_err(|e| format!("Failed to save session model: {}", e))?;
    Ok(())
}

// Helper for internal use without tauri State
fn get_agent_model_map_generic(conn: &rusqlite::Connection) -> Vec<AgentModelInfo> {
    let mut stmt = conn.prepare("SELECT agent_id, model_id, use_proxy, is_default FROM agent_models").unwrap();
    stmt.query_map([], |row| {
        Ok(AgentModelInfo {
            agent_id: row.get(0)?,
            model_id: row.get(1)?,
            use_proxy: row.get::<_, i32>(2)? != 0,
            is_default: row.get::<_, i32>(3)? != 0,
        })
    }).unwrap().filter_map(|r| r.ok()).collect()
}

#[tauri::command]
pub fn read_agent_config_models(agent_id: String) -> Vec<ModelEntry> {
    read_models_from_agent_config(&agent_id)
}

#[tauri::command]
pub fn get_model_aliases(db: State<'_, Mutex<Database>>) -> Vec<ModelAlias> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    let mut stmt = conn.prepare("SELECT alias, model_id, description FROM model_aliases").unwrap();
    stmt.query_map([], |row| {
        Ok(ModelAlias { alias: row.get(0)?, model_id: row.get(1)?, description: row.get(2)? })
    }).unwrap().filter_map(|r| r.ok()).collect()
}

#[tauri::command]
pub fn add_model_alias(alias: String, model_id: String, description: String, db: State<'_, Mutex<Database>>) -> Result<(), String> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    conn.execute("INSERT OR REPLACE INTO model_aliases (alias, model_id, description, created_at) VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)",
        (&alias, &model_id, &description),
    ).map_err(|e| format!("Failed to add alias: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn remove_model_alias(alias: String, db: State<'_, Mutex<Database>>) -> Result<(), String> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    conn.execute("DELETE FROM model_aliases WHERE alias = ?", [&alias])
        .map_err(|e| format!("Failed to remove alias: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn get_model_by_alias(alias: String, db: State<'_, Mutex<Database>>) -> Option<ModelEntry> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    let result: Result<(String,), _> = conn.query_row(
        "SELECT model_id FROM model_aliases WHERE alias = ?", [&alias], |row| Ok((row.get(0)?,))
    );
    match result {
        Ok((model_id,)) => {
            let mut stmt = conn.prepare("SELECT id, name, alias, provider, provider_name, provider_icon, api_base, api_key, protocol, context_window, support_reasoning, tags FROM models WHERE id = ?").unwrap();
            let mut models_iter = stmt.query_map([&model_id], |row| {
                let tags_str: String = row.get(11)?;
                let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
                Ok(ModelEntry {
                    id: row.get(0)?, name: row.get(1)?, alias: row.get(2)?,
                    provider: row.get(3)?, provider_name: row.get(4)?, provider_icon: row.get(5)?,
                    api_base: row.get(6)?, api_key: row.get(7)?, protocol: row.get(8)?,
                    context_window: row.get(9)?, support_reasoning: row.get(10)?, support_tools: true, tags,
                    use_proxy: false,
                })
            }).unwrap();
            models_iter.next().and_then(|m| m.ok())
        }
        Err(_) => None,
    }
}

#[tauri::command]
pub fn sync_model_to_all_agents(model_id: String, db: State<'_, Mutex<Database>>) -> Result<(), String> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    for agent_id in &["claude-code", "codex-cli", "gemini-cli"] {
        conn.execute(
            "INSERT OR IGNORE INTO agent_models (agent_id, model_id, created_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)",
            (agent_id, &model_id),
        ).map_err(|e| format!("Failed to sync to {}: {}", agent_id, e))?;
    }
    Ok(())
}

#[tauri::command]
pub fn set_default_model(agent_id: String, model_id: String, db: State<'_, Mutex<Database>>) -> Result<(), String> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)",
        (&format!("default_model_{}", agent_id), &model_id),
    ).map_err(|e| format!("Failed to set default model: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn get_default_model(agent_id: String, db: State<'_, Mutex<Database>>) -> String {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    let result: Result<String, _> = conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?",
        [&format!("default_model_{}", agent_id)], |row| row.get(0)
    );
    result.unwrap_or_default()
}

#[tauri::command]
pub fn get_agent_permission_mode(agent_id: String, db: State<'_, Mutex<Database>>) -> String {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    let result: Result<String, _> = conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?",
        [&format!("agent_permission_{}", agent_id)], |row| row.get(0)
    );
    result.unwrap_or_default()
}

/// Update the agent config file with the selected model's name and API key.
/// Called when user selects a model in the chat dialog, so the agent
/// sends the correct model name to the proxy.
/// Preserves the proxy URL (already set by configure_agent_proxy).
#[tauri::command]
pub fn set_agent_model_cmd(agent_id: String, model_id: String, db: State<'_, Mutex<Database>>) -> Result<(), String> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    let (model_name, api_key): (String, String) = conn.query_row(
        "SELECT name, api_key FROM models WHERE id = ?1",
        [&model_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|e| format!("Model not found: {}", e))?;
    set_agent_model(&agent_id, &model_name, &api_key)
}

#[tauri::command]
pub fn set_agent_permission_mode(agent_id: String, mode: String, db: State<'_, Mutex<Database>>) -> Result<(), String> {
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)",
        (&format!("agent_permission_{}", agent_id), &mode),
    ).map_err(|e| format!("Failed to save agent permission mode: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn configure_agent_to_proxy(agent_id: String, proxy_url: String) -> Result<(), String> {
    configure_agent_proxy(&agent_id, &proxy_url)
}
