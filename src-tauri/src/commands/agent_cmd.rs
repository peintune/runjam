use crate::agent::detector;
use crate::acp_client::AcpClient;
use crate::db::connection::Database;
use crate::models::agent::Agent;
use crate::state::{AgentState, AppState};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Emitter, State};

#[derive(Debug, Clone, Serialize)]
pub struct AgentWithState {
    #[serde(flatten)]
    pub agent: Agent,
    pub enabled: bool,
}

fn merge_state(agents: Vec<Agent>, app_state: &AppState) -> Vec<AgentWithState> {
    agents
        .into_iter()
        .map(|a| {
            let state = app_state.get_agent(&a.id);
            AgentWithState {
                agent: a,
                enabled: state.enabled,
            }
        })
        .collect()
}

fn load_agent_status_from_db(conn: &rusqlite::Connection, agent_id: &str) -> (String, Option<String>) {
    let mut stmt = conn.prepare("SELECT status, last_tested_at FROM agents WHERE id = ?").unwrap();
    if let Some(row) = stmt.query_row([agent_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))).ok() {
        row
    } else {
        ("not_installed".to_string(), None)
    }
}

/// Update the agent's status and last_tested_at after a Test run.
/// Only touches these two columns; never wipes display_name / installed etc.
fn save_agent_status_to_db(conn: &rusqlite::Connection, agent_id: &str, status: &str, last_tested_at: &str) {
    conn.execute(
        "UPDATE agents SET status = ?1, last_tested_at = ?2 WHERE id = ?3",
        (status, last_tested_at, agent_id),
    ).ok();
}

/// Persist detection results (version, install path, etc.) without overwriting
/// the status that a manual Test already saved.
fn save_detected_agent(conn: &rusqlite::Connection, agent: &Agent) {
    conn.execute(
        "INSERT INTO agents (id, display_name, installed, status, version, install_path, detected_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
             display_name = excluded.display_name,
             installed    = excluded.installed,
             version      = excluded.version,
             install_path = excluded.install_path,
             detected_at  = excluded.detected_at",
        (
            &agent.id,
            &agent.display_name,
            agent.installed as i32,
            &agent.status,
            agent.version.as_deref().unwrap_or(""),
            agent.install_path.as_deref().unwrap_or(""),
            chrono::Local::now().to_rfc3339().as_str(),
        ),
    ).ok();
}

fn load_cached_agents(conn: &rusqlite::Connection) -> Option<Vec<Agent>> {
    let five_minutes_ago = chrono::Local::now() - chrono::Duration::minutes(5);
    let cutoff_time = five_minutes_ago.to_rfc3339();
    
    let mut stmt = conn.prepare("SELECT id, display_name, installed, status, version, install_path, last_tested_at FROM agents WHERE detected_at IS NOT NULL AND detected_at > ?").ok()?;
    
    let agents_iter = stmt.query_map([cutoff_time.as_str()], |row| {
        Ok(Agent {
            id: row.get(0)?,
            display_name: row.get(1)?,
            installed: row.get(2)?,
            status: row.get(3)?,
            version: row.get(4)?,
            install_path: row.get(5)?,
            last_tested_at: row.get(6)?,
        })
    }).ok()?;
    
    let mut agents: Vec<Agent> = agents_iter.filter_map(Result::ok).collect();
    
    if agents.len() >= 3 {
        Some(agents)
    } else {
        None
    }
}

/// Scan all installed agents, with enabled/disabled state.
#[tauri::command]
pub fn detect_agents(app_state: State<'_, Mutex<AppState>>) -> Vec<AgentWithState> {
    let state = app_state.lock().unwrap();
    merge_state(detector::detect_agents(), &state)
}

/// Check single agent.
#[tauri::command]
pub fn check_agent(agent_id: String, app_state: State<'_, Mutex<AppState>>) -> AgentWithState {
    let state = app_state.lock().unwrap();
    let agents = detector::detect_agents();
    let found = agents
        .into_iter()
        .find(|a| a.id == agent_id)
        .unwrap_or_else(|| Agent {
            id: agent_id.clone(),
            display_name: String::new(),
            install_path: None,
            version: None,
            installed: false,
            status: "not_installed".to_string(),
            last_tested_at: None,
        });
    AgentWithState {
        enabled: state.get_agent(&found.id).enabled,
        agent: found,
    }
}

/// Toggle agent enabled/disabled.
#[tauri::command]
pub fn set_agent_enabled(agent_id: String, enabled: bool, app_state: State<'_, Mutex<AppState>>) {
    let mut state = app_state.lock().unwrap();
    state.set_agent(&agent_id, AgentState { enabled });
}

/// Check if Node.js is installed. Returns version string or error.
/// Checks bundled Node.js first, then system PATH.
#[tauri::command]
pub fn check_nodejs(app: tauri::AppHandle) -> Result<String, String> {
    // Prefer bundled Node.js
    if let Some(node_bin) = crate::node_util::get_bundled_node_bin(&app) {
        let output = std::process::Command::new(&node_bin)
            .arg("--version")
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
    }

    // Fall back to system node
    let output = std::process::Command::new("node")
        .arg("--version")
        .output()
        .map_err(|_| "Node.js is not installed. Please install Node.js ≥ 18 from https://nodejs.org".to_string())?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(version)
    } else {
        Err("Node.js not found. Install from https://nodejs.org".to_string())
    }
}

/// Get the nodejs install guide for the current platform.
#[tauri::command]
pub fn get_nodejs_install_guide() -> String {
    if cfg!(target_os = "macos") {
        "brew install node".to_string()
    } else if cfg!(target_os = "windows") {
        "winget install OpenJS.NodeJS.LTS".to_string()
    } else {
        "curl -fsSL https://deb.nodesource.com/setup_lts.x | sudo -E bash - && sudo apt-get install -y nodejs".to_string()
    }
}

/// Open the Node.js download page in the default browser.
#[tauri::command]
pub fn open_nodejs_download() -> Result<(), String> {
    let url = if cfg!(target_os = "macos") {
        "https://nodejs.org/en/download"
    } else if cfg!(target_os = "windows") {
        "https://nodejs.org/en/download"
    } else {
        "https://nodejs.org/en/download"
    };

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Install an agent CLI.
#[tauri::command]
pub async fn install_agent(app: tauri::AppHandle, agent_id: String, db: State<'_, Mutex<Database>>) -> Result<Agent, String> {
    let node_bin_dir = ensure_nodejs(&app, &agent_id).await?;
    let npm_bin = if node_bin_dir.is_empty() { "npm".to_string() } else { format!("{}/npm", node_bin_dir) };
    let path_env = if node_bin_dir.is_empty() {
        crate::agent::detector::get_enhanced_path()
    } else {
        format!("{}:{}", node_bin_dir, std::env::var("PATH").unwrap_or_default())
    };

    let install_cmd = match agent_id.as_str() {
        "claude-code" => vec!["install", "-g", "@anthropic-ai/claude-code"],
        "codex-cli" => vec!["install", "-g", "@openai/codex"],
        "gemini-cli" => vec!["install", "-g", "@google/gemini-cli"],
        _ => return Err(format!("Unknown agent: {}", agent_id)),
    };

    let event_name = format!("agent-install:{}", agent_id);
    let _ = app.emit(
        &event_name,
        serde_json::json!({ "status": "installing", "message": format!("Running: {} {}", npm_bin, install_cmd.join(" ")) }),
    );

    let output = std::process::Command::new(&npm_bin)
        .args(&install_cmd)
        .env("PATH", &path_env)
        .output()
        .map_err(|e| format!("Failed to run installer: {}", e))?;

    if output.status.success() {
        let _ = app.emit(
            &event_name,
            serde_json::json!({ "status": "done", "message": "Installation complete" }),
        );

        let agents = detector::detect_agents();
        let agent = agents.into_iter().find(|a| a.id == agent_id).unwrap_or_else(|| Agent {
            id: agent_id.clone(),
            display_name: String::new(),
            install_path: None,
            version: None,
            installed: true,
            status: "not_installed".to_string(),
            last_tested_at: None,
        });

        // Update DB cache immediately
        let db_guard = db.lock().unwrap();
        let conn = db_guard.conn.lock().unwrap();
        save_detected_agent(&conn, &agent);
        drop(conn);
        drop(db_guard);

        Ok(agent)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = app.emit(
            &event_name,
            serde_json::json!({ "status": "error", "message": stderr }),
        );
        Err(format!("Installation failed: {}", stderr))
    }
}

/// Uninstall an agent CLI.
#[tauri::command]
pub async fn uninstall_agent(app: tauri::AppHandle, agent_id: String, db: State<'_, Mutex<Database>>) -> Result<(), String> {
    let node_bin_dir = ensure_nodejs(&app, &agent_id).await?;
    let npm_bin = if node_bin_dir.is_empty() { "npm".to_string() } else { format!("{}/npm", node_bin_dir) };
    let path_env = if node_bin_dir.is_empty() {
        crate::agent::detector::get_enhanced_path()
    } else {
        format!("{}:{}", node_bin_dir, std::env::var("PATH").unwrap_or_default())
    };

    let uninstall_cmd = match agent_id.as_str() {
        "claude-code" => vec!["uninstall", "-g", "@anthropic-ai/claude-code"],
        "codex-cli" => vec!["uninstall", "-g", "@openai/codex"],
        "gemini-cli" => vec!["uninstall", "-g", "@google/gemini-cli"],
        _ => return Err(format!("Unknown agent: {}", agent_id)),
    };

    let event_name = format!("agent-uninstall:{}", agent_id);
    let _ = app.emit(
        &event_name,
        serde_json::json!({ "status": "uninstalling", "message": format!("Running: {} {}", npm_bin, uninstall_cmd.join(" ")) }),
    );

    let output = std::process::Command::new(&npm_bin)
        .args(&uninstall_cmd)
        .env("PATH", &path_env)
        .output()
        .map_err(|e| format!("Failed to run uninstaller: {}", e))?;

    if output.status.success() {
        // Update cached detection so agent shows as not installed immediately
        let db_guard = db.lock().unwrap();
        let conn = db_guard.conn.lock().unwrap();
        conn.execute(
            "UPDATE agents SET installed = 0, status = 'not_installed', detected_at = ? WHERE id = ?",
            rusqlite::params![chrono::Local::now().to_rfc3339(), &agent_id],
        ).ok();
        drop(conn);
        drop(db_guard);

        let _ = app.emit(
            &event_name,
            serde_json::json!({ "status": "done", "message": "Uninstall complete" }),
        );
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Uninstall failed: {}", stderr))
    }
}

/// Read an agent's config file content.
#[tauri::command]
pub fn read_agent_config(agent_id: String) -> Result<String, String> {
    let config_path = match agent_id.as_str() {
        "claude-code" => dirs_home().join(".claude").join("settings.json"),
        "codex-cli" => dirs_home().join(".codex").join("config.toml"),
        "gemini-cli" => dirs_home().join(".gemini").join("settings.json"),
        _ => return Err(format!("Unknown agent: {}", agent_id)),
    };

    if config_path.exists() {
        std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config: {}", e))
    } else {
        Ok(String::new())
    }
}

/// Write an agent's config file content.
#[tauri::command]
pub fn write_agent_config(agent_id: String, content: String) -> Result<(), String> {
    let config_path = match agent_id.as_str() {
        "claude-code" => dirs_home().join(".claude").join("settings.json"),
        "codex-cli" => dirs_home().join(".codex").join("config.toml"),
        "gemini-cli" => dirs_home().join(".gemini").join("settings.json"),
        _ => return Err(format!("Unknown agent: {}", agent_id)),
    };

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    std::fs::write(&config_path, content)
        .map_err(|e| format!("Failed to write config: {}", e))
}

/// Get detailed info about an agent's data directory.
#[derive(Debug, Clone, Serialize)]
pub struct AgentDirInfo {
    pub path: String,
    pub exists: bool,
    pub total_size_bytes: u64,
    pub config_file: Option<String>,
    pub history_file: Option<String>,
    pub history_size_bytes: u64,
    pub history_lines: u64,
    pub subdirs: Vec<DirEntry>,
    pub key_files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirEntry {
    pub name: String,
    pub item_count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub size_bytes: u64,
}

#[tauri::command]
pub fn get_agent_dir_info(agent_id: String) -> AgentDirInfo {
    let dir = match agent_id.as_str() {
        "claude-code" => dirs_home().join(".claude"),
        "codex-cli" => dirs_home().join(".codex"),
        "gemini-cli" => dirs_home().join(".gemini"),
        _ => return AgentDirInfo {
            path: String::new(),
            exists: false,
            total_size_bytes: 0,
            config_file: None,
            history_file: None,
            history_size_bytes: 0,
            history_lines: 0,
            subdirs: vec![],
            key_files: vec![],
        },
    };

    let path_str = dir.to_string_lossy().to_string();

    if !dir.exists() {
        return AgentDirInfo {
            path: path_str,
            exists: false,
            total_size_bytes: 0,
            config_file: None,
            history_file: None,
            history_size_bytes: 0,
            history_lines: 0,
            subdirs: vec![],
            key_files: vec![],
        };
    }

    let total_size = dir_size(&dir);

    // Find config file
    let configs = ["settings.json", "config.toml", "config.json", "config.yml"];
    let config_file = configs.iter().find_map(|name| {
        let p = dir.join(name);
        if p.exists() { Some(name.to_string()) } else { None }
    });

    // History file
    let history_file = if dir.join("history.jsonl").exists() {
        Some("history.jsonl".to_string())
    } else {
        None
    };
    let (history_size, history_lines) = if let Some(ref hf) = history_file {
        let hp = dir.join(hf);
        let size = hp.metadata().map(|m| m.len()).unwrap_or(0);
        let lines = std::fs::read_to_string(&hp)
            .map(|s| s.lines().count() as u64)
            .unwrap_or(0);
        (size, lines)
    } else {
        (0, 0)
    };

    // Subdirs
    let subdirs: Vec<DirEntry> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() { return None; }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') { return None; }
            let count = std::fs::read_dir(entry.path())
                .map(|rd| rd.count() as u64)
                .unwrap_or(0);
            Some(DirEntry { name, item_count: count })
        })
        .collect();

    // Key files (non-hidden, non-dir)
    let key_files: Vec<FileEntry> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_file() { return None; }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') { return None; }
            let size = entry.metadata().ok()?.len();
            Some(FileEntry { name, size_bytes: size })
        })
        .collect();

    AgentDirInfo {
        path: path_str,
        exists: true,
        total_size_bytes: total_size,
        config_file,
        history_file,
        history_size_bytes: history_size,
        history_lines,
        subdirs,
        key_files,
    }
}

fn dir_size(path: &PathBuf) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    total += dir_size(&entry.path());
                } else {
                    total += meta.len();
                }
            }
        }
    }
    total
}

fn dirs_home() -> PathBuf {
    directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[tauri::command]
pub fn get_agent_statuses(app_state: State<'_, Mutex<AppState>>, db: State<'_, Mutex<Database>>, force_refresh: Option<bool>) -> Vec<AgentWithState> {
    let state = app_state.lock().unwrap();
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    
    let force = force_refresh.unwrap_or(false);
    
    // 1. Try cache first (detected within last 5 min, saves a filesystem scan)
    //    Skip cache when force_refresh is true
    let mut agents = if force {
        let detected = detector::detect_agents();
        for agent in &detected {
            save_detected_agent(&conn, agent);
        }
        detected
    } else {
        load_cached_agents(&conn).unwrap_or_else(|| {
            // 2. Cache miss or stale — full detection + persist
            let detected = detector::detect_agents();
            for agent in &detected {
                save_detected_agent(&conn, agent);
            }
            detected
        })
    };
    
    // 3. Overlay DB status/last_tested_at (preserved from Test runs)
    for agent in agents.iter_mut() {
        let (status, last_tested_at) = load_agent_status_from_db(&conn, &agent.id);
        if agent.installed {
            agent.status = status;
            agent.last_tested_at = last_tested_at;
        } else {
            agent.status = "not_installed".to_string();
            agent.last_tested_at = None;
        }
    }
    merge_state(agents, &state)
}

#[derive(Debug, Clone, Serialize)]
pub struct TestResult {
    pub success: bool,
    pub message: String,
}

#[tauri::command]
pub async fn test_agent(app: tauri::AppHandle, agent_id: String, db: State<'_, Mutex<Database>>) -> Result<TestResult, String> {
    let agents = detector::detect_agents();
    let agent = match agents.into_iter().find(|a| a.id == agent_id) {
        Some(a) => a,
        None => return Ok(TestResult { success: false, message: "Agent not found".to_string() }),
    };
    if !agent.installed {
        return Ok(TestResult { success: false, message: "Agent is not installed".to_string() });
    }
    
    let now = chrono::Local::now().to_rfc3339();

    // All agents now use ACP for testing
    let (status, message) = match AcpClient::new(&agent_id, &now, &app).await {
        Ok(mut client) => {
            match client.test_connection().await {
                Ok(_) => ("available".to_string(), "Connection successful".to_string()),
                Err(e) => ("connection_failed".to_string(), format!("Connection failed: {}", e)),
            }
        }
        Err(e) => ("connection_failed".to_string(), format!("Failed to create ACP client: {}", e)),
    };
    
    let db_guard = db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    save_agent_status_to_db(&conn, &agent_id, &status, &now);
    
    Ok(TestResult { success: status == "available", message })
}

/// Ensure Node.js is available. Prefers bundled Node.js from Tauri resources,
/// falls back to previously-downloaded Node.js in RunJam data dir, then system node.
async fn ensure_nodejs(app: &tauri::AppHandle, agent_id: &str) -> Result<String, String> {
    // 1. Check bundled Node.js (preferred — comes with the app)
    if let Some(bin_dir) = crate::node_util::get_bundled_node_bin_dir(app) {
        let bin_str = bin_dir.to_string_lossy().to_string();
        let npm_path = if cfg!(target_os = "windows") {
            format!("{}\\npm.cmd", bin_str)
        } else {
            format!("{}/npm", bin_str)
        };
        if std::path::Path::new(&npm_path).exists() {
            return Ok(bin_str);
        }
    }

    // 2. Check previously-downloaded Node.js in RunJam data dir
    let data_dir = dirs_data_dir();
    let node_dir = data_dir.join("nodejs").join("node-v22.12.0");
    let bin_dir = if cfg!(target_os = "windows") {
        node_dir.to_string_lossy().to_string()
    } else {
        node_dir.join("bin").to_string_lossy().to_string()
    };
    let npm_path = if cfg!(target_os = "windows") {
        format!("{}\\npm.cmd", bin_dir)
    } else {
        format!("{}/npm", bin_dir)
    };

    if std::path::Path::new(&npm_path).exists() {
        return Ok(bin_dir);
    }

    // 3. Check system node using enhanced PATH (includes Homebrew, nvm, etc.)
    let enhanced_path = crate::agent::detector::get_enhanced_path();
    if std::process::Command::new("npm")
        .arg("--version")
        .env("PATH", &enhanced_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok(String::new());
    }

    // 4. Download Node.js as last resort
    let _ = app.emit(&format!("agent-install:{}", agent_id), serde_json::json!({
        "status": "installing", "message": "Downloading Node.js v22.12.0..."
    }));

    let url = if cfg!(target_os = "macos") {
        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
        format!("https://nodejs.org/dist/v22.12.0/node-v22.12.0-darwin-{}.tar.gz", arch)
    } else if cfg!(target_os = "linux") {
        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
        format!("https://nodejs.org/dist/v22.12.0/node-v22.12.0-linux-{}.tar.gz", arch)
    } else {
        "https://nodejs.org/dist/v22.12.0/node-v22.12.0-win-x64.zip".to_string()
    };

    let archive_name = url.split('/').last().unwrap_or("node.tar.gz");
    let tmp = data_dir.join(archive_name);
    std::fs::create_dir_all(data_dir.parent().unwrap_or(&data_dir)).map_err(|e| format!("Failed to create dir: {}", e))?;

    // Download using curl
    let output = std::process::Command::new("curl")
        .args(["-fsSL", &url, "-o", tmp.to_string_lossy().as_ref()])
        .output().map_err(|e| format!("Download failed: {}", e))?;
    if !output.status.success() {
        return Err(format!("Download failed: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let _ = app.emit(&format!("agent-install:{}", agent_id), serde_json::json!({
        "status": "installing", "message": "Extracting Node.js..."
    }));

    // Extract
    let is_tar = archive_name.ends_with(".tar.gz");
    if is_tar {
        let output = std::process::Command::new("tar")
            .args(["-xzf", tmp.to_string_lossy().as_ref(), "-C", data_dir.to_string_lossy().as_ref()])
            .output().map_err(|e| format!("Extract failed: {}", e))?;
        if !output.status.success() {
            return Err(format!("Extract failed: {}", String::from_utf8_lossy(&output.stderr)));
        }
    } else {
        let output = std::process::Command::new("unzip")
            .args(["-o", tmp.to_string_lossy().as_ref(), "-d", data_dir.to_string_lossy().as_ref()])
            .output().map_err(|e| format!("Extract failed: {}", e))?;
        if !output.status.success() {
            return Err(format!("Extract failed: {}", String::from_utf8_lossy(&output.stderr)));
        }
    }

    // Find extracted directory (starts with "node-v")
    let extracted_dir = std::fs::read_dir(&data_dir).map_err(|e| format!("Read dir failed: {}", e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("node-v")).unwrap_or(false))
        .ok_or("Extracted directory not found".to_string())?;

    // Create parent for node_dir
    if let Some(parent) = std::path::Path::new(&bin_dir).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    // Move to canonical name
    if extracted_dir != node_dir {
        std::fs::rename(&extracted_dir, &node_dir).ok();
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(bin_dir)
}

fn dirs_data_dir() -> std::path::PathBuf {
    if let Some(dir) = directories::ProjectDirs::from("com", "runjam", "RunJam") {
        dir.data_local_dir().to_path_buf()
    } else {
        std::path::PathBuf::from(".")
    }
}
