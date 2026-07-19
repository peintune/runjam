use crate::acp::{AcpEvent, AcpMessage, PermissionOption};
use crate::cost::tracker;
use crate::db::connection::Database;
use crate::node_util;
use crate::rjlog;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: Value,
}

#[derive(Debug, Deserialize)]
struct SessionUpdate {
    method: String,
    params: SessionUpdateParams,
}

#[derive(Debug, Deserialize)]
struct SessionUpdateParams {
    #[serde(rename = "sessionId")]
    session_id: String,
    update: UpdateWrapper,
}

#[derive(Debug, Deserialize)]
struct UpdateWrapper {
    #[serde(rename = "sessionUpdate")]
    session_update: String,
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    used: Option<u64>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    cost: Option<serde_json::Value>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    #[serde(rename = "rawOutput")]
    raw_output: Option<String>,
    #[serde(default)]
    #[serde(rename = "rawInput")]
    raw_input: Option<Value>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default, rename = "_meta")]
    _meta: Option<Value>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    #[serde(rename = "inputTokens")]
    input_tokens: Option<u64>,
    #[serde(default)]
    #[serde(rename = "outputTokens")]
    output_tokens: Option<u64>,
}

/// Helper: extract tool_name from UpdateWrapper (tries top-level, then _meta.claudeCode.toolName)
fn get_tool_name(w: &UpdateWrapper) -> String {
    if let Some(ref name) = w.tool_name {
        if !name.is_empty() { return name.clone(); }
    }
    let from_meta = w._meta.as_ref()
        .and_then(|m| m.get("claudeCode"))
        .and_then(|c| c.get("toolName"))
        .and_then(|v| v.as_str());
    if let Some(name) = from_meta {
        if !name.is_empty() { return name.to_string(); }
    }
    // Debug: if still empty, log _meta structure
    if let Some(ref meta) = w._meta {
        rjlog!("[ACP DEBUG] get_tool_name: _meta present but no toolName found. _meta keys: {:?}", meta.as_object().map(|o| o.keys().collect::<Vec<_>>()));
    } else {
        rjlog!("[ACP DEBUG] get_tool_name: _meta is None, tool_name={:?}", w.tool_name);
    }
    String::new()
}

/// Helper: extract text content from UpdateWrapper's content field (handles both simple {type,text} and array format)
fn get_content_text(w: &UpdateWrapper) -> String {
    match &w.content {
        Some(Value::Object(obj)) => {
            obj.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string()
        }
        Some(Value::Array(arr)) => {
            arr.iter()
                .filter_map(|item| {
                    item.get("content")
                        .and_then(|c| c.get("text"))
                        .and_then(|v| v.as_str())
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => String::new(),
    }
}

/// Helper: extract tool output (tries rawOutput first, then content field)
fn get_tool_output(w: &UpdateWrapper) -> String {
    if let Some(ref raw) = w.raw_output {
        if !raw.is_empty() { return raw.clone(); }
    }
    get_content_text(w)
}

/// Helper: extract tool input (tries input string, then serializes rawInput JSON)
fn get_tool_input(w: &UpdateWrapper) -> String {
    if let Some(ref inp) = w.input {
        if !inp.is_empty() { return inp.clone(); }
    }
    if let Some(ref raw) = w.raw_input {
        if let Value::Object(obj) = raw {
            if obj.is_empty() { return String::new(); }
            return serde_json::to_string(raw).unwrap_or_default();
        }
        return serde_json::to_string(raw).unwrap_or_default();
    }
    String::new()
}

/// Helper: get title from UpdateWrapper
fn get_title(w: &UpdateWrapper) -> Option<String> {
    w.title.clone().filter(|t| !t.is_empty())
}

#[derive(Debug, Deserialize)]
struct PermissionRequest {
    method: String,
    params: PermissionRequestParams,
}

#[derive(Debug, Deserialize)]
struct PermissionRequestParams {
    #[serde(rename = "requestId")]
    request_id: String,
    prompt: String,
    options: Vec<PermissionOption>,
}

#[derive(Debug, Deserialize)]
struct SessionPermissionRequest {
    method: String,
    id: u64,
    params: SessionPermissionRequestParams,
}

#[derive(Debug, Deserialize)]
struct SessionPermissionRequestParams {
    #[serde(rename = "sessionId")]
    session_id: String,
    options: Vec<SessionPermissionOptionRaw>,
    #[serde(rename = "toolCall")]
    tool_call: SessionToolCallInfo,
}

#[derive(Debug, Deserialize)]
struct SessionPermissionOptionRaw {
    kind: String,
    name: String,
    #[serde(default, rename = "optionId")]
    option_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionToolCallInfo {
    #[serde(rename = "toolCallId")]
    _tool_call_id: String,
    #[serde(default)]
    title: Option<String>,
}

/// Map current platform to the Codex ACP npm package suffix (e.g. "darwin-arm64").
fn get_codex_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") { "darwin-arm64" } else { "darwin-x64" }
    } else if cfg!(target_os = "linux") {
        if cfg!(target_arch = "aarch64") { "linux-arm64" } else { "linux-x64" }
    } else if cfg!(target_os = "windows") {
        "win32-x64"
    } else {
        "linux-x64" // fallback
    }
}

/// Resolve the command path, args, and working directory for an agent,
/// using RunJam's bundled Node.js and installing ACP packages to the
/// RunJam data dir as needed.
fn resolve_agent_paths(
    app: &AppHandle,
    agent_type: &str,
) -> Result<(String, Vec<String>, String), String> {
    let node_bin = node_util::resolve_node_bin(app)
        .ok_or_else(|| "Node.js not found. Please install Node.js or rebuild the application.".to_string())?;
    let npm_bin = node_util::resolve_npm_bin(app)
        .ok_or_else(|| "npm not found. Please install Node.js or rebuild the application.".to_string())?;

    let data_dir = node_util::get_runjam_data_dir();
    let acp_dir = data_dir.join("acp");
    std::fs::create_dir_all(&acp_dir).ok();

    match agent_type {
        "claude" | "claude-code" => {
            let pkg_name = "@agentclientprotocol/claude-agent-acp";
            let install_dir = acp_dir.join("claude-agent-acp");
            std::fs::create_dir_all(&install_dir).ok();

            let pkg_dir = install_dir.join("node_modules").join("@agentclientprotocol").join("claude-agent-acp");
            let entry_point = pkg_dir.join("dist").join("index.js");

            if !entry_point.exists() {
                rjlog!("[ACP] Installing {}...", pkg_name);
                let output = Command::new(&npm_bin)
                    .args(["install", "--no-save", pkg_name])
                    .current_dir(&install_dir)
                    .output()
                    .map_err(|e| format!("Failed to install {}: {}", pkg_name, e))?;
                if !output.status.success() {
                    return Err(format!(
                        "Failed to install {}: {}",
                        pkg_name,
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
                rjlog!("[ACP] Installed {}", pkg_name);
            }

            Ok((
                node_bin.to_string_lossy().to_string(),
                vec![entry_point.to_string_lossy().to_string()],
                pkg_dir.to_string_lossy().to_string(),
            ))
        }
        "codex" | "codex-cli" => {
            let os_arch = get_codex_platform();
            let pkg_name = format!("@zed-industries/codex-acp-{}", os_arch);
            let install_dir = acp_dir.join("codex-acp");
            std::fs::create_dir_all(&install_dir).ok();

            let pkg_dir = install_dir.join("node_modules").join("@zed-industries").join(format!("codex-acp-{}", os_arch));
            let binary_name = if cfg!(target_os = "windows") { "codex-acp.exe" } else { "codex-acp" };
            let binary_path = pkg_dir.join("bin").join(binary_name);

            if !binary_path.exists() {
                rjlog!("[ACP] Installing {}...", pkg_name);
                let output = Command::new(&npm_bin)
                    .args(["install", "--no-save", &pkg_name])
                    .current_dir(&install_dir)
                    .output()
                    .map_err(|e| format!("Failed to install {}: {}", pkg_name, e))?;
                if !output.status.success() {
                    return Err(format!(
                        "Failed to install {}: {}",
                        pkg_name,
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
                rjlog!("[ACP] Installed {}", pkg_name);
            }

            Ok((
                binary_path.to_string_lossy().to_string(),
                vec![],
                pkg_dir.to_string_lossy().to_string(),
            ))
        }
        "gemini" | "gemini-cli" => {
            // Gemini has native ACP support via --acp flag
            Ok((
                "gemini".to_string(),
                vec!["--acp".to_string()],
                std::env::current_dir().map(|d| d.to_string_lossy().to_string()).unwrap_or_default(),
            ))
        }
        _ => Err(format!("Unknown agent type: {}", agent_type)),
    }
}

pub struct AcpClient {
    process: Child,
    stdin_writer: Arc<Mutex<std::process::ChildStdin>>,
    request_id: Arc<Mutex<u64>>,
    session_id: Arc<Mutex<String>>,
    responses: Arc<Mutex<HashMap<u64, Value>>>,
    cwd: String,
    mode: String,
    permission_mode: String,
}

impl AcpClient {
    pub fn start(
        app: &AppHandle,
        session_id: &str,
        agent_type: &str,
        directory: Option<&str>,
        model: Option<&str>,
        mode: &str,
        permission_mode: &str,
    ) -> Result<Self, String> {
        rjlog!("[ACP DEBUG] Starting ACP client for session: {}, agent: {}, directory: {:?}, model: {:?}, mode: {}, permission_mode: {}", session_id, agent_type, directory, model, mode, permission_mode);

        let (cmd_path, args, package_dir) = resolve_agent_paths(app, agent_type)?;

        rjlog!("[ACP DEBUG] Command path: {}", cmd_path);
        rjlog!("[ACP DEBUG] Args: {:?}", args);
        rjlog!("[ACP DEBUG] Package dir: {}", package_dir);

        let cwd = directory.unwrap_or(&package_dir);
        rjlog!("[ACP DEBUG] Using cwd: {}", cwd);

        let mut cmd = Command::new(cmd_path);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(cwd);

        if agent_type == "codex" {
            // Read API key and base_url from model_providers.custom (native codex config)
            if let Ok(home) = std::env::var("HOME") {
                let config_path = std::path::PathBuf::from(&home).join(".codex").join("config.toml");
                rjlog!("[CODEX ENV] Reading config from: {}", config_path.display());
                match std::fs::read_to_string(&config_path) {
                    Ok(content) => {
                        match toml::from_str::<toml::Value>(&content) {
                            Ok(doc) => {
                                let mut found_key = false;
                                let mut found_url = false;
                                if let Some(providers) = doc.get("model_providers") {
                                    if let Some(custom) = providers.get("custom") {
                                        if let Some(key) = custom.get("api_key").and_then(|v| v.as_str()) {
                                            if !key.is_empty() {
                                                let masked = if key.len() > 8 {
                                                    format!("{}...{}", &key[..4], &key[key.len()-4..])
                                                } else { key.to_string() };
                                                rjlog!("[CODEX ENV] Found api_key in config: {}, setting OPENAI_API_KEY + CODEX_API_KEY", masked);
                                                cmd.env("OPENAI_API_KEY", key);
                                                cmd.env("CODEX_API_KEY", key);
                                                found_key = true;
                                            } else {
                                                rjlog!("[CODEX ENV] api_key in config is empty");
                                            }
                                        } else {
                                            rjlog!("[CODEX ENV] No api_key found in [model_providers.custom]");
                                        }
                                        if let Some(base) = custom.get("base_url").and_then(|v| v.as_str()) {
                                            if !base.is_empty() {
                                                rjlog!("[CODEX ENV] Found base_url in config: {}", base);
                                                cmd.env("OPENAI_API_BASE", base);
                                                found_url = true;
                                            }
                                        }
                                    } else {
                                        rjlog!("[CODEX ENV] No [model_providers.custom] found");
                                    }
                                } else {
                                    rjlog!("[CODEX ENV] No [model_providers] section found");
                                }
                                if !found_key {
                                    rjlog!("[CODEX ENV] No api_key from config, checking system env vars...");
                                }
                                if !found_url {
                                    rjlog!("[CODEX ENV] No base_url from config");
                                }
                            }
                            Err(e) => {
                                rjlog!("[CODEX ENV] Failed to parse config TOML: {}", e);
                            }
                        }
                        // Debug: verify OPENAI_API_KEY was actually set on the process
                        let env_key_check = std::env::var("OPENAI_API_KEY").unwrap_or_default();
                        if !env_key_check.is_empty() {
                            let masked = if env_key_check.len() > 8 {
                                format!("{}...{}", &env_key_check[..4], &env_key_check[env_key_check.len()-4..])
                            } else { env_key_check.clone() };
                            rjlog!("[CODEX ENV] Parent process OPENAI_API_KEY env = {} (inherited, may be overridden)", masked);
                        }
                    }
                    Err(e) => {
                        rjlog!("[CODEX ENV] Failed to read config file: {}", e);
                    }
                }
            } else {
                rjlog!("[CODEX ENV] HOME env var not set, cannot read codex config");
            }
            // Fallback: set system env vars last (lower priority)
            if let Ok(sys_key) = std::env::var("OPENAI_API_KEY") {
                let masked = if sys_key.len() > 8 {
                    format!("{}...{}", &sys_key[..4], &sys_key[sys_key.len()-4..])
                } else { sys_key.clone() };
                rjlog!("[CODEX ENV] System OPENAI_API_KEY = {} (fallback, will not override config)", masked);
            }
            if let Ok(sys_key) = std::env::var("CODEX_API_KEY") {
                rjlog!("[CODEX ENV] System CODEX_API_KEY present (fallback)");
            }
            if let Ok(sys_base) = std::env::var("CODEX_API_BASE") {
                rjlog!("[CODEX ENV] System CODEX_API_BASE = {} (fallback)", sys_base);
            }
        } else if agent_type == "gemini" {

            // Pass API keys from gemini settings.json
            if let Ok(home) = std::env::var("HOME") {
                let settings_path = std::path::PathBuf::from(&home).join(".gemini").join("settings.json");
                if let Ok(content) = std::fs::read_to_string(&settings_path) {
                    if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(env) = settings.get("env").and_then(|v| v.as_object()) {
                            for (k, v) in env {
                                if let Some(val) = v.as_str() {
                                    cmd.env(k, val);
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut process = cmd.spawn().map_err(|e| {
            rjlog!("[ACP ERROR] Failed to start ACP agent: {}", e);
            format!("Failed to start ACP agent: {}", e)
        })?;

        rjlog!("[ACP DEBUG] ACP agent process started, PID: {}", process.id());

        let stdout = process.stdout.take().ok_or("No stdout")?;
        let stderr = process.stderr.take().ok_or("No stderr")?;

        let _app_clone = app.clone();
        thread::spawn(move || {
            rjlog!("[ACP DEBUG] Started stderr reader");
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                rjlog!("[ACP STDERR] {}", line);
            }
            rjlog!("[ACP DEBUG] Stderr reader exited");
        });

        let responses = Arc::new(Mutex::new(HashMap::new()));
        let responses_clone = responses.clone();
        let app_clone2 = app.clone();
        let session_id_clone = session_id.to_string();
        let permission_mode_clone = permission_mode.to_string();
        let _agent_type_clone = agent_type.to_string();
        let model_clone = model.map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string());
        
        // Track cumulative token usage to calculate deltas
        let last_used = Arc::new(AtomicU64::new(0));
        let last_used_clone = last_used.clone();
        let last_input = Arc::new(AtomicU64::new(0));
        let last_input_clone = last_input.clone();
        let last_output = Arc::new(AtomicU64::new(0));
        let last_output_clone = last_output.clone();
        
        // Track tool call start times for duration calculation
        let tool_start_times: Arc<Mutex<HashMap<String, u64>>> = Arc::new(Mutex::new(HashMap::new()));
        let tool_times_clone = tool_start_times.clone();

        let stdin_writer = Arc::new(Mutex::new(process.stdin.take().ok_or("No stdin")?));
        let stdin_writer_clone = stdin_writer.clone();

        thread::spawn(move || {
            rjlog!("[ACP DEBUG] Started stdout reader");
            let reader = BufReader::new(stdout);
            let mut tool_times = tool_times_clone.lock().unwrap();
            for line in reader.lines().flatten() {
                rjlog!("[ACP RAW] {}", line);
                if let Ok(val) = serde_json::from_str::<Value>(&line) {
                    if val.get("method").is_some() {
                        // Try to parse as permission request first
                        if let Ok(perm_req) = serde_json::from_value::<PermissionRequest>(val.clone()) {
                            if perm_req.method == "permission/request" {
                                rjlog!("[ACP DEBUG] Received permission/request: {}", perm_req.params.prompt);
                                
                                // Auto-approve based on permission mode
                                let should_auto_approve = matches!(permission_mode_clone.as_str(), 
                                    "full_access" | "approve_for_me");
                                
                                if should_auto_approve {
                                    rjlog!("[ACP DEBUG] Auto-approving permission request due to mode: {}", permission_mode_clone);
                                    let approve_option = perm_req.params.options.iter()
                                        .find(|o| o.key == "allow_once" || o.key == "allow")
                                        .or_else(|| perm_req.params.options.first())
                                        .cloned();
                                    if let Some(opt) = approve_option {
                                        let response_params = serde_json::json!({
                                            "requestId": perm_req.params.request_id,
                                            "response": opt.key,
                                        });
                                        let response_request = JsonRpcRequest {
                                            jsonrpc: "2.0".to_string(),
                                            id: 99999,
                                            method: "permission/response".to_string(),
                                            params: response_params,
                                        };
                                        if let Ok(response_json) = serde_json::to_string(&response_request) {
                                            rjlog!("[ACP DEBUG] Sending auto-permission response: {}", response_json);
                                            if let Ok(mut stdin) = stdin_writer_clone.lock() {
                                                let _ = writeln!(&mut stdin, "{}", response_json);
                                                let _ = stdin.flush();
                                            }
                                        }
                                    }
                                } else {
                                    let event_name = format!("acp:{}", session_id_clone);
                                    let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                        &session_id_clone, "0", "0",
                                        AcpEvent::PermissionRequest {
                                            request_id: perm_req.params.request_id,
                                            prompt: perm_req.params.prompt,
                                            options: perm_req.params.options,
                                        }
                                    ));
                                }
                                continue;
                            }
                        }
                        // Try to parse as session/request_permission (has an id field for response)
                        if let Ok(sess_perm) = serde_json::from_value::<SessionPermissionRequest>(val.clone()) {
                            if sess_perm.method == "session/request_permission" {
                                let prompt = sess_perm.params.tool_call.title.clone()
                                    .unwrap_or_else(|| "Tool permission".to_string());
                                let options: Vec<PermissionOption> = sess_perm.params.options.iter().map(|o| {
                                    PermissionOption {
                                        key: o.option_id.clone().unwrap_or_else(|| o.kind.clone()),
                                        label: o.name.clone(),
                                        is_default: o.kind == "allow_once",
                                    }
                                }).collect();
                                rjlog!("[ACP DEBUG] Received session/request_permission id={} prompt={}", sess_perm.id, prompt);
                                
                                // Auto-approve based on permission mode
                                let should_auto_approve = matches!(permission_mode_clone.as_str(), 
                                    "full_access" | "approve_for_me");
                                
                                if should_auto_approve {
                                    rjlog!("[ACP DEBUG] Auto-approving session/request_permission due to mode: {}", permission_mode_clone);
                                    let approve_option = options.iter()
                                        .find(|o| o.key == "allow_once" || o.key == "allow")
                                        .or_else(|| options.first())
                                        .cloned();
                                    if let Some(opt) = approve_option {
                                        let outcome_val = serde_json::json!({
                                            "outcome": "selected",
                                            "optionId": opt.key,
                                        });
                                        let resp_json = serde_json::json!({
                                            "jsonrpc": "2.0",
                                            "id": sess_perm.id,
                                            "result": { "outcome": outcome_val }
                                        });
                                        if let Ok(response_str) = serde_json::to_string(&resp_json) {
                                            rjlog!("[ACP DEBUG] Sending auto-permission response: {}", response_str);
                                            if let Ok(mut stdin) = stdin_writer_clone.lock() {
                                                let _ = writeln!(&mut stdin, "{}", response_str);
                                                let _ = stdin.flush();
                                            }
                                        }
                                    }
                                } else {
                                    let event_name = format!("acp:{}", session_id_clone);
                                    let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                        &session_id_clone, "0", "0",
                                        AcpEvent::PermissionRequest {
                                            request_id: format!("session:{}", sess_perm.id),
                                            prompt,
                                            options,
                                        }
                                    ));
                                }
                                continue;
                            }
                        }
                        // Then try to parse as session update
                        if let Ok(update) = serde_json::from_value::<SessionUpdate>(val) {
                            if update.method == "session/update" {
                                let update_type = update.params.update.session_update.as_str();
                                let tool_name_str = get_tool_name(&update.params.update);
                                rjlog!("[ACP DEBUG] Received session/update type='{}' tool_name={} content_len={} input_len={}",
                                    update_type,
                                    tool_name_str,
                                    get_content_text(&update.params.update).len(),
                                    update.params.update.input.as_ref().map(|s| s.len()).unwrap_or(0),
                                );
                                let event_name = format!("acp:{}", session_id_clone);
                                match update.params.update.session_update.as_str() {
                                    "agent_message_chunk" => {
                                        let text = get_content_text(&update.params.update);
                                        if !text.is_empty() {
                                            rjlog!("[ACP DEBUG] Agent message chunk: {} chars", text.len());
                                            let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                                &session_id_clone, "0", "0",
                                                AcpEvent::Text { content: text }
                                            ));
                                        }
                                    }
                                    "agent_message_end" => {
                                        rjlog!("[ACP DEBUG] Agent message end");
                                        let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                            &session_id_clone, "0", "0",
                                            AcpEvent::Finish { stop_reason: "end".to_string() }
                                        ));
                                    }
                                    "agent_thought_chunk" => {
                                        let text = get_content_text(&update.params.update);
                                        if !text.is_empty() {
                                            rjlog!("[ACP DEBUG] Agent thought chunk: {} chars", text.len());
                                            let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                                &session_id_clone, "0", "0",
                                                AcpEvent::Thinking { content: text, status: "thinking".to_string(), duration: None }
                                            ));
                                        }
                                    }
                                    "agent_thought_end" => {
                                        rjlog!("[ACP DEBUG] Agent thought end");
                                        let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                            &session_id_clone, "0", "0",
                                            AcpEvent::Thinking { content: String::new(), status: "done".to_string(), duration: None }
                                        ));
                                    }
                                    "thinking" => {
                                        let text = get_content_text(&update.params.update);
                                        if !text.is_empty() {
                                            rjlog!("[ACP DEBUG] Thinking update: {} chars", text.len());
                                            let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                                &session_id_clone, "0", "0",
                                                AcpEvent::Thinking { content: text, status: "thinking".to_string(), duration: None }
                                            ));
                                        }
                                    }
                                    "message_chunk" => {
                                        let text = get_content_text(&update.params.update);
                                        if !text.is_empty() {
                                            rjlog!("[ACP DEBUG] Message chunk: {} chars", text.len());
                                            let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                                &session_id_clone, "0", "0",
                                                AcpEvent::Text { content: text }
                                            ));
                                        }
                                    }
                                    "message_end" => {
                                        rjlog!("[ACP DEBUG] Message end");
                                        let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                            &session_id_clone, "0", "0",
                                            AcpEvent::Finish { stop_reason: "end".to_string() }
                                        ));
                                    }
                                    "usage_update" => {
                                        let used = update.params.update.used.unwrap_or(0);
                                        let input_tokens = update.params.update.input_tokens.unwrap_or(0);
                                        let output_tokens = update.params.update.output_tokens.unwrap_or(0);
                                        let prev = last_used_clone.load(Ordering::SeqCst);
                                        let prev_input = last_input_clone.load(Ordering::SeqCst);
                                        let prev_output = last_output_clone.load(Ordering::SeqCst);
                                        let delta = if used > prev { used - prev } else { 0 };
                                        let input_delta = if input_tokens > prev_input { input_tokens - prev_input } else { 0 };
                                        let output_delta = if output_tokens > prev_output { output_tokens - prev_output } else { 0 };
                                        // If agent doesn't provide separate input/output, split total delta
                                        let (record_input, record_output) = if input_delta > 0 || output_delta > 0 {
                                            (input_delta as i64, output_delta as i64)
                                        } else {
                                            (delta as i64, 0)
                                        };
                                        let usage_model = update.params.update.model.clone()
                                            .filter(|m| !m.is_empty())
                                            .unwrap_or_else(|| model_clone.clone());
                                        rjlog!("[ACP DEBUG] Usage update: used={}, delta={}, input_delta={}, output_delta={}, model={}", used, delta, record_input, record_output, usage_model);
                                        if delta > 0 || record_input > 0 || record_output > 0 {
                                            last_used_clone.store(used, Ordering::SeqCst);
                                            if input_delta > 0 { last_input_clone.store(input_tokens, Ordering::SeqCst); }
                                            if output_delta > 0 { last_output_clone.store(output_tokens, Ordering::SeqCst); }
                                            // Record to database via Tauri state
                                            if let Some(state) = app_clone2.try_state::<Mutex<Database>>() {
                                                if let Ok(db) = state.lock() {
                                                    if let Ok(conn) = db.conn.lock() {
                                                        tracker::record_usage(
                                                            &conn,
                                                            &session_id_clone,
                                                            &usage_model,
                                                            record_input,
                                                            record_output,
                                                            0.0,
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    "available_commands_update" => {
                                        rjlog!("[ACP DEBUG] Available commands update");
                                    }
                                    "tool_call" | "tool_call_update" => {
                                        let tool_name = get_tool_name(&update.params.update);
                                        let input = get_tool_input(&update.params.update);
                                        let title = get_title(&update.params.update);
                                        let update_status = update.params.update.status.clone().unwrap_or_default();

                                        // Terminal statuses: completed, failed, error — emit tool_result to stop the timer
                                        let is_terminal = matches!(update_status.as_str(), "completed" | "failed" | "error");
                                        if is_terminal {
                                            let output = get_tool_output(&update.params.update);
                                            let start_time = tool_times.get(&tool_name).copied();
                                            let duration_ms = tool_times.remove(&tool_name).map(|start| {
                                                let now = chrono::Utc::now().timestamp_millis() as u64;
                                                now.saturating_sub(start)
                                            });
                                            rjlog!("[ACP DEBUG] Tool call terminal ({}): {} output_len={} duration_ms={:?} start_time={:?}",
                                                update_status, tool_name, output.len(), duration_ms, start_time);

                                            // Only emit tool_result — frontend updates the matching running tool_call
                                            let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                                &session_id_clone, "0", "0",
                                                AcpEvent::ToolResult { tool_name: tool_name.clone(), output, duration_ms, title }
                                            ));
                                        } else {
                                            // Normal tool_call or tool_call_update (not completed)
                                            let is_new = update.params.update.session_update == "tool_call";
                                            let status = if is_new { "started" } else { "running" };
                                            let start_time = if is_new {
                                                let now = chrono::Utc::now().timestamp_millis() as u64;
                                                tool_times.insert(tool_name.clone(), now);
                                                Some(now)
                                            } else {
                                                tool_times.get(&tool_name).copied()
                                            };
                                            rjlog!("[ACP DEBUG] Tool call: {} status={} start_time={:?} input={}", tool_name, status, start_time,
                                                &input[..input.len().min(100)]);
                                            let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                                &session_id_clone, "0", "0",
                                                AcpEvent::ToolCall { tool_name, input, status: status.to_string(), start_time, title }
                                            ));
                                        }
                                    }
                                    "tool_call_end" => {
                                        let tool_name = get_tool_name(&update.params.update);
                                        let input = get_tool_input(&update.params.update);
                                        let start_time = tool_times.get(&tool_name).copied();
                                        rjlog!("[ACP DEBUG] Tool call end: {} start_time={:?}", tool_name, start_time);
                                        let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                            &session_id_clone, "0", "0",
                                            AcpEvent::ToolCall { tool_name, input, status: "completed".to_string(), start_time, title: None }
                                        ));
                                    }
                                    "tool_result" => {
                                        let tool_name = get_tool_name(&update.params.update);
                                        let output = get_tool_output(&update.params.update);
                                        let duration_ms = tool_times.remove(&tool_name).map(|start| {
                                            let now = chrono::Utc::now().timestamp_millis() as u64;
                                            now.saturating_sub(start)
                                        });
                                        rjlog!("[ACP DEBUG] Tool result: {} duration_ms={:?}", tool_name, duration_ms);
                                        let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                            &session_id_clone, "0", "0",
                                            AcpEvent::ToolResult { tool_name, output, duration_ms, title: None }
                                        ));
                                    }
                                    _ => {
                                        rjlog!("[ACP DEBUG] Unknown update type: {} — raw update JSON keys: {:?}",
                                            update.params.update.session_update,
                                            &line[..line.len().min(300)]
                                        );
                                    }
                                }
                            } else {
                                rjlog!("[ACP DEBUG] Unknown method: {}", update.method);
                            }
                        } else {
                            rjlog!("[ACP DEBUG] Failed to parse session update: {}", line);
                        }
                    } else if val.get("result").is_some() || val.get("error").is_some() {
                        rjlog!("[ACP DEBUG] Response: {}", line);
                        if let Some(error) = val.get("error") {
                            let err_msg = error.get("data")
                                .and_then(|d| d.get("message"))
                                .and_then(|m| m.as_str())
                                .or_else(|| error.get("message").and_then(|m| m.as_str()))
                                .unwrap_or("Unknown ACP error");
                            rjlog!("[ACP ERROR] Emitting error event: {}", err_msg);
                            let event_name = format!("acp:{}", session_id_clone);
                            let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                &session_id_clone, "0", "0",
                                AcpEvent::Error { message: err_msg.to_string() }
                            ));
                        }
                        if let Some(result) = val.get("result") {
                            if result.get("stopReason").is_some() {
                                rjlog!("[ACP DEBUG] Received stopReason, sending finish");
                                let event_name = format!("acp:{}", session_id_clone);
                                let stop_reason = result.get("stopReason")
                                    .and_then(|r| r.as_str())
                                    .unwrap_or("unknown");
                                let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                                    &session_id_clone, "0", "0",
                                    AcpEvent::Finish { stop_reason: stop_reason.to_string() }
                                ));
                            }
                        }
                        if let Some(id) = val.get("id").and_then(|v| v.as_u64()) {
                            responses_clone.lock().unwrap().insert(id, val);
                        }
                    } else {
                        rjlog!("[ACP DEBUG] Unknown JSON: {}", line);
                    }
                } else {
                    rjlog!("[ACP DEBUG] Not JSON: {}", line);
                }
            }
            rjlog!("[ACP DEBUG] Stdout reader exited - process may have terminated");
            let event_name = format!("acp:{}", session_id_clone);
            let _ = app_clone2.emit(&event_name, &AcpMessage::new(
                &session_id_clone, "0", "0",
                AcpEvent::Finish { stop_reason: "process_exit".to_string() }
            ));
        });

        let mut client = Self {
            process,
            stdin_writer,
            request_id: Arc::new(Mutex::new(1)),
            session_id: Arc::new(Mutex::new(session_id.to_string())),
            responses,
            cwd: cwd.to_string(),
            mode: mode.to_string(),
            permission_mode: permission_mode.to_string(),
        };

        Ok(client)
    }

    pub fn initialize_session(&mut self) -> Result<(), String> {
        rjlog!("[ACP DEBUG] Performing ACP handshake...");
        self.initialize()?;
        self.new_session()?;
        rjlog!("[ACP DEBUG] ACP handshake completed");
        Ok(())
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = {
            let mut guard = self.request_id.lock().unwrap();
            let id = *guard;
            *guard += 1;
            id
        };

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };

        let json = serde_json::to_string(&request).map_err(|e| format!("Failed to serialize request: {}", e))?;
        rjlog!("[ACP DEBUG] Sending request: {}", json);

        let mut stdin = self.stdin_writer.lock().map_err(|e| e.to_string())?;
        writeln!(&mut stdin, "{}", json).map_err(|e| format!("Failed to write to stdin: {}", e))?;
        stdin.flush().map_err(|e| format!("Failed to flush stdin: {}", e))?;

        for _ in 0..100 {
            thread::sleep(std::time::Duration::from_millis(50));
            let mut responses = self.responses.lock().unwrap();
            if let Some(val) = responses.remove(&id) {
                if let Some(error) = val.get("error") {
                    return Err(format!("RPC error: {}", error));
                }
                return Ok(val.get("result").cloned().unwrap_or(serde_json::json!({})));
            }
        }

        Err("Timeout waiting for response".to_string())
    }

    fn send_request_no_wait(&mut self, method: &str, params: Value) -> Result<(), String> {
        let id = {
            let mut guard = self.request_id.lock().unwrap();
            let id = *guard;
            *guard += 1;
            id
        };

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };

        let json = serde_json::to_string(&request).map_err(|e| format!("Failed to serialize request: {}", e))?;
        rjlog!("[ACP DEBUG] Sending request (no wait): {}", json);

        let mut stdin = self.stdin_writer.lock().map_err(|e| e.to_string())?;
        writeln!(&mut stdin, "{}", json).map_err(|e| format!("Failed to write to stdin: {}", e))?;
        stdin.flush().map_err(|e| format!("Failed to flush stdin: {}", e))?;

        Ok(())
    }

    fn initialize(&mut self) -> Result<(), String> {
        let params = serde_json::json!({
            "protocolVersion": 1,
            "clientCapabilities": {
                "auth": {
                    "terminal": true
                },
                "_meta": {
                    "terminal-auth": true
                }
            }
        });
        self.send_request("initialize", params)
            .map(|_| ())
            .map_err(|e| format!("Initialize failed: {}", e))
    }

    fn new_session(&mut self) -> Result<(), String> {
        let params = serde_json::json!({
            "cwd": self.cwd,
            "mcpServers": [],
            "mode": self.mode,
        });
        let result = self.send_request("session/new", params)
            .map_err(|e| format!("New session failed: {}", e))?;
        if let Some(session_id) = result.get("sessionId").and_then(|v| v.as_str()) {
            *self.session_id.lock().unwrap() = session_id.to_string();
            rjlog!("[ACP DEBUG] Received ACP session ID: {}", session_id);
        }
        Ok(())
    }

    pub fn send_prompt(&mut self, prompt: &str) -> Result<String, String> {
        rjlog!("[ACP DEBUG] Sending prompt: {} chars", prompt.len());
        let session_id = self.session_id.lock().unwrap().clone();
        let params = serde_json::json!({
            "sessionId": session_id,
            "prompt": [
                {
                    "type": "text",
                    "text": prompt
                }
            ]
        });
        self.send_request_no_wait("session/prompt", params)
            .map(|_| "ok".to_string())
            .map_err(|e| format!("Prompt failed: {}", e))
    }

    pub async fn test_connection(&mut self) -> Result<(), String> {
        rjlog!("[ACP DEBUG] Testing ACP connection...");
        self.initialize().map_err(|e| format!("Initialize failed: {}", e))?;
        rjlog!("[ACP DEBUG] ACP connection test successful");
        Ok(())
    }

    pub fn respond_permission(&mut self, request_id: &str, response: &str) -> Result<(), String> {
        rjlog!("[ACP DEBUG] Responding to permission request {}: {}", request_id, response);
        // Handle session/request_permission (request_id = "session:N")
        if let Some(id_str) = request_id.strip_prefix("session:") {
            if let Ok(rpc_id) = id_str.parse::<u64>() {
                // ACP v1 spec: result must contain `outcome` field of type RequestPermissionOutcome.
                // RequestPermissionOutcome is a tagged union:
                //   { outcome: "selected", optionId: "<id>" }   — user picked an option
                //   { outcome: "cancelled" }                    — user cancelled/denied
                // Gemini CLI specifically parses output.outcome.outcome to check for "cancelled".
                let response_lower = response.to_lowercase();
                let is_deny = response_lower.contains("cancel") || response_lower.contains("deny") || response_lower.contains("reject");

                let outcome_val = if is_deny {
                    serde_json::json!({
                        "outcome": "cancelled"
                    })
                } else {
                    serde_json::json!({
                        "outcome": "selected",
                        "optionId": response
                    })
                };

                let resp_json = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": rpc_id,
                    "result": { "outcome": outcome_val }
                });
                let resp_str = serde_json::to_string(&resp_json)
                    .map_err(|e| format!("Failed to serialize session permission response: {}", e))?;
                rjlog!("[ACP DEBUG] Sending session permission JSON-RPC response: {}", resp_str);
                let mut stdin = self.stdin_writer.lock().map_err(|e| e.to_string())?;
                writeln!(&mut stdin, "{}", resp_str)
                    .map_err(|e| format!("Failed to write session permission response: {}", e))?;
                stdin.flush().map_err(|e| format!("Failed to flush: {}", e))?;
                return Ok(());
            }
        }
        // Handle regular permission/request
        let params = serde_json::json!({
            "requestId": request_id,
            "response": response,
        });
        self.send_request_no_wait("permission/response", params)
            .map_err(|e| format!("Permission response failed: {}", e))
    }

    pub async fn new(agent_id: &str, _session_id: &str, app: &AppHandle) -> Result<Self, String> {
        rjlog!("[ACP DEBUG] Creating test ACP client for agent: {}", agent_id);

        let (cmd_path, args, package_dir) = resolve_agent_paths(app, agent_id)?;

        let mut cmd = Command::new(cmd_path);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(package_dir.clone());

        if agent_id == "codex-cli" || agent_id == "codex" {
            if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
                cmd.env("OPENAI_API_KEY", api_key);
            }
            if let Ok(api_key) = std::env::var("CODEX_API_KEY") {
                cmd.env("CODEX_API_KEY", api_key);
            }
        } else if agent_id == "gemini-cli" || agent_id == "gemini" {
            // Pass API keys from gemini settings.json
            if let Ok(home) = std::env::var("HOME") {
                let settings_path = std::path::PathBuf::from(&home).join(".gemini").join("settings.json");
                if let Ok(content) = std::fs::read_to_string(&settings_path) {
                    if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(env) = settings.get("env").and_then(|v| v.as_object()) {
                            for (k, v) in env {
                                if let Some(val) = v.as_str() {
                                    cmd.env(k, val);
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut process = cmd.spawn().map_err(|e| {
            rjlog!("[ACP ERROR] Failed to start ACP agent: {}", e);
            format!("Failed to start ACP agent: {}", e)
        })?;

        rjlog!("[ACP DEBUG] ACP agent process started for test, PID: {}", process.id());

        let stdout = process.stdout.take().ok_or("No stdout")?;
        let stderr = process.stderr.take().ok_or("No stderr")?;

        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                rjlog!("[ACP TEST STDERR] {}", line);
            }
        });

        let responses = Arc::new(Mutex::new(HashMap::new()));
        let responses_clone = responses.clone();
        
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().flatten() {
                rjlog!("[ACP TEST RAW] {}", line);
                if let Ok(val) = serde_json::from_str::<Value>(&line) {
                    if val.get("result").is_some() || val.get("error").is_some() {
                        if let Some(id) = val.get("id").and_then(|v| v.as_u64()) {
                            responses_clone.lock().unwrap().insert(id, val);
                        }
                    }
                }
            }
        });

        let stdin_writer = Arc::new(Mutex::new(process.stdin.take().ok_or("No stdin")?));
        let client = Self {
            process,
            stdin_writer,
            request_id: Arc::new(Mutex::new(1)),
            session_id: Arc::new(Mutex::new(_session_id.to_string())),
            responses,
            cwd: package_dir.clone(),
            mode: "default".to_string(),
            permission_mode: "ask_approval".to_string(),
        };

        Ok(client)
    }
}
