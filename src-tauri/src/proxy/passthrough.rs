//! 原生直连透传：入口协议与所配上游模型协议一致时（如 Anthropic 入口 +
//! Anthropic 兼容上游、Codex Responses 入口 + 官方 OpenAI、Gemini 入口 +
//! Google 官方），body/SSE 原样转发，零协议转换。usage 统计由 SseTapReader
//! 与非流式解析保持与转换路径一致。

use crate::models_config::ModelEntry;
use crate::rjlog;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};

use super::common::{build_agent, extract_model_from_path, safe_truncate, ProxyResponse};
use super::usage::store_usage_for_latest;
use tiny_http::StatusCode;

/// 透传场景的 SSE reader：与 SseStreamConverter 不同，它**原样转发上游的
/// 每一行**（event:/data:/空行/注释都保留）——Anthropic 与 Gemini 的事件类型
/// 信息在 event: 行上，透传时帧结构必须原样到达 agent。同时"顺带"解析流中
/// 的 usage 数据，在流结束（EOF/错误）时写入全局 usage store，保持转换路径
/// 的统计能力不变。
struct SseTapReader {
    upstream: BufReader<Box<dyn Read + Send>>,
    proto: TapProto,
    model: String,
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
    usage_seen: bool,
    stored: bool,
    pending: Vec<u8>,
    pos: usize,
    done: bool,
}

#[derive(Clone, Copy)]
enum TapProto {
    /// Anthropic Messages SSE：message_start.message.usage.input_tokens、
    /// message_delta.usage.output_tokens、cache_read_input_tokens
    Anthropic,
    /// OpenAI Responses SSE：response.completed.response.usage
    Responses,
    /// Gemini SSE：每个 chunk 的 usageMetadata（累积值，取最后出现的）
    Gemini,
}

impl SseTapReader {
    fn new(upstream: BufReader<Box<dyn Read + Send>>, proto: TapProto, model: String) -> Self {
        Self {
            upstream, proto, model,
            input_tokens: 0, output_tokens: 0, cached_tokens: 0,
            usage_seen: false, stored: false,
            pending: Vec::new(), pos: 0, done: false,
        }
    }

    /// 解析一行 SSE data，按协议累积 usage（不同协议的 usage 事件结构不同）。
    fn tap_line(&mut self, line: &str) {
        let t = line.trim();
        let Some(data) = t.strip_prefix("data:") else { return };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" { return; }
        let Ok(v) = serde_json::from_str::<Value>(data) else { return };
        match self.proto {
            TapProto::Anthropic => {
                // message_start：usage 在 message.usage；message_delta：usage 顶层
                let usages = [v.get("usage"), v.get("message").and_then(|m| m.get("usage"))];
                for u in usages.into_iter().flatten() {
                    if let Some(n) = u.get("input_tokens").and_then(|x| x.as_i64()) {
                        self.input_tokens = n;
                        self.usage_seen = true;
                    }
                    if let Some(n) = u.get("output_tokens").and_then(|x| x.as_i64()) {
                        self.output_tokens = n;
                        self.usage_seen = true;
                    }
                    if let Some(n) = u.get("cache_read_input_tokens").and_then(|x| x.as_i64()) {
                        self.cached_tokens = n;
                    }
                }
            }
            TapProto::Responses => {
                if v.get("type").and_then(|t| t.as_str()) == Some("response.completed") {
                    if let Some(u) = v.get("response").and_then(|r| r.get("usage")) {
                        if let Some(n) = u.get("input_tokens").and_then(|x| x.as_i64()) {
                            self.input_tokens = n;
                            self.usage_seen = true;
                        }
                        if let Some(n) = u.get("output_tokens").and_then(|x| x.as_i64()) {
                            self.output_tokens = n;
                        }
                        if let Some(n) = u.get("input_tokens_details")
                            .and_then(|d| d.get("cached_tokens"))
                            .and_then(|x| x.as_i64()) {
                            self.cached_tokens = n;
                        }
                    }
                }
            }
            TapProto::Gemini => {
                if let Some(u) = v.get("usageMetadata") {
                    if let Some(n) = u.get("promptTokenCount").and_then(|x| x.as_i64()) {
                        self.input_tokens = n;
                        self.usage_seen = true;
                    }
                    if let Some(n) = u.get("candidatesTokenCount").and_then(|x| x.as_i64()) {
                        self.output_tokens = n;
                    }
                    if let Some(n) = u.get("cachedContentTokenCount").and_then(|x| x.as_i64()) {
                        self.cached_tokens = n;
                    }
                }
            }
        }
    }

    /// 流结束时写入全局 usage store（只写一次）。
    fn store_usage(&mut self) {
        if self.stored || !self.usage_seen { return; }
        self.stored = true;
        // Anthropic 的 input_tokens 不含 cache_read 部分；转换路径的口径是
        // input 含缓存命中（OpenAI prompt_tokens 语义），对齐口径。
        let input = if matches!(self.proto, TapProto::Anthropic) {
            self.input_tokens + self.cached_tokens
        } else {
            self.input_tokens
        };
        store_usage_for_latest(self.model.clone(), input, self.output_tokens, self.cached_tokens);
        rjlog!("[PROXY USAGE] Passthrough stream done: input={}, output={}, cached={}",
            input, self.output_tokens, self.cached_tokens);
    }
}

impl Read for SseTapReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.pos < self.pending.len() {
                let n = (self.pending.len() - self.pos).min(buf.len());
                buf[..n].copy_from_slice(&self.pending[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            if self.done {
                self.store_usage();
                return Ok(0);
            }
            let mut line = String::new();
            match self.upstream.read_line(&mut line) {
                Ok(0) => {
                    self.done = true;
                    continue; // 循环回到 done 分支：store_usage 后返回 EOF
                }
                Ok(_) => {
                    self.tap_line(&line);
                    // 原样转发（保留行尾换行，SSE 帧结构不变）
                    self.pending = line.into_bytes();
                    self.pos = 0;
                }
                Err(e) => {
                    self.done = true;
                    self.store_usage();
                    return Err(e);
                }
            }
        }
    }
}

/// 归一化 base URL 与端点路径的拼接：兼容用户填写带或不带 /v1 的习惯
/// （如 `https://api.anthropic.com` 与 `https://api.z.ai/api/anthropic/v1`
/// 都能拼出正确的 /v1/messages）。
fn join_url_path(base: &str, endpoint: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1") || base.ends_with("/v1beta") {
        format!("{}/{}", base, endpoint)
    } else {
        format!("{}/v1/{}", base, endpoint)
    }
}

/// 透传前的最小改写：仅在必要时（模型名与配置不一致 / reasoning 关闭需
/// 剥离 thinking 参数）做一次 JSON round-trip；其余情况原样返回，保持零
/// 解析开销。注意只在 body **已有** model 字段时才改写——Gemini 的 model
/// 在 URL 路径里，body 没有 model 字段，不能凭空插入。
fn passthrough_rewrite_body(body: &str, real_model: &str, reasoning_disabled: bool) -> String {
    let Ok(mut v) = serde_json::from_str::<Value>(body) else { return body.to_string() };
    let mut dirty = false;
    if let Some(obj) = v.as_object_mut() {
        if let Some(req_model) = obj.get("model").and_then(|m| m.as_str()) {
            if !real_model.is_empty() && req_model != real_model {
                obj.insert("model".into(), serde_json::json!(real_model));
                dirty = true;
            }
        }
        if reasoning_disabled {
            // Anthropic `thinking` / Responses `reasoning`（顶层字段，形态不同）
            if obj.remove("thinking").is_some() { dirty = true; }
            if obj.remove("reasoning").is_some() { dirty = true; }
        }
    }
    // Gemini 的 thinking 配置在 generationConfig.thinkingConfig
    if reasoning_disabled {
        if let Some(gc) = v.get_mut("generationConfig").and_then(|g| g.as_object_mut()) {
            if gc.remove("thinkingConfig").is_some() { dirty = true; }
        }
    }
    if dirty { v.to_string() } else { body.to_string() }
}

/// 非流式响应的 usage 提取（各协议响应结构不同，口径与转换路径一致：
/// input 含缓存命中、cached 为命中子集）。
fn anthropic_sync_usage(v: &Value) -> (i64, i64, i64) {
    let u = &v["usage"];
    let input = u.get("input_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
    let output = u.get("output_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
    let cached = u.get("cache_read_input_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
    // Anthropic input_tokens 不含 cache_read，对齐口径
    (input + cached, output, cached)
}

fn responses_sync_usage(v: &Value) -> (i64, i64, i64) {
    let u = &v["usage"];
    let input = u.get("input_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
    let output = u.get("output_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
    let cached = u.get("input_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    (input, output, cached)
}

fn gemini_sync_usage(v: &Value) -> (i64, i64, i64) {
    let u = &v["usageMetadata"];
    let input = u.get("promptTokenCount").and_then(|x| x.as_i64()).unwrap_or(0);
    let output = u.get("candidatesTokenCount").and_then(|x| x.as_i64()).unwrap_or(0);
    let cached = u.get("cachedContentTokenCount").and_then(|x| x.as_i64()).unwrap_or(0);
    (input, output, cached)
}

/// 直连透传核心：原样转发 body 到同协议上游，原样返回响应（含 SSE 流）。
/// 同协议上游的错误体本就是 agent 能理解的格式，直接透传状态码与 body；
/// 4xx 原样透传让 agent 快速失败（与转换路径的 fail-fast 语义一致）。
fn passthrough_upstream(
    url: &str,
    auth_headers: &[(&'static str, String)],
    body: &str,
    stream: bool,
    tap_proto: TapProto,
    model: String,
    sync_usage: fn(&Value) -> (i64, i64, i64),
) -> ProxyResponse {
    rjlog!("[PROXY] Passthrough: POST {} (model={}, stream={}, body_len={})", url, model, stream, body.len());
    let agent = build_agent();
    let mut req = agent.post(url).set("Content-Type", "application/json");
    for (k, v) in auth_headers {
        req = req.set(k, v);
    }
    let resp = req.send_string(body);
    match resp {
        Ok(r) => {
            if stream {
                let reader = r.into_reader();
                let buf_reader = BufReader::new(Box::new(reader) as Box<dyn Read + Send>);
                ProxyResponse::Stream { reader: Box::new(SseTapReader::new(buf_reader, tap_proto, model)) }
            } else {
                let resp_body = r.into_string().unwrap_or_default();
                if let Ok(v) = serde_json::from_str::<Value>(&resp_body) {
                    let (input, output, cached) = sync_usage(&v);
                    if input > 0 || output > 0 || cached > 0 {
                        store_usage_for_latest(model, input, output, cached);
                        rjlog!("[PROXY USAGE] Passthrough sync done: input={}, output={}, cached={}", input, output, cached);
                    }
                }
                ProxyResponse::Sync(StatusCode(200), resp_body)
            }
        }
        Err(ureq::Error::Status(st, r)) => {
            let err_body = r.into_string().unwrap_or_default();
            rjlog!("[PROXY] Passthrough upstream HTTP {}: {}", st, safe_truncate(&err_body, 300));
            ProxyResponse::Sync(StatusCode(st), err_body)
        }
        Err(e) => {
            rjlog!("[PROXY] Passthrough connection error: {:?}", e);
            ProxyResponse::Sync(StatusCode(502), format!("{{\"error\":{{\"message\":\"Proxy connection error: {}\"}}}}", e))
        }
    }
}

/// Anthropic 协议直连：所配上游本身提供 Anthropic Messages 端点（官方 API、
/// Z.ai/智谱等 Anthropic 兼容网关）时，跳过 Anthropic→OpenAI 转换，请求/响应/
/// SSE 全部原样转发。
pub(crate) fn forward_anthropic_passthrough(entry: &ModelEntry, body: &str, reasoning_disabled: bool) -> ProxyResponse {
    let url = join_url_path(&entry.api_base, "messages");
    let out_body = passthrough_rewrite_body(body, &entry.name, reasoning_disabled);
    let stream = serde_json::from_str::<Value>(body).ok()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false);
    // x-api-key 为 Anthropic 标准；部分兼容网关只认 Authorization Bearer，双发兼容
    passthrough_upstream(
        &url,
        &[
            ("x-api-key", entry.api_key.clone()),
            ("Authorization", format!("Bearer {}", entry.api_key)),
            ("anthropic-version", "2023-06-01".to_string()),
        ],
        &out_body,
        stream,
        TapProto::Anthropic,
        entry.name.clone(),
        anthropic_sync_usage,
    )
}

/// OpenAI Responses 协议直连：Codex 入口 + 官方 OpenAI（或任何提供 /responses
/// 端点的上游）时，跳过 Responses↔Chat Completions 双向转换。
pub(crate) fn forward_responses_passthrough(entry: &ModelEntry, body: &str, reasoning_disabled: bool) -> ProxyResponse {
    let url = join_url_path(&entry.api_base, "responses");
    let out_body = passthrough_rewrite_body(body, &entry.name, reasoning_disabled);
    let stream = serde_json::from_str::<Value>(body).ok()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false);
    passthrough_upstream(
        &url,
        &[("Authorization", format!("Bearer {}", entry.api_key))],
        &out_body,
        stream,
        TapProto::Responses,
        entry.name.clone(),
        responses_sync_usage,
    )
}

/// Gemini 协议直连：Gemini CLI 入口 + Google 官方（或 Gemini 兼容上游）时，
/// 跳过 Gemini→OpenAI 转换。model 在 URL 路径里，不一致时替换路径段。
pub(crate) fn forward_gemini_passthrough(entry: &ModelEntry, body: &str, path: &str, reasoning_disabled: bool) -> ProxyResponse {
    // path 形如 /v1beta/models/{model}:generateContent?alt=sse
    let path_model = extract_model_from_path(path).unwrap_or("").to_string();
    let out_path = if !path_model.is_empty() && path_model != entry.name {
        path.replace(&format!("/models/{}", path_model), &format!("/models/{}", entry.name))
    } else {
        path.to_string()
    };
    // base 已带版本段（/v1、/v1beta）时剥掉，避免 /v1beta/v1beta/...
    let base = {
        let b = entry.api_base.trim_end_matches('/');
        if b.ends_with("/v1") || b.ends_with("/v1beta") {
            b.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_else(|| b.to_string())
        } else {
            b.to_string()
        }
    };
    let url = format!("{}{}", base, out_path);
    let out_body = passthrough_rewrite_body(body, &entry.name, reasoning_disabled);
    let stream = out_path.contains("streamGenerateContent")
        || serde_json::from_str::<Value>(body).ok()
            .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
            .unwrap_or(false);
    // x-goog-api-key 是 Google API key 的标准头（Bearer 留给 OAuth 场景；
    // Gemini 兼容网关要兼容 gemini-cli 就必须支持这个头）
    passthrough_upstream(
        &url,
        &[("x-goog-api-key", entry.api_key.clone())],
        &out_body,
        stream,
        TapProto::Gemini,
        entry.name.clone(),
        gemini_sync_usage,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::take_last_usage;

    // ── join_url_path：兼容带/不带 /v1 的 base 填写习惯 ──

    #[test]
    fn test_join_url_path_variants() {
        assert_eq!(join_url_path("https://api.anthropic.com", "messages"), "https://api.anthropic.com/v1/messages");
        assert_eq!(join_url_path("https://api.anthropic.com/", "messages"), "https://api.anthropic.com/v1/messages");
        assert_eq!(join_url_path("https://api.z.ai/api/anthropic", "messages"), "https://api.z.ai/api/anthropic/v1/messages");
        assert_eq!(join_url_path("https://api.z.ai/api/anthropic/v1", "messages"), "https://api.z.ai/api/anthropic/v1/messages");
        assert_eq!(join_url_path("https://api.openai.com", "responses"), "https://api.openai.com/v1/responses");
        assert_eq!(join_url_path("https://api.openai.com/v1", "responses"), "https://api.openai.com/v1/responses");
    }

    // ── passthrough_rewrite_body：仅在必要时改写 ──

    #[test]
    fn test_rewrite_body_model_rename() {
        let body = r#"{"model":"alias-name","stream":true,"messages":[]}"#;
        let out = passthrough_rewrite_body(body, "real-upstream-name", false);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["model"], "real-upstream-name");
        assert_eq!(v["stream"], true, "其余字段必须原样保留");
    }

    #[test]
    fn test_rewrite_body_no_change_when_model_matches() {
        let body = r#"{"model":"same-name","stream":false}"#;
        let out = passthrough_rewrite_body(body, "same-name", false);
        assert_eq!(out, body, "无必要时不做 JSON round-trip，body 原样返回");
    }

    #[test]
    fn test_rewrite_body_strips_thinking_when_reasoning_disabled() {
        let body = r#"{"model":"m","thinking":{"type":"enabled","budget_tokens":1024},"max_tokens":100}"#;
        let out = passthrough_rewrite_body(body, "m", true);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v.get("thinking").is_none(), "reasoning 关闭时应剥离 thinking");
        assert_eq!(v["max_tokens"], 100);

        // Responses 形态的 reasoning 对象同样剥离
        let body2 = r#"{"model":"m","reasoning":{"effort":"high"}}"#;
        let out2 = passthrough_rewrite_body(body2, "m", true);
        let v2: Value = serde_json::from_str(&out2).unwrap();
        assert!(v2.get("reasoning").is_none());

        // Gemini 嵌套形态 generationConfig.thinkingConfig
        let body3 = r#"{"contents":[],"generationConfig":{"thinkingConfig":{"includeThoughts":true}}}"#;
        let out3 = passthrough_rewrite_body(body3, "m", true);
        let v3: Value = serde_json::from_str(&out3).unwrap();
        assert!(v3["generationConfig"].get("thinkingConfig").is_none());

        // reasoning 开启时保留
        let out4 = passthrough_rewrite_body(body, "m", false);
        assert!(serde_json::from_str::<Value>(&out4).unwrap().get("thinking").is_some());
    }

    #[test]
    fn test_rewrite_body_never_inserts_model_for_gemini() {
        // Gemini 的 model 在 URL 路径里，body 没有 model 字段——不能凭空插入
        let body = r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#;
        let out = passthrough_rewrite_body(body, "gemini-2.5-pro", false);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v.get("model").is_none(), "Gemini body 不应被插入 model 字段");
    }

    // ── 非流式 usage 提取（口径：input 含缓存命中） ──

    #[test]
    fn test_sync_usage_extractors() {
        let anth: Value = serde_json::from_str(r#"{"usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":30}}"#).unwrap();
        assert_eq!(anthropic_sync_usage(&anth), (130, 50, 30));

        let resp: Value = serde_json::from_str(r#"{"usage":{"input_tokens":200,"output_tokens":80,"input_tokens_details":{"cached_tokens":60}}}"#).unwrap();
        assert_eq!(responses_sync_usage(&resp), (200, 80, 60));

        let gem: Value = serde_json::from_str(r#"{"usageMetadata":{"promptTokenCount":300,"candidatesTokenCount":90,"cachedContentTokenCount":120}}"#).unwrap();
        assert_eq!(gemini_sync_usage(&gem), (300, 90, 120));

        // 无 usage 字段时全零，不 panic
        let empty: Value = serde_json::from_str("{}").unwrap();
        assert_eq!(anthropic_sync_usage(&empty), (0, 0, 0));
        assert_eq!(responses_sync_usage(&empty), (0, 0, 0));
        assert_eq!(gemini_sync_usage(&empty), (0, 0, 0));
    }

    // ── SseTapReader：原样转发 + usage 解析 ──

    fn tap_stream(lines: &[&str], proto: TapProto) -> String {
        let data = lines.join("\n");
        let reader = BufReader::new(Box::new(std::io::Cursor::new(data.into_bytes())) as Box<dyn Read + Send>);
        let mut tap = SseTapReader::new(reader, proto, "test-model".into());
        let mut out = Vec::new();
        tap.read_to_end(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn test_tap_reader_passthrough_frame_integrity() {
        let input = [
            "event: message_start",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":42}}}",
            "",
            "event: message_delta",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":7}}",
            "",
            "event: message_stop",
            "data: {\"type\":\"message_stop\"}",
            "",
        ];
        let out = tap_stream(&input, TapProto::Anthropic);
        // 每一行（含 event: 行与空行）必须原样转发，帧结构不变
        assert_eq!(out, input.join("\n"));
    }

    #[test]
    fn test_tap_reader_anthropic_usage() {
        let input = [
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":30}}}",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":50}}",
        ];
        let data = input.join("\n");
        let reader = BufReader::new(Box::new(std::io::Cursor::new(data.into_bytes())) as Box<dyn Read + Send>);
        let mut tap = SseTapReader::new(reader, TapProto::Anthropic, "tap-anth".into());
        let mut out = Vec::new();
        tap.read_to_end(&mut out).unwrap();
        // EOF 后 usage 已写入全局 store（input 对齐口径 = input + cache_read）
        let (model, input, output, cached) = take_last_usage("tap-anth").unwrap();
        assert_eq!((model.as_str(), input, output, cached), ("tap-anth", 130, 50, 30));
    }

    #[test]
    fn test_tap_reader_responses_usage() {
        let input = [
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
        ];
        let data = input.join("\n");
        let reader = BufReader::new(Box::new(std::io::Cursor::new(data.into_bytes())) as Box<dyn Read + Send>);
        let mut tap = SseTapReader::new(reader, TapProto::Responses, "tap-resp".into());
        let mut out = Vec::new();
        tap.read_to_end(&mut out).unwrap();
        let (model, input, output, cached) = take_last_usage("tap-resp").unwrap();
        assert_eq!((model.as_str(), input, output, cached), ("tap-resp", 10, 5, 2));
    }

    #[test]
    fn test_tap_reader_gemini_usage_last_chunk_wins() {
        let input = [
            "data: {\"candidates\":[],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":1}}",
            "data: {\"candidates\":[],\"usageMetadata\":{\"promptTokenCount\":300,\"candidatesTokenCount\":90,\"cachedContentTokenCount\":120}}",
        ];
        let data = input.join("\n");
        let reader = BufReader::new(Box::new(std::io::Cursor::new(data.into_bytes())) as Box<dyn Read + Send>);
        let mut tap = SseTapReader::new(reader, TapProto::Gemini, "tap-gem".into());
        let mut out = Vec::new();
        tap.read_to_end(&mut out).unwrap();
        let (model, input, output, cached) = take_last_usage("tap-gem").unwrap();
        assert_eq!((model.as_str(), input, output, cached), ("tap-gem", 300, 90, 120));
    }
}
