use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::rjlog;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub alias: String,
    pub provider: String,
    pub provider_name: String,
    pub provider_icon: String,
    pub api_base: String,
    pub api_key: String,
    pub protocol: String,
    pub context_window: u64,
    pub support_reasoning: bool,
    #[serde(default = "default_support_tools")]
    pub support_tools: bool,
    pub tags: Vec<String>,
    #[serde(default)]
    pub use_proxy: bool,
    /// 企业网关兼容开关：该模型的 `/v1/chat/completions` 端点要求带 tools 时
    /// 必须显式传 `reasoning_effort: "none"`（否则 400）。开启后代理在发送前即
    /// 注入该字段；未开启时代理仍会在上游 400 提及 reasoning_effort 时自动重试
    /// 一次（见 proxy/common.rs send_chat_completions）。
    #[serde(default)]
    pub force_reasoning_none: bool,
}

fn default_support_tools() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAlias {
    pub alias: String,
    pub model_id: String,
    pub description: String,
}

fn get_home_dir() -> PathBuf {
    // 允许用 RUNJAM_HOME 覆盖 home 目录：
    // - 测试可完全隔离，绝不触碰真实 ~/.claude 等文件
    // - 生产上给用户一个逃生舱（把 runjam 指向另一套 CLI 配置）
    if let Ok(dir) = std::env::var("RUNJAM_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn read_models_from_agent_config(agent_id: &str) -> Vec<ModelEntry> {
    let home = get_home_dir();
    let mut models = Vec::new();

    match agent_id {
        "claude-code" => {
            let path = home.join(".claude").join("settings.json");
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(runjam_models) = settings.get("runjam_models").and_then(|v| v.as_array()) {
                            for model_val in runjam_models {
                                if let Ok(model) = serde_json::from_value::<ModelEntry>(model_val.clone()) {
                                    models.push(model);
                                }
                            }
                        } else {
                            models.extend(parse_claude_native_config(&settings));
                        }
                    }
                }
            }
        }
        "codex-cli" => {
            let path = home.join(".codex").join("config.toml");
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(doc) = toml::from_str::<toml::Value>(&content) {
                        if let Some(runjam) = doc.get("runjam") {
                            if let Some(models_str) = runjam.get("models").and_then(|v| v.as_str()) {
                                if let Ok(rj_models) = serde_json::from_str::<Vec<ModelEntry>>(models_str) {
                                    models.extend(rj_models);
                                }
                            }
                        } else {
                            models.extend(parse_codex_native_config(&doc));
                        }
                    }
                }
            }
        }
        "gemini-cli" => {
            let path = home.join(".gemini").join("settings.json");
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(runjam_models) = settings.get("runjam_models").and_then(|v| v.as_array()) {
                            for model_val in runjam_models {
                                if let Ok(model) = serde_json::from_value::<ModelEntry>(model_val.clone()) {
                                    models.push(model);
                                }
                            }
                        } else {
                            models.extend(parse_gemini_native_config(&settings));
                        }
                    }
                }
            }
        }
        _ => {}
    }

    models
}

fn parse_claude_native_config(settings: &serde_json::Value) -> Vec<ModelEntry> {
    let mut models = Vec::new();
    
    if let Some(models_array) = settings.get("models").and_then(|v| v.as_array()) {
        for model_val in models_array {
            let id = model_val.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = model_val.get("name").and_then(|v| v.as_str()).unwrap_or(id);
            let alias = model_val.get("alias").and_then(|v| v.as_str()).unwrap_or(name);
            let api_base = model_val.get("apiBase").or_else(|| model_val.get("api_base")).and_then(|v| v.as_str()).unwrap_or("");
            let api_key = model_val.get("apiKey").or_else(|| model_val.get("api_key")).and_then(|v| v.as_str()).unwrap_or("");
            let context_window = model_val.get("contextWindow").or_else(|| model_val.get("context_window")).and_then(|v| v.as_u64()).unwrap_or(0);
            let support_reasoning = model_val.get("supportReasoning").or_else(|| model_val.get("support_reasoning")).and_then(|v| v.as_bool()).unwrap_or(false);
            let support_tools = model_val.get("supportTools").or_else(|| model_val.get("support_tools")).and_then(|v| v.as_bool()).unwrap_or(true);
            let force_reasoning_none = model_val.get("forceReasoningNone").or_else(|| model_val.get("force_reasoning_none")).and_then(|v| v.as_bool()).unwrap_or(false);
            let tags: Vec<String> = model_val.get("tags").and_then(|v| v.as_array()).map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
            
            if !id.is_empty() {
                models.push(ModelEntry {
                    id: id.to_string(),
                    name: name.to_string(),
                    alias: alias.to_string(),
                    provider: "custom".to_string(),
                    provider_name: "Custom".to_string(),
                    provider_icon: "".to_string(),
                    api_base: api_base.to_string(),
                    api_key: api_key.to_string(),
                    protocol: detect_model_protocol(&name, Some(&api_base)).as_str().to_string(),
                    context_window,
                    support_reasoning,
                    support_tools,
                    tags,
                    use_proxy: false,
                    force_reasoning_none,
                });
            }
        }
    }
    
    models
}

fn parse_codex_native_config(doc: &toml::Value) -> Vec<ModelEntry> {
    let mut models = Vec::new();
    
    let model = doc.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let base_url = doc.get("base_url").and_then(|v| v.as_str()).unwrap_or("");
    
    if !model.is_empty() {
        models.push(ModelEntry {
            id: format!("codex-{}", model),
            name: model.to_string(),
            alias: model.to_string(),
            provider: "custom".to_string(),
            provider_name: "Custom".to_string(),
            provider_icon: "".to_string(),
            api_base: base_url.to_string(),
            api_key: "".to_string(),
            protocol: detect_model_protocol(model, Some(base_url)).as_str().to_string(),
            context_window: 0,
            support_reasoning: false,
            support_tools: true,
            tags: vec![],
            use_proxy: false, force_reasoning_none: false,
        });
    }
    
    if let Some(model_providers) = doc.get("model_providers").and_then(|v| v.as_table()) {
        for (provider_id, provider_config) in model_providers {
            if let Some(model) = provider_config.get("model").and_then(|v| v.as_str()) {
                let base_url = provider_config.get("base_url").and_then(|v| v.as_str()).unwrap_or("");
                models.push(ModelEntry {
                    id: format!("codex-{}-{}", provider_id, model),
                    name: model.to_string(),
                    alias: format!("{} - {}", provider_id, model),
                    provider: "custom".to_string(),
                    provider_name: provider_id.to_string(),
                    provider_icon: "".to_string(),
                    api_base: base_url.to_string(),
                    api_key: "".to_string(),
                    protocol: detect_model_protocol(model, Some(base_url)).as_str().to_string(),
                    context_window: 0,
                    support_reasoning: false,
                    support_tools: true,
                    tags: vec![],
                    use_proxy: false, force_reasoning_none: false,
                });
            }
        }
    }
    
    models
}

fn parse_gemini_native_config(settings: &serde_json::Value) -> Vec<ModelEntry> {
    let mut models = Vec::new();
    
    if let Some(models_array) = settings.get("models").and_then(|v| v.as_array()) {
        for model_val in models_array {
            let id = model_val.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = model_val.get("name").and_then(|v| v.as_str()).unwrap_or(id);
            let alias = model_val.get("alias").and_then(|v| v.as_str()).unwrap_or(name);
            let api_base = model_val.get("apiBase").or_else(|| model_val.get("api_base")).and_then(|v| v.as_str()).unwrap_or("");
            let api_key = model_val.get("apiKey").or_else(|| model_val.get("api_key")).and_then(|v| v.as_str()).unwrap_or("");
            let context_window = model_val.get("contextWindow").or_else(|| model_val.get("context_window")).and_then(|v| v.as_u64()).unwrap_or(0);
            let support_reasoning = model_val.get("supportReasoning").or_else(|| model_val.get("support_reasoning")).and_then(|v| v.as_bool()).unwrap_or(false);
            let support_tools = model_val.get("supportTools").or_else(|| model_val.get("support_tools")).and_then(|v| v.as_bool()).unwrap_or(true);
            let force_reasoning_none = model_val.get("forceReasoningNone").or_else(|| model_val.get("force_reasoning_none")).and_then(|v| v.as_bool()).unwrap_or(false);
            let tags: Vec<String> = model_val.get("tags").and_then(|v| v.as_array()).map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
            
            if !id.is_empty() {
                models.push(ModelEntry {
                    id: id.to_string(),
                    name: name.to_string(),
                    alias: alias.to_string(),
                    provider: "custom".to_string(),
                    provider_name: "Custom".to_string(),
                    provider_icon: "".to_string(),
                    api_base: api_base.to_string(),
                    api_key: api_key.to_string(),
                    protocol: detect_model_protocol(&name, Some(&api_base)).as_str().to_string(),
                    context_window,
                    support_reasoning,
                    support_tools,
                    tags,
                    use_proxy: false,
                    force_reasoning_none,
                });
            }
        }
    }
    
    models
}

/// 一个 CLI 的原生配置里描述出来的模型。
///
/// 只读展示用：runjam 覆写 `~/.claude/settings.json` 等文件后，原生模型信息
/// 只能从快照回读，因此这里把「值」和「来源」一起带给前端。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeModel {
    /// 归属的 agent id（claude-code / codex-cli / gemini-cli）
    pub agent_id: String,
    pub agent_label: String,
    pub name: String,
    pub alias: String,
    pub api_base: String,
    pub protocol: String,
    /// 该条目是否为该 CLI 当前选用的模型
    pub is_current: bool,
    /// runjam 是否已覆写过该 CLI 的配置文件
    pub is_overridden: bool,
    /// 是否存在 runjam 保存的原生快照
    pub has_snapshot: bool,
    pub source_path: String,
}

/// 原生配置文件路径 —— runjam 会覆写的那个文件。
/// 判断某个 base_url 是否指向 runjam 自己的本地代理。
/// 这类地址是 runjam 改写的结果，不属于用户的原生配置。
///
/// 判据必须精确到 runjam 的固定代理端口：用户的原生配置本身就常用本地推理
/// 服务（例如 llama-server 跑在 `http://localhost:19090/v1`），那属于真正的
/// 原生端点，绝不能被当成 runjam 的改写结果清掉。
fn points_at_runjam_proxy(s: &str) -> bool {
    let port = crate::proxy::PROXY_PORT;
    s.contains(&format!("127.0.0.1:{}", port)) || s.contains(&format!("localhost:{}", port))
}

/// 手动留存原生快照（供命令层调用）。
///
/// 快照的含义是「原生基线」，两个前提都必须满足，否则会存下一份假的原生
/// 状态，让「切回原生」恢复出错误结果：
///  1) 不覆盖已有快照 —— 否则会丢掉最早、最可信的那一份
///  2) 不从已被 runjam 改写的配置取快照 —— 那是改写结果，不是原生
pub fn try_native_snapshot(agent_id: &str) -> Result<(), String> {
    let cfg = native_config_path(agent_id).ok_or_else(|| format!("Unknown agent: {}", agent_id))?;
    if !cfg.exists() {
        return Err("config_not_found".to_string());
    }
    let snap = native_snapshot_path(agent_id).ok_or_else(|| format!("Unknown agent: {}", agent_id))?;

    if snap.exists() {
        return Err("snapshot_exists".to_string());
    }
    let raw = std::fs::read_to_string(&cfg).unwrap_or_default();
    if config_is_overridden(agent_id, &raw) {
        return Err("config_already_overridden".to_string());
    }

    std::fs::copy(&cfg, &snap).map_err(|e| format!("Failed to snapshot {}: {}", cfg.display(), e))?;
    rjlog!("[SNAPSHOT] manual snapshot {} -> {}", cfg.display(), snap.display());
    Ok(())
}

pub fn native_config_path(agent_id: &str) -> Option<PathBuf> {
    let home = get_home_dir();
    match agent_id {
        "claude-code" => Some(home.join(".claude").join("settings.json")),
        "codex-cli" => Some(home.join(".codex").join("config.toml")),
        "gemini-cli" => Some(home.join(".gemini").join("settings.json")),
        _ => None,
    }
}

fn with_suffix(path: &std::path::Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// 原生配置快照路径：`<config>.runjam-native`。
///
/// 与 `backup_agent_config` 的 `.backup-<date>` 不同，快照**只创建一次且永不
/// 覆盖**，因此它始终是「runjam 第一次介入之前」的可信原生状态 —— 而日期备份
/// 有可能是在 runjam 已经写过之后才产生的（那样备份里也是被改写过的值）。
pub fn native_snapshot_path(agent_id: &str) -> Option<PathBuf> {
    native_config_path(agent_id).map(|p| with_suffix(&p, ".runjam-native"))
}

/// 在覆写某 CLI 配置**之前**保存原生快照。幂等：已存在则不覆盖。
pub fn ensure_native_snapshot(agent_id: &str) {
    let cfg = match native_config_path(agent_id) {
        Some(p) => p,
        None => return,
    };
    let snap = match native_snapshot_path(agent_id) {
        Some(p) => p,
        None => return,
    };
    if snap.exists() {
        return; // 已经 capture 过首次介入前的状态，绝不覆盖
    }
    if !cfg.exists() {
        return; // 还没有原生配置可存
    }
    match std::fs::copy(&cfg, &snap) {
        Ok(_) => rjlog!("[SNAPSHOT] saved native {} -> {}", cfg.display(), snap.display()),
        Err(e) => rjlog!("[SNAPSHOT] failed {} -> {}: {}", cfg.display(), snap.display(), e),
    }
}

/// runjam 是否已经改写过该 CLI 的配置（用它写入的特征键判断）。
pub fn config_is_overridden(agent_id: &str, raw: &str) -> bool {
    match agent_id {
        "claude-code" => {
            let v: serde_json::Value = match serde_json::from_str(raw) {
                Ok(v) => v,
                Err(_) => return false,
            };
            if v.get("models").is_some() || v.get("runjam_models").is_some() {
                return true;
            }
            v.get("env")
                .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
                .and_then(|u| u.as_str())
                .map(points_at_runjam_proxy)
                .unwrap_or(false)
        }
        "codex-cli" => {
            let doc: toml::Value = match toml::from_str(raw) {
                Ok(v) => v,
                Err(_) => return false,
            };
            if doc.get("disable_response_storage").is_some() || doc.get("runjam").is_some() {
                return true;
            }
            doc.get("model_providers")
                .and_then(|p| p.get("custom"))
                .and_then(|c| c.get("base_url"))
                .and_then(|u| u.as_str())
                .map(points_at_runjam_proxy)
                .unwrap_or(false)
        }
        "gemini-cli" => {
            let v: serde_json::Value = match serde_json::from_str(raw) {
                Ok(v) => v,
                Err(_) => return false,
            };
            if v.get("runjam_models").is_some() {
                return true;
            }
            v.get("env")
                .and_then(|e| e.get("GOOGLE_GEMINI_BASE_URL"))
                .and_then(|u| u.as_str())
                .map(points_at_runjam_proxy)
                .unwrap_or(false)
        }
        _ => false,
    }
}

fn native_label(agent_id: &str) -> &'static str {
    match agent_id {
        "claude-code" => "Claude Code",
        "codex-cli" => "Codex CLI",
        "gemini-cli" => "Gemini CLI",
        _ => "Agent",
    }
}

const NATIVE_AGENTS: [&str; 3] = ["claude-code", "codex-cli", "gemini-cli"];

/// 列出某个配置文件的日期备份，新→旧。
fn list_date_backups(config_path: &std::path::Path) -> Vec<PathBuf> {
    let dir = config_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let file_stem = config_path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = config_path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let prefix = format!("{}.{}", file_stem, ext);
    let mut backups: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&prefix) && name_str.contains(".backup-") {
                backups.push(entry.path());
            }
        }
    }
    backups.sort_by(|a, b| {
        let ta = std::fs::metadata(a).and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let tb = std::fs::metadata(b).and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        tb.cmp(&ta)
    });
    backups
}

/// 原生来源候选（可信度降序）：
/// 1. `.runjam-native` 快照 —— 保证 capture 在 runjam 首次覆写之前
/// 2. 日期备份 `.backup-<date>` —— 新→旧（存量用户往往只有它）
/// 3. 当前配置文件 —— 兜底，可能已被覆写
fn native_source_candidates(agent_id: &str, cfg: &std::path::Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(snap) = native_snapshot_path(agent_id) {
        if snap.exists() {
            out.push(snap);
        }
    }
    out.extend(list_date_backups(cfg));
    if cfg.exists() {
        out.push(cfg.to_path_buf());
    }
    out
}

/// 读取某个 CLI 的原生模型（只读展示用）。
///
/// 按 `native_source_candidates` 的顺序取**第一个能解析出模型**的来源。快照就绪
/// 后它必然排在首位，行为确定；存量用户没有快照时自动退回日期备份。
pub fn read_native_models(agent_id: &str) -> Vec<NativeModel> {
    let cfg = match native_config_path(agent_id) {
        Some(p) => p,
        None => return vec![],
    };
    let has_snapshot = native_snapshot_path(agent_id).map(|p| p.exists()).unwrap_or(false);
    // 「是否已被 runjam 覆写」看当前文件，而不是我们读的那个源
    let is_overridden = std::fs::read_to_string(&cfg)
        .map(|cur| config_is_overridden(agent_id, &cur))
        .unwrap_or(false);

    let label = native_label(agent_id).to_string();
    for src in native_source_candidates(agent_id, &cfg) {
        let raw = match std::fs::read_to_string(&src) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut models = match agent_id {
            "claude-code" => parse_claude_native_models(&raw),
            "codex-cli" => parse_codex_native_models(&raw),
            "gemini-cli" => parse_gemini_native_models(&raw),
            _ => vec![],
        };
        if models.is_empty() {
            continue;
        }
        let src_str = src.display().to_string();
        for m in &mut models {
            m.agent_id = agent_id.to_string();
            m.agent_label = label.clone();
            m.is_overridden = is_overridden;
            m.has_snapshot = has_snapshot;
            m.source_path = src_str.clone();
        }
        return models;
    }
    vec![]
}

/// 解析 Claude Code 原生配置里的模型。
///
/// 原生形态是顶层 `model` + `env.ANTHROPIC_*MODEL*`。注意：`models` 数组是
/// **runjam 自己写入的**，不属于原生信息，故此处忽略它。
fn parse_claude_native_models(raw: &str) -> Vec<NativeModel> {
    let v: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let env = v.get("env").and_then(|e| e.as_object());
    let base = env
        .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
        .and_then(|u| u.as_str())
        .unwrap_or("");
    let current = v.get("model").and_then(|m| m.as_str()).unwrap_or("");
    // 若 base_url 指向 runjam 自己的 proxy（loopback），那不是用户的原生端点，
    // 而是被 runjam 改写的结果。此时不要把它当作"原生端点"展示/还原，否则
    // 用户切回原生后会指向一个已经关闭的代理端口。
    let base = if points_at_runjam_proxy(base) { "" } else { base };

    let mut names: Vec<String> = Vec::new();
    let mut push = |n: &str| {
        let n = n.trim();
        if !n.is_empty() && !names.iter().any(|x| x == n) {
            names.push(n.to_string());
        }
    };
    push(current);
    if let Some(e) = env {
        for key in [
            "ANTHROPIC_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
        ] {
            if let Some(n) = e.get(key).and_then(|x| x.as_str()) {
                push(n);
            }
        }
    }

    names
        .into_iter()
        .map(|name| NativeModel {
            agent_id: String::new(),
            agent_label: String::new(),
            protocol: detect_model_protocol(&name, Some(base)).as_str().to_string(),
            is_current: name == current,
            alias: name.clone(),
            name,
            api_base: base.to_string(),
            is_overridden: false,
            has_snapshot: false,
            source_path: String::new(),
        })
        .collect()
}

/// 解析 Codex CLI 原生配置里的模型。
fn parse_codex_native_models(raw: &str) -> Vec<NativeModel> {
    let doc: toml::Value = match toml::from_str(raw) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let current = doc.get("model").and_then(|m| m.as_str()).unwrap_or("");
    if current.is_empty() {
        return vec![];
    }
    // 当前 model_provider 对应的 provider 表（原生 codex 常不写 model_provider，
    // 此时为空 —— 表示走官方端点）
    let provider = doc
        .get("model_provider")
        .and_then(|p| p.as_str())
        .and_then(|pid| doc.get("model_providers").and_then(|ps| ps.get(pid)));
    let base = provider
        .and_then(|c| c.get("base_url"))
        .and_then(|u| u.as_str())
        .unwrap_or("");
    let base = if points_at_runjam_proxy(base) { "" } else { base };
    // codex 的协议由 provider 的 wire_api 决定，不能只看 URL/模型名：
    // 例如企业网关 base_url 是 ".../v1"（不含 /responses）却走 responses 协议。
    let protocol = match provider.and_then(|c| c.get("wire_api")).and_then(|w| w.as_str()) {
        Some("responses") => LlmProtocol::OpenAiResponses,
        Some("chat") => LlmProtocol::OpenAiChat,
        _ => detect_model_protocol(current, Some(base)),
    };
    vec![NativeModel {
        agent_id: String::new(),
        agent_label: String::new(),
        protocol: protocol.as_str().to_string(),
        is_current: true,
        alias: current.to_string(),
        name: current.to_string(),
        api_base: base.to_string(),
        is_overridden: false,
        has_snapshot: false,
        source_path: String::new(),
    }]
}

/// 解析 Gemini CLI 原生配置里的模型。
fn parse_gemini_native_models(raw: &str) -> Vec<NativeModel> {
    let v: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let env = v.get("env").and_then(|e| e.as_object());
    let name = env
        .and_then(|e| e.get("GEMINI_MODEL"))
        .and_then(|m| m.as_str())
        .unwrap_or("");
    if name.is_empty() {
        return vec![];
    }
    let base = env
        .and_then(|e| e.get("GOOGLE_GEMINI_BASE_URL"))
        .and_then(|u| u.as_str())
        .unwrap_or("");
    // 同 claude：runjam 的 loopback 端点不是原生端点
    let base = if points_at_runjam_proxy(base) { "" } else { base };
    vec![NativeModel {
        agent_id: String::new(),
        agent_label: String::new(),
        protocol: detect_model_protocol(name, Some(base)).as_str().to_string(),
        is_current: true,
        alias: name.to_string(),
        name: name.to_string(),
        api_base: base.to_string(),
        is_overridden: false,
        has_snapshot: false,
        source_path: String::new(),
    }]
}

/// 汇总三个 CLI 的原生模型。
pub fn read_all_native_models() -> Vec<NativeModel> {
    let mut out = Vec::new();
    for agent in NATIVE_AGENTS {
        out.extend(read_native_models(agent));
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelConfig {
    pub models: Vec<ModelEntry>,
}

impl ModelConfig {
    fn path() -> PathBuf {
        let base = directories::ProjectDirs::from("com", "runjam", "RunJam")
            .map(|d| d.data_local_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        std::fs::create_dir_all(&base).ok();
        base.join("models.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(config) = serde_json::from_str(&content) {
                    return config;
                }
            }
        }
        let mut models = Vec::new();
        models.extend(read_models_from_agent_config("claude-code"));
        models.extend(read_models_from_agent_config("codex-cli"));
        models.extend(read_models_from_agent_config("gemini-cli"));
        ModelConfig { models }
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            std::fs::write(&path, json).ok();
        }
    }
}

/// Write model config to an agent's config file.
pub fn sync_to_agent(agent_id: &str, models: &[ModelEntry]) -> Result<(), String> {
    // 覆写前先留存原生快照（幂等，仅首次真正落盘）
    ensure_native_snapshot(agent_id);

    let home = get_home_dir();

    match agent_id {
        "claude-code" => {
            let dir = home.join(".claude");
            std::fs::create_dir_all(&dir).ok();
            let path = dir.join("settings.json");
            let mut settings: serde_json::Value = if path.exists() {
                let s = std::fs::read_to_string(&path).unwrap_or_default();
                serde_json::from_str(&s).unwrap_or(serde_json::json!({}))
            } else {
                serde_json::json!({})
            };

            // Remove legacy runjam_models key
            if let Some(obj) = settings.as_object_mut() {
                obj.remove("runjam_models");
            }

            // Write env vars from the first model
            if let Some(first) = models.first() {
                let existing_env = settings.get("env")
                    .and_then(|v| v.as_object())
                    .map(|o| o.clone())
                    .unwrap_or_default();
                let mut env_map = serde_json::Map::new();
                for (k, v) in existing_env {
                    env_map.insert(k, v);
                }
                if !first.api_key.is_empty() {
                    env_map.insert("ANTHROPIC_AUTH_TOKEN".into(), serde_json::Value::String(first.api_key.clone()));
                }
                if !first.api_base.is_empty() {
                    env_map.insert("ANTHROPIC_BASE_URL".into(), serde_json::Value::String(first.api_base.clone()));
                }
                // Override Claude's tier-specific model defaults so it sends our
                // model name instead of "claude-sonnet-4-6" etc.
                let model_name = first.name.clone();
                env_map.insert("ANTHROPIC_MODEL".into(), serde_json::Value::String(model_name.clone()));
                env_map.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), serde_json::Value::String(model_name.clone()));
                env_map.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME".into(), serde_json::Value::String(model_name.clone()));
                env_map.insert("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), serde_json::Value::String(model_name.clone()));
                env_map.insert("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME".into(), serde_json::Value::String(model_name.clone()));
                env_map.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), serde_json::Value::String(model_name.clone()));
                env_map.insert("ANTHROPIC_DEFAULT_OPUS_MODEL_NAME".into(), serde_json::Value::String(model_name.clone()));
                settings["env"] = serde_json::Value::Object(env_map);

                // Set default model name (use real name, alias is UI-only in SQLite)
                settings["model"] = serde_json::Value::String(first.name.clone());
            }

            // Write models in native Claude Code format (camelCase keys)
            let native_models: Vec<serde_json::Value> = models.iter().map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "name": m.name,
                    "alias": m.alias,
                    "apiBase": m.api_base,
                    "apiKey": m.api_key,
                    "contextWindow": m.context_window,
                    "supportReasoning": m.support_reasoning,
                    "forceReasoningNone": m.force_reasoning_none,
                    "tags": m.tags,
                })
            }).collect();
            settings["models"] = serde_json::Value::Array(native_models);

            std::fs::write(&path, serde_json::to_string_pretty(&settings).unwrap_or_default())
                .map_err(|e| format!("Failed to write claude config: {}", e))?;
        }
        "codex-cli" => {
            let dir = home.join(".codex");
            std::fs::create_dir_all(&dir).ok();
            let path = dir.join("config.toml");
            
            // Parse existing TOML
            let content = if path.exists() {
                std::fs::read_to_string(&path).unwrap_or_default()
            } else {
                String::new()
            };
            let mut doc: toml::Value = toml::from_str(&content).unwrap_or(toml::Value::Table(toml::value::Table::new()));
            
            if let toml::Value::Table(ref mut table) = doc {
                // Remove legacy [runjam] section
                table.remove("runjam");
                
            if let Some(first) = models.first() {
                    // Detect protocol for this model
                    let protocol = detect_model_protocol(&first.name, Some(&first.api_base));
                    let is_openai = matches!(protocol, LlmProtocol::OpenAiChat | LlmProtocol::OpenAiResponses);
                    rjlog!("[CODEX SYNC] Model '{}' detected protocol: {} (openai_compat={})", first.name, protocol.as_str(), is_openai);
                    
                    // Set top-level fields (use real name, alias is UI-only in SQLite)
                    table.insert("model_provider".to_string(), toml::Value::String("custom".to_string()));
                    table.insert("model".to_string(), toml::Value::String(first.name.clone()));
                    table.insert("disable_response_storage".to_string(), toml::Value::Boolean(true));
                    
                    // Build [model_providers.custom] with api_key
                    let mut custom = table.get("model_providers")
                        .and_then(|v| v.get("custom"))
                        .and_then(|v| v.as_table())
                        .cloned()
                        .unwrap_or_default();
                    custom.insert("name".to_string(), toml::Value::String("custom".to_string()));
                    custom.insert("wire_api".to_string(), toml::Value::String("responses".to_string()));
                    custom.insert("requires_openai_auth".to_string(), toml::Value::Boolean(is_openai));
                    if !first.api_base.is_empty() {
                        custom.insert("base_url".to_string(), toml::Value::String(first.api_base.clone()));
                        rjlog!("[CODEX SYNC] base_url = {}", first.api_base);
                    }
                    if !first.api_key.is_empty() {
                        let masked = if first.api_key.len() > 8 {
                            format!("{}...{}", &first.api_key[..4], &first.api_key[first.api_key.len()-4..])
                        } else { "***".to_string() };
                        rjlog!("[CODEX SYNC] api_key = {}", masked);
                        custom.insert("api_key".to_string(), toml::Value::String(first.api_key.clone()));
                    } else {
                        rjlog!("[CODEX SYNC] WARNING: api_key is empty!");
                    }
                    let mut providers = toml::value::Table::new();
                    providers.insert("custom".to_string(), toml::Value::Table(custom));
                    table.insert("model_providers".to_string(), toml::Value::Table(providers));
                } else {
                    rjlog!("[CODEX SYNC] WARNING: no models for codex, config will have no model info");
                }
            }

            let output = toml::to_string_pretty(&doc).unwrap_or_default();
            rjlog!("[CODEX SYNC] Writing config to {}:\n{}", path.display(), output);
            std::fs::write(&path, output)
                .map_err(|e| format!("Failed to write codex config: {}", e))?;

            // Also write .env file — codex ACP bridge may not inherit process env vars,
            // but codex CLI reads OPENAI_API_KEY from ~/.codex/.env
            if let Some(first) = models.first() {
                if !first.api_key.is_empty() {
                    let env_path = dir.join(".env");
                    let env_content = format!("OPENAI_API_KEY={}\n", first.api_key);
                    rjlog!("[CODEX SYNC] Writing .env file to {}: OPENAI_API_KEY=***", env_path.display());
                    std::fs::write(&env_path, env_content).ok();

                    // Also write auth.json — codex reads API key from here
                    let auth_path = dir.join("auth.json");
                    let auth_content = serde_json::json!({
                        "api_key": &first.api_key,
                        "OPENAI_API_KEY": &first.api_key,
                    });
                    rjlog!("[CODEX SYNC] Writing auth.json to {}: api_key=***", auth_path.display());
                    std::fs::write(&auth_path, serde_json::to_string_pretty(&auth_content).unwrap_or_default()).ok();
                }
            }
        }
        "gemini-cli" => {
            let dir = home.join(".gemini");
            std::fs::create_dir_all(&dir).ok();
            let path = dir.join("settings.json");
            let mut settings: serde_json::Value = if path.exists() {
                let s = std::fs::read_to_string(&path).unwrap_or_default();
                serde_json::from_str(&s).unwrap_or(serde_json::json!({}))
            } else {
                serde_json::json!({})
            };

            // Remove legacy runjam_models key
            if let Some(obj) = settings.as_object_mut() {
                obj.remove("runjam_models");
            }

            // Write env vars from the first model
            if let Some(first) = models.first() {
                let existing_env = settings.get("env")
                    .and_then(|v| v.as_object())
                    .map(|o| o.clone())
                    .unwrap_or_default();
                let mut env_map = serde_json::Map::new();
                for (k, v) in existing_env {
                    env_map.insert(k, v);
                }
                if !first.api_base.is_empty() {
                    env_map.insert("GOOGLE_GEMINI_BASE_URL".into(), serde_json::Value::String(first.api_base.clone()));
                }
                if !first.api_key.is_empty() {
                    env_map.insert("GEMINI_API_KEY".into(), serde_json::Value::String(first.api_key.clone()));
                }
                env_map.insert("GEMINI_MODEL".into(), serde_json::Value::String(first.name.clone()));
                settings["env"] = serde_json::Value::Object(env_map);
            }

            std::fs::write(&path, serde_json::to_string_pretty(&settings).unwrap_or_default())
                .map_err(|e| format!("Failed to write gemini config: {}", e))?;
        }
        _ => {}
    }

    Ok(())
}

/// Configure an agent to use the local proxy server.
pub fn configure_agent_proxy(agent_id: &str, proxy_url: &str) -> Result<(), String> {
    // 覆写前先留存原生快照（与 sync_to_agent 共用同一份，幂等）
    ensure_native_snapshot(agent_id);

    let home = get_home_dir();

    match agent_id {
        "claude-code" => {
            let dir = home.join(".claude");
            std::fs::create_dir_all(&dir).ok();
            let path = dir.join("settings.json");
            let mut settings: serde_json::Value = if path.exists() {
                let s = std::fs::read_to_string(&path).unwrap_or_default();
                serde_json::from_str(&s).unwrap_or(serde_json::json!({}))
            } else {
                serde_json::json!({})
            };
            
            let env = settings.get("env")
                .and_then(|v| v.as_object())
                .map(|o| o.clone())
                .unwrap_or_default();
            
            let mut env_map = serde_json::Map::new();
            for (k, v) in env {
                env_map.insert(k, v);
            }
            env_map.insert("ANTHROPIC_BASE_URL".to_string(), serde_json::Value::String(format!("{}/anthropic", proxy_url.trim_end_matches('/'))));
            settings["env"] = serde_json::Value::Object(env_map);
            
            std::fs::write(&path, serde_json::to_string_pretty(&settings).unwrap_or_default())
                .map_err(|e| format!("Failed to configure claude proxy: {}", e))?;
        }
        "codex-cli" => {
            let dir = home.join(".codex");
            std::fs::create_dir_all(&dir).ok();
            let path = dir.join("config.toml");
            let content = if path.exists() {
                std::fs::read_to_string(&path).unwrap_or_default()
            } else {
                String::new()
            };
            let mut doc: toml::Value = toml::from_str(&content).unwrap_or(toml::Value::Table(toml::value::Table::new()));
            
            if let toml::Value::Table(ref mut table) = doc {
                let mut custom = table.get("model_providers")
                    .and_then(|v| v.get("custom"))
                    .and_then(|v| v.as_table())
                    .cloned()
                    .unwrap_or_default();
                custom.insert("base_url".to_string(), toml::Value::String(proxy_url.to_string()));
                let mut providers = toml::value::Table::new();
                providers.insert("custom".to_string(), toml::Value::Table(custom));
                table.insert("model_providers".to_string(), toml::Value::Table(providers));
            }
            
            std::fs::write(&path, toml::to_string_pretty(&doc).unwrap_or_default())
                .map_err(|e| format!("Failed to configure codex proxy: {}", e))?;
        }
        "gemini-cli" => {
            let dir = home.join(".gemini");
            std::fs::create_dir_all(&dir).ok();
            let path = dir.join("settings.json");
            let mut settings: serde_json::Value = if path.exists() {
                let s = std::fs::read_to_string(&path).unwrap_or_default();
                serde_json::from_str(&s).unwrap_or(serde_json::json!({}))
            } else {
                serde_json::json!({})
            };
            
            let env = settings.get("env")
                .and_then(|v| v.as_object())
                .map(|o| o.clone())
                .unwrap_or_default();
            
            let mut env_map = serde_json::Map::new();
            for (k, v) in env {
                env_map.insert(k, v);
            }
            env_map.insert("GOOGLE_GEMINI_BASE_URL".to_string(), serde_json::Value::String(proxy_url.to_string()));
            settings["env"] = serde_json::Value::Object(env_map);
            
            std::fs::write(&path, serde_json::to_string_pretty(&settings).unwrap_or_default())
                .map_err(|e| format!("Failed to configure gemini proxy: {}", e))?;
        }
        _ => {}
    }

    Ok(())
}

/// Update model name and API key in the agent config file, preserving
/// proxy URL and other settings. Called when the user selects a model
/// in the chat dialog, so the agent sends the correct model name to the proxy.
pub fn set_agent_model(agent_id: &str, model_name: &str, api_key: &str) -> Result<(), String> {
    // 覆写前先留存原生快照（幂等）
    ensure_native_snapshot(agent_id);

    let home = get_home_dir();

    match agent_id {
        "claude-code" => {
            let path = home.join(".claude").join("settings.json");
            if !path.exists() {
                return Ok(());
            }
            let s = std::fs::read_to_string(&path).unwrap_or_default();
            let mut settings: serde_json::Value = serde_json::from_str(&s).unwrap_or(serde_json::json!({}));

            settings["model"] = serde_json::Value::String(model_name.to_string());

            if let Some(env) = settings.get_mut("env") {
                if let Some(obj) = env.as_object_mut() {
                    // Update API key for proxy (proxy handles auth, but some agents validate locally)
                    if !api_key.is_empty() {
                        obj.insert("ANTHROPIC_AUTH_TOKEN".into(), serde_json::Value::String(api_key.to_string()));
                    }
                    // Model name
                    obj.insert("ANTHROPIC_MODEL".into(), serde_json::Value::String(model_name.to_string()));
                    obj.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), serde_json::Value::String(model_name.to_string()));
                    obj.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME".into(), serde_json::Value::String(model_name.to_string()));
                    obj.insert("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), serde_json::Value::String(model_name.to_string()));
                    obj.insert("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME".into(), serde_json::Value::String(model_name.to_string()));
                    obj.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), serde_json::Value::String(model_name.to_string()));
                    obj.insert("ANTHROPIC_DEFAULT_OPUS_MODEL_NAME".into(), serde_json::Value::String(model_name.to_string()));
                }
            }

            std::fs::write(&path, serde_json::to_string_pretty(&settings).unwrap_or_default())
                .map_err(|e| format!("Failed to write claude config: {}", e))?;
        }
        "codex-cli" => {
            let path = home.join(".codex").join("config.toml");
            if !path.exists() {
                return Ok(());
            }
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let mut doc: toml::Value = toml::from_str(&content).unwrap_or(toml::Value::Table(toml::value::Table::new()));

            if let toml::Value::Table(ref mut table) = doc {
                table.insert("model".to_string(), toml::Value::String(model_name.to_string()));

                // Update API key in model_providers.custom (proxy URL is preserved)
                if !api_key.is_empty() {
                    let mut custom = table.get("model_providers")
                        .and_then(|v| v.get("custom"))
                        .and_then(|v| v.as_table())
                        .cloned()
                        .unwrap_or_default();
                    custom.insert("api_key".to_string(), toml::Value::String(api_key.to_string()));
                    let mut providers = toml::value::Table::new();
                    providers.insert("custom".to_string(), toml::Value::Table(custom));
                    table.insert("model_providers".to_string(), toml::Value::Table(providers));
                }
            }

            std::fs::write(&path, toml::to_string_pretty(&doc).unwrap_or_default())
                .map_err(|e| format!("Failed to write codex config: {}", e))?;
        }
        "gemini-cli" => {
            let path = home.join(".gemini").join("settings.json");
            if !path.exists() {
                return Ok(());
            }
            let s = std::fs::read_to_string(&path).unwrap_or_default();
            let mut settings: serde_json::Value = serde_json::from_str(&s).unwrap_or(serde_json::json!({}));

            let mut env_map = settings.get("env")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            env_map.insert("GEMINI_MODEL".into(), serde_json::Value::String(model_name.to_string()));
            if !api_key.is_empty() {
                env_map.insert("GEMINI_API_KEY".into(), serde_json::Value::String(api_key.to_string()));
            }
            settings["env"] = serde_json::Value::Object(env_map);

            std::fs::write(&path, serde_json::to_string_pretty(&settings).unwrap_or_default())
                .map_err(|e| format!("Failed to write gemini config: {}", e))?;
        }
        _ => {}
    }
    Ok(())
}

pub fn restore_agent_config(agent_id: &str) -> Result<(), String> {
    let home = get_home_dir();

    let config_path = match agent_id {
        "claude-code" => home.join(".claude").join("settings.json"),
        "codex-cli" => home.join(".codex").join("config.toml"),
        "gemini-cli" => home.join(".gemini").join("settings.json"),
        _ => return Ok(()),
    };

    // 优先用 runjam 原生快照：它一定 capture 在首次覆写之前，比日期备份可靠
    if let Some(snap) = native_snapshot_path(agent_id) {
        if snap.exists() {
            rjlog!("[RESTORE] Restoring {} from native snapshot: {}", agent_id, snap.display());
            if std::fs::copy(&snap, &config_path).is_ok() {
                cleanup_ancillary_files(agent_id, &home);
                return Ok(());
            }
            rjlog!("[RESTORE] snapshot restore failed, falling back to backups");
        }
    }

    // Try to restore from the latest backup first.
    let restored = try_restore_from_backup(agent_id, &config_path);

    if restored {
        // Also clean up ancillary files RunJam created.
        cleanup_ancillary_files(agent_id, &home);
        return Ok(());
    }

    // No backup found — fall back to removing RunJam-specific fields in-place.
    //
    // 重要：只清除**确认由 runjam 写入**的字段。用户的原生配置里可能本来就有
    // 这些键（例如 llama-server 的本地端点 http://localhost:19090/v1），
    // 无条件删除会把用户自己的配置一起删掉。
    match agent_id {
        "claude-code" => {
            if config_path.exists() {
                let s = std::fs::read_to_string(&config_path).unwrap_or_default();
                let mut settings: serde_json::Value = serde_json::from_str(&s).unwrap_or(serde_json::json!({}));
                // runjam 是否接管过这个文件（写入了模型列表，或端点指向 runjam 代理）
                let runjam_owned = config_is_overridden(agent_id, &s);
                let base_is_proxy = settings
                    .get("env")
                    .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
                    .and_then(|u| u.as_str())
                    .map(points_at_runjam_proxy)
                    .unwrap_or(false);
                if let Some(obj) = settings.as_object_mut() {
                    obj.remove("runjam_models");
                    obj.remove("models");
                    if runjam_owned {
                        // 只有在确认是 runjam 写的文件里，才认为 model 指向 runjam 模型
                        obj.remove("model");
                    }
                    if let Some(env_obj) = obj.get_mut("env").and_then(|v| v.as_object_mut()) {
                        if base_is_proxy {
                            env_obj.remove("ANTHROPIC_AUTH_TOKEN");
                            env_obj.remove("ANTHROPIC_BASE_URL");
                        }
                        if runjam_owned {
                            for key in [
                                "ANTHROPIC_MODEL",
                                "ANTHROPIC_DEFAULT_HAIKU_MODEL",
                                "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
                                "ANTHROPIC_DEFAULT_SONNET_MODEL",
                                "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
                                "ANTHROPIC_DEFAULT_OPUS_MODEL",
                                "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
                            ] {
                                env_obj.remove(key);
                            }
                        }
                    }
                }
                std::fs::write(&config_path, serde_json::to_string_pretty(&settings).unwrap_or_default())
                    .map_err(|e| format!("Failed to restore claude config: {}", e))?;
            }
        }
        "codex-cli" => {
            if config_path.exists() {
                let content = std::fs::read_to_string(&config_path).unwrap_or_default();
                let runjam_owned = config_is_overridden(agent_id, &content);
                let mut doc: toml::Value = toml::from_str(&content).unwrap_or(toml::Value::Table(toml::value::Table::new()));
                if let toml::Value::Table(ref mut table) = doc {
                    table.remove("runjam");
                    if runjam_owned {
                        table.remove("model_provider");
                        table.remove("model");
                        table.remove("disable_response_storage");
                    }
                    if let Some(providers) = table.get_mut("model_providers").and_then(|v| v.as_table_mut()) {
                        if let Some(custom) = providers.get_mut("custom").and_then(|v| v.as_table_mut()) {
                            let base_is_proxy = custom
                                .get("base_url")
                                .and_then(|v| v.as_str())
                                .map(points_at_runjam_proxy)
                                .unwrap_or(false);
                            if base_is_proxy {
                                custom.remove("api_key");
                                custom.remove("requires_openai_auth");
                                custom.remove("wire_api");
                                custom.remove("base_url");
                            }
                        }
                    }
                }
                let cleaned = toml::to_string_pretty(&doc).unwrap_or_default();
                std::fs::write(&config_path, cleaned.trim_end())
                    .map_err(|e| format!("Failed to restore codex config: {}", e))?;
                cleanup_ancillary_files(agent_id, &home);
            }
        }
        "gemini-cli" => {
            if config_path.exists() {
                let s = std::fs::read_to_string(&config_path).unwrap_or_default();
                let mut settings: serde_json::Value = serde_json::from_str(&s).unwrap_or(serde_json::json!({}));
                let runjam_owned = config_is_overridden(agent_id, &s);
                let base_is_proxy = settings
                    .get("env")
                    .and_then(|e| e.get("GOOGLE_GEMINI_BASE_URL"))
                    .and_then(|u| u.as_str())
                    .map(points_at_runjam_proxy)
                    .unwrap_or(false);
                if let Some(obj) = settings.as_object_mut() {
                    obj.remove("runjam_models");
                    if let Some(env_obj) = obj.get_mut("env").and_then(|v| v.as_object_mut()) {
                        if base_is_proxy {
                            env_obj.remove("GOOGLE_GEMINI_BASE_URL");
                            env_obj.remove("GEMINI_API_KEY");
                        }
                        if runjam_owned {
                            env_obj.remove("GEMINI_MODEL");
                        }
                    }
                }
                std::fs::write(&config_path, serde_json::to_string_pretty(&settings).unwrap_or_default())
                    .map_err(|e| format!("Failed to restore gemini config: {}", e))?;
            }
        }
        _ => {}
    }

    Ok(())
}

/// 清理 runjam 为某个 agent 额外创建的边车文件。
fn cleanup_ancillary_files(agent_id: &str, home: &std::path::Path) {
    if agent_id == "codex-cli" {
        let dir = home.join(".codex");
        for name in [".env", "auth.json"] {
            let p = dir.join(name);
            if p.exists() {
                std::fs::remove_file(&p).ok();
            }
        }
    }
}

/// Try to restore config from the latest backup file.
/// Returns true if a backup was found and restored.
/// 恢复后擦洗：日期备份可能是在 runjam 已经改写之后才生成的（历史上就发生过：
/// 备份里留着 runjam 的 `http://127.0.0.1:<PROXY_PORT>`）。直接恢复会让 CLI
/// 指向一个只在本应用运行时才存在的代理端口，于是切回原生后反而起不来。
/// 这里把确认属于 runjam 的端点字段擦掉，让 CLI 回退到自己的默认端点。
fn scrub_runjam_proxy_endpoint(agent_id: &str, config_path: &std::path::Path) {
    if !config_path.exists() {
        return;
    }
    let raw = std::fs::read_to_string(config_path).unwrap_or_default();

    match agent_id {
        "claude-code" => {
            let mut v: serde_json::Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(_) => return,
            };
            let is_proxy = v
                .get("env")
                .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
                .and_then(|u| u.as_str())
                .map(points_at_runjam_proxy)
                .unwrap_or(false);
            if !is_proxy {
                return;
            }
            if let Some(env) = v.get_mut("env").and_then(|e| e.as_object_mut()) {
                env.remove("ANTHROPIC_BASE_URL");
                env.remove("ANTHROPIC_AUTH_TOKEN");
            }
            rjlog!("[RESTORE] scrubbed runjam proxy endpoint from restored claude config");
            let _ = std::fs::write(config_path, serde_json::to_string_pretty(&v).unwrap_or_default());
        }
        "codex-cli" => {
            let mut doc: toml::Value = match toml::from_str(&raw) {
                Ok(v) => v,
                Err(_) => return,
            };
            // 判断 custom provider 是否就是 runjam 写入的
            let custom_is_proxy = doc
                .get("model_providers")
                .and_then(|v| v.get("custom"))
                .and_then(|c| c.get("base_url"))
                .and_then(|u| u.as_str())
                .map(points_at_runjam_proxy)
                .unwrap_or(false);
            if !custom_is_proxy {
                return;
            }
            if let Some(table) = doc.as_table_mut() {
                // runjam 的三件套一起撤掉，避免留下 model_provider="custom"
                // 却没有任何 provider 定义的半坏配置
                table.remove("model_provider");
                table.remove("disable_response_storage");
                if let Some(providers) = table.get_mut("model_providers").and_then(|v| v.as_table_mut()) {
                    providers.remove("custom");
                    if providers.is_empty() {
                        table.remove("model_providers");
                    }
                }
            }
            rjlog!("[RESTORE] scrubbed runjam proxy provider from restored codex config");
            let cleaned = toml::to_string_pretty(&doc).unwrap_or_default();
            let _ = std::fs::write(config_path, cleaned.trim_end());
        }
        "gemini-cli" => {
            let mut v: serde_json::Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(_) => return,
            };
            let is_proxy = v
                .get("env")
                .and_then(|e| e.get("GOOGLE_GEMINI_BASE_URL"))
                .and_then(|u| u.as_str())
                .map(points_at_runjam_proxy)
                .unwrap_or(false);
            if !is_proxy {
                return;
            }
            if let Some(env) = v.get_mut("env").and_then(|e| e.as_object_mut()) {
                env.remove("GOOGLE_GEMINI_BASE_URL");
                env.remove("GEMINI_API_KEY");
            }
            rjlog!("[RESTORE] scrubbed runjam proxy endpoint from restored gemini config");
            let _ = std::fs::write(config_path, serde_json::to_string_pretty(&v).unwrap_or_default());
        }
        _ => {}
    }
}

fn try_restore_from_backup(agent_id: &str, config_path: &std::path::Path) -> bool {
    let dir = config_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let file_stem = config_path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = config_path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();

    // Find all backup files matching "filename.*.backup-*"
    let mut backups: Vec<std::path::PathBuf> = Vec::new();
    let backup_prefix = format!("{}.{}", file_stem, ext);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&backup_prefix) && name_str.contains(".backup-") {
                backups.push(entry.path());
            }
        }
    }

    if backups.is_empty() {
        return false;
    }

    // Sort by modification time (newest first) and pick the latest
    backups.sort_by(|a, b| {
        let ta = std::fs::metadata(a).and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let tb = std::fs::metadata(b).and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        tb.cmp(&ta)
    });

    let latest = &backups[0];
    rjlog!("[RESTORE] Restoring from backup: {}", latest.display());
    match std::fs::copy(latest, config_path) {
        Ok(_) => {
            rjlog!("[RESTORE] Successfully restored config from backup (backup kept: {})", latest.display());
            // 备份本身可能已含 runjam 代理端点，恢复后擦洗掉
            scrub_runjam_proxy_endpoint(agent_id, config_path);
            true
        }
        Err(e) => {
            rjlog!("[RESTORE] Failed to restore from backup {}: {}", latest.display(), e);
            false
        }
    }
}

/// Backup the current agent config file before syncing.
/// Creates a timestamped backup (e.g. config.toml.backup-2026-08-10).
/// Never overwrites existing backups — each sync gets its own file.
pub fn backup_agent_config(agent_id: &str) -> Result<(), String> {
    let home = get_home_dir();

    let config_path = match agent_id {
        "claude-code" => home.join(".claude").join("settings.json"),
        "codex-cli" => home.join(".codex").join("config.toml"),
        "gemini-cli" => home.join(".gemini").join("settings.json"),
        _ => return Ok(()),
    };

    if config_path.exists() {
        let date_str = chrono::Local::now().format("%Y-%m-%d").to_string();
        let bak_path = format!("{}.backup-{}", config_path.display(), date_str);
        // Don't overwrite — if today's backup already exists, add a suffix
        let mut final_bak = bak_path.clone();
        let mut counter = 1;
        while std::path::Path::new(&final_bak).exists() {
            final_bak = format!("{}.backup-{}-v{}", config_path.display(), date_str, counter);
            counter += 1;
        }
        std::fs::copy(&config_path, &final_bak)
            .map_err(|e| format!("Failed to backup {} config to {}: {}", agent_id, final_bak, e))?;
        rjlog!("[BACKUP] Created backup: {}", final_bak);
    }

    Ok(())
}

/// Protocol types for LLM APIs.
/// `openai_chat` is the industry-standard Chat Completions API (99% of 3rd-party models).
/// `openai_responses` is the newer OpenAI Responses API (used by Codex).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LlmProtocol {
    Anthropic,
    OpenAiChat,
    OpenAiResponses,
    Gemini,
}

impl LlmProtocol {
    pub fn as_str(&self) -> &str {
        match self {
            LlmProtocol::Anthropic => "anthropic",
            LlmProtocol::OpenAiChat => "openai_chat",
            LlmProtocol::OpenAiResponses => "openai_responses",
            LlmProtocol::Gemini => "gemini",
        }
    }
}

/// Auto-detect LLM protocol from model name and base URL.
///
/// Three-level detection (priority high to low):
/// 1. Base URL domain/path matching
/// 2. Model name prefix matching
/// 3. Default: OpenAI Chat (industry standard for 3rd-party models)
pub fn detect_model_protocol(model_name: &str, api_base: Option<&str>) -> LlmProtocol {
    let model_lower = model_name.trim().to_lowercase();

    // Level 1: Base URL domain matching (highest priority)
    if let Some(url) = api_base {
        let url_lower = url.to_lowercase();
        if url_lower.contains("anthropic.com") || url_lower.contains("/anthropic") || url_lower.contains("/claude") {
            return LlmProtocol::Anthropic;
        }
        if url_lower.contains("generativelanguage.googleapis.com") || url_lower.contains("/gemini") {
            return LlmProtocol::Gemini;
        }
        if url_lower.contains("openai.com") || url_lower.contains("azure.com") {
            if url_lower.contains("/v1/responses") || url_lower.contains("/responses") {
                return LlmProtocol::OpenAiResponses;
            }
            return LlmProtocol::OpenAiChat;
        }
    }

    // Level 2: Model name prefix matching
    if model_lower.starts_with("claude-") {
        return LlmProtocol::Anthropic;
    }
    if model_lower.starts_with("gemini-") {
        return LlmProtocol::Gemini;
    }
    if model_lower.starts_with("gpt-") || model_lower.starts_with("o1-") || model_lower.starts_with("o3-") || model_lower.starts_with("text-") {
        return LlmProtocol::OpenAiChat;
    }

    // Level 3: Default — most 3rd-party/open-source models use OpenAI Chat compat
    // (qwen-, glm-, llama-, deepseek-, mistral-, yi-, etc.)
    LlmProtocol::OpenAiChat
}

#[cfg(test)]
mod native_tests {
    use super::*;

    // ── Claude Code ──────────────────────────────────────────────────────
    // 关键点：`models` 数组是 runjam 自己写的，不属于原生信息；原生模型来自
    // 顶层 `model` 与 env.ANTHROPIC_*MODEL*。解析必须忽略前者。
    const CLAUDE_NATIVE: &str = r#"{
        "model": "A/Ornith-1.5-9B-Q4_K_M.gguf",
        "env": {
            "ANTHROPIC_BASE_URL": "http://127.0.0.1:59268/anthropic",
            "ANTHROPIC_MODEL": "A/Ornith-1.5-9B-Q4_K_M.gguf",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL": "A/Ornith-1.5-9B-Q4_K_M.gguf",
            "ANTHROPIC_DEFAULT_SONNET_MODEL": "A/Ornith-1.5-9B-Q4_K_M.gguf",
            "ANTHROPIC_DEFAULT_OPUS_MODEL": "A/Ornith-1.5-9B-Q4_K_M.gguf"
        },
        "models": [{"id": "runjam-x", "name": "gpt-6-luna", "alias": "gpt-6-luna"}]
    }"#;

    #[test]
    fn test_claude_native_parses_from_top_level_model_not_runjam_models_array() {
        let models = parse_claude_native_models(CLAUDE_NATIVE);
        // 去重后只有一个原生模型（四个 env 槽位 + 顶层 model 都是同一个值）
        assert_eq!(models.len(), 1, "同名槽位应去重: {:?}", models.iter().map(|m| &m.name).collect::<Vec<_>>());
        assert_eq!(models[0].name, "A/Ornith-1.5-9B-Q4_K_M.gguf");
        assert!(models[0].is_current);
        // 端点指向 runjam 自己的代理，因此不能冒充成"原生端点"（否则切回后会指向死端口）
        assert_eq!(models[0].api_base, "", "runjam 的 loopback 代理地址不应被当作原生端点");
        // 绝不能把 runjam 写入的 "gpt-6-luna" 当成原生模型
        assert!(
            !models.iter().any(|m| m.name == "gpt-6-luna"),
            "runjam 写入的 models[] 不应被当作原生配置"
        );
    }

    #[test]
    fn test_claude_native_keeps_distinct_tier_models() {
        let raw = r#"{
            "model": "claude-sonnet-4-5",
            "env": {
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "claude-haiku-4-5",
                "ANTHROPIC_BASE_URL": "https://api.anthropic.com"
            }
        }"#;
        let models = parse_claude_native_models(raw);
        let names: Vec<&str> = models.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"claude-sonnet-4-5"));
        assert!(names.contains(&"claude-haiku-4-5"));
        // 当前模型标记只在顶层 model 上
        assert!(models.iter().find(|m| m.name == "claude-sonnet-4-5").unwrap().is_current);
        assert!(!models.iter().find(|m| m.name == "claude-haiku-4-5").unwrap().is_current);
        // 协议探测
        assert!(models.iter().all(|m| m.protocol == "anthropic"));
    }

    #[test]
    fn test_runjam_proxy_endpoint_is_not_reported_as_native_base() {
        // 三种 CLI 被 runjam 覆写后，loopback 端点必须被清空，而模型名保留
        let c = parse_claude_native_models(
            r#"{"model":"m","env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:59268/anthropic"}}"#,
        );
        assert_eq!(c[0].api_base, "");
        assert_eq!(c[0].name, "m");

        let x = parse_codex_native_models(
            "model = \"m\"\nmodel_provider = \"custom\"\n\n[model_providers.custom]\nbase_url = \"http://localhost:59268\"\n",
        );
        assert_eq!(x[0].api_base, "");
        assert_eq!(x[0].name, "m");

        let g = parse_gemini_native_models(
            r#"{"env":{"GEMINI_MODEL":"m","GOOGLE_GEMINI_BASE_URL":"http://127.0.0.1:59268"}}"#,
        );
        assert_eq!(g[0].api_base, "");
        assert_eq!(g[0].name, "m");

        // 真实的外部端点必须原样保留
        let real = parse_claude_native_models(
            r#"{"model":"m","env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"}}"#,
        );
        assert_eq!(real[0].api_base, "https://api.anthropic.com");
    }

    /// 用户的原生配置常常就是本地推理服务（llama-server 等）。它们只是恰好
    /// 是回环地址，但并非 runjam 代理，必须被当作真正的原生端点保留下来。
    #[test]
    fn test_user_own_local_inference_endpoint_is_kept_as_native() {
        let local = parse_claude_native_models(
            r#"{"model":"A/Ornith-1.5-9B-Q4_K_M.gguf","env":{"ANTHROPIC_BASE_URL":"http://localhost:19090/v1"}}"#,
        );
        assert_eq!(
            local[0].api_base, "http://localhost:19090/v1",
            "用户自己的本地推理端点不是 runjam 代理，必须保留"
        );
        assert!(!config_is_overridden(
            "claude-code",
            r#"{"model":"A/x.gguf","env":{"ANTHROPIC_BASE_URL":"http://localhost:19090/v1"}}"#
        ), "本地推理服务不应被判定为已被 runjam 改写");
    }

    #[test]
    fn test_claude_native_empty_and_malformed() {
        assert!(parse_claude_native_models("{}").is_empty());
        assert!(parse_claude_native_models("not json").is_empty());
        assert!(parse_claude_native_models(r#"{"model":"  "}"#).is_empty());
    }

    // ── Codex CLI ────────────────────────────────────────────────────────
    #[test]
    fn test_codex_native_parses_model_and_provider_base_url() {
        let raw = r#"
model = "gpt-6-astra"
model_provider = "custom"

[model_providers.custom]
base_url = "https://llm-api.patsnap.info/v1"
wire_api = "responses"
"#;
        let models = parse_codex_native_models(raw);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "gpt-6-astra");
        assert!(models[0].is_current);
        assert_eq!(models[0].api_base, "https://llm-api.patsnap.info/v1");
        assert_eq!(models[0].protocol, "openai_responses");
    }

    #[test]
    fn test_codex_native_without_model_provider_has_empty_base() {
        // 原生 codex 常不写 model_provider（走官方端点）
        let models = parse_codex_native_models("model = \"gpt-5-codex\"\n");
        assert_eq!(models.len(), 1);
        assert!(models[0].api_base.is_empty());
    }

    #[test]
    fn test_codex_native_no_model_is_empty() {
        assert!(parse_codex_native_models("model_provider = \"x\"\n").is_empty());
        assert!(parse_codex_native_models("### not toml").is_empty());
    }

    // ── Gemini CLI ───────────────────────────────────────────────────────
    #[test]
    fn test_gemini_native_parses_from_env() {
        let raw = r#"{
            "env": {
                "GEMINI_MODEL": "gemini-2.5-pro",
                "GOOGLE_GEMINI_BASE_URL": "https://generativelanguage.googleapis.com"
            }
        }"#;
        let models = parse_gemini_native_models(raw);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "gemini-2.5-pro");
        assert!(models[0].is_current);
        assert_eq!(models[0].protocol, "gemini");
    }

    #[test]
    fn test_gemini_native_empty_without_model() {
        assert!(parse_gemini_native_models(r#"{"env":{}}"#).is_empty());
        assert!(parse_gemini_native_models("{}").is_empty());
    }

    // ── 覆写判定 ─────────────────────────────────────────────────────────
    // 这条判定决定 UI 上是否提示「原生配置已被 runjam 改写」，必须准。
    #[test]
    fn test_config_is_overridden_detects_runjam_fingerprints() {
        assert!(config_is_overridden("claude-code", r#"{"models":[{"id":"x"}]}"#));
        assert!(config_is_overridden(
            "claude-code",
            r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:59268/anthropic"}}"#
        ));
        assert!(config_is_overridden("codex-cli", "disable_response_storage = true\n"));
        assert!(config_is_overridden(
            "codex-cli",
            "[model_providers.custom]\nbase_url = \"http://localhost:59268\"\n"
        ));
        assert!(config_is_overridden("gemini-cli", r#"{"runjam_models":[]}"#));
        assert!(config_is_overridden(
            "gemini-cli",
            r#"{"env":{"GOOGLE_GEMINI_BASE_URL":"http://127.0.0.1:59268"}}"#
        ));
    }

    #[test]
    fn test_config_is_overridden_false_for_pristine_native() {
        // 真正原生的配置不能被误报为已覆写
        assert!(!config_is_overridden(
            "claude-code",
            r#"{"model":"A/x.gguf","env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"}}"#
        ));
        assert!(!config_is_overridden("codex-cli", "model = \"gpt-6-astra\"\n"));
        assert!(!config_is_overridden(
            "codex-cli",
            "[model_providers.custom]\nbase_url = \"https://llm-api.patsnap.info/v1\"\n"
        ));
        assert!(!config_is_overridden(
            "gemini-cli",
            r#"{"env":{"GOOGLE_GEMINI_BASE_URL":"https://generativelanguage.googleapis.com"}}"#
        ));
        // 非法内容不应 panic，只当作未覆写
        assert!(!config_is_overridden("claude-code", "not json"));
        assert!(!config_is_overridden("codex-cli", "### not toml"));
    }

    #[test]
    fn test_native_snapshot_path_is_sibling_with_suffix() {
        let snap = native_snapshot_path("claude-code").expect("claude-code has a config path");
        let name = snap.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(name, "settings.json.runjam-native");
        let cfg = native_config_path("codex-cli").unwrap();
        assert_eq!(
            native_snapshot_path("codex-cli").unwrap(),
            with_suffix(&cfg, ".runjam-native")
        );
        assert!(native_config_path("nope").is_none());
        assert!(native_snapshot_path("nope").is_none());
    }

    #[test]
    fn test_date_backup_pattern_matches_existing_mechanism() {
        // 与 backup_agent_config() 生成的 "<config>.backup-<date>" 保持一致，
        // 且不能把 .runjam-native 快照误当成日期备份。
        let dir = std::env::temp_dir().join(format!("rj_native_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("settings.json");
        std::fs::write(&cfg, "{}").unwrap();
        std::fs::write(dir.join("settings.json.backup-2026-09-28"), "{}").unwrap();
        std::fs::write(dir.join("settings.json.backup-2026-09-27"), "{}").unwrap();
        std::fs::write(dir.join("settings.json.runjam-native"), "{}").unwrap();

        let backups = list_date_backups(&cfg);
        assert_eq!(backups.len(), 2, "只应匹配日期备份: {:?}", backups);
        assert!(backups
            .iter()
            .all(|p| p.to_string_lossy().contains(".backup-")));

        std::fs::remove_dir_all(&dir).ok();
    }
}


/// 端到端验证「备份 → 覆写 → 还原」在真实文件上不丢任何内容。
///
/// 用 `RUNJAM_HOME` 指向临时目录，因此**绝不触碰真实 ~/.claude / ~/.codex**。
/// 这些测试串行执行（共用进程级环境变量），用互斥锁保护。
#[cfg(test)]
mod e2e_native_tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 在临时 HOME 下准备一套「原生」配置，返回 (home, 各文件原生内容)
    fn setup_isolated_home(tag: &str) -> (PathBuf, std::collections::HashMap<String, String>) {
        let home = std::env::temp_dir().join(format!("runjam_e2e_{}_{}", tag, std::process::id()));
        std::fs::remove_dir_all(&home).ok();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::create_dir_all(home.join(".gemini")).unwrap();

        let claude = r#"{
  "model": "A/Ornith-1.5-9B-Q4_K_M.gguf",
  "env": {
    "ANTHROPIC_BASE_URL": "https://api.anthropic.com",
    "ANTHROPIC_MODEL": "A/Ornith-1.5-9B-Q4_K_M.gguf"
  },
  "permissions": { "allow": ["Bash"] }
}"#;
        let codex = r#"model = "gpt-6-astra"
model_reasoning_effort = "medium"
service_tier = "priority"

[features]
hooks = true

[marketplaces.patsnap-openai-plugins]
source = "http://git.patsnap.com/patsnap/openai-plugins.git"
source_type = "git"

[plugins."browser@openai-bundled"]
enabled = true
"#;
        let gemini = r#"{
  "env": { "GEMINI_MODEL": "gemini-2.5-pro" },
  "theme": "dark"
}"#;
        std::fs::write(home.join(".claude/settings.json"), claude).unwrap();
        std::fs::write(home.join(".codex/config.toml"), codex).unwrap();
        std::fs::write(home.join(".gemini/settings.json"), gemini).unwrap();

        let mut map = std::collections::HashMap::new();
        map.insert("claude".into(), claude.to_string());
        map.insert("codex".into(), codex.to_string());
        map.insert("gemini".into(), gemini.to_string());
        (home, map)
    }

    fn read(p: PathBuf) -> String {
        std::fs::read_to_string(&p).unwrap_or_default()
    }

    #[test]
    fn test_e2e_snapshot_then_restore_preserves_native_content_exactly() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (home, native) = setup_isolated_home("exact");
        std::env::set_var("RUNJAM_HOME", &home);

        // 1) 未覆写前：快照不存在
        assert!(!home.join(".claude/settings.json.runjam-native").exists());

        // 2) 模拟用户配置 runjam 模型 → 覆写原生配置
        let entry = ModelEntry {
            id: "m1".into(), name: "gpt-6-luna".into(), alias: "gpt-6-luna".into(),
            provider: "openai".into(), provider_name: "OpenAI".into(), provider_icon: "".into(),
            api_base: "http://127.0.0.1:59268".into(), api_key: "sk-test".into(),
            protocol: "openai_chat".into(), context_window: 0, support_reasoning: false,
            support_tools: true, tags: vec![], use_proxy: false, force_reasoning_none: false,
        };
        for agent in ["claude-code", "codex-cli", "gemini-cli"] {
            ensure_native_snapshot(agent); // 覆写前留存
            sync_to_agent(agent, &[entry.clone()]).expect("sync should succeed");
        }

        // 快照必须存在，且**逐字节等于**原生内容
        assert_eq!(read(native_snapshot_path("claude-code").unwrap()), native["claude"]);
        assert_eq!(read(native_snapshot_path("codex-cli").unwrap()), native["codex"]);
        assert_eq!(read(native_snapshot_path("gemini-cli").unwrap()), native["gemini"]);

        // 覆写确实改动了文件（否则测试没有意义）
        assert_ne!(read(home.join(".claude/settings.json")), native["claude"]);
        assert_ne!(read(home.join(".codex/config.toml")), native["codex"]);

        // 3) 还原
        for agent in ["claude-code", "codex-cli", "gemini-cli"] {
            restore_agent_config(agent).expect("restore should succeed");
        }

        // 4) 恢复后必须与原生内容逐字节一致（这是本功能的核心保证）
        assert_eq!(read(home.join(".claude/settings.json")), native["claude"], "claude 配置必须完整还原");
        assert_eq!(read(home.join(".codex/config.toml")), native["codex"], "codex 配置必须完整还原");
        assert_eq!(read(home.join(".gemini/settings.json")), native["gemini"], "gemini 配置必须完整还原");

        std::env::remove_var("RUNJAM_HOME");
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn test_e2e_native_snapshot_is_idempotent_and_never_overwritten() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (home, native) = setup_isolated_home("idem");
        std::env::set_var("RUNJAM_HOME", &home);

        // 第一次留存
        ensure_native_snapshot("claude-code");
        let snap = native_snapshot_path("claude-code").unwrap();
        assert_eq!(read(snap.clone()), native["claude"]);

        // 再覆写一次文件，然后再次调用 ensure：快照绝不能被改写
        std::fs::write(home.join(".claude/settings.json"), r#"{"model":"changed"}"#).unwrap();
        ensure_native_snapshot("claude-code");
        assert_eq!(read(snap.clone()), native["claude"], "快照必须保持首次状态");

        std::env::remove_var("RUNJAM_HOME");
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn test_e2e_read_native_models_prefers_snapshot_over_current() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (home, _native) = setup_isolated_home("prefer");
        std::env::set_var("RUNJAM_HOME", &home);

        // 覆写 claude 配置（指向 runjam proxy + runjam 管理的模型）
        ensure_native_snapshot("claude-code");
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"model":"gpt-6-luna","env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:59268/anthropic"}}"#,
        ).unwrap();

        let models = read_native_models("claude-code");
        assert_eq!(models.len(), 1);
        // 必须报出原生模型，而不是 runjam 覆写后的 gpt-6-luna
        assert_eq!(models[0].name, "A/Ornith-1.5-9B-Q4_K_M.gguf");
        assert!(models[0].is_overridden, "应识别出配置已被 runjam 改写");
        assert!(models[0].has_snapshot);
        assert!(models[0].source_path.ends_with(".runjam-native"));

        std::env::remove_var("RUNJAM_HOME");
        std::fs::remove_dir_all(&home).ok();
    }

    /// 就地清除路径（无快照、无日期备份）只应删除确认由 runjam 写入的值。
    /// 用户自己的本地推理端点必须保留 —— 这是最容易被误删的场景。
    #[test]
    fn test_e2e_inplace_cleanup_preserves_user_own_local_endpoint() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (home, _native) = setup_isolated_home("inplace");
        std::env::set_var("RUNJAM_HOME", &home);

        // 用户在 runjam 之外自己配的 claude：走本地 llama-server，并且**没有**
        // 任何 runjam 痕迹（无 models 键、端点非 runjam 端口）
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"model":"A/Ornith-1.5-9B-Q4_K_M.gguf","env":{"ANTHROPIC_BASE_URL":"http://localhost:19090/v1","ANTHROPIC_MODEL":"A/Ornith-1.5-9B-Q4_K_M.gguf"}}"#,
        ).unwrap();
        std::fs::remove_file(home.join(".claude/settings.json.runjam-native")).ok();

        restore_agent_config("claude-code").expect("restore should succeed");

        let after: serde_json::Value =
            serde_json::from_str(&read(home.join(".claude/settings.json"))).unwrap();
        // 用户自己的本地端点与模型必须原样保留
        assert_eq!(
            after["env"]["ANTHROPIC_BASE_URL"], "http://localhost:19090/v1",
            "用户原生本地端点不能被清除"
        );
        assert_eq!(after["model"], "A/Ornith-1.5-9B-Q4_K_M.gguf", "用户原生模型不能被清除");
        assert_eq!(after["env"]["ANTHROPIC_MODEL"], "A/Ornith-1.5-9B-Q4_K_M.gguf");

        std::env::remove_var("RUNJAM_HOME");
        std::fs::remove_dir_all(&home).ok();
    }

    /// 反之：确认由 runjam 写入的字段必须被清除干净。
    #[test]
    fn test_e2e_inplace_cleanup_removes_runjam_written_fields() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (home, _native) = setup_isolated_home("cleanup");
        std::env::set_var("RUNJAM_HOME", &home);

        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"model":"gpt-6-luna","models":[{"id":"m1","name":"gpt-6-luna"}],"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:59268/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-x","ANTHROPIC_MODEL":"gpt-6-luna","ANTHROPIC_DEFAULT_SONNET_MODEL":"gpt-6-luna"}}"#,
        ).unwrap();
        std::fs::remove_file(home.join(".claude/settings.json.runjam-native")).ok();

        restore_agent_config("claude-code").expect("restore should succeed");

        let after: serde_json::Value =
            serde_json::from_str(&read(home.join(".claude/settings.json"))).unwrap();
        assert!(after.get("models").is_none(), "runjam 写入的 models 键必须被移除");
        assert!(after.get("model").is_none(), "runjam 写入的 model 必须被移除");
        assert!(after["env"].get("ANTHROPIC_BASE_URL").is_none(), "runjam 代理端点必须被移除");
        assert!(after["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
        assert!(after["env"].get("ANTHROPIC_MODEL").is_none());
        assert!(after["env"].get("ANTHROPIC_DEFAULT_SONNET_MODEL").is_none());

        std::env::remove_var("RUNJAM_HOME");
        std::fs::remove_dir_all(&home).ok();
    }

    /// 存量用户的日期备份本身可能已含 runjam 的代理端点（历史上就是这样生成的）。
    /// 恢复这类备份后必须擦掉该端点，否则 CLI 会指向一个只在应用运行时存在的端口。
    #[test]
    fn test_e2e_backup_restore_scrubs_runjam_proxy_endpoint() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (home, _native) = setup_isolated_home("scrub");
        std::env::set_var("RUNJAM_HOME", &home);

        // 当前配置（被 runjam 覆写） + 已被污染的日期备份（含 runjam 端点），无快照
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"model":"gpt-6-luna","env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:59268/anthropic"}}"#,
        ).unwrap();
        std::fs::write(
            home.join(".claude/settings.json.backup-2026-09-28"),
            r#"{"model":"A/Ornith-1.5-9B-Q4_K_M.gguf","env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:59268/anthropic","ANTHROPIC_MODEL":"A/Ornith-1.5-9B-Q4_K_M.gguf"}}"#,
        ).unwrap();
        std::fs::remove_file(home.join(".claude/settings.json.runjam-native")).ok();

        restore_agent_config("claude-code").expect("restore should succeed");

        let after: serde_json::Value =
            serde_json::from_str(&read(home.join(".claude/settings.json"))).unwrap();
        // 原生模型名从备份恢复
        assert_eq!(after["model"], "A/Ornith-1.5-9B-Q4_K_M.gguf");
        // 但 runjam 的代理端点必须被擦洗掉（回退到 CLI 默认端点）
        assert!(
            after["env"].get("ANTHROPIC_BASE_URL").is_none(),
            "恢复自受污染备份时，runjam 代理端点必须被擦洗: {after}"
        );

        std::env::remove_var("RUNJAM_HOME");
        std::fs::remove_dir_all(&home).ok();
    }

    /// 反之：备份里的用户真实端点必须原样保留。
    #[test]
    fn test_e2e_backup_restore_keeps_user_own_endpoint() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (home, _native) = setup_isolated_home("keependpoint");
        std::env::set_var("RUNJAM_HOME", &home);

        std::fs::write(home.join(".claude/settings.json"), r#"{"model":"gpt-6-luna"}"#).unwrap();
        std::fs::write(
            home.join(".claude/settings.json.backup-2026-09-28"),
            r#"{"model":"A/Ornith-1.5-9B-Q4_K_M.gguf","env":{"ANTHROPIC_BASE_URL":"http://localhost:19090/v1"}}"#,
        ).unwrap();
        std::fs::remove_file(home.join(".claude/settings.json.runjam-native")).ok();

        restore_agent_config("claude-code").expect("restore should succeed");

        let after: serde_json::Value =
            serde_json::from_str(&read(home.join(".claude/settings.json"))).unwrap();
        assert_eq!(
            after["env"]["ANTHROPIC_BASE_URL"], "http://localhost:19090/v1",
            "用户自己的本地端点必须保留"
        );

        std::env::remove_var("RUNJAM_HOME");
        std::fs::remove_dir_all(&home).ok();
    }

    /// 手动快照的两个前提：不能覆盖已有快照，也不能从已被 runjam 改写的
    /// 配置里取快照（否则「切回原生」会恢复出一份假的"原生"状态）。
    #[test]
    fn test_manual_snapshot_rejects_overwrite_and_overridden_source() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (home, native) = setup_isolated_home("manual");
        std::env::set_var("RUNJAM_HOME", &home);

        let cfg = home.join(".claude/settings.json");
        let snap = home.join(".claude/settings.json.runjam-native");
        std::fs::remove_file(&snap).ok();

        // 原生状态：可以快照
        assert!(try_native_snapshot("claude-code").is_ok());
        assert_eq!(read(snap.clone()), native["claude"]);

        // 已有快照：拒绝覆盖
        let err = try_native_snapshot("claude-code").unwrap_err();
        assert_eq!(err, "snapshot_exists");
        assert_eq!(read(snap.clone()), native["claude"], "已有快照不能被覆盖");

        // 已被 runjam 改写的配置：拒绝取快照
        std::fs::remove_file(&snap).ok();
        std::fs::write(&cfg, r#"{"model":"gpt-6-luna","models":[{"id":"m1"}]}"#).unwrap();
        let err2 = try_native_snapshot("claude-code").unwrap_err();
        assert_eq!(err2, "config_already_overridden");
        assert!(!snap.exists(), "被改写的配置不应产生快照");

        std::env::remove_var("RUNJAM_HOME");
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn test_e2e_falls_back_to_date_backup_for_legacy_users() {
        // 存量用户：在快照功能上线前就已被覆写，只有 .backup-<date>
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (home, _native) = setup_isolated_home("legacy");
        std::env::set_var("RUNJAM_HOME", &home);

        // 写入一份"被覆写过的"当前配置 + 原生日期备份，且**没有**快照
        std::fs::write(
            home.join(".codex/config.toml"),
            "model = \"gpt-6-luna\"\nmodel_provider = \"custom\"\n",
        ).unwrap();
        std::fs::write(
            home.join(".codex/config.toml.backup-2026-01-01"),
            "model = \"gpt-6-astra\"\n",
        ).unwrap();

        let models = read_native_models("codex-cli");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "gpt-6-astra", "应退回日期备份里的原生模型");
        assert!(!models[0].has_snapshot);
        assert!(models[0].source_path.contains(".backup-"));

        std::env::remove_var("RUNJAM_HOME");
        std::fs::remove_dir_all(&home).ok();
    }
}

