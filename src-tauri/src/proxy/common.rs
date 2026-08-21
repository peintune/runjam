//! Proxy 内部共享工具：模型路由、SSE 转换 reader、消息规整等。

use crate::models_config::ModelEntry;
use crate::rjlog;
use crate::rjlogd;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::time::Duration;
use tiny_http::StatusCode;

/// 构建带长整体超时的 ureq Agent。
///
/// 本地模型（llama-server 等）处理巨型 prompt 可能需要十几分钟，
/// 而 ureq 默认整体超时只有 30 秒——模型还没算完代理就超时返回空结果，
/// 表现为"会话没数据返回"。因此统一使用足够长的整体超时（30 分钟）。
pub(crate) fn build_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(1800))
        .build()
}

/// llama.cpp 本地模型的工具数量上限。
///
/// 本地小模型（Qwen3-4B 等）+ CPU 推理时，工具定义过多会拖慢 prefill 与
/// grammar 解析，且小模型无法在几十个工具中正确选择调用。裁剪到少量核心工具。
/// 只影响 llama.cpp 本地模型，商业模型（Claude/GPT/Gemini 等）不受影响。
pub(crate) const LLAMA_MAX_TOOLS: usize = 5;

/// 核心工具关键字（大小写不敏感）：裁剪时优先保留的工具名子串。
const CORE_TOOL_SUBSTRINGS: &[&str] = &[
    "read", "write", "edit", "bash", "shell", "glob", "grep",
    "task", "command", "exec", "patch", "search", "list", "fetch",
];

/// 对 llama.cpp 本地模型裁剪工具数量：优先保留核心工具（read/write/edit/
/// bash/shell/glob/grep 等），不足 max 时按原顺序补足；输出按 name 排序
/// 保证确定性（利于 upstream 缓存命中）。工具数不超过 max 时原样返回。
pub(crate) fn limit_tools_for_llama(tools: &[Value], max: usize) -> Vec<Value> {
    if tools.len() <= max {
        return tools.to_vec();
    }
    let mut kept: Vec<Value> = Vec::new();
    let mut rest: Vec<Value> = Vec::new();
    for t in tools {
        let name = t["function"]["name"].as_str().unwrap_or("").to_lowercase();
        if CORE_TOOL_SUBSTRINGS.iter().any(|k| name.contains(k)) {
            kept.push(t.clone());
        } else {
            rest.push(t.clone());
        }
    }
    kept.extend(rest);
    kept.truncate(max);
    kept.sort_by(|a, b| {
        a["function"]["name"]
            .as_str()
            .unwrap_or("")
            .cmp(&b["function"]["name"].as_str().unwrap_or(""))
    });
    kept
}

/// 安全截断字符串到 max_bytes 字节以内，确保不会切在多字节 UTF-8 字符中间。
pub(crate) fn safe_truncate(s: &str, max_bytes: usize) -> &str {
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
pub(crate) enum ProxyResponse {
    Sync(StatusCode, String),
    Stream {
        reader: Box<dyn Read + Send>,
    },
}

/// Wraps a BufReader from the upstream SSE response and a line-conversion
/// closure into a `Read` impl that tiny_http can use as a streaming body.
pub(crate) struct SseStreamConverter {
    upstream: BufReader<Box<dyn Read + Send>>,
    convert: Box<dyn FnMut(&str) -> Vec<u8> + Send>,
    pending: Vec<u8>,
    pos: usize,
    done: bool,
    first: bool,
}

impl SseStreamConverter {
    pub(crate) fn new(
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
            rjlogd!("[PROXY STREAM] Read line: {} ({} bytes)", safe_truncate(trimmed, 80), trimmed.len());
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
            rjlogd!("[PROXY STREAM] Pending {} bytes for output", self.pending.len());
        }
    }
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
pub(crate) fn find_model<'a>(
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
            // Prefer matches in the agent's preferred_id order, NOT the models
            // list order: when several entries share the same name (e.g. two
            // "MiniMax-M3" with different base URLs), the assigned model must win.
            for id in ids {
                if let Some(m) = matches.iter().copied().find(|m| &m.id == id) {
                    return Some(m);
                }
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
            rjlog!("[PROXY] WARNING: requested model '{}' not found in config; falling back to assigned model '{}' (id={})", model_name, m.name, m.id);
            return Some(m);
        }
        None
    }
    // --- Step 3: nothing ---
    else {
        None
    }
}

/// Safety net: normalise tool_calls ↔ tool message pairing so the upstream
/// Chat Completions API doesn't reject the request. Handles two failure modes:
///
/// 1. "insufficient tool messages following tool_calls message"
///    — an assistant message carries `tool_calls` but some `tool_call_id`
///    has no following `tool` response. A placeholder `tool` message is
///    inserted before the next non-tool message (or at the end).
///
/// 2. "Messages with role 'tool' must be a response to a preceding message
///    with 'tool_calls'"
///    — a `tool` message exists whose `tool_call_id` doesn't match any
///    pending `tool_calls` from a preceding assistant message (e.g. the
///    assistant turn was dropped during conversion, or the history is
///    malformed). The orphaned `tool` message is dropped.
pub(crate) fn ensure_tool_calls_paired(messages: Vec<Value>) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::with_capacity(messages.len() + 4);
    // tool_call_ids emitted by an assistant message but not yet answered.
    let mut pending: Vec<String> = Vec::new();

    for msg in messages.iter() {
        let role = msg["role"].as_str().unwrap_or("");

        // Before any non-tool message, flush placeholders for tool_calls
        // that were never answered — tool responses must come before the
        // next user/assistant turn.
        if role != "tool" && !pending.is_empty() {
            for call_id in &pending {
                rjlog!("[PROXY] Inserting placeholder tool result for unanswered tool_call_id: {}", call_id);
                result.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": ""
                }));
            }
            pending.clear();
        }

        match role {
            "assistant" => {
                result.push(msg.clone());
                if let Some(tool_calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tool_calls {
                        if let Some(id) = tc["id"].as_str() {
                            pending.push(id.to_string());
                        }
                    }
                }
            }
            "tool" => {
                let tool_call_id = msg["tool_call_id"].as_str().unwrap_or("");
                if !pending.iter().any(|id| id == tool_call_id) {
                    rjlog!("[PROXY] Dropping orphaned tool message (no preceding tool_calls for id: {:?})", tool_call_id);
                    continue;
                }
                // Remove only the first match so duplicate ids are handled correctly.
                if let Some(pos) = pending.iter().position(|id| id == tool_call_id) {
                    pending.remove(pos);
                }
                result.push(msg.clone());
            }
            _ => {
                result.push(msg.clone());
            }
        }
    }

    // Flush any tool_calls still unanswered at the end of the conversation.
    for call_id in &pending {
        rjlog!("[PROXY] Inserting placeholder tool result for unanswered tool_call_id: {}", call_id);
        result.push(serde_json::json!({
            "role": "tool",
            "tool_call_id": call_id,
            "content": ""
        }));
    }

    result
}

pub(crate) fn extract_model_from_path(path: &str) -> Option<&str> {
    for prefix in &["/v1/models/", "/v1beta/models/"] {
        if let Some(start) = path.find(prefix) {
            let start = start + prefix.len();
            // Gemini 路径格式: /v1beta/models/{model}:{method}?{params}
            // 模型名在第 1 个 ':'（方法分隔）、'/' 或 '?' 处结束。
            // 例如 MiniMax-M3:streamGenerateContent?alt=sse → MiniMax-M3
            let end = path[start..].find(|c: char| c == ':' || c == '/' || c == '?')
                .unwrap_or(path[start..].len());
            return Some(&path[start..start + end]);
        }
    }
    None
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::models_config::ModelEntry;

    fn m(id: &str, name: &str, key: &str) -> ModelEntry {
        ModelEntry {
            id: id.into(),
            name: name.into(),
            alias: String::new(),
            provider: String::new(),
            provider_name: String::new(),
            provider_icon: String::new(),
            api_base: "https://example.com".into(),
            api_key: key.into(),
            protocol: "openai_chat".into(),
            context_window: 0,
            support_reasoning: false,
            support_tools: true,
            tags: vec![],
            use_proxy: true,
        }
    }

    // ── find_model：三级解析优先级 ──

    #[test]
    fn test_find_model_prefers_agent_assigned_entry() {
        // 两个同名模型、agent 分配的是第二个——必须优先返回分配的那个
        let models = vec![m("a", "deepseek-v4", "key-a"), m("b", "deepseek-v4", "key-b")];
        let preferred = vec!["b".to_string()];
        let found = find_model(&models, "deepseek-v4", Some(&preferred)).unwrap();
        assert_eq!(found.id, "b");
    }

    #[test]
    fn test_find_model_alias_match() {
        let mut e = m("a", "deepseek-v4", "key-a");
        e.alias = "ds".into();
        let models = [e];
        let found = find_model(&models, "ds", None).unwrap();
        assert_eq!(found.name, "deepseek-v4");
    }

    #[test]
    fn test_find_model_fallback_to_assigned_when_name_unknown() {
        let models = vec![m("a", "real-name", "key-a")];
        let preferred = vec!["a".to_string()];
        // 请求模型名对不上时信任分配（如 agent 用默认模型名发起的请求）
        let found = find_model(&models, "whatever-requested", Some(&preferred)).unwrap();
        assert_eq!(found.name, "real-name");
    }

    #[test]
    fn test_find_model_none_when_no_match_no_assignment() {
        assert!(find_model(&[m("a", "x", "k")], "nope", None).is_none());
    }

    // ── ensure_tool_calls_paired：tool_calls ↔ tool 消息配对修复 ──

    #[test]
    fn test_ensure_tool_calls_paired_inserts_placeholder() {
        // assistant 发起 tool_call 但没有 tool 响应 → 下一条非 tool 消息前补占位
        let msgs: Vec<Value> = vec![
            serde_json::json!({"role": "user", "content": "hi"}),
            serde_json::json!({"role": "assistant", "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}
            ]}),
            serde_json::json!({"role": "user", "content": "next"}),
        ];
        let out = ensure_tool_calls_paired(msgs);
        let roles: Vec<&str> = out.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, vec!["user", "assistant", "tool", "user"]);
        assert_eq!(out[2]["tool_call_id"], "call_1");
    }

    #[test]
    fn test_ensure_tool_calls_paired_drops_orphan_tool() {
        // 孤儿 tool 消息（无前置 tool_calls）→ 丢弃，否则上游 400
        let msgs: Vec<Value> = vec![
            serde_json::json!({"role": "user", "content": "hi"}),
            serde_json::json!({"role": "tool", "tool_call_id": "ghost", "content": "x"}),
            serde_json::json!({"role": "assistant", "content": "done"}),
        ];
        let out = ensure_tool_calls_paired(msgs);
        let roles: Vec<&str> = out.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, vec!["user", "assistant"]);
    }

    #[test]
    fn test_ensure_tool_calls_paired_keeps_valid_pair() {
        let msgs: Vec<Value> = vec![
            serde_json::json!({"role": "assistant", "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}
            ]}),
            serde_json::json!({"role": "tool", "tool_call_id": "call_1", "content": "result"}),
        ];
        let out = ensure_tool_calls_paired(msgs);
        assert_eq!(out.len(), 2, "合法配对不应被改动");
    }

    // ── extract_model_from_path：Gemini 路径解析 ──

    #[test]
    fn test_extract_model_from_path() {
        assert_eq!(extract_model_from_path("/v1beta/models/gemini-2.5-pro:generateContent"), Some("gemini-2.5-pro"));
        assert_eq!(extract_model_from_path("/v1beta/models/MiniMax-M3:streamGenerateContent?alt=sse"), Some("MiniMax-M3"));
        assert_eq!(extract_model_from_path("/v1/models/gemini-2.5-flash:generateContent"), Some("gemini-2.5-flash"));
        assert_eq!(extract_model_from_path("/v1/chat/completions"), None);
    }

    // ── safe_truncate：多字节安全截断 ──

    #[test]
    fn test_safe_truncate_multibyte() {
        assert_eq!(safe_truncate("你好世界", 6), "你好");
        assert_eq!(safe_truncate("你好世界", 7), "你好", "7 字节落在'世'中间，必须回退到字符边界");
        assert_eq!(safe_truncate("你好世界", 100), "你好世界");
        assert_eq!(safe_truncate("abc", 2), "ab");
    }
}
