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

/// The user-installed skills directory: `~/.runjam/skills/`.
/// Skills uploaded as .zip packages are extracted here (one folder per skill,
/// each with a SKILL.md). This is the same unified user-data location used by
/// the database, logs and sessions (see `lib.rs`).
pub fn user_skills_dir() -> PathBuf {
    let home = directories::UserDirs::new()
        .map(|d| d.home_dir().join(".runjam").join("skills"))
        .unwrap_or_else(|| PathBuf::from(".runjam").join("skills"));
    std::fs::create_dir_all(&home).ok();
    home
}

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

/// Scan a directory of skill folders (each containing SKILL.md) and return
/// metadata for every skill with valid YAML frontmatter.
fn scan_skills_dir(base: &Path) -> Vec<Skill> {
    let mut skills = Vec::new();
    if let Ok(entries) = std::fs::read_dir(base) {
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
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Scan the builtin-skills directory and return metadata for every skill
/// that has a valid SKILL.md with YAML frontmatter.
pub fn list_builtin_skills(app: &AppHandle) -> Vec<Skill> {
    let Some(base) = builtin_skills_dir(app) else {
        rjlog!("[SKILL] builtin-skills directory not found");
        return Vec::new();
    };
    let skills = scan_skills_dir(&base);
    rjlog!("[SKILL] Discovered {} builtin skills in {:?}", skills.len(), base);
    skills
}

/// Scan the user-installed skills directory (`~/.runjam/skills/`).
pub fn list_user_skills() -> Vec<Skill> {
    scan_skills_dir(&user_skills_dir())
}

/// Every skill available to RunJam: user-installed first (so a user skill with
/// the same name overrides a builtin), then builtins.
pub fn list_all_skills(app: &AppHandle) -> Vec<Skill> {
    let mut skills = list_user_skills();
    let builtin = list_builtin_skills(app);
    let mut seen = std::collections::HashSet::new();
    skills.retain(|s| seen.insert(s.name.clone()));
    for s in builtin {
        if seen.insert(s.name.clone()) {
            skills.push(s);
        }
    }
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

    let all_skills = list_all_skills(app);
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
        // runjam-defaults carries per-session placeholders (the absolute
        // working directory) that must be rendered for this session; all
        // other skills are copied verbatim.
        let copied = if skill.name == RUNJAM_DEFAULTS_SKILL {
            deploy_skill_rendered(&skill.source_dir, &dest, cwd)
        } else {
            copy_dir_recursive(&skill.source_dir, &dest)
        };
        if let Err(e) = copied {
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
    //rjlog!("[SKILL] list_session_skills: reading {:?} (agent={}, cwd={})", skills_dir, agent_type, cwd);
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

    // Find the skill among builtin + user-installed skills.
    let all_skills = list_all_skills(app);
    let skill = all_skills
        .iter()
        .find(|s| s.name == skill_name)
        .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

    let dest = dest_dir.join(skill_name);
    if dest.exists() {
        std::fs::remove_dir_all(&dest).ok();
    }
    let copied = if skill_name == RUNJAM_DEFAULTS_SKILL {
        deploy_skill_rendered(&skill.source_dir, &dest, cwd)
    } else {
        copy_dir_recursive(&skill.source_dir, &dest)
    };
    copied.map_err(|e| format!("Failed to copy skill '{}': {}", skill_name, e))?;

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

/// Install skills from a base64-encoded .zip package into `~/.runjam/skills/`.
///
/// The archive may contain one skill (SKILL.md at the zip root, or inside a
/// single wrapping folder) or several top-level skill folders. Every folder
/// with a valid SKILL.md (YAML frontmatter) is installed; a skill with the
/// same name as an existing one is overwritten. Returns the installed skills.
///
/// Security: entries are path-squashed and any absolute / `..` paths are
/// rejected (zip-slip protection).
pub fn install_skill_zip(zip_base64: &str) -> Result<Vec<Skill>, String> {
    use base64::Engine;
    use zip::ZipArchive;

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(zip_base64)
        .map_err(|e| format!("Invalid zip data: {e}"))?;
    let mut archive =
        ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(|e| format!("Invalid zip archive: {e}"))?;

    // Stage into a temp dir first, then scan for skill folders.
    let staging = std::env::temp_dir().join(format!("runjam-skill-{}", std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).ok();
    }
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;

    let extract_result = extract_zip(&mut archive, &staging);
    if let Err(e) = extract_result {
        std::fs::remove_dir_all(&staging).ok();
        return Err(e);
    }

    let dest = user_skills_dir();
    let mut installed = Vec::new();

    // Case 1: SKILL.md at the zip root → the whole package is one skill.
    if staging.join("SKILL.md").is_file() {
        try_install_one(&staging, &dest, &mut installed);
        std::fs::remove_dir_all(&staging).ok();
        if installed.is_empty() {
            return Err("The .zip does not contain a valid skill (a SKILL.md with YAML frontmatter)".into());
        }
        return Ok(installed);
    }

    // Case 2: top-level folders, each possibly a skill. Some packages wrap the
    // skill in one extra folder — check one level down as well.
    let top_entries = std::fs::read_dir(&staging).map_err(|e| e.to_string())?;
    for entry in top_entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("SKILL.md").is_file() {
            try_install_one(&path, &dest, &mut installed);
        } else if let Ok(inner) = std::fs::read_dir(&path) {
            for e2 in inner.flatten() {
                if e2.path().is_dir() && e2.path().join("SKILL.md").is_file() {
                    try_install_one(&e2.path(), &dest, &mut installed);
                }
            }
        }
    }
    std::fs::remove_dir_all(&staging).ok();

    if installed.is_empty() {
        return Err("The .zip does not contain a valid skill (a folder with SKILL.md containing YAML frontmatter)".into());
    }
    Ok(installed)
}

/// Extract every entry of a zip archive into `dest`, rejecting unsafe paths.
fn extract_zip<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    dest: &Path,
) -> Result<(), String> {
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("Zip read error: {e}"))?;
        let name = entry.name().replace('\\', "/");
        // Zip-slip protection.
        let clean = Path::new(&name);
        if clean.is_absolute() || name.split('/').any(|c| c == "..") || name.contains('\0') {
            return Err(format!("Unsafe path in zip: {name}"));
        }
        let out_path = dest.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).ok();
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut file = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Copy a staged skill folder into the user skills dir under its frontmatter
/// name (overwriting an existing skill of the same name). Returns true if a
/// valid skill was installed.
fn try_install_one(src: &Path, dest: &Path, installed: &mut Vec<Skill>) -> bool {
    let Some(meta) = parse_skill_frontmatter(&src.join("SKILL.md")) else {
        rjlog!("[SKILL] Skipping {} — no valid SKILL.md frontmatter", src.display());
        return false;
    };
    if !is_safe_skill_name(&meta.name) {
        rjlog!("[SKILL] Rejecting unsafe skill name '{}'", meta.name);
        return false;
    }
    let target = dest.join(&meta.name);
    if target.exists() {
        std::fs::remove_dir_all(&target).ok();
    }
    if let Err(e) = copy_dir_recursive(src, &target) {
        rjlog!("[SKILL] Failed to install '{}': {}", meta.name, e);
        return false;
    }
    rjlog!("[SKILL] Installed user skill '{}' → {}", meta.name, target.display());
    installed.push(Skill {
        name: meta.name,
        description: meta.description,
        source_dir: target,
    });
    true
}

/// Remove a user-installed skill (by its frontmatter name) from `~/.runjam/skills/`.
pub fn remove_user_skill(skill_name: &str) -> Result<(), String> {
    if !is_safe_skill_name(skill_name) {
        return Err(format!("Unsafe skill name: {skill_name}"));
    }
    let dir = user_skills_dir().join(skill_name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("Failed to remove skill '{skill_name}': {e}"))?;
        rjlog!("[SKILL] Removed user skill '{skill_name}'");
    }
    Ok(())
}

/// Allow only plain folder names (no separators / traversal) for skill names.
fn is_safe_skill_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains('\\') && name != "." && name != ".."
}

/// Name of the auto-injected default constraints skill. Its SKILL.md carries
/// per-session placeholders rendered at deploy time (see `render_skill_md`).
const RUNJAM_DEFAULTS_SKILL: &str = "runjam-defaults";

/// Placeholder in runjam-defaults/SKILL.md replaced with the session's
/// absolute working directory at deploy time. Gives the agent an explicit,
/// authoritative answer to "where am I" even if the ACP layer's cwd handling
/// differs between agent implementations.
const SESSION_CWD_PLACEHOLDER: &str = "{{SESSION_CWD}}";

/// Best-effort absolute path: pass through absolute inputs, otherwise resolve
/// against the current process directory.
fn absolute_path(cwd: &str) -> String {
    let p = Path::new(cwd);
    if p.is_absolute() {
        return p.to_string_lossy().to_string();
    }
    if let Ok(cd) = std::env::current_dir() {
        return cd.join(p).to_string_lossy().to_string();
    }
    cwd.to_string()
}

/// Render the session working directory into a skill file's content.
fn render_skill_md(content: &str, cwd: &str) -> String {
    content.replace(SESSION_CWD_PLACEHOLDER, &absolute_path(cwd))
}

/// Deploy a skill directory, rendering `{{SESSION_CWD}}` inside SKILL.md.
/// All other files are copied verbatim.
fn deploy_skill_rendered(src: &Path, dst: &Path, cwd: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if src_path.is_file() {
            if src_path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md") {
                let content = std::fs::read_to_string(&src_path)?;
                std::fs::write(&dst_path, render_skill_md(&content, cwd))?;
            } else {
                std::fs::copy(&src_path, &dst_path)?;
            }
        }
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
