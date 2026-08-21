//! Anthropic Messages 入口 ↔ OpenAI Chat Completions 出口的双向转换。
//! Claude Code 说 Anthropic 协议，绝大多数第三方模型只提供 OpenAI 兼容端点。

use crate::models_config::ModelEntry;
use crate::rjlog;
use crate::rjlogd;
use serde_json::Value;
use std::io::{BufReader, Read, Write};

use crate::proxy::common::{build_agent, ensure_tool_calls_paired, find_model, limit_tools_for_llama, safe_truncate, LLAMA_MAX_TOOLS, ProxyResponse, SseStreamConverter};
use crate::proxy::usage::store_usage_for_latest;
use tiny_http::StatusCode;

pub(crate) fn proxy_anthropic_to_openai(body: &str, models: &[ModelEntry], preferred_ids: Option<&[String]>, reasoning_disabled: bool) -> ProxyResponse {
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
    rjlogd!("[CACHE DEBUG] Raw request: system_field={}, system_type={}", has_system_field, system_field_type);
    
    // Debug: check messages structure
    if let Some(msgs) = messages.as_array() {
        if let Some(first_msg) = msgs.first() {
            let first_role = first_msg["role"].as_str().unwrap_or("unknown");
            let content_type = if first_msg["content"].is_string() { "string" }
                else if first_msg["content"].is_array() { "array" }
                else { "other/none" };
            let content_len = first_msg["content"].as_str().map(|s| s.len()).unwrap_or(0);
            rjlogd!("[CACHE DEBUG] First message: role={}, content_type={}, content_len={}", first_role, content_type, content_len);
            
            // Log first 100 chars of system message if it exists
            if first_role == "system" {
                if let Some(content) = first_msg["content"].as_str() {
                    rjlogd!("[CACHE DEBUG] System content (first 150): {:?}", safe_truncate(content, 150));
                } else if let Some(arr) = first_msg["content"].as_array() {
                    rjlogd!("[CACHE DEBUG] System content is array with {} blocks", arr.len());
                    for (i, block) in arr.iter().enumerate().take(3) {
                        let block_type = block["type"].as_str().unwrap_or("unknown");
                        if let Some(text) = block["text"].as_str() {
                            rjlogd!("[CACHE DEBUG] System block {}: type={}, text (first 80): {:?}", i, block_type, safe_truncate(text, 80));
                        } else {
                            rjlogd!("[CACHE DEBUG] System block {}: type={}", i, block_type);
                        }
                    }
                }
            }
        }
    }
    
    let mut system = req["system"].as_str().map(|s| s.to_string());
    
    // Handle array-type system prompt (multiple system messages)
    if system.is_none() && req["system"].is_array() {
        rjlogd!("[CACHE DEBUG] system field is array, extracting...");
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
                rjlogd!("[CACHE DEBUG] Extracted system prompt from array ({} items)", system_texts.len());
            }
        }
    }
    
    // Also check if there's a system message in the messages array (newer API format)
    if system.is_none() {
        rjlogd!("[CACHE DEBUG] No system field, searching messages array for system message...");
        if let Some(msgs) = messages.as_array() {
            // Search all messages for system role (not just the first one)
            for msg in msgs {
                if msg["role"].as_str() == Some("system") {
                    rjlogd!("[CACHE DEBUG] Found system message in array, extracting...");
                    if let Some(content) = msg["content"].as_str() {
                        system = Some(content.to_string());
                        rjlogd!("[CACHE DEBUG] Extracted system prompt from messages array (string)");
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
                            rjlogd!("[CACHE DEBUG] Extracted system prompt from messages array ({} text blocks)", text_parts.len());
                            break;
                        }
                    }
                }
            }
            if system.is_none() {
                rjlogd!("[CACHE DEBUG] No system message found in messages array");
            }
        }
    }
    
    let is_llama_cpp_check = req["model"].as_str().map(|m| m.contains("llama-") || m.ends_with(".gguf")).unwrap_or(false);
    let mut max_tokens = req["max_tokens"].as_u64().unwrap_or(if is_llama_cpp_check { 2048 } else { 4096 });
    // llama.cpp 本地模型推理慢（尤其长上下文），请求方可能传 32000 这类巨大值，
    // 在慢速推理下会导致请求长时间挂起、会话"无数据返回"。统一限制上限。
    if is_llama_cpp_check && max_tokens > 4096 {
        rjlog!("[PROXY] llama_cpp: reduced max_tokens from {} to 4096", max_tokens);
        max_tokens = 4096;
    }
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
    rjlogd!("[CACHE DEBUG] Model resolution: requested={}, real_model={}, provider={}, base_url={}", 
        model_name, real_model, provider, base_url);
    
    // Log system prompt prefix for comparison
    if let Some(ref sys) = system {
        rjlogd!("[CACHE DEBUG] System prompt (first 100 chars): {:?}", safe_truncate(sys, 100));
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
                rjlogd!("[CACHE DEBUG] Skipping system message in array (already extracted as system prompt)");
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

    // Safety net: ensure every tool_calls message has a paired tool response.
    let mut openai_messages = ensure_tool_calls_paired(openai_messages);

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
                // input_schema 缺失/null 时用空对象 {}（OpenAI 兼容端点不接受 null）
                let mut params = tool.get("input_schema").cloned().unwrap_or(serde_json::Value::Null);
                if params.is_null() {
                    params = serde_json::json!({"type": "object", "properties": {}});
                }
                params
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
    rjlogd!("[CACHE DEBUG] Outbound request: model={}, messages={}, tools={}, stream={}", 
        real_model, openai_messages.len(), openai_tools.len(), stream);
    if let Some(sys_msg) = openai_messages.first() {
        if sys_msg["role"].as_str() == Some("system") {
            let sys_content = sys_msg["content"].as_str().unwrap_or("");
            rjlogd!("[CACHE DEBUG] Outbound system prompt (first 100): {:?}", safe_truncate(sys_content, 100));
        }
    }
    if !openai_tools.is_empty() && support_tools {
        // llama.cpp 本地模型：工具定义过多会拖慢 prefill 与 grammar 解析，
        // 裁剪到少量核心工具（LLAMA_MAX_TOOLS），优先保留 read/write/edit/bash 等。
        let limited_tools = if is_llama_cpp && openai_tools.len() > LLAMA_MAX_TOOLS {
            rjlog!("[PROXY] Anthropic→OpenAI: limiting tools from {} to {} for llama.cpp", openai_tools.len(), LLAMA_MAX_TOOLS);
            limit_tools_for_llama(&openai_tools, LLAMA_MAX_TOOLS)
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

    // llama.cpp 本地模型性能修复：
    // 1) Claude Code 的系统提示词高达 ~20K tokens，CPU 推理 prefill 需要数十分钟，
    //    导致请求必然超时。对本地模型替换为简短助手提示词，prompt 降到 ~1K tokens。
    // 2) Qwen3 等模型默认开启思考模式（先输出 reasoning 再回答），会消耗大量
    //    max_tokens 并拖慢响应。通过 chat_template_kwargs 关闭。
    if is_llama_cpp {
        if let Some(first) = openai_messages.first_mut() {
            if first["role"].as_str() == Some("system") {
                let orig_len = first["content"].as_str().map(|s| s.len()).unwrap_or(0);
                first["content"] = serde_json::json!("You are a helpful AI assistant running locally on the user's machine. You have access to the tools listed below. If a user request can be accomplished with a tool, call the tool with valid JSON arguments. Otherwise answer the user directly and concisely.");
                rjlog!("[PROXY] llama_cpp: replaced Claude Code system prompt ({} chars) with compact local prompt", orig_len);
            }
        }
        openai_body["chat_template_kwargs"] = serde_json::json!({"enable_thinking": false});
    }

    if reasoning_disabled {
        // 只发送 OpenAI 标准参数。thinking/reasoning_effort/enable_thinking 是
        // Anthropic/DeepSeek 扩展，OpenAI 兼容端点（如火山引擎）会报 400。
        openai_body["temperature"] = serde_json::json!(0.6);
    }

    // Forward to OpenAI-compatible endpoint
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let request_body_str = openai_body.to_string();
    let body_preview: String = safe_truncate(&request_body_str, 500).to_string();
    rjlog!("[PROXY] Anthropic→OpenAI: POST {} model={} stream={} msgs={} max_tokens={} body_len={}", 
        url, real_model, stream, openai_messages.len(), max_tokens, request_body_str.len());
    rjlogd!("[PROXY] Request body preview: {}...", body_preview);

    let agent = build_agent();
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
                    Box::new(make_anthropic_sse_converter(model_name.to_string(), reasoning_disabled)),
                );
                rjlog!("[PROXY] Anthropic→OpenAI: returning streaming response");
                ProxyResponse::Stream { reader: Box::new(converter) }
            } else {
                let resp_body = response.into_string().unwrap_or_default();
                // Log upstream response model for cache debugging
                if let Ok(resp_json) = serde_json::from_str::<Value>(&resp_body) {
                    let upstream_model = resp_json.get("model").and_then(|v| v.as_str()).unwrap_or("unknown");
                    rjlogd!("[CACHE DEBUG] Upstream response model: {} (requested: {})", upstream_model, real_model);
                    if let Some(usage) = resp_json.get("usage") {
                        rjlogd!("[CACHE DEBUG] Upstream usage: {}", serde_json::to_string(usage).unwrap_or_default());
                    }
                }
                let converted = convert_openai_to_anthropic(&resp_body, model_name, reasoning_disabled);
                ProxyResponse::Sync(StatusCode(200), converted)
            }
        }
        Err(ureq::Error::Status(st, r)) => {
            let body = r.into_string().unwrap_or_default();
            rjlog!("[PROXY] Anthropic→OpenAI: upstream HTTP {}: {}", st, safe_truncate(&body, 500));
            // Pass 4xx through so the agent fails fast instead of running
            // its own retry loop on a permanent error (bad model name → endless
            // upstream 404s while the UI shows an infinite spinner).
            // 错误类型按 Anthropic API 规范映射：Claude Code 等客户端按
            // error.type 分类错误。401 若标成 invalid_request_error，客户端
            // 不识别为认证失败，会当作瞬时错误反复重试（日志中连续多次 401），
            // 前端期间收不到任何事件只能等超时。标成 authentication_error 后
            // 客户端立即停止并上报，错误秒级显示。
            let resp_status = if (400..500).contains(&st) { StatusCode(st) } else { StatusCode(502) };
            let err_type = match st {
                401 => "authentication_error",
                403 => "permission_error",
                404 => "not_found_error",
                429 => "rate_limit_error",
                s if (400..500).contains(&s) => "invalid_request_error",
                _ => "api_error",
            };
            let err_body = serde_json::json!({
                "type": "error",
                "error": {"type": err_type, "message": format!("Upstream {}: {}", st, safe_truncate(&body, 200))}
            });
            rjlog!("[PROXY] Anthropic→OpenAI: returning HTTP {} (type={}) to agent", resp_status.0, err_type);
            ProxyResponse::Sync(resp_status, err_body.to_string())
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

fn forward_to_anthropic(body: &str) -> (StatusCode, String) {
    let req: Value = match serde_json::from_str(body) { Ok(v) => v, Err(_) => return (StatusCode(400), "Invalid JSON".into()) };
    let model = req["model"].as_str().unwrap_or("claude-3-5-sonnet");
    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    let base = std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".into());

    let url = format!("{}/v1/messages", base.trim_end_matches('/'));
    rjlog!("[PROXY] Anthropic direct: POST {} (model={})", url, model);
    let resp = build_agent()
        .post(&url)
        .set("x-api-key", &api_key)
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .send_string(body);

    match resp {
        Ok(r) => (StatusCode(200), r.into_string().unwrap_or_default()),
        Err(e) => (StatusCode(502), format!("Forward error: {}", e)),
    }
}

fn convert_openai_to_anthropic(openai_resp: &str, model_name: &str, reasoning_disabled: bool) -> String {
    let resp: Value = match serde_json::from_str(openai_resp) { Ok(v) => v, Err(_) => return openai_resp.to_string() };
    let choice = &resp["choices"][0];
    let reasoning_content = choice["message"].get("reasoning_content").and_then(|v| v.as_str()).unwrap_or("");
    let content = choice["message"]["content"].as_str().unwrap_or("");
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("stop");
    let input_tokens = resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
    let output_tokens = resp["usage"]["completion_tokens"].as_u64().unwrap_or(0);

    let mut content_blocks: Vec<Value> = vec![];
    // reasoning_disabled 时剥离思考内容，不生成 thinking block（否则即使关闭
    // reasoning，思考型模型输出的 reasoning_content 仍会以 thought 形式显示）
    if !reasoning_disabled && !reasoning_content.is_empty() {
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
/// `reasoning_disabled` 为 true 时剥离上游 reasoning_content，不生成 thinking block。
pub(crate) fn make_anthropic_sse_converter(model_name: String, reasoning_disabled: bool) -> impl FnMut(&str) -> Vec<u8> {
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

            rjlogd!("[CACHE DEBUG] SSE DONE: input_tokens={}, output_tokens={}, cached_tokens={}", input_tokens, output_tokens, cached_tokens);
            
            // 存储 usage 数据到全局存储，供 ACP Client 读取
            let _ = store_usage_for_latest(
                model_name.clone(),
                input_tokens as i64,
                output_tokens as i64,
                cached_tokens as i64,
            );
            rjlogd!("[CACHE DEBUG] Stored usage to global store: model={}, input={}, output={}, cached={}", 
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
                rjlogd!("[CACHE DEBUG] Anthropic SSE chunk #{} (FULL): {}", chunk_counter, data);
            } else {
                rjlogd!("[CACHE DEBUG] Anthropic SSE chunk #{}: data={}", chunk_counter, safe_truncate(data, 300));
            }
        }

        if let Ok(chunk) = serde_json::from_str::<Value>(data) {
            // Log all top-level keys for debugging
            if data.contains("usage") {
                let keys = chunk.as_object().map(|o| o.keys().collect::<Vec<_>>());
                rjlogd!("[CACHE DEBUG] Chunk keys: {:?}", keys);
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
            // reasoning_disabled 时剥离思考内容（不生成 thinking block）
            if !reasoning_disabled {
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


#[cfg(test)]
mod tests {
    use super::*;

    // ── tool_choice：Anthropic → OpenAI 语义映射 ──

    #[test]
    fn test_convert_tool_choice_variants() {
        // 字符串形态
        assert_eq!(convert_tool_choice_anthropic_to_openai(&serde_json::json!("auto")), serde_json::json!("auto"));
        assert_eq!(convert_tool_choice_anthropic_to_openai(&serde_json::json!("any")), serde_json::json!("required"));
        // 对象形态（DeepSeek 等只解析字符串或 function 对象，{type:"auto"} 会 400）
        assert_eq!(convert_tool_choice_anthropic_to_openai(&serde_json::json!({"type": "auto"})), serde_json::json!("auto"));
        assert_eq!(convert_tool_choice_anthropic_to_openai(&serde_json::json!({"type": "any"})), serde_json::json!("required"));
        assert_eq!(
            convert_tool_choice_anthropic_to_openai(&serde_json::json!({"type": "tool", "name": "get_weather"})),
            serde_json::json!({"type": "function", "function": {"name": "get_weather"}})
        );
        // 无 name 的 tool 对象：原样直通（不猜测）
        let passthrough = serde_json::json!({"type": "tool"});
        assert_eq!(convert_tool_choice_anthropic_to_openai(&passthrough), passthrough);
    }

    // ── convert_openai_to_anthropic：OpenAI 响应 → Anthropic 消息 ──

    #[test]
    fn test_convert_openai_to_anthropic_full() {
        let chat = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "final answer",
                    "reasoning_content": "thinking...",
                    "tool_calls": [{
                        "id": "call_9",
                        "type": "function",
                        "function": {"name": "read_file", "arguments": "{\"path\":\"/tmp/x\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 11, "completion_tokens": 22}
        }).to_string();
        let out = convert_openai_to_anthropic(&chat, "deepseek-v4", false);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["type"], "message");
        assert_eq!(v["stop_reason"], "tool_use", "tool_calls 结束必须映射为 tool_use");
        // 内容块顺序：thinking → text → tool_use
        let blocks = v["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "thinking");
        assert_eq!(blocks[1]["type"], "text");
        assert_eq!(blocks[2]["type"], "tool_use");
        assert_eq!(blocks[2]["id"], "call_9");
        // arguments 字符串必须被解析为 JSON 对象
        assert_eq!(blocks[2]["input"]["path"], "/tmp/x");
        assert_eq!(v["usage"]["input_tokens"], 11);
        assert_eq!(v["usage"]["output_tokens"], 22);
    }

    #[test]
    fn test_convert_openai_to_anthropic_plain_stop() {
        let chat = serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2}
        }).to_string();
        let v: Value = serde_json::from_str(&convert_openai_to_anthropic(&chat, "m", false)).unwrap();
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["content"][0]["type"], "text");
        assert_eq!(v["content"][0]["text"], "hi");
    }

    #[test]
    fn test_convert_openai_to_anthropic_reasoning_disabled_strips_thinking() {
        let chat = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "final answer",
                    "reasoning_content": "internal chain-of-thought"
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 10}
        }).to_string();
        // reasoning_disabled=true：必须剥离 thinking block
        let out = convert_openai_to_anthropic(&chat, "m", true);
        let v: Value = serde_json::from_str(&out).unwrap();
        let blocks = v["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1, "reasoning_disabled 时不能有 thinking block");
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "final answer");
        // reasoning_disabled=false：正常输出 thinking block
        let out2 = convert_openai_to_anthropic(&chat, "m", false);
        let v2: Value = serde_json::from_str(&out2).unwrap();
        let blocks2 = v2["content"].as_array().unwrap();
        assert_eq!(blocks2[0]["type"], "thinking");
        assert_eq!(blocks2[0]["thinking"], "internal chain-of-thought");
    }
}
