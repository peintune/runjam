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

/// List the skill names already deployed in a session's per-agent skills
/// directory (e.g. `{cwd}/.claude/skills/`).
#[tauri::command]
pub fn list_session_skills(cwd: String, cli: String) -> Vec<String> {
    skill::list_session_skills(&cwd, &cli)
}

/// Deploy a single builtin skill to a session's per-agent skills directory.
/// Called when the user toggles a skill on in an active session.
#[tauri::command]
pub fn deploy_session_skill(app: AppHandle, cwd: String, cli: String, skill_name: String) -> Result<String, String> {
    skill::deploy_single_skill(&app, &cwd, &cli, &skill_name)
}

/// Remove a single skill from a session's per-agent skills directory.
/// Called when the user toggles a skill off in an active session.
#[tauri::command]
pub fn remove_session_skill(cwd: String, cli: String, skill_name: String) -> Result<(), String> {
    skill::remove_single_skill(&cwd, &cli, &skill_name)
}
