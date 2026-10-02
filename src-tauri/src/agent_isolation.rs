//! Isolated config directories for the agent CLIs.
//!
//! RunJam drives `claude`, `codex` and `gemini`, and each of them reads its
//! configuration from the user's home directory (`~/.claude`, `~/.codex`,
//! `~/.gemini`). RunJam used to WRITE its model/proxy settings straight into those
//! files, which destroyed the user's own configuration — and, worse, their stored
//! credentials. A real incident: `~/.codex/auth.json` was overwritten without a
//! backup, so the user's Codex login was lost and their standalone `codex` stopped
//! working entirely (it had been pointed at RunJam's local proxy).
//!
//! The fix is to stop touching those files at all. Each CLI supports an
//! environment variable that relocates its config directory, verified by running
//! each one with the variable set:
//!
//!   * codex  — `CODEX_HOME`        (reported by `codex doctor` as the config root)
//!   * claude — `CLAUDE_CONFIG_DIR` (used internally by `claude-agent-acp`)
//!   * gemini — `GEMINI_CLI_HOME`   (gemini creates `<dir>/.gemini` under it)
//!
//! RunJam points these at its own directory, so an agent run from RunJam sees
//! only RunJam's configuration and the user's own setup is never read or written.
//!
//! TRADE-OFF, accepted deliberately: isolating the directory also hides the user's
//! own agent extensions (their MCP servers, skills, plugins) from sessions started
//! inside RunJam. That is the price of never being able to damage the user's
//! setup; RunJam's own MCP support (see `commands::mcp_cmd`) is the replacement.

use std::path::PathBuf;

/// Where RunJam keeps each agent's isolated config directory.
///
/// Lives under the RunJam data dir so it is uninstalled together with the app and
/// never mixes with user-owned files.
///
/// Honours `RUNJAM_HOME` when set. That variable already relocates the "user home"
/// RunJam reads (`get_home_dir`), and it exists for two reasons: tests must never
/// touch a real `~/.claude`, and users get an escape hatch to point RunJam at a
/// different CLI setup. The isolation root must follow it, or a test would write
/// into the real application-data directory (and two concurrent tests would fight
/// over the same files).
pub fn isolated_config_root() -> PathBuf {
    if let Ok(dir) = std::env::var("RUNJAM_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("agent-config");
        }
    }
    crate::node_util::get_runjam_data_dir().join("agent-config")
}

/// The isolated config directory for one agent.
///
/// The layout mirrors what each CLI expects:
///
/// * codex  → `<root>/codex` (`CODEX_HOME`; holds `config.toml`)
/// * claude → `<root>/claude` (`CLAUDE_CONFIG_DIR`; holds `settings.json`)
/// * gemini → `<root>/gemini` with a nested `.gemini` (`GEMINI_CLI_HOME`;
///   gemini creates `<GEMINI_CLI_HOME>/.gemini/settings.json`)
///
/// `agent_id` accepts the RunJam ids (`codex-cli`, `claude-code`, `gemini-cli`)
/// and the short CLI names used on the spawn path (`codex`, `claude`, `gemini`).
pub fn isolated_config_dir(agent_id: &str) -> Option<PathBuf> {
    let root = isolated_config_root();
    match agent_id {
        "codex" | "codex-cli" => Some(root.join("codex")),
        "claude" | "claude-code" => Some(root.join("claude")),
        "gemini" | "gemini-cli" => Some(root.join("gemini")),
        _ => None,
    }
}

/// The environment variable that relocates an agent's config directory.
pub fn config_dir_env_var(agent_id: &str) -> Option<&'static str> {
    match agent_id {
        "codex" | "codex-cli" => Some("CODEX_HOME"),
        "claude" | "claude-code" => Some("CLAUDE_CONFIG_DIR"),
        "gemini" | "gemini-cli" => Some("GEMINI_CLI_HOME"),
        _ => None,
    }
}

/// The (env var, value) pair that isolates one agent's config.
///
/// `None` for an unknown agent, so callers can simply do nothing rather than
/// setting a variable that means nothing. The directory is created as a side
/// effect: the CLIs expect it to exist, and creating it here keeps callers from
/// each having to remember.
pub fn isolated_config_env(agent_id: &str) -> Option<(&'static str, String)> {
    let dir = isolated_config_dir(agent_id)?;
    std::fs::create_dir_all(&dir).ok();
    let var = config_dir_env_var(agent_id)?;
    Some((var, dir.to_string_lossy().to_string()))
}

/// Path of the config FILE RunJam writes for one agent, inside the isolation dir.
///
/// This is the file RunJam owns outright now — there is no user file to merge
/// with, so writes are plain overwrites of RunJam's own data.
///
/// For gemini the result is `<root>/gemini/.gemini/settings.json`: gemini treats
/// `GEMINI_CLI_HOME` as a HOME-like root and looks for `.gemini` inside it, so the
/// extra level is required for the file to be found.
pub fn isolated_config_file(agent_id: &str) -> Option<PathBuf> {
    let dir = isolated_config_dir(agent_id)?;
    match agent_id {
        "codex" | "codex-cli" => Some(dir.join("config.toml")),
        "claude" | "claude-code" => Some(dir.join("settings.json")),
        "gemini" | "gemini-cli" => Some(dir.join(".gemini").join("settings.json")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_agent_gets_a_distinct_directory() {
        let c = isolated_config_dir("codex-cli").unwrap();
        let l = isolated_config_dir("claude-code").unwrap();
        let g = isolated_config_dir("gemini-cli").unwrap();
        assert_ne!(c, l);
        assert_ne!(c, g);
        assert_ne!(l, g);
    }

    #[test]
    fn short_and_long_ids_resolve_to_the_same_directory() {
        // The spawn path uses short names ("codex"); the settings/DB path uses
        // long ones ("codex-cli"). They MUST agree or a session would read a
        // different config than the one that was written.
        assert_eq!(isolated_config_dir("codex"), isolated_config_dir("codex-cli"));
        assert_eq!(isolated_config_dir("claude"), isolated_config_dir("claude-code"));
        assert_eq!(isolated_config_dir("gemini"), isolated_config_dir("gemini-cli"));
    }

    #[test]
    fn the_directory_is_inside_the_runjam_data_dir() {
        // Never anywhere near the user's home config, which is the entire point.
        let dir = isolated_config_dir("codex-cli").unwrap();
        assert!(dir.starts_with(isolated_config_root()), "got {dir:?}");
        assert!(!dir.to_string_lossy().contains("/.codex"));
        assert!(!dir.to_string_lossy().contains("/.claude"));
        assert!(!dir.to_string_lossy().contains("/.gemini"));
    }

    #[test]
    fn env_var_names_match_what_each_cli_documents() {
        assert_eq!(config_dir_env_var("codex-cli"), Some("CODEX_HOME"));
        assert_eq!(config_dir_env_var("claude-code"), Some("CLAUDE_CONFIG_DIR"));
        assert_eq!(config_dir_env_var("gemini-cli"), Some("GEMINI_CLI_HOME"));
        assert_eq!(config_dir_env_var("nope"), None);
    }

    #[test]
    fn gemini_config_file_sits_one_level_deeper() {
        // GEMINI_CLI_HOME is a HOME-like root: gemini looks for `.gemini` inside.
        // Getting this wrong means the settings file is written but never read.
        let f = isolated_config_file("gemini-cli").unwrap();
        assert!(f.ends_with(".gemini/settings.json"), "got {f:?}");
        let c = isolated_config_file("codex-cli").unwrap();
        assert!(c.ends_with("config.toml"), "got {c:?}");
        let l = isolated_config_file("claude-code").unwrap();
        assert!(l.ends_with("settings.json"), "got {l:?}");
    }

    #[test]
    fn unknown_agents_are_reported_rather_than_guessed() {
        assert!(isolated_config_dir("gpt-5").is_none());
        assert!(config_dir_env_var("gpt-5").is_none());
        assert!(isolated_config_file("gpt-5").is_none());
    }
}