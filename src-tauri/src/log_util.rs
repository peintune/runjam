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

/// Debug-level log: only emitted when RUNJAM_PROXY_DEBUG=1/true.
/// 代理热路径（每条 SSE 行、每个请求 body 预览等）平时会刷爆日志文件并
/// 浪费磁盘 I/O，默认关闭；排查协议问题时设环境变量开启。
static DEBUG_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

pub fn debug_enabled() -> bool {
    *DEBUG_ENABLED.get_or_init(|| {
        matches!(
            std::env::var("RUNJAM_PROXY_DEBUG").unwrap_or_default().to_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// Debug log gated on RUNJAM_PROXY_DEBUG. Use for hot-path/verbose logs
/// (per-SSE-line traces, request body previews); keep errors and key
/// lifecycle events on `rjlog!` so they are always recorded.
#[macro_export]
macro_rules! rjlogd {
    ($($arg:tt)*) => {{
        if $crate::log_util::debug_enabled() {
            $crate::rjlog!($($arg)*);
        }
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
