use crate::db::connection::Database;
use crate::telemetry;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;
use tauri::State;

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
        let conn = guard.conn.lock().map_err(|e| e.to_string())?;
        telemetry::set_enabled(&conn, enabled).map_err(|e| e.to_string())?;
        telemetry::mark_consent_shown(&conn).map_err(|e| e.to_string())?;
        // Re-register so the server sees the latest consent state immediately.
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

/// Phase 2 seed: update check against the backend (GitHub Releases metadata
/// mirrored in Supabase). The actual download/install goes through the Tauri
/// updater plugin once it is configured with signing keys.
#[tauri::command]
pub async fn check_for_updates(current: String) -> Result<UpdateInfo, String> {
    let base = telemetry::api_base();
    let url = format!(
        "{}/api/updates/latest?platform={}&arch={}&current={}",
        base,
        platform_name(),
        std::env::consts::ARCH,
        current
    );
    let resp = ureq::get(&url)
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| format!("update check failed: {}", e))?;
    resp.into_json::<UpdateInfo>().map_err(|e| format!("bad update response: {}", e))
}

/// Map Rust OS names to the platform strings the backend expects.
pub fn platform_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}
