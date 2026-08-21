//! OpenAI Chat Completions 入口的直通处理：本身即目标协议，只做模型路由、
//! 鉴权替换与 llama.cpp 等特殊兼容处理，无格式转换。

use crate::models_config::{ModelConfig, ModelEntry};
use crate::rjlog;
use serde_json::Value;
use std::io::{BufReader, Read, Write};

use crate::proxy::common::{build_agent, find_model, limit_tools_for_llama, safe_truncate, LLAMA_MAX_TOOLS, ProxyResponse, SseStreamConverter};
use super::anthropic::make_anthropic_sse_converter;
use tiny_http::StatusCode;

pub(crate) fn proxy_openai_direct(body: &str, _models: &[ModelEntry], preferred_ids: Option<&[String]>, reasoning_disabled: bool) -> ProxyResponse {
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
            // llama.cpp 本地模型：工具定义过多会拖慢 prefill 与 grammar 解析，裁剪到核心工具
            if let Some(tools) = req_body.get("tools").and_then(|v| v.as_array()) {
                if tools.len() > LLAMA_MAX_TOOLS {
                    rjlog!("[PROXY] OpenAI direct: limiting tools from {} to {} for llama.cpp", tools.len(), LLAMA_MAX_TOOLS);
                    req_body["tools"] = serde_json::json!(limit_tools_for_llama(tools, LLAMA_MAX_TOOLS));
                }
            }
        }
        
        if reasoning_disabled {
            // 只发送 OpenAI 标准参数（thinking/reasoning_effort/enable_thinking
            // 非标准，OpenAI 兼容端点如火山引擎会报 400）
            req_body["temperature"] = serde_json::json!(0.6);
            rjlog!("[PROXY] reasoning_disabled=true, modified body: temperature=0.6");
        }
        let modified_body = req_body.to_string();
        rjlog!("[PROXY] OpenAI direct: POST {} (model={}, stream={}, body_len={})", url, model_name, stream, modified_body.len());
        
        let request_start = std::time::Instant::now();
        let resp = build_agent()
            .post(&url)
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
                            Box::new(make_llama_cpp_sse_converter(model_name_clone, reasoning_disabled))
                        } else {
                            Box::new(make_anthropic_sse_converter(model_name_clone, reasoning_disabled))
                        },
                    );
                    rjlog!("[PROXY] OpenAI direct: returning streaming response is_llama={} in {:?}", is_llama_cpp, request_start.elapsed());
                    ProxyResponse::Stream { reader: Box::new(converter) }
                } else {
                    ProxyResponse::Sync(StatusCode(200), r.into_string().unwrap_or_default())
                }
            }
            Err(ureq::Error::Status(st, r)) => {
                let body = r.into_string().unwrap_or_default();
                rjlog!("[PROXY] OpenAI direct: upstream HTTP {}: {}", st, safe_truncate(&body, 500));
                // Pass 4xx through so the agent fails fast instead of retrying
                // a permanent error (bad model name → endless 404s).
                // 错误类型按 Anthropic API 规范映射（openai.rs 的响应会转为
                // Anthropic 格式）：401 标成 authentication_error，客户端立即
                // 识别为认证失败并停止，而非当瞬时错误重试。
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
                rjlog!("[PROXY] OpenAI direct: returning HTTP {} (type={}) to agent", resp_status.0, err_type);
                ProxyResponse::Sync(resp_status, err_body.to_string())
            }
            Err(e) => ProxyResponse::Sync(StatusCode(502), format!("Proxy error: {}", e)),
        }
    } else {
        ProxyResponse::Sync(StatusCode(404), format!("Model {} not configured", model_name))
    }
}

fn make_llama_cpp_sse_converter(model_name: String, reasoning_disabled: bool) -> impl FnMut(&str) -> Vec<u8> {
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
                        // reasoning_disabled 时丢弃 think 标签内的内容（不进入 think_buffer）
                        if let Some(end_pos) = remaining.find("<｜end_of_thought｜>") {
                            if !reasoning_disabled {
                                think_buffer.push_str(&remaining[..end_pos]);
                            }
                            remaining = remaining[end_pos + "<｜end_of_thought｜>".len()..].to_string();
                            in_think_tag = false;
                        } else if let Some(end_pos) = remaining.find("</think>") {
                            if !reasoning_disabled {
                                think_buffer.push_str(&remaining[..end_pos]);
                            }
                            remaining = remaining[end_pos + "</think>".len()..].to_string();
                            in_think_tag = false;
                        } else {
                            if !reasoning_disabled {
                                think_buffer.push_str(&remaining);
                            }
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

