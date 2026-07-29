use std::process::Command;
use std::env;
use std::time::Duration;
use std::io::Write;
use tauri::{AppHandle, Emitter};
use serde::{Serialize, Deserialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};
use crate::util::hidden_command;

static LLAMA_SERVER_RUNNING: AtomicBool = AtomicBool::new(false);
static LLAMA_SERVER_PORT: AtomicU16 = AtomicU16::new(19090);
static LLAMA_SERVER_PID: AtomicU32 = AtomicU32::new(0);
static LLAMA_SERVER_MODEL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

static DOWNLOADING_MODEL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static DOWNLOAD_PROGRESS: std::sync::Mutex<Option<LlamaPullProgress>> = std::sync::Mutex::new(None);

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
    let platform_dir = get_platform_dir();
    let binary_name = if cfg!(target_os = "windows") { "llama-server.exe" } else { "llama-server" };
    
    #[cfg(debug_assertions)]
    {
        let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dev_path = src_dir.join("binaries").join("llama-server").join(&platform_dir).join(binary_name);
        if dev_path.exists() {
            return Some(dev_path);
        }
    }
    
    let mut exe_path = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    exe_path.pop();
    let exe_relative_path = exe_path.join("binaries").join("llama-server").join(&platform_dir).join(binary_name);
    if exe_relative_path.exists() {
        return Some(exe_relative_path);
    }
    
    #[cfg(target_os = "macos")]
    {
        let mut bundle_path = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        bundle_path.pop(); // remove "runjam" -> Contents/MacOS
        bundle_path.pop(); // remove "MacOS" -> Contents
        // bundle_path is now RunJam.app/Contents/
        let bundle_resources = bundle_path.join("Resources").join("binaries").join("llama-server").join(&platform_dir).join(binary_name);
        if bundle_resources.exists() {
            return Some(bundle_resources);
        }
    }
    
    #[cfg(target_os = "windows")]
    {
        let mut exe_path = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        exe_path.pop();
        let resources_path = exe_path.join("resources").join("binaries").join("llama-server").join(&platform_dir).join(binary_name);
        if resources_path.exists() {
            return Some(resources_path);
        }
    }
    
    #[cfg(target_os = "linux")]
    {
        let mut exe_path = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        exe_path.pop();
        let resources_path = exe_path.join("share").join("runjam").join("binaries").join("llama-server").join(&platform_dir).join(binary_name);
        if resources_path.exists() {
            return Some(resources_path);
        }
    }
    
    let app_dir = directories::ProjectDirs::from("com", "runjam", "RunJam")
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let app_server_path = app_dir.join("binaries").join("llama-server").join(platform_dir).join(binary_name);
    
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
pub fn open_llama_models_dir() -> Result<(), String> {
    let models_dir = get_models_dir();
    if !models_dir.exists() {
        std::fs::create_dir_all(&models_dir)
            .map_err(|e| format!("Failed to create models directory: {}", e))?;
    }
    
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        Command::new("open")
            .arg(&models_dir)
            .spawn()
            .map_err(|e| format!("Failed to open directory: {}", e))?;
    }
    
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        hidden_command("explorer")
            .arg(&models_dir)
            .spawn()
            .map_err(|e| format!("Failed to open directory: {}", e))?;
    }
    
    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        Command::new("xdg-open")
            .arg(&models_dir)
            .spawn()
            .map_err(|e| format!("Failed to open directory: {}", e))?;
    }
    
    Ok(())
}

#[tauri::command]
pub fn get_llama_server_status() -> String {
    if LLAMA_SERVER_RUNNING.load(Ordering::Relaxed) {
        return format!("running:{}", LLAMA_SERVER_PORT.load(Ordering::Relaxed));
    }
    
    for port in 19090..19100 {
        if check_server_running(port) {
            return format!("running:{}", port);
        }
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
    let model_filename = std::path::Path::new(&model_path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| model_path.clone());
    
    println!("[DEBUG] start_llama_server: model_path={}, model_filename={}", model_path, model_filename);
    
    if LLAMA_SERVER_RUNNING.load(Ordering::Relaxed) {
        *LLAMA_SERVER_MODEL.lock().unwrap() = Some(model_filename.clone());
        println!("[DEBUG] Server already running (flag), set LLAMA_SERVER_MODEL to: {}", model_filename);
        return Ok(LLAMA_SERVER_PORT.load(Ordering::Relaxed));
    }
    
    if check_server_running(19090) {
        LLAMA_SERVER_PORT.store(19090, Ordering::Relaxed);
        LLAMA_SERVER_RUNNING.store(true, Ordering::Relaxed);
        *LLAMA_SERVER_MODEL.lock().unwrap() = Some(model_filename.clone());
        println!("[DEBUG] Server detected on port 19090, set LLAMA_SERVER_MODEL to: {}", model_filename);
        return Ok(19090);
    }
    
    let server_path = get_llama_server_path()
        .ok_or_else(|| {
            let mut msg = "llama-server binary not found. Searched paths:\n".to_string();
            msg.push_str("- CARGO_MANIFEST_DIR/binaries/\n");
            msg.push_str("- Executable directory/binaries/\n");
            msg.push_str("- App resources/binaries/\n");
            msg.push_str("- User data directory/binaries/");
            msg
        })?;
    
    let models_dir = get_models_dir();
    
    let full_model_path = if std::path::Path::new(&model_path).is_absolute() {
        PathBuf::from(model_path)
    } else {
        models_dir.join(&model_filename)
    };
    
    if !full_model_path.exists() {
        return Err(format!("Model file not found: {}", full_model_path.display()));
    }
    
    let port = find_free_port(19090);
    LLAMA_SERVER_PORT.store(port, Ordering::Relaxed);
    
    let mut cmd = hidden_command(&server_path);
    cmd.arg("-m").arg(&full_model_path)
        .arg("--port").arg(port.to_string())
        .arg("--host").arg("127.0.0.1")
        .arg("--jinja")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    
    println!("[DEBUG] Starting llama-server: path={}, model={}, port={}", 
        server_path.display(), full_model_path.display(), port);
    
    let mut child = cmd.spawn()
        .map_err(|e| format!("Failed to start llama-server. Server path: {}, Model path: {}, Error: {}", 
            server_path.display(), full_model_path.display(), e))?;
    
    let pid = child.id();
    println!("[DEBUG] llama-server started with PID: {}", pid);
    
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    
    let app_handle_stdout = app_handle.clone();
    let app_handle_stderr = app_handle.clone();
    let app_handle_stderr_report = app_handle.clone();
    let app_handle_exit = app_handle.clone();
    let model_filename_exit = model_filename.clone();
    let port_for_exit = port;
    
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(line) = line {
                println!("[llama-server stdout] {}", line);
                let _ = app_handle_stdout.emit("llama_server_log", line);
            }
        }
    });
    
    let mut stderr_lines: Vec<String> = Vec::new();
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                println!("[llama-server stderr] {}", line);
                stderr_lines.push(line.clone());
                let _ = app_handle_stderr.emit("llama_server_log", line);
            }
        }
        
        let _ = app_handle_stderr_report.emit("llama_server_stderr", stderr_lines);
    });
    
    std::thread::spawn(move || {
        let status = child.wait();
        LLAMA_SERVER_RUNNING.store(false, Ordering::Relaxed);
        LLAMA_SERVER_PID.store(0, Ordering::Relaxed);
        
        match status {
            Ok(exit_status) => {
                let exit_code = exit_status.code().unwrap_or(-1);
                println!("[DEBUG] llama-server exited with code: {}", exit_code);
                let _ = app_handle_exit.emit("llama_server_log", 
                    format!("llama-server stopped (exit code: {})", exit_code));
                
                if exit_code != 0 && !check_server_running(port_for_exit) {
                    let error_msg = format!("llama-server failed to start (exit code: {}). Model: {}", exit_code, model_filename_exit);
                    println!("[ERROR] {}", error_msg);
                    let _ = app_handle_exit.emit("llama_server_failed", error_msg);
                }
            }
            Err(e) => {
                println!("[ERROR] Failed to wait for llama-server: {}", e);
                let _ = app_handle_exit.emit("llama_server_failed", 
                    format!("Failed to monitor llama-server: {}", e));
            }
        }
    });
    
    let app_handle_check = app_handle.clone();
    let port_check = port;
    std::thread::spawn(move || {
        println!("[DEBUG] Waiting for llama-server to become ready on port {}...", port_check);
        for i in 0..1200 {
            std::thread::sleep(Duration::from_millis(500));
            if check_server_running(port_check) {
                println!("[DEBUG] llama-server is ready on port {} after {} attempts", port_check, i + 1);
                LLAMA_SERVER_RUNNING.store(true, Ordering::Relaxed);
                *LLAMA_SERVER_MODEL.lock().unwrap() = Some(model_filename.clone());
                let _ = app_handle_check.emit("llama_server_started", port_check);
                return;
            }
        }
        println!("[ERROR] llama-server failed to become ready on port {} after 60 seconds", port_check);
        let _ = app_handle_check.emit("llama_server_started", 0);
    });
    
    Ok(port)
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
    let url = format!("http://127.0.0.1:{}/v1/models", port);
    match ureq::get(&url).timeout(Duration::from_secs(5)).call() {
        Ok(response) => {
            let status = response.status();
            status == 200 || status == 201 || status == 204
        }
        Err(_) => false,
    }
}

#[tauri::command]
pub fn get_server_status() -> Result<serde_json::Value, String> {
    let port = LLAMA_SERVER_PORT.load(Ordering::Relaxed);
    let model = LLAMA_SERVER_MODEL.lock().unwrap().clone();
    
    println!("[DEBUG] get_server_status: port={}, model={:?}, LLAMA_SERVER_RUNNING={}", port, model, LLAMA_SERVER_RUNNING.load(Ordering::Relaxed));
    
    if port != 0 && check_server_running(port) {
        println!("[DEBUG] Server running on port {}, model={:?}", port, model);
        Ok(serde_json::json!({
            "running": true,
            "port": port,
            "model": model
        }))
    } else {
        for p in 19090..19100 {
            if check_server_running(p) {
                LLAMA_SERVER_PORT.store(p, Ordering::Relaxed);
                println!("[DEBUG] Detected server on port {}, model={:?}", p, model);
                return Ok(serde_json::json!({
                    "running": true,
                    "port": p,
                    "model": model
                }));
            }
        }
        println!("[DEBUG] No server running");
        Ok(serde_json::json!({
            "running": false,
            "port": 0,
            "model": null
        }))
    }
}

#[tauri::command]
pub fn stop_llama_server() -> Result<(), String> {
    LLAMA_SERVER_RUNNING.store(false, Ordering::Relaxed);
    
    let pid = LLAMA_SERVER_PID.load(Ordering::Relaxed);
    LLAMA_SERVER_PID.store(0, Ordering::Relaxed);
    *LLAMA_SERVER_MODEL.lock().unwrap() = None;
    
    if pid != 0 {
        #[cfg(target_os = "windows")]
        {
            let _ = hidden_command("taskkill")
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
    }
    
    #[cfg(target_os = "windows")]
    {
        let _ = hidden_command("taskkill")
            .arg("/F")
            .arg("/IM")
            .arg("llama-server.exe")
            .output();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("pkill")
            .arg("-TERM")
            .arg("-f")
            .arg("llama-server")
            .output();
    }
    
    Ok(())
}

#[tauri::command]
pub fn download_llama_model(hf_repo: String, filename: String, app_handle: AppHandle) -> Result<(), String> {
    let models_dir = get_models_dir();
    let output_path = models_dir.join(&filename);
    
    if output_path.exists() {
        let url = format!("https://huggingface.co/{}/resolve/main/{}?download=true", hf_repo, filename);
        match ureq::head(&url).call() {
            Ok(response) => {
                if let Some(content_length) = response.header("Content-Length").and_then(|s| s.parse::<u64>().ok()) {
                    if let Ok(metadata) = std::fs::metadata(&output_path) {
                        if metadata.len() >= content_length {
                            let _ = app_handle.emit("llama_pull_progress", LlamaPullProgress {
                                status: "completed".to_string(),
                                total: Some(content_length),
                                completed: Some(metadata.len()),
                                percentage: 100.0,
                            });
                            return Ok(());
                        }
                    }
                }
            }
            Err(_) => {}
        }
    }
    
    let url = format!("https://huggingface.co/{}/resolve/main/{}?download=true", hf_repo, filename);
    
    *DOWNLOADING_MODEL.lock().unwrap() = Some(filename.clone());
    
    std::thread::spawn(move || {
        let app_handle_clone = app_handle.clone();
        let app_handle_download = app_handle.clone();
        
        let mut progress = LlamaPullProgress {
            status: "downloading".to_string(),
            total: None,
            completed: None,
            percentage: 0.0,
        };
        *DOWNLOAD_PROGRESS.lock().unwrap() = Some(progress.clone());
        let _ = app_handle_clone.emit("llama_pull_progress", progress);
        
        std::thread::spawn(move || {
            let interval_handle = app_handle_clone;
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let current_progress = DOWNLOAD_PROGRESS.lock().unwrap().clone();
                if let Some(p) = current_progress {
                    if p.status != "downloading" {
                        break;
                    }
                    let _ = interval_handle.emit("llama_pull_progress", p);
                } else {
                    break;
                }
            }
        });
        
        match ureq::get(&url).call() {
            Ok(response) => {
                let total_size = response.header("Content-Length")
                    .and_then(|s| s.parse::<u64>().ok());
                
                let mut file = match std::fs::File::create(&output_path) {
                    Ok(f) => f,
                    Err(e) => {
                        let progress = LlamaPullProgress {
                            status: "failed".to_string(),
                            total: None,
                            completed: None,
                            percentage: 0.0,
                        };
                        *DOWNLOAD_PROGRESS.lock().unwrap() = Some(progress.clone());
                        *DOWNLOADING_MODEL.lock().unwrap() = None;
                        let _ = app_handle_download.emit("llama_pull_progress", progress);
                        let _ = app_handle_download.emit("llama_pull_error", format!("Failed to create file: {}", e));
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
                                let progress = LlamaPullProgress {
                                    status: "failed".to_string(),
                                    total: None,
                                    completed: None,
                                    percentage: 0.0,
                                };
                                *DOWNLOAD_PROGRESS.lock().unwrap() = Some(progress.clone());
                                *DOWNLOADING_MODEL.lock().unwrap() = None;
                                let _ = app_handle_download.emit("llama_pull_progress", progress);
                                let _ = app_handle_download.emit("llama_pull_error", format!("Failed to write file: {}", e));
                                return;
                            }
                            downloaded += n as u64;
                            
                            let percentage = if let Some(total) = total_size {
                                if total > 0 { (downloaded as f64 / total as f64) * 100.0 } else { 0.0 }
                            } else {
                                0.0
                            };
                            
                            let progress = LlamaPullProgress {
                                status: "downloading".to_string(),
                                total: total_size,
                                completed: Some(downloaded),
                                percentage,
                            };
                            *DOWNLOAD_PROGRESS.lock().unwrap() = Some(progress.clone());
                            let _ = app_handle_download.emit("llama_pull_progress", progress);
                        }
                        Err(e) => {
                            let progress = LlamaPullProgress {
                                status: "failed".to_string(),
                                total: None,
                                completed: None,
                                percentage: 0.0,
                            };
                            *DOWNLOAD_PROGRESS.lock().unwrap() = Some(progress.clone());
                            *DOWNLOADING_MODEL.lock().unwrap() = None;
                            let _ = app_handle_download.emit("llama_pull_progress", progress);
                            let _ = app_handle_download.emit("llama_pull_error", format!("Download failed: {}", e));
                            return;
                        }
                    }
                }
                
                let progress = LlamaPullProgress {
                    status: "completed".to_string(),
                    total: total_size,
                    completed: Some(downloaded),
                    percentage: 100.0,
                };
                *DOWNLOAD_PROGRESS.lock().unwrap() = Some(progress.clone());
                *DOWNLOADING_MODEL.lock().unwrap() = None;
                let _ = app_handle_download.emit("llama_pull_progress", progress);
            }
            Err(e) => {
                let progress = LlamaPullProgress {
                    status: "failed".to_string(),
                    total: None,
                    completed: None,
                    percentage: 0.0,
                };
                *DOWNLOAD_PROGRESS.lock().unwrap() = Some(progress.clone());
                *DOWNLOADING_MODEL.lock().unwrap() = None;
                let _ = app_handle_download.emit("llama_pull_progress", progress);
                let _ = app_handle_download.emit("llama_pull_error", format!("Download failed: {}", e));
            }
        }
    });
    
    Ok(())
}

#[tauri::command]
pub fn get_download_status() -> Result<serde_json::Value, String> {
    let downloading = DOWNLOADING_MODEL.lock().unwrap().clone();
    let progress = DOWNLOAD_PROGRESS.lock().unwrap().clone();
    Ok(serde_json::json!({
        "downloading": downloading,
        "progress": progress.unwrap_or(LlamaPullProgress {
            status: "idle".to_string(),
            total: None,
            completed: None,
            percentage: 0.0,
        }),
    }))
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