use crate::proxy::{self, ProxyState};
use crate::rjlog;
use std::sync::{Arc, Mutex};

#[tauri::command]
pub fn get_proxy_port(state: tauri::State<'_, Arc<Mutex<ProxyState>>>) -> u16 {
    state.lock().unwrap().port
}

#[tauri::command]
pub fn get_proxy_url(state: tauri::State<'_, Arc<Mutex<ProxyState>>>) -> String {
    let port = state.lock().unwrap().port;
    if port > 0 {
        format!("http://127.0.0.1:{}", port)
    } else {
        String::new()
    }
}

#[tauri::command]
pub async fn set_reasoning_disabled<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, Arc<Mutex<ProxyState>>>,
    disabled: bool,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(id) = session_id {
        let state = state.inner().clone();
        return super::session_cmd::run_session_task(app, move |manager| {
            if let Some(info) = manager.runtime_info(&id) {
                let mut proxy = state.lock().map_err(|e| e.to_string())?;
                if let Some(route) = proxy.session_routes.get_mut(&info.run_id) {
                    route.reasoning_disabled = disabled;
                }
            }
            Ok(())
        }).await?;
    }
    state.lock().map_err(|e| e.to_string())?.reasoning_disabled = disabled;
    Ok(())
}

pub fn init_proxy(state: Arc<Mutex<ProxyState>>) {
    match proxy::start_proxy(state) {
        Ok(port) => rjlog!("[proxy] started on port {}", port),
        Err(e) => rjlog!("[proxy] failed to start: {}", e),
    }
}
