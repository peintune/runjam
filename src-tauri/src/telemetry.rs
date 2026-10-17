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
pub const KEY_PROXY_URL: &str = "outbound_proxy";
// 注意用 www 前缀：裸域名 runjam.app 会 308 跳转到 www.runjam.app，而 ureq 2.x
// 对带 body 的 POST 不自动跟随 307/308（见 AgentBuilder::redirects 文档），
// 会导致所有上报静默失败。post_json_with 里另有防御性重定向跟随。
pub const DEFAULT_API_BASE: &str = "https://www.runjam.app";

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

/// Default ON (industry standard for dev tools), user can opt out anytime in
/// Settings → General.
pub fn is_enabled(conn: &Connection) -> bool {
    let v: Result<String, _> = conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        [KEY_TELEMETRY_ENABLED],
        |r| r.get(0),
    );
    match v {
        Ok(s) => s == "1" || s.eq_ignore_ascii_case("true"),
        // Opt-out by default: telemetry starts enabled so we can learn how the
        // product is used. Users who don't want it can disable the anonymous
        // usage-data switch in Settings → General. Users who explicitly opted
        // out keep their choice (the row is stored as "0").
        Err(_) => true,
    }
}

pub fn set_enabled(conn: &Connection, enabled: bool) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)",
        rusqlite::params![KEY_TELEMETRY_ENABLED, if enabled { "1" } else { "0" }],
    )?;
    // Opt-out also wipes anything queued but not yet sent.
    if !enabled {
        conn.execute("DELETE FROM telemetry_queue WHERE kind <> 'feedback'", [])?;
    }
    Ok(())
}

/// Outbound proxy used for telemetry reporting. Empty string = no proxy.
pub fn get_proxy_url(conn: &Connection) -> String {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        [KEY_PROXY_URL],
        |r| r.get(0),
    )
    .unwrap_or_default()
}

pub fn set_proxy_url(conn: &Connection, url: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)",
        rusqlite::params![KEY_PROXY_URL, url],
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
        let mut end = 8000;
        while !s.is_char_boundary(end) { end -= 1; }
        s.truncate(end);
    }
    s
}

/// Recursively sanitize every string in a JSON value. Context objects can
/// carry raw protocol lines (paths, tokens) that would otherwise leave the
/// machine unsanitized.
fn sanitize_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(sanitize(s)),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(sanitize_value).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter().map(|(k, v)| (k.clone(), sanitize_value(v))).collect(),
        ),
        other => other.clone(),
    }
}

// ── queue ───────────────────────────────────────────────────────────────

fn diagnostic_payload(payload: &serde_json::Value) -> serde_json::Value {
    let mut value = payload.clone();
    let allowed = ["code", "status", "http_status", "duration_ms", "retry_count", "agent_type", "cli", "model", "stop_reason", "stopReason", "update_type", "platform", "arch"];
    if let Some(errors) = value.get_mut("errors").and_then(|v| v.as_array_mut()) {
        for item in errors {
            let category = item.get("category").and_then(|v| v.as_str())
                .filter(|v| !v.is_empty() && v.len() <= 32 && v.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_:-".contains(&b)))
                .unwrap_or("unknown").to_string();
            let context: serde_json::Map<String, serde_json::Value> = item.get("context").and_then(|v| v.as_object())
                .map(|map| map.iter().filter(|(key, v)| allowed.contains(&key.as_str()) &&
                    (v.is_boolean() || v.is_number() || v.as_str().map(|s| !s.is_empty() && s.len() <= 100 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.:/-".contains(&b))).unwrap_or(false)))
                    .map(|(k,v)| (k.clone(), if let Some(text) = v.as_str() { serde_json::Value::String(sanitize(text)) } else { v.clone() })).collect())
                .unwrap_or_default();
            *item = serde_json::json!({"level":item.get("level"), "category":category,
                "message":format!("客户端诊断：{}", category), "context":context, "created_at":item.get("created_at")});
        }
    }
    value
}

fn queue_body(kind: &str, payload: &serde_json::Value) -> String {
    let mut value = if kind == "errors" { diagnostic_payload(payload) } else { payload.clone() };
    if let Some(object) = value.as_object_mut() {
        let key = if kind == "feedback" { "request_id" } else { "batch_id" };
        object.entry(key).or_insert_with(|| serde_json::Value::String(uuid::Uuid::new_v4().to_string()));
    }
    value.to_string()
}

fn report_route(base: &str, kind: &str) -> String {
    if kind == "feedback" {
        format!("{}/api/v1/runjam/feedback", base.trim_end_matches('/'))
    } else {
        format!("{}/api/v1/runjam/telemetry/{}", base.trim_end_matches('/'), kind)
    }
}

fn enqueue_forced(db: &Database, kind: &str, payload: &serde_json::Value) {
    let conn = match db.conn.lock() {
        Ok(c) => c,
        Err(_) => return,
    };
    let body = queue_body(kind, payload);
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

/// App version + platform for the current build, sourced from the app handle
/// (set once at startup). Falls back to empty strings if the handle is not yet
/// available (e.g. very early startup or tests) so the payload stays valid.
fn app_version_and_platform() -> (String, String) {
    match APP_HANDLE.get() {
        Some(app) => (
            app.package_info().version.to_string(),
            crate::commands::telemetry_cmd::platform_name().to_string(),
        ),
        None => (String::new(), String::new()),
    }
}

/// Key feature usage event.
pub fn track(db: &Database, event_name: &str, props: serde_json::Value) {
    let id = installation_id(db);
    if id.is_empty() {
        return;
    }
    let (app_version, platform) = app_version_and_platform();
    let payload = serde_json::json!({
        "installation_id": id,
        "app_version": app_version,
        "platform": platform,
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
    let (app_version, platform) = app_version_and_platform();
    let payload = serde_json::json!({
        "installation_id": id,
        "app_version": app_version,
        "platform": platform,
        "errors": [{
            "level": level,
            "category": category,
            "message": sanitize(message),
            "stack": stack.map(sanitize),
            "context": sanitize_value(&context),
            "created_at": chrono::Utc::now().to_rfc3339(),
        }],
    });
    enqueue(db, "errors", &payload);
}

/// Same as `report_error`, for call sites that only hold an `AppHandle`
/// (ACP stdout reader threads, session runner, ...).
///
/// Flushes immediately afterwards: an error is the one payload worth pushing
/// out right away, since a crash may follow within milliseconds. The flush
/// itself runs on its own thread, so this stays cheap.
///
/// NOT safe to call from a panic hook — use `report_panic` there, which takes
/// the database with `try_lock`.
pub fn report_error_from_app(
    app: &tauri::AppHandle,
    level: &str,
    category: &str,
    message: &str,
    stack: Option<&str>,
    context: serde_json::Value,
) {
    // A blocking lock on purpose: unlike `try_lock` it cannot silently drop
    // the report when a flush happens to hold the database at that moment.
    if let Some(db) = app.try_state::<Mutex<Database>>() {
        if let Ok(guard) = db.lock() {
            report_error(&guard, level, category, message, stack, context);
        }
    }
    flush_async(app);
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

    // Build an agent honoring the configured outbound proxy (if any). The
    // proxy URL is re-read on every flush so config changes take effect
    // immediately.
    let agent = build_agent(app);

    let mut ok_ids: Vec<i64> = Vec::new();
    let mut fail_ids: Vec<i64> = Vec::new();
    for (id, kind, payload) in rows {
        // A user can opt out while a flush owns a snapshot of the queue.
        // Recheck before each outgoing telemetry request; explicit feedback is
        // independent of this setting and must not disappear with the queue.
        if kind != "feedback" {
            let enabled = app.state::<Mutex<Database>>().lock().ok()
                .and_then(|db| db.conn.lock().ok().map(|conn| is_enabled(&conn)))
                .unwrap_or(false);
            if !enabled { ok_ids.push(id); continue; }
        }
        // Upgrade old queued diagnostics before they leave the machine too.
        let payload = if kind == "errors" {
            match serde_json::from_str::<serde_json::Value>(&payload) {
                Ok(mut value) => {
                    if let Some(object) = value.as_object_mut() {
                        object.entry("batch_id").or_insert_with(|| serde_json::Value::String(format!("legacy-queue-{}", id)));
                    }
                    queue_body("errors", &value)
                },
                Err(_) => { ok_ids.push(id); continue; }
            }
        } else { payload };
        let url = report_route(&base, &kind);
        if post_json_with(&agent, &url, &payload) {
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

/// Build an HTTP agent that routes requests through the configured outbound
/// proxy (HTTP/HTTPS/SOCKS5). Invalid proxy configs fall back to direct.
pub fn build_agent(app: &tauri::AppHandle) -> ureq::Agent {
    let proxy_url = proxy_url_from_app(app);
    let mut builder = ureq::builder().timeout(Duration::from_secs(10));
    let proxy_url = proxy_url.trim();
    if !proxy_url.is_empty() {
        if let Ok(p) = ureq::Proxy::new(proxy_url) {
            builder = builder.proxy(p);
        }
    }
    builder.build()
}

/// Read the configured outbound proxy URL. Returns empty string if the
/// database is unavailable or no proxy is configured.
fn proxy_url_from_app(app: &tauri::AppHandle) -> String {
    let db = app.state::<Mutex<Database>>();
    let Ok(guard) = db.lock() else {
        return String::new();
    };
    let Ok(conn) = guard.conn.lock() else {
        return String::new();
    };
    get_proxy_url(&conn)
}

fn post_json_with(agent: &ureq::Agent, url: &str, body: &str) -> bool {
    let mut url = url.to_string();
    // ureq 2.x 对带 body 的 POST 不会自动跟随 307/308（见 AgentBuilder::redirects
    // 文档）。若 API 域名存在裸域名 308 跳转，这里手动跟随，最多 3 跳。
    for _ in 0..3 {
        let resp = agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_string(body);
        match resp {
            Ok(r) if matches!(r.status(), 307 | 308) => {
                let Some(loc) = r.header("location") else {
                    return false;
                };
                match resolve_redirect(&url, loc) {
                    Some(next) => url = next,
                    None => return false,
                }
            }
            Ok(r) => return (200..300).contains(&r.status()),
            Err(_) => return false,
        }
    }
    false
}

/// 将重定向 location 解析为完整 URL（支持绝对 URL 与根路径相对 URL）。
fn resolve_redirect(current: &str, location: &str) -> Option<String> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Some(location.to_string());
    }
    let (scheme, rest) = current.split_once("://")?;
    let host = rest.split('/').next()?;
    if let Some(path) = location.strip_prefix('/') {
        Some(format!("{}://{}/{}", scheme, host, path))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_diagnostics_drop_free_text_before_network() {
        let input = serde_json::json!({"errors":[{"category":"session_turn_failed",
            "message":"personal conversation", "stack":"private workspace source",
            "context":{"raw":"unpublished protocol", "cli":"codex"}}]});
        let body = queue_body("errors", &input);
        assert!(!body.contains("personal"));
        assert!(!body.contains("private"));
        assert!(!body.contains("unpublished"));
        assert!(body.contains("codex"));
    }

    #[test]
    fn opt_out_preserves_explicit_feedback() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT,updated_at TEXT);
            CREATE TABLE telemetry_queue(kind TEXT,payload TEXT);
            INSERT INTO telemetry_queue VALUES('events','{}'),('errors','{}'),('feedback','{}');").unwrap();
        set_enabled(&conn, false).unwrap();
        let remaining: String = conn.query_row("SELECT group_concat(kind) FROM telemetry_queue", [], |r| r.get(0)).unwrap();
        assert_eq!(remaining, "feedback");
    }

    #[test]
    fn sanitizer_caps_chinese_on_a_utf8_boundary() {
        let text = sanitize(&"汉".repeat(3000));
        assert!(text.len() <= 8000);
        assert!(text.ends_with('汉'));
    }

    #[test]
    fn all_report_routes_are_product_scoped_and_retry_identity_is_stable() {
        assert_eq!(report_route("https://www.runjam.app", "events"), "https://www.runjam.app/api/v1/runjam/telemetry/events");
        assert_eq!(report_route("https://www.runjam.app", "feedback"), "https://www.runjam.app/api/v1/runjam/feedback");
        let first = queue_body("events", &serde_json::json!({"events": []}));
        let value: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert!(value["batch_id"].as_str().is_some());
        assert_eq!(queue_body("events", &value), first);
    }

    #[test]
    fn resolve_redirect_handles_absolute_and_root_relative() {
        assert_eq!(
            resolve_redirect(
                "https://runjam.app/api/telemetry/events",
                "https://www.runjam.app/api/telemetry/events"
            )
            .as_deref(),
            Some("https://www.runjam.app/api/telemetry/events")
        );
        assert_eq!(
            resolve_redirect("https://runjam.app/api/telemetry/events", "/api/telemetry/events").as_deref(),
            Some("https://runjam.app/api/telemetry/events")
        );
        assert_eq!(resolve_redirect("https://runjam.app/a/b", "x"), None);
    }

    /// 对真实后端做冒烟测试：POST 经裸域名（308）跟随后必须成功。
    /// 需要网络；CI 不可用时自动跳过（post_json_with 返回 false 无法区分
    /// 网络错误与 3xx 未跟随，此处直接断言成功以覆盖本地开发场景）。
    #[test]
    fn post_follows_308_redirect_to_www() {
        let agent = ureq::builder().timeout(std::time::Duration::from_secs(15)).build();
        // 裸域名 runjam.app 会 308 到 www.runjam.app；默认 base 已用 www，
        // 这里显式用裸域名验证手动跟随逻辑生效。
        let ok = post_json_with(&agent, "https://runjam.app/api/telemetry/register", r#"{"installation_id":"00000000-0000-4000-8000-000000000001"}"#);
        assert!(ok, "POST 经 308 重定向后应返回 2xx");
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
    let stack = location.map(|l| format!("at {}", l));
    // Hand-rolled instead of `report_error_from_app`: try_lock, because if the
    // panic happened while another thread held the db lock we must not
    // deadlock inside the hook.
    if let Some(db) = app.try_state::<Mutex<Database>>() {
        if let Ok(guard) = db.try_lock() {
            report_error(&guard, "error", "rust_panic", msg, stack.as_deref(), serde_json::json!({}));
        }
    }
    flush_async(app);
}
