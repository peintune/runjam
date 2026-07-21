//! Local HTTP proxy that translates between LLM API protocols.
//! Enables using any model provider with any Agent CLI.

use crate::models_config::{ModelConfig, ModelEntry};
use crate::rjlog;
use serde_json::Value;


use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use tiny_http::{Header, Response, Server, StatusCode};

pub struct ProxyState {
    pub port: u16,
    pub running: bool,
    pub models: Vec<ModelEntry>,
    /// Maps agent_id → model ids assigned to that agent (used to disambiguate
    /// models that share the same name; lookups prefer the agent's own model).
    pub agent_models: HashMap<String, Vec<String>>,
}

impl ProxyState {
    pub fn new() -> Self {
        Self { port: 0, running: false, models: vec![], agent_models: HashMap::new() }
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
            // Catch panics from handle_request so a single malformed request
            // doesn't kill the whole proxy thread.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let path = request.url().to_string();
                let method = request.method().as_str().to_string();
                handle_request(&path, &method, &mut request, &state)
            }));
            let (status, body) = match result {
                Ok(r) => r,
                Err(e) => {
                    let detail = e.downcast_ref::<String>()
                        .map(|s| s.as_str())
                        .or_else(|| e.downcast_ref::<&str>().copied())
                        .unwrap_or("");
                    rjlog!("[PROXY] PANIC in handler: {}", detail);
                    (StatusCode(500), format!(r#"{{"error":"Internal proxy error: {}"}}"#, detail))
                }
            };

            let response = Response::from_string(&body)
                .with_status_code(status)
                .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());

            // SSE streaming: Chat Completions uses "data:" prefix, Responses API uses "event:" prefix
            if body.starts_with("data:") || body.starts_with("event:") {
                let stream_response = Response::from_string(&body)
                    .with_status_code(200)
                    .with_header(Header::from_bytes("Content-Type", "text/event-stream").unwrap());
                request.respond(stream_response).ok();
            } else {
                request.respond(response).ok();
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
) -> (StatusCode, String) {
    rjlog!("[PROXY] >>> {} {} (from {}:{})", method, path, request.remote_addr().map(|a| a.to_string()).unwrap_or_default(), request.remote_addr().map(|a| a.port()).unwrap_or(0));
    if method != "POST" {
        if path == "/v1/models" || path == "/v1beta/models" {
            return (StatusCode(200), r#"{"object":"list","data":[]}"#.to_string());
        }
        return (StatusCode(405), "Method not allowed".to_string());
    }

    let body = {
        let mut buf = String::new();
        request.as_reader().read_to_string(&mut buf).ok();
        buf
    };

    rjlog!("[PROXY] <<< body ({} chars) first 300: {}", body.len(), &body[..body.len().min(300)]);

    // Reload models on each request so saved models take effect without restart
    {
        let mut s = state.lock().unwrap();
        s.models = ModelConfig::load().models;
    }
    let models = { state.lock().unwrap().models.clone() };

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

    // Anthropic Messages API → proxy (supports /anthropic/v1/messages prefix)
    if path == "/v1/messages" || path.ends_with("/v1/messages") || path.contains("/anthropic/v1/messages") {
        return proxy_anthropic_to_openai(&body, &models, preferred_ref);
    }

    // OpenAI Chat Completions → proxy
    if path == "/v1/chat/completions" || path.ends_with("/v1/chat/completions") {
        return proxy_openai_direct(&body, &models, preferred_ref);
    }

    // OpenAI Responses API → Chat Completions (Codex uses /responses)
    if path == "/responses" || path == "/v1/responses" || path.ends_with("/v1/responses") {
        return proxy_responses_to_openai(&body, &models, preferred_ref);
    }

    // Gemini GenerateContent API → proxy
    if (path.contains("/v1/") || path.contains("/v1beta/")) && (path.contains(":generateContent") || path.contains("/models/")) {
        return proxy_gemini_to_openai(&body, &models, &path, preferred_ref);
    }

    (StatusCode(404), "Not found".to_string())
}

fn proxy_anthropic_to_openai(body: &str, models: &[ModelEntry], preferred_ids: Option<&[String]>) -> (StatusCode, String) {
    let req: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (StatusCode(400), format!("Invalid JSON: {}", e)) };

    // Extract messages and model
    let model_name = req["model"].as_str().unwrap_or("claude-3-5-sonnet");
    let messages = &req["messages"];
    let system = req["system"].as_str();
    let max_tokens = req["max_tokens"].as_u64().unwrap_or(4096);
    let stream = req["stream"].as_bool().unwrap_or(false);

    // Find matching model in our config
    let target = find_model(models, model_name, preferred_ids);

    let (api_key, base_url, real_model) = if let Some(m) = target {
        (m.api_key.clone(), m.api_base.clone(), m.name.clone())
    } else {
        // No match — try to forward as-is to Anthropic
        return forward_to_anthropic(body);
    };

    // Build OpenAI-format request
    let mut openai_messages: Vec<Value> = vec![];
    if let Some(sys) = system {
        openai_messages.push(serde_json::json!({"role": "system", "content": sys}));
    }
    if let Some(msgs) = messages.as_array() {
        for m in msgs {
            let role = m["role"].as_str().unwrap_or("user");
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
                    // In OpenAI format, text blocks stay as user messages; tool_result
                    // blocks become separate "tool" role messages.
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
                    // In OpenAI format: content = concatenated text, tool_calls = array.
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
                    // system or other roles — just extract text
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

    // Convert Anthropic tools format → OpenAI tools format
    let mut openai_tools: Vec<Value> = vec![];
    if let Some(tools) = req.get("tools").and_then(|v| v.as_array()) {
        for tool in tools {
            let name = tool["name"].as_str().unwrap_or("");
            let description = tool["description"].as_str().unwrap_or("");
            let input_schema = &tool["input_schema"];
            openai_tools.push(serde_json::json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": description,
                    "parameters": input_schema
                }
            }));
        }
    }

    let mut openai_body = serde_json::json!({
        "model": real_model,
        "messages": openai_messages,
        "max_tokens": max_tokens,
        "stream": stream,
    });
    if !openai_tools.is_empty() {
        openai_body["tools"] = serde_json::json!(openai_tools);
        if let Some(tc) = req.get("tool_choice") {
            let converted_tc = convert_tool_choice_anthropic_to_openai(tc);
            rjlog!("[PROXY] Anthropic→OpenAI: tool_choice raw={:?}, converted={:?}", tc, converted_tc);
            openai_body["tool_choice"] = converted_tc;
        }
    }

    // Forward to OpenAI-compatible endpoint
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    rjlog!("[PROXY] Anthropic→OpenAI: POST {} model={} stream={} msgs={}", url, real_model, stream, openai_messages.len());

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build();
    let resp = agent.post(&url)
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_string(&openai_body.to_string());

    match resp {
        Ok(response) => {
            let status = response.status();
            let resp_body = response.into_string().unwrap_or_default();
            rjlog!("[PROXY] Anthropic→OpenAI: upstream response status={} body_len={}", status, resp_body.len());
            rjlog!("[PROXY] Anthropic→OpenAI: upstream body first 300: {}", &resp_body[..resp_body.len().min(300)]);
            if stream {
                // Convert OpenAI SSE → Anthropic SSE
                let converted = convert_openai_sse_to_claude_sse(&resp_body, model_name);
                rjlog!("[PROXY] Anthropic→OpenAI: converted SSE output {} chars, first 500: {}",
                    converted.len(), &converted[..converted.len().min(500)]);
                (StatusCode(200), converted)
            } else {
                // Convert OpenAI response → Anthropic format
                let converted = convert_openai_to_anthropic(&resp_body, model_name);
                rjlog!("[PROXY] Anthropic→OpenAI: converted non-stream output {} chars", converted.len());
                (StatusCode(200), converted)
            }
        }
        Err(ureq::Error::Status(st, r)) => {
            let body = r.into_string().unwrap_or_default();
            rjlog!("[PROXY] Anthropic→OpenAI: upstream HTTP {}: {}", st, &body[..body.len().min(500)]);
            let err_body = serde_json::json!({
                "type": "error",
                "error": {"type": "api_error", "message": format!("Upstream {}: {}", st, &body[..body.len().min(200)])}
            });
            (StatusCode(502), err_body.to_string())
        }
        Err(e) => {
            rjlog!("[PROXY] Anthropic→OpenAI: connection error: {:?}", e);
            let err_body = serde_json::json!({
                "type": "error",
                "error": {"type": "api_error", "message": format!("Proxy error: {}", e)}
            });
            (StatusCode(502), err_body.to_string())
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

fn proxy_openai_direct(body: &str, _models: &[ModelEntry], preferred_ids: Option<&[String]>) -> (StatusCode, String) {
    // For OpenAI requests, forward directly to the target
    let req: Value = match serde_json::from_str(body) { Ok(v) => v, Err(_) => return (StatusCode(400), "Invalid JSON".into()) };
    let model_name = req["model"].as_str().unwrap_or("gpt-4o");

    // Find matching model config
    let models = ModelConfig::load().models;
    let target = find_model(&models, model_name, preferred_ids);

    if let Some(m) = target {
        let url = format!("{}/chat/completions", m.api_base.trim_end_matches('/'));
        let resp = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", m.api_key))
            .set("Content-Type", "application/json")
            .send_string(body);
        match resp {
            Ok(r) => (StatusCode(200), r.into_string().unwrap_or_default()),
            Err(e) => (StatusCode(502), format!("Proxy error: {}", e)),
        }
    } else {
        (StatusCode(404), format!("Model {} not configured", model_name))
    }
}

/// Translate OpenAI Responses API → OpenAI Chat Completions.
/// Codex uses the Responses API (/responses), but most providers (DeepSeek, etc.)
/// only support Chat Completions (/v1/chat/completions).
fn proxy_responses_to_openai(body: &str, models: &[ModelEntry], preferred_ids: Option<&[String]>) -> (StatusCode, String) {
    let req: Value = match serde_json::from_str(body) { Ok(v) => v, Err(_) => return (StatusCode(400), "Invalid JSON".into()) };
    let model_name = req["model"].as_str().unwrap_or("");
    let stream = req["stream"].as_bool().unwrap_or(false);

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
                            "content": null,
                            "tool_calls": [{
                                "id": call_id,
                                "type": "function",
                                "function": {"name": name, "arguments": arguments}
                            }]
                        }));
                    }
                    "function_call_output" => {
                        let call_id = item["call_id"].as_str().unwrap_or("");
                        let output = item["output"].as_str().unwrap_or("");
                        msgs.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": output
                        }));
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
                                let text = arr.iter()
                                    .filter_map(|p| p["text"].as_str())
                                    .collect::<Vec<_>>()
                                    .join("");
                                Value::String(text)
                            } else {
                                c.clone()
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
        return (StatusCode(400), r#"{"error":"Missing model or input"}"#.into());
    }

    // Find matching model in config
    let target = find_model(models, model_name, preferred_ids);
    let (api_key, base_url, real_model) = if let Some(m) = target {
        let masked_key = if m.api_key.len() > 8 {
            format!("{}...{}", &m.api_key[..4], &m.api_key[m.api_key.len()-4..])
        } else { m.api_key.clone() };
        rjlog!("[PROXY] Responses→Chat: Found model '{}' api_key={} base_url={}", m.name, masked_key, m.api_base);
        (m.api_key.clone(), m.api_base.clone(), m.name.clone())
    } else {
        rjlog!("[PROXY] Responses→Chat: Model '{}' NOT FOUND in {} models. Available: {:?}",
            model_name, models.len(),
            models.iter().map(|m| format!("{}({})", m.name, m.id)).collect::<Vec<_>>());
        return (StatusCode(404), format!(r#"{{"error":"Model {} not configured"}}"#, model_name));
    };

    // Build Chat Completions request (convert Responses API tool format to Chat format)
    let mut chat_body = serde_json::json!({
        "model": real_model,
        "messages": messages,
        "stream": stream,
    });
    if let Some(tools) = req.get("tools").and_then(|v| v.as_array()) {
        // Responses API tools: {type: "function", name, description, parameters}
        //                          OR {type: "namespace", namespace: ...} etc.
        // Chat Completions tools: only {type: "function", function: {name, description, parameters}}
        let chat_tools: Vec<Value> = tools.iter()
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
        chat_body["tools"] = serde_json::Value::Array(chat_tools);
        if let Some(tc) = req.get("tool_choice") {
            chat_body["tool_choice"] = tc.clone();
        }
    }

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    rjlog!("[PROXY] Responses→Chat: POST {} (model={}, stream={})", url, real_model, stream);

    let request_body = chat_body.to_string();
    rjlog!("[PROXY] Responses→Chat: body ({} chars) — model:{} messages:{}",
        request_body.len(), real_model, messages.len());

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build();
    let resp = agent.post(&url)
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_string(&request_body);

    match resp {
        Ok(r) => {
            let status = r.status();
            rjlog!("[PROXY] Upstream response status: {} (stream={})", status, stream);
            match r.into_string() {
                Ok(resp_body) => {
                    rjlog!("[PROXY] Upstream response body: {} chars. First 300 chars: {}",
                        resp_body.len(), &resp_body[..resp_body.len().min(300)]);
                    if status >= 400 {
                        rjlog!("[PROXY] Upstream error ({} chars): {}", resp_body.len(), &resp_body[..resp_body.len().min(1000)]);
                        return (StatusCode(status), resp_body);
                    }
                    // Convert Chat Completions → Responses API format
                    if stream {
                        rjlog!("[PROXY] Converting SSE stream ({} chars) to Responses format", resp_body.len());
                        let converted = convert_chat_sse_to_responses_sse(&resp_body, &real_model);
                        rjlog!("[PROXY] Converted SSE output: {} chars. First 500 chars: {}", converted.len(), &converted[..converted.len().min(500)]);
                        (StatusCode(200), converted)
                    } else {
                        rjlog!("[PROXY] Converting non-stream response to Responses format");
                        let converted = convert_chat_to_responses(&resp_body, &real_model, false);
                        rjlog!("[PROXY] Converted non-stream output: {} chars", converted.len());
                        (StatusCode(200), converted)
                    }
                }
                Err(e) => {
                    rjlog!("[PROXY] Failed to read response body: {}", e);
                    (StatusCode(502), format!(r#"{{"error":"Failed to read response: {}"}}"#, e))
                }
            }
        }
        Err(ureq::Error::Status(status, r)) => {
            let body = r.into_string().unwrap_or_default();
            rjlog!("[PROXY] Upstream HTTP {}: {}", status, &body[..body.len().min(1000)]);
            (StatusCode(502), format!(r#"{{"error":"Upstream {}: {}"}}"#, status, body))
        }
        Err(e) => {
            rjlog!("[PROXY] Connection error: {:?}", e);
            (StatusCode(502), format!(r#"{{"error":"Proxy error: {}"}}"#, e))
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
    let input_tokens = resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
    let output_tokens = resp["usage"]["completion_tokens"].as_u64().unwrap_or(0);
    let total_tokens = resp["usage"]["total_tokens"].as_u64().unwrap_or(input_tokens + output_tokens);

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
                "id": format!("fc_{}", chrono::Utc::now().timestamp_nanos()),
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
        }
    }).to_string()
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
                    input_tokens = usage["prompt_tokens"].as_u64().unwrap_or(input_tokens);
                    output_tokens = usage["completion_tokens"].as_u64().unwrap_or(output_tokens);
                    total_tokens = usage["total_tokens"].as_u64().unwrap_or(total_tokens);
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
        serde_json::json!({"type":"response.completed","response":{"id":response_id,"object":"response","model":model,"status":"completed","output":output_items,"usage":{"input_tokens":input_tokens,"output_tokens":output_tokens,"total_tokens":total_tokens}}})
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

fn proxy_gemini_to_openai(body: &str, models: &[ModelEntry], path: &str, preferred_ids: Option<&[String]>) -> (StatusCode, String) {
    let req: Value = match serde_json::from_str(body) { Ok(v) => v, Err(e) => return (StatusCode(400), format!("Invalid JSON: {}", e)) };

    let model_name = extract_model_from_path(path).unwrap_or("gemini-1.5-pro");

    rjlog!("[PROXY] Gemini: {} (body {} chars)", path, body.len());

    // handle non-generateContent calls (countTokens, embedContent, etc.)
    let is_count_tokens = path.contains(":countTokens");
    let is_stream_generate = path.contains(":streamGenerateContent");
    if is_count_tokens {
        // Return a dummy count — Gemini only needs a plausible number
        return (StatusCode(200), serde_json::json!({"totalTokens": 0}).to_string());
    }
    // streamGenerateContent is always streaming; generateContent may or may not be
    let stream = is_stream_generate || req.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    // Use safe .get() — Gemini may also send list models / other requests
    let Some(contents) = req.get("contents") else {
        return (StatusCode(200), "{}".to_string());
    };
    
    let max_output_tokens = if let Some(config) = req.get("generationConfig").and_then(|v| v.as_object()) {
        config.get("maxOutputTokens").and_then(|v| v.as_u64()).unwrap_or(4096)
    } else {
        4096
    };

    let target = find_model(models, model_name, preferred_ids);

    let (api_key, base_url, real_model) = if let Some(m) = target {
        (m.api_key.clone(), m.api_base.clone(), m.name.clone())
    } else {
        return forward_to_gemini(body, path);
    };

    let mut openai_messages: Vec<Value> = vec![];
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
                        // Gemini functionCall has no id, synthesize one
                        let call_id = format!("gc_{}_{}", ci, pi);
                        tool_calls.push(serde_json::json!({
                            "id": call_id,
                            "type": "function",
                            "function": {"name": name, "arguments": args}
                        }));
                    } else if let Some(fr) = part.get("functionResponse") {
                        let name = fr["name"].as_str().unwrap_or("");
                        let response = fr["response"].to_string();
                        // Use function name as key to match with the call
                        let call_id = format!("gc_{}", name);
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
    if !openai_tools.is_empty() {
        openai_body["tools"] = serde_json::json!(openai_tools);
    }

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_string(&openai_body.to_string());

    match resp {
        Ok(response) => {
            let resp_body = response.into_string().unwrap_or_default();
            if stream {
                let converted = convert_openai_sse_to_gemini_sse(&resp_body);
                (StatusCode(200), converted)
            } else {
                let converted = convert_openai_to_gemini(&resp_body);
                (StatusCode(200), converted)
            }
        }
        Err(e) => {
            let err_body = serde_json::json!({
                "error": {"code": 502, "message": format!("Proxy error: {}", e)}
            });
            (StatusCode(502), err_body.to_string())
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
        parts.push(serde_json::json!({"text": reasoning_content, "thought": true}));
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

fn convert_openai_sse_to_gemini_sse(openai_sse: &str) -> String {
    use std::collections::HashMap;
    let mut result = String::new();
    // Track accumulating tool calls: index → (name, accumulated_args)
    let mut pending_tools: HashMap<usize, (String, String)> = HashMap::new();
    let mut has_pending_tools = false;

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
                result.push_str("data: {\"done\":true}\n\n");
                continue;
            }
            if let Ok(chunk) = serde_json::from_str::<Value>(data) {
                let delta = &chunk["choices"][0]["delta"];

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
                    result.push_str(&format!("data: {}\n\n", serde_json::json!({
                        "candidates": [{
                            "content": {
                                "parts": [{"text": reasoning, "thought": true}],
                                "role": "model"
                            },
                            "finishReason": null,
                            "safetyRatings": []
                        }]
                    })));
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
