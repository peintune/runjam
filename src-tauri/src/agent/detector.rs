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

/// Candidate file names of an agent binary inside a directory.
///
/// On Windows npm writes a shim `<name>`, a `<name>.cmd`, a `<name>.ps1` and
/// (for some packages) a standalone `<name>.exe`. `.cmd` is preferred: the
/// standalone `.exe` bundles a Node runtime that statically links ConPTY APIs
/// (ClosePseudoConsole, Win10 1809+) and fails to load on older Windows.
fn bin_candidates(bin_name: &str) -> Vec<String> {
    if cfg!(target_os = "windows") {
        vec![
            format!("{}.cmd", bin_name),
            format!("{}.exe", bin_name),
            bin_name.to_string(),
        ]
    } else {
        vec![bin_name.to_string()]
    }
}

fn find_bin_in(dir: &std::path::Path, bin_name: &str) -> Option<PathBuf> {
    bin_candidates(bin_name)
        .iter()
        .map(|c| dir.join(c))
        .find(|p| p.exists())
}

/// Node.js shipped inside the app bundle (Tauri resource dir). Detection runs
/// without an AppHandle, so resolve it relative to the executable, which is
/// where Tauri places resources (Windows: next to the .exe, macOS: ../Resources).
fn bundled_node_dirs() -> Vec<PathBuf> {
    let exe = match std::env::current_exe().ok() {
        Some(e) => e,
        None => return Vec::new(),
    };
    let exe_dir = match exe.parent() {
        Some(d) => d.to_path_buf(),
        None => return Vec::new(),
    };
    if cfg!(target_os = "windows") {
        vec![exe_dir.join("nodejs")]
    } else if cfg!(target_os = "macos") {
        vec![exe_dir.join("..").join("Resources").join("nodejs").join("bin")]
    } else {
        vec![
            exe_dir.join("nodejs").join("bin"),
            exe_dir
                .join("..")
                .join("lib")
                .join("runjam")
                .join("nodejs")
                .join("bin"),
        ]
    }
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
        let data_node_dir = dirs_data_dir().join("nodejs").join("node-v22.12.0");
        let mut extra_paths: Vec<std::path::PathBuf> = vec![
            // Common npm global dirs
            home_dir().unwrap_or_default().join(".npm-global").join("bin"),
            // ~/.local/bin (common user-local install location)
            home_dir().unwrap_or_default().join(".local").join("bin"),
        ];
        // RunJam auto-downloaded Node.js. The Windows Node build keeps binaries
        // next to node.exe (no `bin/` subdir), so probe both layouts.
        extra_paths.push(data_node_dir.join("bin"));
        extra_paths.push(data_node_dir);
        extra_paths.extend(bundled_node_dirs());
        if cfg!(target_os = "windows") {
            // npm's default global prefix on Windows.
            if let Some(appdata) = std::env::var_os("APPDATA") {
                extra_paths.push(PathBuf::from(appdata).join("npm"));
            }
        }
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
            if let Some(bin_path) = find_bin_in(dir, bin_name) {
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

        // For claude, the pre-bundled ACP adapter (shipped in app resources)
        // is enough — ACP mode runs via node + the bundled
        // @agentclientprotocol/claude-agent-acp package, so the standalone
        // `claude` CLI isn't required.
        if agent.id == "claude-code" {
            if let Some(entry) = crate::acp_client::find_bundled_claude_acp() {
                let version = entry
                    .parent()
                    .and_then(|p| p.parent())
                    .and_then(|p| std::fs::read_to_string(p.join("package.json")).ok())
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                    .and_then(|j| {
                        j.get("version")
                            .and_then(|v| v.as_str())
                            .map(|s| format!("v{}", s))
                    });
                agent.install_path = Some(entry.to_string_lossy().to_string());
                agent.version = version;
                agent.installed = true;
                agent.status = "installed".to_string();
                continue;
            }
        }

        // Fallback to system PATH (using enhanced_path that includes Homebrew, nvm, etc.)
        let resolved = if cfg!(target_os = "windows") {
            // `where` resolves one pattern per run, so probe the candidates in
            // preference order — otherwise it returns the extensionless npm
            // shim, which can't be executed (and yields no version) on Windows.
            bin_candidates(bin_name).iter().find_map(|c| {
                hidden_command("where").arg(c).output().ok().and_then(|o| {
                    if o.status.success() {
                        String::from_utf8_lossy(&o.stdout)
                            .lines()
                            .next()
                            .map(|s| s.trim().to_string())
                    } else {
                        None
                    }
                })
            })
        } else {
            Command::new("which")
                .arg(bin_name)
                .env("PATH", &enhanced_path)
                .output()
                .ok()
                .and_then(|o| {
                    if o.status.success() {
                        String::from_utf8_lossy(&o.stdout)
                            .lines()
                            .next()
                            .map(|s| s.trim().to_string())
                    } else {
                        None
                    }
                })
        };

        if let Some(path) = resolved {
            let dir = std::path::Path::new(&path)
                .parent()
                .unwrap_or_else(|| std::path::Path::new(""))
                .to_path_buf();
            agent.version = safe_get_version(&agent_id, &path, &dir, &enhanced_path);
            agent.install_path = Some(path);
            agent.installed = true;
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