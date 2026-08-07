use crate::models::agent::Agent;
use std::process::Command;
use std::env;
use std::path::PathBuf;
use crate::util::hidden_command;

pub(crate) fn get_enhanced_path() -> String {
    let mut paths = Vec::new();

    if let Some(home) = home_dir() {
        paths.push(home.join(".nvm").join("versions").join("node"));
        paths.push(home.join(".local").join("nodejs"));
        paths.push(home.join(".local").join("bin"));
        paths.push(home.join(".npm-global").join("bin"));
        paths.push(home.join(".yarn").join("bin"));
        paths.push(home.join(".cargo").join("bin"));
    }

    paths.push(PathBuf::from("/opt/homebrew/bin"));
    paths.push(PathBuf::from("/usr/local/bin"));
    paths.push(PathBuf::from("/usr/bin"));
    paths.push(PathBuf::from("/bin"));

    let mut enhanced = String::new();

    for base in paths {
        if base.exists() {
            // Always include the base dir itself — catches Homebrew (/opt/homebrew/bin),
            // /usr/local/bin, npm global dirs, cargo/bin, etc.
            if !enhanced.is_empty() {
                enhanced.push(':');
            }
            enhanced.push_str(base.to_string_lossy().as_ref());

            // Additionally scan for nvm-style versioned Node.js installs
            // (e.g. ~/.nvm/versions/node/v22.12.0/bin)
            if let Ok(entries) = std::fs::read_dir(&base) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                            if dir_name.starts_with("v") || dir_name.starts_with("node-") {
                                    let bin_path = path.join("bin");
                                    if bin_path.exists() {
                                        if !enhanced.is_empty() {
                                            enhanced.push(':');
                                        }
                                        enhanced.push_str(bin_path.to_string_lossy().as_ref());
                                    }
                                }
                        }
                    }
                }
            }
        }
    }

    if let Some(existing) = env::var_os("PATH") {
        if !enhanced.is_empty() {
            enhanced.push(':');
        }
        enhanced.push_str(existing.to_string_lossy().as_ref());
    }

    enhanced
}

fn home_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// Read version from a package's package.json instead of running --version.
fn version_from_package(bin_dir: &std::path::Path, scope: &str, pkg: &str) -> Option<String> {
    // npm global layout: <node_dir>/bin/claude (Unix) or <node_dir>/claude.exe (Windows)
    // with <node_dir>/node_modules/<scope>/<pkg>/package.json
    // Try bin_dir first, then its parent (covers both layouts)
    for dir in [bin_dir, &bin_dir.parent().unwrap_or(bin_dir)] {
        let pkg_json = dir.join("node_modules").join(scope).join(pkg).join("package.json");
        if let Ok(content) = std::fs::read_to_string(&pkg_json) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(ver) = json.get("version").and_then(|v| v.as_str()) {
                    return Some(format!("v{}", ver));
                }
            }
        }
    }
    None
}

/// On Windows, standalone .exe binaries for claude/codex statically link
/// ConPTY APIs (ClosePseudoConsole/ResizePseudoConsole) which only exist on
/// Windows 10 1809+. Running --version triggers a system dialog on older
/// Windows. Fall back to reading version from package.json instead.
fn safe_get_version(agent_id: &str, bin_path: &str, bin_dir: &std::path::Path, path_env: &str) -> Option<String> {
    if cfg!(target_os = "windows") {
        match agent_id {
            "claude-code" => {
                if let Some(v) = version_from_package(bin_dir, "@anthropic-ai", "claude-code") {
                    return Some(v);
                }
            }
            "codex-cli" => {
                if let Some(v) = version_from_package(bin_dir, "@openai", "codex") {
                    return Some(v);
                }
            }
            _ => {}
        }
    }
    get_version(bin_path, path_env)
}

/// Scan PATH for installed AI coding agents.
pub fn detect_agents() -> Vec<Agent> {
    let mut agents = Agent::builtin_agents();
    let enhanced_path = get_enhanced_path();

    for agent in agents.iter_mut() {
        let bin_name = match agent.id.as_str() {
            "claude-code" => "claude",
            "codex-cli" => "codex",
            "gemini-cli" => "gemini",
            _ => continue,
        };
        let agent_id = agent.id.clone();

        // Also check RunJam's bundled Node.js global bin dir and common npm dirs
        let mut extra_paths: Vec<std::path::PathBuf> = vec![
            // RunJam auto-downloaded Node.js
            dirs_data_dir().join("nodejs").join("node-v22.12.0").join("bin"),
            // Common npm global dirs
            home_dir().unwrap_or_default().join(".npm-global").join("bin"),
            // ~/.local/bin (common user-local install location)
            home_dir().unwrap_or_default().join(".local").join("bin"),
        ];
        // Scan ~/.local/nodejs/*/bin for versioned standalone Node.js installs
        if let Some(home) = home_dir() {
            let local_nodejs = home.join(".local").join("nodejs");
            if let Ok(entries) = std::fs::read_dir(&local_nodejs) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let bin_path = path.join("bin");
                        if bin_path.exists() {
                            extra_paths.push(bin_path);
                        }
                    }
                }
            }
        }
        let mut found = false;

        // Check extra paths
        for dir in &extra_paths {
            let bin_path = dir.join(bin_name);
            if bin_path.exists() {
                let version = safe_get_version(
                    &agent_id,
                    &bin_path.to_string_lossy(),
                    dir,
                    &enhanced_path,
                );
                agent.install_path = Some(bin_path.to_string_lossy().to_string());
                agent.version = version;
                agent.installed = true;
                found = true;
                break;
            }
        }

        if found { continue; }

        // Fallback to system PATH (using enhanced_path that includes Homebrew, nvm, etc.)
        let which = if cfg!(target_os = "windows") {
            hidden_command("where").arg(bin_name).output()
        } else {
            Command::new("which").arg(bin_name).env("PATH", &enhanced_path).output()
        };

        if let Ok(output) = which {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .map(|s| s.trim().to_string());

                let version = if let Some(ref p) = path {
                    let dir = std::path::Path::new(p).parent().unwrap_or_else(|| std::path::Path::new("")).to_path_buf();
                    safe_get_version(&agent_id, p, &dir, &enhanced_path)
                } else {
                    None
                };

                agent.install_path = path;
                agent.version = version;
                agent.installed = true;
            }
        }
    }

    agents
}

fn dirs_data_dir() -> std::path::PathBuf {
    if let Some(dir) = directories::ProjectDirs::from("com", "runjam", "RunJam") {
        dir.data_local_dir().to_path_buf()
    } else {
        std::path::PathBuf::from(".")
    }
}

pub fn get_version(bin: &str, path: &str) -> Option<String> {
    let output = hidden_command(bin)
        .arg("--version")
        .env("PATH", path)
        .output()
        .ok()?;
    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_string();
        Some(version)
    } else {
        None
    }
}