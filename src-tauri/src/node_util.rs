use std::path::PathBuf;
use tauri::{AppHandle, Manager};

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

/// Get the full path to the bundled npm binary.
pub fn get_bundled_npm_bin(app: &AppHandle) -> Option<PathBuf> {
    let bin_dir = get_bundled_node_bin_dir(app)?;
    let npm = if cfg!(target_os = "windows") {
        bin_dir.join("npm.cmd")
    } else {
        bin_dir.join("npm")
    };
    Some(npm)
}

/// Get the RunJam app data directory (for installing ACP packages etc).
pub fn get_runjam_data_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "runjam", "RunJam")
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Resolve Node.js binary with fallback: bundled > data dir > system PATH.
pub fn resolve_node_bin(app: &AppHandle) -> Option<PathBuf> {
    // 1. Try bundled Node.js from Tauri resources
    if let Some(node) = get_bundled_node_bin(app) {
        return Some(node);
    }

    // 2. Try previously-downloaded Node.js in RunJam data dir
    let data_dir = get_runjam_data_dir();
    let node_dir = data_dir.join("nodejs").join("node-v22.12.0");
    let bin_dir = if cfg!(target_os = "windows") {
        node_dir
    } else {
        node_dir.join("bin")
    };
    let node_bin = if cfg!(target_os = "windows") {
        bin_dir.join("node.exe")
    } else {
        bin_dir.join("node")
    };
    if node_bin.exists() {
        return Some(node_bin);
    }

    // 3. Try system Node.js from PATH
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some(PathBuf::from("node"));
    }

    None
}

/// Resolve npm binary with fallback: bundled > data dir > system PATH.
pub fn resolve_npm_bin(app: &AppHandle) -> Option<PathBuf> {
    // 1. Try bundled npm from Tauri resources
    if let Some(npm) = get_bundled_npm_bin(app) {
        return Some(npm);
    }

    // 2. Try previously-downloaded npm in RunJam data dir
    let data_dir = get_runjam_data_dir();
    let node_dir = data_dir.join("nodejs").join("node-v22.12.0");
    let bin_dir = if cfg!(target_os = "windows") {
        node_dir
    } else {
        node_dir.join("bin")
    };
    let npm_bin = if cfg!(target_os = "windows") {
        bin_dir.join("npm.cmd")
    } else {
        bin_dir.join("npm")
    };
    if npm_bin.exists() {
        return Some(npm_bin);
    }

    // 3. Try system npm from PATH
    if std::process::Command::new("npm")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some(PathBuf::from("npm"));
    }

    None
}
