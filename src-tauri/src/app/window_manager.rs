use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSScreen, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use crate::app_state::SettingsState;
use crate::database::DbState;
use crate::global_state::*;
use crate::infrastructure::repository::settings_repo::SettingsRepository;

// ---------------------------------------------------------------------------
// Devtools
// ---------------------------------------------------------------------------

#[cfg(feature = "devtools")]
static DEVTOOLS_OPENED_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "devtools")]
pub fn maybe_open_devtools(window: &WebviewWindow) {
    let wanted = std::env::var("COPY_PASTE_AUTO_OPEN_DEVTOOLS")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);
    if wanted && !DEVTOOLS_OPENED_ONCE.swap(true, Ordering::Relaxed) {
        window.open_devtools();
    }
}

#[cfg(not(feature = "devtools"))]
pub fn maybe_open_devtools(_window: &WebviewWindow) {}

// ---------------------------------------------------------------------------
// Liquid glass
// ---------------------------------------------------------------------------

/// Matches `--window-radius` in src/styles/glass.css.
const WINDOW_CORNER_RADIUS: f64 = 18.0;

/// The level the native material currently shows; -1 = not applied yet.
static APPLIED_GLASS_LEVEL: AtomicI64 = AtomicI64::new(-1);

/// Up to this level the system's see-through Clear style is used; above it
/// Regular, with a white tint that grows towards 100.
const GLASS_CLEAR_MAX_LEVEL: u64 = 24;
const GLASS_MAX_TINT_ALPHA: f64 = 0.72;

/// `level` 0–100 → (uses the Clear style, white tint alpha 0–255).
pub(crate) fn glass_appearance(level: u64) -> (bool, u8) {
    let level = level.min(100);
    if level <= GLASS_CLEAR_MAX_LEVEL {
        return (true, 0);
    }
    let span = (100 - GLASS_CLEAR_MAX_LEVEL) as f64;
    let t = (level - GLASS_CLEAR_MAX_LEVEL) as f64 / span;
    (false, (t * GLASS_MAX_TINT_ALPHA * 255.0).round() as u8)
}

/// One native glass view behind the transparent WebView, as in Maccy: macOS 26+
/// gets NSGlassEffectView, older systems NSVisualEffectView (frosted). AppKit
/// view changes must happen together on the main thread.
fn apply_native_glass(window: &WebviewWindow, level: u64) -> tauri::Result<()> {
    let target = window.clone();
    window.run_on_main_thread(move || {
        use window_vibrancy::{
            apply_liquid_glass, apply_vibrancy, clear_liquid_glass, clear_vibrancy,
            LiquidGlassOptions, NSGlassEffectViewStyle, NSVisualEffectMaterial,
            NSVisualEffectState,
        };

        let _ = clear_liquid_glass(&target);
        let _ = clear_vibrancy(&target);

        let (clear_style, tint_alpha) = glass_appearance(level);
        let style = if clear_style {
            NSGlassEffectViewStyle::Clear
        } else {
            NSGlassEffectViewStyle::Regular
        };
        let mut options = LiquidGlassOptions::new(style)
            .radius(WINDOW_CORNER_RADIUS)
            .opaque(false);
        if tint_alpha > 0 {
            options = options.tint_color((255, 255, 255, tint_alpha));
        }

        if let Err(error) = apply_liquid_glass(&target, options) {
            if !matches!(error, window_vibrancy::Error::UnsupportedPlatformVersion(_)) {
                eprintln!("[glass] liquid glass failed, using frosted glass: {error}");
            }
            if let Err(error) = apply_vibrancy(
                &target,
                NSVisualEffectMaterial::Popover,
                // The panel stays visible while other apps own focus.
                Some(NSVisualEffectState::Active),
                Some(WINDOW_CORNER_RADIUS),
            ) {
                eprintln!("[glass] frosted glass failed: {error}");
            }
        }
        round_window_corners(&target);
    })
}

/// The glass view rounds itself, but the window under it stays a rectangle, so
/// the system drew its shadow and hairline around square corners. As in Maccy,
/// the content view and its parent are clipped too. Main thread only.
fn round_window_corners(window: &WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSString;

    let Ok(pointer) = window.ns_window() else { return };
    let ns_window = pointer as *mut AnyObject;
    let curve = NSString::from_str("continuous");
    unsafe {
        let content: *mut AnyObject = msg_send![ns_window, contentView];
        if content.is_null() {
            return;
        }
        let parent: *mut AnyObject = msg_send![content, superview];
        for view in [content, parent] {
            if view.is_null() {
                continue;
            }
            let _: () = msg_send![view, setWantsLayer: true];
            let layer: *mut AnyObject = msg_send![view, layer];
            if layer.is_null() {
                continue;
            }
            let _: () = msg_send![layer, setCornerRadius: WINDOW_CORNER_RADIUS];
            let _: () = msg_send![layer, setCornerCurve: &*curve];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }
    }
    invalidate_window_shadow(window);
}

/// Have the system measure the window's shape again, or the shadow keeps the
/// shape it had before the content changed. Main thread only.
pub(crate) fn invalidate_window_shadow(window: &WebviewWindow) {
    let Ok(pointer) = window.ns_window() else { return };
    let ns_window = unsafe { &*(pointer as *const objc2_app_kit::NSWindow) };
    ns_window.invalidateShadow();
}

/// Attaches the glass once; it stays attached across hide/show.
pub fn apply_window_vibrancy(app: &AppHandle, window: &WebviewWindow) {
    let level = app.state::<SettingsState>().glass_level.load(Ordering::Relaxed);
    if APPLIED_GLASS_LEVEL.load(Ordering::SeqCst) == level as i64 {
        return;
    }
    match apply_native_glass(window, level) {
        Ok(()) => APPLIED_GLASS_LEVEL.store(level as i64, Ordering::SeqCst),
        Err(error) => eprintln!("[glass] scheduling failed: {error}"),
    }
}

/// 「Liquid Glass 透明度」 slider. 0 = clearest, 100 = most tinted.
#[tauri::command]
pub fn set_glass_level(app_handle: AppHandle, level: u64) -> Result<(), String> {
    let level = level.min(100);
    app_handle
        .state::<SettingsState>()
        .glass_level
        .store(level, Ordering::Relaxed);
    app_handle
        .state::<DbState>()
        .settings_repo
        .set("app.glass_level", &level.to_string())
        .map_err(|e| e.to_string())?;
    if let Some(window) = app_handle.get_webview_window("main") {
        apply_window_vibrancy(&app_handle, &window);
    }
    Ok(())
}

/// Whether the system has real Liquid Glass (macOS 26+). Older systems get
/// frosted glass, and the slider drives a CSS tint instead.
#[tauri::command]
pub fn supports_liquid_glass() -> bool {
    objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some()
}

// ---------------------------------------------------------------------------
// Window position
// ---------------------------------------------------------------------------

pub const POSITION_FOLLOW_MOUSE: &str = "follow_mouse";
pub const POSITION_LAST: &str = "last";
pub const POSITION_FIXED: &str = "fixed";

const LAST_POSITION_KEY: &str = "app.window_last_pos";
const FIXED_POSITION_KEY: &str = "app.window_fixed_pos";
const FIXED_POSITION_GUARD: &str = "fixed-position-setup";
/// Between the cursor and the window that opens for it.
const CURSOR_GAP: f64 = 12.0;
/// Between a window and the edge it was pushed against.
const WINDOW_EDGE_GAP: f64 = 5.0;

pub fn normalize_position_mode(mode: &str) -> &'static str {
    match mode.trim() {
        POSITION_LAST => POSITION_LAST,
        POSITION_FIXED => POSITION_FIXED,
        _ => POSITION_FOLLOW_MOUSE,
    }
}

/// A screen in logical points: left, top, width, height.
type LayoutScreen = (f64, f64, f64, f64);

fn layout_rect(rect: NSRect, primary_height: f64) -> LayoutScreen {
    (
        rect.origin.x,
        primary_height - rect.origin.y - rect.size.height,
        rect.size.width,
        rect.size.height,
    )
}

fn primary_height(mtm: MainThreadMarker) -> Option<f64> {
    Some(NSScreen::screens(mtm).firstObject()?.frame().size.height)
}

fn layout_screens() -> Vec<LayoutScreen> {
    let mtm = MainThreadMarker::new().expect("Window geometry requires the main thread");
    let Some(height) = primary_height(mtm) else { return Vec::new() };
    NSScreen::screens(mtm).iter().map(|screen| layout_rect(screen.frame(), height)).collect()
}

/// The visible part of the screen under `point`, in logical points.
fn work_area_at(point: (f64, f64)) -> Option<LayoutScreen> {
    let mtm = MainThreadMarker::new().expect("Window geometry requires the main thread");
    let screens = NSScreen::screens(mtm);
    let primary = screens.firstObject()?;
    let height = primary.frame().size.height;
    let screen = screens.iter().find(|screen| {
        let (x, y, w, h) = layout_rect(screen.frame(), height);
        point.0 >= x && point.0 < x + w && point.1 >= y && point.1 < y + h
    }).unwrap_or(primary);
    Some(layout_rect(screen.visibleFrame(), height))
}

/// The cursor in logical points.
fn global_cursor_layout() -> Option<(f64, f64)> {
    let mtm = MainThreadMarker::new().expect("Window geometry requires the main thread");
    let cursor = NSEvent::mouseLocation();
    Some((cursor.x, primary_height(mtm)? - cursor.y))
}

/// Where a `size` window opens for the cursor: centred below it, or above it
/// if there is no room below. `area` is the cursor's screen without the menu
/// bar and Dock; with the whole screen, the Dock covered the window's bottom.
fn follow_mouse_target(cursor: (f64, f64), size: (f64, f64), area: LayoutScreen) -> (f64, f64) {
    let (cursor_x, cursor_y) = cursor;
    let (w, h) = size;
    let (left, top, width, height) = area;
    let (right, bottom) = (left + width, top + height);

    let x = (cursor_x - w / 2.0).min(right - w - WINDOW_EDGE_GAP).max(left + WINDOW_EDGE_GAP);
    let below = cursor_y + CURSOR_GAP;
    let above = cursor_y - h - CURSOR_GAP;
    let y = if below + h <= bottom {
        below
    } else if above >= top {
        above
    } else {
        bottom - h - WINDOW_EDGE_GAP
    };
    (x, y.max(top + WINDOW_EDGE_GAP))
}

/// The middle of `area`, for a window nobody has placed yet.
fn centered_target(size: (f64, f64), area: LayoutScreen) -> (f64, f64) {
    let (left, top, width, height) = area;
    (left + (width - size.0) / 2.0, (top + (height - size.1) / 2.0).max(top))
}

fn logical_frame(window: &WebviewWindow) -> Option<WindowFrame> {
    let mtm = MainThreadMarker::new().expect("Window geometry requires the main thread");
    let pointer = window.ns_window().ok()?;
    let ns_window = unsafe { &*(pointer as *const NSWindow) };
    let (x, y, width, height) = layout_rect(ns_window.frame(), primary_height(mtm)?);
    Some(WindowFrame { x, y, width, height })
}

fn set_logical_position(window: &WebviewWindow, x: f64, y: f64) {
    let mtm = MainThreadMarker::new().expect("Window geometry requires the main thread");
    let Ok(pointer) = window.ns_window() else { return };
    let Some(height) = primary_height(mtm) else { return };
    let ns_window = unsafe { &*(pointer as *const NSWindow) };
    ns_window.setFrameTopLeftPoint(NSPoint::new(x, height - y));
}

fn position_window_follow_mouse(window: &WebviewWindow) {
    let Some(frame) = logical_frame(window) else { return };
    let Some(cursor) = global_cursor_layout() else { return };
    let Some(area) = work_area_at(cursor) else { return };
    let (x, y) = follow_mouse_target(cursor, (frame.width, frame.height), area);
    set_logical_position(window, x, y);
}

/// The very first opening: the middle of the screen in use.
pub fn position_window_centered(window: &WebviewWindow) {
    let Some(frame) = logical_frame(window) else { return };
    let cursor = global_cursor_layout().unwrap_or((0.0, 0.0));
    let Some(area) = work_area_at(cursor) else { return };
    let (x, y) = centered_target((frame.width, frame.height), area);
    set_logical_position(window, x, y);
}

fn parse_position(value: &str) -> Option<(f64, f64)> {
    let (x, y) = value.split_once(',')?;
    let point = (x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?);
    (point.0.is_finite() && point.1.is_finite()).then_some(point)
}

fn stored_position(app: &AppHandle, key: &str) -> Option<(f64, f64)> {
    let value = app.state::<DbState>().settings_repo.get(key).ok().flatten()?;
    parse_position(&value)
}

/// True if a window at `position` keeps its title area on some screen; a
/// display may have been unplugged since the position was stored.
fn position_is_on_screen(position: (f64, f64), screens: &[LayoutScreen]) -> bool {
    let probe = (position.0 + 60.0, position.1 + 20.0);
    screens
        .iter()
        .any(|(x, y, w, h)| probe.0 >= *x && probe.0 < x + w && probe.1 >= *y && probe.1 < y + h)
}

fn apply_stored_position(app: &AppHandle, window: &WebviewWindow, key: &str) -> bool {
    match stored_position(app, key) {
        Some(position) if position_is_on_screen(position, &layout_screens()) => {
            set_logical_position(window, position.0, position.1);
            true
        }
        _ => false,
    }
}

/// Places the window for 窗口弹出位置, for every way of summoning it.
pub fn position_window_for_show(app: &AppHandle, window: &WebviewWindow) {
    let mode = app
        .state::<SettingsState>()
        .window_position_mode
        .lock()
        .unwrap()
        .clone();
    let placed = match mode.as_str() {
        POSITION_LAST => apply_stored_position(app, window, LAST_POSITION_KEY),
        POSITION_FIXED => apply_stored_position(app, window, FIXED_POSITION_KEY),
        _ => false,
    };
    if !placed {
        position_window_follow_mouse(window);
    }
}

fn remember_last_position(app: &AppHandle, window: &WebviewWindow) {
    let origin = base_origin().or_else(|| logical_frame(window).map(|frame| (frame.x, frame.y)));
    if let Some((x, y)) = origin {
        let _ = app
            .state::<DbState>()
            .settings_repo
            .set(LAST_POSITION_KEY, &format!("{x:.1},{y:.1}"));
    }
}

/// 固定位置: keep the window open while the user drags it into place.
#[tauri::command]
pub fn begin_fixed_position_setup() {
    acquire_blur_guard(FIXED_POSITION_GUARD);
}

/// 「记住此位置」. Later moves don't change it.
#[tauri::command]
pub fn save_fixed_window_position(app_handle: AppHandle) -> Result<(), String> {
    release_blur_guard(FIXED_POSITION_GUARD);
    let window = app_handle
        .get_webview_window("main")
        .ok_or("Main window not found")?;
    let (x, y) = base_origin()
        .or_else(|| logical_frame(&window).map(|frame| (frame.x, frame.y)))
        .ok_or("Window position unavailable")?;
    app_handle
        .state::<DbState>()
        .settings_repo
        .set(FIXED_POSITION_KEY, &format!("{x:.1},{y:.1}"))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cancel_fixed_position_setup() {
    release_blur_guard(FIXED_POSITION_GUARD);
}

#[tauri::command]
pub fn has_fixed_window_position(app_handle: AppHandle) -> bool {
    stored_position(&app_handle, FIXED_POSITION_KEY).is_some()
}

// ---------------------------------------------------------------------------
// Side panel (常用 / 编辑 / 批量粘贴): the window grows to the right
// ---------------------------------------------------------------------------

/// The window before a side panel opened, in logical points.
#[derive(Clone, Copy)]
struct WindowFrame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl WindowFrame {
    fn native_rect(self, primary_height: f64) -> NSRect {
        NSRect::new(
            NSPoint::new(self.x, primary_height - self.y - self.height),
            NSSize::new(self.width, self.height),
        )
    }
}

static BASE_FRAME: Mutex<Option<WindowFrame>> = Mutex::new(None);

const SCREEN_MARGIN: f64 = 8.0;

fn base_origin() -> Option<(f64, f64)> {
    BASE_FRAME
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .map(|frame| (frame.x, frame.y))
}

/// Native frame change, so width and origin move as one step. `frame` is in
/// top-left logical coordinates.
fn set_window_frame(window: &WebviewWindow, frame: WindowFrame, animate: bool) {
    let target = window.clone();
    let _ = window.run_on_main_thread(move || {
        let Ok(pointer) = target.ns_window() else { return };
        let Some(mtm) = MainThreadMarker::new() else { return };
        // Cocoa's origin is the bottom-left of the primary screen.
        let Some(primary_height) = primary_height(mtm) else { return };
        let ns_window = unsafe { &*(pointer as *const NSWindow) };
        ns_window.setFrame_display_animate(frame.native_rect(primary_height), true, animate);
        ns_window.invalidateShadow();
    });
}

/// Opens (`extra` = added width) or closes (`None`) the right-hand panel area.
/// Returns the width the panel really got, which is less on a narrow screen.
#[tauri::command]
pub fn set_side_panel(app_handle: AppHandle, extra: Option<f64>) -> Result<f64, String> {
    let window = app_handle
        .get_webview_window("main")
        .ok_or("Main window not found")?;
    let mut base_slot = BASE_FRAME.lock().unwrap_or_else(|p| p.into_inner());

    let Some(extra) = extra.filter(|value| *value > 0.0) else {
        let base = base_slot.take();
        drop(base_slot);
        if let Some(base) = base {
            set_window_frame(&window, base, true);
        }
        return Ok(0.0);
    };

    let base = match *base_slot {
        Some(base) => base,
        None => logical_frame(&window).ok_or("Window frame unavailable")?,
    };

    let area = work_area_at((base.x + 1.0, base.y + 1.0)).ok_or("Screen unavailable")?;
    let max_width = area.2 - SCREEN_MARGIN * 2.0;
    let width = (base.width + extra).min(max_width);
    let right_limit = area.0 + area.2 - SCREEN_MARGIN;
    // No room to the right: the whole window moves left.
    let x = base.x.min(right_limit - width).max(area.0 + SCREEN_MARGIN);

    *base_slot = Some(base);
    drop(base_slot);
    set_window_frame(
        &window,
        WindowFrame { x, y: base.y, width, height: base.height },
        true,
    );
    Ok(width - base.width)
}

/// Every way of hiding the window goes through here: side panels collapse
/// (so an expanded width can't stick), and 上次打开 records where it was.
pub fn before_hide(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window("main") else { return };
        remember_last_position(&handle, &window);
        let base = BASE_FRAME.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(base) = base {
            set_window_frame(&window, base, false);
            let _ = handle.emit("side-panel-collapsed", ());
        }
    });
}

// ---------------------------------------------------------------------------
// Show / hide
// ---------------------------------------------------------------------------

pub fn toggle_window(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || toggle_window_on_main(&handle));
}

fn toggle_window_on_main(app: &AppHandle) {
    use crate::infrastructure::macos_api::window as panel;

    let Some(window) = app.get_webview_window("main") else { return };
    let is_visible = if panel::is_clipboard_panel_available(app) {
        panel::is_clipboard_panel_visible()
    } else {
        window.is_visible().unwrap_or(false)
    };

    if is_visible {
        // Pinned: the hotkey can't close it either (only Esc does). Bring it
        // forward instead, in case it was behind something.
        if WINDOW_PINNED.load(Ordering::Relaxed) {
            focus_clipboard_window_on_main(app, &window);
            if panel::responds_to_global_escape(app) {
                let _ = app.emit("window-pinned-close-blocked", ());
            }
            return;
        }
        let _ = hide_window(app.clone(), true);
        return;
    }

    focus_clipboard_window_on_main(app, &window);
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn show_main_window(app: &AppHandle, window: &WebviewWindow) {
    use crate::infrastructure::macos_api::window as panel;

    let pinned = WINDOW_PINNED.load(Ordering::Relaxed);
    let _ = app.emit("window-pinned-changed", pinned);
    LAST_SHOW_TIMESTAMP.store(now_ms(), Ordering::Relaxed);
    position_window_for_show(app, window);

    if !panel::show_clipboard_panel(app) {
        let _ = panel::refresh_clipboard_panel_policy(app);
        apply_window_vibrancy(app, window);
        let _ = window.show();
        panel::present_overlay_above_fullscreen_async(window);
        let _ = app.emit("clipboard-shown", ());
    }
    panel::reinforce_overlay_above_fullscreen(window);
    maybe_open_devtools(window);
}



/// A non-activating NSPanel: Tauri's `set_focus()` often fails to make it key
/// (or blurs the focused input); makeKeyWindow lets fields receive typing.
#[tauri::command]
pub fn activate_window_focus(app_handle: AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        if crate::infrastructure::macos_api::window::make_clipboard_panel_key(&app_handle) {
            return Ok(());
        }
        let _ = window.set_focus();
    }
    Ok(())
}

/// Native dialogs require this app to be active. Keep this separate from
/// editable WebView focus: activating the whole process for an input click can
/// make the non-activating panel resign key mid-keystroke.
#[tauri::command]
pub fn activate_app_focus(app_handle: AppHandle) -> Result<(), String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    if let Some(mtm) = MainThreadMarker::new() {
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    } else {
        crate::infrastructure::macos_api::apps::activate_app_by_pid(std::process::id() as i32);
    }

    activate_window_focus(app_handle)
}

fn hide_window(app_handle: AppHandle, restore_focus: bool) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        before_hide(&app_handle);
        if !crate::infrastructure::macos_api::window::hide_clipboard_panel(&app_handle) {
            let _ = window.hide();
        }
        NAVIGATION_ENABLED.store(false, Ordering::SeqCst);
        NAVIGATION_MODE_ACTIVE.store(false, Ordering::SeqCst);
        if restore_focus {
            let _ = restore_previous_app_focus(app_handle.clone());
        }
    }
    Ok(())
}

/// Red button and native close request share the current pin policy.
#[tauri::command]
pub fn hide_window_cmd(app_handle: AppHandle) -> Result<(), String> {
    if WINDOW_PINNED.load(Ordering::Relaxed) {
        if crate::infrastructure::macos_api::window::responds_to_global_escape(&app_handle) {
            let _ = app_handle.emit("window-pinned-close-blocked", ());
            return Ok(());
        }
        return escape_window_cmd(app_handle);
    }
    hide_window(app_handle, true)
}

/// Esc: the one way out of a pinned window. Unpins, then closes.
#[tauri::command]
pub fn escape_window_cmd(app_handle: AppHandle) -> Result<(), String> {
    if WINDOW_PINNED.load(Ordering::Relaxed) {
        crate::app::commands::set_pinned_state(&app_handle, false);
        let _ = app_handle.emit("window-pinned-changed", false);
    }
    hide_window(app_handle, true)
}

pub fn hide_window_on_resign(app_handle: AppHandle, generation: u64) -> tauri::Result<()> {
    let handle = app_handle.clone();
    app_handle.run_on_main_thread(move || {
        if crate::infrastructure::macos_api::window::can_hide_clipboard_panel_on_resign(&handle, generation) {
            let _ = hide_window(handle, false);
        }
    })
}

#[tauri::command]
pub fn toggle_window_cmd(app_handle: AppHandle) -> Result<(), String> {
    toggle_window(&app_handle);
    Ok(())
}

/// The panel normally floats over every app. While the user grants a permission
/// it drops to the normal level, so System Settings can come in front of it.
#[tauri::command]
pub fn set_panel_lowered(app_handle: AppHandle, lowered: bool, owner: Option<String>) -> Result<(), String> {
    crate::infrastructure::macos_api::window::set_clipboard_panel_lowered(
        &app_handle,
        owner.unwrap_or_else(|| "permission-onboarding".to_string()),
        lowered,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn focus_clipboard_window(app_handle: AppHandle) -> Result<(), String> {
    let Some(window) = app_handle.get_webview_window("main") else {
        return Err("Main window not found".to_string());
    };
    let handle = app_handle.clone();
    app_handle
        .run_on_main_thread(move || focus_clipboard_window_on_main(&handle, &window))
        .map_err(|error| error.to_string())
}

fn focus_clipboard_window_on_main(app: &AppHandle, window: &WebviewWindow) {
    use crate::infrastructure::macos_api::window as panel;

    panel::remember_frontmost_external_app();
    if panel::is_clipboard_panel_visible() {
        LAST_SHOW_TIMESTAMP.store(now_ms(), Ordering::Relaxed);
        let _ = panel::make_clipboard_panel_key(app);
        maybe_open_devtools(window);
        return;
    }

    NAVIGATION_ENABLED.store(true, Ordering::SeqCst);
    show_main_window(app, window);
}

#[tauri::command]
pub fn restore_previous_app_focus(_app_handle: AppHandle) -> Result<(), String> {
    let prev_pid = LAST_ACTIVE_APP_PID.load(Ordering::Relaxed);
    if prev_pid != 0 {
        crate::infrastructure::macos_api::apps::activate_app_by_pid(prev_pid as i32);
    } else {
        let prev_app = get_last_active_app_name();
        if !prev_app.trim().is_empty() {
            crate::infrastructure::macos_api::apps::activate_app_by_name(&prev_app);
        }
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::{
        centered_target, follow_mouse_target, glass_appearance, layout_rect, normalize_position_mode,
        parse_position, position_is_on_screen, NSPoint, NSRect, NSSize, WindowFrame,
    };

    // A Retina main screen (1440x900 points) with a 1x screen to its right.
    const SCREENS: [(f64, f64, f64, f64); 2] =
        [(0.0, 0.0, 1440.0, 900.0), (1440.0, 0.0, 1920.0, 1080.0)];

    /// A 14-inch MacBook: 982 points high, less the menu bar and the Dock.
    const WORK_AREA: (f64, f64, f64, f64) = (0.0, 37.0, 1512.0, 875.0);

    #[test]
    fn native_screen_and_window_frames_share_logical_coordinates() {
        let primary_height = 900.0;
        let screen = NSRect::new(NSPoint::new(-1920.0, 160.0), NSSize::new(1920.0, 1080.0));
        let layout = layout_rect(screen, primary_height);
        assert_eq!(layout, (-1920.0, -340.0, 1920.0, 1080.0));
        let (x, y) = follow_mouse_target((-1200.0, -100.0), (372.0, 600.0), layout);
        let frame = WindowFrame { x, y, width: 372.0, height: 600.0 };
        let native = frame.native_rect(primary_height);
        assert_eq!(native.origin, NSPoint::new(-1386.0, 388.0));
        assert_eq!(layout_rect(native, primary_height), (x, y, 372.0, 600.0));
    }

    #[test]
    fn follow_mouse_stays_inside_the_work_area() {
        let size = (372.0, 600.0);
        // Room below.
        assert_eq!(follow_mouse_target((700.0, 100.0), size, WORK_AREA), (514.0, 112.0));
        // None below: above the cursor.
        assert_eq!(follow_mouse_target((700.0, 800.0), size, WORK_AREA), (514.0, 188.0));
        // None either way. On the whole screen this bottom edge was under the Dock.
        let (_, y) = follow_mouse_target((700.0, 500.0), size, WORK_AREA);
        assert_eq!(y + size.1, 907.0);
        // Pushed in from the sides.
        assert_eq!(follow_mouse_target((10.0, 100.0), size, WORK_AREA).0, 5.0);
        assert_eq!(follow_mouse_target((1500.0, 100.0), size, WORK_AREA).0, 1135.0);
    }

    #[test]
    fn follow_mouse_keeps_a_too_tall_window_under_the_menu_bar() {
        assert_eq!(follow_mouse_target((700.0, 500.0), (372.0, 1000.0), WORK_AREA).1, 42.0);
    }

    #[test]
    fn the_first_window_opens_in_the_middle() {
        assert_eq!(centered_target((372.0, 600.0), WORK_AREA), (570.0, 174.5));
        // A second screen to the right.
        assert_eq!(centered_target((400.0, 600.0), SCREENS[1]), (2200.0, 240.0));
        // Taller than the screen: its top stays reachable.
        assert_eq!(centered_target((372.0, 1000.0), WORK_AREA).1, 37.0);
    }

    #[test]
    fn glass_slider_goes_from_clear_to_tinted() {
        assert_eq!(glass_appearance(0), (true, 0));
        assert_eq!(glass_appearance(24), (true, 0));
        let (clear, low) = glass_appearance(30);
        assert!(!clear && low > 0);
        let (_, high) = glass_appearance(100);
        assert!(high > low);
        assert_eq!(high, 184);
    }

    #[test]
    fn position_modes_fall_back_to_follow_mouse() {
        assert_eq!(normalize_position_mode("last"), "last");
        assert_eq!(normalize_position_mode("fixed"), "fixed");
        assert_eq!(normalize_position_mode("whatever"), "follow_mouse");
    }

    #[test]
    fn stored_positions_parse_and_must_be_on_a_screen() {
        assert_eq!(parse_position("10.5, 20"), Some((10.5, 20.0)));
        assert_eq!(parse_position("nope"), None);
        assert!(position_is_on_screen((100.0, 100.0), &SCREENS));
        assert!(!position_is_on_screen((9000.0, 100.0), &SCREENS));
    }
}
