use crate::db::connection::Database;
use crate::telemetry;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;
use tauri::State;
use tauri_plugin_updater::UpdaterExt;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryStatus {
    pub installation_id: String,
    pub enabled: bool,
    pub consent_shown: bool,
}

#[tauri::command]
pub fn get_telemetry_status(db: State<'_, Mutex<Database>>) -> TelemetryStatus {
    let guard = db.lock().unwrap();
    let conn = guard.conn.lock().unwrap();
    TelemetryStatus {
        installation_id: telemetry::get_or_create_installation_id(&conn),
        enabled: telemetry::is_enabled(&conn),
        consent_shown: telemetry::consent_shown(&conn),
    }
}

#[tauri::command]
pub fn set_telemetry_enabled(
    app: tauri::AppHandle,
    db: State<'_, Mutex<Database>>,
    enabled: bool,
) -> Result<TelemetryStatus, String> {
    {
        let guard = db.lock().map_err(|e| e.to_string())?;
        {
            let conn = guard.conn.lock().map_err(|e| e.to_string())?;
            telemetry::set_enabled(&conn, enabled).map_err(|e| e.to_string())?;
            telemetry::mark_consent_shown(&conn).map_err(|e| e.to_string())?;
        }
        // Re-register so the server sees the latest consent state immediately.
        // NOTE: conn must be dropped before register/track — those functions
        // lock Database.conn themselves (std::sync::Mutex is not reentrant).
        if enabled {
            let version = app.package_info().version.to_string();
            let platform = platform_name();
            telemetry::register(&guard, &version, platform, std::env::consts::ARCH, "", true);
        }
    }
    telemetry::flush_async(&app);
    Ok(get_telemetry_status(db))
}

#[tauri::command]
pub fn track_event(
    db: State<'_, Mutex<Database>>,
    event_name: String,
    event_props: Option<serde_json::Value>,
) {
    let guard = db.lock().unwrap();
    telemetry::track(&guard, &event_name, event_props.unwrap_or_else(|| serde_json::json!({})));
}

// ── outbound proxy (used for telemetry reporting) ──────────────────────

/// Current outbound proxy URL (empty string = direct connection).
#[tauri::command]
pub fn get_proxy_config(db: State<'_, Mutex<Database>>) -> Result<String, String> {
    let guard = db.lock().map_err(|e| e.to_string())?;
    let conn = guard.conn.lock().map_err(|e| e.to_string())?;
    Ok(telemetry::get_proxy_url(&conn))
}

/// Save the outbound proxy URL (empty string clears it). Applies on the
/// next telemetry flush.
#[tauri::command]
pub fn set_proxy_config(
    app: tauri::AppHandle,
    db: State<'_, Mutex<Database>>,
    proxy: String,
) -> Result<(), String> {
    {
        let guard = db.lock().map_err(|e| e.to_string())?;
        let conn = guard.conn.lock().map_err(|e| e.to_string())?;
        telemetry::set_proxy_url(&conn, proxy.trim()).map_err(|e| e.to_string())?;
    }
    // Kick a flush so the new config is exercised right away (no-op if the
    // queue is empty).
    telemetry::flush_async(&app);
    Ok(())
}

/// Verify the given proxy actually connects (GET https://example.com through
/// it). The telemetry endpoint itself may be unreachable from some networks,
/// so a generic reachable probe is used instead.
#[tauri::command]
pub fn test_proxy(proxy: String) -> Result<(), String> {
    let proxy = proxy.trim();
    if proxy.is_empty() {
        return Err("proxy address is empty".into());
    }
    let p = ureq::Proxy::new(proxy).map_err(|e| format!("invalid proxy: {}", e))?;
    let agent = ureq::builder()
        .timeout(Duration::from_secs(8))
        .proxy(p)
        .build();
    match agent.get("https://example.com").call() {
        Ok(r) if (200..300).contains(&r.status()) => Ok(()),
        Ok(r) => Err(format!("unexpected status: {}", r.status())),
        Err(e) => Err(format!("connect failed: {}", e)),
    }
}

#[tauri::command]
pub fn submit_feedback(
    app: tauri::AppHandle,
    db: State<'_, Mutex<Database>>,
    email: Option<String>,
    feedback_type: String,
    content: String,
    screenshot_url: Option<String>,
) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("feedback content is empty".into());
    }
    let guard = db.lock().map_err(|e| e.to_string())?;
    let conn = guard.conn.lock().map_err(|e| e.to_string())?;
    let installation_id = telemetry::get_or_create_installation_id(&conn);
    drop(conn);
    let payload = serde_json::json!({
        "installation_id": installation_id,
        "email": email,
        "type": feedback_type,
        "content": telemetry::sanitize(&content),
        "screenshot_url": screenshot_url,
        "app_version": app.package_info().version.to_string(),
    });
    telemetry::enqueue_feedback(&guard, &payload);
    drop(guard);
    telemetry::flush_async(&app);
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub update_available: bool,
    pub latest_version: Option<String>,
    pub published_at: Option<String>,
    pub notes: Option<String>,
    pub download_url: Option<String>,
    pub download_urls: Option<serde_json::Value>,
}

/// Raw response from the update API endpoint.
#[derive(Deserialize)]
struct BackendUpdateResponse {
    version: String,
    pub_date: Option<String>,
    notes: Option<String>,
    platforms: std::collections::HashMap<String, BackendPlatformInfo>,
}

#[derive(Deserialize)]
struct BackendPlatformInfo {
    url: String,
    #[allow(dead_code)]
    signature: String,
}

/// Phase 2 seed: update check against the backend (GitHub Releases metadata
/// mirrored in Supabase). The actual download/install goes through the Tauri
/// updater plugin once it is configured with signing keys.
额3xzAa0PO///
/// Routes through the configured outbound proxy (if any), same as telemetry
/// and announcements.
#[tauri::command]
pub async fn check_for_updates(
    app: tauri::AppHandle,
    current: String,
) -> Result<UpdateInfo, String> {
    let base = telemetry::api_base();
    let url = format!(
        "{}/api/updates/latest?platform={}&arch={}&current={}",
        base,
        platform_name(),
        std::env::consts::ARCH,
        current
    );
    let agent = telemetry::build_agent(&app);
    let resp = agent
        .get(&url)
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| format!("update check failed: {}", e))?;
    let backend: BackendUpdateResponse = resp
        .into_json()
        .map_err(|e| format!("bad update response: {}", e))?;

    let update_available = crate::updates::version_ge(&backend.version, &current)
        && backend.version.trim() != current.trim();
    let platform_key = format!("{}-{}", platform_name(), std::env::consts::ARCH);
    let download_url = backend
        .platforms
        .get(&platform_key)
        .map(|p| p.url.trim().to_string());

    Ok(UpdateInfo {
        update_available,
        latest_version: Some(backend.version),
        published_at: backend.pub_date,
        notes: backend.notes,
        download_url,
        download_urls: None,
    })
}

/// Map Rust OS names to the platform strings the backend expects.
pub fn platform_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// Fetch unread announcements (server filters active + min_version by the
/// current app version). Client filters locally-read ones.
#[tauri::command]
pub async fn get_announcements(
    app: tauri::AppHandle,
    db: State<'_, Mutex<Database>>,
) -> Result<Vec<crate::updates::Announcement>, String> {
    let base = telemetry::api_base();
    let version = app.package_info().version.to_string();
    let url = format!("{}/api/announcements?current={}", base, version);
    let agent = telemetry::build_agent(&app);
    let resp = agent
        .get(&url)
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| format!("announcements fetch failed: {}", e))?;
    let items: Vec<crate::updates::Announcement> = resp
        .into_json()
        .map_err(|e| format!("bad announcements response: {}", e))?;

    let guard = db.lock().map_err(|e| e.to_string())?;
    let conn = guard.conn.lock().map_err(|e| e.to_string())?;
    Ok(crate::updates::filter_unread(&conn, items))
}

/// Mark an announcement as read so it is not shown again.
#[tauri::command]
pub fn mark_announcement_read(
    db: State<'_, Mutex<Database>>,
    id: String,
) -> Result<(), String> {
    let guard = db.lock().map_err(|e| e.to_string())?;
    let conn = guard.conn.lock().map_err(|e| e.to_string())?;
    crate::updates::mark_announcement_read(&conn, &id).map_err(|e| e.to_string())
}

/// Unified update check. Windows uses the updater plugin; macOS/Linux use
/// the backend metadata endpoint and return a download URL for redirect.
///
/// The `current` argument is kept only for invoke compatibility with the
/// frontend; the true current version is always sourced from the app's own
/// package metadata so the check never reports a stale hardcoded version.
#[tauri::command]
pub async fn check_update_ui(
    app: tauri::AppHandle,
    current: String,
) -> Result<crate::updates::UpdateCheckResult, String> {
    // The `current` argument is required for invoke compatibility (Tauri v2
    // matches arguments by name), but its value is ignored — the true current
    // version is always sourced from the app's own package metadata so the
    // check never reports a stale hardcoded version.
    let _ = &current;
    if crate::updates::is_windows() {
        let updater = app
            .updater()
            .map_err(|e| format!("updater init failed: {}", e))?;
        match updater.check().await {
            Ok(Some(update)) => Ok(crate::updates::UpdateCheckResult {
                update_available: true,
                action: "install".into(),
                latest_version: Some(update.version.to_string()),
                notes: update.body.clone(),
                download_url: None,
            }),
            Ok(None) => Ok(crate::updates::UpdateCheckResult::none()),
            Err(e) => Err(format!("update check failed: {}", e)),
        }
    } else {
        // macOS/Linux: reuse the existing metadata check, sourcing the
        // current version from the app's own package metadata.
        let info = check_for_updates(app.clone(), app.package_info().version.to_string()).await?;
        Ok(crate::updates::UpdateCheckResult {
            update_available: info.update_available,
            action: "open_download".into(),
            latest_version: info.latest_version,
            notes: info.notes,
            download_url: info.download_url,
        })
    }
}

/// Windows: trigger download + install of the pending update.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| format!("updater init failed: {}", e))?;
    if let Some(update) = updater.check().await.map_err(|e| format!("update check failed: {}", e))? {
        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|e| format!("install failed: {}", e))?;
    }
    Ok(())
}
