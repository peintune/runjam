//! Skill system: discover built-in skills from app resources and copy them
//! into each agent session's working directory before the agent starts.
//!
//! Each agent (Claude, Codex, Gemini) discovers skills from a per-agent
//! directory under the session cwd:
//!   - Claude:  {cwd}/.claude/skills/{name}/SKILL.md
//!   - Codex:   {cwd}/.codex/skills/{name}/SKILL.md
//!   - Gemini:  {cwd}/.gemini/skills/{name}/SKILL.md
//!
//! RunJam ships a `builtin-skills/` directory in app resources. On session
//! start, the selected skills are copied from there into the matching
//! per-agent directory so the agent picks them up natively — no ACP protocol
//! changes needed.

use crate::rjlog;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// Parsed YAML frontmatter from a SKILL.md file. Only `name` and
/// `description` are loaded into the agent's context at all times; the rest
/// of the SKILL.md body is read only when the agent decides to use the skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
}

/// A discovered skill with its source path.
#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Absolute path to the skill directory (e.g. .../builtin-skills/pdf/).
    #[serde(skip_serializing)]
    pub source_dir: PathBuf,
}

/// Resolve the built-in skills directory from app resources.
///
/// Dev mode: `src-tauri/builtin-skills/` (relative to CARGO_MANIFEST_DIR).
/// Packaged: `{resource_dir}/builtin-skills/`.
pub fn builtin_skills_dir(app: &AppHandle) -> Option<PathBuf> {
    // Packaged app: resources/builtin-skills/
    if let Ok(resource_dir) = app.path().resource_dir() {
        let packaged = resource_dir.join("builtin-skills");
        if packaged.is_dir() {
            return Some(packaged);
        }
    }

    // Dev mode: src-tauri/builtin-skills/ (next to Cargo.toml)
    #[cfg(debug_assertions)]
    {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("builtin-skills");
        if dev.is_dir() {
            return Some(dev);
        }
    }

    None
}

/// Parse the YAML frontmatter from a SKILL.md file.
/// Returns None if the file is missing or has no valid frontmatter.
fn parse_skill_frontmatter(skill_md: &Path) -> Option<SkillMeta> {
    let content = std::fs::read_to_string(skill_md).ok()?;
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    // Find the closing ---
    let rest = &content[3..];
    let end = rest.find("\n---")?;
    let yaml = &rest[..end];
    // Minimal YAML parser: extract `name:` and `description:` fields.
    // We don't pull in a full YAML crate for two fields — keep deps lean.
    let mut name = String::new();
    let mut description = String::new();
    let mut in_description = false;
    for line in yaml.lines() {
        let trimmed = line.trim_end();
        if let Some(val) = trimmed.strip_prefix("name:") {
            name = val.trim().trim_matches('"').trim_matches('\'').to_string();
            in_description = false;
        } else if let Some(val) = trimmed.strip_prefix("description:") {
            description = val.trim().trim_matches('"').trim_matches('\'').to_string();
            in_description = true;
        } else if in_description {
            // Multi-line description continuation (lines after "description:")
            let cont = trimmed.trim();
            if cont.is_empty() {
                in_description = false;
            } else {
                description.push(' ');
                description.push_str(cont);
            }
        }
    }
    if name.is_empty() {
        return None;
    }
    Some(SkillMeta { name, description })
}

/// Scan the builtin-skills directory and return metadata for every skill
/// that has a valid SKILL.md with YAML frontmatter.
pub fn list_builtin_skills(app: &AppHandle) -> Vec<Skill> {
    let Some(base) = builtin_skills_dir(app) else {
        rjlog!("[SKILL] builtin-skills directory not found");
        return Vec::new();
    };
    let mut skills = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            if let Some(meta) = parse_skill_frontmatter(&skill_md) {
                skills.push(Skill {
                    name: meta.name,
                    description: meta.description,
                    source_dir: path,
                });
            }
        }
    }
    rjlog!("[SKILL] Discovered {} builtin skills in {:?}", skills.len(), base);
    skills
}

/// The per-agent skills directory inside a session's working directory.
/// Agents look here to discover skills natively:
///   - Claude reads `.claude/skills/`
///   - Codex reads `.codex/skills/`
///   - Gemini reads `.gemini/skills/`
fn agent_skills_subdir(agent_type: &str) -> Option<PathBuf> {
    let dir_name = match agent_type {
        "claude" | "claude-code" => ".claude",
        "codex" | "codex-cli" => ".codex",
        "gemini" | "gemini-cli" => ".gemini",
        _ => return None,
    };
    Some(PathBuf::from(dir_name).join("skills"))
}

/// Copy the named skills from the builtin-skills resource directory into
/// `{cwd}/.claude/skills/` (or `.codex`/`.gemini` for other agents).
///
/// - `cwd`: the session's working directory
/// - `agent_type`: "claude" | "codex" | "gemini"
/// - `skill_names`: skill names to copy; empty = deploy nothing (user opted out)
///
/// Existing skill directories are overwritten so updates take effect on the
/// next session. This is called BEFORE the agent process starts.
pub fn deploy_skills_to_session(
    app: &AppHandle,
    cwd: &str,
    agent_type: &str,
    skill_names: &[String],
) -> Result<usize, String> {
    // Fast path: user selected nothing → do not deploy any skills.
    if skill_names.is_empty() {
        rjlog!("[SKILL] No skills selected, skipping deployment for {}", agent_type);
        return Ok(0);
    }

    let Some(skills_subdir) = agent_skills_subdir(agent_type) else {
        rjlog!("[SKILL] Agent {} has no skills directory, skipping", agent_type);
        return Ok(0);
    };
    let dest_dir = Path::new(cwd).join(&skills_subdir);
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("Failed to create skills dir {}: {}", dest_dir.display(), e))?;

    let all_skills = list_builtin_skills(app);
    let to_deploy: Vec<&Skill> = all_skills
        .iter()
        .filter(|s| skill_names.iter().any(|n| n == &s.name))
        .collect();

    let mut count = 0;
    for skill in &to_deploy {
        let dest = dest_dir.join(&skill.name);
        // Remove old copy so updates take effect cleanly.
        if dest.exists() {
            std::fs::remove_dir_all(&dest).ok();
        }
        if let Err(e) = copy_dir_recursive(&skill.source_dir, &dest) {
            rjlog!(
                "[SKILL] Failed to copy {} → {}: {}",
                skill.name,
                dest.display(),
                e
            );
            continue;
        }
        count += 1;
        rjlog!(
            "[SKILL] Deployed skill '{}' to {}",
            skill.name,
            dest.display()
        );
    }
    rjlog!(
        "[SKILL] Deployed {}/{} skills to {} for agent {}",
        count,
        to_deploy.len(),
        dest_dir.display(),
        agent_type
    );
    Ok(count)
}

/// List the skill names already deployed in a session's per-agent skills
/// directory (e.g. `{cwd}/.claude/skills/*/`).
///
/// Returns a list of skill directory names that contain a SKILL.md.
pub fn list_session_skills(cwd: &str, agent_type: &str) -> Vec<String> {
    let Some(subdir) = agent_skills_subdir(agent_type) else {
        rjlog!("[SKILL] list_session_skills: unknown agent_type '{}', cwd='{}'", agent_type, cwd);
        return Vec::new();
    };
    let skills_dir = Path::new(cwd).join(&subdir);
    rjlog!("[SKILL] list_session_skills: reading {:?} (agent={}, cwd={})", skills_dir, agent_type, cwd);
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("SKILL.md").exists() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

/// Deploy a single builtin skill to a session's per-agent skills directory.
/// Returns the skill name on success.
pub fn deploy_single_skill(
    app: &AppHandle,
    cwd: &str,
    agent_type: &str,
    skill_name: &str,
) -> Result<String, String> {
    let Some(subdir) = agent_skills_subdir(agent_type) else {
        return Err(format!("Agent {} has no skills directory", agent_type));
    };
    let dest_dir = Path::new(cwd).join(&subdir);
    std::fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("Failed to create skills dir: {}", e))?;

    // Find the skill in builtin-skills.
    let all_skills = list_builtin_skills(app);
    let skill = all_skills
        .iter()
        .find(|s| s.name == skill_name)
        .ok_or_else(|| format!("Skill '{}' not found in builtin-skills", skill_name))?;

    let dest = dest_dir.join(skill_name);
    if dest.exists() {
        std::fs::remove_dir_all(&dest).ok();
    }
    copy_dir_recursive(&skill.source_dir, &dest)
        .map_err(|e| format!("Failed to copy skill '{}': {}", skill_name, e))?;

    rjlog!(
        "[SKILL] Deployed single skill '{}' to {}",
        skill_name,
        dest.display()
    );
    Ok(skill_name.to_string())
}

/// Remove a single skill from a session's per-agent skills directory.
pub fn remove_single_skill(
    cwd: &str,
    agent_type: &str,
    skill_name: &str,
) -> Result<(), String> {
    let Some(subdir) = agent_skills_subdir(agent_type) else {
        return Err(format!("Agent {} has no skills directory", agent_type));
    };
    let skill_dir = Path::new(cwd).join(&subdir).join(skill_name);
    if skill_dir.exists() {
        std::fs::remove_dir_all(&skill_dir)
            .map_err(|e| format!("Failed to remove skill '{}': {}", skill_name, e))?;
        rjlog!("[SKILL] Removed skill '{}' from {}", skill_name, skill_dir.display());
    }
    Ok(())
}

/// Recursively copy a directory tree (files + subdirs).
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if ft.is_file() {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_basic() {
        let tmp = std::env::temp_dir().join("test_skill.md");
        std::fs::write(
            &tmp,
            "---\nname: my-skill\ndescription: \"A test skill\"\n---\n# Body\n",
        )
        .unwrap();
        let meta = parse_skill_frontmatter(&tmp).unwrap();
        assert_eq!(meta.name, "my-skill");
        assert_eq!(meta.description, "A test skill");
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_parse_frontmatter_multiline_desc() {
        let tmp = std::env::temp_dir().join("test_skill2.md");
        std::fs::write(
            &tmp,
            "---\nname: multi\ndescription: \"Line one\n  line two\"\n---\n# Body\n",
        )
        .unwrap();
        let meta = parse_skill_frontmatter(&tmp).unwrap();
        assert_eq!(meta.name, "multi");
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let tmp = std::env::temp_dir().join("test_skill3.md");
        std::fs::write(&tmp, "# Just markdown\nNo frontmatter").unwrap();
        assert!(parse_skill_frontmatter(&tmp).is_none());
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_agent_skills_subdir() {
        assert_eq!(
            agent_skills_subdir("claude").unwrap(),
            PathBuf::from(".claude/skills")
        );
        assert_eq!(
            agent_skills_subdir("codex").unwrap(),
            PathBuf::from(".codex/skills")
        );
        assert_eq!(
            agent_skills_subdir("gemini").unwrap(),
            PathBuf::from(".gemini/skills")
        );
        assert!(agent_skills_subdir("unknown").is_none());
    }
}
