//! Proxy 内部共享工具：模型路由、SSE 转换 reader、消息规整等。

use crate::models_config::ModelEntry;
use crate::rjlog;
use crate::rjlogd;
use serde_json::Value;
use std::collections::HashSet;
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

/// Claude 专属内置工具（依赖 Anthropic 服务端能力或 Claude Code 特有机制），
/// 对第三方模型（Qwen/DeepSeek/Gemini/火山引擎等）不兼容：
/// - WebFetch / WebSearch：依赖 Anthropic 服务端抓取。第三方模型不理解其 schema，
///   常生成空参数调用（日志可见 tool_call_update input_len=0），CLI 侧校验失败报
///   "InputValidationError: The required parameter `url` is missing"，模型反复重试
///   还会形成死循环刷爆日志。
/// - AskUserQuestion：交互式提问工具，RunJam 前端不支持展示，第三方模型一旦调用，
///   会话会卡在等待用户输入。
/// - Task（subagent）：Claude Code 的派生子任务工具。第三方模型生成的子任务提示词
///   质量不可控、subagent 对话同样走代理易失败，对小模型简化工具面更稳。
///   如需 subagent 能力可移出此名单。
/// 注意：Claude 官方模型走 passthrough（不进工具转换），不受此名单影响。
pub(crate) const CLAUDE_ONLY_TOOLS: &[&str] = &["WebFetch", "WebSearch", "AskUserQuestion", "Task"];

/// 判断工具名是否为 Claude 专属工具（大小写不敏感）。
pub(crate) fn is_claude_only_tool(name: &str) -> bool {
    CLAUDE_ONLY_TOOLS.iter().any(|t| t.eq_ignore_ascii_case(name))
}

/// 追加到第三方模型系统提示词尾部的工具调用规范（仅当请求启用了工具）。
///
/// 背景：Bash 是核心工具不能剥离，但 Qwen 等模型调用 Bash 时经常漏掉必填的
/// `command` 参数（日志可见 "InputValidationError: The required parameter
/// `command` is missing"），CLI 校验失败后模型还会反复重试。在系统提示词中
/// 明确约束能让模型在生成阶段就带上完整的 command。
pub(crate) const BASH_TOOL_GUIDANCE: &str =
    "\n\nTool usage requirement: when calling the Bash tool, the \"command\" parameter is REQUIRED. Always provide a complete, non-empty shell command string (e.g. \"ls -la\"). Never call Bash without a command or with an empty command string.";

/// 向 system 提示追加 Bash 调用规范（幂等：已包含标记文本则不重复追加）。
pub(crate) fn append_bash_guidance(system: &str) -> String {
    if system.contains("Tool usage requirement") {
        system.to_string()
    } else {
        format!("{}{}", system, BASH_TOOL_GUIDANCE)
    }
}

/// 强化 Bash 工具的 OpenAI 格式定义，缓解第三方模型漏传 `command` 参数：
/// - 确保 `properties.command` 存在且为 string，description 明确要求完整非空命令；
/// - 确保 `required` 数组包含 `command`；
/// - 其余字段原样保留，不影响 Claude Code 侧执行。
/// 非 Bash 工具原样返回。
pub(crate) fn harden_bash_tool(tool: &Value) -> Value {
    let mut t = tool.clone();
    let name = t["function"]["name"].as_str().unwrap_or("");
    if !name.eq_ignore_ascii_case("bash") {
        return t;
    }
    if !t["function"]["parameters"].is_object() {
        t["function"]["parameters"] = serde_json::json!({"type": "object", "properties": {}});
    }
    let params = t["function"]["parameters"].as_object_mut().unwrap();
    let props = params.entry("properties").or_insert_with(|| serde_json::json!({}));
    if !props.is_object() {
        *props = serde_json::json!({});
    }
    let props = props.as_object_mut().unwrap();
    let command = props.entry("command").or_insert_with(|| serde_json::json!({"type": "string"}));
    if !command.is_object() {
        *command = serde_json::json!({"type": "string"});
    }
    let cmd = command.as_object_mut().unwrap();
    cmd.insert("type".into(), serde_json::json!("string"));
    let desc = cmd.get("description").and_then(|v| v.as_str()).unwrap_or("The bash command to execute");
    cmd.insert(
        "description".into(),
        serde_json::json!(format!(
            "{} REQUIRED: must be a complete, non-empty shell command string (e.g. \"ls -la\"). Never omit it or pass an empty string.",
            desc
        )),
    );
    let required = params.entry("required").or_insert_with(|| serde_json::json!([]));
    if !required.is_array() {
        *required = serde_json::json!([]);
    }
    let req_arr = required.as_array_mut().unwrap();
    if !req_arr.iter().any(|v| v.as_str() == Some("command")) {
        req_arr.push(serde_json::json!("command"));
    }
    t
}

/// 批量应用 Bash 工具强化（就地修改）。
pub(crate) fn apply_bash_tool_hardening(tools: &mut Vec<Value>) {
    for t in tools.iter_mut() {
        *t = harden_bash_tool(t);
    }
}

/// 提取 tool call 的 `arguments` 字符串。
///
/// OpenAI 规范要求 `arguments` 是 JSON 字符串；但不少兼容端点（OpenRouter
/// 免费模型、部分本地网关等）直接把 JSON 对象放在 `arguments` 字段。只按
/// 字符串解析会静默丢弃参数——CLI 收到的 tool_use input 为空对象，于是报
/// "The required parameter `command` is missing"（Bash）之类校验错误。
/// 本函数兼容两种形态：
/// - 非空字符串：原样返回；
/// - 非空对象：序列化为 JSON 字符串；
/// - 空字符串 / 空对象 / null / 缺失：返回 None（视作参数缺失，交由上层兜底）。
pub(crate) fn tool_call_args_str(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        if s.trim().is_empty() {
            return None;
        }
        return Some(s.to_string());
    }
    if let Some(obj) = v.as_object() {
        if obj.is_empty() {
            return None;
        }
        return Some(serde_json::to_string(obj).unwrap_or_default());
    }
    None
}

/// 同 `tool_call_args_str`，但显式给出空参数（空字符串/空对象）时记录诊断日志，
/// 用于区分两类问题：模型真的没生成参数 vs 转换层丢失参数。
/// `ctx` 描述调用场景（如 "anthropic streaming"），仅空参数时打日志，不刷屏。
pub(crate) fn tool_call_args_str_tracked(v: &Value, tool_name: &str, ctx: &str) -> Option<String> {
    let r = tool_call_args_str(v);
    if r.is_none() && !v.is_null() {
        let preview = match v {
            Value::String(_) => "empty string".to_string(),
            Value::Object(o) => format!("empty object ({} keys)", o.len()),
            _ => format!("value={}", v),
        };
        rjlog!("[PROXY] {}: tool '{}' arguments {} → dropped (CLI may report 'required parameter missing')", ctx, tool_name, preview);
    }
    r
}

/// 在 OpenAI 格式请求 body 的 messages 中注入 Bash 调用规范（幂等）。
/// 若首条已是 system 则在其尾部追加；否则在最前插入一条 system 消息。
pub(crate) fn inject_bash_guidance(body: &mut Value) {
    let msgs = body.get_mut("messages").and_then(|v| v.as_array_mut());
    if let Some(msgs) = msgs {
        if let Some(first) = msgs.first_mut() {
            if first["role"].as_str() == Some("system") {
                let c = first["content"].as_str().unwrap_or("").to_string();
                first["content"] = serde_json::json!(append_bash_guidance(&c));
            } else {
                msgs.insert(0, serde_json::json!({"role": "system", "content": append_bash_guidance("You are a helpful assistant.")}));
            }
        } else {
            msgs.insert(0, serde_json::json!({"role": "system", "content": append_bash_guidance("You are a helpful assistant.")}));
        }
    }
}

/// 将请求的 max_tokens 限制到模型配置的 context_window 内。
/// 部分小窗口模型（如 8K/16K）收到远超其能力的 max_tokens（如 32000）会直接报 400；
/// context_window=0（未配置）时不作限制，返回原值。
pub(crate) fn clamp_max_tokens(max_tokens: u64, context_window: u64) -> u64 {
    if context_window > 0 && max_tokens > context_window {
        context_window
    } else {
        max_tokens
    }
}

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
///    has no following `tool` response (e.g. the agent replayed a tool_call
///    batch while those tools were still executing, or the history is
///    malformed). The unpaired `tool_calls` are *removed from the assistant
///    message* instead of inventing fake empty tool results — an empty
///    placeholder would make the model believe the tool returned nothing
///    (Codex 会回复"所有命令都返回了空结果"，而界面显示的是真实输出).
///
/// 2. "Messages with role 'tool' must be a response to a preceding message
///    with 'tool_calls'"
///    — a `tool` message exists whose `tool_call_id` doesn't match any
///    `tool_calls` from a preceding assistant message (e.g. the assistant
///    turn was dropped during conversion, or the history is malformed).
///    The orphaned `tool` message is dropped.
pub(crate) fn ensure_tool_calls_paired(messages: Vec<Value>) -> Vec<Value> {
    // First pass: collect every tool_call_id that actually has a tool result
    // somewhere in this history. Only these may survive on their assistant
    // message — a tool_call without any result must not be forwarded, and it
    // must not be "answered" with a fake empty tool response either.
    let has_result: HashSet<String> = messages
        .iter()
        .filter(|m| m["role"].as_str() == Some("tool"))
        .filter_map(|m| m["tool_call_id"].as_str().map(|s| s.to_string()))
        .collect();

    let mut result: Vec<Value> = Vec::with_capacity(messages.len());
    // call_ids declared by the current assistant message but not yet paired
    // with their tool response as we walk the history in order.
    let mut active: HashSet<String> = HashSet::new();

    for msg in messages.iter() {
        let role = msg["role"].as_str().unwrap_or("");
        match role {
            "assistant" => {
                let mut m = msg.clone();
                if let Some(tool_calls) = m.get_mut("tool_calls").and_then(|v| v.as_array_mut()) {
                    // Keep only tool_calls that have a real tool result in the
                    // history. Empty or unmatched call_ids (e.g. a replayed
                    // batch whose results haven't arrived yet) are dropped —
                    // never turned into fake empty tool responses.
                    let kept: Vec<Value> = tool_calls
                        .iter()
                        .filter(|tc| {
                            let id = tc["id"].as_str().unwrap_or("");
                            !id.is_empty() && has_result.contains(id)
                        })
                        .cloned()
                        .collect();
                    active.clear();
                    for tc in &kept {
                        if let Some(id) = tc["id"].as_str() {
                            active.insert(id.to_string());
                        }
                    }
                    if kept.is_empty() {
                        if let Some(obj) = m.as_object_mut() {
                            obj.remove("tool_calls");
                        }
                    } else {
                        *tool_calls = kept;
                    }
                }
                // assistant 移除全部 tool_calls 后若没有正文，整条丢弃：
                // content 为空的 assistant 消息会让部分兼容端点（火山引擎 ark）
                // 报 InvalidParameter。
                let content = m["content"].as_str().unwrap_or("");
                let has_text = m["content"].is_array() && !m["content"].as_array().map(|a| a.is_empty()).unwrap_or(true);
                if m.get("tool_calls").is_none() && content.trim().is_empty() && !has_text {
                    rjlog!("[PROXY] Dropping empty assistant message (no content, no tool_calls)");
                    continue;
                }
                result.push(m);
            }
            "tool" => {
                let tool_call_id = msg["tool_call_id"].as_str().unwrap_or("");
                if tool_call_id.is_empty() || !active.contains(tool_call_id) {
                    rjlog!(
                        "[PROXY] Dropping orphaned tool message (no preceding tool_calls for id: {:?})",
                        tool_call_id
                    );
                    continue;
                }
                active.remove(tool_call_id);
                result.push(msg.clone());
            }
            _ => {
                result.push(msg.clone());
            }
        }
    }

    result
}

/// 消息序列最终规范化——OpenAI 兼容端点（火山引擎 ark、DashScope 等）对
/// 相邻同 role 消息非常严格：
/// - 相邻两条 system：部分端点报 InvalidParameter（Codex 的 `<permissions
///   instructions>` 沙箱指令在历史重放时容易重复出现）；
/// - 相邻两条 assistant：报 "insufficient tool messages" 或 InvalidParameter
///   （Codex 工具未完成时发起请求 / 历史重放时容易出现）。
/// 必须在 `ensure_tool_calls_paired` 之后调用：先保证 tool_calls↔tool 配对，
/// 再合并相邻同 role 消息——合并不会破坏配对（tool_calls 只是 union，
/// tool 消息顺序与内容不变）。
/// 合并规则：
/// 1. system + system → content 拼接（换行分隔）；
/// 2. assistant + assistant → content 拼接 + tool_calls 按 id 去重 union；
/// 3. 其余消息原样保留。
pub(crate) fn normalize_message_sequence(messages: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(messages.len());
    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("");
        let can_merge = role == "system" || role == "assistant";
        if can_merge {
            if let Some(last) = out.last_mut() {
                if last["role"].as_str() == Some(role) {
                    merge_adjacent_message(last, &msg);
                    continue;
                }
            }
        }
        out.push(msg);
    }
    out
}

/// 将 `src` 的消息内容合并进 `target`（就地修改 target）。
/// - content：字符串形态拼接（换行分隔，跳过空段）；target 无 content 时直接补上；
/// - tool_calls（仅 assistant）：按 id 去重追加到 target 的 tool_calls。
fn merge_adjacent_message(target: &mut Value, src: &Value) {
    let t_content = target.get("content");
    let s_content = src.get("content");
    let t_str = t_content.and_then(|v| v.as_str());
    let s_str = s_content.and_then(|v| v.as_str());
    match (t_str, s_str) {
        (Some(ts), Some(ss)) => {
            if !ss.trim().is_empty() {
                let joined = if ts.trim().is_empty() {
                    ss.to_string()
                } else {
                    format!("{}\n{}", ts, ss)
                };
                target["content"] = serde_json::json!(joined);
            }
        }
        (None, Some(_)) => {
            target["content"] = s_content.unwrap().clone();
        }
        _ => {}
    }
    if target["role"].as_str() == Some("assistant") {
        if let Some(src_tcs) = src.get("tool_calls").and_then(|v| v.as_array()) {
            if src_tcs.is_empty() {
                return;
            }
            if let Some(dst) = target.get_mut("tool_calls").and_then(|v| v.as_array_mut()) {
                for tc in src_tcs {
                    let id = tc["id"].as_str().unwrap_or("");
                    if !id.is_empty() && !dst.iter().any(|x| x["id"].as_str() == Some(id)) {
                        dst.push(tc.clone());
                    }
                }
            } else {
                target["tool_calls"] = serde_json::json!(src_tcs.clone());
            }
        }
    }
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
    fn test_ensure_tool_calls_paired_removes_unpaired_tool_calls() {
        // assistant 发起 tool_call 但没有 tool 响应（工具仍在执行/历史重放）→
        // 移除未配对的 tool_calls，而不是插入空占位符：空占位符会让模型
        // 误以为"工具返回了空结果"（Codex 现象：界面有输出但回复"所有命令
        // 都返回了空结果"）。移除后 assistant 无正文无 tool_calls，整条丢弃
        // （空 assistant 消息会让火山引擎等端点报 InvalidParameter）。
        let msgs: Vec<Value> = vec![
            serde_json::json!({"role": "user", "content": "hi"}),
            serde_json::json!({"role": "assistant", "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}
            ]}),
            serde_json::json!({"role": "user", "content": "next"}),
        ];
        let out = ensure_tool_calls_paired(msgs);
        let roles: Vec<&str> = out.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, vec!["user", "user"], "空 assistant 消息应被丢弃");
        assert!(out.iter().all(|m| m["role"].as_str() != Some("assistant")));
    }

    #[test]
    fn test_ensure_tool_calls_paired_keeps_text_assistant_without_calls() {
        // assistant 有正文但 tool_calls 无结果 → 保留正文、移除 tool_calls
        let msgs: Vec<Value> = vec![
            serde_json::json!({"role": "user", "content": "hi"}),
            serde_json::json!({"role": "assistant", "content": "好的，我来搜索。", "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}}
            ]}),
            serde_json::json!({"role": "user", "content": "next"}),
        ];
        let out = ensure_tool_calls_paired(msgs);
        let roles: Vec<&str> = out.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert!(out[1].get("tool_calls").is_none(), "未配对的 tool_calls 应被移除");
        assert_eq!(out[1]["content"], "好的，我来搜索。", "正文应保留");
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
    fn test_ensure_tool_calls_paired_keeps_only_answered() {
        // assistant 带 3 个 tool_calls，只有 2 个有 tool 结果 → 只保留已配对的
        // （对应 Codex 并行工具执行中、部分结果未返回的历史重放场景）
        let msgs: Vec<Value> = vec![
            serde_json::json!({"role": "user", "content": "hi"}),
            serde_json::json!({"role": "assistant", "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}},
                {"id": "call_2", "type": "function", "function": {"name": "f", "arguments": "{}"}},
                {"id": "call_3", "type": "function", "function": {"name": "f", "arguments": "{}"}}
            ]}),
            serde_json::json!({"role": "tool", "tool_call_id": "call_1", "content": "r1"}),
            serde_json::json!({"role": "tool", "tool_call_id": "call_2", "content": "r2"}),
            serde_json::json!({"role": "user", "content": "next"}),
        ];
        let out = ensure_tool_calls_paired(msgs);
        let roles: Vec<&str> = out.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, vec!["user", "assistant", "tool", "tool", "user"]);
        let ids: Vec<&str> = out[1]["tool_calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tc| tc["id"].as_str())
            .collect();
        assert_eq!(ids, vec!["call_1", "call_2"], "无结果的 call_3 应被移除");
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

    // ── harden_bash_tool：Bash 工具 command 必填强化 ──

    #[test]
    fn test_harden_bash_tool_adds_required_command() {
        // 模拟 Claude Code 的 Bash 工具定义（properties 缺失 command、required 为空）
        let tool = serde_json::json!({
            "type": "function",
            "function": {
                "name": "Bash",
                "description": "Run a bash command",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "description": {"type": "string"}
                    },
                    "required": []
                }
            }
        });
        let out = harden_bash_tool(&tool);
        let params = &out["function"]["parameters"];
        // command 属性被补全且为 string
        assert_eq!(params["properties"]["command"]["type"], "string");
        // required 数组包含 command
        let required = params["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("command")));
        // description 明确要求完整非空命令
        let desc = params["properties"]["command"]["description"].as_str().unwrap();
        assert!(desc.to_lowercase().contains("non-empty"));
    }

    #[test]
    fn test_harden_bash_tool_keeps_other_tools_unchanged() {
        let tool = serde_json::json!({
            "type": "function",
            "function": {"name": "Read", "description": "d", "parameters": {"type": "object", "properties": {}}}
        });
        assert_eq!(harden_bash_tool(&tool), tool);
    }

    #[test]
    fn test_append_bash_guidance_idempotent() {
        let once = append_bash_guidance("sys");
        let twice = append_bash_guidance(&once);
        assert!(once.contains("command") && once.contains("REQUIRED"));
        assert_eq!(once, twice, "重复追加必须幂等");
    }

    #[test]
    fn test_inject_bash_guidance_appends_to_existing_system() {
        let mut body = serde_json::json!({
            "messages": [{"role": "system", "content": "you are helpful"}]
        });
        inject_bash_guidance(&mut body);
        let content = body["messages"][0]["content"].as_str().unwrap().to_string();
        assert!(content.starts_with("you are helpful"));
        assert!(content.contains("Tool usage requirement"));
        // 再次注入不重复（幂等）
        inject_bash_guidance(&mut body);
        let again = body["messages"][0]["content"].as_str().unwrap();
        assert_eq!(again.matches("Tool usage requirement").count(), 1);
    }

    // ── tool_call_args_str：arguments 兼容字符串与 JSON 对象形态 ──

    #[test]
    fn test_tool_call_args_str_string_form() {
        // OpenAI 规范：字符串形态原样返回
        let v = serde_json::json!("{\"command\": \"ls -la\"}");
        assert_eq!(tool_call_args_str(&v).as_deref(), Some("{\"command\": \"ls -la\"}"));
    }

    #[test]
    fn test_tool_call_args_str_object_form() {
        // 兼容端点：对象形态序列化为 JSON 字符串（此场景是之前 Bash 缺 command 的元凶之一）
        let v = serde_json::json!({"command": "ls -la", "description": "list files"});
        let s = tool_call_args_str(&v).unwrap();
        let parsed: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["command"], "ls -la");
        assert_eq!(parsed["description"], "list files");
    }

    #[test]
    fn test_tool_call_args_str_empty_forms_none() {
        assert_eq!(tool_call_args_str(&serde_json::json!("")), None);
        assert_eq!(tool_call_args_str(&serde_json::json!("  ")), None);
        assert_eq!(tool_call_args_str(&serde_json::json!({})), None);
        assert_eq!(tool_call_args_str(&serde_json::Value::Null), None);
    }

    #[test]
    fn test_tool_call_args_str_tracked_returns_same() {
        let ok = serde_json::json!("{\"command\": \"pwd\"}");
        assert_eq!(tool_call_args_str_tracked(&ok, "Bash", "test"), Some("{\"command\": \"pwd\"}".to_string()));
        // 空对象 → None（并触发诊断日志）
        assert_eq!(tool_call_args_str_tracked(&serde_json::json!({}), "Bash", "test"), None);
        // 缺失（Null）→ None 但不打日志
        assert_eq!(tool_call_args_str_tracked(&serde_json::Value::Null, "Bash", "test"), None);
    }

    #[test]
    fn test_inject_bash_guidance_prepends_system_if_absent() {
        let mut body = serde_json::json!({
            "messages": [{"role": "user", "content": "hi"}]
        });
        inject_bash_guidance(&mut body);
        let first = &body["messages"][0];
        assert_eq!(first["role"], "system");
        assert!(first["content"].as_str().unwrap().contains("Bash"));
    }

    #[test]
    fn test_normalize_merges_adjacent_system() {
        let msgs: Vec<Value> = vec![
            serde_json::json!({"role": "system", "content": "A"}),
            serde_json::json!({"role": "system", "content": "B"}),
            serde_json::json!({"role": "user", "content": "hi"}),
        ];
        let out = normalize_message_sequence(msgs);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["role"], "system");
        assert_eq!(out[0]["content"], "A\nB");
        assert_eq!(out[1]["role"], "user");
    }

    #[test]
    fn test_normalize_merges_adjacent_assistant_dedup_tool_calls() {
        let msgs: Vec<Value> = vec![
            serde_json::json!({"role": "user", "content": "hi"}),
            serde_json::json!({"role": "assistant", "content": "思考中", "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}
            ]}),
            serde_json::json!({"role": "assistant", "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}},
                {"id": "c2", "type": "function", "function": {"name": "g", "arguments": "{}"}}
            ]}),
            serde_json::json!({"role": "tool", "tool_call_id": "c1", "content": "r1"}),
            serde_json::json!({"role": "tool", "tool_call_id": "c2", "content": "r2"}),
        ];
        let out = normalize_message_sequence(msgs);
        assert_eq!(out.len(), 4);
        assert_eq!(out[1]["role"], "assistant");
        assert!(out[1]["content"].as_str().unwrap().contains("思考中"));
        let ids: Vec<&str> = out[1]["tool_calls"].as_array().unwrap().iter()
            .filter_map(|t| t["id"].as_str()).collect();
        assert_eq!(ids, vec!["c1", "c2"], "c1 去重，c2 追加");
    }

    #[test]
    fn test_normalize_keeps_non_adjacent() {
        let msgs: Vec<Value> = vec![
            serde_json::json!({"role": "system", "content": "S"}),
            serde_json::json!({"role": "user", "content": "u"}),
            serde_json::json!({"role": "system", "content": "S2"}),
        ];
        let out = normalize_message_sequence(msgs);
        assert_eq!(out.len(), 3, "中间隔了 user，不应合并");
    }

    #[test]
    fn test_normalize_skips_tool_and_user() {
        let msgs: Vec<Value> = vec![
            serde_json::json!({"role": "tool", "tool_call_id": "c1", "content": "r1"}),
            serde_json::json!({"role": "tool", "tool_call_id": "c2", "content": "r2"}),
        ];
        let out = normalize_message_sequence(msgs);
        assert_eq!(out.len(), 2, "tool 消息不合并");
    }
}
