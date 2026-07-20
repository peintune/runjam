use std::path::PathBuf;

fn get_app_data_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "runjam", "RunJam")
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn open_path(path: &PathBuf) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub fn get_data_dir() -> String {
    let dir = get_app_data_dir();
    dir.to_string_lossy().to_string()
}

#[tauri::command]
pub fn open_data_dir() -> Result<(), String> {
    let dir = get_app_data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    open_path(&dir)
}

#[tauri::command]
pub fn open_in_finder(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("Path does not exist: {}", path));
    }
    open_path(&p)
}
