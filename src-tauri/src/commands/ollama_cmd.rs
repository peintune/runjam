use std::process::Command;
use std::env;
use std::time::Duration;
use std::sync::{Mutex, Arc};
use tauri::{AppHandle, Emitter};
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    pub size: String,
    pub digest: String,
    pub modified_at: String,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct OllamaPullProgress {
    pub status: String,
    pub digest: Option<String>,
    pub total: Option<u64>,
    pub completed: Option<u64>,
    pub percentage: f64,
}

fn find_ollama_path() -> Option<String> {
    if let Ok(path) = env::var("OLLAMA_BIN") {
        if !path.is_empty() {
            return Some(path);
        }
    }

    if let Ok(output) = Command::new("which").arg("ollama").output() {
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
    }

    if let Ok(output) = Command::new("where").arg("ollama").output() {
        if output.status.success() {
            let paths = String::from_utf8_lossy(&output.stdout);
            return Some(paths.lines().next().unwrap_or("").trim().to_string());
        }
    }

    #[cfg(target_os = "macos")]
    {
        let path = "/usr/local/bin/ollama";
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
        let path = "/opt/homebrew/bin/ollama";
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }

    #[cfg(target_os = "linux")]
    {
        let paths = ["/usr/local/bin/ollama", "/usr/bin/ollama", "/opt/homebrew/bin/ollama"];
        for path in paths.iter() {
            if std::path::Path::new(path).exists() {
                return Some(path.to_string());
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = env::var("LOCALAPPDATA") {
            let path = format!("{}\\Ollama\\ollama.exe", local_app_data);
            if std::path::Path::new(&path).exists() {
                return Some(path);
            }
        }
    }

    None
}

fn ensure_ollama_running() -> Result<(), String> {
    let path = match find_ollama_path() {
        Some(p) => p,
        None => return Err("Ollama not found".to_string()),
    };

    let output = Command::new(&path).arg("list").output();
    match output {
        Ok(out) if out.status.success() => return Ok(()),
        _ => {}
    }

    let _ = Command::new(&path).arg("serve").spawn()
        .map_err(|e| format!("Failed to start Ollama service: {}", e))?;

    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(500));
        let output = Command::new(&path).arg("list").output();
        if let Ok(out) = output {
            if out.status.success() {
                return Ok(());
            }
        }
    }

    Err("Failed to start Ollama service".to_string())
}

#[tauri::command]
pub fn check_ollama_installed() -> bool {
    find_ollama_path().is_some()
}

#[tauri::command]
pub fn get_ollama_status() -> String {
    let path = match find_ollama_path() {
        Some(p) => p,
        None => return "not_installed".to_string(),
    };

    let output = Command::new(&path).arg("--version").output();
    match output {
        Ok(out) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            format!("installed:{}", version)
        }
        _ => "installed_but_error".to_string(),
    }
}

#[tauri::command]
pub fn list_ollama_models() -> Result<Vec<OllamaModel>, String> {
    let path = match find_ollama_path() {
        Some(p) => p,
        None => return Err("Ollama not found".to_string()),
    };

    ensure_ollama_running()?;

    let output = Command::new(&path).arg("list").output()
        .map_err(|e| format!("Failed to run ollama list: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("Ollama list failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    
    let mut models = Vec::new();
    
    for (i, line) in lines.iter().enumerate() {
        if i == 0 && line.starts_with("NAME") {
            continue;
        }
        
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 3 {
            let name = parts[0].to_string();
            let size = parts[2].to_string();
            let digest = parts[1].to_string();
            
            let size_bytes = parse_size(&size);
            
            models.push(OllamaModel {
                name,
                size,
                digest,
                modified_at: if parts.len() >= 4 { parts[3..].join(" ").to_string() } else { "".to_string() },
                size_bytes,
            });
        }
    }
    
    Ok(models)
}

fn parse_size(size_str: &str) -> u64 {
    if size_str == "-" {
        return 0;
    }
    let s = size_str.to_lowercase();
    let num: f64 = s.chars().filter(|c| c.is_numeric() || *c == '.').collect::<String>().parse().unwrap_or(0.0);
    
    if s.ends_with("gb") {
        (num * 1_000_000_000.0) as u64
    } else if s.ends_with("mb") {
        (num * 1_000_000.0) as u64
    } else if s.ends_with("kb") {
        (num * 1_000.0) as u64
    } else {
        num as u64
    }
}

fn clean_escape_codes(s: &str) -> String {
    let mut result = String::new();
    let mut i = 0;
    let bytes = s.as_bytes();
    
    while i < bytes.len() {
        if bytes[i] == b'\x1b' {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'[' || bytes[i] == b';' || bytes[i] == b'h' || bytes[i] == b'l' || bytes[i] == b'G' || bytes[i] == b'K') {
                i += 1;
            }
        } else if bytes[i] == b'\r' {
            i += 1;
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    
    result
}

fn extract_error_message(output: &str) -> String {
    for line in output.lines().rev() {
        let cleaned_str = clean_escape_codes(line);
        let cleaned = cleaned_str.trim();
        if cleaned.starts_with("Error:") || cleaned.starts_with("error:") || cleaned.contains("permission") || cleaned.contains("denied") {
            return cleaned.to_string();
        }
    }
    
    let cleaned = clean_escape_codes(output);
    let lines: Vec<&str> = cleaned.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() > 3 {
        lines[lines.len() - 3..].join("\n").to_string()
    } else {
        cleaned.trim().to_string()
    }
}

fn ensure_ollama_dir_permissions() -> bool {
    #[cfg(target_os = "macos")]
    {
        let home_dir = match directories::UserDirs::new() {
            Some(d) => d.home_dir().to_path_buf(),
            None => return false,
        };
        let ollama_dir = home_dir.join(".ollama");
        if !ollama_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&ollama_dir) {
                return false;
            }
        }
        
        let _ = Command::new("chmod")
            .arg("-R")
            .arg("755")
            .arg(&ollama_dir)
            .status();
        
        let _ = Command::new("xattr")
            .arg("-rd")
            .arg("com.apple.provenance")
            .arg(&ollama_dir)
            .status();
        
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

#[tauri::command]
pub fn pull_ollama_model(model_name: String, app_handle: AppHandle) -> Result<(), String> {
    let path = match find_ollama_path() {
        Some(p) => p,
        None => return Err("Ollama not found".to_string()),
    };

    ensure_ollama_running()?;
    ensure_ollama_dir_permissions();

    std::thread::spawn(move || {
        let app_handle1 = app_handle.clone();
        let app_handle2 = app_handle.clone();

        let error_output = Arc::new(Mutex::new(String::new()));
        let error_output_clone = error_output.clone();

        let mut child = match Command::new(&path)
            .arg("pull")
            .arg(&model_name)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = app_handle.emit("ollama_pull_progress", OllamaPullProgress {
                    status: "failed".to_string(),
                    digest: None,
                    total: None,
                    completed: None,
                    percentage: 0.0,
                });
                return;
            }
        };

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let _ = app_handle.emit("ollama_pull_progress", OllamaPullProgress {
                    status: "failed".to_string(),
                    digest: None,
                    total: None,
                    completed: None,
                    percentage: 0.0,
                });
                return;
            }
        };
        let stderr = match child.stderr.take() {
            Some(s) => s,
            None => {
                let _ = app_handle.emit("ollama_pull_progress", OllamaPullProgress {
                    status: "failed".to_string(),
                    digest: None,
                    total: None,
                    completed: None,
                    percentage: 0.0,
                });
                return;
            }
        };

        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    let cleaned_line = clean_escape_codes(&line);
                    if let Ok(progress) = parse_pull_progress(&cleaned_line) {
                        let _ = app_handle1.emit("ollama_pull_progress", &progress);
                    }
                }
            }
        });

        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                if let Ok(line) = line {
                    error_output.lock().unwrap().push_str(&line);
                    error_output.lock().unwrap().push('\n');
                    let cleaned_line = clean_escape_codes(&line);
                    if let Ok(progress) = parse_pull_progress(&cleaned_line) {
                        let _ = app_handle2.emit("ollama_pull_progress", &progress);
                    }
                }
            }
        });

        let status = match child.wait() {
            Ok(s) => s,
            Err(_) => {
                let _ = app_handle.emit("ollama_pull_progress", OllamaPullProgress {
                    status: "failed".to_string(),
                    digest: None,
                    total: None,
                    completed: None,
                    percentage: 0.0,
                });
                return;
            }
        };
        
        if status.success() {
            let _ = app_handle.emit("ollama_pull_progress", OllamaPullProgress {
                status: "completed".to_string(),
                digest: None,
                total: None,
                completed: None,
                percentage: 100.0,
            });
        } else {
            let raw_error = error_output_clone.lock().unwrap().trim().to_string();
            let extracted_error = extract_error_message(&raw_error);
            
            let _ = app_handle.emit("ollama_pull_progress", OllamaPullProgress {
                status: "failed".to_string(),
                digest: None,
                total: None,
                completed: None,
                percentage: 0.0,
            });
            
            if extracted_error.contains("operation not permitted") || extracted_error.contains("permission") {
                let _ = app_handle.emit("ollama_pull_error", format!("Permission denied. Please run `ollama pull {}` in terminal first.", model_name));
            } else {
                let _ = app_handle.emit("ollama_pull_error", format!("Ollama pull failed: {}", extracted_error));
            }
        }
    });

    Ok(())
}

fn parse_pull_progress(line: &str) -> Result<OllamaPullProgress, ()> {
    let trimmed = line.trim();
    
    if trimmed.starts_with("pulling manifest") {
        return Ok(OllamaPullProgress {
            status: "pulling_manifest".to_string(),
            digest: None,
            total: None,
            completed: None,
            percentage: 0.0,
        });
    }
    
    if trimmed.starts_with("pulling") {
        if let Some(digest) = trimmed.split_whitespace().nth(1) {
            return Ok(OllamaPullProgress {
                status: "pulling".to_string(),
                digest: Some(digest.to_string()),
                total: None,
                completed: None,
                percentage: 0.0,
            });
        }
    }
    
    if trimmed.starts_with("verifying") {
        return Ok(OllamaPullProgress {
            status: "verifying".to_string(),
            digest: None,
            total: None,
            completed: None,
            percentage: 95.0,
        });
    }
    
    if trimmed.starts_with("writing") {
        return Ok(OllamaPullProgress {
            status: "writing".to_string(),
            digest: None,
            total: None,
            completed: None,
            percentage: 98.0,
        });
    }
    
    if trimmed.starts_with("success") {
        return Ok(OllamaPullProgress {
            status: "success".to_string(),
            digest: None,
            total: None,
            completed: None,
            percentage: 100.0,
        });
    }
    
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() >= 3 {
        let completed_str = parts[0];
        let total_str = parts[2];
        
        if let (Ok(completed), Ok(total)) = (completed_str.parse::<u64>(), total_str.parse::<u64>()) {
            let percentage = if total > 0 { (completed as f64 / total as f64) * 100.0 } else { 0.0 };
            return Ok(OllamaPullProgress {
                status: "downloading".to_string(),
                digest: None,
                total: Some(total),
                completed: Some(completed),
                percentage,
            });
        }
    }
    
    Err(())
}

#[tauri::command]
pub fn create_ollama_model(model_name: String) -> Result<String, String> {
    let api_base = "http://localhost:11434/v1";
    let model_id = format!("ollama-{}", model_name);
    
    Ok(serde_json::json!({
        "id": model_id,
        "name": model_name,
        "alias": model_name,
        "provider": "ollama",
        "provider_name": "Ollama",
        "provider_icon": "ollama",
        "api_base": api_base,
        "api_key": "ollama",
        "protocol": "openai_chat",
        "context_window": 0,
        "support_reasoning": false,
        "tags": ["local"],
    }).to_string())
}