//! Local HTTP proxy that translates between LLM API protocols.
//! Enables using any model provider with any Agent CLI.
//!
//! All proxy handlers support both sync and streaming modes:
//! - Sync: wait for full upstream response, convert, return.
//! - Stream: read upstream SSE line by line, convert on the fly, stream back.

use crate::models_config::{ModelConfig, ModelEntry};
use crate::rjlog;
use serde_json::Value;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;
use tiny_http::{Header, Response, Server, StatusCode};

/// 全局 usage 存储，用于 Proxy 存储检测到的 usage 数据，ACP Client 可以读取
struct UsageRecord {
    session_id: String,
    model: String,
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
    timestamp: Instant,
}

fn usage_store() -> &'static Mutex<Vec<UsageRecord>> {
    static STORE: std::sync::OnceLock<Mutex<Vec<UsageRecord>>> = std::sync::OnceLock::new();
    STORE.get_or_init(|| Mutex::new(Vec::new()))
}

/// 存储 usage 数据到全局存储（使用最近的请求作为默认 session）
pub fn store_usage_for_latest(model: String, input_tokens: i64, output_tokens: i64, cached_tokens: i64) {
    let mut store = usage_store().lock().unwrap();
    // 只保留最近 100 条记录
    if store.len() >= 100 {
        store.remove(0);
    }
    // 使用空字符串作为 session_id，后续通过 last_usage 获取
    store.push(UsageRecord {
        session_id: String::new(),
        model,
        input_tokens,
        output_tokens,
        cached_tokens,
        timestamp: Instant::now(),
    });
}

/// 获取并清除最近的 usage 数据（优先按 model 匹配，回退到最近一条）
///
/// 并发场景下，多个会话可能共用同一批上游模型名（前端模型 id 与 agent 实际
/// 请求的 model 名还可能不一致，如 deepseek-mrivjyj7 vs deepseek-v4-pro），
/// 所以先按 model 精确匹配；匹配不到时取最近一条，避免单会话场景拿不到。
pub fn take_last_usage(model: &str) -> Option<(String, i64, i64, i64)> {
    let mut store = usage_store().lock().unwrap();
    if store.is_empty() {
        return None;
    }
    let idx = store.iter().rposition(|r| r.model == model)
        .unwrap_or(store.len() - 1);
    let record = store.remove(idx);
    // 清理超过 5 分钟的旧记录
    store.retain(|r| r.timestamp.elapsed() < std::time::Duration::from_secs(300));
    Some((record.model, record.input_tokens, record.output_tokens, record.cached_tokens))
}

/// 获取最近的 usage 数据（不清除，优先按 model 匹配）
pub fn get_last_usage(model: &str) -> Option<(String, i64, i64, i64)> {
    let store = usage_store().lock().unwrap();
    let record = store.iter().rev().find(|r| r.model == model)
        .or_else(|| store.last())?;
    Some((record.model.clone(), record.input_tokens, record.output_tokens, record.cached_tokens))
}

/// 安全截断字符串到 max_bytes 字节以内，确保不会切在多字节 UTF-8 字符中间。
fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Response from handle_request: either a completed String response,
/// or a streaming data source for SSE endpoints.
enum ProxyResponse {
    Sync(StatusCode, String),
    Stream {
        reader: Box<dyn Read + Send>,
    },
}

/// Wraps a BufReader from the upstream SSE response and a line-conversion
/// closure into a `Read` impl that tiny_http can use as a streaming body.
struct SseStreamConverter {
    upstream: BufReader<Box<dyn Read + Send>>,
    convert: Box<dyn FnMut(&str) -> Vec<u8> + Send>,
    pending: Vec<u8>,
    pos: usize,
    done: bool,
    first: bool,
}

impl SseStreamConverter {
    fn new(
        upstream: BufReader<Box<dyn Read + Send>>,
        convert: Box<dyn FnMut(&str) -> Vec<u8> + Send>,
    ) -> Self {
        Self { upstream, convert, pending: Vec::new(), pos: 0, done: false, first: true }
    }
}

impl Read for SseStreamConverter {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            // Return pending data first
            if self.pos < self.pending.len() {
                let n = (self.pending.len() - self.pos).min(buf.len());
                buf[..n].copy_from_slice(&self.pending[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            if self.done {
                return Ok(0);
            }

            // Read next line from upstream SSE
            let mut line = String::new();
            let read_start = std::time::Instant::now();
            match self.upstream.read_line(&mut line) {
                Ok(0) => {
                    rjlog!("[PROXY STREAM] EOF from upstream reader");
                    self.done = true;
                    return Ok(0);
                }
                Ok(_) => {}
                Err(e) => {
                    rjlog!("[PROXY STREAM] Error reading upstream: {}", e);
                    self.done = true;
                    return Err(e);
                }
            }

            let trimmed = line.trim();
            if read_start.elapsed() > std::time::Duration::from_millis(100) {
                rjlog!("[PROXY STREAM] Slow read: {}ms, line: {} bytes", read_start.elapsed().as_millis(), trimmed.len());
            }
            if trimmed.is_empty()  || trimmed.len() == 0{
                continue;
            }
            rjlog!("[PROXY STREAM] Read line: {} ({} bytes)", safe_truncate(trimmed, 80), trimmed.len());
            let converted = if trimmed.starts_with("data: ") {
                (self.convert)(line.trim_end())
            } else if trimmed.is_empty() {
                // Empty line in SSE: separator between events, skip it
                continue;
            } else if trimmed.starts_with(':') {
                // SSE comment line (heartbeat/keep-alive), skip it
                continue;
            } else {
                // Non-data lines (event: etc.), skip
                continue;
            };

            if converted.is_empty() {
                rjlog!("[PROXY STREAM] Converted empty, skipping");
                continue;
            }

            self.first = false;
            self.pending = converted;
            self.pos = 0;
            rjlog!("[PROXY STREAM] Pending {} bytes for output", self.pending.len());
        }
    }
}

pub struct ProxyState {
    pub port: u16,
    pub running: bool,
    pub models: Vec<ModelEntry>,
    /// Maps agent_id → model ids assigned to that agent (used to disambiguate
    /// models that share the same name; lookups prefer the agent's own model).
    pub agent_models: HashMap<String, Vec<String>>,
    /// Whether reasoning mode is disabled (global state).
    pub reasoning_disabled: bool,
    /// Local response cache for exact-match requests (non-streaming, no tools)
    pub response_cache: HashMap<String, (String, std::time::Instant)>,
}

impl ProxyState {
    pub fn new() -> Self {
        Self {
            port: 0,
            running: false,
            models: vec![],
            agent_models: HashMap::new(),
            reasoning_disabled: false,
            response_cache: HashMap::new(),
        }
    }
}

/// Fixed port for the local proxy. A fixed port means agent configs only need
/// to be written once (when the proxy is enabled for that agent) and stay
/// valid across app restarts — no per-restart rewriting required.
const PROXY_PORT: u16 = 59268;

/// Start the proxy server on the fixed port.
/// Returns the port number.
pub fn start_proxy(state: Arc<Mutex<ProxyState>>) -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", PROXY_PORT))
        .map_err(|e| format!("Failed to bind port {}: {}", PROXY_PORT, e))?;
    let port = listener.local_addr().map_err(|e| format!("{}", e))?.port();

    {
        let mut s = state.lock().unwrap();
        s.port = port;
        s.running = true;
        s.models = ModelConfig::load().models;
    }

    let server = Server::from_listener(listener, None)
        .map_err(|e| format!("Failed to create server: {}", e))?;

    thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let path = request.url().to_string();
                let method = request.method().as_str().to_string();
                handle_request(&path, &method, &mut request, &state)
            }));
            match result {
                Ok(ProxyResponse::Sync(status, body)) => {
                    let response = Response::from_string(&body)
                        .with_status_code(status);
                    // SSE: add text/event-stream content type
                    if body.starts_with("data:") || body.starts_with("event:") {
                        let stream_response = Response::from_string(&body)
                            .with_status_code(200)
                            .with_header(Header::from_bytes("Content-Type", "text/event-stream").unwrap());
                        request.respond(stream_response).ok();
                    } else {
                        let response = response
                            .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
                        request.respond(response).ok();
                    }
                }
                Ok(ProxyResponse::Stream { mut reader }) => {
                    rjlog!("[PROXY STREAM] Sending streaming SSE response to agent");
                    let stream_start = std::time::Instant::now();
                    let mut writer = request.into_writer();
                    let _ = write!(&mut writer, "HTTP/1.1 200 OK\r\n");
                    let _ = write!(&mut writer, "Content-Type: text/event-stream\r\n");
                    let _ = write!(&mut writer, "Cache-Control: no-cache, no-store\r\n");
                    let _ = write!(&mut writer, "Transfer-Encoding: chunked\r\n");
                    let _ = write!(&mut writer, "X-Accel-Buffering: no\r\n");
                    let _ = write!(&mut writer, "\r\n");
                    let _ = writer.flush();
                    
                    rjlog!("[PROXY STREAM] Headers sent in {:?}", stream_start.elapsed());
                    let mut buf = [0u8; 4096];
                    loop {
                        let read_start = std::time::Instant::now();
                        match reader.read(&mut buf) {
                            Ok(0) => {
                                rjlog!("[PROXY STREAM] End of stream (total: {:?})", stream_start.elapsed());
                                let _ = write!(&mut writer, "0\r\n\r\n");
                                let _ = writer.flush();
                                break;
                            }
                            Ok(n) => {
                                if read_start.elapsed() > std::time::Duration::from_millis(100) {
                                    rjlog!("[PROXY STREAM] Slow upstream read: {}ms, got {} bytes", read_start.elapsed().as_millis(), n);
                                }
                                let _ = write!(&mut writer, "{:x}\r\n", n);
                                let _ = writer.write_all(&buf[..n]);
                                let _ = write!(&mut writer, "\r\n");
                                let _ = writer.flush();
                            }
                            Err(e) => {
                                rjlog!("[PROXY STREAM] Error reading from upstream: {}", e);
                                let _ = write!(&mut writer, "0\r\n\r\n");
                                let _ = writer.flush();
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    let detail = e.downcast_ref::<String>()
                        .map(|s| s.as_str())
                        .or_else(|| e.downcast_ref::<&str>().copied())
                        .unwrap_or("");
                    rjlog!("[PROXY] PANIC in handler: {}", detail);
                    let response = Response::from_string(
                        &format!(r#"{{"error":"Internal proxy error: {}"}}"#, detail)
                    )
                    .with_status_code(StatusCode(500))
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
                    request.respond(response).ok();
                }
            }
        }
    });

    Ok(port)
}

/// Find the best matching model entry by name or alias.
///
/// Resolution priority:
/// 1. Match by `name` or `alias` == the request model name:
///    a. Among matches, prefer the entry whose `id` is in `preferred_ids`
///       (the calling agent's assigned model ids).
///    b. Otherwise prefer non-empty api_key.
///    c. Fall back to the first match.
/// 2. No name match, but `preferred_ids` is given:
///    → use the first model whose `id` is in the agent's assigned set,
///      completely ignoring the request model name (we trust the assignment).
/// 3. Nothing found → None.
fn find_model<'a>(
    models: &'a [ModelEntry],
    model_name: &str,
    preferred_ids: Option<&[String]>,
) -> Option<&'a ModelEntry> {
    // --- Step 1: match by name / alias ---
    let matches: Vec<&ModelEntry> = models
        .iter()
        .filter(|m| m.name == model_name || m.alias == model_name)
        .collect();
    if !matches.is_empty() {
        if let Some(ids) = preferred_ids {
            if let Some(m) = matches.iter().copied().find(|m| ids.contains(&m.id)) {
                return Some(m);
            }
        }
        matches
            .iter()
            .copied()
            .find(|m| !m.api_key.is_empty())
            .or_else(|| matches.first().copied())
    }
    // --- Step 2: name didn't match, but agent has assigned models → use the first one ---
    else if let Some(ids) = preferred_ids {
        if let Some(m) = models.iter().find(|m| ids.contains(&m.id)) {
            return Some(m);
        }
        None
    }
    // --- Step 3: nothing ---
    else {
        None
    }
}

fn handle_request(
    path: &str, method: &str, request: &mut tiny_http::Request, state: &Arc<Mutex<ProxyState>>,
) -> ProxyResponse {
    rjlog!("[PROXY] >>> {} {} (from {}:{})", method, path, request.remote_addr().map(|a| a.to_string()).unwrap_or_default(), request.remote_addr().map(|a| a.port()).unwrap_or(0));
    if method != "POST" {
        if path == "/v1/models" || path == "/v1beta/models" {
            return ProxyResponse::Sync(StatusCode(200), r#"{"object":"list","data":[]}"#.to_string());
        }
        return ProxyResponse::Sync(StatusCode(405), "Method not allowed".to_string());
    }

    let body = {
        let mut buf = String::new();
        request.as_reader().read_to_string(&mut buf).ok();
        buf
    };

    rjlog!("[PROXY] <<< body ({} chars) first 300: {}", body.len(), safe_truncate(&body, 300));

    // Compute request hash for cache debugging
    let body_hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        body.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    };
    rjlog!("[CACHE DEBUG] Request hash: {} (path: {}, body_len: {})", body_hash, path, body.len());

    // Log message count and structure for debugging
    if let Ok(req_json) = serde_json::from_str::<Value>(&body) {
        let msg_count = req_json.get("messages").and_then(|m| m.as_array()).map(|arr| arr.len()).unwrap_or(0);
        let tools_count = req_json.get("tools").and_then(|t| t.as_array()).map(|arr| arr.len()).unwrap_or(0);
        let has_system = req_json.get("messages")
            .and_then(|m| m.as_array())
            .map(|arr| arr.iter().any(|msg| msg.get("role").and_then(|r| r.as_str()) == Some("system")))
            .unwrap_or(false);
        let stream = req_json.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
        rjlog!("[CACHE DEBUG] Structure: {} messages, {} tools, system_prompt={}, stream={}", 
            msg_count, tools_count, has_system, stream);
        
        // Log message roles sequence for comparison
        if msg_count > 0 {
            let roles: Vec<String> = req_json.get("messages")
                .and_then(|m| m.as_array())
                .map(|arr| {
                    arr.iter().map(|msg| msg.get("role").and_then(|r| r.as_str()).unwrap_or("?").to_string()).collect()
                })
                .unwrap_or_default();
            rjlog!("[CACHE DEBUG] Message roles: {:?}", roles);
        }
    }

    // Check local cache for exact-match non-streaming requests without tools
    let req_for_cache: Option<Value> = serde_json::from_str(&body).ok();
    let can_use_cache = req_for_cache.as_ref().map(|r| {
        let is_stream = r["stream"].as_bool().unwrap_or(false);
        let has_tools = r["tools"].as_array().map(|t| !t.is_empty()).unwrap_or(false);
        !is_stream && !has_tools
    }).unwrap_or(false);

    if can_use_cache {
        let cache_key = format!("{}:{}", path, body);
        let state_guard = state.lock().unwrap();
        if let Some((cached_response, cached_at)) = state_guard.response_cache.get(&cache_key) {
            if cached_at.elapsed() < std::time::Duration::from_secs(300) {
                rjlog!("[PROXY] Cache hit for key: {} chars", cache_key.len());
                return ProxyResponse::Sync(StatusCode(200), cached_response.clone());
            }
        }
        drop(state_guard);
    }

    // Load models from config file (updated when models are saved)
    let models = ModelConfig::load().models;
    
    rjlog!("[PROXY] Using {} models from config file", models.len());
    for m in &models {
        rjlog!("[PROXY] Model: {} (id={}, support_tools={})", m.name, m.id, m.support_tools);
    }

    // Identify the calling agent from the request path so we can prefer the
    // model ids actually assigned to that agent (names may collide).
    let agent = if path == "/responses" || path == "/v1/responses" || path.ends_with("/v1/responses") {
        "codex-cli"
    } else if path == "/v1/messages" || path.ends_with("/v1/messages") || path.contains("/anthropic/v1/messages") {
        "claude-code"
    } else if (path.contains("/v1/") || path.contains("/v1beta/")) && (path.contains(":generateContent") || path.contains("/models/")) {
        "gemini-cli"
    } else {
        ""
    };
    let preferred_ids: Option<Vec<String>> = if agent.is_empty() {
        None
    } else {
        state.lock().unwrap().agent_models.get(agent).cloned()
    };
    let preferred_ref = preferred_ids.as_deref();
    
    let reasoning_disabled = state.lock().unwrap().reasoning_disabled;

    // Route to appropriate proxy handler and capture response
    let response = if path == "/v1/messages" || path.ends_with("/v1/messages") || path.contains("/anthropic/v1/messages") {
        proxy_anthropic_to_openai(&body, &models, preferred_ref, reasoning_disabled)
    } else if path == "/v1/chat/completions" || path.ends_with("/v1/chat/completions") {
        proxy_openai_direct(&body, &models, preferred_ref, reasoning_disabled)
    } else if path == "/responses" || path == "/v1/responses" || path.ends_with("/v1/responses") {
        proxy_responses_to_openai(&body, &models, preferred_ref, reasoning_disabled)
    } else if path.contains("/v1/") || path.contains("/v1beta/") && (path.contains(":generateContent") || path.contains("/models/")) {
        proxy_gemini_to_openai(&body, &models, &path, preferred_ref, reasoning_disabled)
    } else {
        ProxyResponse::Sync(StatusCode(404), "Not found".to_string())
    };

    // Store response in cache if conditions are met
    if can_use_cache {
        if let ProxyResponse::Sync(response_status, ref response_body) = response {
            if response_status == StatusCode(200) {
                let cache_key = format!("{}:{}", path, body);
                let mut state_guard = state.lock().unwrap();
                state_guard.response_cache.insert(
                    cache_key,
                    (response_body.clone(), std::time::Instant::now()),
                );
                // Clean up expired entries (older than 5 minutes)
                let now = std::time::Instant::now();
                state_guard.response_cache.retain(|_, (_, t)| now.duration_since(*t) < std::time::Duration::from_secs(300));
                rjlog!("[PROXY] Cache stored, total cached entries: {}", state_guard.response_cache.len());
            }
        }
    }

    response
}

fn proxy_anthropic_to_openai(body: &str, models: &[ModelEntry], preferred_ids: Option<&[String]>, reasoning_disabled: bool) -> ProxyResponse {
    let req: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return ProxyResponse::Sync(StatusCode(400), format!("Invalid JSON: {}", e)) };

    // Extract messages and model
    let model_name = req["model"].as_str().unwrap_or("claude-3-5-sonnet");
    let messages = &req["messages"];
    
    // Debug: check raw request structure
    let has_system_field = req.get("system").is_some();
    let system_field_type = if has_system_field {
        if req["system"].is_string() { "string" }
        else if req["system"].is_array() { "array" }
        else { "other" }
    } else { "none" };
    rjlog!("[CACHE DEBUG] Raw request: system_field={}, system_type={}", has_system_field, system_field_type);
    
    // Debug: check messages structure
    if let Some(msgs) = messages.as_array() {
        if let Some(first_msg) = msgs.first() {
            let first_role = first_msg["role"].as_str().unwrap_or("unknown");
            let content_type = if first_msg["content"].is_string() { "string" }
                else if first_msg["content"].is_array() { "array" }
                else { "other/none" };
            let content_len = first_msg["content"].as_str().map(|s| s.len()).unwrap_or(0);
            rjlog!("[CACHE DEBUG] First message: role={}, content_type={}, content_len={}", first_role, content_type, content_len);
            
            // Log first 100 chars of system message if it exists
            if first_role == "system" {
                if let Some(content) = first_msg["content"].as_str() {
                    rjlog!("[CACHE DEBUG] System content (first 150): {:?}", safe_truncate(content, 150));
                } else if let Some(arr) = first_msg["content"].as_array() {
                    rjlog!("[CACHE DEBUG] System content is array with {} blocks", arr.len());
                    for (i, block) in arr.iter().enumerate().take(3) {
                        let block_type = block["type"].as_str().unwrap_or("unknown");
                        if let Some(text) = block["text"].as_str() {
                            rjlog!("[CACHE DEBUG] System block {}: type={}, text (first 80): {:?}", i, block_type, safe_truncate(text, 80));
                        } else {
                            rjlog!("[CACHE DEBUG] System block {}: type={}", i, block_type);
                        }
                    }
                }
            }
        }
    }
    
    let mut system = req["system"].as_str().map(|s| s.to_string());
    
    // Handle array-type system prompt (multiple system messages)
    if system.is_none() && req["system"].is_array() {
        rjlog!("[CACHE DEBUG] system field is array, extracting...");
        if let Some(system_arr) = req["system"].as_array() {
            let mut system_texts: Vec<String> = vec![];
            for item in system_arr {
                if let Some(text) = item.as_str() {
                    system_texts.push(text.to_string());
                } else if let Some(obj) = item.as_object() {
                    if let Some(content) = obj.get("content").and_then(|c| c.as_str()) {
                        system_texts.push(content.to_string());
                    } else if let Some(content_arr) = obj.get("content").and_then(|c| c.as_array()) {
                        for block in content_arr {
                            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                    system_texts.push(text.to_string());
                                }
                            }
                        }
                    }
                }
            }
            if !system_texts.is_empty() {
                system = Some(system_texts.join("\n"));
                rjlog!("[CACHE DEBUG] Extracted system prompt from array ({} items)", system_texts.len());
            }
        }
    }
    
    // Also check if there's a system message in the messages array (newer API format)
    if system.is_none() {
        rjlog!("[CACHE DEBUG] No system field, searching messages array for system message...");
        if let Some(msgs) = messages.as_array() {
            // Search all messages for system role (not just the first one)
            for msg in msgs {
                if msg["role"].as_str() == Some("system") {
                    rjlog!("[CACHE DEBUG] Found system message in array, extracting...");
                    if let Some(content) = msg["content"].as_str() {
                        system = Some(content.to_string());
                        rjlog!("[CACHE DEBUG] Extracted system prompt from messages array (string)");
                        break;
                    } else if let Some(content_arr) = msg["content"].as_array() {
                        let text_parts: Vec<String> = content_arr.iter()
                            .filter_map(|block| {
                                if block["type"].as_str() == Some("text") {
                                    block["text"].as_str().map(|s| s.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if !text_parts.is_empty() {
                            system = Some(text_parts.join("\n"));
                            rjlog!("[CACHE DEBUG] Extracted system prompt from messages array ({} text blocks)", text_parts.len());
                            break;
                        }
                    }
                }
            }
            if system.is_none() {
                rjlog!("[CACHE DEBUG] No system message found in messages array");
            }
        }
    }
    
    let is_llama_cpp_check = req["model"].as_str().map(|m| m.contains("llama-") || m.ends_with(".gguf")).unwrap_or(false);
    let max_tokens = req["max_tokens"].as_u64().unwrap_or(if is_llama_cpp_check { 2048 } else { 4096 });
    let stream = req["stream"].as_bool().unwrap_or(false);
    rjlog!("[PROXY] Anthropic→OpenAI stream={} max_tokens={} is_llama={}", stream, max_tokens, is_llama_cpp_check);

    // Find matching model in our config
    let target = find_model(models, model_name, preferred_ids);

    let (api_key, base_url, real_model, support_tools, provider) = if let Some(m) = target {
        (m.api_key.clone(), m.api_base.clone(), m.name.clone(), m.support_tools, m.provider.clone())
    } else {
        // No match — try to forward as-is to Anthropic
        let (s, b) = forward_to_anthropic(body);
        return ProxyResponse::Sync(s, b);
    };

    // Log model resolution for debugging
    rjlog!("[CACHE DEBUG] Model resolution: requested={}, real_model={}, provider={}, base_url={}", 
        model_name, real_model, provider, base_url);
    
    // Log system prompt prefix for comparison
    if let Some(ref sys) = system {
        rjlog!("[CACHE DEBUG] System prompt (first 100 chars): {:?}", safe_truncate(sys, 100));
    }

    // Build OpenAI-format request
    let mut openai_messages: Vec<Value> = vec![];
    let is_llama_cpp = real_model.contains("llama-") || real_model.ends_with(".gguf");
    if let Some(ref sys) = system {
        openai_messages.push(serde_json::json!({"role": "system", "content": sys}));
    } else if !is_llama_cpp {
        openai_messages.push(serde_json::json!({"role": "system", "content": "You are a helpful assistant."}));
    }
    rjlog!("[PROXY] system present: {}, is_llama={}", system.is_some(), is_llama_cpp);
    if let Some(msgs) = messages.as_array() {
        for m in msgs {
            let role = m["role"].as_str().unwrap_or("user");
            
            // Skip system messages if we already extracted the system prompt
            if role == "system" && system.is_some() {
                rjlog!("[CACHE DEBUG] Skipping system message in array (already extracted as system prompt)");
                continue;
            }
            
            if m["content"].is_string() {
                openai_messages.push(serde_json::json!({"role": role, "content": m["content"]}));
                continue;
            }
            let Some(blocks) = m["content"].as_array() else {
                openai_messages.push(serde_json::json!({"role": role, "content": m["content"]}));
                continue;
            };

            match role {
                "user" => {
                    // User messages may contain text blocks and/or tool_result blocks.
                    let mut text_parts: Vec<&str> = vec![];
                    for block in blocks {
                        match block["type"].as_str() {
                            Some("text") => {
                                if let Some(t) = block["text"].as_str() { text_parts.push(t); }
                            }
                            Some("tool_result") => {
                                let tool_use_id = block["tool_use_id"].as_str().unwrap_or("");
                                let result_content = block["content"].as_str().map(|s| s.to_string())
                                    .or_else(|| {
                                        block["content"].as_array().map(|arr| {
                                            arr.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")
                                        })
                                    }).unwrap_or_default();
                                openai_messages.push(serde_json::json!({
                                    "role": "tool",
                                    "tool_call_id": tool_use_id,
                                    "content": result_content
                                }));
                            }
                            _ => {} // skip thinking blocks
                        }
                    }
                    let user_text = text_parts.join("");
                    if !user_text.is_empty() {
                        openai_messages.push(serde_json::json!({"role": "user", "content": user_text}));
                    }
                }
                "assistant" => {
                    // Assistant messages may contain text and/or tool_use blocks.
                    let mut text_parts: Vec<&str> = vec![];
                    let mut tool_calls: Vec<Value> = vec![];
                    for block in blocks {
                        match block["type"].as_str() {
                            Some("text") => {
                                if let Some(t) = block["text"].as_str() { text_parts.push(t); }
                            }
                            Some("tool_use") => {
                                let id = block["id"].as_str().unwrap_or("");
                                let name = block["name"].as_str().unwrap_or("");
                                let arguments = block["input"].to_string();
                                tool_calls.push(serde_json::json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {"name": name, "arguments": arguments}
                                }));
                            }
                            _ => {} // skip thinking blocks
                        }
                    }
                    let content = text_parts.join("");
                    if content.is_empty() && tool_calls.is_empty() { continue; }
                    let mut msg = serde_json::json!({"role": "assistant", "content": content});
                    if !tool_calls.is_empty() {
                        msg["tool_calls"] = serde_json::json!(tool_calls);
                    }
                    openai_messages.push(msg);
                }
                _ => {
                    let text = blocks.iter()
                        .filter_map(|b| if b["type"].as_str() == Some("text") { b["text"].as_str() } else { None })
                        .collect::<Vec<_>>().join("");
                    if !text.is_empty() {
                        openai_messages.push(serde_json::json!({"role": role, "content": text}));
                    }
                }
            }
        }
    }

    let is_llama_cpp = real_model.contains("llama-") || real_model.ends_with(".gguf");

    // Convert Anthropic tools format → OpenAI tools format
    let mut openai_tools: Vec<Value> = vec![];
    if let Some(tools) = req.get("tools").and_then(|v| v.as_array()) {
        for tool in tools {
            let name = tool["name"].as_str().unwrap_or("");
            let description = tool["description"].as_str().unwrap_or("");

            let parameters = if is_llama_cpp {
                // Simplified schema for llama.cpp - complex schemas cause GBNF parse failures
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "description": description
                })
            } else {
                tool["input_schema"].clone()
            };

            openai_tools.push(serde_json::json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": description,
                    "parameters": parameters
                }
            }));
        }
    }

    // Sort tools by name for deterministic serialization to improve cache hit rate
    openai_tools.sort_by(|a, b| {
        a["function"]["name"].as_str()
            .unwrap_or("")
            .cmp(&b["function"]["name"].as_str().unwrap_or(""))
    });

    let mut openai_body = serde_json::json!({
        "model": real_model,
        "messages": openai_messages,
        "max_tokens": max_tokens,
        "stream": stream,
    });
    
    // Log outbound request details for cache debugging
    rjlog!("[CACHE DEBUG] Outbound request: model={}, messages={}, tools={}, stream={}", 
        real_model, openai_messages.len(), openai_tools.len(), stream);
    if let Some(sys_msg) = openai_messages.first() {
        if sys_msg["role"].as_str() == Some("system") {
            let sys_content = sys_msg["content"].as_str().unwrap_or("");
            rjlog!("[CACHE DEBUG] Outbound system prompt (first 100): {:?}", safe_truncate(sys_content, 100));
        }
    }
    if !openai_tools.is_empty() && support_tools {
        // llama.cpp has limits on tool count for GBNF grammar parsing
        let limited_tools = if is_llama_cpp && openai_tools.len() > 20 {
            rjlog!("[PROXY] Anthropic→OpenAI: limiting tools from {} to 20 for llama.cpp", openai_tools.len());
            openai_tools[..20].to_vec()
        } else {
            openai_tools.clone()
        };
        openai_body["tools"] = serde_json::json!(limited_tools);
        if let Some(tc) = req.get("tool_choice") {
            let converted_tc = convert_tool_choice_anthropic_to_openai(tc);
            rjlog!("[PROXY] Anthropic→OpenAI: tool_choice raw={:?}, converted={:?}", tc, converted_tc);
            openai_body["tool_choice"] = converted_tc;
        }
        rjlog!("[PROXY] Anthropic→OpenAI: tools enabled for model={} is_llama={} count={}", real_model, is_llama_cpp, openai_tools.len());
    }

    if reasoning_disabled {
        openai_body["thinking"] = serde_json::json!({"type": "disabled"});
        openai_body["reasoning_effort"] = serde_json::json!("low");
        openai_body["enable_thinking"] = serde_json::json!(false);
        openai_body["temperature"] = serde_json::json!(0.6);
    }

    // Forward to OpenAI-compatible endpoint
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let request_body_str = openai_body.to_string();
    let body_preview: String = safe_truncate(&request_body_str, 500).to_string();
    rjlog!("[PROXY] Anthropic→OpenAI: POST {} model={} stream={} msgs={} max_tokens={} body_len={}", 
        url, real_model, stream, openai_messages.len(), max_tokens, request_body_str.len());
    rjlog!("[PROXY] Request body preview: {}...", body_preview);

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .build();
    let request_start = std::time::Instant::now();
    let resp = agent.post(&url)
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_string(&request_body_str);
    rjlog!("[PROXY] Request completed in {:?}", request_start.elapsed());

    match resp {
        Ok(response) => {
            if stream {
                let reader = response.into_reader();
                let buf_reader = BufReader::new(Box::new(reader) as Box<dyn Read + Send>);
                let converter = SseStreamConverter::new(
                    buf_reader,
                    Box::new(make_anthropic_sse_converter(model_name.to_string())),
                );
                rjlog!("[PROXY] Anthropic→OpenAI: returning streaming response");
                ProxyResponse::Stream { reader: Box::new(converter) }
            } else {
                let resp_body = response.into_string().unwrap_or_default();
                // Log upstream response model for cache debugging
                if let Ok(resp_json) = serde_json::from_str::<Value>(&resp_body) {
                    let upstream_model = resp_json.get("model").and_then(|v| v.as_str()).unwrap_or("unknown");
                    rjlog!("[CACHE DEBUG] Upstream response model: {} (requested: {})", upstream_model, real_model);
                    if let Some(usage) = resp_json.get("usage") {
                        rjlog!("[CACHE DEBUG] Upstream usage: {}", serde_json::to_string(usage).unwrap_or_default());
                    }
                }
                let converted = convert_openai_to_anthropic(&resp_body, model_name);
                ProxyResponse::Sync(StatusCode(200), converted)
            }
        }
        Err(ureq::Error::Status(st, r)) => {
            let body = r.into_string().unwrap_or_default();
            rjlog!("[PROXY] Anthropic→OpenAI: upstream HTTP {}: {}", st, safe_truncate(&body, 500));
            let err_body = serde_json::json!({
                "type": "error",
                "error": {"type": "api_error", "message": format!("Upstream {}: {}", st, safe_truncate(&body, 200))}
            });
            ProxyResponse::Sync(StatusCode(502), err_body.to_string())
        }
        Err(e) => {
            rjlog!("[PROXY] Anthropic→OpenAI: connection error: {:?}", e);
            let error_msg = if real_model.contains("llama-") || real_model.ends_with(".gguf") {
                "Llama.cpp server is not running. Please start the server in the Models settings page first.".to_string()
            } else {
                "Proxy connection error".to_string()
            };
            let err_body = serde_json::json!({
                "error": {
                    "code": 503,
                    "message": error_msg,
                    "type": "service_unavailable"
                }
            });
            ProxyResponse::Sync(StatusCode(503), err_body.to_string())
        }
    }
}

/// Convert Anthropic-format `tool_choice` to OpenAI-format.
///
/// Anthropic uses: "auto", "any", or { type: "auto"|"any"|"tool", name?: "..." }
/// OpenAI uses: "auto", "none", "required", or { type: "function", function: { name: "..." } }
///
/// DeepSeek (and some other providers) reject objects like { type: "auto" }
/// because they only parse { type: "function", ... } or plain strings.
fn convert_tool_choice_anthropic_to_openai(tc: &Value) -> Value {
    // String form: "auto" | "any" | "tool"
    if let Some(s) = tc.as_str() {
        return match s {
            "auto" => Value::String("auto".into()),
            "any" => Value::String("required".into()),
            _ => tc.clone(), // pass through
        };
    }
    // Object form: { type: "auto"|"any"|"tool", name?: "..." }
    if let Some(obj) = tc.as_object() {
        let tc_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match tc_type {
            "auto" => return Value::String("auto".into()),
            "any" => return Value::String("required".into()),
            "tool" => {
                if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
                    return serde_json::json!({
                        "type": "function",
                        "function": { "name": name }
                    });
                }
            }
            _ => {}
        }
    }
    // Fallback: pass through as-is
    tc.clone()
}

fn proxy_openai_direct(body: &str, _models: &[ModelEntry], preferred_ids: Option<&[String]>, reasoning_disabled: bool) -> ProxyResponse {
    // For OpenAI requests, forward directly to the target
    let req: Value = match serde_json::from_str(body) { Ok(v) => v, Err(_) => return ProxyResponse::Sync(StatusCode(400), "Invalid JSON".into()) };
    let model_name = req["model"].as_str().unwrap_or("gpt-4o");
    let stream = req["stream"].as_bool().unwrap_or(false);
    let is_llama_cpp = model_name.contains("llama-") || model_name.ends_with(".gguf");
    rjlog!("[PROXY] OpenAI direct stream={} is_llama={}", stream, is_llama_cpp);

    // Find matching model config
    let models = ModelConfig::load().models;
    let target = find_model(&models, model_name, preferred_ids);

    if let Some(m) = target {
        let url = format!("{}/chat/completions", m.api_base.trim_end_matches('/'));
        
        let mut req_body: Value = serde_json::from_str(body).unwrap_or_default();
        
        if is_llama_cpp {
            let current_max = req_body["max_tokens"].as_u64().unwrap_or(4096);
            if current_max > 2048 {
                req_body["max_tokens"] = serde_json::json!(2048);
                rjlog!("[PROXY] llama_cpp: reduced max_tokens from {} to 2048", current_max);
            }
        }
        
        if reasoning_disabled {
            req_body["thinking"] = serde_json::json!({"type": "disabled"});
            req_body["reasoning_effort"] = serde_json::json!("low");
            req_body["enable_thinking"] = serde_json::json!(false);
            req_body["temperature"] = serde_json::json!(0.6);
            rjlog!("[PROXY] reasoning_disabled=true, modified body: enable_thinking=false, temperature=0.6");
        }
        let modified_body = req_body.to_string();
        rjlog!("[PROXY] Forwarding to {} with stream={} body_len={}", url, stream, modified_body.len());
        
        let request_start = std::time::Instant::now();
        let resp = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", m.api_key))
            .set("Content-Type", "application/json")
            .send_string(&modified_body);
        rjlog!("[PROXY] ureq request completed in {:?}", request_start.elapsed());
        match resp {
            Ok(r) => {
                if stream {
                    let reader = r.into_reader();
                    rjlog!("[PROXY] got reader in {:?}", request_start.elapsed());
                    let buf_reader = BufReader::new(Box::new(reader) as Box<dyn Read + Send>);
                    
                    let model_name_clone = model_name.to_string();
                    let converter = SseStreamConverter::new(
                        buf_reader,
                        if is_llama_cpp {
                            Box::new(make_llama_cpp_sse_converter(model_name_clone))
                        } else {
                            Box::new(make_anthropic_sse_converter(model_name_clone))
                        },
                    );
                    rjlog!("[PROXY] OpenAI direct: returning streaming response is_llama={} in {:?}", is_llama_cpp, request_start.elapsed());
                    ProxyResponse::Stream { reader: Box::new(converter) }
                } else {
                    ProxyResponse::Sync(StatusCode(200), r.into_string().unwrap_or_default())
                }
            }
            Err(e) => ProxyResponse::Sync(StatusCode(502), format!("Proxy error: {}", e)),
        }
    } else {
        ProxyResponse::Sync(StatusCode(404), format!("Model {} not configured", model_name))
    }
}

/// Translate OpenAI Responses API → OpenAI Chat Completions.
/// Codex uses the Responses API (/responses), but most providers (DeepSeek, etc.)
/// only support Chat Completions (/v1/chat/completions).
fn proxy_responses_to_openai(body: &str, models: &[ModelEntry], preferred_ids: Option<&[String]>, reasoning_disabled: bool) -> ProxyResponse {
    let req: Value = match serde_json::from_str(body) { Ok(v) => v, Err(_) => return ProxyResponse::Sync(StatusCode(400), "Invalid JSON".into()) };
    let model_name = req["model"].as_str().unwrap_or("");
    let stream = req["stream"].as_bool().unwrap_or(false);
    rjlog!("[PROXY] Responses→Chat stream={}", stream);

    // Convert Responses API `input` → Chat Completions `messages`
    let messages = if let Some(input) = req.get("input") {
        if let Some(arr) = input.as_array() {
            let mut msgs: Vec<Value> = vec![];
            for item in arr {
                let item_type = item["type"].as_str().unwrap_or("");

                match item_type {
                    "function_call" => {
                        let call_id = item["call_id"].as_str().unwrap_or("");
                        let name = item["name"].as_str().unwrap_or("");
                        let arguments = item["arguments"].as_str().unwrap_or("");
                        msgs.push(serde_json::json!({
                            "role": "assistant",
                            // 上游（DeepSeek 等）校验 content 必须是 string 或 list，
                            // null 会被拒绝（"content should be a string or a list"）。
                            "content": "",
                            "tool_calls": [{
                                "id": call_id,
                                "type": "function",
                                "function": {"name": name, "arguments": arguments}
                            }]
                        }));
                    }
                    "function_call_output" => {
                        let call_id = item["call_id"].as_str().unwrap_or("");
                        // output 可能是字符串或对象（工具 JSON 结果）——统一转字符串
                        let output = match item.get("output") {
                            Some(v) if v.is_string() => v.as_str().unwrap_or("").to_string(),
                            Some(v) => v.to_string(),
                            None => String::new(),
                        };
                        msgs.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": output
                        }));
                    }
                    "reasoning" => {
                        // codex 回传的推理项：无 role，且上游模型自己会推理，
                        // 转成消息只会产生空的 user 消息导致上游校验失败。跳过。
                    }
                    _ => {
                        // Regular message items (message, developer, etc.)
                        let role = match item["role"].as_str().unwrap_or("user") {
                            "developer" => "system",
                            r => r,
                        };
                        let content = if let Some(c) = item.get("content") {
                            if let Some(s) = c.as_str() {
                                Value::String(s.to_string())
                            } else if let Some(arr) = c.as_array() {
                                // 提取所有文本 part（text / output_text / input_text…）
                                let text = arr.iter()
                                    .filter_map(|p| p["text"].as_str())
                                    .collect::<Vec<_>>()
                                    .join("");
                                Value::String(text)
                            } else if c.is_null() {
                                // content: null → 空字符串（上游要求 string/list）
                                Value::String("".into())
                            } else {
                                // 对象等非标准结构——尝试取 text，绝不把对象塞进 content
                                let text = c.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                Value::String(text.to_string())
                            }
                        } else {
                            Value::String("".into())
                        };
                        msgs.push(serde_json::json!({"role": role, "content": content}));
                    }
                }
            }
            msgs
        } else {
            vec![]
        }
    } else if let Some(msgs) = req.get("messages").and_then(|v| v.as_array()) {
        msgs.clone()
    } else {
        vec![]
    };

    if model_name.is_empty() || messages.is_empty() {
        return ProxyResponse::Sync(StatusCode(400), r#"{"error":"Missing model or input"}"#.into());
    }

    // Find matching model in config
    let target = find_model(models, model_name, preferred_ids);
    let (api_key, base_url, real_model, support_tools, provider) = if let Some(m) = target {
        let masked_key = if m.api_key.len() > 8 {
            format!("{}...{}", &m.api_key[..4], &m.api_key[m.api_key.len()-4..])
        } else { m.api_key.clone() };
        rjlog!("[PROXY] Responses→Chat: Found model '{}' api_key={} base_url={}", m.name, masked_key, m.api_base);
        (m.api_key.clone(), m.api_base.clone(), m.name.clone(), m.support_tools, m.provider.clone())
    } else {
        rjlog!("[PROXY] Responses→Chat: Model '{}' NOT FOUND in {} models. Available: {:?}",
            model_name, models.len(),
            models.iter().map(|m| format!("{}({})", m.name, m.id)).collect::<Vec<_>>());
        return ProxyResponse::Sync(StatusCode(404), format!(r#"{{"error":"Model {} not configured"}}"#, model_name));
    };

    // Build Chat Completions request (convert Responses API tool format to Chat format)
    let mut chat_body = serde_json::json!({
        "model": real_model,
        "messages": messages,
        "stream": stream,
    });
    if let Some(tools) = req.get("tools").and_then(|v| v.as_array()) {
        let mut chat_tools: Vec<Value> = tools.iter()
            .filter(|t| t["type"].as_str() == Some("function"))
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t["name"],
                        "description": t.get("description").unwrap_or(&Value::Null),
                        "parameters": t.get("parameters").unwrap_or(&Value::Null),
                    }
                })
            })
            .collect();

        // Sort tools by name for deterministic serialization to improve cache hit rate
        chat_tools.sort_by(|a, b| {
            a["function"]["name"].as_str()
                .unwrap_or("")
                .cmp(&b["function"]["name"].as_str().unwrap_or(""))
        });

        if support_tools {
            chat_body["tools"] = serde_json::Value::Array(chat_tools);
            if let Some(tc) = req.get("tool_choice") {
                chat_body["tool_choice"] = tc.clone();
            }
        } else {
            rjlog!("[PROXY] Responses→Chat: model {} does not support tools, skipping tool definitions", real_model);
        }
    }

    if reasoning_disabled {
        chat_body["thinking"] = serde_json::json!({"type": "disabled"});
        chat_body["reasoning_effort"] = serde_json::json!("low");
        chat_body["enable_thinking"] = serde_json::json!(false);
        chat_body["temperature"] = serde_json::json!(0.6);
    }

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    rjlog!("[PROXY] Responses→Chat: POST {} (model={}, stream={})", url, real_model, stream);

    let request_body = chat_body.to_string();
    rjlog!("[PROXY] Responses→Chat: body ({} chars) — model:{} messages:{}",
        request_body.len(), real_model, messages.len());

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .build();
    let resp = agent.post(&url)
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_string(&request_body);

    match resp {
        Ok(r) => {
            let status = r.status();
            rjlog!("[PROXY] Upstream response status: {} (stream={})", status, stream);
            if stream {
                let reader = r.into_reader();
                let buf_reader = BufReader::new(Box::new(reader) as Box<dyn Read + Send>);
                let converter = SseStreamConverter::new(
                    buf_reader,
                    Box::new(make_responses_sse_converter(real_model)),
                );
                rjlog!("[PROXY] Responses→Chat: returning streaming response");
                ProxyResponse::Stream { reader: Box::new(converter) }
            } else {
                match r.into_string() {
                    Ok(resp_body) => {
                        if status >= 400 {
                            return ProxyResponse::Sync(StatusCode(status), resp_body);
                        }
                        let converted = convert_chat_to_responses(&resp_body, &real_model, false);
                        ProxyResponse::Sync(StatusCode(200), converted)
                    }
                    Err(e) => {
                        ProxyResponse::Sync(StatusCode(502), format!(r#"{{"error":"Failed to read response: {}"}}"#, e))
                    }
                }
            }
        }
        Err(ureq::Error::Status(status, r)) => {
            let body = r.into_string().unwrap_or_default();
            rjlog!("[PROXY] Upstream HTTP {}: {}", status, safe_truncate(&body, 1000));
            ProxyResponse::Sync(StatusCode(502), format!(r#"{{"error":"Upstream {}: {}"}}"#, status, body))
        }
        Err(e) => {
            rjlog!("[PROXY] Connection error: {:?}", e);
            ProxyResponse::Sync(StatusCode(502), format!(r#"{{"error":"Proxy error: {}"}}"#, e))
        }
    }
}

/// Convert Chat Completions response to Responses API format.
/// Separates reasoning_content into a separate reasoning output item (matching
/// the OpenAI Responses API output format that Codex ACP client expects).
fn convert_chat_to_responses(chat_resp: &str, model: &str, _stream: bool) -> String {
    let resp: Value = match serde_json::from_str(chat_resp) { Ok(v) => v, Err(_) => return chat_resp.to_string() };
    let choice = &resp["choices"][0];
    let reasoning_content = choice["message"].get("reasoning_content").and_then(|v| v.as_str()).unwrap_or("");
    let assistant_content = choice["message"]["content"].as_str().unwrap_or("");
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("stop");
    let input_tokens = resp["usage"]["prompt_tokens"].as_u64()
        .or_else(|| resp["usage"]["input_tokens"].as_u64())
        .or_else(|| resp["usage"]["inputTokens"].as_u64())
        .unwrap_or(0);
    let output_tokens = resp["usage"]["completion_tokens"].as_u64()
        .or_else(|| resp["usage"]["output_tokens"].as_u64())
        .or_else(|| resp["usage"]["outputTokens"].as_u64())
        .unwrap_or(0);
    let total_tokens = resp["usage"]["total_tokens"].as_u64()
        .or_else(|| resp["usage"]["totalTokens"].as_u64())
        .unwrap_or(input_tokens + output_tokens);
    let cached_tokens = resp["usage"]["cached_tokens"].as_u64()
        .or_else(|| resp["usage"]["cache_hit_tokens"].as_u64())
        .or_else(|| resp["usage"]["cachedReadTokens"].as_u64())
        .or_else(|| resp["usage"]["cached_content_tokens"].as_u64())
        .unwrap_or(0);
    rjlog!("[PROXY USAGE] Non-stream: input={}, output={}, cached={}, raw_usage={}", 
        input_tokens, output_tokens, cached_tokens,
        serde_json::to_string(&resp["usage"]).unwrap_or_default());

    let mut output_items: Vec<Value> = vec![];

    // If the model returned reasoning_content, emit it as a separate reasoning item.
    if !reasoning_content.is_empty() {
        rjlog!("[PROXY] Non-stream: adding reasoning output_item ({} chars)", reasoning_content.len());
        output_items.push(serde_json::json!({
            "type": "reasoning",
            "id": format!("rs_{}", chrono::Utc::now().timestamp_millis()),
            "status": "completed",
            "role": "assistant",
            "summary": [{"type": "summary_text", "text": reasoning_content}],
        }));
    }

    // Tool calls from the upstream model
    if let Some(tool_calls) = choice["message"].get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tool_calls {
            let call_id = tc["id"].as_str().unwrap_or("");
            let name = tc["function"]["name"].as_str().unwrap_or("");
            let arguments = tc["function"]["arguments"].as_str().unwrap_or("");
            output_items.push(serde_json::json!({
                "type": "function_call",
                "id": format!("fc_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)),
                "call_id": call_id,
                "name": name,
                "arguments": arguments,
                "status": "completed",
            }));
        }
    }

    if !assistant_content.is_empty() {
        output_items.push(serde_json::json!({
            "type": "message",
            "id": format!("msg_{}", chrono::Utc::now().timestamp_millis()),
            "status": finish_reason,
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": assistant_content,
            }]
        }));
    }

    serde_json::json!({
        "id": format!("resp_{}", chrono::Utc::now().timestamp_millis()),
        "object": "response",
        "model": model,
        "output": output_items,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "total_tokens": total_tokens,
            "cached_tokens": cached_tokens,
        }
    }).to_string()
}

/// Returns a closure that converts OpenAI Chat Completions SSE lines
/// → OpenAI Responses API SSE format, line by line (for streaming).
///
/// Codex expects the full Responses streaming event chain:
///   response.created
///   response.output_item.added  ← activates the output item
///   response.content_part.added ← activates the text part
///   response.output_text.delta  ← actual token deltas (0..N)
///   response.reasoning_text.delta ← thinking deltas
///   response.function_call_arguments.delta ← tool call args
///   response.content_part.done
///   response.output_item.done
///   response.completed
fn make_responses_sse_converter(model: String) -> impl FnMut(&str) -> Vec<u8> {
    use std::collections::HashMap;
    let response_id = format!("resp_{}", chrono::Utc::now().timestamp_millis());
    let reasoning_id = format!("rs_{}", chrono::Utc::now().timestamp_millis());
    let item_id = format!("msg_{}", chrono::Utc::now().timestamp_millis());
    let part_id = format!("part_{}", chrono::Utc::now().timestamp_millis());
    let mut started = false;
    let mut reasoning_started = false;
    let mut reasoning_done = false;
    let mut msg_started = false;
    let mut full_reasoning = String::new();
    let mut full_text = String::new();
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut total_tokens: u64 = 0;
    let mut cached_tokens: u64 = 0;
    let mut tool_items: HashMap<usize, (String, String, String, String)> = HashMap::new();
    let mut tool_output_indices: HashMap<usize, usize> = HashMap::new();
    let mut chunk_counter: u64 = 0;

    move |line: &str| -> Vec<u8> {
        let mut result = Vec::new();

        let data = match line.strip_prefix("data: ") {
            Some(d) => d,
            None => return result,
        };

        // Log raw SSE data for debugging
        chunk_counter += 1;
        if chunk_counter <= 3 || data == "[DONE]" || data.contains("usage") {
            rjlog!("[CACHE DEBUG] SSE chunk #{}: data={}", chunk_counter, safe_truncate(data, 300));
        }

        // Helper: close reasoning item inline
        let mut close_reasoning = |result: &mut Vec<u8>,
                                   started: &mut bool,
                                   done: &mut bool,
                                   reasoning_id: &str,
                                   full_reasoning: &str|
        {
            if *started && !*done {
                *done = true;
                let _ = write!(result, "event: response.reasoning_text.done\ndata: {}\n\n", serde_json::json!({
                    "type": "response.reasoning_text.done", "item_id": reasoning_id, "output_index": 0, "content_index": 0, "text": full_reasoning
                }));
                let _ = write!(result, "event: response.output_item.done\ndata: {}\n\n", serde_json::json!({
                    "type": "response.output_item.done", "output_index": 0, "item": {
                        "id": reasoning_id, "object": "realtime.item", "type": "reasoning", "status": "completed",
                        "role": "assistant", "summary": [{"type": "summary_text", "text": full_reasoning}]
                    }
                }));
            }
        };

        if data == "[DONE]" {
            close_reasoning(&mut result, &mut reasoning_started, &mut reasoning_done, &reasoning_id, &full_reasoning);

            for (_, (fc_id, _, _, args)) in tool_items.drain() {
                let oi = 1;
                let _ = write!(result, "event: response.function_call_arguments.done\ndata: {}\n\n", serde_json::json!({
                    "type": "response.function_call_arguments.done", "output_index": oi, "item_id": fc_id
                }));
                let _ = write!(result, "event: response.output_item.done\ndata: {}\n\n", serde_json::json!({
                    "type": "response.output_item.done", "output_index": oi, "item": {
                        "id": fc_id, "object": "realtime.item", "type": "function_call", "status": "completed",
                        "call_id": "", "name": "", "arguments": args
                    }
                }));
            }

            let msg_idx = if reasoning_done { 1 } else { 0 };
            if msg_started {
                let _ = write!(result, "event: response.content_part.done\ndata: {}\n\n", serde_json::json!({
                    "type": "response.content_part.done", "output_index": msg_idx, "content_index": 0, "item_id": item_id,
                    "part": {"id": part_id, "object": "realtime.item", "type": "output_text", "text": full_text}
                }));
                let _ = write!(result, "event: response.output_item.done\ndata: {}\n\n", serde_json::json!({
                    "type": "response.output_item.done", "output_index": msg_idx, "item": {
                        "id": item_id, "object": "realtime.item", "type": "message", "status": "completed",
                        "role": "assistant", "content": [{"type": "output_text", "text": full_text}]
                    }
                }));
            }

            let mut output_items: Vec<Value> = vec![];
            if reasoning_done {
                output_items.push(serde_json::json!({"type":"reasoning","id":reasoning_id,"status":"completed","role":"assistant","summary":[{"type":"summary_text","text":full_reasoning}]}));
            }
            if msg_started || !full_text.is_empty() {
                output_items.push(serde_json::json!({"type":"message","id":item_id,"status":"completed","role":"assistant","content":[{"type":"output_text","text":full_text}]}));
            }
            let _ = write!(result, "event: response.completed\ndata: {}\n\n", serde_json::json!({
                "type": "response.completed", "response": {
                    "id": response_id, "object": "response", "model": model, "status": "completed",
                    "output": output_items,
                    "usage": {
                        "input_tokens": input_tokens,
                        "output_tokens": output_tokens,
                        "total_tokens": total_tokens,
                        "cached_tokens": cached_tokens,
                    }
                }
            }));
            // Store usage to the global store so the ACP client can attach it to the
            // finish event (codex sessions previously showed no token/cache info).
            store_usage_for_latest(model.clone(), input_tokens as i64, output_tokens as i64, cached_tokens as i64);
            return result;
        }

        if let Ok(chunk) = serde_json::from_str::<Value>(data) {
            if !started {
                started = true;
                let _ = write!(result, "event: response.created\ndata: {}\n\n", serde_json::json!({
                    "type": "response.created", "response": {
                        "id": response_id, "object": "response", "model": model, "status": "in_progress", "output": []
                    }
                }));
            }

            let choice = &chunk["choices"][0];

            // --- Reasoning / thinking ---
            if let Some(reasoning) = choice["delta"]["reasoning_content"].as_str() {
                if !reasoning_done && !msg_started {
                    if !reasoning_started {
                        reasoning_started = true;
                        let _ = write!(result, "event: response.output_item.added\ndata: {}\n\n", serde_json::json!({
                            "type": "response.output_item.added", "output_index": 0, "item": {
                                "id": reasoning_id, "object": "realtime.item", "type": "reasoning",
                                "status": "in_progress", "role": "assistant", "summary": []
                            }
                        }));
                    }
                    full_reasoning.push_str(reasoning);
                    let _ = write!(result, "event: response.reasoning_text.delta\ndata: {}\n\n", serde_json::json!({
                        "type": "response.reasoning_text.delta", "item_id": reasoning_id, "output_index": 0, "content_index": 0, "delta": reasoning
                    }));
                }
            }

            // --- Tool calls → function_call output items ---
            if let Some(tool_calls) = choice["delta"]["tool_calls"].as_array() {
                close_reasoning(&mut result, &mut reasoning_started, &mut reasoning_done, &reasoning_id, &full_reasoning);

                let base_oi = if reasoning_done { 1usize } else { 0usize };
                for tc in tool_calls {
                    let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                    let oi = base_oi + idx;

                    if let Some(id) = tc["id"].as_str() {
                        if !tool_items.contains_key(&idx) {
                            let fc_id = format!("fc_{}_{}", chrono::Utc::now().timestamp_millis(), idx);
                            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                            tool_items.insert(idx, (fc_id.clone(), id.to_string(), name.clone(), String::new()));
                            tool_output_indices.insert(idx, oi);
                            let _ = write!(result, "event: response.output_item.added\ndata: {}\n\n", serde_json::json!({
                                "type": "response.output_item.added", "output_index": oi, "item": {
                                    "id": fc_id, "object": "realtime.item", "type": "function_call",
                                    "status": "in_progress", "call_id": id, "name": name, "arguments": ""
                                }
                            }));
                        }
                    }

                    if let Some(args) = tc["function"]["arguments"].as_str() {
                        if let Some(entry) = tool_items.get_mut(&idx) {
                            entry.3.push_str(args);
                        }
                        let _ = write!(result, "event: response.function_call_arguments.delta\ndata: {}\n\n", serde_json::json!({
                            "type": "response.function_call_arguments.delta", "output_index": tool_output_indices.get(&idx).copied().unwrap_or(0), "call_id": "", "delta": args
                        }));
                    }
                }
            }

            // --- Actual content ---
            if let Some(delta_content) = choice["delta"]["content"].as_str() {
                close_reasoning(&mut result, &mut reasoning_started, &mut reasoning_done, &reasoning_id, &full_reasoning);

                if !msg_started {
                    msg_started = true;
                    let msg_idx = if reasoning_done { 1 } else { 0 };
                    let _ = write!(result, "event: response.output_item.added\ndata: {}\n\n", serde_json::json!({
                        "type": "response.output_item.added", "output_index": msg_idx, "item": {
                            "id": item_id, "object": "realtime.item", "type": "message", "status": "in_progress", "role": "assistant", "content": []
                        }
                    }));
                    let _ = write!(result, "event: response.content_part.added\ndata: {}\n\n", serde_json::json!({
                        "type": "response.content_part.added", "output_index": msg_idx, "content_index": 0, "item_id": item_id,
                        "part": {"id": part_id, "object": "realtime.item", "type": "output_text", "text": ""}
                    }));
                }
                full_text.push_str(delta_content);
                let msg_idx = if reasoning_done { 1 } else { 0 };
                let _ = write!(result, "event: response.output_text.delta\ndata: {}\n\n", serde_json::json!({
                    "type": "response.output_text.delta", "item_id": item_id, "output_index": msg_idx, "content_index": 0, "delta": delta_content
                }));
            }

            if let Some(usage) = chunk.get("usage") {
                let raw_usage = serde_json::to_string(usage).unwrap_or_default();
                rjlog!("[PROXY USAGE] Raw usage from upstream: {}", raw_usage);
                input_tokens = usage["prompt_tokens"].as_u64()
                    .or_else(|| usage["input_tokens"].as_u64())
                    .or_else(|| usage["inputTokens"].as_u64())
                    .unwrap_or(input_tokens);
                output_tokens = usage["completion_tokens"].as_u64()
                    .or_else(|| usage["output_tokens"].as_u64())
                    .or_else(|| usage["outputTokens"].as_u64())
                    .unwrap_or(output_tokens);
                total_tokens = usage["total_tokens"].as_u64()
                    .or_else(|| usage["totalTokens"].as_u64())
                    .unwrap_or(total_tokens);
                cached_tokens = usage["cached_tokens"].as_u64()
                    .or_else(|| usage["cache_hit_tokens"].as_u64())
                    .or_else(|| usage["prompt_cache_hit_tokens"].as_u64())  // ← DeepSeek 字段
                    .or_else(|| usage["cachedReadTokens"].as_u64())
                    .or_else(|| usage["cached_content_tokens"].as_u64())
                    .or_else(|| {
                        usage["prompt_tokens_details"].as_object()
                            .and_then(|d| d["cached_tokens"].as_u64())
                    })
                    .unwrap_or_else(|| {
                        rjlog!("[CACHE] No cached_tokens found in usage. Keys: {:?}", usage.as_object().map(|o| o.keys().collect::<Vec<_>>()));
                        0
                    });
                if cached_tokens > 0 {
                    rjlog!("[CACHE HIT] Proxy detected {} cached tokens", cached_tokens);
                }
            }
        }

        result
    }
}

/// Convert OpenAI Chat Completions SSE stream → Responses API SSE stream.
///
/// Codex expects the full Responses streaming event chain:
///   response.created
///   response.output_item.added  ← activates the output item
///   response.content_part.added ← activates the text part
///   response.output_text.delta  ← actual token deltas (0..N)
///   response.content_part.done
///   response.output_item.done
///   response.completed
fn convert_chat_sse_to_responses_sse(chat_sse: &str, model: &str) -> String {
    let response_id = format!("resp_{}", chrono::Utc::now().timestamp_millis());
    let reasoning_id = format!("rs_{}", chrono::Utc::now().timestamp_millis());
    let item_id = format!("msg_{}", chrono::Utc::now().timestamp_millis());
    let part_id = format!("part_{}", chrono::Utc::now().timestamp_millis());
    let mut result = String::new();
    let mut started = false;
    let mut reasoning_started = false;
    let mut reasoning_done = false;
    let mut msg_started = false;
    let mut full_reasoning = String::new();
    let mut full_text = String::new();
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut total_tokens: u64 = 0;
    let mut cached_tokens: u64 = 0;
    let mut chunk_count = 0;
    let mut reasoning_count = 0;
    let mut content_count = 0;

    for line in chat_sse.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if data == "[DONE]" { continue; }
            if let Ok(chunk) = serde_json::from_str::<Value>(data) {
                chunk_count += 1;
                if !started {
                    result.push_str(&format!(
                        "event: response.created\ndata: {}\n\n",
                        serde_json::json!({"type":"response.created","response":{"id":response_id,"object":"response","model":model,"status":"in_progress","output":[]}})
                    ));
                    started = true;
                }
                let choice = &chunk["choices"][0];

                // --- Reasoning / thinking ---
                // OpenAI Responses API uses "response.reasoning_text.delta" for reasoning text
                if let Some(reasoning) = choice["delta"]["reasoning_content"].as_str() {
                    reasoning_count += 1;
                    if !reasoning_done && !msg_started {
                        if !reasoning_started {
                            reasoning_started = true;
                            rjlog!("[PROXY SSE] Starting reasoning output_item (id={})", reasoning_id);
                            result.push_str(&format!(
                                "event: response.output_item.added\ndata: {}\n\n",
                                serde_json::json!({"type":"response.output_item.added","output_index":0,"item":{"id":reasoning_id,"object":"realtime.item","type":"reasoning","status":"in_progress","role":"assistant","summary":[]}})
                            ));
                        }
                        full_reasoning.push_str(reasoning);
                        result.push_str(&format!(
                            "event: response.reasoning_text.delta\ndata: {}\n\n",
                            serde_json::json!({"type":"response.reasoning_text.delta","item_id":reasoning_id,"output_index":0,"content_index":0,"delta":reasoning})
                        ));
                    }
                }

                // --- Tool calls → function_call output items ---
                if let Some(tool_calls) = choice["delta"]["tool_calls"].as_array() {
                    // Close reasoning before emitting tool calls
                    if reasoning_started && !reasoning_done {
                        reasoning_done = true;
                        rjlog!("[PROXY SSE] Closing reasoning before tool_call");
                        result.push_str(&format!(
                            "event: response.reasoning_text.done\ndata: {}\n\n",
                            serde_json::json!({"type":"response.reasoning_text.done","item_id":reasoning_id,"output_index":0,"content_index":0,"text":full_reasoning})
                        ));
                        result.push_str(&format!(
                            "event: response.output_item.done\ndata: {}\n\n",
                            serde_json::json!({"type":"response.output_item.done","output_index":0,"item":{"id":reasoning_id,"object":"realtime.item","type":"reasoning","status":"completed","role":"assistant","summary":[{"type":"summary_text","text":full_reasoning}]}})
                        ));
                    }

                    let base_oi = if reasoning_done { 1usize } else { 0usize };
                    for tc in tool_calls {
                        let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                        let oi = base_oi + idx;

                        // First time seeing this tool → emit output_item.added
                        if tc.get("id").is_some() {
                            let fc_id = format!("fc_{}_{}", chrono::Utc::now().timestamp_millis(), idx);
                            let call_id = tc["id"].as_str().unwrap_or("");
                            let name = tc["function"]["name"].as_str().unwrap_or("");
                            rjlog!("[PROXY SSE] Starting function_call item (oi={}, id={}, name={})", oi, fc_id, name);
                            result.push_str(&format!(
                                "event: response.output_item.added\ndata: {}\n\n",
                                serde_json::json!({"type":"response.output_item.added","output_index":oi,"item":{"id":fc_id,"object":"realtime.item","type":"function_call","status":"in_progress","call_id":call_id,"name":name,"arguments":""}})
                            ));
                            content_count += 1;
                        }

                        // Arguments delta
                        if let Some(args) = tc["function"]["arguments"].as_str() {
                            let call_id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            result.push_str(&format!(
                                "event: response.function_call_arguments.delta\ndata: {}\n\n",
                                serde_json::json!({"type":"response.function_call_arguments.delta","output_index":oi,"call_id":call_id,"delta":args})
                            ));
                        }
                    }
                }

                // --- Actual content ---
                if let Some(delta_content) = choice["delta"]["content"].as_str() {
                    content_count += 1;
                    if reasoning_started && !reasoning_done {
                        reasoning_done = true;
                        rjlog!("[PROXY SSE] Reasoning done, closing reasoning item ({} chars)", full_reasoning.len());
                        result.push_str(&format!(
                            "event: response.reasoning_text.done\ndata: {}\n\n",
                            serde_json::json!({"type":"response.reasoning_text.done","item_id":reasoning_id,"output_index":0,"content_index":0,"text":full_reasoning})
                        ));
                        result.push_str(&format!(
                            "event: response.output_item.done\ndata: {}\n\n",
                            serde_json::json!({"type":"response.output_item.done","output_index":0,"item":{"id":reasoning_id,"object":"realtime.item","type":"reasoning","status":"completed","role":"assistant","summary":[{"type":"summary_text","text":full_reasoning}]}})
                        ));
                    }
                    if !msg_started {
                        msg_started = true;
                        let msg_idx = if reasoning_done { 1 } else { 0 };
                        result.push_str(&format!(
                            "event: response.output_item.added\ndata: {}\n\n",
                            serde_json::json!({"type":"response.output_item.added","output_index":msg_idx,"item":{"id":item_id,"object":"realtime.item","type":"message","status":"in_progress","role":"assistant","content":[]}})
                        ));
                        result.push_str(&format!(
                            "event: response.content_part.added\ndata: {}\n\n",
                            serde_json::json!({"type":"response.content_part.added","output_index":msg_idx,"content_index":0,"item_id":item_id,"part":{"id":part_id,"object":"realtime.item","type":"output_text","text":""}})
                        ));
                    }
                    full_text.push_str(delta_content);
                    let msg_idx = if reasoning_done { 1 } else { 0 };
                    result.push_str(&format!(
                        "event: response.output_text.delta\ndata: {}\n\n",
                        serde_json::json!({"type":"response.output_text.delta","item_id":item_id,"output_index":msg_idx,"content_index":0,"delta":delta_content})
                    ));
                }
                if let Some(usage) = chunk.get("usage") {
                    input_tokens = usage["prompt_tokens"].as_u64()
                        .or_else(|| usage["input_tokens"].as_u64())
                        .or_else(|| usage["inputTokens"].as_u64())
                        .unwrap_or(input_tokens);
                    output_tokens = usage["completion_tokens"].as_u64()
                        .or_else(|| usage["output_tokens"].as_u64())
                        .or_else(|| usage["outputTokens"].as_u64())
                        .unwrap_or(output_tokens);
                    total_tokens = usage["total_tokens"].as_u64()
                        .or_else(|| usage["totalTokens"].as_u64())
                        .unwrap_or(total_tokens);
                    cached_tokens = usage["cached_tokens"].as_u64()
                        .or_else(|| usage["cache_hit_tokens"].as_u64())
                        .or_else(|| usage["prompt_cache_hit_tokens"].as_u64())  // ← DeepSeek 字段
                        .or_else(|| usage["cachedReadTokens"].as_u64())
                        .or_else(|| usage["cached_content_tokens"].as_u64())
                        .or_else(|| {
                            usage["prompt_tokens_details"].as_object()
                                .and_then(|d| d["cached_tokens"].as_u64())
                        })
                        .unwrap_or(cached_tokens);
                    if cached_tokens > 0 {
                        rjlog!("[CACHE HIT] Chat SSE detected {} cached tokens", cached_tokens);
                    }
                }
            }
        }
    }

    rjlog!("[PROXY SSE] Processed {} chunks: reasoning={}, content={}", chunk_count, reasoning_count, content_count);

    // Close reasoning if never closed (no content after reasoning)
    if reasoning_started && !reasoning_done {
        reasoning_done = true;
        rjlog!("[PROXY SSE] Closing reasoning at end (no content deltas, {} chars)", full_reasoning.len());
        result.push_str(&format!(
            "event: response.reasoning_text.done\ndata: {}\n\n",
            serde_json::json!({"type":"response.reasoning_text.done","item_id":reasoning_id,"output_index":0,"content_index":0,"text":full_reasoning})
        ));
        result.push_str(&format!(
            "event: response.output_item.done\ndata: {}\n\n",
            serde_json::json!({"type":"response.output_item.done","output_index":0,"item":{"id":reasoning_id,"object":"realtime.item","type":"reasoning","status":"completed","role":"assistant","summary":[{"type":"summary_text","text":full_reasoning}]}})
        ));
    }

    let msg_idx = if reasoning_done { 1 } else { 0 };
    if msg_started {
        result.push_str(&format!(
            "event: response.content_part.done\ndata: {}\n\n",
            serde_json::json!({"type":"response.content_part.done","output_index":msg_idx,"content_index":0,"item_id":item_id,"part":{"id":part_id,"object":"realtime.item","type":"output_text","text":full_text}})
        ));
        result.push_str(&format!(
            "event: response.output_item.done\ndata: {}\n\n",
            serde_json::json!({"type":"response.output_item.done","output_index":msg_idx,"item":{"id":item_id,"object":"realtime.item","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":full_text}]}})
        ));
    }

    let mut output_items: Vec<Value> = vec![];
    if reasoning_done {
        output_items.push(serde_json::json!({"type":"reasoning","id":reasoning_id,"status":"completed","role":"assistant","summary":[{"type":"summary_text","text":full_reasoning}]}));
    }
    if msg_started || !full_text.is_empty() {
        output_items.push(serde_json::json!({"type":"message","id":item_id,"status":"completed","role":"assistant","content":[{"type":"output_text","text":full_text}]}));
    }

    result.push_str(&format!(
        "event: response.completed\ndata: {}\n\n",
        serde_json::json!({"type":"response.completed","response":{"id":response_id,"object":"response","model":model,"status":"completed","output":output_items,"usage":{"input_tokens":input_tokens,"output_tokens":output_tokens,"total_tokens":total_tokens,"cached_tokens":cached_tokens}}})
    ));

    rjlog!("[PROXY SSE] Total events emitted, reasoning_len={}, text_len={}, output_event_lines={}", full_reasoning.len(), full_text.len(), result.lines().count());
    if result.is_empty() { result = chat_sse.to_string(); rjlog!("[PROXY SSE] Result was empty, falling back to raw chat_sse"); }
    result
}

fn forward_to_anthropic(body: &str) -> (StatusCode, String) {
    let req: Value = match serde_json::from_str(body) { Ok(v) => v, Err(_) => return (StatusCode(400), "Invalid JSON".into()) };
    let model = req["model"].as_str().unwrap_or("claude-3-5-sonnet");
    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    let base = std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".into());

    let url = format!("{}/v1/messages", base.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .set("x-api-key", &api_key)
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .send_string(body);

    match resp {
        Ok(r) => (StatusCode(200), r.into_string().unwrap_or_default()),
        Err(e) => (StatusCode(502), format!("Forward error: {}", e)),
    }
}

fn convert_openai_to_anthropic(openai_resp: &str, model_name: &str) -> String {
    let resp: Value = match serde_json::from_str(openai_resp) { Ok(v) => v, Err(_) => return openai_resp.to_string() };
    let choice = &resp["choices"][0];
    let reasoning_content = choice["message"].get("reasoning_content").and_then(|v| v.as_str()).unwrap_or("");
    let content = choice["message"]["content"].as_str().unwrap_or("");
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("stop");
    let input_tokens = resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
    let output_tokens = resp["usage"]["completion_tokens"].as_u64().unwrap_or(0);

    let mut content_blocks: Vec<Value> = vec![];
    if !reasoning_content.is_empty() {
        content_blocks.push(serde_json::json!({"type": "thinking", "thinking": reasoning_content}));
    }
    if !content.is_empty() {
        content_blocks.push(serde_json::json!({"type": "text", "text": content}));
    }
    if let Some(tool_calls) = choice["message"].get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tool_calls {
            let id = tc["id"].as_str().unwrap_or("");
            let name = tc["function"]["name"].as_str().unwrap_or("");
            // Parse arguments string into JSON Value
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let input: Value = serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
            content_blocks.push(serde_json::json!({
                "type": "tool_use", "id": id, "name": name, "input": input
            }));
        }
    }

    let stop_reason = match finish_reason {
        "tool_calls" => "tool_use",
        "stop" => "end_turn",
        _ => "end_turn",
    };

    serde_json::json!({
        "id": format!("msg_{}", chrono::Utc::now().timestamp_millis()),
        "type": "message",
        "role": "assistant",
        "model": model_name,
        "content": content_blocks,
        "stop_reason": stop_reason,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
        }
    }).to_string()
}

/// Returns a closure that converts OpenAI Chat Completions SSE lines
/// → Anthropic Messages API SSE format, line by line (for streaming).
fn make_anthropic_sse_converter(model_name: String) -> impl FnMut(&str) -> Vec<u8> {
    use std::collections::HashMap;
    let msg_id = format!("msg_{}", chrono::Utc::now().timestamp_millis());
    let mut started = false;
    let mut next_block_idx: u32 = 0;
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut cached_tokens: u64 = 0;
    let mut finish_reason = String::new();
    let mut thinking_block: Option<u32> = None;
    let mut text_block: Option<u32> = None;
    let mut tool_blocks: HashMap<usize, (u32, bool, String)> = HashMap::new();
    let mut chunk_counter: u64 = 0;

    let mut close_thinking = |result: &mut Vec<u8>, tb: &mut Option<u32>| {
        if let Some(bi) = tb.take() {
            let _ = write!(result, "event: content_block_stop\ndata: {}\n\n", serde_json::json!({
                "type": "content_block_stop", "index": bi
            }));
        }
    };
    let mut close_text = |result: &mut Vec<u8>, tb: &mut Option<u32>| {
        if let Some(bi) = tb.take() {
            let _ = write!(result, "event: content_block_stop\ndata: {}\n\n", serde_json::json!({
                "type": "content_block_stop", "index": bi
            }));
        }
    };
    let mut close_tools = |result: &mut Vec<u8>, tbs: &mut HashMap<usize, (u32, bool, String)>| {
        for (_, (bi, started, _)) in tbs.iter() {
            if *started {
                let _ = write!(result, "event: content_block_stop\ndata: {}\n\n", serde_json::json!({
                    "type": "content_block_stop", "index": bi
                }));
            }
        }
        tbs.clear();
    };

    move |line: &str| -> Vec<u8> {
        let mut result = Vec::new();

        let data = match line.strip_prefix("data: ") {
            Some(d) => d,
            None => return result,
        };

        if data == "[DONE]" {
            close_thinking(&mut result, &mut thinking_block);
            close_tools(&mut result, &mut tool_blocks);
            close_text(&mut result, &mut text_block);

            rjlog!("[CACHE DEBUG] SSE DONE: input_tokens={}, output_tokens={}, cached_tokens={}", input_tokens, output_tokens, cached_tokens);
            
            // 存储 usage 数据到全局存储，供 ACP Client 读取
            let _ = store_usage_for_latest(
                model_name.clone(),
                input_tokens as i64,
                output_tokens as i64,
                cached_tokens as i64,
            );
            rjlog!("[CACHE DEBUG] Stored usage to global store: model={}, input={}, output={}, cached={}", 
                model_name, input_tokens, output_tokens, cached_tokens);
            
            if started {
                let stop_reason = if finish_reason == "tool_calls" { "tool_use" } else { "end_turn" };
                let _ = write!(result, "event: message_delta\ndata: {}\n\n", serde_json::json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": stop_reason, "stop_sequence": null},
                    "usage": {
                        // claude-agent-acp reads snake_case (input_tokens etc.);
                        // keep camelCase too for any client that expects it.
                        "input_tokens": input_tokens,
                        "output_tokens": output_tokens,
                        "cache_read_input_tokens": cached_tokens,
                        "cache_creation_input_tokens": 0,
                        "inputTokens": input_tokens,
                        "outputTokens": output_tokens,
                        "cachedReadTokens": cached_tokens,
                        "cachedWriteTokens": 0
                    }
                }));
                let _ = write!(result, "event: message_stop\ndata: {}\n\n", serde_json::json!({
                    "type": "message_stop"
                }));
            }
            return result;
        }

        // Log chunk data for debugging
        chunk_counter += 1;
        if chunk_counter <= 3 || data.contains("usage") {
            if data.contains("usage") {
                // Print full data for usage chunks
                rjlog!("[CACHE DEBUG] Anthropic SSE chunk #{} (FULL): {}", chunk_counter, data);
            } else {
                rjlog!("[CACHE DEBUG] Anthropic SSE chunk #{}: data={}", chunk_counter, safe_truncate(data, 300));
            }
        }

        if let Ok(chunk) = serde_json::from_str::<Value>(data) {
            // Log all top-level keys for debugging
            if data.contains("usage") {
                let keys = chunk.as_object().map(|o| o.keys().collect::<Vec<_>>());
                rjlog!("[CACHE DEBUG] Chunk keys: {:?}", keys);
            }

            if !started {
                started = true;
                let _ = write!(result, "event: message_start\ndata: {}\n\n", serde_json::json!({
                    "type": "message_start",
                    "message": {"id": msg_id, "type": "message", "role": "assistant", "content": [], "model": model_name, "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 0, "output_tokens": 0}}
                }));
            }

            let delta = &chunk["choices"][0]["delta"];

            // ---- Thinking / reasoning ----
            if let Some(reasoning) = delta["reasoning_content"].as_str() {
                if text_block.is_none() && tool_blocks.is_empty() {
                    if thinking_block.is_none() {
                        let bi = next_block_idx; next_block_idx += 1;
                        thinking_block = Some(bi);
                        let _ = write!(result, "event: content_block_start\ndata: {}\n\n", serde_json::json!({
                            "type": "content_block_start", "index": bi,
                            "content_block": {"type": "thinking", "thinking": ""}
                        }));
                    }
                    let bi = thinking_block.unwrap();
                    let _ = write!(result, "event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                        "type": "content_block_delta", "index": bi,
                        "delta": {"type": "thinking_delta", "thinking": reasoning}
                    }));
                }
            }

            // ---- Tool calls → tool_use content blocks ----
            if let Some(tool_calls) = delta["tool_calls"].as_array() {
                close_thinking(&mut result, &mut thinking_block);
                close_text(&mut result, &mut text_block);

                for tc in tool_calls {
                    let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                    let entry = tool_blocks.entry(idx).or_insert((0, false, String::new()));

                    if let Some(id) = tc["id"].as_str() {
                        if !entry.1 {
                            entry.1 = true;
                            let name = tc["function"]["name"].as_str().unwrap_or("");
                            entry.2 = name.to_string();
                            let bi = next_block_idx; next_block_idx += 1;
                            entry.0 = bi;
                            let _ = write!(result, "event: content_block_start\ndata: {}\n\n", serde_json::json!({
                                "type": "content_block_start", "index": bi,
                                "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}
                            }));
                        }
                    }

                    if let Some(args) = tc["function"]["arguments"].as_str() {
                        let _ = write!(result, "event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                            "type": "content_block_delta", "index": entry.0,
                            "delta": {"type": "input_json_delta", "partial_json": args}
                        }));
                    }
                }
            }

            // ---- Text content ----
            if let Some(content) = delta["content"].as_str() {
                close_thinking(&mut result, &mut thinking_block);
                close_tools(&mut result, &mut tool_blocks);

                if text_block.is_none() {
                    let bi = next_block_idx; next_block_idx += 1;
                    text_block = Some(bi);
                    let _ = write!(result, "event: content_block_start\ndata: {}\n\n", serde_json::json!({
                        "type": "content_block_start", "index": bi,
                        "content_block": {"type": "text", "text": ""}
                    }));
                }
                let bi = text_block.unwrap();
                let _ = write!(result, "event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                    "type": "content_block_delta", "index": bi,
                    "delta": {"type": "text_delta", "text": content}
                }));
            }

            if let Some(fr) = chunk["choices"][0]["finish_reason"].as_str() {
                finish_reason = fr.to_string();
            }
            if let Some(usage) = chunk.get("usage") {
                let raw_usage = serde_json::to_string(usage).unwrap_or_default();
                rjlog!("[PROXY USAGE] Raw usage from upstream: {}", raw_usage);
                input_tokens = usage["prompt_tokens"].as_u64()
                    .or_else(|| usage["input_tokens"].as_u64())
                    .or_else(|| usage["inputTokens"].as_u64())
                    .unwrap_or(input_tokens);
                output_tokens = usage["completion_tokens"].as_u64()
                    .or_else(|| usage["output_tokens"].as_u64())
                    .or_else(|| usage["outputTokens"].as_u64())
                    .unwrap_or(output_tokens);
                cached_tokens = usage["cached_tokens"].as_u64()
                    .or_else(|| usage["cache_hit_tokens"].as_u64())
                    .or_else(|| usage["prompt_cache_hit_tokens"].as_u64())  // ← DeepSeek 字段
                    .or_else(|| usage["cachedReadTokens"].as_u64())
                    .or_else(|| usage["cached_content_tokens"].as_u64())
                    .or_else(|| usage["cachedTokens"].as_u64())
                    // Try nested prompt_tokens_details.cached_tokens
                    .or_else(|| {
                        usage["prompt_tokens_details"].as_object()
                            .and_then(|d| d["cached_tokens"].as_u64())
                    })
                    .unwrap_or_else(|| {
                        rjlog!("[CACHE] No cached_tokens found in usage. Keys: {:?}", usage.as_object().map(|o| o.keys().collect::<Vec<_>>()));
                        0
                    });
                if cached_tokens > 0 {
                    rjlog!("[CACHE HIT] Anthropic SSE detected {} cached tokens", cached_tokens);
                }
            }
        }

        result
    }
}

fn make_llama_cpp_sse_converter(model_name: String) -> impl FnMut(&str) -> Vec<u8> {
    use std::collections::HashMap;
    let msg_id = format!("msg_{}", chrono::Utc::now().timestamp_millis());
    let mut started = false;
    let mut next_block_idx: u32 = 0;
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut finish_reason = String::new();
    let mut thinking_block: Option<u32> = None;
    let mut text_block: Option<u32> = None;
    let mut tool_blocks: HashMap<usize, (u32, bool, String)> = HashMap::new();
    let mut in_think_tag = false;
    let mut think_buffer = String::new();

    let mut close_thinking = |result: &mut Vec<u8>, tb: &mut Option<u32>| {
        if let Some(bi) = tb.take() {
            let _ = write!(result, "event: content_block_stop\ndata: {}\n\n", serde_json::json!({
                "type": "content_block_stop", "index": bi
            }));
        }
    };
    let mut close_text = |result: &mut Vec<u8>, tb: &mut Option<u32>| {
        if let Some(bi) = tb.take() {
            let _ = write!(result, "event: content_block_stop\ndata: {}\n\n", serde_json::json!({
                "type": "content_block_stop", "index": bi
            }));
        }
    };
    let mut close_tools = |result: &mut Vec<u8>, tbs: &mut HashMap<usize, (u32, bool, String)>| {
        for (_, (bi, started, _)) in tbs.iter() {
            if *started {
                let _ = write!(result, "event: content_block_stop\ndata: {}\n\n", serde_json::json!({
                    "type": "content_block_stop", "index": bi
                }));
            }
        }
        tbs.clear();
    };

    move |line: &str| -> Vec<u8> {
        let mut result = Vec::new();

        let data = match line.strip_prefix("data: ") {
            Some(d) => d,
            None => return result,
        };

        if data == "[DONE]" {
            if !think_buffer.is_empty() {
                if thinking_block.is_none() {
                    let bi = next_block_idx; next_block_idx += 1;
                    thinking_block = Some(bi);
                    let _ = write!(result, "event: content_block_start\ndata: {}\n\n", serde_json::json!({
                        "type": "content_block_start", "index": bi,
                        "content_block": {"type": "thinking", "thinking": ""}
                    }));
                }
                let bi = thinking_block.unwrap();
                let _ = write!(result, "event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                    "type": "content_block_delta", "index": bi,
                    "delta": {"type": "thinking_delta", "thinking": &think_buffer}
                }));
                think_buffer.clear();
            }
            close_thinking(&mut result, &mut thinking_block);
            close_tools(&mut result, &mut tool_blocks);
            close_text(&mut result, &mut text_block);

            if started {
                let stop_reason = if finish_reason == "tool_calls" { "tool_use" } else { "end_turn" };
                let _ = write!(result, "event: message_delta\ndata: {}\n\n", serde_json::json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": stop_reason, "stop_sequence": null},
                    "usage": {"output_tokens": output_tokens}
                }));
                let _ = write!(result, "event: message_stop\ndata: {}\n\n", serde_json::json!({
                    "type": "message_stop"
                }));
            }
            return result;
        }

        if let Ok(chunk) = serde_json::from_str::<Value>(data) {
            if !started {
                started = true;
                let _ = write!(result, "event: message_start\ndata: {}\n\n", serde_json::json!({
                    "type": "message_start",
                    "message": {"id": msg_id, "type": "message", "role": "assistant", "content": [], "model": model_name, "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 0, "output_tokens": 0}}
                }));
            }

            let delta = &chunk["choices"][0]["delta"];

            if let Some(reasoning) = delta["reasoning_content"].as_str() {
                if text_block.is_none() && tool_blocks.is_empty() {
                    if thinking_block.is_none() {
                        let bi = next_block_idx; next_block_idx += 1;
                        thinking_block = Some(bi);
                        let _ = write!(result, "event: content_block_start\ndata: {}\n\n", serde_json::json!({
                            "type": "content_block_start", "index": bi,
                            "content_block": {"type": "thinking", "thinking": ""}
                        }));
                    }
                    let bi = thinking_block.unwrap();
                    let _ = write!(result, "event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                        "type": "content_block_delta", "index": bi,
                        "delta": {"type": "thinking_delta", "thinking": reasoning}
                    }));
                }
            }

            if let Some(tool_calls) = delta["tool_calls"].as_array() {
                close_thinking(&mut result, &mut thinking_block);
                close_text(&mut result, &mut text_block);

                for tc in tool_calls {
                    let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                    let entry = tool_blocks.entry(idx).or_insert((0, false, String::new()));

                    if let Some(id) = tc["id"].as_str() {
                        if !entry.1 {
                            entry.1 = true;
                            let name = tc["function"]["name"].as_str().unwrap_or("");
                            entry.2 = name.to_string();
                            let bi = next_block_idx; next_block_idx += 1;
                            entry.0 = bi;
                            let _ = write!(result, "event: content_block_start\ndata: {}\n\n", serde_json::json!({
                                "type": "content_block_start", "index": bi,
                                "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}
                            }));
                        }
                    }

                    if let Some(args) = tc["function"]["arguments"].as_str() {
                        let _ = write!(result, "event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                            "type": "content_block_delta", "index": entry.0,
                            "delta": {"type": "input_json_delta", "partial_json": args}
                        }));
                    }
                }
            }

            if let Some(content) = delta["content"].as_str() {
                close_tools(&mut result, &mut tool_blocks);

                let mut remaining = content.to_string();
                while !remaining.is_empty() {
                    if in_think_tag {
                        if let Some(end_pos) = remaining.find("<｜end_of_thought｜>") {
                            think_buffer.push_str(&remaining[..end_pos]);
                            remaining = remaining[end_pos + "<｜end_of_thought｜>".len()..].to_string();
                            in_think_tag = false;
                        } else if let Some(end_pos) = remaining.find("</think>") {
                            think_buffer.push_str(&remaining[..end_pos]);
                            remaining = remaining[end_pos + "</think>".len()..].to_string();
                            in_think_tag = false;
                        } else {
                            think_buffer.push_str(&remaining);
                            remaining.clear();
                        }
                    } else if let Some(start_pos) = remaining.find("<｜begin_of_thought｜>") {
                        let before = &remaining[..start_pos];
                        if !before.is_empty() {
                            if text_block.is_none() {
                                let bi = next_block_idx; next_block_idx += 1;
                                text_block = Some(bi);
                                let _ = write!(result, "event: content_block_start\ndata: {}\n\n", serde_json::json!({
                                    "type": "content_block_start", "index": bi,
                                    "content_block": {"type": "text", "text": ""}
                                }));
                            }
                            let bi = text_block.unwrap();
                            let _ = write!(result, "event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                                "type": "content_block_delta", "index": bi,
                                "delta": {"type": "text_delta", "text": before}
                            }));
                        }
                        remaining = remaining[start_pos + "<｜begin_of_thought｜>".len()..].to_string();
                        in_think_tag = true;
                    } else if let Some(start_pos) = remaining.find("<think>") {
                        let before = &remaining[..start_pos];
                        if !before.is_empty() {
                            if text_block.is_none() {
                                let bi = next_block_idx; next_block_idx += 1;
                                text_block = Some(bi);
                                let _ = write!(result, "event: content_block_start\ndata: {}\n\n", serde_json::json!({
                                    "type": "content_block_start", "index": bi,
                                    "content_block": {"type": "text", "text": ""}
                                }));
                            }
                            let bi = text_block.unwrap();
                            let _ = write!(result, "event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                                "type": "content_block_delta", "index": bi,
                                "delta": {"type": "text_delta", "text": before}
                            }));
                        }
                        remaining = remaining[start_pos + "<think>".len()..].to_string();
                        in_think_tag = true;
                    } else {
                        if text_block.is_none() {
                            let bi = next_block_idx; next_block_idx += 1;
                            text_block = Some(bi);
                            let _ = write!(result, "event: content_block_start\ndata: {}\n\n", serde_json::json!({
                                "type": "content_block_start", "index": bi,
                                "content_block": {"type": "text", "text": ""}
                            }));
                        }
                        let bi = text_block.unwrap();
                        let _ = write!(result, "event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                            "type": "content_block_delta", "index": bi,
                            "delta": {"type": "text_delta", "text": &remaining}
                        }));
                        remaining.clear();
                    }
                }

                if !think_buffer.is_empty() {
                    if thinking_block.is_none() {
                        let bi = next_block_idx; next_block_idx += 1;
                        thinking_block = Some(bi);
                        let _ = write!(result, "event: content_block_start\ndata: {}\n\n", serde_json::json!({
                            "type": "content_block_start", "index": bi,
                            "content_block": {"type": "thinking", "thinking": ""}
                        }));
                    }
                    let bi = thinking_block.unwrap();
                    let _ = write!(result, "event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                        "type": "content_block_delta", "index": bi,
                        "delta": {"type": "thinking_delta", "thinking": &think_buffer}
                    }));
                    think_buffer.clear();
                }
            }

            if let Some(fr) = chunk["choices"][0]["finish_reason"].as_str() {
                finish_reason = fr.to_string();
            }
            if let Some(usage) = chunk.get("usage") {
                input_tokens = usage["prompt_tokens"].as_u64().unwrap_or(input_tokens);
                output_tokens = usage["completion_tokens"].as_u64().unwrap_or(output_tokens);
            }
        }

        result
    }
}

fn convert_openai_sse_to_claude_sse(openai_sse: &str, model_name: &str) -> String {
    use std::collections::HashMap;

    let msg_id = format!("msg_{}", chrono::Utc::now().timestamp_millis());
    let mut result = String::new();
    let mut started = false;
    let mut next_block_idx: u32 = 0;
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut finish_reason = String::new();

    // Track currently-open content blocks (block_index)
    let mut thinking_block: Option<u32> = None;
    let mut text_block: Option<u32> = None;
    // Active tool_use blocks: OpenAI tool_call index → (block_idx, started, name)
    let mut tool_blocks: HashMap<usize, (u32, bool, String)> = HashMap::new();

    // ----- helpers -----
    let close_thinking = |result: &mut String, thinking_block: &mut Option<u32>| {
        if let Some(bi) = thinking_block.take() {
            result.push_str(&format!("event: content_block_stop\ndata: {}\n\n", serde_json::json!({
                "type": "content_block_stop", "index": bi
            })));
        }
    };
    let close_text = |result: &mut String, text_block: &mut Option<u32>| {
        if let Some(bi) = text_block.take() {
            result.push_str(&format!("event: content_block_stop\ndata: {}\n\n", serde_json::json!({
                "type": "content_block_stop", "index": bi
            })));
        }
    };
    let close_tools = |result: &mut String, tool_blocks: &mut HashMap<usize, (u32, bool, String)>| {
        for (_, (bi, started, _)) in tool_blocks.iter() {
            if *started {
                result.push_str(&format!("event: content_block_stop\ndata: {}\n\n", serde_json::json!({
                    "type": "content_block_stop", "index": bi
                })));
            }
        }
        tool_blocks.clear();
    };

    for line in openai_sse.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if data == "[DONE]" { continue; }
            if let Ok(chunk) = serde_json::from_str::<Value>(data) {
                if !started {
                    started = true;
                    result.push_str(&format!("event: message_start\ndata: {}\n\n", serde_json::json!({
                        "type": "message_start",
                        "message": {"id": msg_id, "type": "message", "role": "assistant", "content": [], "model": model_name, "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 0, "output_tokens": 0}}
                    })));
                }

                let delta = &chunk["choices"][0]["delta"];

                // ---- Thinking / reasoning ----
                if let Some(reasoning) = delta["reasoning_content"].as_str() {
                    // Only emit thinking if no text or tool blocks are active yet
                    if text_block.is_none() && tool_blocks.is_empty() {
                        if thinking_block.is_none() {
                            let bi = next_block_idx; next_block_idx += 1;
                            thinking_block = Some(bi);
                            rjlog!("[PROXY ANTHROPIC SSE] Starting thinking block (idx={})", bi);
                            result.push_str(&format!("event: content_block_start\ndata: {}\n\n", serde_json::json!({
                                "type": "content_block_start", "index": bi,
                                "content_block": {"type": "thinking", "thinking": ""}
                            })));
                        }
                        let bi = thinking_block.unwrap();
                        result.push_str(&format!("event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                            "type": "content_block_delta", "index": bi,
                            "delta": {"type": "thinking_delta", "thinking": reasoning}
                        })));
                    }
                }

                // ---- Tool calls → tool_use content blocks ----
                if let Some(tool_calls) = delta["tool_calls"].as_array() {
                    close_thinking(&mut result, &mut thinking_block);
                    close_text(&mut result, &mut text_block);

                    for tc in tool_calls {
                        let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                        let entry = tool_blocks.entry(idx).or_insert((0, false, String::new()));

                        if let Some(id) = tc["id"].as_str() {
                            if !entry.1 {
                                entry.1 = true;
                                let name = tc["function"]["name"].as_str().unwrap_or("");
                                entry.2 = name.to_string();
                                let bi = next_block_idx; next_block_idx += 1;
                                entry.0 = bi;
                                rjlog!("[PROXY ANTHROPIC SSE] Starting tool_use block (idx={}, name={})", bi, name);
                                result.push_str(&format!("event: content_block_start\ndata: {}\n\n", serde_json::json!({
                                    "type": "content_block_start", "index": bi,
                                    "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}
                                })));
                            }
                        }

                        if let Some(args) = tc["function"]["arguments"].as_str() {
                            result.push_str(&format!("event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                                "type": "content_block_delta", "index": entry.0,
                                "delta": {"type": "input_json_delta", "partial_json": args}
                            })));
                        }
                    }
                }

                // ---- Text content ----
                if let Some(content) = delta["content"].as_str() {
                    close_thinking(&mut result, &mut thinking_block);
                    close_tools(&mut result, &mut tool_blocks);

                    if text_block.is_none() {
                        let bi = next_block_idx; next_block_idx += 1;
                        text_block = Some(bi);
                        result.push_str(&format!("event: content_block_start\ndata: {}\n\n", serde_json::json!({
                            "type": "content_block_start", "index": bi,
                            "content_block": {"type": "text", "text": ""}
                        })));
                    }
                    let bi = text_block.unwrap();
                    result.push_str(&format!("event: content_block_delta\ndata: {}\n\n", serde_json::json!({
                        "type": "content_block_delta", "index": bi,
                        "delta": {"type": "text_delta", "text": content}
                    })));
                }

                if let Some(fr) = chunk["choices"][0]["finish_reason"].as_str() {
                    finish_reason = fr.to_string();
                }
                if let Some(usage) = chunk.get("usage") {
                    input_tokens = usage["prompt_tokens"].as_u64().unwrap_or(input_tokens);
                    output_tokens = usage["completion_tokens"].as_u64().unwrap_or(output_tokens);
                }
            }
        }
    }

    // Close all remaining open blocks
    close_thinking(&mut result, &mut thinking_block);
    close_tools(&mut result, &mut tool_blocks);
    close_text(&mut result, &mut text_block);

    let stop_reason = if finish_reason == "tool_calls" { "tool_use" } else { "end_turn" };

    if started {
        result.push_str(&format!("event: message_delta\ndata: {}\n\n", serde_json::json!({
            "type": "message_delta",
            "delta": {"stop_reason": stop_reason, "stop_sequence": null},
            "usage": {"output_tokens": output_tokens}
        })));
        result.push_str(&format!("event: message_stop\ndata: {}\n\n", serde_json::json!({
            "type": "message_stop"
        })));
    }

    rjlog!("[PROXY ANTHROPIC SSE] thinking={}, text={}, tools={}, output lines={}",
        thinking_block.is_some(), text_block.is_some(), tool_blocks.len(), result.lines().count());
    if result.is_empty() { result = openai_sse.to_string(); rjlog!("[PROXY ANTHROPIC SSE] Result was empty, falling back to raw openai_sse"); }
    result
}

fn proxy_gemini_to_openai(body: &str, models: &[ModelEntry], path: &str, preferred_ids: Option<&[String]>, reasoning_disabled: bool) -> ProxyResponse {
    let req: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return ProxyResponse::Sync(StatusCode(400), format!("Invalid JSON: {}", e)) };

    let model_name = extract_model_from_path(path).unwrap_or("gemini-1.5-pro");

    rjlog!("[PROXY] Gemini: {} (body {} chars)", path, body.len());

    // handle non-generateContent calls (countTokens, embedContent, etc.)
    let is_count_tokens = path.contains(":countTokens");
    let is_stream_generate = path.contains(":streamGenerateContent");
    if is_count_tokens {
        // Return a dummy count — Gemini only needs a plausible number
        return ProxyResponse::Sync(StatusCode(200), serde_json::json!({"totalTokens": 0}).to_string());
    }
    // streamGenerateContent is always streaming; generateContent may or may not be
    let stream = is_stream_generate || req.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    rjlog!("[PROXY] Gemini streaming decision: path_stream={}, req_stream={}, stream={}", is_stream_generate, req.get("stream").and_then(|v| v.as_bool()).unwrap_or(false), stream);

    // Use safe .get() — Gemini may also send list models / other requests
    let Some(contents) = req.get("contents") else {
        return ProxyResponse::Sync(StatusCode(200), "{}".to_string());
    };
    
    let max_output_tokens = if let Some(config) = req.get("generationConfig").and_then(|v| v.as_object()) {
        config.get("maxOutputTokens").and_then(|v| v.as_u64()).unwrap_or(4096)
    } else {
        4096
    };

    let target = find_model(models, model_name, preferred_ids);

    let (api_key, base_url, real_model, support_tools, provider) = if let Some(m) = target {
        (m.api_key.clone(), m.api_base.clone(), m.name.clone(), m.support_tools, m.provider.clone())
    } else {
        let (s, b) = forward_to_gemini(body, path);
        return ProxyResponse::Sync(s, b);
    };

    let mut openai_messages: Vec<Value> = vec![];
    // Gemini functionCall parts carry no id. OpenAI requires each tool message to
    // reference an assistant tool_call id, so synthesize ids deterministically and
    // pair every functionResponse with the NEXT pending call (Gemini emits calls
    // and their responses in order). The previous `gc_{name}` key never matched
    // `gc_{ci}_{pi}`, so any tool-using turn was rejected by the upstream API
    // with 400 ("tool_call_id not found") — exactly what surfaced after thinking.
    let mut pending_function_call_ids: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    if let Some(contents_array) = contents.as_array() {
        for (ci, content) in contents_array.iter().enumerate() {
            if let Some(parts) = content.get("parts").and_then(|v| v.as_array()) {
                let role = content.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                let is_model = role == "model";
                let mut text = String::new();
                let mut tool_calls: Vec<Value> = vec![];

                for (pi, part) in parts.iter().enumerate() {
                    if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                        text.push_str(t);
                    } else if let Some(fc) = part.get("functionCall") {
                        let name = fc["name"].as_str().unwrap_or("");
                        let args = fc["args"].to_string();
                        // Gemini functionCall has no id, synthesize one deterministically
                        let call_id = format!("gc_{}_{}", ci, pi);
                        pending_function_call_ids.push_back(call_id.clone());
                        tool_calls.push(serde_json::json!({
                            "id": call_id,
                            "type": "function",
                            "function": {"name": name, "arguments": args}
                        }));
                    } else if let Some(fr) = part.get("functionResponse") {
                        let response = fr["response"].to_string();
                        // Pair with the pending function call (responses arrive in
                        // the same order as their calls).
                        let call_id = pending_function_call_ids.pop_front()
                            .unwrap_or_else(|| {
                                let name = fr["name"].as_str().unwrap_or("");
                                rjlog!("[PROXY] Gemini→OpenAI: functionResponse without matching functionCall (name={})", name);
                                format!("gc_{}", name)
                            });
                        openai_messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": response
                        }));
                    }
                }

                if is_model {
                    let mut msg = serde_json::json!({"role": "assistant", "content": text});
                    if !tool_calls.is_empty() {
                        msg["tool_calls"] = serde_json::json!(tool_calls);
                    }
                    if !text.is_empty() || !tool_calls.is_empty() {
                        openai_messages.push(msg);
                    }
                } else if !text.is_empty() {
                    openai_messages.push(serde_json::json!({"role": "user", "content": text}));
                }
            }
        }
    }

    // Convert Gemini tools → OpenAI tools format
    let mut openai_tools: Vec<Value> = vec![];
    if let Some(tools_arr) = req.get("tools").and_then(|v| v.as_array()) {
        for tool in tools_arr {
            if let Some(decls) = tool.get("functionDeclarations").and_then(|v| v.as_array()) {
                for decl in decls {
                    let name = decl["name"].as_str().unwrap_or("");
                    let description = decl["description"].as_str().unwrap_or("");
                    let params = &decl["parameters"];
                    openai_tools.push(serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": name,
                            "description": description,
                            "parameters": params
                        }
                    }));
                }
            }
        }
    }

    let mut openai_body = serde_json::json!({
        "model": real_model,
        "messages": openai_messages,
        "max_tokens": max_output_tokens,
        "stream": stream,
    });
    if !openai_tools.is_empty() && support_tools {
        openai_body["tools"] = serde_json::json!(openai_tools);
    } else if !openai_tools.is_empty() && !support_tools {
        rjlog!("[PROXY] Gemini→OpenAI: model {} does not support tools, skipping tool definitions", real_model);
    }

    if reasoning_disabled {
        openai_body["thinking"] = serde_json::json!({"type": "disabled"});
        openai_body["reasoning_effort"] = serde_json::json!("low");
        openai_body["enable_thinking"] = serde_json::json!(false);
        openai_body["temperature"] = serde_json::json!(0.6);
    }

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_string(&openai_body.to_string());

    match resp {
        Ok(response) => {
            if stream {
                let reader = response.into_reader();
                let buf_reader = BufReader::new(Box::new(reader) as Box<dyn Read + Send>);
                let converter = SseStreamConverter::new(
                    buf_reader,
                    Box::new(make_gemini_sse_converter(model_name.to_string())),
                );
                rjlog!("[PROXY] Gemini: returning streaming response");
                ProxyResponse::Stream { reader: Box::new(converter) }
            } else {
                let resp_body = response.into_string().unwrap_or_default();
                let converted = convert_openai_to_gemini(&resp_body);
                ProxyResponse::Sync(StatusCode(200), converted)
            }
        }
        Err(e) => {
            // Surface the upstream's actual response body — a bare "status code
            // 400" says nothing about WHICH field the API rejected.
            let detail = match e {
                ureq::Error::Status(code, resp) => {
                    let body = resp.into_string().unwrap_or_default();
                    rjlog!("[PROXY] Gemini upstream returned {}: {}", code, body.chars().take(500).collect::<String>());
                    format!("status code {}: {}", code, body.chars().take(300).collect::<String>())
                }
                other => other.to_string(),
            };
            let err_body = serde_json::json!({
                "error": {"code": 502, "message": format!("Proxy error: {}", detail)}
            });
            ProxyResponse::Sync(StatusCode(502), err_body.to_string())
        }
    }
}

fn extract_model_from_path(path: &str) -> Option<&str> {
    for prefix in &["/v1/models/", "/v1beta/models/"] {
        if let Some(start) = path.find(prefix) {
            let start = start + prefix.len();
            let mut colon_count = 0;
            let end = path[start..].find(|c: char| {
                if c == ':' {
                    colon_count += 1;
                    colon_count >= 2
                } else {
                    c == '/'
                }
            }).unwrap_or(path[start..].len());
            return Some(&path[start..start + end]);
        }
    }
    None
}

fn forward_to_gemini(body: &str, path: &str) -> (StatusCode, String) {
    let api_key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
    let base = std::env::var("GOOGLE_GEMINI_BASE_URL").unwrap_or_else(|_| "https://generativelanguage.googleapis.com".into());
    
    let url = format!("{}{}", base, path);
    let resp = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_string(body);

    match resp {
        Ok(r) => (StatusCode(200), r.into_string().unwrap_or_default()),
        Err(e) => (StatusCode(502), format!("Forward error: {}", e)),
    }
}

fn convert_openai_to_gemini(openai_resp: &str) -> String {
    let resp: Value = match serde_json::from_str(openai_resp) { Ok(v) => v, Err(_) => return openai_resp.to_string() };
    let choice = &resp["choices"][0];
    let reasoning_content = choice["message"].get("reasoning_content").and_then(|v| v.as_str()).unwrap_or("");
    let content = choice["message"]["content"].as_str().unwrap_or("");

    let mut parts: Vec<Value> = vec![];
    if !reasoning_content.is_empty() {
        // Strip newlines from reasoning content to keep Gemini thinking clean.
        let reasoning_clean = reasoning_content.replace('\n', " ");
        parts.push(serde_json::json!({"text": reasoning_clean, "thought": true}));
    }
    // Tool calls → functionCall parts
    if let Some(tool_calls) = choice["message"].get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tool_calls {
            let name = tc["function"]["name"].as_str().unwrap_or("");
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let args: Value = serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
            parts.push(serde_json::json!({"functionCall": {"name": name, "args": args}}));
        }
    }
    if !content.is_empty() {
        parts.push(serde_json::json!({"text": content}));
    }

    serde_json::json!({
        "candidates": [{
            "content": {
                "parts": parts,
                "role": "model"
            },
            "finishReason": "STOP",
            "safetyRatings": []
        }],
        "usageMetadata": {
            "promptTokenCount": resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            "candidatesTokenCount": resp["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            "totalTokenCount": resp["usage"]["total_tokens"].as_u64().unwrap_or(0),
        }
    }).to_string()
}

/// Returns a closure that converts OpenAI Chat Completions SSE lines
/// → Gemini SSE format, line by line (for streaming).
fn make_gemini_sse_converter(model: String) -> impl FnMut(&str) -> Vec<u8> {
    use std::collections::HashMap;
    let mut pending_tools: HashMap<usize, (String, String)> = HashMap::new();
    let mut has_pending_tools = false;
    // Gemini requires a candidate chunk carrying finishReason for the stream to
    // be considered valid. Without it, gemini-cli treats the stream as an
    // InvalidStreamError (NO_FINISH_REASON) and RETRIES the whole request —
    // each retry re-generates a full reply that then gets appended to the same
    // UI message (the "repeated welcome messages" bug). Track whether we've
    // emitted a finishReason so we can synthesize one at stream end if the
    // upstream never sent it.
    let mut sent_finish = false;
    // Accumulate upstream token usage so it can be stored to the global store at
    // [DONE] — the ACP client attaches it to the finish event, and gemini-cli
    // itself never forwards usage, so without this gemini sessions show no
    // token/cache info at all.
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut cached_tokens: u64 = 0;

    let flush_tools = |result: &mut Vec<u8>, pending: &mut HashMap<usize, (String, String)>, has: &mut bool| {
        if !*has { return; }
        for (_, (name, args_str)) in pending.drain() {
            let args: Value = serde_json::from_str(&args_str).unwrap_or(Value::Null);
            let event = serde_json::json!({
                "candidates": [{
                    "content": {
                        "parts": [{"functionCall": {"name": name, "args": args}}],
                        "role": "model"
                    },
                    "finishReason": null,
                    "safetyRatings": []
                }]
            });
            let _ = write!(result, "data: {}\n\n", event);
        }
        *has = false;
    };

    move |line: &str| -> Vec<u8> {
        let mut result = Vec::new();

        let data = match line.strip_prefix("data: ") {
            Some(d) => d,
            None => return result,
        };

        if data == "[DONE]" {
            flush_tools(&mut result, &mut pending_tools, &mut has_pending_tools);
            // Store upstream usage to the global store so the ACP client can
            // attach it to the finish event (gemini-cli never sends usage_update).
            store_usage_for_latest(model.clone(), input_tokens as i64, output_tokens as i64, cached_tokens as i64);
            rjlog!("[PROXY USAGE] Gemini stream done: input={}, output={}, cached={}", input_tokens, output_tokens, cached_tokens);
            // Gemini requires a finishReason to consider the stream valid; the
            // upstream [DONE] carries none, so synthesize one now (unless a
            // finishReason chunk was already forwarded above).
            if !sent_finish {
                sent_finish = true;
                let event = serde_json::json!({
                    "candidates": [{
                        "content": { "parts": [], "role": "model" },
                        "finishReason": "STOP",
                        "safetyRatings": []
                    }]
                });
                let _ = write!(result, "data: {}\n\n", event);
            }
            // Gemini CLI reads the stream's final `usageMetadata` (promptTokenCount
            // / candidatesTokenCount / cachedContentTokenCount) and forwards it via
            // ACP `usage_update` — without this, gemini-cli reports used=0 and the
            // UI shows no token/cache info even though upstream returned usage.
            if input_tokens > 0 || output_tokens > 0 || cached_tokens > 0 {
                let usage_event = serde_json::json!({
                    "candidates": [],
                    "usageMetadata": {
                        "promptTokenCount": input_tokens,
                        "candidatesTokenCount": output_tokens,
                        "cachedContentTokenCount": cached_tokens,
                        "totalTokenCount": input_tokens + output_tokens,
                    }
                });
                let _ = write!(result, "data: {}\n\n", usage_event);
            }
            result.extend_from_slice(b"data: {\"done\":true}\n\n");
            return result;
        }

        if let Ok(chunk) = serde_json::from_str::<Value>(data) {
            let delta = &chunk["choices"][0]["delta"];

            // Forward the upstream finish_reason (e.g. "stop"/"length") as a
            // Gemini finishReason chunk so gemini-cli never sees a stream that
            // ends without one (which would trigger a request retry).
            if let Some(fr) = chunk["choices"][0]["finish_reason"].as_str() {
                if !fr.is_empty() && !sent_finish {
                    sent_finish = true;
                    let gemini_fr = if fr == "length" { "MAX_TOKENS" } else { "STOP" };
                    let event = serde_json::json!({
                        "candidates": [{
                            "content": { "parts": [], "role": "model" },
                            "finishReason": gemini_fr,
                            "safetyRatings": []
                        }]
                    });
                    let _ = write!(result, "data: {}\n\n", event);
                }
            }

            // Accumulate upstream token usage (OpenAI-style `usage` object in the
            // final chunk: prompt_tokens/completion_tokens + DeepSeek's
            // prompt_cache_hit_tokens / prompt_tokens_details.cached_tokens).
            if let Some(usage) = chunk.get("usage") {
                input_tokens = usage["prompt_tokens"].as_u64()
                    .or_else(|| usage["input_tokens"].as_u64())
                    .or_else(|| usage["inputTokens"].as_u64())
                    .unwrap_or(input_tokens);
                output_tokens = usage["completion_tokens"].as_u64()
                    .or_else(|| usage["output_tokens"].as_u64())
                    .or_else(|| usage["outputTokens"].as_u64())
                    .unwrap_or(output_tokens);
                cached_tokens = usage["cached_tokens"].as_u64()
                    .or_else(|| usage["cache_hit_tokens"].as_u64())
                    .or_else(|| usage["prompt_cache_hit_tokens"].as_u64())
                    .or_else(|| usage["cachedReadTokens"].as_u64())
                    .or_else(|| usage["cached_content_tokens"].as_u64())
                    .or_else(|| {
                        usage["prompt_tokens_details"].as_object()
                            .and_then(|d| d["cached_tokens"].as_u64())
                    })
                    .unwrap_or(cached_tokens);
            }

            // Tool calls — accumulate for later emission
            if let Some(tcs) = delta["tool_calls"].as_array() {
                has_pending_tools = true;
                for tc in tcs {
                    let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                    let entry = pending_tools.entry(idx).or_insert_with(|| {
                        (tc["function"]["name"].as_str().unwrap_or("").to_string(), String::new())
                    });
                    if let Some(args) = tc["function"]["arguments"].as_str() {
                        entry.1.push_str(args);
                    }
                }
            }

            // Reasoning — flush tools first, then emit thought
            if let Some(reasoning) = delta["reasoning_content"].as_str() {
                flush_tools(&mut result, &mut pending_tools, &mut has_pending_tools);
                let reasoning_clean = reasoning.replace('\n', " ");
                if !reasoning_clean.trim().is_empty() {
                    let event = serde_json::json!({
                        "candidates": [{
                            "content": {
                                "parts": [{"text": reasoning_clean, "thought": true}],
                                "role": "model"
                            },
                            "finishReason": null,
                            "safetyRatings": []
                        }]
                    });
                    let _ = write!(result, "data: {}\n\n", event);
                }
            }

            // Regular text — flush tools first, then emit text
            if let Some(content) = delta["content"].as_str() {
                flush_tools(&mut result, &mut pending_tools, &mut has_pending_tools);
                let event = serde_json::json!({
                    "candidates": [{
                        "content": {
                            "parts": [{"text": content}],
                            "role": "model"
                        },
                        "finishReason": null,
                        "safetyRatings": []
                    }]
                });
                let _ = write!(result, "data: {}\n\n", event);
            }
        }

        result
    }
}

fn convert_openai_sse_to_gemini_sse(openai_sse: &str) -> String {
    use std::collections::HashMap;
    let mut result = String::new();
    // Track accumulating tool calls: index → (name, accumulated_args)
    let mut pending_tools: HashMap<usize, (String, String)> = HashMap::new();
    let mut has_pending_tools = false;
    // Same finishReason contract as make_gemini_sse_converter: gemini-cli treats a
    // stream that ends without finishReason as InvalidStreamError and RETRIES the
    // whole request, duplicating the reply. Emit a synthesized finishReason chunk
    // at stream end if the upstream never sent one.
    let mut sent_finish = false;

    let flush_tools = |result: &mut String, pending: &mut HashMap<usize, (String, String)>, has: &mut bool| {
        if !*has { return; }
        for (_, (name, args_str)) in pending.drain() {
            let args: Value = serde_json::from_str(&args_str).unwrap_or(Value::Null);
            result.push_str(&format!("data: {}\n\n", serde_json::json!({
                "candidates": [{
                    "content": {
                        "parts": [{"functionCall": {"name": name, "args": args}}],
                        "role": "model"
                    },
                    "finishReason": null,
                    "safetyRatings": []
                }]
            })));
        }
        *has = false;
    };

    for line in openai_sse.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if data == "[DONE]" {
                flush_tools(&mut result, &mut pending_tools, &mut has_pending_tools);
                // Upstream [DONE] carries no finishReason — synthesize one so the
                // stream is valid for gemini-cli (prevents request retries).
                if !sent_finish {
                    sent_finish = true;
                    result.push_str(&format!("data: {}\n\n", serde_json::json!({
                        "candidates": [{
                            "content": { "parts": [], "role": "model" },
                            "finishReason": "STOP",
                            "safetyRatings": []
                        }]
                    })));
                }
                result.push_str("data: {\"done\":true}\n\n");
                continue;
            }
            if let Ok(chunk) = serde_json::from_str::<Value>(data) {
                let delta = &chunk["choices"][0]["delta"];

                // Forward the upstream finish_reason as a Gemini finishReason chunk
                // (same contract as make_gemini_sse_converter).
                if let Some(fr) = chunk["choices"][0]["finish_reason"].as_str() {
                    if !fr.is_empty() && !sent_finish {
                        sent_finish = true;
                        let gemini_fr = if fr == "length" { "MAX_TOKENS" } else { "STOP" };
                        result.push_str(&format!("data: {}\n\n", serde_json::json!({
                            "candidates": [{
                                "content": { "parts": [], "role": "model" },
                                "finishReason": gemini_fr,
                                "safetyRatings": []
                            }]
                        })));
                    }
                }

                // Tool calls — accumulate for later emission
                if let Some(tcs) = delta["tool_calls"].as_array() {
                    has_pending_tools = true;
                    for tc in tcs {
                        let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                        let entry = pending_tools.entry(idx).or_insert_with(|| {
                            (tc["function"]["name"].as_str().unwrap_or("").to_string(), String::new())
                        });
                        if let Some(args) = tc["function"]["arguments"].as_str() {
                            entry.1.push_str(args);
                        }
                    }
                }

                // Reasoning — flush tools first, then emit thought.
                // Strip newlines: Gemini thinking content with line breaks can break the
                // UI rendering and downstream ACP processing.
                if let Some(reasoning) = delta["reasoning_content"].as_str() {
                    flush_tools(&mut result, &mut pending_tools, &mut has_pending_tools);
                    let reasoning_clean = reasoning.replace('\n', " ");
                    if !reasoning_clean.trim().is_empty() {
                        result.push_str(&format!("data: {}\n\n", serde_json::json!({
                            "candidates": [{
                                "content": {
                                    "parts": [{"text": reasoning_clean, "thought": true}],
                                    "role": "model"
                                },
                                "finishReason": null,
                                "safetyRatings": []
                            }]
                        })));
                    }
                }

                // Regular text — flush tools first, then emit text
                if let Some(content) = delta["content"].as_str() {
                    flush_tools(&mut result, &mut pending_tools, &mut has_pending_tools);
                    result.push_str(&format!("data: {}\n\n", serde_json::json!({
                        "candidates": [{
                            "content": {
                                "parts": [{"text": content}],
                                "role": "model"
                            },
                            "finishReason": null,
                            "safetyRatings": []
                        }]
                    })));
                }
            }
        }
    }
    // Final flush at stream end
    flush_tools(&mut result, &mut pending_tools, &mut has_pending_tools);
    if result.is_empty() { result = openai_sse.to_string(); }
    result
}
