//! Launch contract for the App Server based Codex ACP adapter.
use crate::agent_launch::RuntimeInfo;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const PACKAGE: &str = "@agentclientprotocol/codex-acp@2.2.2";

pub fn entry_point(install_dir: &Path) -> PathBuf {
    install_dir.join("node_modules/@agentclientprotocol/codex-acp/dist/index.js")
}

pub fn configure_command(
    cmd: &mut Command,
    codex_path: &Path,
    runtime: Option<&RuntimeInfo>,
    permission: &str,
) -> Result<(), String> {
    use crate::agent_launch::ConfigMode;
    cmd.env("CODEX_PATH", codex_path);
    // These are adapter-specific controls, not native CLI configuration.
    // Never inherit stale provider/auth/permission overrides from the parent.
    for key in [
        "CODEX_CONFIG",
        "MODEL_PROVIDER",
        "DEFAULT_AUTH_REQUEST",
        "INITIAL_AGENT_MODE",
    ] {
        cmd.env_remove(key);
    }
    let mut config = serde_json::Map::new();
    if let Some(info) = runtime.filter(|r| r.config_mode == ConfigMode::Managed) {
        let raw = std::fs::read_to_string(Path::new(&info.config_root).join("config.toml"))
            .map_err(|e| format!("Read Codex run config: {e}"))?;
        let doc: toml::Value =
            toml::from_str(&raw).map_err(|e| format!("Invalid Codex run config: {e}"))?;
        let provider = doc
            .get("model_providers")
            .and_then(|p| p.get("runjam"))
            .ok_or("RunJam provider missing from Codex run config")?;
        config.insert("model".into(), info.requested_model.clone().into());
        config.insert("model_provider".into(), "runjam".into());
        config.insert(
            "model_providers".into(),
            serde_json::json!({"runjam": provider}),
        );
        cmd.env("MODEL_PROVIDER", "runjam");
    }
    if permission != "inherit" {
        let (approval, sandbox) = crate::agent_permissions::codex_permission_overrides(permission);
        config.insert("approval_policy".into(), approval.into());
        config.insert("sandbox_mode".into(), sandbox.into());
        if sandbox == "workspace-write" {
            config.insert(
                "sandbox_workspace_write".into(),
                serde_json::json!({"network_access": true}),
            );
        }
    }
    if !config.is_empty() {
        cmd.env(
            "CODEX_CONFIG",
            serde_json::Value::Object(config).to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_launch::ConfigMode;
    use std::ffi::OsStr;

    fn env(cmd: &Command, key: &str) -> Option<String> {
        cmd.get_envs()
            .find(|(k, _)| *k == OsStr::new(key))
            .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    fn runtime(root: &Path, mode: ConfigMode) -> RuntimeInfo {
        RuntimeInfo {
            run_id: "test".into(),
            model_id: "model-id".into(),
            config_mode: mode,
            config_root: root.display().to_string(),
            requested_model: "selected-model".into(),
            authentication: "native_cli".into(),
            permission_mode: "inherit".into(),
            reasoning_disabled: false,
            acp_session_id: None,
        }
    }

    #[test]
    fn uses_maintained_node_adapter_not_the_archived_binary() {
        assert_eq!(
            entry_point(Path::new("/data/codex")),
            PathBuf::from("/data/codex/node_modules/@agentclientprotocol/codex-acp/dist/index.js")
        );
        assert_eq!(PACKAGE, "@agentclientprotocol/codex-acp@2.2.2");
    }

    #[test]
    fn native_launch_uses_local_cli_without_model_or_permission_overrides() {
        let mut cmd = Command::new("node");
        let info = runtime(Path::new("/native"), ConfigMode::Native);
        configure_command(&mut cmd, Path::new("/local/codex"), Some(&info), "inherit").unwrap();
        assert_eq!(env(&cmd, "CODEX_PATH").as_deref(), Some("/local/codex"));
        assert!(cmd.get_args().next().is_none());
        for key in [
            "CODEX_CONFIG",
            "MODEL_PROVIDER",
            "DEFAULT_AUTH_REQUEST",
            "INITIAL_AGENT_MODE",
        ] {
            assert!(
                cmd.get_envs()
                    .any(|(k, v)| k == OsStr::new(key) && v.is_none()),
                "{key} must not leak into native mode"
            );
        }
    }

    #[test]
    fn read_only_keeps_the_sandbox_without_the_removed_untrusted_policy() {
        let mut cmd = Command::new("node");
        configure_command(&mut cmd, Path::new("/local/codex"), None, "read_only").unwrap();
        let config: serde_json::Value =
            serde_json::from_str(&env(&cmd, "CODEX_CONFIG").unwrap()).unwrap();
        assert_eq!(config["sandbox_mode"], "read-only");
        assert_eq!(config["approval_policy"], "on-request");
        assert!(config.get("sandbox_workspace_write").is_none());
    }

    #[test]
    fn managed_launch_passes_model_provider_and_permissions_as_json_not_cli_flags() {
        let dir = std::env::temp_dir().join(format!("runjam-adapter-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), "model_provider = 'runjam'\n[model_providers.runjam]\nbase_url = 'http://127.0.0.1:1234/rj/test/v1'\nenv_key = 'RUNJAM_PROXY_TOKEN'\nrequires_openai_auth = false\nwire_api = 'responses'\n").unwrap();
        let mut cmd = Command::new("node");
        let info = runtime(&dir, ConfigMode::Managed);
        configure_command(
            &mut cmd,
            Path::new("/local/codex"),
            Some(&info),
            "ask_approval",
        )
        .unwrap();
        let config: serde_json::Value =
            serde_json::from_str(&env(&cmd, "CODEX_CONFIG").unwrap()).unwrap();
        assert_eq!(config["model"], "selected-model");
        assert_eq!(config["model_provider"], "runjam");
        assert_eq!(
            config["model_providers"]["runjam"]["env_key"],
            "RUNJAM_PROXY_TOKEN"
        );
        assert_eq!(config["sandbox_workspace_write"]["network_access"], true);
        assert_eq!(config["approval_policy"], "on-request");
        assert_eq!(env(&cmd, "MODEL_PROVIDER").as_deref(), Some("runjam"));
        assert!(cmd.get_args().next().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
