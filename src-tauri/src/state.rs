use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub enabled: bool,
}

impl Default for AgentState {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppState {
    pub agents: HashMap<String, AgentState>,
}

impl AppState {
    fn state_path() -> PathBuf {
        let base = directories::ProjectDirs::from("com", "runjam", "RunJam")
            .map(|d| d.data_local_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        std::fs::create_dir_all(&base).ok();
        base.join("agent-state.json")
    }

    pub fn load() -> Self {
        let path = Self::state_path();
        if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) {
        let path = Self::state_path();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            std::fs::write(path, json).ok();
        }
    }

    pub fn get_agent(&self, id: &str) -> AgentState {
        self.agents.get(id).cloned().unwrap_or_default()
    }

    pub fn set_agent(&mut self, id: &str, state: AgentState) {
        self.agents.insert(id.to_string(), state);
        self.save();
    }
}
