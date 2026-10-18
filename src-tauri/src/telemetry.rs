//! Local metadata queue and native feedback transport through the product Edge gateway.
//! The server records the public request IP. Automatic reports contain only
//! approved usage/system metadata; explicit feedback is independent of opt-out.

use crate::db::connection::Database;
use rusqlite::Connection;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::Manager;

pub const KEY_INSTALLATION_ID: &str = "installation_id";
pub const KEY_TELEMETRY_ENABLED: &str = "telemetry_enabled";
pub const KEY_PROXY_URL: &str = "outbound_proxy";
pub const DEFAULT_API_BASE: &str = "https://supabase-proxy.runjam.app/functions/v1/platform/runjam";

const MAX_QUEUE_BATCH: i64 = 50;
const FLUSH_THRESHOLD: i64 = 20;

/// Set once in `run()` so background threads and the panic hook can reach the
/// managed database + trigger flushes.
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();
static FLUSHING: AtomicBool = AtomicBool::new(false);

pub(crate) fn api_base() -> String {
    // Local fixtures may override the full product base in debug builds only.
    if cfg!(debug_assertions) {
        if let Ok(base) = std::env::var("RUNJAM_API_BASE") {
            if let Ok(url) = url::Url::parse(&base) {
                if matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) {
                    return base.trim_end_matches('/').to_string();
                }
            }
        }
    }
    DEFAULT_API_BASE.to_string()
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
        // product is used. Users who don't want it can disable the
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
    let allowed = ["code", "status", "http_status", "duration_ms", "retry_count", "agent_type", "cli", "stop_reason", "stopReason", "update_type", "platform", "arch"];
    if let Some(errors) = value.get_mut("errors").and_then(|v| v.as_array_mut()) {
        for item in errors {
            let category = item.get("category").and_then(|v| v.as_str())
                .filter(|v| !v.is_empty() && v.len() <= 32 && v.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_:-".contains(&b)))
                .unwrap_or("unknown").to_string();
            let context: serde_json::Map<String, serde_json::Value> = item.get("context").and_then(|v| v.as_object())
                .map(|map| map.iter().filter(|(key, v)| allowed.contains(&key.as_str()) &&
                    (v.is_boolean() || v.is_number() || v.as_str().map(|s| !s.is_empty() && s.len() <= 100 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))).unwrap_or(false)))
                    .map(|(k,v)| (k.clone(), if let Some(text) = v.as_str() { serde_json::Value::String(sanitize(text)) } else { v.clone() })).collect())
                .unwrap_or_default();
            *item = serde_json::json!({"level":item.get("level"), "category":category,
                "message":format!("客户端诊断：{}", category), "context":context, "created_at":item.get("created_at")});
        }
    }
    value
}

fn usage_payload(payload: &serde_json::Value) -> serde_json::Value {
    let mut value = payload.clone();
    if let Some(events) = value.get_mut("events").and_then(|v| v.as_array_mut()) {
        for item in events {
            let name = item
                .get("event_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let name = if [
                "app_started",
                "page_view",
                "session_started",
                "session_stopped",
                "attach_files",
                "runaway_guard_stopped",
                "message_sent",
                "model_changed",
                "skill_toggled",
                "message_stopped",
                "llama_server_start",
            ]
            .contains(&name)
            {
                name
            } else {
                "unknown"
            };
            let mut props = serde_json::Map::new();
            if let Some(original) = item.get("event_props").and_then(|v| v.as_object()) {
                for (key, v) in original {
                    let allowed_number = [
                        "count",
                        "duration_ms",
                        "input_tokens",
                        "output_tokens",
                        "retry_count",
                    ]
                    .contains(&key.as_str())
                        && v.is_number();
                    let allowed_bool =
                        ["enabled", "has_attachments"].contains(&key.as_str()) && v.is_boolean();
                    let allowed_enum = match key.as_str() {
                        "platform" => matches!(
                            v.as_str(),
                            Some("macos" | "windows" | "darwin" | "win32" | "linux")
                        ),
                        "arch" => matches!(v.as_str(), Some("aarch64" | "x86_64" | "x86" | "arm")),
                        "cli" => matches!(v.as_str(), Some("claude" | "codex" | "gemini")),
                        "version" | "os_version" => v
                            .as_str()
                            .map(|s| {
                                s.len() <= 100
                                    && s.bytes()
                                        .all(|b| b.is_ascii_alphanumeric() || b".-_ ".contains(&b))
                            })
                            .unwrap_or(false),
                        "launch_id" => v
                            .as_str()
                            .map(|s| uuid::Uuid::parse_str(s).is_ok())
                            .unwrap_or(false),
                        _ => false,
                    };
                    if allowed_number || allowed_bool || allowed_enum {
                        props.insert(key.clone(), v.clone());
                    }
                }
            }
            *item = serde_json::json!({"event_name":name,"event_props":props,"event_time":item.get("event_time")});
        }
    }
    value
}

fn queue_body(kind: &str, payload: &serde_json::Value) -> String {
    let mut value = if kind == "errors" { diagnostic_payload(payload) } else if kind == "events" { usage_payload(payload) } else { payload.clone() };
    if let Some(object) = value.as_object_mut() {
        let key = if kind == "feedback" { "request_id" } else { "batch_id" };
        object.entry(key).or_insert_with(|| serde_json::Value::String(uuid::Uuid::new_v4().to_string()));
    }
    value.to_string()
}

fn report_route(base: &str, kind: &str) -> String {
    if kind == "feedback" {
        format!("{}/feedback", base.trim_end_matches('/'))
    } else {
        format!("{}/telemetry/{}", base.trim_end_matches('/'), kind)
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

/// Enqueue one durable launch event with an ID retained across offline retries.
pub fn enqueue_startup(db: &Database, version: &str, platform: &str, arch: &str, os_version: &str) {
    register(db, version, platform, arch, os_version, true);
    let payload = serde_json::json!({
        "installation_id": installation_id(db), "app_version":version, "platform":platform,
        "events":[{"event_name":"app_started", "event_time":chrono::Utc::now().to_rfc3339(),
            "event_props":{"version":version,"platform":platform,"arch":arch,"os_version":os_version,
                "launch_id":uuid::Uuid::new_v4().to_string()}}]
    });
    enqueue(db, "events", &payload);
}

pub fn os_version() -> String {
    #[cfg(unix)]
    {
        #[cfg(target_os = "macos")]
        let mut command = { let mut c = std::process::Command::new("/usr/bin/sw_vers"); c.arg("-productVersion"); c };
        #[cfg(not(target_os = "macos"))]
        let mut command = { let mut c = std::process::Command::new("uname"); c.arg("-r"); c };
        command.output().ok().filter(|r| r.status.success())
            .map(|r| String::from_utf8_lossy(&r.stdout).trim().to_string()).unwrap_or_default()
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .creation_flags(0x08000000)
            .output()
            .ok()
            .filter(|r| r.status.success())
            .and_then(|r| {
                String::from_utf8_lossy(&r.stdout)
                    .split(|c: char| !c.is_ascii_digit() && c != '.')
                    .filter(|part| part.contains('.') && part.chars().any(|c| c.is_ascii_digit()))
                    .max_by_key(|part| part.len())
                    .map(str::to_string)
            })
            .unwrap_or_default()
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        String::new()
    }
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

/// Drain the queue in one batch (called on a background thread).
pub fn flush_sync(app: &tauri::AppHandle) {
    let agent = build_agent(app);
    flush_database(&app.state::<Mutex<Database>>(), &agent, &api_base());
}

fn flush_database(db: &Mutex<Database>, agent: &ureq::Agent, base: &str) {
    let rows: Vec<(i64, String, String)> = {
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
        // A user can opt out while a flush owns a snapshot of the queue.
        // Recheck before each outgoing telemetry request; explicit feedback is
        // independent of this setting and must not disappear with the queue.
        if kind != "feedback" {
            let enabled = db.lock().ok()
                .and_then(|db| db.conn.lock().ok().map(|conn| is_enabled(&conn)))
                .unwrap_or(false);
            if !enabled { ok_ids.push(id); continue; }
        }
        // Upgrade and persist legacy identities before the first send, so retries
        // remain idempotent even after restarting the app. Scrub old automatic data.
        let payload = match serde_json::from_str::<serde_json::Value>(&payload) {
            Ok(mut value) => {
                if kind == "feedback" {
                    if let Some(object) = value.as_object_mut() {
                        object.entry("platform").or_insert_with(|| serde_json::json!(crate::commands::telemetry_cmd::platform_name()));
                        object.entry("app_version").or_insert_with(|| serde_json::json!(env!("CARGO_PKG_VERSION")));
                    }
                }
                queue_body(&kind, &value)
            },
            Err(_) => { ok_ids.push(id); continue; }
        };
        {
            let Ok(guard) = db.lock() else { return; };
            let Ok(conn) = guard.conn.lock() else { return; };
            if conn.execute("UPDATE telemetry_queue SET payload = ?1 WHERE id = ?2", rusqlite::params![payload, id]).is_err() { continue; }
        }
        let url = report_route(base, &kind);
        if post_json_with(agent, &url, &payload) {
            ok_ids.push(id);
        } else {
            fail_ids.push(id);
        }
    }

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
        // Retain offline work until acknowledgement; opt-out explicitly clears
        // automatic rows. Retrying must never silently lose a queued launch.
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

/// Feedback succeeds only after the server confirms persisted storage.
pub fn send_feedback_with(
    agent: &ureq::Agent,
    base: &str,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    post_acknowledged_json(
        agent,
        &report_route(base, "feedback"),
        &payload.to_string(),
        true,
    )
}

fn post_acknowledged_json(
    agent: &ureq::Agent,
    url: &str,
    body: &str,
    feedback: bool,
) -> Result<serde_json::Value, String> {
    let response = agent
        .post(url)
        .set("Content-Type", "application/json")
        .send_string(body)
        .map_err(|_| "Could not deliver the request. Please retry.".to_string())?;
    let status = response.status();
    if !(200..300).contains(&status) || (feedback && status != 201) {
        return Err("The server did not acknowledge the request. Please retry.".into());
    }
    let result: serde_json::Value = response
        .into_json()
        .map_err(|_| "Invalid server acknowledgement".to_string())?;
    if result["ok"] != true
        || (feedback && result["id"].as_str().filter(|s| uuid::Uuid::parse_str(s).is_ok()).is_none())
    {
        return Err("The server did not acknowledge the request. Please retry.".into());
    }
    Ok(result)
}

fn post_json_with(agent: &ureq::Agent, url: &str, body: &str) -> bool {
    post_acknowledged_json(agent, url, body, url.ends_with("/feedback")).is_ok()
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
        assert_eq!(report_route("https://supabase-proxy.runjam.app/functions/v1/platform/runjam", "events"), "https://supabase-proxy.runjam.app/functions/v1/platform/runjam/telemetry/events");
        assert_eq!(report_route("https://supabase-proxy.runjam.app/functions/v1/platform/runjam", "feedback"), "https://supabase-proxy.runjam.app/functions/v1/platform/runjam/feedback");
        let first = queue_body("events", &serde_json::json!({"events": []}));
        let value: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert!(value["batch_id"].as_str().is_some());
        assert_eq!(queue_body("events", &value), first);
    }

    fn test_db() -> Mutex<Database> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT,updated_at TEXT);
            CREATE TABLE telemetry_queue(id INTEGER PRIMARY KEY,kind TEXT,payload TEXT,attempts INTEGER,created_at TEXT);").unwrap();
        Mutex::new(Database { conn: Mutex::new(conn) })
    }

    #[test]
    fn startup_reports_real_metadata_once_per_launch_and_respects_disabled_preference() {
        let db = test_db();
        let guard = db.lock().unwrap();
        enqueue_startup(&guard, "1.2.3", "macos", "aarch64", "24.6.0");
        let conn = guard.conn.lock().unwrap();
        let body: String = conn.query_row("SELECT payload FROM telemetry_queue WHERE kind='events'", [], |r| r.get(0)).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(payload["app_version"], "1.2.3");
        assert_eq!(payload["platform"], "macos");
        assert_eq!(payload["events"][0]["event_name"], "app_started");
        assert_eq!(payload["events"][0]["event_props"]["arch"], "aarch64");
        assert_eq!(payload["events"][0]["event_props"]["os_version"], "24.6.0");
        assert!(uuid::Uuid::parse_str(payload["events"][0]["event_props"]["launch_id"].as_str().unwrap()).is_ok());
        set_enabled(&conn, false).unwrap();
        drop(conn);
        enqueue_startup(&guard, "1.2.3", "macos", "aarch64", "24.6.0");
        let count: i64 = guard.conn.lock().unwrap().query_row("SELECT count(*) FROM telemetry_queue", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn startup_survives_offline_retry_limits_without_changing_launch_or_batch_ids() {
        let db = test_db();
        let before: Vec<String> = {
            let guard = db.lock().unwrap();
            enqueue_startup(&guard, "1.2.3", "darwin", "aarch64", "24.6.0");
            let conn = guard.conn.lock().unwrap();
            conn.execute("UPDATE telemetry_queue SET attempts=5", []).unwrap();
            let mut statement = conn.prepare("SELECT payload FROM telemetry_queue ORDER BY id").unwrap();
            let rows = statement.query_map([], |r| r.get(0)).unwrap();
            rows.map(Result::unwrap).collect()
        };
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}/functions/v1/platform/runjam", server.server_addr());
        let worker = std::thread::spawn(move || {
            for _ in 0..2 {
                let request = server.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
                request.respond(tiny_http::Response::from_string("offline").with_status_code(503)).unwrap();
            }
        });
        flush_database(&db, &ureq::agent(), &base);
        worker.join().unwrap();
        let guard = db.lock().unwrap();
        let conn = guard.conn.lock().unwrap();
        let mut statement = conn.prepare("SELECT payload FROM telemetry_queue ORDER BY id").unwrap();
        let rows = statement.query_map([], |r| r.get::<_,String>(0)).unwrap();
        let after: Vec<String> = rows.map(Result::unwrap).collect();
        assert_eq!(after, before, "offline startup metadata must remain durable until acknowledgement or opt-out");
    }

    #[test]
    fn disabled_flush_sends_only_explicit_legacy_feedback_with_durable_retry_id() {
        let db = test_db();
        {
            let guard = db.lock().unwrap();
            let conn = guard.conn.lock().unwrap();
            set_enabled(&conn, false).unwrap();
            // Stale pre-opt-out snapshot plus feedback submitted by an older release.
            conn.execute("INSERT INTO telemetry_queue(kind,payload,attempts) VALUES('events',?1,0)", [r#"{"events":[{"event_name":"app_started"}]}"#]).unwrap();
            conn.execute("INSERT INTO telemetry_queue(kind,payload,attempts) VALUES('feedback',?1,5)", [r#"{"type":"bug","content":"Legacy draft"}"#]).unwrap();
        }
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}/functions/v1/platform/runjam", server.server_addr());
        let worker = std::thread::spawn(move || {
            let mut ids = Vec::new();
            for status in [503, 201] {
                let mut request = server.recv_timeout(Duration::from_secs(2)).unwrap().expect("feedback request missing");
                assert_eq!(request.url(), "/functions/v1/platform/runjam/feedback");
                let mut text = String::new();
                request.as_reader().read_to_string(&mut text).unwrap();
                let payload: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(payload["content"], "Legacy draft");
                assert_eq!(payload["platform"], crate::commands::telemetry_cmd::platform_name());
                let id = payload["request_id"].as_str().unwrap().to_string();
                assert!(uuid::Uuid::parse_str(&id).is_ok());
                ids.push(id);
                request.respond(tiny_http::Response::from_string(r#"{"ok":true,"id":"2337ab27-69db-42c9-bf68-243017e28b5e"}"#).with_status_code(status)).unwrap();
            }
            assert_eq!(ids[0], ids[1]);
        });
        flush_database(&db, &ureq::agent(), &base);
        let count: i64 = db.lock().unwrap().conn.lock().unwrap().query_row("SELECT count(*) FROM telemetry_queue WHERE kind='feedback'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1, "failed legacy feedback must remain queued");
        flush_database(&db, &ureq::agent(), &base);
        worker.join().unwrap();
        let count: i64 = db.lock().unwrap().conn.lock().unwrap().query_row("SELECT count(*) FROM telemetry_queue", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn rejects_success_status_without_server_acknowledgement() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}/feedback", server.server_addr());
        let worker = std::thread::spawn(move || {
            let request = server.recv().unwrap();
            assert_eq!(request.url(), "/feedback");
            request.respond(tiny_http::Response::from_string(r#"{"ok":false}"#).with_status_code(201)).unwrap();
        });
        let ok = post_json_with(&ureq::agent(), &url, "{}");
        worker.join().unwrap();
        assert!(!ok);
    }

    #[test]
    fn explicit_feedback_requires_a_persisted_id_and_sends_only_the_user_payload() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}/functions/v1/platform/runjam", server.server_addr());
        let payload = serde_json::json!({"request_id":"1352c622-e643-4b55-943d-3bfead7d0bd1",
            "installation_id":"084c70ab-1292-4fed-b93a-ab12e7f14cb0", "type":"feature",
            "content":"My suggestion", "app_version":"1.2.3", "platform":"darwin"});
        let expected = payload.clone();
        let worker = std::thread::spawn(move || {
            for (status, body) in [(201, r#"{"ok":true}"#), (202, r#"{"ok":true,"id":"saved"}"#), (201, r#"{"ok":true,"id":"queued"}"#), (201, r#"{"ok":true,"id":"2337ab27-69db-42c9-bf68-243017e28b5e"}"#)] {
                let mut request = server.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
                assert_eq!(request.url(), "/functions/v1/platform/runjam/feedback");
                assert!(!request.headers().iter().any(|h| h.field.equiv("authorization")));
                let mut body_sent = String::new();
                request.as_reader().read_to_string(&mut body_sent).unwrap();
                assert_eq!(serde_json::from_str::<serde_json::Value>(&body_sent).unwrap(), expected);
                request.respond(tiny_http::Response::from_string(body).with_status_code(status)).unwrap();
            }
        });
        assert!(send_feedback_with(&ureq::agent(), &base, &payload).is_err());
        assert!(send_feedback_with(&ureq::agent(), &base, &payload).is_err());
        assert!(send_feedback_with(&ureq::agent(), &base, &payload).is_err());
        assert_eq!(send_feedback_with(&ureq::agent(), &base, &payload).unwrap()["id"], "2337ab27-69db-42c9-bf68-243017e28b5e");
        worker.join().unwrap();
    }

    #[test]
    fn queued_events_never_include_user_content_or_paths() {
        let payload = serde_json::json!({"events":[{"event_name":"message_sent", "event_time":"2026-10-18T00:00:00Z",
            "event_props":{"count":2,"path":"/private/workspace", "model":"private-model-name", "content":"conversation"}}]});
        let result = queue_body("events", &payload);
        assert!(!result.contains("/private"));
        assert!(!result.contains("conversation"));
        assert!(!result.contains("private-model"));
        assert!(result.contains("\"count\":2"));
    }

    /// Historical endpoint probe retained only for compatibility investigation.
    /// Never run during local validation: it performs a production write.
    #[test]
    #[ignore = "Legacy production-network probe; run local fixture tests instead"]
    fn post_follows_308_redirect_to_www() {
        let agent = ureq::builder().timeout(std::time::Duration::from_secs(15)).build();
        // This legacy route is not used by the client transport.
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
