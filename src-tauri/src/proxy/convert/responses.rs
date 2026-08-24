//! OpenAI Responses 入口 ↔ OpenAI Chat Completions 出口的双向转换。
//! Codex 只走 /responses（wire_api=chat 已被官方移除），而多数第三方模型
//! 只提供 /chat/completions，因此需要请求与 SSE 事件链的完整双向模拟。

use crate::models_config::ModelEntry;
use crate::rjlog;
use crate::rjlogd;
use serde_json::Value;
use std::io::{BufReader, Read, Write};

use crate::proxy::common::{apply_bash_tool_hardening, build_agent, clamp_max_tokens, ensure_tool_calls_paired, find_model, inject_bash_guidance, is_claude_only_tool, limit_tools_for_llama, normalize_message_sequence, safe_truncate, tool_call_args_str, tool_call_args_str_tracked, LLAMA_MAX_TOOLS, ProxyResponse, SseStreamConverter};
use crate::proxy::usage::store_usage_for_latest;
use tiny_http::StatusCode;

/// Translate OpenAI Responses API → OpenAI Chat Completions.
/// Codex uses the Responses API (/responses), but most providers (DeepSeek, etc.)
/// only support Chat Completions (/v1/chat/completions).
pub(crate) fn proxy_responses_to_openai(body: &str, models: &[ModelEntry], preferred_ids: Option<&[String]>, reasoning_disabled: bool) -> ProxyResponse {
    let req: Value = match serde_json::from_str(body) { Ok(v) => v, Err(_) => return ProxyResponse::Sync(StatusCode(400), "Invalid JSON".into()) };
    let model_name = req["model"].as_str().unwrap_or("");
    let stream = req["stream"].as_bool().unwrap_or(false);
    rjlog!("[PROXY] Responses→Chat stream={}", stream);

    // Convert Responses API `input` → Chat Completions `messages`
    let messages = if let Some(input) = req.get("input") {
        if let Some(arr) = input.as_array() {
            // Pre-collect call_ids that have a corresponding function_call_output.
            // A function_call without a matching output produces an assistant
            // message with tool_calls but no following tool message — the
            // upstream API rejects this ("insufficient tool messages following
            // tool_calls message"). Skip orphaned function_calls to prevent this.
            let output_call_ids: std::collections::HashSet<&str> = arr.iter()
                .filter(|item| item["type"].as_str() == Some("function_call_output"))
                .filter_map(|item| item["call_id"].as_str())
                .collect();
            let mut msgs: Vec<Value> = vec![];
            for item in arr {
                let item_type = item["type"].as_str().unwrap_or("");

                match item_type {
                    "function_call" => {
                        let call_id = item["call_id"].as_str().unwrap_or("");
                        // Skip function_calls whose output is missing — sending
                        // them would violate the API's tool_calls pairing rule.
                        // 空 call_id 也一并跳过：`"id": ""` 的 tool_call 会被
                        // OpenAI 兼容端点（如火山引擎 ark）以 InvalidParameter 拒绝。
                        if call_id.is_empty() || !output_call_ids.contains(call_id) {
                            rjlog!("[PROXY] Responses→Chat: skipping orphaned function_call {:?} (no output or empty id)", call_id);
                            continue;
                        }
                        let name = item["name"].as_str().unwrap_or("");
                        // 空参数（{} / "" / null）时兜底为合法 JSON `{}`——
                        // `"arguments": ""` 不是合法 JSON，火山引擎 ark 等
                        // OpenAI 兼容端点会以 InvalidParameter 拒绝。
                        let arguments = tool_call_args_str(&item["arguments"]).unwrap_or_else(|| "{}".into());
                        let new_tc = serde_json::json!({
                            "id": call_id,
                            "type": "function",
                            "function": {"name": name, "arguments": arguments}
                        });
                        // Merge consecutive function_calls into a single assistant
                        // message with multiple tool_calls. The Chat API requires
                        // tool responses to follow the assistant message that emitted
                        // the tool_calls — separate assistant messages would violate
                        // this ordering and cause "insufficient tool messages" errors.
                        // 同时与前一 assistant（含纯文本回复）合并：连续两个
                        // assistant 消息（如 codex 先回复文本、紧接着发起工具调用）
                        // 会让部分兼容端点（火山引擎 ark）报 InvalidParameter。
                        if let Some(last) = msgs.last_mut() {
                            if last["role"].as_str() == Some("assistant") {
                                if let Some(arr) = last["tool_calls"].as_array_mut() {
                                    arr.push(new_tc);
                                    continue;
                                } else {
                                    // 纯文本 assistant 收到 tool_call → 合并 tool_calls
                                    // 进同一消息（content 保留），避免相邻两个 assistant。
                                    last["tool_calls"] = serde_json::json!([new_tc]);
                                    continue;
                                }
                            }
                        }
                        msgs.push(serde_json::json!({
                            "role": "assistant",
                            // 纯工具调用的 assistant 无文本：用官方格式 `null` 而非
                            // 空字符串 `""`（火山引擎 ark 等兼容端点对空字符串
                            // content 的 assistant 消息会报 InvalidParameter；
                            // Gemini/Claude 调用工具时常带文字所以未触发）。
                            "content": null,
                            "tool_calls": [new_tc]
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
                        // codex 回传的推理项（无 role）：不转发。reasoning_content
                        // 输入字段并非所有端点都接受——火山引擎 MiniMax-M3 会报
                        // InvalidParameter；且大多数端点（含 qwen）不强制要求回传。
                        // 只有 DeepSeek 官方 API 强制校验，而 codex 路径目前不面向
                        // 该端点，因此这里直接跳过，避免向上游发送不被接受的字段。
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
    // Safety net: ensure every tool_calls message has a paired tool response,
    // in case the targeted skip above missed an edge case.
    let messages = ensure_tool_calls_paired(messages);

    if model_name.is_empty() || messages.is_empty() {
        return ProxyResponse::Sync(StatusCode(400), r#"{"error":"Missing model or input"}"#.into());
    }

    // Find matching model in config
    let target = find_model(models, model_name, preferred_ids);
    let (api_key, base_url, real_model, support_tools, context_window) = if let Some(m) = target {
        let masked_key = if m.api_key.len() > 8 {
            format!("{}...{}", &m.api_key[..4], &m.api_key[m.api_key.len()-4..])
        } else { m.api_key.clone() };
        rjlog!("[PROXY] Responses→Chat: Found model '{}' api_key={} base_url={}", m.name, masked_key, m.api_base);
        (m.api_key.clone(), m.api_base.clone(), m.name.clone(), m.support_tools, m.context_window)
    } else {
        rjlog!("[PROXY] Responses→Chat: Model '{}' NOT FOUND in {} models. Available: {:?}",
            model_name, models.len(),
            models.iter().map(|m| format!("{}({})", m.name, m.id)).collect::<Vec<_>>());
        return ProxyResponse::Sync(StatusCode(404), format!(r#"{{"error":"Model {} not configured"}}"#, model_name));
    };

    // llama.cpp 本地模型：工具定义过多会拖慢 prefill 与 grammar 解析，裁剪到核心工具
    let is_llama_cpp = real_model.contains("llama-") || real_model.ends_with(".gguf");

    // Build Chat Completions request (convert Responses API tool format to Chat format)
    let mut chat_body = serde_json::json!({
        "model": real_model,
        "messages": messages,
        "stream": stream,
    });
    // Responses API 的 max_output_tokens → Chat 的 max_tokens，并用 context_window 限制
    // 小窗口模型收到超大 max_tokens 会报 400。
    let mot = req["max_output_tokens"].as_u64().unwrap_or(4096);
    let clamped = clamp_max_tokens(mot, context_window);
    if clamped != mot {
        rjlog!("[PROXY] {}: reduced max_output_tokens from {} to context_window {}", real_model, mot, clamped);
    }
    chat_body["max_tokens"] = serde_json::json!(clamped);
    if let Some(tools) = req.get("tools").and_then(|v| v.as_array()) {
        let mut chat_tools: Vec<Value> = tools.iter()
            .filter(|t| t["type"].as_str() == Some("function"))
            .filter(|t| !is_claude_only_tool(t["name"].as_str().unwrap_or("")))
            .map(|t| {
                // parameters 缺失/null 时用空对象 {}（OpenAI 兼容端点不接受 null）
                let mut parameters = t.get("parameters").cloned().unwrap_or(serde_json::Value::Null);
                if parameters.is_null() {
                    parameters = serde_json::json!({"type": "object", "properties": {}});
                }
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t["name"],
                        // description 缺失/null 时用空字符串：部分 OpenAI 兼容端点
                        // （如 DashScope）对 null description 会报 InvalidParameter
                        "description": t.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                        "parameters": parameters,
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

        // Bash 工具 schema 强化：确保 command 必填（见 common.rs harden_bash_tool）
        apply_bash_tool_hardening(&mut chat_tools);

        if is_llama_cpp && chat_tools.len() > LLAMA_MAX_TOOLS {
            rjlog!("[PROXY] Responses→Chat: limiting tools from {} to {} for llama.cpp", chat_tools.len(), LLAMA_MAX_TOOLS);
            chat_tools = limit_tools_for_llama(&chat_tools, LLAMA_MAX_TOOLS);
        }

        if support_tools {
            chat_body["tools"] = serde_json::Value::Array(chat_tools);
            // 第三方模型 Bash 调用规范（仅工具请求，普通聊天零开销）
            inject_bash_guidance(&mut chat_body);
            if let Some(tc) = req.get("tool_choice") {
                // Responses API 的 tool_choice 与 Chat API 格式不同：
                //   Responses: {"type":"function","name":"x"}
                //   Chat:      {"type":"function","function":{"name":"x"}}
                // 直接透传会让 OpenAI 兼容端点（如 DashScope）报 InvalidParameter。
                if let Some(name) = tc.get("name").and_then(|v| v.as_str()) {
                    chat_body["tool_choice"] = serde_json::json!({
                        "type": "function",
                        "function": {"name": name}
                    });
                } else {
                    chat_body["tool_choice"] = tc.clone();
                }
            }
        } else {
            rjlog!("[PROXY] Responses→Chat: model {} does not support tools, skipping tool definitions", real_model);
        }
    }

    if reasoning_disabled {
        // 只发送 OpenAI 标准参数（thinking/reasoning_effort/enable_thinking
        // 非标准，OpenAI 兼容端点如火山引擎会报 400）
        chat_body["temperature"] = serde_json::json!(0.6);
    }

    // 最终消息序列规范化：合并相邻 system/assistant（在 ensure_tool_calls_paired
    // 之后调用，先配对、再合并）。Codex 历史重放 / 工具未完成时发起请求极易
    // 产生相邻同 role 消息或重复的沙箱指令 system，兼容端点（火山引擎 ark）
    // 会以 InvalidParameter 拒绝。
    if let Some(msgs) = chat_body.get_mut("messages").and_then(|v| v.as_array_mut()) {
        let norm = normalize_message_sequence(std::mem::take(msgs));
        *msgs = norm;
    }

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    rjlog!("[PROXY] Responses→Chat: POST {} (model={}, stream={})", url, real_model, stream);

    let request_body = chat_body.to_string();
    let role_seq: Vec<&str> = chat_body["messages"].as_array()
        .map(|a| a.iter().filter_map(|m| m["role"].as_str()).collect())
        .unwrap_or_default();
    rjlog!("[PROXY] Responses→Chat: body ({} chars) — model:{} messages:{} roles:[{}]",
        request_body.len(), real_model, role_seq.len(), role_seq.join(","));

    let agent = build_agent();
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
                    Box::new(make_responses_sse_converter(real_model, reasoning_disabled)),
                );
                rjlog!("[PROXY] Responses→Chat: returning streaming response");
                ProxyResponse::Stream { reader: Box::new(converter) }
            } else {
                match r.into_string() {
                    Ok(resp_body) => {
                        if status >= 400 {
                            return ProxyResponse::Sync(StatusCode(status), resp_body);
                        }
                        let converted = convert_chat_to_responses(&resp_body, &real_model, false, reasoning_disabled);
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
            if status == 400 {
                // 400 参数错误时 dump 请求体的消息骨架（每条 role / content 形态
                // 与长度 / tool_calls 数量与名称 / tool_call_id），避免被超长
                // system（Codex 沙箱指令）撑满 3000 字符截断而看不到问题消息。
                if let Some(msgs) = chat_body["messages"].as_array() {
                    let outline: Vec<String> = msgs.iter().enumerate().map(|(i, m)| {
                        let role = m["role"].as_str().unwrap_or("?");
                        let content = match m.get("content") {
                            Some(Value::String(s)) => format!("str[{}]:{}", s.len(), safe_truncate(s, 60)),
                            Some(Value::Array(a)) => format!("arr[{}]", a.len()),
                            Some(Value::Null) | None => "none".to_string(),
                            Some(v) => format!("{:?}", v).chars().take(60).collect::<String>(),
                        };
                        let tc_count = m.get("tool_calls").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                        let tc_names: Vec<&str> = m.get("tool_calls").and_then(|v| v.as_array())
                            .map(|a| a.iter().filter_map(|t| t["function"]["name"].as_str()).collect())
                            .unwrap_or_default();
                        let tcid = m["tool_call_id"].as_str().unwrap_or("");
                        format!("#{} {} | content={} | tool_calls={}{} | tc_id={}",
                            i, role, content, tc_count,
                            if tc_names.is_empty() { String::new() } else { format!("({})", tc_names.join(",")) },
                            tcid)
                    }).collect();
                    rjlog!("[PROXY] HTTP 400 messages outline:\n{}", outline.join("\n"));
                }
                rjlog!("[PROXY] HTTP 400 request body preview: {}", safe_truncate(&request_body, 3000));
            }
            // Pass 4xx through so agents fail fast instead of retrying a
            // permanent error (e.g. bad model name → endless 404s).
            // 错误类型按 Anthropic API 规范映射：401 → authentication_error，
            // 客户端立即识别为认证失败并停止，而非当瞬时错误重试。
            let resp_status = if (400..500).contains(&status) { StatusCode(status) } else { StatusCode(502) };
            let err_type = match status {
                401 => "authentication_error",
                403 => "permission_error",
                404 => "not_found_error",
                429 => "rate_limit_error",
                s if (400..500).contains(&s) => "invalid_request_error",
                _ => "api_error",
            };
            rjlog!("[PROXY] Responses: returning HTTP {} (type={}) to agent", resp_status.0, err_type);
            ProxyResponse::Sync(resp_status, format!(r#"{{"error":{{"message":"Upstream {}: {}","type":"{}"}}}}"#, status, safe_truncate(&body, 300), err_type))
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
/// `reasoning_disabled` 为 true 时剥离 reasoning_content，不生成 reasoning item。
fn convert_chat_to_responses(chat_resp: &str, model: &str, _stream: bool, reasoning_disabled: bool) -> String {
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
    // reasoning_disabled 时剥离思考内容
    if !reasoning_disabled && !reasoning_content.is_empty() {
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
            let arguments = tool_call_args_str_tracked(&tc["function"]["arguments"], &name, "responses non-streaming").unwrap_or_default();
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
/// `reasoning_disabled` 为 true 时剥离 reasoning_content，不发 reasoning 事件。
fn make_responses_sse_converter(model: String, reasoning_disabled: bool) -> impl FnMut(&str) -> Vec<u8> {
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
            rjlogd!("[CACHE DEBUG] SSE chunk #{}: data={}", chunk_counter, safe_truncate(data, 300));
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

            for (_, (fc_id, call_id, name, args)) in tool_items.drain() {
                let oi = 1;
                let _ = write!(result, "event: response.function_call_arguments.done\ndata: {}\n\n", serde_json::json!({
                    "type": "response.function_call_arguments.done", "output_index": oi, "item_id": fc_id
                }));
                let _ = write!(result, "event: response.output_item.done\ndata: {}\n\n", serde_json::json!({
                    "type": "response.output_item.done", "output_index": oi, "item": {
                        "id": fc_id, "object": "realtime.item", "type": "function_call", "status": "completed",
                        // 必须回传真实的 call_id / name：codex 用它构造 function_call_output，
                        // 空 call_id 会让 codex 回传空 id，proxy 配对时产生孤儿 tool 消息
                        // 并被丢弃，最终上游收到 tool_call_id 为空串的请求 → 400
                        // InvalidParameter。
                        "call_id": call_id, "name": name, "arguments": args
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
            // reasoning_disabled 时剥离思考内容（不发 reasoning 事件）
            if !reasoning_disabled {
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

                    if let Some(args) = tool_call_args_str_tracked(&tc["function"]["arguments"], &tc["function"]["name"].as_str().unwrap_or(""), "responses streaming") {
                        if let Some(entry) = tool_items.get_mut(&idx) {
                            entry.3.push_str(&args);
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



#[cfg(test)]
mod tests {
    use super::*;

    // ── convert_chat_to_responses：OpenAI Chat 响应 → Responses API 结构 ──
    // Codex 只认 Responses 格式；reasoning_content 必须拆成独立的 reasoning
    // output item（Codex 的思考折叠 UI 依赖它），tool_calls 拆成 function_call。

    #[test]
    fn test_convert_chat_to_responses_reasoning_and_text() {
        let chat = serde_json::json!({
            "choices": [{
                "message": {"role": "assistant", "content": "done", "reasoning_content": "hmm"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7}
        }).to_string();
        let out = convert_chat_to_responses(&chat, "deepseek-v4", false, false);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["model"], "deepseek-v4");
        let items = v["output"].as_array().unwrap();
        // reasoning item 在前、message item 在后
        assert_eq!(items[0]["type"], "reasoning");
        assert_eq!(items[0]["summary"][0]["text"], "hmm");
        assert_eq!(items[1]["type"], "message");
        assert_eq!(items[1]["content"][0]["text"], "done");
        assert_eq!(v["usage"]["input_tokens"], 3);
        assert_eq!(v["usage"]["output_tokens"], 4);
        assert_eq!(v["usage"]["total_tokens"], 7);
    }

    #[test]
    fn test_convert_chat_to_responses_tool_calls() {
        let chat = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{"id": "call_7", "function": {"name": "shell", "arguments": "{\"cmd\":\"ls\"}"}}]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        }).to_string();
        let v: Value = serde_json::from_str(&convert_chat_to_responses(&chat, "m", false, false)).unwrap();
        let items = v["output"].as_array().unwrap();
        let fc = items.iter().find(|i| i["type"] == "function_call").expect("必须输出 function_call item");
        assert_eq!(fc["call_id"], "call_7");
        assert_eq!(fc["name"], "shell");
        assert_eq!(fc["arguments"], "{\"cmd\":\"ls\"}");
        assert_eq!(fc["status"], "completed");
    }

    #[test]
    fn test_convert_chat_to_responses_invalid_json_passthrough() {
        // 非 JSON 输入原样返回（上游异常时的兜底行为）
        assert_eq!(convert_chat_to_responses("not-json", "m", false, false), "not-json");
    }
}
