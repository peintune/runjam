//! Simple file+stdout logger so we can tail ~/.runjam/runjam.log.
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 单个日志文件上限：超过即轮转（默认 50MB）。
const MAX_LOG_BYTES: u64 = 50 * 1024 * 1024;
/// 保留的历史备份份数（runjam.log.1 … runjam.log.{MAX}），更旧的自动删除。
/// 磁盘峰值 ≈ MAX_LOG_BYTES × (1 + MAX_LOG_FILES)。
const MAX_LOG_FILES: u32 = 1;

static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

fn log_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".runjam").join("runjam.log")
}

fn backup_path(path: &Path, n: u32) -> PathBuf {
    PathBuf::from(format!("{}.{}", path.display(), n))
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

    // 大小轮转：当前文件超过上限时切分。
    // 注意：这里不能用 rjlog! —— 会递归进入 write_to_file → ensure_file。
    if let Some(ref f) = *guard {
        let size = f.metadata().map(|m| m.len()).unwrap_or(0);
        if size >= MAX_LOG_BYTES {
            let oversized = size >= MAX_LOG_BYTES * 2;
            guard.take(); // 关闭句柄，释放当前文件
            let path = log_path();
            if oversized {
                // 异常大的文件（升级遗留 / 死循环刷爆，数百 MB）：直接删除不归档，
                // 否则一份备份就超过整个磁盘预算。例如用户现状 766MB 即走此分支。
                println!("[runjam-log] Dropped oversized log file ({} bytes)", size);
                let _ = std::fs::remove_file(&path);
            } else {
                rotate_logs_at(&path);
            }
            *guard = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok();
        }
    }
}

/// 轮转：runjam.log → runjam.log.1，历史备份依次后移，保留范围外的直接删除。
/// 纯函数（只操作传入路径），便于单元测试。
fn rotate_logs_at(path: &Path) {
    // 1) 删除保留范围外的历史（MAX_LOG_FILES=1 时清掉 .2/.3/… 遗留）
    for i in (MAX_LOG_FILES + 1).. {
        let p = backup_path(path, i);
        if !p.exists() {
            break;
        }
        let _ = std::fs::remove_file(&p);
    }
    // 2) 已有备份后移（.1 → .2 …），为新的 .1 腾位
    for i in (1..MAX_LOG_FILES).rev() {
        let from = backup_path(path, i);
        let to = backup_path(path, i + 1);
        if from.exists() {
            let _ = std::fs::rename(&from, &to);
        }
    }
    // 3) 删除将被覆盖的 .1，保持磁盘上限
    let b1 = backup_path(path, 1);
    let _ = std::fs::remove_file(&b1);
    // 4) 当前文件归档为 .1
    let _ = std::fs::rename(path, &b1);
    println!("[runjam-log] Rotated log (size >= {} bytes)", MAX_LOG_BYTES);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 每个测试用独立临时目录（按 tag 区分），避免并行测试互相干扰。
    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("runjam_logtest_{}_{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn test_rotate_moves_current_to_dot1_and_drops_older() {
        let dir = tmpdir("basic");
        let path = dir.join("runjam.log");
        fs::write(&path, "current").unwrap();
        fs::write(dir.join("runjam.log.1"), "old-1").unwrap();
        // .2 是保留范围外的遗留（MAX_LOG_FILES=1），轮转时应被清掉
        fs::write(dir.join("runjam.log.2"), "old-2").unwrap();

        rotate_logs_at(&path);

        assert!(!path.exists(), "当前文件应被归档为 .1");
        assert_eq!(
            fs::read_to_string(dir.join("runjam.log.1")).unwrap(),
            "current",
            "最新日志应归档为 .1"
        );
        assert!(!dir.join("runjam.log.2").exists(), "保留范围外的历史应被删除");
    }

    #[test]
    fn test_rotate_overwrites_previous_backup() {
        let dir = tmpdir("overwrite");
        let path = dir.join("runjam.log");
        fs::write(&path, "new").unwrap();
        fs::write(dir.join("runjam.log.1"), "old").unwrap();

        rotate_logs_at(&path);

        assert_eq!(
            fs::read_to_string(dir.join("runjam.log.1")).unwrap(),
            "new",
            "旧备份应被新归档覆盖，磁盘总量不超过 当前 + MAX_LOG_FILES 份"
        );
    }
}
