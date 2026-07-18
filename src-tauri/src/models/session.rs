use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub cli: String,
    pub cli_display_name: String,
    pub directory: Option<String>,
    pub pid: Option<u32>,
    pub status: String,
    pub created_at: String,
}
