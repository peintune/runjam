//! Desktop pet: a small always-on-top launcher icon in the bottom-right corner
//! of the screen, plus a lightweight Q&A popup it opens.
//!
//! Both are *top-level* windows (not child webviews like app-tabs) because the
//! pet must float above every other application, not just above the RunJam UI.
//!
//! Window roles:
//! - `pet-icon` — 56x56 transparent, circular icon. Never focused, never in the
//!   taskbar, always on top. Clicking it toggles the chat popup; dragging it
//!   moves it anywhere on screen.
//! - `pet-chat` — 460x560 Q&A panel anchored just to the LEFT of the icon (so it
//!   never covers the button that closes it). Created lazily on first click,
//!   then shown/hidden afterwards.
//!
//! Both load the same frontend bundle with a `window=` query parameter so
//! `main.ts` can mount a dedicated lightweight component instead of the full
//! app shell.

use tauri::{
    AppHandle, Emitter, LogicalPosition, Manager, Monitor, WebviewUrl, WebviewWindowBuilder,
};

pub const PET_ICON_LABEL: &str = "pet-icon";
pub const PET_CHAT_LABEL: &str = "pet-chat";

/// Icon edge length in logical px. Kept a little larger than the visible logo
/// (36px) so there is a transparent rim that the user can grab to drag it.
const ICON_SIZE: f64 = 44.0;
/// Extra lift above the work area's bottom edge. The work area already clears a
/// visible Dock, but it does NOT when the Dock is set to auto-hide — so nudge
/// the icon up a little further to stay comfortably clear of it either way.
const ICON_LIFT: f64 = 28.0;
/// Gap between the icon and the screen edges.
const ICON_MARGIN: f64 = 18.0;
/// Chat popup dimensions.
const CHAT_WIDTH: f64 = 460.0;
const CHAT_HEIGHT: f64 = 560.0;
/// Smallest size the popup may be resized to. The toolbar needs roughly this
/// width to stay on one line, so going narrower would just clip it.
const CHAT_MIN_WIDTH: f64 = 360.0;
const CHAT_MIN_HEIGHT: f64 = 320.0;
/// Gap between the chat popup and the launcher icon.
const CHAT_GAP: f64 = 12.0;
/// Gap between the chat popup and the screen edges.
const CHAT_MARGIN: f64 = 18.0;

/// Resolve the monitor the cursor / main window is on, falling back to the
/// primary monitor. Used to anchor the pet to the correct display's corner.
fn target_monitor(app: &AppHandle) -> Option<Monitor> {
    // Prefer the monitor under the main window, so on multi-display setups the
    // pet lands on the display the user is actually working on.
    if let Some(main) = app.get_webview_window("main") {
        if let Ok(Some(monitor)) = main.current_monitor() {
            return Some(monitor);
        }
    }
    app.primary_monitor().ok().flatten()
}

/// Bottom-right corner position (logical) for a window of the given size.
///
/// Anchored to the monitor's WORK AREA rather than its full frame: the work
/// area excludes the Dock and the menu bar, so the icon lands on visible
/// desktop instead of underneath the Dock.
fn corner_position(app: &AppHandle, width: f64, height: f64, margin: f64) -> LogicalPosition<f64> {
    let Some(monitor) = target_monitor(app) else {
        return LogicalPosition::new(0.0, 0.0);
    };
    let scale = monitor.scale_factor();
    // Monitor geometry is physical; convert to logical before layout.
    let area = monitor.work_area();
    let logical_w = area.size.width as f64 / scale;
    let logical_h = area.size.height as f64 / scale;
    let logical_x = area.position.x as f64 / scale;
    let logical_y = area.position.y as f64 / scale;
    LogicalPosition::new(
        logical_x + logical_w - width - margin,
        logical_y + logical_h - height - margin - ICON_LIFT,
    )
}

/// Position for the chat popup: immediately to the LEFT of the launcher icon and
/// bottom-aligned with it.
///
/// The popup deliberately does NOT overlap the icon: the icon is the toggle for
/// this window, so covering it would leave the user with no way to close the
/// popup, and would also hide the app's only persistent affordance.
fn left_of_icon_position(app: &AppHandle) -> LogicalPosition<f64> {
    let Some(monitor) = target_monitor(app) else {
        return LogicalPosition::new(0.0, 0.0);
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let logical_w = area.size.width as f64 / scale;
    let logical_h = area.size.height as f64 / scale;
    let logical_x = area.position.x as f64 / scale;
    let logical_y = area.position.y as f64 / scale;

    // Anchor to where the icon ACTUALLY is — the user can drag it anywhere.
    // Falls back to the default bottom-right corner if it cannot be read.
    let (icon_x, icon_y) = app
        .get_webview_window(PET_ICON_LABEL)
        .and_then(|icon| {
            let p = icon.outer_position().ok()?;
            let icon_scale = icon.scale_factor().ok()?;
            if icon_scale <= 0.0 {
                return None;
            }
            Some((p.x as f64 / icon_scale, p.y as f64 / icon_scale))
        })
        .unwrap_or((
            logical_x + logical_w - ICON_SIZE - ICON_MARGIN,
            logical_y + logical_h - ICON_SIZE - ICON_MARGIN - ICON_LIFT,
        ));

    // Share the icon's bottom edge so the two read as one unit.
    let mut y = icon_y + ICON_SIZE - CHAT_HEIGHT;

    // Prefer the icon's left (the icon normally lives in the bottom-right
    // corner). If the user dragged the icon so far left that the popup would
    // not fit, flip to its right — otherwise the popup would be clamped back
    // onto the icon and cover the very button that toggles it.
    let mut x = icon_x - CHAT_GAP - CHAT_WIDTH;
    if x < logical_x + CHAT_MARGIN {
        x = icon_x + ICON_SIZE + CHAT_GAP;
    }

    // Still no room on either side (tiny display): keep it fully on-screen
    // rather than sliding it off the edge (the window size itself is fixed).
    let left_limit = logical_x + logical_w - CHAT_WIDTH - CHAT_MARGIN;
    x = x.clamp(logical_x + CHAT_MARGIN, left_limit.max(logical_x + CHAT_MARGIN));
    y = y.max(logical_y + CHAT_MARGIN);
    LogicalPosition::new(x, y)
}

/// `NSWindowCollectionBehaviorCanJoinAllApplications` only exists on macOS 13+.
#[cfg(target_os = "macos")]
fn supports_join_all_applications() -> bool {
    // SAFETY: plain read of the global AppKit version number, which AppKit sets
    // before any app code runs.
    unsafe { objc2_app_kit::NSAppKitVersionNumber >= objc2_app_kit::NSAppKitVersionNumber13_0 }
}

/// Make a pet window visible over *other applications'* native full-screen
/// spaces.
///
/// Two things are needed, and Tauri/tao gives us neither:
///
/// 1. **Panel-ness.** A plain `NSWindow` is not allowed into the Space another
///    application owns when it goes full screen, no matter which
///    `collectionBehavior` flags are set — `FullScreenAuxiliary`'s documented
///    meaning is "can be shown WITH the fullscreen window", i.e. alongside *our
///    own* full-screen window. Electron (its `type: "panel"` windows),
///    `tauri-nspanel` (Cap / EcoPaste / Overlayed / the BongoCat desktop pet)
///    all solve this the same way: swizzle the live `NSWindow`'s class to an
///    `NSPanel` subclass. We do the same (`attach_panel_class`).
/// 2. **`CanJoinAllApplications`** (macOS 13+), Apple's documented flag for
///    "allowing it to join OTHER APPS' sets and full screen spaces … commonly
///    used for floating windows and system overlays". On older systems
///    `FullScreenAuxiliary` remains the best available fallback.
///
/// Must run on the main thread (AppKit is main-thread-only).
#[cfg(target_os = "macos")]
fn elevate_over_fullscreen(window: &tauri::WebviewWindow) -> Result<(), String> {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior as B, NSWindowStyleMask};

    let ptr = window
        .ns_window()
        .map_err(|e| format!("[pet] no ns_window handle: {e}"))? as *mut NSWindow;
    if ptr.is_null() {
        return Err("[pet] ns_window handle was null".into());
    }
    // AppKit is main-thread-only; bail loudly rather than mutate off-thread.
    if objc2::MainThreadMarker::new().is_none() {
        return Err("[pet] elevate_over_fullscreen must run on the main thread".into());
    }

    // SAFETY: `ns_window()` returns the live `NSWindow` backing this webview
    // window, which outlives the call; we verified we are on the main thread.
    let window_ref = unsafe { &*ptr };

    // Turn the window into a panel (idempotent). This is the part that decides
    // whether the window may enter ANOTHER app's full-screen space at all, but
    // it can legitimately fail (layout mismatch). Treat it as best-effort so a
    // failure still leaves us with the collection-behavior fallback below rather
    // than aborting the whole elevation.
    if let Err(e) = attach_panel_class(window_ref) {
        eprintln!("[pet] panel conversion skipped: {e}");
    }

    // Make the window "eligible" for other apps' full-screen spaces.
    //
    // Apple documents `CanJoinAllApplications` as letting a window join other
    // apps' spaces "when eligible" — and the eligibility condition is that the
    // window carries the non-activating panel style mask. AppKit reserves this
    // mask for NSPanel, which is why it can only be applied *after* the class
    // swizzle above. Electron does exactly this in its own NSPanel subclass
    // ("The Nonactivating mask is reserved for NSPanel, but we can use this
    // workaround to add it at runtime"), as does Overlayed.
    let original_mask = window_ref.styleMask();
    let wanted_mask = original_mask | NSWindowStyleMask::NonactivatingPanel;
    if !original_mask.contains(NSWindowStyleMask::NonactivatingPanel) {
        // Changing `styleMask` resizes the window (borderless panels lose/add
        // the titlebar box), so pin the frame and restore it afterwards.
        let frame = window_ref.frame();
        window_ref.setStyleMask(wanted_mask);
        window_ref.setFrame_display(frame, true);
    }
    // Only complain when the mask did NOT take: that would mean the popup
    // cannot join other apps' full-screen spaces, which is worth surfacing.
    let applied = window_ref.styleMask();
    if !applied.contains(NSWindowStyleMask::NonactivatingPanel) {
        eprintln!(
            "[pet] WARNING: NonactivatingPanel mask did not stick (styleMask = {:#x})",
            applied.bits(),
        );
    }

    // Belt and braces: a window that hides when the app is deactivated would
    // vanish the moment the user works in another (full-screen) application.
    window_ref.setHidesOnDeactivate(false);

    let mut behavior = window_ref.collectionBehavior();

    // `Primary` / `Auxiliary` / `CanJoinAllApplications` are documented as
    // mutually exclusive ("specify at most one"), so clear them before choosing.
    behavior.remove(B::Primary | B::Auxiliary | B::CanJoinAllApplications);
    behavior.insert(B::CanJoinAllSpaces);
    if supports_join_all_applications() {
        behavior.insert(B::CanJoinAllApplications);
        // `FullScreenAuxiliary` and `CanJoinAllApplications` express competing
        // ideas about which full-screen space the window belongs to; prefer the
        // documented cross-app one when the OS supports it.
        behavior.remove(B::FullScreenAuxiliary);
    } else {
        behavior.insert(B::FullScreenAuxiliary);
    }
    window_ref.setCollectionBehavior(behavior);

    // A window that is already on screen is re-ordered so AppKit re-evaluates
    // which Space it belongs to with the new behavior (a hidden window is left
    // hidden — `orderFrontRegardless` would otherwise reveal it early).
    if window_ref.isVisible() {
        window_ref.orderFrontRegardless();
    }
    Ok(())
}

// ── NSPanel swizzling ──────────────────────────────────────────────────────
//
// `object_setClass` requires the old and new classes to have the SAME instance
// layout (`instance_size`), otherwise it is undefined behaviour. tao creates
// the window as its own `TaoWindow` subclass of `NSWindow` which adds a
// `focusable` ivar and overrides `canBecomeKeyWindow` / `canBecomeMainWindow`,
// so our panel subclass must reproduce that ivar and those overrides rather
// than being a bare `NSPanel`.
#[cfg(target_os = "macos")]
mod panel_class {
    use objc2::runtime::{AnyClass, Bool, ClassBuilder, Ivar, Sel};
    use objc2::{sel, ClassType};
    use objc2_app_kit::{NSPanel, NSWindow};
    use std::ffi::CStr;
    use std::sync::OnceLock;

    /// Unique runtime name for the swizzled class.
    const CLASS_NAME: &CStr = c"RunJamPetPanel";

    static PANEL_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();

    /// Mirrors tao's `is_focusable`, which returns the `focusable` ivar.
    ///
    /// Signature matches AppKit's `-[NSWindow canBecomeKeyWindow]` /
    /// `canBecomeMainWindow`: both return `BOOL` and take no arguments.
    pub(super) extern "C-unwind" fn can_become_key_window(this: &NSWindow, _: Sel) -> Bool {
        let Some(ivar) = focusable_ivar() else {
            // Unreachable: the ivar is declared at registration time. Returning
            // NO preserves tao's "not focusable" default rather than panicking
            // inside an Objective-C method.
            return Bool::NO;
        };
        // SAFETY: `this` is a live window of our class, which declares this
        // ivar with exactly the `Bool` type.
        unsafe { *ivar.load::<Bool>(this) }
    }

    /// The `focusable` ivar as registered on our class.
    pub(super) fn focusable_ivar() -> Option<&'static Ivar> {
        get().instance_variable(c"focusable")
    }

    /// Register (once) a `NSPanel` subclass with tao's layout + overrides.
    pub(super) fn get() -> &'static AnyClass {
        PANEL_CLASS.get_or_init(|| {
            let superclass = NSPanel::class();
            let mut builder = ClassBuilder::new(CLASS_NAME, superclass)
                .expect("failed to allocate RunJamPetPanel");

            // Reproduce tao's `focusable: Bool` ivar so `instance_size` matches
            // the window we are about to re-class, and so tao's own
            // `get_ivar("focusable")` reads/writes keep working.
            builder.add_ivar::<Bool>(c"focusable");

            // SAFETY: signatures match AppKit's `-[NSWindow canBecomeKeyWindow]`
            // (returns BOOL, takes no arguments).
            unsafe {
                builder.add_method(
                    sel!(canBecomeKeyWindow),
                    can_become_key_window as extern "C-unwind" fn(_, _) -> _,
                );
                builder.add_method(
                    sel!(canBecomeMainWindow),
                    can_become_key_window as extern "C-unwind" fn(_, _) -> _,
                );
            }

            builder.register()
        })
    }
}

/// Re-class a live window to our `NSPanel` subclass. Idempotent.
///
/// `object_setClass` needs identical instance sizes; tao's `TaoWindow` carries a
/// trailing `focusable` ivar, so seeding that ivar on our class keeps the sizes
/// equal (and lets us carry the current value across).
#[cfg(target_os = "macos")]
fn attach_panel_class(window: &objc2_app_kit::NSWindow) -> Result<(), String> {
    let new_class = panel_class::get();
    let old_class = window.class();

    if old_class == new_class {
        return Ok(()); // already swizzled
    }

    let (old_size, new_size) = (old_class.instance_size(), new_class.instance_size());
    if old_size != new_size {
        // Writing the class anyway would be UB; refuse and keep the window
        // working (it simply stays a plain NSWindow).
        return Err(format!(
            "instance size mismatch: {} is {old_size}, {} is {new_size}",
            old_class.name().to_string_lossy(),
            new_class.name().to_string_lossy(),
        ));
    }

    // Carry tao's `focusable` value across the re-class. Both classes place the
    // ivar at the same offset (they have identical layouts, checked above), but
    // reading it BEFORE the swap keeps this correct even if that ever changes.
    let focusable = unsafe {
        match old_class.instance_variable(c"focusable") {
            Some(ivar) => *ivar.load::<objc2::runtime::Bool>(window),
            None => objc2::runtime::Bool::NO,
        }
    };

    // SAFETY: we just established that both classes share the same instance
    // layout, which is `object_setClass`'s soundness requirement.
    unsafe {
        use objc2::runtime::AnyObject;
        let obj: &AnyObject = &*(window as *const _ as *const AnyObject);
        AnyObject::set_class(obj, new_class);
    }

    // Re-write the ivar through the NEW class's accessor so the value is
    // well-defined under the new type. We go through the raw pointer returned by
    // `load_ptr` (rather than `load_mut`) because casting an AppKit-owned `&T`
    // into `&mut T` would itself be undefined behaviour.
    if let Some(ivar) = panel_class::focusable_ivar() {
        // SAFETY: `window` is a live window owned by AppKit; the ivar belongs to
        // the class we just installed and has exactly the `Bool` type. We are on
        // the main thread and no other reference to that ivar is alive.
        unsafe {
            let obj: &objc2::runtime::AnyObject =
                &*(window as *const _ as *const objc2::runtime::AnyObject);
            *ivar.load_ptr::<objc2::runtime::Bool>(obj) = focusable;
        }
    }
    Ok(())
}

/// Apply [`elevate_over_fullscreen`] on the main thread, from any caller.
///
/// If we are *already* on the main thread (the usual case — `ensure_pet_*` runs
/// from `setup` or from a command handler) the work is done synchronously:
/// deferring it through `run_on_main_thread` can land after the window has
/// already been ordered in, which is too late for AppKit to place it in the
/// right Space.
///
/// Every path logs under `[pet]` so a silent no-op can be told apart from a
/// successful elevation in the dev console.
#[cfg(target_os = "macos")]
fn elevate_over_fullscreen_on_main(app: &AppHandle, label: &str) {
    if objc2::MainThreadMarker::new().is_some() {
        elevate_now(app, label);
        return;
    }

    let app_handle = app.clone();
    let label = label.to_string();
    let label_for_err = label.clone();
    if let Err(e) = app.run_on_main_thread(move || elevate_now(&app_handle, &label)) {
        eprintln!("[pet] {label_for_err}: run_on_main_thread failed: {e}");
    }
}

/// Shared body of [`elevate_over_fullscreen_on_main`]; callers must be on the
/// main thread.
#[cfg(target_os = "macos")]
fn elevate_now(app: &AppHandle, label: &str) {
    let Some(window) = app.get_webview_window(label) else {
        eprintln!("[pet] {label}: window not found for elevation");
        return;
    };
    // Silent on success — only a failure is worth reporting.
    if let Err(e) = elevate_over_fullscreen(&window) {
        eprintln!("[pet] {label}: elevation failed: {e}");
    }
}

/// No-op off macOS (Windows/Linux have no equivalent "full screen space").
#[cfg(not(target_os = "macos"))]
fn elevate_over_fullscreen_on_main(_app: &AppHandle, _label: &str) {}

/// Create the always-on-top launcher icon. Idempotent: if it already exists the
/// window is just re-positioned (e.g. after a monitor change).
pub fn ensure_pet_icon(app: &AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window(PET_ICON_LABEL) {
        let _ = existing.set_position(corner_position(app, ICON_SIZE, ICON_SIZE, ICON_MARGIN));
        // Re-assert the cross-application space behaviour (idempotent) so a
        // re-created/toggled icon is not left with only tao's defaults.
        elevate_over_fullscreen_on_main(app, PET_ICON_LABEL);
        return Ok(());
    }

    let pos = corner_position(app, ICON_SIZE, ICON_SIZE, ICON_MARGIN);
    let mut builder = WebviewWindowBuilder::new(
        app,
        PET_ICON_LABEL,
        WebviewUrl::App("index.html?window=pet-icon".into()),
    )
    .title("RunJam Pet")
    .inner_size(ICON_SIZE, ICON_SIZE)
    .position(pos.x, pos.y)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    // macOS: an unfocused (non-key) window normally swallows the first click
    // just to become key, so one click on the icon would do nothing and the
    // user would have to click twice. `accept_first_mouse` delivers that first
    // click to the button. It is a no-op on Windows/Linux.
    .accept_first_mouse(true)
    .focused(false);

    // macOS: float above full-screen apps and appear on every Space, otherwise
    // the icon disappears the moment the user switches to a full-screen window.
    #[cfg(target_os = "macos")]
    {
        builder = builder
            .visible_on_all_workspaces(true);
    }

    builder
        .build()
        .map_err(|e| format!("[pet] failed to create icon window: {e}"))?;
    // The builder flag only covers `CanJoinAllSpaces`; add the auxiliary layer
    // so the icon also survives another app being full screen.
    elevate_over_fullscreen_on_main(app, PET_ICON_LABEL);
    Ok(())
}

/// Create the chat popup (hidden). Idempotent.
fn ensure_pet_chat(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(existing) = app.get_webview_window(PET_CHAT_LABEL) {
        elevate_over_fullscreen_on_main(app, PET_CHAT_LABEL);
        return Ok(existing);
    }

    let pos = left_of_icon_position(app);
    let mut builder = WebviewWindowBuilder::new(
        app,
        PET_CHAT_LABEL,
        WebviewUrl::App("index.html?window=pet-chat".into()),
    )
    .title("RunJam Pet Chat")
    .inner_size(CHAT_WIDTH, CHAT_HEIGHT)
    .min_inner_size(CHAT_MIN_WIDTH, CHAT_MIN_HEIGHT)
    .position(pos.x, pos.y)
    // Resizable: the popup is a real work surface now (toolbar + transcript),
    // and a long answer needs more room than the 460x560 default. The user
    // drags the edges — no decorations needed for that on macOS/Windows.
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false);

    #[cfg(target_os = "macos")]
    {
        builder = builder.visible_on_all_workspaces(true);
    }

    let window = builder
        .build()
        .map_err(|e| format!("[pet] failed to create chat window: {e}"))?;
    // Same auxiliary-layer treatment as the icon, so the popup stays visible
    // when another app is full screen.
    elevate_over_fullscreen_on_main(app, PET_CHAT_LABEL);
    Ok(window)
}

/// Show the chat popup, creating it on first use. Repositions it to the right
/// edge, then tells the frontend to focus its input so the user can type
/// immediately.
///
/// The window is only ever *hidden*, never destroyed, so hiding/showing is the
/// fast path — mirrors `close_pet_chat`.
fn show_pet_chat(app: &AppHandle) -> Result<(), String> {
    let window = ensure_pet_chat(app)?;
    let pos = left_of_icon_position(app);
    let _ = window.set_position(pos);
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.emit("pet:opened", ());
    Ok(())
}

/// Toggle the chat popup from the launcher icon: a second click on the icon
/// closes the popup instead of re-focusing it. Such a gesture is only
/// meaningful while the popup is actually on screen, so anything other than
/// "visible" is treated as "open it".
#[tauri::command]
pub async fn toggle_pet_chat(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(PET_CHAT_LABEL) {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
            return Ok(());
        }
    }
    show_pet_chat(&app)
}

/// Show and focus the chat popup (explicit "open", e.g. from the main window).
#[tauri::command]
pub async fn open_pet_chat(app: AppHandle) -> Result<(), String> {
    show_pet_chat(&app)
}

/// Hide the chat popup WITHOUT destroying it: the underlying session keeps
/// running so reopening within the idle window continues the same conversation.
#[tauri::command]
pub async fn close_pet_chat(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(PET_CHAT_LABEL) {
        let _ = window.hide();
    }
    Ok(())
}

/// Bring the main window to the front (un-minimise + focus).
///
/// The pet popup runs in its own webview, which cannot focus another window, so
/// the "open in main window" hand-off goes through the backend. The frontend
/// then emits the session id to switch to.
#[tauri::command]
pub async fn focus_main_window(app: AppHandle) -> Result<(), String> {
    let Some(main) = app.get_webview_window("main") else {
        return Err("[pet] main window not found".into());
    };
    if main.is_minimized().unwrap_or(false) {
        let _ = main.unminimize();
    }
    let _ = main.show();
    let _ = main.set_focus();
    Ok(())
}

/// Toggle the pet system on/off. Used by the settings page.
#[tauri::command]
pub async fn set_pet_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    // Persist the preference so it survives a restart.
    {
        let state = app.state::<std::sync::Mutex<crate::state::AppState>>();
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        guard.pet_enabled = enabled;
        guard.save();
    }

    if enabled {
        ensure_pet_icon(&app)
    } else {
        if let Some(chat) = app.get_webview_window(PET_CHAT_LABEL) {
            let _ = chat.close();
        }
        if let Some(icon) = app.get_webview_window(PET_ICON_LABEL) {
            let _ = icon.close();
        }
        Ok(())
    }
}

/// Whether the pet icon window currently exists.
#[tauri::command]
pub fn pet_enabled(app: AppHandle) -> bool {
    app.get_webview_window(PET_ICON_LABEL).is_some()
}

/// Make the chat popup draggable by its header (the window has no decorations).
#[tauri::command]
pub async fn start_pet_drag(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(PET_CHAT_LABEL) {
        let _ = window.start_dragging();
    }
    Ok(())
}