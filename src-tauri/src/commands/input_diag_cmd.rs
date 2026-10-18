//! Read-only input diagnostics, written locally without any entered content.

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputDiagnostic {
    stage: DiagnosticStage,
    target: DiagnosticTarget,
    disabled: bool,
    read_only: bool,
    document_focused: bool,
    target_active: bool,
    visible: bool,
    key_events: u32,
    before_input_events: u32,
    input_events: u32,
    elapsed_ms: u32,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum DiagnosticStage {
    AfterClick,
    AfterWait,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum DiagnosticTarget {
    Input,
    Textarea,
    Contenteditable,
}

#[tauri::command]
pub async fn report_input_diagnostic(
    window: tauri::WebviewWindow,
    snapshot: InputDiagnostic,
) -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Ok(());
    }
    if !matches!(window.label(), "main" | "pet-chat") {
        return Ok(());
    }
    let label = window.label().to_string();
    let native_window = window.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    window.run_on_main_thread(move || {
        let focused = native_window.is_focused().unwrap_or(false);
        #[cfg(target_os = "macos")]
        let native = {
            let marker = objc2::MainThreadMarker::new().expect("main-thread callback");
            let app = objc2_app_kit::NSApplication::sharedApplication(marker);
            native_window.ns_window().ok().map(|ptr| {
                // SAFETY: Tauri owns this live window and this callback runs on the main thread.
                let ns = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
                serde_json::json!({
                    "focused": focused,
                    "appActive": app.isActive(),
                    "keyWindow": app.keyWindow().map(|w| w.windowNumber()),
                    "window": ns.windowNumber(),
                    "isMain": ns.isMainWindow(),
                    "responder": ns.firstResponder().map(|r| r.class().name().to_string_lossy().into_owned()),
                })
            }).unwrap_or_else(|| serde_json::json!({"focused": focused}))
        };
        #[cfg(not(target_os = "macos"))]
        let native = serde_json::json!({"focused": focused});
        let _ = send.send(native);
    }).map_err(|e| e.to_string())?;
    let native = receive.await.map_err(|e| e.to_string())?;
    // File/stdout I/O belongs on a worker, never in the native event loop.
    tauri::async_runtime::spawn_blocking(move || {
        crate::rjlog!(
            "[INPUT DIAG] window={} dom={} native={}",
            label,
            serde_json::to_string(&snapshot).unwrap_or_default(),
            native
        );
    })
    .await
    .map_err(|e| e.to_string())
}
