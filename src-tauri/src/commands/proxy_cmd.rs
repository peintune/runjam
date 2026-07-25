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
pub fn set_reasoning_disabled(
    state: tauri::State<'_, Arc<Mutex<ProxyState>>>,
    disabled: bool,
) {
    state.lock().unwrap().reasoning_disabled = disabled;
}

pub fn init_proxy(state: Arc<Mutex<ProxyState>>) {
    match proxy::start_proxy(state) {
        Ok(port) => rjlog!("[proxy] started on port {}", port),
        Err(e) => rjlog!("[proxy] failed to start: {}", e),
    }
}
