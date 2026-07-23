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
                    if let Ok(doc) = content.parse::<toml::Value>() {
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
            use_proxy: false,
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
                    use_proxy: false,
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
                });
            }
        }
    }
    
    models
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
    let home = directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

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
    let home = directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

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
    let home = directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

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
    let home = directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    match agent_id {
        "claude-code" => {
            let path = home.join(".claude").join("settings.json");
            if path.exists() {
                let s = std::fs::read_to_string(&path).unwrap_or_default();
                let mut settings: serde_json::Value = serde_json::from_str(&s).unwrap_or(serde_json::json!({}));

                if let Some(obj) = settings.as_object_mut() {
                    obj.remove("runjam_models");
                    obj.remove("models");
                    obj.remove("model");
                    if let Some(env_obj) = obj.get_mut("env").and_then(|v| v.as_object_mut()) {
                        env_obj.remove("ANTHROPIC_AUTH_TOKEN");
                        env_obj.remove("ANTHROPIC_BASE_URL");
                        env_obj.remove("ANTHROPIC_MODEL");
                        env_obj.remove("ANTHROPIC_DEFAULT_HAIKU_MODEL");
                        env_obj.remove("ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME");
                        env_obj.remove("ANTHROPIC_DEFAULT_SONNET_MODEL");
                        env_obj.remove("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME");
                        env_obj.remove("ANTHROPIC_DEFAULT_OPUS_MODEL");
                        env_obj.remove("ANTHROPIC_DEFAULT_OPUS_MODEL_NAME");
                    }
                }

                std::fs::write(&path, serde_json::to_string_pretty(&settings).unwrap_or_default())
                    .map_err(|e| format!("Failed to restore claude config: {}", e))?;
            }
        }
        "codex-cli" => {
            let path = home.join(".codex").join("config.toml");
            if path.exists() {
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                let mut doc: toml::Value = toml::from_str(&content).unwrap_or(toml::Value::Table(toml::value::Table::new()));
                
                if let toml::Value::Table(ref mut table) = doc {
                    table.remove("runjam");
                    table.remove("model_provider");
                    table.remove("model");
                    table.remove("disable_response_storage");
                    
                    // Clean up fields we wrote in model_providers.custom
                    if let Some(providers) = table.get_mut("model_providers").and_then(|v| v.as_table_mut()) {
                        if let Some(custom) = providers.get_mut("custom").and_then(|v| v.as_table_mut()) {
                            custom.remove("api_key");
                            custom.remove("requires_openai_auth");
                            custom.remove("wire_api");
                            // Remove base_url if it's a proxy URL
                            if let Some(url) = custom.get("base_url").and_then(|v| v.as_str()) {
                                if url.starts_with("http://127.0.0.1:") {
                                    custom.remove("base_url");
                                }
                            }
                        }
                    }
                }
                
                let cleaned = toml::to_string_pretty(&doc).unwrap_or_default();
                std::fs::write(&path, cleaned.trim_end())
                    .map_err(|e| format!("Failed to restore codex config: {}", e))?;

                // Also clean up .env and auth.json files
                let env_path = home.join(".codex").join(".env");
                if env_path.exists() {
                    std::fs::remove_file(&env_path).ok();
                }
                let auth_path = home.join(".codex").join("auth.json");
                if auth_path.exists() {
                    std::fs::remove_file(&auth_path).ok();
                }
            }
        }
        "gemini-cli" => {
            let path = home.join(".gemini").join("settings.json");
            if path.exists() {
                let s = std::fs::read_to_string(&path).unwrap_or_default();
                let mut settings: serde_json::Value = serde_json::from_str(&s).unwrap_or(serde_json::json!({}));

                if let Some(obj) = settings.as_object_mut() {
                    obj.remove("runjam_models");
                    if let Some(env_obj) = obj.get_mut("env").and_then(|v| v.as_object_mut()) {
                        env_obj.remove("GOOGLE_GEMINI_BASE_URL");
                        env_obj.remove("GEMINI_API_KEY");
                        env_obj.remove("GEMINI_MODEL");
                    }
                }

                std::fs::write(&path, serde_json::to_string_pretty(&settings).unwrap_or_default())
                    .map_err(|e| format!("Failed to restore gemini config: {}", e))?;
            }
        }
        _ => {}
    }

    Ok(())
}

/// Backup the current agent config file before syncing.
pub fn backup_agent_config(agent_id: &str) -> Result<(), String> {
    let home = directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let config_path = match agent_id {
        "claude-code" => home.join(".claude").join("settings.json"),
        "codex-cli" => home.join(".codex").join("config.toml"),
        "gemini-cli" => home.join(".gemini").join("settings.json"),
        _ => return Ok(()),
    };

    if config_path.exists() {
        let bak_path = format!("{}.bak", config_path.display());
        std::fs::copy(&config_path, &bak_path)
            .map_err(|e| format!("Failed to backup {} config: {}", agent_id, e))?;
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
