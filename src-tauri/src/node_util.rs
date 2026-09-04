use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Manager};
use crate::util::hidden_command;

/// Get the directory containing the bundled Node.js binary (for PATH).
/// On macOS/Linux this is `{resource_dir}/nodejs/bin`,
/// on Windows it's `{resource_dir}/nodejs`.
pub fn get_bundled_node_bin_dir(app: &AppHandle) -> Option<PathBuf> {
    let resource_dir = app.path().resource_dir().ok()?;
    let node_dir = resource_dir.join("nodejs");

    let bin_dir = if cfg!(target_os = "windows") {
        node_dir.clone()
    } else {
        node_dir.join("bin")
    };

    let node_bin = if cfg!(target_os = "windows") {
        bin_dir.join("node.exe")
    } else {
        bin_dir.join("node")
    };

    if node_bin.exists() {
        Some(bin_dir)
    } else {
        None
    }
}

/// Get the full path to the bundled Node.js binary.
pub fn get_bundled_node_bin(app: &AppHandle) -> Option<PathBuf> {
    let bin_dir = get_bundled_node_bin_dir(app)?;
    let node = if cfg!(target_os = "windows") {
        bin_dir.join("node.exe")
    } else {
        bin_dir.join("node")
    };
    Some(node)
}

/// How to launch an npm invocation.
///
/// We deliberately do NOT rely on the `bin/npm` symlink shipped inside the
/// official Node.js distributions: when Tauri bundles `nodejs/` as a resource
/// it copies each entry with `fs::copy`, which follows symlinks, so inside the
/// final `.app`/installer `bin/npm` ends up as a plain (non-executable) text
/// file holding the contents of `npm-cli.js`. Checking for a live symlink
/// would therefore make the bundled npm unusable on every clean machine.
///
/// Instead we always invoke npm through the (real, executable) `node` binary
/// as `node <npm-cli.js> <args>`, which only requires two regular files.
#[derive(Debug, Clone)]
pub enum NpmRunner {
    /// Run `node <npm-cli.js> <args>`.
    NodeCli { node: PathBuf, npm_cli: PathBuf },
    /// Run a directly executable npm entry point
    /// (e.g. Windows `npm.cmd`, or `npm` on the system PATH).
    Direct(PathBuf),
}

impl NpmRunner {
    /// Build a [`Command`] pre-filled with the program and the npm entry
    /// argument (if any). Callers can append npm sub-command arguments,
    /// working dir and env.
    pub fn command(&self) -> Command {
        match self {
            NpmRunner::NodeCli { node, npm_cli } => {
                let mut cmd = hidden_command(node);
                cmd.arg(npm_cli);
                cmd
            }
            NpmRunner::Direct(program) => hidden_command(program),
        }
    }

    /// Human-readable form, for log/status messages.
    pub fn display(&self) -> String {
        match self {
            NpmRunner::NodeCli { node, npm_cli } => {
                format!("{} {}", node.display(), npm_cli.display())
            }
            NpmRunner::Direct(program) => program.display().to_string(),
        }
    }
}

/// Locate `node` and the npm CLI inside a Node.js distribution root.
///
/// The root is the directory that contains `bin/` (macOS/Linux tarballs) or
/// `node.exe` + `node_modules/` (Windows zips). Returns `None` when neither
/// `npm-cli.js` nor a direct npm entry point can be found.
pub fn npm_runner_at_install_root(root: &Path) -> Option<NpmRunner> {
    let node = if cfg!(target_os = "windows") {
        root.join("node.exe")
    } else {
        root.join("bin").join("node")
    };
    if !node.is_file() {
        return None;
    }

    // npm-cli.js lives at `lib/node_modules/npm/bin/` in macOS/Linux tarballs
    // and at `node_modules/npm/bin/` in Windows zips. Try both layouts so a
    // copied/bundled installation also works regardless of layout.
    let npm_cli_candidates = [
        root.join("lib").join("node_modules").join("npm").join("bin").join("npm-cli.js"),
        root.join("node_modules").join("npm").join("bin").join("npm-cli.js"),
    ];
    for npm_cli in npm_cli_candidates {
        if npm_cli.is_file() {
            return Some(NpmRunner::NodeCli { node, npm_cli });
        }
    }

    // Last resort: a directly executable npm entry in the distribution root
    // (Windows `npm.cmd`; macOS/Linux `bin/npm` if still intact).
    let npm = if cfg!(target_os = "windows") {
        root.join("npm.cmd")
    } else {
        root.join("bin").join("npm")
    };
    if npm.exists() {
        return Some(NpmRunner::Direct(npm));
    }

    None
}

/// Locate an npm runner from the "bin directory" of a Node.js installation —
/// i.e. the directory that directly contains `node`/`node.exe`
/// (`{root}/bin` on macOS/Linux, the root itself on Windows).
pub fn npm_runner_at_bin_dir(bin_dir: &Path) -> Option<NpmRunner> {
    let root = if cfg!(target_os = "windows") {
        bin_dir.to_path_buf()
    } else {
        bin_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| bin_dir.to_path_buf())
    };
    npm_runner_at_install_root(&root)
}

/// Get the RunJam app data directory (for installing ACP packages etc).
pub fn get_runjam_data_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "runjam", "RunJam")
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Bin directory of the Node.js previously downloaded into the RunJam data
/// dir (`{data_dir}/nodejs/node-v22.12.0/bin`, or the root on Windows).
pub fn get_data_dir_node_bin_dir() -> PathBuf {
    let node_dir = get_runjam_data_dir()
        .join("nodejs")
        .join("node-v22.12.0");
    if cfg!(target_os = "windows") {
        node_dir
    } else {
        node_dir.join("bin")
    }
}

/// Resolve Node.js binary with fallback: bundled > data dir > system PATH.
pub fn resolve_node_bin(app: &AppHandle) -> Option<PathBuf> {
    // 1. Try bundled Node.js from Tauri resources
    if let Some(node) = get_bundled_node_bin(app) {
        return Some(node);
    }

    // 2. Try previously-downloaded Node.js in RunJam data dir
    let bin_dir = get_data_dir_node_bin_dir();
    let node_bin = if cfg!(target_os = "windows") {
        bin_dir.join("node.exe")
    } else {
        bin_dir.join("node")
    };
    if node_bin.exists() {
        return Some(node_bin);
    }

    // 3. Try system Node.js using an enhanced PATH
    //    (includes Homebrew, nvm, etc. — the .app process may not inherit them).
    if hidden_command("node")
        .arg("--version")
        .env("PATH", crate::agent::detector::get_enhanced_path())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some(PathBuf::from("node"));
    }

    None
}

/// Resolve an npm runner with fallback: bundled > data dir > system PATH.
///
/// The bundled / data-dir installations are always invoked as
/// `node <npm-cli.js>`, so they work even when the resource bundler flattened
/// the `bin/npm` symlink into a plain text file.
pub fn resolve_npm_runner(app: &AppHandle) -> Option<NpmRunner> {
    // 1. Try bundled npm from Tauri resources
    if let Some(bin_dir) = get_bundled_node_bin_dir(app) {
        if let Some(runner) = npm_runner_at_bin_dir(&bin_dir) {
            return Some(runner);
        }
    }

    // 2. Try previously-downloaded npm in RunJam data dir
    if let Some(runner) = npm_runner_at_bin_dir(&get_data_dir_node_bin_dir()) {
        return Some(runner);
    }

    // 3. Try system npm from PATH (enhanced with Homebrew/nvm etc.)
    if hidden_command("npm")
        .arg("--version")
        .env("PATH", crate::agent::detector::get_enhanced_path())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some(NpmRunner::Direct(PathBuf::from("npm")));
    }

    None
}
