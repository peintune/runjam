use std::process::Command;
use std::env;
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
    }

    #[cfg(target_os = "linux")]
    {
        let paths = ["/usr/local/bin/ollama", "/usr/bin/ollama"];
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
            let size = parts[1].to_string();
            let digest = parts[2].to_string();
            
            let size_bytes = parse_size(&size);
            
            models.push(OllamaModel {
                name,
                size,
                digest,
                modified_at: "".to_string(),
                size_bytes,
            });
        }
    }
    
    Ok(models)
}

fn parse_size(size_str: &str) -> u64 {
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

#[tauri::command]
pub fn pull_ollama_model(model_name: String, app_handle: AppHandle) -> Result<(), String> {
    let path = match find_ollama_path() {
        Some(p) => p,
        None => return Err("Ollama not found".to_string()),
    };

    let mut child = Command::new(&path)
        .arg("pull")
        .arg(&model_name)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start ollama pull: {}", e))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(line) = line {
                if let Ok(progress) = parse_pull_progress(&line) {
                    let _ = app_handle.emit("ollama_pull_progress", &progress);
                }
            }
        }
    });

    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                if let Ok(progress) = parse_pull_progress(&line) {
                    let _ = app_handle.emit("ollama_pull_progress", &progress);
                }
            }
        }
    });

    let status = child.wait().map_err(|e| format!("Failed to wait for ollama pull: {}", e))?;
    
    if status.success() {
        let _ = app_handle.emit("ollama_pull_progress", OllamaPullProgress {
            status: "completed".to_string(),
            digest: None,
            total: None,
            completed: None,
            percentage: 100.0,
        });
        Ok(())
    } else {
        let _ = app_handle.emit("ollama_pull_progress", OllamaPullProgress {
            status: "failed".to_string(),
            digest: None,
            total: None,
            completed: None,
            percentage: 0.0,
        });
        Err("Ollama pull failed".to_string())
    }
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