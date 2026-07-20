use crate::models::agent::Agent;
use std::process::Command;
use std::env;
use std::path::PathBuf;

pub(crate) fn get_enhanced_path() -> String {
    let mut paths = Vec::new();

    if let Some(home) = home_dir() {
        paths.push(home.join(".nvm").join("versions").join("node"));
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
                            if dir_name.starts_with("v") {
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

        // Also check RunJam's bundled Node.js global bin dir and common npm dirs
        let extra_paths: Vec<std::path::PathBuf> = vec![
            // RunJam auto-downloaded Node.js
            dirs_data_dir().join("nodejs").join("node-v22.12.0").join("bin"),
            // Common npm global dirs
            home_dir().unwrap_or_default().join(".npm-global").join("bin"),
        ];
        let mut found = false;

        // Check extra paths
        for dir in &extra_paths {
            let bin_path = dir.join(bin_name);
            if bin_path.exists() {
                let version = get_version(&bin_path.to_string_lossy(), &enhanced_path);
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
            Command::new("where").arg(bin_name).output()
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
                    get_version(p, &enhanced_path)
                } else {
                    get_version(bin_name, &enhanced_path)
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

fn get_version(bin: &str, path: &str) -> Option<String> {
    let output = Command::new(bin)
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
