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

/// 上游 400 错误体是否在抱怨 reasoning_effort。
///
/// 企业内部网关（如 llm-api.patsnap.info）在 `/v1/chat/completions` 上有一套
/// 非标准行为：**只要请求带 function tools，就必须显式传
/// `reasoning_effort: "none"`**，不传即 400：
///   "Function tools with reasoning_effort are not supported for <model>
///    in /v1/chat/completions. To use function tools, use /v1/responses or
///    set reasoning_effort to 'none'."
/// 报文里 `"param":"reasoning_effort"` 容易误读成"代理发错了字段"，实际是
/// 网关自报缺少该字段。此处仅做**字符串嗅探**，命中即触发一次降级重试。
pub(crate) fn upstream_rejects_missing_reasoning_effort(err_body: &str) -> bool {
    err_body.to_lowercase().contains("reasoning_effort")
}

/// 给 OpenAI Chat 请求体注入 `reasoning_effort: "none"`。
///
/// 仅在请求**确实带 tools**且顶层尚未设置 reasoning_effort 时注入：
/// - 该网关的 400 只在带 tools 时出现，无 tools 时上游并不校验，凭空多送一个
///   非标准字段反而可能被其他严格端点（火山引擎 ark、DeepSeek）拒绝；
/// - 顶层已有 reasoning_effort（透传的客户端意图）时尊重原值。
/// 返回 None 表示无需/无法注入，调用方应沿用原 body。
pub(crate) fn apply_reasoning_none(body: &str) -> Option<String> {
    let mut v: Value = serde_json::from_str(body).ok()?;
    let obj = v.as_object_mut()?;
    let has_tools = obj
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    if !has_tools || obj.contains_key("reasoning_effort") {
        return None;
    }
    obj.insert("reasoning_effort".into(), serde_json::json!("none"));
    Some(v.to_string())
}

/// 发往 OpenAI Chat Completions 上游的结果：与 `ureq::Error` 区分开，因为
/// 自动重试需要在读掉错误体后仍能把状态码/错误体交回调用方。
pub(crate) enum ChatUpstream {
    Ok(ureq::Response),
    /// 上游返回非 2xx（body 已读出）
    Http { status: u16, body: String },
    /// 连接/传输层错误
    Transport(ureq::Error),
}

/// 发送 OpenAI Chat Completions 请求，内置企业网关的 reasoning_effort 兜底。
///
/// 企业内网 LLM 接口常看不出是什么协议，也不遵循 OpenAI 标准：网关可能强制
/// 要求带 tools 时显式 `reasoning_effort: "none"`，但错误信息里只会说
/// "reasoning_effort ... not supported"，看起来像是**我方多发了**该字段。
/// 这里做两件事：
/// 1. `inject_reasoning_none=true`（模型配置开关或用户关闭推理）时，发送前
///    就给带 tools 的请求补上 `reasoning_effort: "none"`；
/// 2. 上游仍以 400 拒绝且错误体提及 reasoning_effort 时，注入该字段**重试一次**
///    （只重试一次，第二次失败照常返回错误，无死循环风险）。
pub(crate) fn send_chat_completions(
    url: &str,
    api_key: &str,
    body: &str,
    inject_reasoning_none: bool,
) -> ChatUpstream {
    let agent = build_agent();
    let send = |b: &str| {
        agent
            .post(url)
            .set("Authorization", &format!("Bearer {}", api_key))
            .set("Content-Type", "application/json")
            .send_string(b)
    };

    let mut out_body = body.to_string();
    if inject_reasoning_none {
        if let Some(b) = apply_reasoning_none(body) {
            rjlog!("[PROXY] injecting reasoning_effort=\"none\" (tools present; upstream requires explicit disable)");
            out_body = b;
        }
    }

    match send(&out_body) {
        Ok(r) => ChatUpstream::Ok(r),
        Err(ureq::Error::Status(status, r)) => {
            let err_body = r.into_string().unwrap_or_default();
            if status == 400 && upstream_rejects_missing_reasoning_effort(&err_body) {
                if let Some(retry_body) = apply_reasoning_none(&out_body) {
                    rjlog!("[PROXY] upstream HTTP 400 mentions reasoning_effort — retrying once with reasoning_effort=\"none\"");
                    return match send(&retry_body) {
                        Ok(r) => ChatUpstream::Ok(r),
                        Err(ureq::Error::Status(st, r)) => ChatUpstream::Http {
                            status: st,
                            body: r.into_string().unwrap_or_default(),
                        },
                        Err(e) => ChatUpstream::Transport(e),
                    };
                }
            }
            ChatUpstream::Http { status, body: err_body }
        }
        Err(e) => ChatUpstream::Transport(e),
    }
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
            use_proxy: true, force_reasoning_none: false,
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

    // ── reasoning_effort 企业网关兜底（llm-api.patsnap.info 等） ──

    #[test]
    fn test_rejects_missing_reasoning_effort_sniffs_error_body() {
        // 网关真实报文：错误里提到 reasoning_effort（即使我方并未发送该字段）
        let body = r#"{"error":{"http_code":400,"code":0,"message":"Function tools with reasoning_effort are not supported for gpt-6-luna in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'.","param":"reasoning_effort","type":"invalid_request_error"}}"#;
        assert!(upstream_rejects_missing_reasoning_effort(body));
        // 大小写不敏感
        assert!(upstream_rejects_missing_reasoning_effort("REASONING_EFFORT not allowed"));
        // 无关错误不误判
        assert!(!upstream_rejects_missing_reasoning_effort(r#"{"error":{"message":"invalid api key"}}"#));
    }

    #[test]
    fn test_apply_reasoning_none_injects_when_tools_present() {
        let body = r#"{"model":"m","messages":[],"tools":[{"type":"function","function":{"name":"x"}}]}"#;
        let out = apply_reasoning_none(body).expect("带 tools 应注入");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["reasoning_effort"], "none");
    }

    #[test]
    fn test_apply_reasoning_none_skips_without_tools() {
        // 无 tools 时上游不校验：凭空多送非标准字段可能被其他严格端点拒绝
        let body = r#"{"model":"m","messages":[]}"#;
        assert!(apply_reasoning_none(body).is_none());
        // 空 tools 数组同样跳过
        let body2 = r#"{"model":"m","messages":[],"tools":[]}"#;
        assert!(apply_reasoning_none(body2).is_none());
    }

    #[test]
    fn test_apply_reasoning_none_respects_existing_value() {
        // 客户端/透传已显式设置 reasoning_effort 时不得覆盖（如 "low"）
        let body = r#"{"model":"m","tools":[{"type":"function","function":{"name":"x"}}],"reasoning_effort":"low"}"#;
        assert!(apply_reasoning_none(body).is_none());
    }

    #[test]
    fn test_apply_reasoning_none_invalid_json() {
        assert!(apply_reasoning_none("not json").is_none());
    }

    // ── send_chat_completions：真实 HTTP 层（含 400 自动重试） ──

    /// 带超时地取出 stub 上游收到的下一个请求体。用超时而不用 `recv()`：
    /// 入口若因参数不合法提前返回（根本没发出请求），无超时的 recv 会永久
    /// 阻塞并让整个 test binary 挂住（表现为 cargo test 永不结束）。
    fn recv_body(rx: &std::sync::mpsc::Receiver<String>) -> String {
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("5 秒内应收到上游请求（入口可能提前返回而未发出请求）")
    }

    /// 断言短期内没有额外请求到达（比 try_recv 更可靠：留出传输延迟余量）。
    fn assert_no_more_requests(rx: &std::sync::mpsc::Receiver<String>) {
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(),
            "不应发生额外请求"
        );
    }

    /// 极简一次性 HTTP 服务器：首个请求按 `first_status` 返回 `first_body`，
    /// 之后一律返回 200 `{"ok":true}`；把收到的 body 通过 channel 传出。
    fn spawn_stub_upstream(
        first_status: u16,
        first_body: &'static str,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            for (i, stream) in listener.incoming().enumerate() {
                let Ok(mut s) = stream else { break };
                let mut reader = BufReader::new(s.try_clone().unwrap());
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 { break; }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    if line == "\r\n" { break; }
                }
                let mut buf = vec![0u8; len];
                if len > 0 { std::io::Read::read_exact(&mut reader, &mut buf).unwrap(); }
                let _ = tx.send(String::from_utf8_lossy(&buf).to_string());

                let (status, body) = if i == 0 {
                    (first_status, first_body.to_string())
                } else {
                    (200, r#"{"ok":true}"#.to_string())
                };
                let resp = format!(
                    "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    status, body.len(), body
                );
                let _ = s.write_all(resp.as_bytes());
                let _ = s.flush();
            }
        });
        (format!("http://{}/v1/chat/completions", addr), rx)
    }

    #[test]
    fn test_send_chat_completions_retries_once_on_reasoning_effort_400() {
        // 网关典型行为：第一次因缺 reasoning_effort 报 400，显式传 "none" 后成功。
        // 这里 stub 无条件按序号应答，正是要验证客户端确实重试了、且重试体带上了字段。
        let err = r#"{"error":{"message":"Function tools with reasoning_effort are not supported for gpt-6-luna ... set reasoning_effort to 'none'.","param":"reasoning_effort"}}"#;
        let (url, rx) = spawn_stub_upstream(400, err);
        let body = r#"{"model":"gpt-6-luna","messages":[],"tools":[{"type":"function","function":{"name":"x"}}]}"#;

        let out = send_chat_completions(&url, "sk-test", body, false);
        assert!(matches!(out, ChatUpstream::Ok(_)), "重试后应成功");

        let first = recv_body(&rx);
        assert!(!first.contains("reasoning_effort"), "首次请求原样透传（不带注入开关）");
        let second = recv_body(&rx);
        let v: Value = serde_json::from_str(&second).unwrap();
        assert_eq!(v["reasoning_effort"], "none", "重试请求应注入 reasoning_effort=none");
    }

    #[test]
    fn test_send_chat_completions_no_retry_for_unrelated_400() {
        // 400 与 reasoning_effort 无关（如 key 无效）→ 不重试，原样返回错误体
        let err = r#"{"error":{"message":"invalid api key"}}"#;
        let (url, rx) = spawn_stub_upstream(400, err);
        let body = r#"{"model":"m","messages":[],"tools":[{"type":"function","function":{"name":"x"}}]}"#;

        let out = send_chat_completions(&url, "sk-test", body, false);
        match out {
            ChatUpstream::Http { status, body } => {
                assert_eq!(status, 400);
                assert!(body.contains("invalid api key"));
            }
            _ => panic!("应为 Http(400)"),
        }
        let _ = recv_body(&rx);
        assert_no_more_requests(&rx);
    }

    // ── 请求体形态：企业网关兜底在各入口装配出的最终 body ──
    //
    // 说明：这里刻意不经过真实 HTTP（不启 stub 上游）。原因有二：
    // 1) `proxy_openai_direct` 内部用 `ModelConfig::load()` 读取磁盘配置，而非
    //    调用方传入的 models 切片（既有设计），直接调用会绕过本测试构造的模型；
    // 2) 真实 HTTP stub 在这些测试里引入的端口/线程/超时复杂度，远大于它所能
    //    验证的额外信息——真正的 HTTP 重试链路已由上面的
    //    `test_send_chat_completions_retries_once_on_reasoning_effort_400` 覆盖。
    // 因此此处固定住「最终 body 里 reasoning_effort 的有无」这一契约。

    /// 带 tools 的 OpenAI Chat body：任何入口在强制/关闭推理时都必须注入 none。
    fn chat_body_with_tools() -> String {
        serde_json::json!({
            "model": "stub-model",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {"name": "get_weather"}}]
        }).to_string()
    }

    #[test]
    fn test_apply_reasoning_none_contract_for_all_entry_shapes() {
        // 四类入口最终都会汇成 OpenAI Chat body，注入规则只有一条：
        // 带 tools 且未显式设置 reasoning_effort 时补 "none"。
        assert_eq!(
            serde_json::from_str::<Value>(&apply_reasoning_none(&chat_body_with_tools()).unwrap()).unwrap()["reasoning_effort"],
            "none"
        );
        // 无 tools 的各种形态：绝不注入
        for body in [
            r#"{"model":"m","messages":[]}"#,
            r#"{"model":"m","messages":[],"tools":[]}"#,
            r#"{"model":"m","messages":[],"tools":null}"#,
        ] {
            assert!(apply_reasoning_none(body).is_none(), "无 tools 不应注入: {}", body);
        }
    }

    #[test]
    fn test_reasoning_effort_absent_in_body_without_tools_is_untouched() {
        // 无 tools 时上游不校验；凭空注入非标准字段反而会被严格端点拒绝，
        // 因此契约是「无 tools → 最终 body 不含 reasoning_effort」。
        let body = r#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#;
        assert!(apply_reasoning_none(body).is_none());
    }

    #[test]
    fn test_upstream_rejects_missing_reasoning_effort_only_matches_400_bodies() {
        // 触发重试的判定必须足够窄：只有"提到 reasoning_effort"才重试，
        // 否则任何 400 都会被重试一次（放大上游压力、掩盖真实错误）。
        assert!(upstream_rejects_missing_reasoning_effort(
            r#"{"error":{"message":"... reasoning_effort ... set reasoning_effort to 'none'."}}"#
        ));
        assert!(!upstream_rejects_missing_reasoning_effort(r#"{"error":{"message":"InvalidParameter: temperature"}}"#));
        assert!(!upstream_rejects_missing_reasoning_effort(""));
    }
}
