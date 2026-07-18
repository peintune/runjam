//! Simple file+stdout logger so we can tail ~/.runjam/runjam.log.
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

fn log_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".runjam").join("runjam.log")
}

fn ensure_file() {
    let mut guard = LOG_FILE.lock().unwrap();
    if guard.is_none() {
        let path = log_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        *guard = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
    }
}

/// Log a line to both stdout and the log file.
#[macro_export]
macro_rules! rjlog {
    ($($arg:tt)*) => {{
        let line = format!($($arg)*);
        println!("{}", line);
        $crate::log_util::write_to_file(&line);
    }};
}

/// Write a raw string to the log file (no extra formatting).
pub fn write_to_file(line: &str) {
    ensure_file();
    let mut guard = LOG_FILE.lock().unwrap();
    if let Some(ref mut f) = *guard {
        let ts = chrono::Local::now().format("%H:%M:%S%.3f");
        let _ = writeln!(f, "[{}] {}", ts, line);
        let _ = f.flush();
    }
}

/// Write a block of text to the log file, prefixing each line with a timestamp.
pub fn write_block(prefix: &str, block: &str) {
    let now = chrono::Local::now().format("%H:%M:%S%.3f");
    for l in block.lines() {
        let line = format!("[{}] {} {}", now, prefix, l);
        println!("{}", line);
        ensure_file();
        let mut guard = LOG_FILE.lock().unwrap();
        if let Some(ref mut f) = *guard {
            let _ = writeln!(f, "{}", line);
            let _ = f.flush();
        }
    }
}
