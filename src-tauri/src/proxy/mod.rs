//! Local HTTP proxy that translates between LLM API protocols.
//! Enables using any model provider with any Agent CLI.
//!
//! 模块布局（方案 B 工程化重构）：
//! - `usage`：全局 usage store（转换/透传路径写入，ACP client 读取）
//! - `common`：跨模块共享（模型路由 find_model、SSE 转换 reader、工具配对修复等）
//! - `passthrough`：原生直连透传（入口协议 == 上游协议时零转换转发）
//! - `convert::{anthropic,responses,gemini,openai}`：各入口协议 ↔ OpenAI Chat
//!
//! All proxy handlers support both sync and streaming modes:
//! - Sync: wait for full upstream response, convert, return.
//! - Stream: read upstream SSE line by line, convert on the fly, stream back.

mod common;
mod convert;
mod passthrough;
mod usage;

pub use usage::take_last_usage;

use crate::models_config::{ModelConfig, ModelEntry};
use crate::rjlog;
use crate::rjlogd;
use serde_json::Value;

use std::collections::HashMap;
use std::io::Read;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use tiny_http::{Header, Response, Server, StatusCode};

use common::{extract_model_from_path, find_model, safe_truncate, ProxyResponse};
use convert::anthropic::proxy_anthropic_to_openai;
use convert::gemini::proxy_gemini_to_openai;
use convert::openai::proxy_openai_direct;
use convert::responses::proxy_responses_to_openai;
use passthrough::{forward_anthropic_passthrough, forward_gemini_passthrough, forward_responses_passthrough};

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
        // 每个请求在独立线程处理：本地模型（llama.cpp）推理很慢，流式请求会
        // 占用线程长达数分钟。若所有请求在单线程里串行，一个本地模型会话会
        // 阻塞其他所有会话（如商业模型）的请求，造成"卡住等本地模型"。
        for request in server.incoming_requests() {
            let state = state.clone();
            thread::spawn(move || {
                let mut request = request;
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
            });
        }
    });

    Ok(port)
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

    rjlogd!("[PROXY] <<< body ({} chars) first 300: {}", body.len(), safe_truncate(&body, 300));

    // Compute request hash for cache debugging
    let body_hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        body.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    };
    rjlogd!("[CACHE DEBUG] Request hash: {} (path: {}, body_len: {})", body_hash, path, body.len());

    // Log message count and structure for debugging
    if let Ok(req_json) = serde_json::from_str::<Value>(&body) {
        let msg_count = req_json.get("messages").and_then(|m| m.as_array()).map(|arr| arr.len()).unwrap_or(0);
        let tools_count = req_json.get("tools").and_then(|t| t.as_array()).map(|arr| arr.len()).unwrap_or(0);
        let has_system = req_json.get("messages")
            .and_then(|m| m.as_array())
            .map(|arr| arr.iter().any(|msg| msg.get("role").and_then(|r| r.as_str()) == Some("system")))
            .unwrap_or(false);
        let stream = req_json.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
        rjlogd!("[CACHE DEBUG] Structure: {} messages, {} tools, system_prompt={}, stream={}", 
            msg_count, tools_count, has_system, stream);
        
        // Log message roles sequence for comparison
        if msg_count > 0 {
            let roles: Vec<String> = req_json.get("messages")
                .and_then(|m| m.as_array())
                .map(|arr| {
                    arr.iter().map(|msg| msg.get("role").and_then(|r| r.as_str()).unwrap_or("?").to_string()).collect()
                })
                .unwrap_or_default();
            rjlogd!("[CACHE DEBUG] Message roles: {:?}", roles);
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
    
    rjlogd!("[PROXY] Using {} models from config file", models.len());
    for m in &models {
        rjlogd!("[PROXY] Model: {} (id={}, support_tools={})", m.name, m.id, m.support_tools);
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

    // ── 原生直连优先 ──────────────────────────────────────────────────
    // 入口协议与所配上游模型的协议一致时（Anthropic 入口 + Anthropic 兼容上游、
    // Codex Responses 入口 + 提供 /responses 的上游、Gemini 入口 + Gemini 兼容
    // 上游），纯透传：body/SSE 原样转发，零协议转换。同协议上游本就输出 agent
    // 期望的格式，转换只会引入丢失/失真风险还白耗 CPU。usage 统计由
    // SseTapReader / 非流式解析保持不变。
    let req_model = match extract_model_from_path(path) {
        // Gemini 协议的 model 在 URL 路径里，其余入口在 body.model
        Some(gm) => gm.to_string(),
        None => serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(|s| s.to_string()))
            .unwrap_or_default(),
    };
    if !req_model.is_empty() {
        if let Some(m) = find_model(&models, &req_model, preferred_ref) {
            let is_anthropic_in = path == "/v1/messages" || path.ends_with("/v1/messages") || path.contains("/anthropic/v1/messages");
            let is_responses_in = path == "/responses" || path == "/v1/responses" || path.ends_with("/v1/responses");
            let is_gemini_in = extract_model_from_path(path).is_some();
            let passthrough = if is_anthropic_in && m.protocol == "anthropic" {
                Some(forward_anthropic_passthrough(m, &body, reasoning_disabled))
            } else if is_responses_in && m.protocol == "openai_responses" {
                Some(forward_responses_passthrough(m, &body, reasoning_disabled))
            } else if is_gemini_in && m.protocol == "gemini" {
                Some(forward_gemini_passthrough(m, &body, path, reasoning_disabled))
            } else {
                None
            };
            if let Some(resp) = passthrough {
                return resp;
            }
        }
    }

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

