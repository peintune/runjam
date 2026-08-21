//! Gemini generateContent 入口 ↔ OpenAI Chat Completions 出口的双向转换。
//! Gemini CLI 只说 Gemini 协议（新版已移除 openAiCompatProvider 实验特性）。

use crate::models_config::ModelEntry;
use crate::rjlog;
use serde_json::Value;
use std::io::{BufReader, Read, Write};

use crate::proxy::common::{build_agent, extract_model_from_path, find_model, limit_tools_for_llama, LLAMA_MAX_TOOLS, ProxyResponse, SseStreamConverter};
use crate::proxy::usage::store_usage_for_latest;
use tiny_http::StatusCode;

pub(crate) fn proxy_gemini_to_openai(body: &str, models: &[ModelEntry], path: &str, preferred_ids: Option<&[String]>, reasoning_disabled: bool) -> ProxyResponse {
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

    let (api_key, base_url, real_model, support_tools) = if let Some(m) = target {
        rjlog!("[PROXY] Gemini→OpenAI: model '{}' resolved id={} provider={} base_url={}", m.name, m.id, m.provider, m.api_base);
        (m.api_key.clone(), m.api_base.clone(), m.name.clone(), m.support_tools)
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
                    // Gemini 的 functionDeclaration 可能没有 parameters 字段，
                    // 缺失/null 时用空对象 {}（OpenAI 兼容端点如火山引擎不接受 null）
                    let mut params = decl.get("parameters").cloned().unwrap_or(serde_json::Value::Null);
                    if params.is_null() {
                        params = serde_json::json!({"type": "object", "properties": {}});
                    }
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

    // llama.cpp 本地模型：工具定义过多会拖慢 prefill 与 grammar 解析，裁剪到核心工具
    let is_llama_cpp = real_model.contains("llama-") || real_model.ends_with(".gguf");
    if is_llama_cpp && openai_tools.len() > LLAMA_MAX_TOOLS {
        rjlog!("[PROXY] Gemini→OpenAI: limiting tools from {} to {} for llama.cpp", openai_tools.len(), LLAMA_MAX_TOOLS);
        openai_tools = limit_tools_for_llama(&openai_tools, LLAMA_MAX_TOOLS);
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
        // 只发送 OpenAI 标准参数。thinking/reasoning_effort/enable_thinking 是
        // Anthropic/DeepSeek 扩展，OpenAI 兼容端点（如火山引擎）会报 400。
        openai_body["temperature"] = serde_json::json!(0.6);
    }

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    rjlog!("[PROXY] Gemini→OpenAI: POST {} (model={}, stream={})", url, real_model, stream);
    let body_str = openai_body.to_string();
    // 结构化诊断：非 messages 的顶层键 + 每条消息的 role/content 类型/tool_calls
    // （messages 正文可能很大，只打结构不打内容）
    if let Some(obj) = openai_body.as_object() {
        for (k, v) in obj.iter() {
            if k == "messages" { continue; }
            rjlog!("[PROXY] Gemini→OpenAI: body[{}]={}", k, v);
        }
    }
    for (i, m) in openai_messages.iter().enumerate() {
        let role = m["role"].as_str().unwrap_or("?");
        let content_desc = match m.get("content") {
            Some(c) if c.is_null() => "null".to_string(),
            Some(c) if c.is_string() => format!("str({})", c.as_str().unwrap().len()),
            Some(c) if c.is_array() => format!("array({})", c.as_array().unwrap().len()),
            Some(_) => "other".to_string(),
            None => "missing".to_string(),
        };
        let has_tc = m.get("tool_calls").is_some() || m.get("tool_call_id").is_some();
        rjlog!("[PROXY] Gemini→OpenAI: msg[{}] role={} content={} tool_calls={}", i, role, content_desc, has_tc);
    }
    rjlog!("[PROXY] Gemini→OpenAI: body_len={}", body_str.len());
    let resp = build_agent()
        .post(&url)
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
                    Box::new(make_gemini_sse_converter(model_name.to_string(), reasoning_disabled)),
                );
                rjlog!("[PROXY] Gemini: returning streaming response");
                ProxyResponse::Stream { reader: Box::new(converter) }
            } else {
                let resp_body = response.into_string().unwrap_or_default();
                let converted = convert_openai_to_gemini(&resp_body, reasoning_disabled);
                ProxyResponse::Sync(StatusCode(200), converted)
            }
        }
        Err(e) => {
            // Surface the upstream's actual response body — a bare "status code
            // 400" says nothing about WHICH field the API rejected.
            let (detail, upstream_code) = match e {
                ureq::Error::Status(code, resp) => {
                    let body = resp.into_string().unwrap_or_default();
                    rjlog!("[PROXY] Gemini upstream returned {}: {}", code, body.chars().take(500).collect::<String>());
                    (format!("status code {}: {}", code, body.chars().take(300).collect::<String>()), Some(code))
                }
                other => (other.to_string(), None),
            };
            // Pass 4xx status through so the agent fails fast instead of
            // retrying a permanent error (bad model name → endless 404s).
            let status = match upstream_code {
                Some(code) if (400..500).contains(&code) => StatusCode(code),
                _ => StatusCode(502),
            };
            let err_body = serde_json::json!({
                "error": {"code": status.0, "message": format!("Proxy error: {}", detail)}
            });
            ProxyResponse::Sync(status, err_body.to_string())
        }
    }
}

fn forward_to_gemini(body: &str, path: &str) -> (StatusCode, String) {
    let api_key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
    let base = std::env::var("GOOGLE_GEMINI_BASE_URL").unwrap_or_else(|_| "https://generativelanguage.googleapis.com".into());
    
    let url = format!("{}{}", base, path);
    rjlog!("[PROXY] Gemini direct: POST {} (model={})", url, extract_model_from_path(path).unwrap_or("?"));
    let resp = build_agent()
        .post(&url)
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_string(body);

    match resp {
        Ok(r) => (StatusCode(200), r.into_string().unwrap_or_default()),
        Err(e) => (StatusCode(502), format!("Forward error: {}", e)),
    }
}

fn convert_openai_to_gemini(openai_resp: &str, reasoning_disabled: bool) -> String {
    let resp: Value = match serde_json::from_str(openai_resp) { Ok(v) => v, Err(_) => return openai_resp.to_string() };
    let choice = &resp["choices"][0];
    let reasoning_content = choice["message"].get("reasoning_content").and_then(|v| v.as_str()).unwrap_or("");
    let content = choice["message"]["content"].as_str().unwrap_or("");

    let mut parts: Vec<Value> = vec![];
    // reasoning_disabled 时剥离思考内容（不生成 thought part）
    if !reasoning_disabled && !reasoning_content.is_empty() {
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
/// `reasoning_disabled` 为 true 时剥离 reasoning_content，不生成 thought part。
fn make_gemini_sse_converter(model: String, reasoning_disabled: bool) -> impl FnMut(&str) -> Vec<u8> {
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
            // reasoning_disabled 时剥离思考内容（不生成 thought part）
            if !reasoning_disabled {
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


#[cfg(test)]
mod tests {
    use super::*;

    // ── convert_openai_to_gemini：OpenAI 响应 → Gemini 结构 ──

    #[test]
    fn test_convert_openai_to_gemini_full() {
        let chat = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "answer",
                    "reasoning_content": "think think",
                    "tool_calls": [{
                        "id": "call_1",
                        "function": {"name": "list_dir", "arguments": "{\"path\":\".\"}"}
                    }]
                }
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 7, "total_tokens": 12}
        }).to_string();
        let v: Value = serde_json::from_str(&convert_openai_to_gemini(&chat, false)).unwrap();
        let parts = v["candidates"][0]["content"]["parts"].as_array().unwrap();
        // part 顺序：thought → functionCall → text
        assert_eq!(parts[0]["thought"], true, "reasoning_content 必须标记为 thought part");
        assert_eq!(parts[0]["text"], "think think");
        assert_eq!(parts[1]["functionCall"]["name"], "list_dir");
        assert_eq!(parts[1]["functionCall"]["args"]["path"], ".", "arguments 字符串必须解析为 args 对象");
        assert_eq!(parts[2]["text"], "answer");
        assert_eq!(v["usageMetadata"]["promptTokenCount"], 5);
        assert_eq!(v["usageMetadata"]["candidatesTokenCount"], 7);
        assert_eq!(v["usageMetadata"]["totalTokenCount"], 12);
        assert_eq!(v["candidates"][0]["finishReason"], "STOP");
    }

    #[test]
    fn test_convert_openai_to_gemini_text_only() {
        let chat = serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": "hello"}}],
            "usage": {}
        }).to_string();
        let v: Value = serde_json::from_str(&convert_openai_to_gemini(&chat, false)).unwrap();
        let parts = v["candidates"][0]["content"]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "hello");
    }
}
