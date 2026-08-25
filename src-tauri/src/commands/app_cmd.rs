use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use tauri::{LogicalPosition, LogicalSize, WebviewBuilder, WebviewUrl};
#[cfg(target_os = "windows")]
use crate::util::hidden_command;

/// Height (in logical px) occupied by the top nav bar plus the app tab strip.
/// Child webviews hosting quick-launch apps are laid out below this strip, so
/// the top bar and the tab strip always stay visible and clickable above them.
pub const APP_TAB_STRIP_TOP: f64 = 68.0;

const APP_TAB_LABEL_PREFIX: &str = "app-tab-";

/// Labels of app-tab webviews that are currently visible. Hidden tabs are also
/// resized to (0,0) as a belt-and-braces fallback: on some platforms `setHidden`
/// on a child WKWebView is not reliable, so a zero-size webview guarantees the
/// content below it is never covered. `layout_app_tabs` only re-aligns tabs in
/// this set so a hidden tab is never dragged back into view.
static VISIBLE_TABS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Handles to every open app-tab child webview, keyed by label.
///
/// We keep the handles ourselves because resolving child webviews back through
/// Tauri (`WebviewWindow::get_webview`) proved unreliable — on macOS the lookup
/// filters by `window_label()` which does not match child webviews, so hide/
/// close calls were silently no-ops and the website stayed on screen. Holding
/// the `Webview` returned by `add_child` guarantees every operation targets the
/// right webview.
static APP_TABS: LazyLock<Mutex<HashMap<String, tauri::Webview>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Make a label legal for Tauri AND safe to use as a path segment on every
/// platform. Window/webview labels only allow alphanumeric characters, `-`,
/// `/`, `:` and `_`; but `:` and `/` are illegal in Windows file names, and
/// app-tab labels double as data-directory names for persistent cookies — so
/// collapse everything down to `[A-Za-z0-9_-]` here.
fn sanitize_label(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Derive a unique, stable label for an app tab from its URL. The host is
/// sanitized for readability; a short hash of the full URL keeps distinct
/// URLs (e.g. `google.com` vs `google.com/maps`) on distinct tabs.
fn tab_label_for(url: &url::Url) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    url.as_str().hash(&mut hasher);
    let host = url.host_str().unwrap_or("app");
    format!("{}{}-{:08x}", APP_TAB_LABEL_PREFIX, sanitize_label(host), hasher.finish())
}

/// Size (in logical px) an app-tab webview should occupy: the full window minus
/// the top bar + tab strip.
fn child_viewport(window: &tauri::Window) -> Result<(f64, f64), String> {
    let scale = window.scale_factor().unwrap_or(1.0);
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let width = (size.width as f64 / scale).max(1.0);
    let height = (size.height as f64 / scale - APP_TAB_STRIP_TOP).max(1.0);
    Ok((width, height))
}

fn mark_visible(label: &str, visible: bool) {
    let mut set = VISIBLE_TABS.lock().unwrap();
    if visible {
        set.insert(label.to_string());
    } else {
        set.remove(label);
    }
}

/// Return the stored handle for an app-tab webview. Falls back to an app-level
/// lookup for robustness (e.g. handles created before a previous app version).
fn get_tab(label: &str) -> Option<tauri::Webview> {
    if let Some(wv) = APP_TABS.lock().unwrap().get(label).cloned() {
        return Some(wv);
    }
    None
}

fn store_tab(label: &str, webview: &tauri::Webview) {
    APP_TABS.lock().unwrap().insert(label.to_string(), webview.clone());
}

fn remove_tab(label: &str) {
    APP_TABS.lock().unwrap().remove(label);
}

/// Open an external website as a browser-like tab INSIDE the main window. The
/// site is rendered by a child webview positioned below the top bar + app tab
/// strip, so it feels like a tab of the window rather than a separate window.
/// When `tab_id` is supplied the tab is always created fresh (browser-like:
/// the same site can be open in several tabs at once); otherwise an
/// already-open tab for the same URL is shown and focused instead.
#[tauri::command]
pub fn open_app_tab(
    window: tauri::Webview,
    url: String,
    tab_id: Option<String>,
) -> Result<String, String> {
    // Only allow http(s) destinations.
    let parsed = url::Url::parse(url.trim()).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("Unsupported URL scheme: {}", parsed.scheme()));
    }
    // A caller-supplied unique id means "open a brand-new tab" (e.g. repeated
    // clicks on the same app in the sidebar) — use it directly as the label so
    // the URL-based reuse check below can never match an existing tab.
    let label = match tab_id {
        Some(id) if !id.trim().is_empty() => {
            format!("{}{}", APP_TAB_LABEL_PREFIX, sanitize_label(id.trim()))
        }
        _ => tab_label_for(&parsed),
    };

    // Reuse an already-open tab for the same URL.
    if let Some(wv) = get_tab(&label) {
        eprintln!("[app-tab] open: reuse {label}");
        mark_visible(&label, true);
        if let Ok((w, h)) = child_viewport(&window.window()) {
            let _ = wv.set_position(LogicalPosition::new(0.0, APP_TAB_STRIP_TOP));
            let _ = wv.set_size(LogicalSize::new(w, h));
        }
        let _ = wv.show();
        let _ = wv.set_focus();
        return Ok(label);
    }

    // Child webviews keep their bounds when the window resizes, so compute them
    // from the current window size and re-apply on every window resize.
    let (width, height) = child_viewport(&window.window())?;

    let parent = window.window();
    // Give each tab its own on-disk data directory (cookies, localStorage,
    // IndexedDB, ...) so sessions survive closing the app — child webviews use
    // a non-persistent in-memory store by default. Reusing the same directory
    // per label means reopening a tab restores its cookies/state.
    let data_dir = get_app_data_dir().join("app-tabs").join(&label);
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let builder = WebviewBuilder::new(&label, WebviewUrl::External(parsed))
        .data_directory(data_dir);
    let webview = parent
        .add_child(
            builder,
            LogicalPosition::new(0.0, APP_TAB_STRIP_TOP),
            LogicalSize::new(width, height),
        )
        .map_err(|e| e.to_string())?;

    eprintln!("[app-tab] open: created {label}");
    store_tab(&label, &webview);
    mark_visible(&label, true);
    // Ensure the freshly created webview is actually shown and focused. On
    // macOS child webviews default to visible, but on Windows the WebView2
    // controller can start out hidden — make it explicit everywhere.
    let _ = webview.show();
    let _ = webview.set_focus();
    // The new webview was created against the current layout — keep every other
    // open tab aligned as well.
    let _ = layout_app_tabs(window);
    Ok(label)
}

/// Close (destroy) an app tab's child webview.
#[tauri::command]
pub fn close_app_tab(label: String) -> Result<(), String> {
    mark_visible(&label, false);
    match get_tab(&label) {
        Some(wv) => {
            eprintln!("[app-tab] close: {label}");
            remove_tab(&label);
            wv.close().map_err(|e| e.to_string())?;
        }
        None => eprintln!("[app-tab] close: NOT FOUND {label}"),
    }
    Ok(())
}

/// Browser-style navigation on the active app tab: go back / forward in its
/// history, or reload the page.
#[tauri::command]
pub fn app_tab_navigate(label: String, action: String) -> Result<(), String> {
    let wv = get_tab(&label).ok_or_else(|| format!("[app-tab] navigate: NOT FOUND {label}"))?;
    let result = match action.as_str() {
        "back" => wv.eval("window.history.back()"),
        "forward" => wv.eval("window.history.forward()"),
        "reload" => wv.reload(),
        other => return Err(format!("[app-tab] navigate: unknown action '{other}'")),
    };
    eprintln!("[app-tab] navigate: {label} -> {action}");
    result.map_err(|e| format!("[app-tab] navigate {action} failed: {e}"))
}

/// Show/hide an app tab's child webview (tab switching).
///
/// Hiding also shrinks the webview to (0,0): `setHidden` alone is not always
/// honored on macOS child WKWebViews, and a zero-size webview can never cover
/// the RunJam UI below it. Showing restores the full viewport bounds first so
/// the tab appears already laid out (no flash of a 0-sized webview).
#[tauri::command]
pub fn set_app_tab_visible(
    window: tauri::Webview,
    label: String,
    visible: bool,
) -> Result<(), String> {
    match get_tab(&label) {
        Some(wv) => {
            eprintln!("[app-tab] set_visible: {label} -> {visible}");
            if visible {
                mark_visible(&label, true);
                if let Ok((w, h)) = child_viewport(&window.window()) {
                    let _ = wv.set_position(LogicalPosition::new(0.0, APP_TAB_STRIP_TOP));
                    let _ = wv.set_size(LogicalSize::new(w, h));
                }
                let _ = wv.show();
                let _ = wv.set_focus();
            } else {
                mark_visible(&label, false);
                let _ = wv.hide();
                let _ = wv.set_size(LogicalSize::new(0.0, 0.0));
            }
        }
        None => eprintln!("[app-tab] set_visible: NOT FOUND {label} -> {visible}"),
    }
    Ok(())
}

/// Re-align visible app-tab webviews below the top bar + tab strip.
/// Called on window resize so tabs always fill the area under the strip.
/// Hidden tabs keep their (0,0) size and are never moved back into view.
#[tauri::command]
pub fn layout_app_tabs(window: tauri::Webview) -> Result<(), String> {
    let (width, height) = child_viewport(&window.window())?;

    let labels: Vec<String> = {
        let visible = VISIBLE_TABS.lock().unwrap();
        APP_TABS
            .lock()
            .unwrap()
            .keys()
            .filter(|label| visible.contains(*label))
            .cloned()
            .collect()
    };
    for label in labels {
        if let Some(wv) = get_tab(&label) {
            let _ = wv.set_position(LogicalPosition::new(0.0, APP_TAB_STRIP_TOP));
            let _ = wv.set_size(LogicalSize::new(width, height));
        }
    }
    Ok(())
}

fn get_app_data_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "runjam", "RunJam")
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn open_path(path: &PathBuf) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "windows")]
    {
        hidden_command("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub fn get_data_dir() -> String {
    let dir = get_app_data_dir();
    dir.to_string_lossy().to_string()
}

#[tauri::command]
pub fn open_data_dir() -> Result<(), String> {
    let dir = get_app_data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    open_path(&dir)
}

#[tauri::command]
pub fn open_in_finder(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("Path does not exist: {}", path));
    }
    open_path(&p)
}

/// Reveal a path in the system file manager.
/// - For a directory: opens it (same as open_in_finder).
/// - For a file: reveals/selects it in its parent folder (macOS `open -R`,
///   Windows `explorer /select,`). On Linux, falls back to opening the parent
///   directory since there's no standard "reveal" primitive.
#[tauri::command]
pub fn reveal_path(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("Path does not exist: {}", path));
    }
    if p.is_dir() {
        return open_path(&p);
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&p)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        hidden_command("explorer")
            .arg("/select,")
            .arg(&p)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Linux: no standard reveal — open the parent directory instead.
    #[cfg(target_os = "linux")]
    {
        let parent = p.parent().unwrap_or(&p).to_path_buf();
        return open_path(&parent);
    }
}
