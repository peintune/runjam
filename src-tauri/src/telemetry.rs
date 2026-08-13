//! Telemetry: local queue + batched reporting to the RunJam backend
//! (Vercel + Supabase).
//!
//! Design rules:
//! - **No IP / PII collection client-side.** The server resolves the country
//!   from the request IP; we never send the IP itself.
//! - **Local SQLite queue** so network failures never block the app.
//! - **Opt-out supported**: disabling telemetry clears the queue and stops
//!   further enqueues. User-submitted feedback is always sent.
//! - All outgoing text is sanitized (home paths, API keys/tokens) and
//!   length-capped before it leaves the machine.
//!
//! Endpoint layout (see `website/app/api/...`):
//! - POST {base}/api/telemetry/register
//! - POST {base}/api/telemetry/events
//! - POST {base}/api/telemetry/errors
//! - POST {base}/api/feedback

use crate::db::connection::Database;
use rusqlite::Connection;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::Manager;

pub const KEY_INSTALLATION_ID: &str = "installation_id";
pub const KEY_TELEMETRY_ENABLED: &str = "telemetry_enabled";
pub const KEY_CONSENT_SHOWN: &str = "telemetry_consent_shown";
pub const DEFAULT_API_BASE: &str = "https://runjam.app";

const MAX_QUEUE_BATCH: i64 = 50;
const MAX_ATTEMPTS: i64 = 5;
const FLUSH_THRESHOLD: i64 = 20;

/// Set once in `run()` so background threads and the panic hook can reach the
/// managed database + trigger flushes.
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();
static FLUSHING: AtomicBool = AtomicBool::new(false);

pub(crate) fn api_base() -> String {
    std::env::var("RUNJAM_API_BASE").unwrap_or_else(|_| DEFAULT_API_BASE.to_string())
}

// ── settings ───────────────────────────────────────────────────────────

pub fn get_or_create_installation_id(conn: &Connection) -> String {
    let existing: Result<String, _> = conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        [KEY_INSTALLATION_ID],
        |r| r.get(0),
    );
    match existing {
        Ok(id) => id,
        Err(_) => {
            let id = uuid::Uuid::new_v4().to_string();
            let _ = conn.execute(
                "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)",
                rusqlite::params![KEY_INSTALLATION_ID, id],
            );
            id
        }
    }
}

/// Default ON (industry standard for dev tools), user can opt out.
pub fn is_enabled(conn: &Connection) -> bool {
    let v: Result<String, _> = conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        [KEY_TELEMETRY_ENABLED],
        |r| r.get(0),
    );
    match v {
        Ok(s) => s == "1" || s.eq_ignore_ascii_case("true"),
        Err(_) => true,
    }
}

pub fn consent_shown(conn: &Connection) -> bool {
    let v: Result<String, _> = conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        [KEY_CONSENT_SHOWN],
        |r| r.get(0),
    );
    v.is_ok()
}

pub fn set_enabled(conn: &Connection, enabled: bool) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)",
        rusqlite::params![KEY_TELEMETRY_ENABLED, if enabled { "1" } else { "0" }],
    )?;
    // Opt-out also wipes anything queued but not yet sent.
    if !enabled {
        conn.execute("DELETE FROM telemetry_queue", [])?;
    }
    Ok(())
}

pub fn mark_consent_shown(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)",
        rusqlite::params![KEY_CONSENT_SHOWN, "1"],
    )
    .map(|_| ())
}

// ── sanitization ────────────────────────────────────────────────────────

static REDACT_RE: OnceLock<regex::Regex> = OnceLock::new();

/// Strip local paths and secrets from any text before it leaves the machine.
pub fn sanitize(text: &str) -> String {
    let home = directories::UserDirs::new()
        .map(|d| d.home_dir().to_string_lossy().to_string())
        .unwrap_or_default();
    let mut s = text.to_string();
    if !home.is_empty() {
        s = s.replace(&home, "~");
    }
    let re = REDACT_RE.get_or_init(|| {
        regex::Regex::new(
            r#"(sk-[A-Za-z0-9_-]{8,}|Bearer\s+[A-Za-z0-9._\-]+|api[_-]?key["']?\s*[=:]\s*["']?[A-Za-z0-9._\-]{6,}|AKIA[0-9A-Z]{16})"#,
        )
        .expect("valid redact regex")
    });
    s = re.replace_all(&s, "[REDACTED]").to_string();
    if s.len() > 8000 {
        s.truncate(8000);
    }
    s
}

// ── queue ───────────────────────────────────────────────────────────────

fn enqueue_forced(db: &Database, kind: &str, payload: &serde_json::Value) {
    let conn = match db.conn.lock() {
        Ok(c) => c,
        Err(_) => return,
    };
    let body = payload.to_string();
    let _ = conn.execute(
        "INSERT INTO telemetry_queue (kind, payload, attempts, created_at) VALUES (?1, ?2, 0, CURRENT_TIMESTAMP)",
        rusqlite::params![kind, body],
    );
}

fn enqueue(db: &Database, kind: &str, payload: &serde_json::Value) {
    let conn = match db.conn.lock() {
        Ok(c) => c,
        Err(_) => return,
    };
    if !is_enabled(&conn) {
        return; // opted out: drop silently
    }
    drop(conn);
    enqueue_forced(db, kind, payload);
}

fn queue_len(db: &Database) -> i64 {
    let conn = match db.conn.lock() {
        Ok(c) => c,
        Err(_) => return 0,
    };
    conn.query_row("SELECT COUNT(*) FROM telemetry_queue", [], |r| r.get(0))
        .unwrap_or(0)
}

fn installation_id(db: &Database) -> String {
    let conn = match db.conn.lock() {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    get_or_create_installation_id(&conn)
}

// ── public API ──────────────────────────────────────────────────────────

/// Device registration (idempotent on the server). No IP is included.
pub fn register(db: &Database, app_version: &str, platform: &str, arch: &str, os_version: &str, enabled: bool) {
    let id = installation_id(db);
    if id.is_empty() {
        return;
    }
    let payload = serde_json::json!({
        "installation_id": id,
        "app_version": app_version,
        "platform": platform,
        "arch": arch,
        "os_version": os_version,
        "telemetry_enabled": enabled,
    });
    enqueue(db, "register", &payload);
}

/// Key feature usage event.
pub fn track(db: &Database, event_name: &str, props: serde_json::Value) {
    let id = installation_id(db);
    if id.is_empty() {
        return;
    }
    let payload = serde_json::json!({
        "installation_id": id,
        "events": [{
            "event_name": event_name,
            "event_props": props,
            "event_time": chrono::Utc::now().to_rfc3339(),
        }],
    });
    enqueue(db, "events", &payload);
    if queue_len(db) >= FLUSH_THRESHOLD {
        if let Some(app) = APP_HANDLE.get() {
            flush_async(app);
        }
    }
}

/// Sanitized error log entry.
pub fn report_error(db: &Database, level: &str, category: &str, message: &str, stack: Option<&str>, context: serde_json::Value) {
    let id = installation_id(db);
    if id.is_empty() {
        return;
    }
    let payload = serde_json::json!({
        "installation_id": id,
        "errors": [{
            "level": level,
            "category": category,
            "message": sanitize(message),
            "stack": stack.map(sanitize),
            "context": context,
            "created_at": chrono::Utc::now().to_rfc3339(),
        }],
    });
    enqueue(db, "errors", &payload);
}

/// User feedback — sent even when telemetry is disabled (explicit user action).
pub fn enqueue_feedback(db: &Database, payload: &serde_json::Value) {
    enqueue_forced(db, "feedback", payload);
}

/// Drain the queue in one batch (called on a background thread).
pub fn flush_sync(app: &tauri::AppHandle) {
    let base = api_base();
    let rows: Vec<(i64, String, String)> = {
        let db = app.state::<Mutex<Database>>();
        let guard = match db.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let conn = match guard.conn.lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut stmt = match conn.prepare("SELECT id, kind, payload FROM telemetry_queue ORDER BY id LIMIT ?1") {
            Ok(s) => s,
            Err(_) => return,
        };
        let iter = match stmt.query_map([MAX_QUEUE_BATCH], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))) {
            Ok(i) => i,
            Err(_) => return,
        };
        iter.flatten().collect()
    };

    if rows.is_empty() {
        return;
    }

    let mut ok_ids: Vec<i64> = Vec::new();
    let mut fail_ids: Vec<i64> = Vec::new();
    for (id, kind, payload) in rows {
        let url = match kind.as_str() {
            "feedback" => format!("{}/api/feedback", base),
            k => format!("{}/api/telemetry/{}", base, k),
        };
        if post_json(&url, &payload) {
            ok_ids.push(id);
        } else {
            fail_ids.push(id);
        }
    }

    let db = app.state::<Mutex<Database>>();
    let guard = match db.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let conn = match guard.conn.lock() {
        Ok(c) => c,
        Err(_) => return,
    };
    for id in ok_ids {
        let _ = conn.execute("DELETE FROM telemetry_queue WHERE id = ?1", [id]);
    }
    for id in fail_ids {
        let _ = conn.execute(
            "UPDATE telemetry_queue SET attempts = attempts + 1 WHERE id = ?1",
            [id],
        );
        // Give up after MAX_ATTEMPTS so a dead endpoint can't grow the queue forever.
        let _ = conn.execute(
            "DELETE FROM telemetry_queue WHERE id = ?1 AND attempts >= ?2",
            rusqlite::params![id, MAX_ATTEMPTS],
        );
    }
}

/// Spawn a one-shot background flush (guarded against pile-up).
pub fn flush_async(app: &tauri::AppHandle) {
    if FLUSHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        flush_sync(&app);
        FLUSHING.store(false, Ordering::SeqCst);
    });
}

fn post_json(url: &str, body: &str) -> bool {
    let resp = ureq::post(url)
        .timeout(Duration::from_secs(10))
        .set("Content-Type", "application/json")
        .send_string(body);
    match resp {
        Ok(r) => (200..300).contains(&r.status()),
        Err(_) => false,
    }
}

// ── panic hook ──────────────────────────────────────────────────────────

pub fn init_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        prev(info);
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| info.to_string());
        let location = info.location().map(|l| l.to_string());
        report_panic(&msg, location.as_deref());
    }));
}

pub fn set_app_handle(app: tauri::AppHandle) {
    let _ = APP_HANDLE.set(app);
}

fn report_panic(msg: &str, location: Option<&str>) {
    let Some(app) = APP_HANDLE.get() else { return };
    let db = app.state::<Mutex<Database>>();
    // try_lock: if the panic happened while another thread held the db lock we
    // must not deadlock in the hook.
    if let Ok(guard) = db.try_lock() {
        let stack = location.map(|l| format!("at {}", l));
        report_error(&guard, "error", "rust_panic", msg, stack.as_deref(), serde_json::json!({}));
    }
    flush_async(app);
}
