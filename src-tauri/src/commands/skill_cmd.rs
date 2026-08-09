//! Tauri commands for the skill system.
//!
//! Skills are discovered from the built-in `builtin-skills/` directory shipped
//! in app resources (and, in the future, from `~/.runjam/skills/`). The frontend
//! lists them via `list_skills` so users can pick which skills to enable for a
//! session; the selected skills are then copied into the session's working
//! directory before the agent starts (see `skill::deploy_skills_to_session`).

use crate::skill::{self, Skill};
use tauri::AppHandle;

/// List all built-in skills available to RunJam.
/// Returns each skill's name, description, and whether it's enabled by default.
#[tauri::command]
pub fn list_skills(app: AppHandle) -> Vec<Skill> {
    skill::list_builtin_skills(&app)
}
