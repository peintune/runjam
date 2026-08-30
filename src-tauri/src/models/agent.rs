use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub display_name: String,
    pub install_path: Option<String>,
    pub version: Option<String>,
    pub installed: bool,
    pub status: String,
    pub last_tested_at: Option<String>,
}

impl Agent {
    /// Canonical display name for a built-in agent id. Unknown ids fall back to
    /// the id itself so the UI always has something to render.
    pub fn display_name_for(id: &str) -> String {
        Self::builtin_agents()
            .into_iter()
            .find(|a| a.id == id)
            .map(|a| a.display_name)
            .unwrap_or_else(|| id.to_string())
    }

    /// Never let an empty `display_name` through: the UI renders it verbatim
    /// (new-session agent picker, session titles), and once an empty value is
    /// persisted it keeps being served from the agents cache.
    pub fn resolve_display_name(id: &str, display_name: &str) -> String {
        if display_name.trim().is_empty() {
            Self::display_name_for(id)
        } else {
            display_name.to_string()
        }
    }

    pub fn builtin_agents() -> Vec<Agent> {
        vec![
            Agent {
                id: "claude-code".into(),
                display_name: "Claude Code".into(),
                install_path: None,
                version: None,
                installed: false,
                status: "not_installed".into(),
                last_tested_at: None,
            },
            Agent {
                id: "codex-cli".into(),
                display_name: "Codex CLI".into(),
                install_path: None,
                version: None,
                installed: false,
                status: "not_installed".into(),
                last_tested_at: None,
            },
            Agent {
                id: "gemini-cli".into(),
                display_name: "Gemini CLI".into(),
                install_path: None,
                version: None,
                installed: false,
                status: "not_installed".into(),
                last_tested_at: None,
            },
        ]
    }
}
