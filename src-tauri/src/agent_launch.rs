//! Shared native launch plans and per-run managed Codex configs.
use crate::models_config::ModelEntry;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const NATIVE_MODEL_ID: &str = "__runjam_native__";

pub fn supports_native(agent: &str) -> bool {
    matches!(agent, "claude-code" | "gemini-cli" | "codex-cli")
}

/// Read only the current CLI config. No snapshot restoration or credential copy.
/// Gemini's root is HOME-like; the other two point at their config directory.
pub fn read_native_launch_config(agent: &str, file: &Path) -> Result<(PathBuf, String), String> {
    if !supports_native(agent) {
        return Err(format!("Native mode is unsupported for {agent}"));
    }
    let dir = file.parent().ok_or("Invalid native config path")?;
    if !dir.is_dir() {
        return Err(format!(
            "Native config directory is missing for {agent}; configure or log in to the CLI first"
        ));
    }
    let raw = if file.exists() {
        std::fs::read_to_string(file).map_err(|e| format!("Read native {agent} config: {e}"))?
    } else {
        String::new()
    };
    let model = if agent == "codex-cli" {
        let doc: toml::Value =
            toml::from_str(&raw).map_err(|e| format!("Invalid native Codex config: {e}"))?;
        doc.get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("CLI default")
            .to_string()
    } else {
        let doc: serde_json::Value = if raw.is_empty() && !file.exists() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&raw).map_err(|e| format!("Invalid native {agent} config: {e}"))?
        };
        if !doc.is_object() {
            return Err(format!("Native {agent} config must be a JSON object"));
        }
        doc.get("model")
            .and_then(|v| {
                v.as_str()
                    .or_else(|| v.get("name").and_then(|n| n.as_str()))
            })
            .or_else(|| {
                doc.get("env")
                    .and_then(|v| v.get("ANTHROPIC_MODEL"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("CLI default")
            .to_string()
    };
    let root = if agent == "gemini-cli" {
        dir.parent().ok_or("Invalid native Gemini root")?
    } else {
        dir
    };
    Ok((root.to_path_buf(), model))
}

pub fn prepare_native(agent: &str, reasoning_disabled: bool) -> Result<AgentLaunchPlan, String> {
    let file = crate::models_config::native_config_path(agent)
        .ok_or("Native config path is unavailable")?;
    let (root, requested_model) = read_native_launch_config(agent, &file)?;
    Ok(AgentLaunchPlan {
        info: RuntimeInfo {
            run_id: uuid::Uuid::new_v4().to_string(),
            model_id: NATIVE_MODEL_ID.into(),
            config_mode: ConfigMode::Native,
            config_root: root.display().to_string(),
            requested_model,
            authentication: "native_cli".into(),
            permission_mode: "inherit".into(),
            reasoning_disabled,
            acp_session_id: None,
        },
        proxy_token: None,
        lease: None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigMode {
    Native,
    Managed,
}

pub fn resolve_mode(
    explicit: Option<&str>,
    model: Option<&str>,
    default: ConfigMode,
) -> Result<ConfigMode, String> {
    let model = model.filter(|m| !m.is_empty());
    let mode = match explicit {
        Some("native") => ConfigMode::Native,
        Some("managed") => ConfigMode::Managed,
        Some(other) => return Err(format!("Unknown config mode: {other}")),
        None if model == Some(NATIVE_MODEL_ID) => ConfigMode::Native,
        None if model.is_some() => ConfigMode::Managed,
        None => default,
    };
    match (mode, model) {
        (ConfigMode::Native, Some(m)) if m != NATIVE_MODEL_ID => {
            Err("Native mode cannot use a managed model ID".into())
        }
        (ConfigMode::Managed, None) | (ConfigMode::Managed, Some(NATIVE_MODEL_ID)) => {
            Err("Select a RunJam model for managed mode".into())
        }
        _ => Ok(mode),
    }
}

pub fn write_managed_config(
    template: &Path,
    runs: &Path,
    run_id: &str,
    model: &ModelEntry,
    proxy_url: &str,
) -> Result<PathBuf, String> {
    if run_id.is_empty()
        || !run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("Invalid run ID".into());
    }
    let mut doc: toml::Value = if template.exists() {
        let raw =
            std::fs::read_to_string(template).map_err(|e| format!("Read managed template: {e}"))?;
        toml::from_str(&raw).map_err(|e| format!("Invalid managed template: {e}"))?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let table = doc
        .as_table_mut()
        .ok_or("Managed config must be a TOML table")?;
    for key in [
        "runjam",
        "model_providers",
        "profiles",
        "profile",
        "forced_login_method",
        "forced_chatgpt_workspace_id",
        "openai_base_url",
        "chatgpt_base_url",
    ] {
        table.remove(key);
    }
    for key in [
        "api_key",
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "env",
        "shell_environment_policy",
    ] {
        table.remove(key);
    }
    table.insert("model".into(), model.name.clone().into());
    table.insert("model_provider".into(), "runjam".into());
    table.insert("cli_auth_credentials_store".into(), "ephemeral".into());
    table.insert(
        "model_providers".into(),
        toml::Value::try_from(serde_json::json!({
            "runjam": { "name":"RunJam session", "base_url":proxy_url, "wire_api":"responses",
                "env_key":"RUNJAM_PROXY_TOKEN", "requires_openai_auth":false }
        }))
        .map_err(|e| e.to_string())?,
    );
    let dir = runs.join(run_id).join("codex");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Create managed config directory: {e}"))?;
    std::fs::write(
        dir.join("config.toml"),
        toml::to_string_pretty(&doc).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("Write managed config: {e}"))?;
    Ok(dir)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub run_id: String,
    pub model_id: String,
    pub config_mode: ConfigMode,
    pub config_root: String,
    pub requested_model: String,
    pub authentication: String,
    pub permission_mode: String,
    pub reasoning_disabled: bool,
    pub acp_session_id: Option<String>,
}

pub struct AgentLaunchPlan {
    pub info: RuntimeInfo,
    pub proxy_token: Option<String>,
    lease: Option<(
        std::sync::Arc<std::sync::Mutex<crate::proxy::ProxyState>>,
        String,
    )>,
}

pub fn apply_config_environment(
    cmd: &mut std::process::Command,
    agent: &str,
    plan: &AgentLaunchPlan,
) -> Result<(), String> {
    let var = crate::agent_isolation::config_dir_env_var(agent)
        .ok_or("Agent config environment is unavailable")?;
    cmd.env(var, &plan.info.config_root);
    if plan.info.config_mode == ConfigMode::Native {
        cmd.env_remove("RUNJAM_PROXY_TOKEN");
    } else if let Some(token) = &plan.proxy_token {
        for key in [
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
            "OPENAI_API_BASE",
            "CODEX_API_BASE",
            "OPENAI_BASE_URL",
        ] {
            cmd.env_remove(key);
        }
        cmd.env("RUNJAM_PROXY_TOKEN", token);
    }
    Ok(())
}

impl Drop for AgentLaunchPlan {
    fn drop(&mut self) {
        if let Some((state, id)) = &self.lease {
            if let Ok(mut ps) = state.lock() {
                ps.session_routes.remove(id);
            }
        }
    }
}

impl AgentLaunchPlan {
    pub fn runtime_info(&self) -> RuntimeInfo {
        let mut info = self.info.clone();
        if let Some((state, id)) = &self.lease {
            if let Ok(ps) = state.lock() {
                if let Some(route) = ps.session_routes.get(id) {
                    info.reasoning_disabled = route.reasoning_disabled;
                }
            }
        }
        info
    }
}

pub fn prepare_codex(
    app: &tauri::AppHandle,
    mode: ConfigMode,
    model_id: Option<&str>,
    reasoning_disabled: bool,
) -> Result<AgentLaunchPlan, String> {
    use tauri::Manager;
    let run_id = uuid::Uuid::new_v4().to_string();
    if mode == ConfigMode::Native {
        return prepare_native("codex-cli", reasoning_disabled);
    }
    let id = model_id.ok_or("Managed model is required")?;
    let model = crate::commands::models_cmd::get_models(app.state())
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("Model {id} is not configured"))?;
    let proxy = app
        .state::<std::sync::Arc<std::sync::Mutex<crate::proxy::ProxyState>>>()
        .inner()
        .clone();
    let port = {
        let ps = proxy.lock().map_err(|e| e.to_string())?;
        if !ps.running || ps.port == 0 {
            return Err("RunJam model proxy is unavailable".into());
        }
        ps.port
    };
    let token = uuid::Uuid::new_v4().to_string();
    let template = crate::agent_isolation::isolated_config_file("codex-cli")
        .ok_or("Missing managed template path")?;
    let runs = crate::agent_isolation::isolated_config_root().join("runs");
    let root = write_managed_config(
        &template,
        &runs,
        &run_id,
        &model,
        &format!("http://127.0.0.1:{port}/rj/{run_id}/v1"),
    )?;
    proxy
        .lock()
        .map_err(|e| e.to_string())?
        .session_routes
        .insert(
            run_id.clone(),
            crate::proxy::SessionProxyRoute {
                model: model.clone(),
                token: token.clone(),
                reasoning_disabled,
            },
        );
    Ok(AgentLaunchPlan {
        info: RuntimeInfo {
            run_id: run_id.clone(),
            model_id: id.into(),
            config_mode: mode,
            config_root: root.display().to_string(),
            requested_model: model.name,
            authentication: "runjam_proxy".into(),
            permission_mode: "ask_approval".into(),
            reasoning_disabled,
            acp_session_id: None,
        },
        proxy_token: Some(token),
        lease: Some((proxy, run_id)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_commands_select_the_right_config_root_without_runjam_proxy_auth() {
        for (agent, var) in [
            ("claude", "CLAUDE_CONFIG_DIR"),
            ("gemini", "GEMINI_CLI_HOME"),
            ("codex", "CODEX_HOME"),
        ] {
            let info = RuntimeInfo {
                run_id: "test".into(),
                model_id: NATIVE_MODEL_ID.into(),
                config_mode: ConfigMode::Native,
                config_root: "/native-test-root".into(),
                requested_model: "CLI default".into(),
                authentication: "native_cli".into(),
                permission_mode: "full_access".into(),
                reasoning_disabled: false,
                acp_session_id: None,
            };
            let plan = AgentLaunchPlan {
                info,
                proxy_token: None,
                lease: None,
            };
            let mut cmd = std::process::Command::new("unused-test-program");
            cmd.env("RUNJAM_PROXY_TOKEN", "stale-runjam-token");
            apply_config_environment(&mut cmd, agent, &plan).unwrap();
            let env: std::collections::HashMap<_, _> = cmd.get_envs().collect();
            assert_eq!(env[std::ffi::OsStr::new(var)].unwrap(), "/native-test-root");
            assert_eq!(env[std::ffi::OsStr::new("RUNJAM_PROXY_TOKEN")], None);
            for other in ["CLAUDE_CONFIG_DIR", "GEMINI_CLI_HOME", "CODEX_HOME"] {
                if other != var {
                    assert!(!env.contains_key(std::ffi::OsStr::new(other)));
                }
            }
        }
    }
    #[test]
    fn native_launch_reads_current_config_and_keeps_auth_and_snapshot_untouched() {
        let root =
            std::env::temp_dir().join(format!("runjam-native-launch-{}", uuid::Uuid::new_v4()));
        for (agent, relative, raw) in [
            (
                "claude-code",
                ".claude/settings.json",
                r#"{"model":"claude-native"}"#,
            ),
            (
                "gemini-cli",
                ".gemini/settings.json",
                r#"{"model":{"name":"gemini-native"}}"#,
            ),
            ("codex-cli", ".codex/config.toml", "model = 'codex-native'"),
        ] {
            let file = root.join(relative);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, raw).unwrap();
            let auth = file.parent().unwrap().join("auth.json");
            std::fs::write(&auth, "native credentials").unwrap();
            let (config_root, model) = read_native_launch_config(agent, &file).unwrap();
            assert!(model.ends_with("-native"));
            let expected = if agent == "gemini-cli" {
                root.as_path()
            } else {
                file.parent().unwrap()
            };
            assert_eq!(config_root, expected);
            assert_eq!(std::fs::read_to_string(&file).unwrap(), raw);
            assert_eq!(std::fs::read_to_string(auth).unwrap(), "native credentials");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    fn model() -> ModelEntry {
        serde_json::from_value(serde_json::json!({
            "id":"m1", "name":"same-name", "alias":"", "provider":"custom",
            "provider_name":"Test", "provider_icon":"", "api_base":"https://example.test/v1",
            "api_key":"UPSTREAM_SECRET", "protocol":"openai_chat", "context_window":0,
            "support_reasoning":true, "tags":[]
        }))
        .unwrap()
    }
    #[test]
    fn explicit_native_and_managed_choices_are_validated() {
        assert_eq!(
            resolve_mode(Some("native"), Some(NATIVE_MODEL_ID), ConfigMode::Managed).unwrap(),
            ConfigMode::Native
        );
        assert_eq!(
            resolve_mode(None, Some("m1"), ConfigMode::Native).unwrap(),
            ConfigMode::Managed
        );
        assert_eq!(
            resolve_mode(None, None, ConfigMode::Native).unwrap(),
            ConfigMode::Native
        );
        assert!(resolve_mode(Some("native"), Some("m1"), ConfigMode::Native).is_err());
        assert!(resolve_mode(Some("managed"), None, ConfigMode::Native).is_err());
        assert!(resolve_mode(Some("invalid"), None, ConfigMode::Native).is_err());
    }
    #[test]
    fn managed_runs_have_independent_configs_and_no_upstream_credentials() {
        let root =
            std::env::temp_dir().join(format!("runjam-launch-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let template = root.join("template.toml");
        let original = "model = 'old'\nmodel_provider = 'old'\n[model_providers.old]\napi_key = 'OLD_SECRET'\n";
        std::fs::write(&template, original).unwrap();
        let a = write_managed_config(
            &template,
            &root,
            "a",
            &model(),
            "http://127.0.0.1:59268/rj/a/v1",
        )
        .unwrap();
        let b = write_managed_config(
            &template,
            &root,
            "b",
            &model(),
            "http://127.0.0.1:59268/rj/b/v1",
        )
        .unwrap();
        assert_ne!(a, b);
        let raw = std::fs::read_to_string(a.join("config.toml")).unwrap();
        assert!(raw.contains("/rj/a/v1"));
        assert!(!raw.contains("SECRET"));
        assert!(!a.join("auth.json").exists());
        assert_eq!(std::fs::read_to_string(template).unwrap(), original);
        assert!(
            write_managed_config(Path::new("missing"), &root, "../escape", &model(), "x").is_err()
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn user_project_skills_cannot_be_deleted_by_runjam() {
        let root =
            std::env::temp_dir().join(format!("runjam-skill-owner-test-{}", uuid::Uuid::new_v4()));
        let skill = root.join(".codex/skills/user-skill");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "user owned").unwrap();
        let result =
            crate::skill::remove_single_skill(root.to_str().unwrap(), "codex", "user-skill");
        assert!(result.is_err());
        assert!(skill.join("SKILL.md").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
