fn main() {
    let app_dir = directories::ProjectDirs::from("com", "runjam", "RunJam")
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    
    println!("App dir: {}", app_dir.display());
    println!("DB path: {}", app_dir.join("runjam.db").display());
    println!("Dir exists: {}", app_dir.exists());
    
    let db_path = app_dir.join("runjam.db");
    println!("DB exists: {}", db_path.exists());
    
    match std::fs::metadata(&db_path) {
        Ok(m) => println!("DB size: {} bytes", m.len()),
        Err(e) => println!("DB metadata error: {}", e),
    }
}
