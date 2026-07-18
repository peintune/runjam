use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    // Download Node.js for the target platform into src-tauri/nodejs/
    // This gets bundled as a Tauri resource and used at runtime.
    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = Path::new("nodejs");

    // Check if already downloaded (verify node binary exists)
    let node_bin = if target.contains("windows") {
        out_dir.join("node.exe")
    } else {
        out_dir.join("bin").join("node")
    };

    if !node_bin.exists() {
        download_nodejs(&target, out_dir);
    }

    // Ensure the directory exists so Tauri's resource glob doesn't fail
    if !out_dir.exists() {
        std::fs::create_dir_all(out_dir).ok();
    }

    tauri_build::build()
}

fn download_nodejs(target: &str, out_dir: &Path) {
    let (node_os, node_arch, ext) = if target.contains("apple-darwin") {
        let arch = if target.contains("aarch64") { "arm64" } else { "x64" };
        ("darwin", arch, "tar.gz")
    } else if target.contains("linux") {
        let arch = if target.contains("aarch64") { "arm64" } else { "x64" };
        ("linux", arch, "tar.gz")
    } else if target.contains("windows") {
        ("win", "x64", "zip")
    } else {
        eprintln!(
            "cargo:warning=Unknown target platform: {}, skipping Node.js download",
            target
        );
        return;
    };

    let version = "v22.12.0";
    let filename = format!("node-{}-{}-{}.{}", version, node_os, node_arch, ext);
    let url = format!("https://nodejs.org/dist/{}/{}", version, filename);

    println!(
        "cargo:warning=Downloading Node.js {} for {} ({})...",
        version, target, url
    );

    // Download
    let dl_status = Command::new("curl")
        .args(["-fsSL", &url, "-o", &filename])
        .status();

    match dl_status {
        Ok(s) if s.success() => {}
        _ => {
            eprintln!("cargo:warning=Failed to download Node.js from {}", url);
            return;
        }
    }

    // Extract
    let extract_ok = if ext == "tar.gz" {
        Command::new("tar")
            .args(["-xzf", &filename])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        Command::new("unzip")
            .args(["-o", &filename])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };

    if !extract_ok {
        eprintln!("cargo:warning=Failed to extract Node.js archive");
        let _ = std::fs::remove_file(&filename);
        return;
    }

    // Rename extracted directory (e.g. "node-v22.12.0-darwin-arm64") to "nodejs"
    let prefix = format!("node-{}", version);
    if let Ok(entries) = std::fs::read_dir(".") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&prefix) {
                let extracted = entry.path();
                // Remove existing nodejs dir if present
                if out_dir.exists() {
                    let _ = std::fs::remove_dir_all(out_dir);
                }
                if std::fs::rename(&extracted, out_dir).is_ok() {
                    println!("cargo:warning=Node.js installed to {:?}", out_dir);
                }
                break;
            }
        }
    }

    // Clean up archive
    let _ = std::fs::remove_file(&filename);
}
