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
