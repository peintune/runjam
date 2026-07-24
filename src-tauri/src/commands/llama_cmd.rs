use std::process::Command;
use std::env;
use std::time::Duration;
use std::io::Write;
use tauri::{AppHandle, Emitter};
use serde::{Serialize, Deserialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};

static LLAMA_SERVER_RUNNING: AtomicBool = AtomicBool::new(false);
static LLAMA_SERVER_PORT: AtomicU16 = AtomicU16::new(8080);
static LLAMA_SERVER_PID: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Serialize, Deserialize)]
pub struct LlamaModel {
    pub name: String,
    pub size: u64,
    pub modified_at: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct LlamaPullProgress {
    pub status: String,
    pub total: Option<u64>,
    pub completed: Option<u64>,
    pub percentage: f64,
}

fn get_binaries_dir() -> PathBuf {
    let mut path = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    path.pop();
    path.join("binaries").join("llama-server")
}

fn get_platform_dir() -> String {
    #[cfg(target_os = "macos")]
    {
        if cfg!(target_arch = "aarch64") {
            "macos-aarch64".to_string()
        } else {
            "macos-x86_64".to_string()
        }
    }
    #[cfg(target_os = "linux")]
    {
        "linux-x86_64".to_string()
    }
    #[cfg(target_os = "windows")]
    {
        "windows-x86_64".to_string()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        "linux-x86_64".to_string()
    }
}

fn get_llama_server_path() -> Option<PathBuf> {
    let binaries_dir = get_binaries_dir();
    let platform_dir = get_platform_dir();
    let server_path = binaries_dir.join(&platform_dir).join(if cfg!(target_os = "windows") { "llama-server.exe" } else { "llama-server" });
    
    if server_path.exists() {
        return Some(server_path);
    }
    
    let app_dir = directories::ProjectDirs::from("com", "runjam", "RunJam")
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let app_server_path = app_dir.join("binaries").join("llama-server").join(platform_dir).join(if cfg!(target_os = "windows") { "llama-server.exe" } else { "llama-server" });
    
    if app_server_path.exists() {
        return Some(app_server_path);
    }
    
    None
}

fn get_models_dir() -> PathBuf {
    let app_dir = directories::ProjectDirs::from("com", "runjam", "RunJam")
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let models_dir = app_dir.join("models");
    std::fs::create_dir_all(&models_dir).ok();
    models_dir
}

#[tauri::command]
pub fn check_llama_server_available() -> bool {
    get_llama_server_path().is_some()
}

#[tauri::command]
pub fn get_llama_server_status() -> String {
    if LLAMA_SERVER_RUNNING.load(Ordering::Relaxed) {
        return format!("running:{}", LLAMA_SERVER_PORT.load(Ordering::Relaxed));
    }
    
    if !check_llama_server_available() {
        return "not_available".to_string();
    }
    
    "stopped".to_string()
}

#[tauri::command]
pub fn list_llama_models() -> Result<Vec<LlamaModel>, String> {
    let models_dir = get_models_dir();
    
    if !models_dir.exists() {
        return Ok(Vec::new());
    }
    
    let mut models = Vec::new();
    
    for entry in std::fs::read_dir(&models_dir).map_err(|e| format!("Failed to read models dir: {}", e))? {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();
        
        if path.is_file() && path.extension().map(|e| e.to_str().unwrap_or("")) == Some("gguf") {
            let name = path.file_name().unwrap().to_str().unwrap().to_string();
            let metadata = std::fs::metadata(&path).unwrap();
            let size = metadata.len();
            let modified_at = format!("{:?}", metadata.modified().unwrap());
            
            models.push(LlamaModel { name, size, modified_at });
        }
    }
    
    Ok(models)
}

#[tauri::command]
pub fn start_llama_server(model_path: String, app_handle: AppHandle) -> Result<u16, String> {
    if LLAMA_SERVER_RUNNING.load(Ordering::Relaxed) {
        return Ok(LLAMA_SERVER_PORT.load(Ordering::Relaxed));
    }
    
    let server_path = get_llama_server_path()
        .ok_or_else(|| "llama-server binary not found".to_string())?;
    
    let models_dir = get_models_dir();
    let full_model_path = if std::path::Path::new(&model_path).is_absolute() {
        PathBuf::from(model_path)
    } else {
        models_dir.join(model_path)
    };
    
    if !full_model_path.exists() {
        return Err(format!("Model file not found: {}", full_model_path.display()));
    }
    
    let port = find_free_port(8080);
    LLAMA_SERVER_PORT.store(port, Ordering::Relaxed);
    
    let mut cmd = Command::new(&server_path);
    cmd.arg("-m").arg(&full_model_path)
        .arg("--port").arg(port.to_string())
        .arg("--host").arg("127.0.0.1")
        .arg("--api-key").arg("llama")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    
    let mut child = cmd.spawn()
        .map_err(|e| format!("Failed to start llama-server: {}", e))?;
    
    let pid = child.id();
    LLAMA_SERVER_PID.store(pid, Ordering::Relaxed);
    LLAMA_SERVER_RUNNING.store(true, Ordering::Relaxed);
    
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    
    std::thread::spawn(move || {
        let app_handle_stdout = app_handle.clone();
        let app_handle_stderr = app_handle.clone();
        
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    let _ = app_handle_stdout.emit("llama_server_log", line);
                }
            }
        });
        
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                if let Ok(line) = line {
                    let _ = app_handle_stderr.emit("llama_server_log", line);
                }
            }
        });
        
        let _ = child.wait();
        LLAMA_SERVER_RUNNING.store(false, Ordering::Relaxed);
        LLAMA_SERVER_PID.store(0, Ordering::Relaxed);
        let _ = app_handle.emit("llama_server_log", "llama-server stopped".to_string());
    });
    
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(500));
        if check_server_running(port) {
            return Ok(port);
        }
    }
    
    Err("Failed to start llama-server".to_string())
}

fn find_free_port(start_port: u16) -> u16 {
    let mut port = start_port;
    loop {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
        port += 1;
    }
}

fn check_server_running(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{}/health", port);
    match ureq::get(&url).timeout(Duration::from_secs(2)).call() {
        Ok(_) => true,
        Err(_) => false,
    }
}

#[tauri::command]
pub fn stop_llama_server() -> Result<(), String> {
    if !LLAMA_SERVER_RUNNING.load(Ordering::Relaxed) {
        return Ok(());
    }
    
    LLAMA_SERVER_RUNNING.store(false, Ordering::Relaxed);
    let pid = LLAMA_SERVER_PID.load(Ordering::Relaxed);
    
    if pid == 0 {
        return Ok(());
    }
    
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("taskkill")
            .arg("/F")
            .arg("/PID")
            .arg(pid.to_string())
            .output();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .output();
    }
    
    Ok(())
}

#[tauri::command]
pub fn download_llama_model(hf_repo: String, filename: String, app_handle: AppHandle) -> Result<(), String> {
    let models_dir = get_models_dir();
    let output_path = models_dir.join(&filename);
    
    if output_path.exists() {
        let _ = app_handle.emit("llama_pull_progress", LlamaPullProgress {
            status: "completed".to_string(),
            total: None,
            completed: None,
            percentage: 100.0,
        });
        return Ok(());
    }
    
    let url = format!("https://huggingface.co/{}/resolve/main/{}", hf_repo, filename);
    
    std::thread::spawn(move || {
        let app_handle1 = app_handle.clone();
        let app_handle2 = app_handle.clone();
        
        let _ = app_handle.emit("llama_pull_progress", LlamaPullProgress {
            status: "downloading".to_string(),
            total: None,
            completed: None,
            percentage: 0.0,
        });
        
        match ureq::get(&url).call() {
            Ok(response) => {
                let total_size = response.header("Content-Length")
                    .and_then(|s| s.parse::<u64>().ok());
                
                let mut file = match std::fs::File::create(&output_path) {
                    Ok(f) => f,
                    Err(e) => {
                        let _ = app_handle.emit("llama_pull_progress", LlamaPullProgress {
                            status: "failed".to_string(),
                            total: None,
                            completed: None,
                            percentage: 0.0,
                        });
                        let _ = app_handle.emit("llama_pull_error", format!("Failed to create file: {}", e));
                        return;
                    }
                };
                
                let mut reader = response.into_reader();
                let mut buffer = [0u8; 8192];
                let mut downloaded = 0u64;
                
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => {
                            if let Err(e) = file.write_all(&buffer[..n]) {
                                let _ = app_handle.emit("llama_pull_progress", LlamaPullProgress {
                                    status: "failed".to_string(),
                                    total: None,
                                    completed: None,
                                    percentage: 0.0,
                                });
                                let _ = app_handle.emit("llama_pull_error", format!("Failed to write file: {}", e));
                                return;
                            }
                            downloaded += n as u64;
                            
                            let percentage = if let Some(total) = total_size {
                                if total > 0 { (downloaded as f64 / total as f64) * 100.0 } else { 0.0 }
                            } else {
                                0.0
                            };
                            
                            let _ = app_handle.emit("llama_pull_progress", LlamaPullProgress {
                                status: "downloading".to_string(),
                                total: total_size,
                                completed: Some(downloaded),
                                percentage,
                            });
                        }
                        Err(e) => {
                            let _ = app_handle.emit("llama_pull_progress", LlamaPullProgress {
                                status: "failed".to_string(),
                                total: None,
                                completed: None,
                                percentage: 0.0,
                            });
                            let _ = app_handle.emit("llama_pull_error", format!("Download failed: {}", e));
                            return;
                        }
                    }
                }
                
                let _ = app_handle.emit("llama_pull_progress", LlamaPullProgress {
                    status: "completed".to_string(),
                    total: total_size,
                    completed: Some(downloaded),
                    percentage: 100.0,
                });
            }
            Err(e) => {
                let _ = app_handle.emit("llama_pull_progress", LlamaPullProgress {
                    status: "failed".to_string(),
                    total: None,
                    completed: None,
                    percentage: 0.0,
                });
                let _ = app_handle.emit("llama_pull_error", format!("Download failed: {}", e));
            }
        }
    });
    
    Ok(())
}

#[tauri::command]
pub fn create_llama_model(model_name: String, port: u16) -> Result<String, String> {
    let api_base = format!("http://127.0.0.1:{}/v1", port);
    let model_id = format!("llama-{}", model_name);
    
    Ok(serde_json::json!({
        "id": model_id,
        "name": model_name,
        "alias": model_name,
        "provider": "llama",
        "provider_name": "Llama.cpp",
        "provider_icon": "llama",
        "api_base": api_base,
        "api_key": "llama",
        "protocol": "openai_chat",
        "context_window": 0,
        "support_reasoning": false,
        "tags": ["local"],
    }).to_string())
}