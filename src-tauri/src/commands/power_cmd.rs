//! Power management: keep the machine awake while an agent is running.
//!
//! When the user enables this in Settings, a running session holds an idle-sleep
//! assertion so closing the laptop lid (or simply walking away) does not freeze
//! the agent mid-task.
//!
//! `caffeinate -i` is used rather than a native IOKit power assertion because it
//! is a small macOS-maintained system binary that holds exactly the "prevent
//! idle system sleep" assertion we want. `-i` deliberately does NOT keep the
//! display on: the screen may still dim or turn off, which is what someone
//! closing a laptop expects — only the CPU keeps working.
//!
//! macOS-only; the commands are inert elsewhere.

/// The running `caffeinate` child, if we started one.
#[cfg(target_os = "macos")]
static SLEEP_ASSERTION: std::sync::Mutex<Option<std::process::Child>> =
    std::sync::Mutex::new(None);

/// Start or stop the idle-sleep assertion.
///
/// Idempotent: repeating the same value is a no-op, so the frontend can call it
/// liberally (e.g. whenever the running-session count crosses zero).
#[tauri::command]
pub fn set_prevent_sleep(enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut guard = SLEEP_ASSERTION.lock().map_err(|e| e.to_string())?;

        if !enabled {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Ok(());
        }

        if guard.is_some() {
            return Ok(()); // already asserting
        }

        let child = std::process::Command::new("caffeinate")
            // `-i` : prevent idle SYSTEM sleep. Deliberately not `-d`, so the
            //        display may still turn off (what closing a lid implies).
            // `-w <pid>` : exit as soon as this app is gone. Without it a crash
            //        or `kill -9` would leave `caffeinate` orphaned and keep the
            //        machine awake forever. Verified: it self-terminates when the
            //        watched pid disappears.
            .arg("-i")
            .arg("-w")
            .arg(std::process::id().to_string())
            .spawn()
            .map_err(|e| format!("failed to start caffeinate: {e}"))?;
        *guard = Some(child);
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = enabled;
        Err("prevent-sleep is only implemented on macOS".into())
    }
}

/// Whether the assertion is currently held (for the settings UI).
#[tauri::command]
pub fn prevent_sleep_active() -> bool {
    #[cfg(target_os = "macos")]
    {
        SLEEP_ASSERTION.lock().map(|g| g.is_some()).unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Drop the assertion if held. Called on real app exit so a stray `caffeinate`
/// never outlives the app.
pub fn release_sleep_assertion() {
    #[cfg(target_os = "macos")]
    {
        if let Ok(mut guard) = SLEEP_ASSERTION.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}