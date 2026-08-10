//! Tauri commands for the skill system.
//!
//! Skills are discovered from the built-in `builtin-skills/` directory shipped
//! in app resources (and, in the future, from `~/.runjam/skills/`). The frontend
//! lists them via `list_skills` so users can pick which skills to enable for a
//! session; the selected skills are then copied into the session's working
//! directory before the agent starts (see `skill::deploy_skills_to_session`).

use crate::skill::{self, Skill};
use tauri::AppHandle;

/// Skills that are auto-injected into every session by the backend
/// (see `session_cmd::start_session`) but hidden from the UI skill picker.
/// Users cannot toggle these on/off — they are always present.
const HIDDEN_SKILLS: &[&str] = &["runjam-defaults"];

/// List all built-in skills available to RunJam, excluding hidden skills
/// that are auto-injected (e.g. `runjam-defaults`). The frontend uses this
/// to populate the skill picker grid.
#[tauri::command]
pub fn list_skills(app: AppHandle) -> Vec<Skill> {
    skill::list_builtin_skills(&app)
        .into_iter()
        .filter(|s| !HIDDEN_SKILLS.contains(&s.name.as_str()))
        .collect()
}

/// List the skill names already deployed in a session's per-agent skills
/// directory (e.g. `{cwd}/.claude/skills/`), excluding hidden skills so
/// they don't show as toggleable tags in the UI.
#[tauri::command]
pub fn list_session_skills(cwd: String, cli: String) -> Vec<String> {
    skill::list_session_skills(&cwd, &cli)
        .into_iter()
        .filter(|name| !HIDDEN_SKILLS.contains(&name.as_str()))
        .collect()
}

/// Deploy a single builtin skill to a session's per-agent skills directory.
/// Called when the user toggles a skill on in an active session.
#[tauri::command]
pub fn deploy_session_skill(app: AppHandle, cwd: String, cli: String, skill_name: String) -> Result<String, String> {
    skill::deploy_single_skill(&app, &cwd, &cli, &skill_name)
}

/// Remove a single skill from a session's per-agent skills directory.
/// Called when the user toggles a skill off in an active session.
/// Hidden skills (e.g. `runjam-defaults`) cannot be removed — they are
/// always present in every session.
#[tauri::command]
pub fn remove_session_skill(cwd: String, cli: String, skill_name: String) -> Result<(), String> {
    if HIDDEN_SKILLS.contains(&skill_name.as_str()) {
        return Err(format!(
            "Skill '{}' is a system skill and cannot be removed.",
            skill_name
        ));
    }
    skill::remove_single_skill(&cwd, &cli, &skill_name)
}
