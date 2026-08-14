//! Update & announcement helpers (pure functions, unit-testable).

use serde::Serialize;

/// True on Windows (updater active). macOS/Linux use the download-redirect path.
pub fn is_windows() -> bool {
    std::env::consts::OS == "windows"
}

/// Semantic version comparison: does `current` >= `min`?
/// Accepts optional leading 'v'. Empty `min` means no floor (always true).
pub fn version_ge(current: &str, min: &str) -> bool {
    if min.trim().is_empty() {
        return true;
    }
    let a = parse_version(current);
    let b = parse_version(min);
    match (a, b) {
        (Some(a), Some(b)) => {
            (a.0, a.1, a.2) >= (b.0, b.1, b.2)
        }
        // Unparseable: fall back to string comparison so we never hide a
        // release due to a parse bug.
        _ => current.trim() >= min.trim(),
    }
}

fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let s = v.trim().trim_start_matches('v');
    let mut it = s.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.split(['-', '+']).next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// Unified result returned to the frontend. `action` is "install" (Windows)
/// or "open_download" (macOS/Linux).
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub update_available: bool,
    pub action: String,
    pub latest_version: Option<String>,
    pub notes: Option<String>,
    pub download_url: Option<String>,
}

impl UpdateCheckResult {
    pub fn none() -> Self {
        Self {
            update_available: false,
            action: if is_windows() { "install".into() } else { "open_download".into() },
            latest_version: None,
            notes: None,
            download_url: None,
        }
    }
}

use rusqlite::Connection;
use serde::Deserialize;

pub const KEY_ANNOUNCEMENT_READ_PREFIX: &str = "announcement_read:";

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Announcement {
    pub id: String,
    pub title: String,
    pub body: String,
    pub level: String, // "info" | "important"
    pub created_at: Option<String>,
}

pub fn is_announcement_read(conn: &Connection, id: &str) -> bool {
    let key = format!("{}{}", KEY_ANNOUNCEMENT_READ_PREFIX, id);
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        [&key],
        |r| r.get::<_, String>(0),
    )
    .map(|v| v == "1")
    .unwrap_or(false)
}

pub fn mark_announcement_read(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    let key = format!("{}{}", KEY_ANNOUNCEMENT_READ_PREFIX, id);
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?1, '1', CURRENT_TIMESTAMP)",
        [&key],
    )
    .map(|_| ())
}

pub fn filter_unread(conn: &Connection, items: Vec<Announcement>) -> Vec<Announcement> {
    items
        .into_iter()
        .filter(|a| !is_announcement_read(conn, &a.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ge_compares_semver() {
        assert!(version_ge("v0.2.0", "v0.1.0"));
        assert!(version_ge("v0.1.0", "v0.1.0"));
        assert!(!version_ge("v0.1.0", "v0.2.0"));
        assert!(version_ge("v0.2.0", "v0.1.5"));
        assert!(version_ge("v0.1.0", "v0.1.0-beta"));
    }

    #[test]
    fn version_ge_handles_missing_v_prefix() {
        assert!(version_ge("0.2.0", "v0.1.0"));
        assert!(version_ge("v0.2.0", "0.1.0"));
    }

    #[test]
    fn version_ge_min_version_null_means_always_visible() {
        // min_version 为空时公告始终可见（由调用方处理，这里测试解析空串）
        assert!(version_ge("v0.1.0", ""));
    }

    #[test]
    fn filter_unread_removes_read_items() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);",
        )
        .unwrap();
        mark_announcement_read(&conn, "a").unwrap();

        let items = vec![
            Announcement { id: "a".into(), title: "read".into(), body: "".into(), level: "info".into(), created_at: None },
            Announcement { id: "b".into(), title: "new".into(), body: "".into(), level: "important".into(), created_at: None },
        ];
        let unread = filter_unread(&conn, items);
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].id, "b");
    }
}
