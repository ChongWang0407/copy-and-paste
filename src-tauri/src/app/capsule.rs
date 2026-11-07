//! 复制提示弹窗: a pill at the top of the screen that grows in for a moment
//! after each copy (what, from which app) and shrinks away again.
//!
//! The pill is its own transparent NSPanel ("capsule"), placed once at its
//! full size. Growing and shrinking is a Core Animation mask on the window's
//! content (the glass or black fill and the page): the render server animates
//! it in step with the display, and nothing is laid out again on the way, so
//! there is no stutter. (Resizing the window every frame made the WebView and
//! the glass redraw out of step with the screen.) The page only draws the icon
//! and text and fades them in and out (`capsule.html`, `src/capsule/`).
//!
//! Three looks (`app.capsule_style`):
//! - `glass` 玻璃胶囊: a Liquid Glass pill just under the menu bar.
//! - `black` 黑色胶囊: the same pill, black.
//! - `island` 黑色刘海: black, grown out of the notch on a screen that has
//!   one; elsewhere a black pill at the top edge.

use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use crate::app_state::SettingsState;
use crate::database::DbState;
use crate::domain::models::ClipboardEntry;
use crate::infrastructure::repository::settings_repo::SettingsRepository;

pub const LABEL: &str = "capsule";
pub const ENABLED_KEY: &str = "app.capsule_enabled";
pub const STYLE_KEY: &str = "app.capsule_style";
pub const STYLE_GLASS: &str = "glass";
/// 黑色刘海. The value is still `island` so saved settings keep working.
pub const STYLE_ISLAND: &str = "island";
/// 黑色胶囊: the glass pill's shape and place, filled black.
pub const STYLE_BLACK: &str = "black";

pub const DURATION_KEY: &str = "app.capsule_duration_ms";
/// How long the pill stays out after the last copy: the slider's middle.
pub const DEFAULT_DURATION_MS: u64 = 2200;
pub const MIN_DURATION_MS: u64 = 1000;
pub const MAX_DURATION_MS: u64 = 3400;

pub fn normalize_duration(value: &str) -> u64 {
    value
        .trim()
        .parse::<u64>()
        .map(|ms| ms.clamp(MIN_DURATION_MS, MAX_DURATION_MS))
        .unwrap_or(DEFAULT_DURATION_MS)
}
/// The page fades its content out for this long before the pill shrinks.
const CONTENT_FADE_OUT: Duration = Duration::from_millis(140);
/// A paste writes the clipboard too; the capsule is for the user's own copies.
const AFTER_PASTE_QUIET_SECS: u64 = 2;
const PREVIEW_CHARS: usize = 64;

pub fn normalize_style(style: &str) -> &'static str {
    match style.trim() {
        STYLE_ISLAND => STYLE_ISLAND,
        STYLE_BLACK => STYLE_BLACK,
        _ => STYLE_GLASS,
    }
}

// ---------------------------------------------------------------------------
// Geometry (AppKit points, origin bottom-left)
// ---------------------------------------------------------------------------

/// The window: a fixed rect in screen points (origin bottom-left).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// The visible part of the window: a rect hanging from the window's top
/// edge, centred, in window points.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Mask {
    width: f64,
    height: f64,
    radius: f64,
    opacity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Geometry {
    window: Rect,
    collapsed: Mask,
    expanded: Mask,
    /// Hangs from the notch: only the bottom corners are rounded.
    notch: bool,
    /// Room the page leaves at the top for the notch.
    top_inset: f64,
}

/// What the geometry needs to know about the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ScreenInfo {
    /// Full frame.
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    /// Top of the visible frame (below the menu bar when it shows).
    visible_top: f64,
    /// Width and height of the camera notch, when the screen has one.
    notch: Option<(f64, f64)>,
}

const PILL_SIZE: (f64, f64) = (340.0, 52.0);
const NOTCH_EXTRA_WIDTH: f64 = 150.0;
const NOTCH_MIN_WIDTH: f64 = 360.0;
const NOTCH_CONTENT_HEIGHT: f64 = 46.0;
const GAP_UNDER_MENU_BAR: f64 = 8.0;
const ISLAND_TOP_GAP: f64 = 6.0;

fn geometry_for(style: &str, screen: ScreenInfo) -> Geometry {
    let cx = screen.x + screen.width / 2.0;
    let screen_top = screen.y + screen.height;
    // A fullscreen app hides the menu bar: then the visible frame reaches the top.
    let menu_bar = (screen_top - screen.visible_top).max(0.0);
    let window = |width: f64, height: f64, top: f64| Rect { x: (cx - width / 2.0).round(), y: top - height, width, height };

    if style == STYLE_ISLAND {
        if let Some((notch_w, notch_h)) = screen.notch {
            let width = (notch_w + NOTCH_EXTRA_WIDTH).max(NOTCH_MIN_WIDTH);
            let height = notch_h + NOTCH_CONTENT_HEIGHT;
            return Geometry {
                window: window(width, height, screen_top),
                // Exactly the notch, so it is hidden in it while it is small.
                collapsed: Mask { width: notch_w, height: notch_h, radius: 8.0, opacity: 1.0 },
                expanded: Mask { width, height, radius: 22.0, opacity: 1.0 },
                notch: true,
                top_inset: notch_h,
            };
        }
        let (w, h) = PILL_SIZE;
        return Geometry {
            window: window(w, h, screen_top - ISLAND_TOP_GAP),
            collapsed: Mask { width: h, height: h, radius: h / 2.0, opacity: 0.0 },
            expanded: Mask { width: w, height: h, radius: h / 2.0, opacity: 1.0 },
            notch: false,
            top_inset: 0.0,
        };
    }

    // 玻璃胶囊 and 黑色胶囊: under the menu bar. A bubble that stretches out
    // into the pill.
    let (w, h) = PILL_SIZE;
    Geometry {
        window: window(w, h, screen_top - menu_bar - GAP_UNDER_MENU_BAR),
        collapsed: Mask { width: h, height: h, radius: h / 2.0, opacity: 0.0 },
        expanded: Mask { width: w, height: h, radius: h / 2.0, opacity: 1.0 },
        notch: false,
        top_inset: 0.0,
    }
}

/// Core Animation curves, as cubic Béziers. Both only slow down into place:
/// no overshoot, nothing to wobble.
#[derive(Debug, Clone, Copy)]
struct Curve {
    duration: Duration,
    points: [f32; 4],
}

const EXPAND: Curve = Curve { duration: Duration::from_millis(460), points: [0.22, 1.0, 0.36, 1.0] };
const COLLAPSE: Curve = Curve { duration: Duration::from_millis(320), points: [0.4, 0.0, 0.2, 1.0] };

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

struct CapsuleState {
    /// The panel is on screen (growing, shown or shrinking).
    visible: bool,
    /// Shrinking away; a new copy brings it back out.
    hiding: bool,
    /// Bumped by each show and each hide, so an older timer does nothing.
    hide_gen: u64,
    geometry: Option<Geometry>,
    /// Copies while the pill stayed out.
    count: u32,
    /// The look its native views are set up for.
    applied_style: &'static str,
    seq: u64,
}

static STATE: Mutex<CapsuleState> = Mutex::new(CapsuleState {
    visible: false,
    hiding: false,
    hide_gen: 0,
    geometry: None,
    count: 0,
    applied_style: "",
    seq: 0,
});

fn state() -> std::sync::MutexGuard<'static, CapsuleState> {
    STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---------------------------------------------------------------------------
// What the page shows
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CapsulePayload {
    /// `text` `rich_text` `code` `url` `image` `video` `file`, or `sample`.
    pub kind: String,
    pub preview: String,
    pub app_name: String,
    /// Data URL of the source app's icon.
    pub app_icon: Option<String>,
    /// Thumbnail path of a copied image.
    pub image_path: Option<String>,
    pub file_count: usize,
    // Filled in by `present`.
    pub style: String,
    pub language: String,
    pub count: u32,
    /// The pill has just started growing (content waits for it).
    pub fresh: bool,
    pub seq: u64,
    pub width: f64,
    pub height: f64,
    pub top_inset: f64,
}

fn first_line_preview(content: &str) -> String {
    let line = content.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("");
    let mut preview: String = line.chars().take(PREVIEW_CHARS).collect();
    if line.chars().count() > PREVIEW_CHARS {
        preview.push('…');
    }
    preview
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path.trim())
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.trim().to_string())
}

fn payload_for(app: &AppHandle, entry: &ClipboardEntry) -> CapsulePayload {
    let kind = entry.content_type.clone();
    let mut payload = CapsulePayload {
        kind: kind.clone(),
        app_name: entry.source_app.clone(),
        ..Default::default()
    };
    match kind.as_str() {
        "image" => {
            let path = entry.content.trim();
            if !path.starts_with("data:") && std::path::Path::new(path).is_file() {
                payload.image_path = crate::app::commands::thumbnail_cmd::thumbnail_dir(app)
                    .ok()
                    .and_then(|dir| crate::app::commands::thumbnail_cmd::build_thumbnail(path, &dir).ok())
                    .or_else(|| Some(path.to_string()));
            }
        }
        "file" | "video" => {
            let paths: Vec<&str> = entry.content.lines().filter(|line| !line.trim().is_empty()).collect();
            payload.file_count = paths.len().max(1);
            payload.preview = paths.first().map(|path| file_name(path)).unwrap_or_default();
        }
        _ => payload.preview = first_line_preview(&entry.content),
    }
    #[cfg(target_os = "macos")]
    if let Some(path) = entry.source_app_path.as_deref().filter(|path| !path.is_empty()) {
        payload.app_icon =
            crate::infrastructure::macos_api::apps::get_executable_icon(path.to_string()).ok().flatten();
    }
    payload
}

fn language(app: &AppHandle) -> String {
    app.state::<DbState>()
        .settings_repo
        .get("app.language")
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Called for every recorded copy (pipeline `DistributionStage`).
pub fn notify_copied(app: &AppHandle, entry: &ClipboardEntry) {
    let settings = app.state::<SettingsState>();
    if !settings.capsule_enabled.load(Ordering::Relaxed) {
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let last_paste = crate::LAST_APP_SET_TIMESTAMP.load(Ordering::SeqCst);
    if last_paste != 0 && now.saturating_sub(last_paste) < AFTER_PASTE_QUIET_SECS {
        return;
    }
    let style = normalize_style(&settings.capsule_style.lock().map(|s| s.clone()).unwrap_or_default());
    // Icon lookup and the image thumbnail can take a moment; the capture
    // worker shouldn't wait for them.
    let app = app.clone();
    let entry = entry.clone();
    std::thread::spawn(move || {
        let payload = payload_for(&app, &entry);
        present(&app, payload, style);
    });
}

/// 设置页: show what the chosen look looks like.
#[tauri::command]
pub fn preview_capsule(app_handle: AppHandle, style: Option<String>) {
    let style = match style {
        Some(style) => normalize_style(&style),
        None => normalize_style(
            &app_handle
                .state::<SettingsState>()
                .capsule_style
                .lock()
                .map(|s| s.clone())
                .unwrap_or_default(),
        ),
    };
    let payload = CapsulePayload {
        kind: "sample".into(),
        app_name: "Copy & Paste".into(),
        ..Default::default()
    };
    present(&app_handle, payload, style);
}

// ---------------------------------------------------------------------------
// Showing and hiding
// ---------------------------------------------------------------------------

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

fn emit_show(app: &AppHandle, mut payload: CapsulePayload, style: &str, geometry: Geometry, count: u32, fresh: bool, seq: u64) {
    payload.style = style.to_string();
    payload.language = language(app);
    payload.count = count;
    payload.fresh = fresh;
    payload.seq = seq;
    payload.width = geometry.window.width;
    payload.height = geometry.window.height;
    payload.top_inset = geometry.top_inset;
    let _ = app.emit_to(LABEL, "capsule-show", payload);
}

fn present(app: &AppHandle, payload: CapsulePayload, style: &'static str) {
    let Some(win) = window(app) else { return };
    let handle = app.clone();
    let _ = win.clone().run_on_main_thread(move || {
        let Some(screen) = native::current_screen() else { return };
        let geometry = geometry_for(style, screen);

        let mut st = state();
        st.seq += 1;
        let seq = st.seq;
        st.hide_gen += 1;
        let hide_gen = st.hide_gen;
        let warm = st.visible && !st.hiding && st.applied_style == style && st.geometry == Some(geometry);
        if warm {
            // Already out: just the new content.
            st.count += 1;
            let count = st.count;
            drop(st);
            emit_show(&handle, payload, style, geometry, count, false, seq);
            schedule_hide(handle, hide_gen);
            return;
        }

        // Shrinking with the same look and place: grow back from where the
        // animation is now. Otherwise start over from the collapsed shape.
        let restyle = st.applied_style != style;
        let resume = st.visible && !restyle && st.geometry == Some(geometry);
        st.visible = true;
        st.hiding = false;
        st.count = 1;
        st.geometry = Some(geometry);
        st.applied_style = style;
        drop(st);

        if restyle {
            native::apply_style(&win, style);
        }
        if !resume {
            native::set_frame(&win, geometry.window);
            native::set_mask(&win, geometry.collapsed, geometry.notch, None);
        }
        native::order_front(&win);
        emit_show(&handle, payload, style, geometry, 1, true, seq);
        native::set_mask(&win, geometry.expanded, geometry.notch, Some(EXPAND));
        schedule_hide(handle, hide_gen);
    });
}

fn show_for(app: &AppHandle) -> Duration {
    Duration::from_millis(app.state::<SettingsState>().capsule_duration_ms.load(Ordering::Relaxed))
}

fn schedule_hide(app: AppHandle, hide_gen: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(show_for(&app));
        begin_hide(app, hide_gen);
    });
}

fn begin_hide(app: AppHandle, hide_gen: u64) {
    {
        let mut st = state();
        if st.hide_gen != hide_gen || !st.visible || st.hiding {
            return;
        }
        st.hiding = true;
    }
    let _ = app.emit_to(LABEL, "capsule-hide", ());
    std::thread::spawn(move || {
        std::thread::sleep(CONTENT_FADE_OUT);
        let Some(win) = window(&app) else { return };
        let target = win.clone();
        let _ = win.run_on_main_thread(move || {
            let st = state();
            if st.hide_gen != hide_gen || !st.hiding {
                return;
            }
            let Some(geometry) = st.geometry else { return };
            drop(st);
            native::set_mask(&target, geometry.collapsed, geometry.notch, Some(COLLAPSE));
        });
        // Order it out once the shrink is over, unless a copy brought it back.
        std::thread::sleep(COLLAPSE.duration + Duration::from_millis(40));
        let Some(win) = window(&app) else { return };
        let target = win.clone();
        let _ = win.run_on_main_thread(move || {
            let mut st = state();
            if st.hide_gen != hide_gen || !st.hiding {
                return;
            }
            st.visible = false;
            st.hiding = false;
            st.count = 0;
            drop(st);
            native::order_out(&target);
        });
    });
}

/// Hides it at once (the switch was turned off).
pub fn dismiss(app: &AppHandle) {
    let hide_gen = {
        let mut st = state();
        st.hide_gen += 1;
        st.hide_gen
    };
    begin_hide(app.clone(), hide_gen);
}

// ---------------------------------------------------------------------------
// The window
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    panel!(CapsulePanel {
        config: {
            is_floating_panel: true,
            can_become_key_window: false,
            can_become_main_window: false
        }
    })
}

/// Above the menu bar (24) and its status items (25), below menus.
const CAPSULE_LEVEL: i32 = 26;

/// Created hidden at startup so the first copy has a page ready.
pub fn create_window(app: &AppHandle) {
    let built = tauri::WebviewWindowBuilder::new(app, LABEL, tauri::WebviewUrl::App("capsule.html".into()))
        .title("Copy & Paste Capsule")
        .inner_size(PILL_SIZE.0, PILL_SIZE.1)
        .decorations(false)
        .transparent(true)
        .resizable(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .accept_first_mouse(false)
        .visible_on_all_workspaces(true)
        .build();
    let win = match built {
        Ok(win) => win,
        Err(error) => {
            eprintln!("[capsule] window failed: {error}");
            return;
        }
    };
    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::{CollectionBehavior, PanelLevel, StyleMask, WebviewWindowExt};
        match win.to_panel::<CapsulePanel>() {
            Ok(panel) => {
                panel.set_level(PanelLevel::Custom(CAPSULE_LEVEL).value());
                panel.set_style_mask(StyleMask::empty().borderless().nonactivating_panel().into());
                panel.set_hides_on_deactivate(false);
                panel.set_ignores_mouse_events(true);
                panel.set_has_shadow(false);
                panel.set_collection_behavior(
                    CollectionBehavior::new()
                        .can_join_all_spaces()
                        .full_screen_auxiliary()
                        .stationary()
                        .ignores_cycle()
                        .into(),
                );
            }
            Err(error) => eprintln!("[capsule] panel conversion failed: {error:?}"),
        }
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::{Curve, Mask, Rect, ScreenInfo, PILL_SIZE, STYLE_GLASS};
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSScreen, NSWindow};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_core_graphics::CGColor;
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use objc2_quartz_core::{CACornerMask, CALayer, CAMediaTimingFunction, CATransaction};
    use tauri::WebviewWindow;

    fn ns_window(win: &WebviewWindow) -> Option<&NSWindow> {
        let pointer = win.ns_window().ok()?;
        if pointer.is_null() {
            return None;
        }
        Some(unsafe { &*(pointer as *const NSWindow) })
    }

    /// The window content's layer: the glass or black fill, and the page.
    fn content_layer(window: &NSWindow) -> Option<Retained<CALayer>> {
        unsafe {
            let ns_window = window as *const NSWindow as *mut AnyObject;
            let content: *mut AnyObject = msg_send![ns_window, contentView];
            if content.is_null() {
                return None;
            }
            let _: () = msg_send![content, setWantsLayer: true];
            msg_send![content, layer]
        }
    }

    fn contains(rect: NSRect, point: NSPoint) -> bool {
        point.x >= rect.origin.x
            && point.x < rect.origin.x + rect.size.width
            && point.y >= rect.origin.y
            && point.y < rect.origin.y + rect.size.height
    }

    /// The screen under the mouse: that's where the user is looking.
    pub fn current_screen() -> Option<ScreenInfo> {
        let mtm = MainThreadMarker::new()?;
        let screens = NSScreen::screens(mtm);
        let mouse = NSEvent::mouseLocation();
        let count = screens.count();
        if count == 0 {
            return None;
        }
        let mut chosen = screens.objectAtIndex(0);
        for index in 0..count {
            let screen = screens.objectAtIndex(index);
            if contains(screen.frame(), mouse) {
                chosen = screen;
                break;
            }
        }
        let frame = chosen.frame();
        let visible = chosen.visibleFrame();
        let insets = chosen.safeAreaInsets();
        let left = chosen.auxiliaryTopLeftArea();
        let right = chosen.auxiliaryTopRightArea();
        let notch = if insets.top > 0.0 && left.size.width > 0.0 && right.size.width > 0.0 {
            let width = frame.size.width - left.size.width - right.size.width;
            (width > 0.0).then_some((width, insets.top))
        } else {
            None
        };
        Some(ScreenInfo {
            x: frame.origin.x,
            y: frame.origin.y,
            width: frame.size.width,
            height: frame.size.height,
            visible_top: visible.origin.y + visible.size.height,
            notch,
        })
    }

    pub fn set_frame(win: &WebviewWindow, rect: Rect) {
        let Some(window) = ns_window(win) else { return };
        let frame = NSRect::new(NSPoint::new(rect.x, rect.y), NSSize::new(rect.width, rect.height));
        window.setFrame_display(frame, true);
        window.setAlphaValue(1.0);
    }

    /// Moves the mask to `mask`: at once (`curve` None), or animated by Core
    /// Animation from wherever it is on screen right now. Main thread only.
    pub fn set_mask(win: &WebviewWindow, mask: Mask, notch: bool, curve: Option<Curve>) {
        let Some(window) = ns_window(win) else { return };
        let Some(layer) = content_layer(window) else { return };
        let bounds = layer.bounds();
        let flipped = layer.contentsAreFlipped();

        let layer_mask = match layer.mask() {
            Some(existing) => existing,
            None => {
                let created = CALayer::new();
                let opaque = CGColor::new_generic_gray(0.0, 1.0);
                created.setBackgroundColor(Some(&opaque));
                unsafe { layer.setMask(Some(&created)) };
                created
            }
        };

        CATransaction::begin();
        match curve {
            None => CATransaction::setDisableActions(true),
            Some(curve) => {
                // Start from what is on screen, so a shrink turned back into a
                // grow carries on smoothly instead of jumping.
                if let Some(shown) = unsafe { layer_mask.presentationLayer() } {
                    CATransaction::setDisableActions(true);
                    layer_mask.setBounds(shown.bounds());
                    layer_mask.setPosition(shown.position());
                    layer_mask.setCornerRadius(shown.cornerRadius());
                    layer_mask.setOpacity(shown.opacity());
                    CATransaction::setDisableActions(false);
                }
                let [a, b, c, d] = curve.points;
                let timing = CAMediaTimingFunction::functionWithControlPoints(a, b, c, d);
                CATransaction::setAnimationDuration(curve.duration.as_secs_f64());
                CATransaction::setAnimationTimingFunction(Some(&timing));
            }
        }
        // Hangs from the top edge of the window, centred.
        layer_mask.setAnchorPoint(CGPoint::new(0.5, if flipped { 0.0 } else { 1.0 }));
        layer_mask.setPosition(CGPoint::new(
            bounds.size.width / 2.0,
            if flipped { 0.0 } else { bounds.size.height },
        ));
        layer_mask.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(mask.width, mask.height)));
        layer_mask.setCornerRadius(mask.radius.min(mask.height / 2.0));
        layer_mask.setMaskedCorners(if notch {
            // The two corners at the bottom of the screen.
            if flipped {
                CACornerMask::LayerMinXMaxYCorner | CACornerMask::LayerMaxXMaxYCorner
            } else {
                CACornerMask::LayerMinXMinYCorner | CACornerMask::LayerMaxXMinYCorner
            }
        } else {
            CACornerMask::LayerMinXMinYCorner
                | CACornerMask::LayerMaxXMinYCorner
                | CACornerMask::LayerMinXMaxYCorner
                | CACornerMask::LayerMaxXMaxYCorner
        });
        layer_mask.setOpacity(mask.opacity);
        CATransaction::commit();
    }

    /// Native views for a look: Liquid Glass (frosted glass before macOS 26),
    /// or a black fill. The mask cuts the pill out of either. Main thread only.
    pub fn apply_style(win: &WebviewWindow, style: &str) {
        use window_vibrancy::{
            apply_liquid_glass, apply_vibrancy, clear_liquid_glass, clear_vibrancy, LiquidGlassOptions,
            NSGlassEffectViewStyle, NSVisualEffectMaterial, NSVisualEffectState,
        };
        let Some(window) = ns_window(win) else { return };
        let _ = clear_liquid_glass(win);
        let _ = clear_vibrancy(win);
        let Some(layer) = content_layer(window) else { return };
        if style != STYLE_GLASS {
            let black = CGColor::new_generic_gray(0.0, 1.0);
            layer.setBackgroundColor(Some(&black));
            return;
        }
        layer.setBackgroundColor(None);
        let radius = PILL_SIZE.1 / 2.0;
        // The pill sits on whatever is under the menu bar: keep some tint so
        // the text stays readable.
        let options = LiquidGlassOptions::new(NSGlassEffectViewStyle::Regular)
            .radius(radius)
            .opaque(false)
            .tint_color((255, 255, 255, 90));
        if apply_liquid_glass(win, options).is_err() {
            let _ = apply_vibrancy(
                win,
                NSVisualEffectMaterial::Popover,
                Some(NSVisualEffectState::Active),
                Some(radius),
            );
        }
    }

    pub fn order_front(win: &WebviewWindow) {
        if let Some(window) = ns_window(win) {
            window.orderFrontRegardless();
        }
    }

    pub fn order_out(win: &WebviewWindow) {
        if let Some(window) = ns_window(win) {
            window.orderOut(None);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use super::{Curve, Mask, Rect, ScreenInfo};
    use tauri::WebviewWindow;
    pub fn current_screen() -> Option<ScreenInfo> {
        None
    }
    pub fn set_frame(_: &WebviewWindow, _: Rect) {}
    pub fn set_mask(_: &WebviewWindow, _: Mask, _: bool, _: Option<Curve>) {}
    pub fn apply_style(_: &WebviewWindow, _: &str) {}
    pub fn order_front(_: &WebviewWindow) {}
    pub fn order_out(_: &WebviewWindow) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(notch: Option<(f64, f64)>) -> ScreenInfo {
        ScreenInfo { x: 0.0, y: 0.0, width: 1512.0, height: 982.0, visible_top: 982.0 - 37.0, notch }
    }

    #[test]
    fn unknown_styles_fall_back_to_glass() {
        assert_eq!(normalize_style("island"), STYLE_ISLAND);
        assert_eq!(normalize_style("black"), STYLE_BLACK);
        assert_eq!(normalize_style(" glass "), STYLE_GLASS);
        assert_eq!(normalize_style(""), STYLE_GLASS);
        assert_eq!(normalize_style("neon"), STYLE_GLASS);
    }

    #[test]
    fn pills_hang_under_the_menu_bar_in_the_middle() {
        for style in [STYLE_GLASS, STYLE_BLACK] {
            let g = geometry_for(style, screen(Some((200.0, 37.0))));
            assert_eq!(g.window.x + g.window.width / 2.0, 756.0);
            assert_eq!(g.window.y + g.window.height, 982.0 - 37.0 - GAP_UNDER_MENU_BAR);
            assert_eq!(g.expanded.width, g.window.width);
            assert!(!g.notch);
            assert_eq!(g.collapsed.opacity, 0.0);
        }
    }

    #[test]
    fn notch_style_grows_out_of_the_notch() {
        let g = geometry_for(STYLE_ISLAND, screen(Some((200.0, 37.0))));
        assert!(g.notch);
        assert_eq!(g.top_inset, 37.0);
        assert_eq!(g.window.y + g.window.height, 982.0);
        assert_eq!((g.collapsed.width, g.collapsed.height), (200.0, 37.0));
        assert_eq!(g.collapsed.opacity, 1.0, "it hides inside the notch, no fade");
        assert_eq!((g.expanded.width, g.expanded.height), (g.window.width, g.window.height));
    }

    #[test]
    fn notch_style_without_a_notch_is_a_pill_at_the_top() {
        let g = geometry_for(STYLE_ISLAND, screen(None));
        assert!(!g.notch);
        assert_eq!(g.top_inset, 0.0);
        assert_eq!(g.window.y + g.window.height, 982.0 - ISLAND_TOP_GAP);
    }

    #[test]
    fn stay_time_is_kept_on_the_slider() {
        assert_eq!(normalize_duration("2200"), 2200);
        assert_eq!(normalize_duration("10"), MIN_DURATION_MS);
        assert_eq!(normalize_duration("99999"), MAX_DURATION_MS);
        assert_eq!(normalize_duration("x"), DEFAULT_DURATION_MS);
        assert_eq!(DEFAULT_DURATION_MS, (MIN_DURATION_MS + MAX_DURATION_MS) / 2);
    }

    #[test]
    fn previews_take_the_first_line_and_are_cut() {
        assert_eq!(first_line_preview("\n  hello \nworld"), "hello");
        let long = "字".repeat(80);
        let preview = first_line_preview(&long);
        assert_eq!(preview.chars().count(), PREVIEW_CHARS + 1);
        assert!(preview.ends_with('…'));
        assert_eq!(file_name("/Users/a/Desktop/报告.pdf"), "报告.pdf");
    }
}

