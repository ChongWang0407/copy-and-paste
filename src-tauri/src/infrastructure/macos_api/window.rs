#[cfg(target_os = "macos")]
use objc2::rc::autoreleasepool;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;
use tauri::AppHandle;
#[cfg(target_os = "macos")]
use tauri::Manager;

#[derive(Debug, Clone, Default)]
pub struct ActiveAppInfo {
    pub app_name: String,
    pub pid: u32,
    pub process_path: Option<String>,
}

pub fn start_window_tracking() {
    use block2::RcBlock;
    use objc2_app_kit::{NSWorkspaceApplicationKey, NSWorkspaceDidActivateApplicationNotification};
    use objc2_foundation::{NSNotification, NSOperationQueue};
    use std::ptr::NonNull;

    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let handler = RcBlock::new(|notification: NonNull<NSNotification>| {
        // The Dock may already have activated us by the time this arrives.
        // The notification still identifies the external app that preceded us.
        let notification = unsafe { notification.as_ref() };
        if let Some(app) = notification.userInfo()
            .and_then(|info| info.objectForKey(unsafe { NSWorkspaceApplicationKey }))
            .and_then(|app| app.downcast::<NSRunningApplication>().ok())
        {
            remember_external_app(&app);
        }
    });
    // The observer lives for the lifetime of the application.
    unsafe {
        let observer = center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidActivateApplicationNotification),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &handler,
        );
        let _ = objc2::rc::Retained::into_raw(observer);
    }
    remember_frontmost_external_app();
}

pub fn remember_frontmost_external_app() {
    autoreleasepool(|_| {
        let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
            return;
        };
        remember_external_app(&app);
    });
}

fn remember_external_app(app: &NSRunningApplication) {
    let pid = app.processIdentifier() as u32;
    if pid != 0 && pid != std::process::id() {
        let name = app.localizedName().map(|name| name.to_string()).unwrap_or_default();
        crate::global_state::set_last_active_app_name(name);
        crate::global_state::LAST_ACTIVE_APP_PID.store(pid, AtomicOrdering::Relaxed);
    }
}

pub fn get_active_app_snapshot() -> ActiveAppInfo {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let app = workspace.frontmostApplication();

        if let Some(app) = app {
            let name = app
                .localizedName()
                .map(|s: objc2::rc::Retained<NSString>| s.to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            let pid = app.processIdentifier() as u32;
            let bundle_url = app.bundleURL();
            let process_path = if let Some(url) = bundle_url {
                url.path()
                    .map(|s: objc2::rc::Retained<NSString>| s.to_string())
            } else {
                None
            };

            ActiveAppInfo {
                app_name: name,
                pid,
                process_path,
            }
        } else {
            ActiveAppInfo {
                app_name: "macOS App".into(),
                pid: 0,
                process_path: None,
            }
        }
    });

}

pub const PIN_MODE_KEY: &str = "app.window_pin_mode";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowPinMode {
    #[default]
    AlwaysOnTop,
    Normal,
}

impl WindowPinMode {
    pub fn from_setting(value: &str) -> Self {
        if value == "normal" { Self::Normal } else { Self::AlwaysOnTop }
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            Self::AlwaysOnTop => "always_on_top",
            Self::Normal => "normal",
        }
    }
}

use tauri_nspanel::PanelLevel;

fn panel_policy(pinned: bool, mode: WindowPinMode, lowered: bool) -> (i64, bool, bool) {
    let floating = !lowered && !(pinned && mode == WindowPinMode::Normal);
    let level = if floating { PanelLevel::Dock } else { PanelLevel::Normal };
    (level.value(), floating, pinned && mode == WindowPinMode::AlwaysOnTop)
}

fn pin_mode(app: &AppHandle) -> WindowPinMode {
    app.try_state::<crate::app_state::SettingsState>()
        .map(|settings| *settings.window_pin_mode.lock().unwrap())
        .unwrap_or_default()
}

pub fn responds_to_global_escape(app: &AppHandle) -> bool {
    let (_, _, responds) = panel_policy(
        crate::global_state::WINDOW_PINNED.load(AtomicOrdering::Relaxed),
        pin_mode(app),
        is_panel_lowered(),
    );
    responds
}

fn lowered_owners() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static OWNERS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::LazyLock::new(Default::default);
    &OWNERS
}

fn is_panel_lowered() -> bool {
    !lowered_owners().lock().unwrap().is_empty()
}

// Called on the main thread, including by delayed show and fullscreen work.
fn apply_panel_policy(app: &AppHandle) {
    use tauri_nspanel::ManagerExt;
    let pinned = crate::global_state::WINDOW_PINNED.load(AtomicOrdering::Relaxed);
    let (level, floating, _) = panel_policy(pinned, pin_mode(app), is_panel_lowered());
    if let Ok(panel) = app.get_webview_panel("main") {
        if panel.is_floating_panel() != floating {
            panel.set_floating_panel(floating);
        }
        if panel.as_panel().level() as i64 != level {
            panel.set_level(level);
        }
        // Pinned palettes keep the paste target's keyboard focus on ordinary clicks.
        if panel.becomes_key_only_if_needed() != pinned {
            panel.set_becomes_key_only_if_needed(pinned);
        }
    } else if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_always_on_top(floating);
    }
}

pub fn refresh_clipboard_panel_policy(app: &AppHandle) -> tauri::Result<()> {
    let handle = app.clone();
    app.run_on_main_thread(move || apply_panel_policy(&handle))
}

#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    panel!(ClipboardPanel {
        config: {
            can_become_key_window: true,
            can_become_main_window: false
        }
    })

    panel_event!(ClipboardPanelEventHandler {
        window_did_resign_key(notification: &NSNotification) -> (),
    })
}

/// After `to_panel`, tao's `focusable` ivar no longer exists on the ObjC class.
/// Calling `WebviewWindow::set_focusable` then panics. Skip it once converted.
#[cfg(target_os = "macos")]
pub fn set_window_focusable(window: &tauri::WebviewWindow, focusable: bool) {
    use tauri_nspanel::ManagerExt;
    if window.app_handle().get_webview_panel("main").is_ok() {
        return;
    }
    let _ = window.set_focusable(focusable);
}


/// Convert the main Tauri NSWindow into a non-activating NSPanel.
///
/// A plain NSWindow cannot appear on another app's native fullscreen Space,
/// even with FullScreenAuxiliary — this is a known AppKit/Tauri limitation.
/// See EcoPaste / tauri-nspanel fullscreen example.
#[cfg(target_os = "macos")]
pub fn setup_clipboard_panel(window: &tauri::WebviewWindow) -> Result<(), String> {
    use tauri_nspanel::{CollectionBehavior, StyleMask, WebviewWindowExt};

    let panel = window
        .to_panel::<ClipboardPanel>()
        .map_err(|e| format!("to_panel failed: {e:?}"))?;

    // Keep NonactivatingPanel for fullscreen overlay, but preserve Resizable —
    // StyleMask::empty().nonactivating_panel() alone wiped the resize bit and
    // made the undecorated window edges undraggable.
    panel.set_style_mask(StyleMask::empty().nonactivating_panel().resizable().into());
    // Keep visibility under our resign-key state machine. NSPanel defaults can
    // otherwise order the panel out when the app is activated/deactivated without
    // updating `clipboard_panel_visible`, leaving the next hotkey invocation in
    // a visible-but-not-key (or internally-visible-but-actually-hidden) state.
    panel.set_hides_on_deactivate(false);
    panel.set_works_when_modal(true);
    apply_panel_policy(window.app_handle());
    // Idle/hidden: move_to_active_space so the next show lands on the current Space
    // (including another app's fullscreen desktop).
    panel.set_collection_behavior(
        CollectionBehavior::new()
            .full_screen_auxiliary()
            .move_to_active_space()
            .stationary()
            .into(),
    );
    let initially_visible = panel.is_visible();
    clipboard_panel_visible().store(initially_visible, AtomicOrdering::SeqCst);
    if initially_visible {
        let generation = overlay_show_generation().fetch_add(1, AtomicOrdering::SeqCst) + 1;
        overlay_presented_generation().store(generation, AtomicOrdering::SeqCst);
    }

    let handler = ClipboardPanelEventHandler::new();
    let app_handle = window.app_handle().clone();
    handler.window_did_resign_key(move |_| {
        remember_frontmost_external_app();
        if should_keep_clipboard_panel_visible() || !is_clipboard_panel_visible() {
            return;
        }

        // AppKit can deliver a stale resign event during show_and_make_key. Check
        // one run-loop later: a newer show changes the generation, while a real
        // outside click leaves this panel visible but no longer key.
        let generation = overlay_show_generation().load(AtomicOrdering::SeqCst);
        let handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(16)).await;
            if !is_current_presentation(generation)
                || !is_clipboard_panel_visible()
                || should_keep_clipboard_panel_visible()
            {
                return;
            }

            let _ = crate::app::window_manager::hide_window_on_resign(handle, generation);
        });
    });
    panel.set_event_handler(Some(handler.as_ref()));

    eprintln!("[macos-overlay] main window converted to NSPanel");
    Ok(())
}

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};

#[cfg(target_os = "macos")]
fn clipboard_panel_visible() -> &'static AtomicBool {
    static VISIBLE: AtomicBool = AtomicBool::new(false);
    &VISIBLE
}

#[cfg(target_os = "macos")]
pub fn is_clipboard_panel_visible() -> bool {
    clipboard_panel_visible().load(AtomicOrdering::SeqCst)
}

#[cfg(target_os = "macos")]
pub fn is_clipboard_panel_available(app: &AppHandle) -> bool {
    use tauri_nspanel::ManagerExt;
    app.get_webview_panel("main").is_ok()
}

#[cfg(target_os = "macos")]
fn should_keep_clipboard_panel_visible() -> bool {
    crate::global_state::is_blur_ignored()
        || crate::global_state::WINDOW_PINNED.load(AtomicOrdering::Relaxed)
}

/// Bumps on every show/hide so deferred reinforce work can cancel itself.
#[cfg(target_os = "macos")]
fn overlay_show_generation() -> &'static AtomicU64 {
    static SHOW_GEN: AtomicU64 = AtomicU64::new(0);
    &SHOW_GEN
}

#[cfg(target_os = "macos")]
fn overlay_presented_generation() -> &'static AtomicU64 {
    static PRESENTED_GEN: AtomicU64 = AtomicU64::new(0);
    &PRESENTED_GEN
}

/// Show the non-activating panel and make it key without activating the app.
#[cfg(target_os = "macos")]
pub fn set_clipboard_panel_lowered(app: &AppHandle, owner: String, lowered: bool) -> tauri::Result<()> {
    {
        let mut owners = lowered_owners().lock().unwrap();
        if lowered {
            owners.insert(owner);
        } else {
            owners.remove(&owner);
        }
    }
    refresh_clipboard_panel_policy(app)
}

pub fn show_clipboard_panel(app: &AppHandle) -> bool {
    use std::sync::atomic::Ordering;
    use tauri_nspanel::{CollectionBehavior, ManagerExt};

    if app.get_webview_panel("main").is_err() {
        eprintln!("[macos-overlay] get_webview_panel(main) failed");
        return false;
    }

    let gen = overlay_show_generation().fetch_add(1, AtomicOrdering::SeqCst) + 1;
    clipboard_panel_visible().store(true, AtomicOrdering::SeqCst);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    crate::global_state::LAST_SHOW_TIMESTAMP.store(now, Ordering::Relaxed);

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // Match EcoPaste's one-frame delay so Dock/hotkey processing finishes
        // before AppKit changes the key panel.
        tokio::time::sleep(std::time::Duration::from_millis(16)).await;
        if overlay_show_generation().load(AtomicOrdering::SeqCst) != gen
            || !is_clipboard_panel_visible()
        {
            return;
        }

        let app_for_panel = handle.clone();
        if let Err(err) = handle.run_on_main_thread(move || {
            if overlay_show_generation().load(AtomicOrdering::SeqCst) != gen
                || !is_clipboard_panel_visible()
            {
                return;
            }

            if let Ok(panel) = app_for_panel.get_webview_panel("main") {
                // Attached once; a no-op unless it hasn't been yet.
                if let Some(window) = app_for_panel.get_webview_window("main") {
                    crate::app::window_manager::apply_window_vibrancy(&app_for_panel, &window);
                }
                apply_panel_policy(&app_for_panel);
                // This is the hidden -> visible transition, so there is no live
                // editor caret to preserve. Initializing the panel content view
                // as first responder is required for first-mouse delivery after
                // a pinned panel has resigned key to another application.
                panel.show_and_make_key();
                // That leaves the content view, not the WebView inside it, as
                // first responder, so keys (Esc, arrows) never reached the page
                // until a click. Hand the keyboard back to the WebView.
                if let Some(window) = app_for_panel.get_webview_window("main") {
                    crate::app::window_manager::invalidate_window_shadow(&window);
                    let _ = window.with_webview(|webview| unsafe {
                        let ns_window = &*(webview.ns_window() as *const objc2_app_kit::NSWindow);
                        let web_view = &*(webview.inner() as *const objc2_app_kit::NSView);
                        let _ = ns_window.makeFirstResponder(Some(&**web_view));
                    });
                }
                panel.set_collection_behavior(
                    CollectionBehavior::new()
                        .full_screen_auxiliary()
                        .can_join_all_spaces()
                        .stationary()
                        .into(),
                );
                overlay_presented_generation().store(gen, AtomicOrdering::SeqCst);
                crate::global_state::NAVIGATION_ENABLED.store(true, Ordering::SeqCst);
                use tauri::Emitter;
                let _ = app_for_panel.emit("clipboard-shown", ());
                eprintln!("[macos-overlay] panel.show_and_make_key() gen={gen}");
            } else if overlay_show_generation().load(AtomicOrdering::SeqCst) == gen {
                clipboard_panel_visible().store(false, AtomicOrdering::SeqCst);
                overlay_presented_generation().store(0, AtomicOrdering::SeqCst);
            }
        }) {
            if overlay_show_generation().load(AtomicOrdering::SeqCst) == gen {
                clipboard_panel_visible().store(false, AtomicOrdering::SeqCst);
                overlay_presented_generation().store(0, AtomicOrdering::SeqCst);
            }
            eprintln!("[macos-overlay] schedule panel show failed: {err}");
        }
    });
    true
}

/// A Dock policy transition may resign the key panel. Keep that transient
/// event from closing the settings window, without resurrecting a hidden panel.
#[cfg(target_os = "macos")]
pub fn set_dock_visibility_preserving_panel(app: &AppHandle, visible: bool) -> tauri::Result<()> {
    static TRANSITION: AtomicU64 = AtomicU64::new(0);
    let ticket = TRANSITION.fetch_add(1, AtomicOrdering::SeqCst) + 1;
    let owner = format!("dock-visibility:{ticket}");
    let was_visible = is_clipboard_panel_visible();
    let generation = overlay_show_generation().load(AtomicOrdering::SeqCst);
    if was_visible {
        crate::global_state::acquire_blur_guard(&owner);
    }
    if let Err(error) = app.set_dock_visibility(visible) {
        if was_visible { crate::global_state::release_blur_guard(&owner); }
        return Err(error);
    }
    if was_visible {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let panel_handle = handle.clone();
            let owner_on_main = owner.clone();
            let result = handle.run_on_main_thread(move || {
                if TRANSITION.load(AtomicOrdering::SeqCst) == ticket
                    && overlay_show_generation().load(AtomicOrdering::SeqCst) == generation
                    && is_clipboard_panel_visible()
                {
                    let _ = make_clipboard_panel_key(&panel_handle);
                }
                crate::global_state::release_blur_guard(&owner_on_main);
            });
            if result.is_err() { crate::global_state::release_blur_guard(&owner); }
        });
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn make_clipboard_panel_key(app: &AppHandle) -> bool {
    use tauri_nspanel::ManagerExt;

    let handle = app.clone();
    let app_for_panel = handle.clone();
    handle
        .run_on_main_thread(move || {
            if let Ok(panel) = app_for_panel.get_webview_panel("main") {
                panel.order_front_regardless();
                panel.make_key_window();
            }
        })
        .is_ok()
}

pub fn can_hide_clipboard_panel_on_resign(app: &AppHandle, generation: u64) -> bool {
    use tauri_nspanel::ManagerExt;

    if !is_current_presentation(generation)
        || !is_clipboard_panel_visible()
        || should_keep_clipboard_panel_visible()
    {
        return false;
    }
    app.get_webview_panel("main")
        .is_ok_and(|panel| panel.is_visible() && !panel.as_panel().isKeyWindow())
}

fn is_current_presentation(generation: u64) -> bool {
    overlay_show_generation().load(AtomicOrdering::SeqCst) == generation
        && overlay_presented_generation().load(AtomicOrdering::SeqCst) == generation
}

#[cfg(target_os = "macos")]
pub fn resign_clipboard_panel_key(app: &AppHandle) -> bool {
    use tauri_nspanel::ManagerExt;

    let handle = app.clone();
    let app_for_panel = handle.clone();
    handle
        .run_on_main_thread(move || {
            if let Ok(panel) = app_for_panel.get_webview_panel("main") {
                panel.resign_key_window();
            }
        })
        .is_ok()
}

/// Hide the clipboard panel and re-arm move_to_active_space for the next show.
#[cfg(target_os = "macos")]
pub fn hide_clipboard_panel(app: &AppHandle) -> bool {
    use tauri_nspanel::{CollectionBehavior, ManagerExt};

    // Cancel reinforce retries from the previous show.
    let gen = overlay_show_generation().fetch_add(1, AtomicOrdering::SeqCst) + 1;
    clipboard_panel_visible().store(false, AtomicOrdering::SeqCst);
    overlay_presented_generation().store(0, AtomicOrdering::SeqCst);

    if app.get_webview_panel("main").is_err() {
        return false;
    }

    let handle = app.clone();
    let app_for_panel = handle.clone();
    let scheduled = handle
        .run_on_main_thread(move || {
            // A newer show may have overtaken this queued hide.
            if overlay_show_generation().load(AtomicOrdering::SeqCst) != gen
                || is_clipboard_panel_visible()
            {
                return;
            }
            if let Ok(panel) = app_for_panel.get_webview_panel("main") {
                panel.hide();
                panel.set_collection_behavior(
                    CollectionBehavior::new()
                        .full_screen_auxiliary()
                        .move_to_active_space()
                        .stationary()
                        .into(),
                );
                eprintln!("[macos-overlay] panel.hide() gen={gen}");
                // The page stops its timers until the next clipboard-shown.
                use tauri::Emitter;
                let _ = app_for_panel.emit("clipboard-hidden", ());
            }
        })
        .is_ok();

    if !scheduled && overlay_show_generation().load(AtomicOrdering::SeqCst) == gen {
        clipboard_panel_visible().store(true, AtomicOrdering::SeqCst);
    }
    scheduled
}

/// Kept for startup re-apply paths that still hold a WebviewWindow handle.
#[cfg(target_os = "macos")]
pub fn configure_overlay_space_behavior(window: &tauri::WebviewWindow) {
    use tauri_nspanel::{CollectionBehavior, ManagerExt};

    let app = window.app_handle();
    if let Ok(panel) = app.get_webview_panel("main") {
        let behavior = if is_clipboard_panel_visible() {
            CollectionBehavior::new()
                .full_screen_auxiliary()
                .can_join_all_spaces()
                .stationary()
        } else {
            CollectionBehavior::new()
                .full_screen_auxiliary()
                .move_to_active_space()
                .stationary()
        };
        panel.set_collection_behavior(behavior.into());
        return;
    }

    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
    if let Ok(ns_window_ptr) = window.ns_window() {
        let ns_window = ns_window_ptr as *const NSWindow;
        if ns_window.is_null() {
            return;
        }
        unsafe {
            let current = (*ns_window).collectionBehavior()
                & !NSWindowCollectionBehavior::MoveToActiveSpace
                & !NSWindowCollectionBehavior::FullScreenPrimary
                & !NSWindowCollectionBehavior::FullScreenNone
                & !NSWindowCollectionBehavior::Stationary;
            (*ns_window).setCollectionBehavior(
                current
                    | NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
            (*ns_window).setHidesOnDeactivate(false);
        }
    }
}

#[cfg(target_os = "macos")]
pub fn configure_overlay_space_behavior_async(window: &tauri::WebviewWindow) {
    let window_for_main = window.clone();
    let _ = window.run_on_main_thread(move || {
        configure_overlay_space_behavior(&window_for_main);
    });
}



/// After panel conversion, show/hide go through [`show_clipboard_panel`].
#[cfg(target_os = "macos")]
pub fn present_overlay_above_fullscreen_async(window: &tauri::WebviewWindow) {
    let app = window.app_handle().clone();
    let _ = window.run_on_main_thread(move || {
        let _ = show_clipboard_panel(&app);
    });
}

/// Re-apply Space flags after tao/window-state settles — without calling show again
/// (repeated show_and_make_key steals the first click / fights hide).
#[cfg(target_os = "macos")]
pub fn reinforce_overlay_above_fullscreen(window: &tauri::WebviewWindow) {
    use tauri_nspanel::{CollectionBehavior, ManagerExt};

    let gen = overlay_show_generation().load(AtomicOrdering::SeqCst);
    let app_for_retry = window.app_handle().clone();
    std::thread::spawn(move || {
        for delay_ms in [50_u64, 200] {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            if overlay_show_generation().load(AtomicOrdering::SeqCst) != gen {
                return;
            }
            let handle = app_for_retry.clone();
            let _ = handle.clone().run_on_main_thread(move || {
                if overlay_show_generation().load(AtomicOrdering::SeqCst) != gen {
                    return;
                }
                if let Ok(panel) = handle.get_webview_panel("main") {
                    apply_panel_policy(&handle);
                    panel.set_collection_behavior(
                        CollectionBehavior::new()
                            .full_screen_auxiliary()
                            .can_join_all_spaces()
                            .stationary()
                            .into(),
                    );
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{panel_policy, PanelLevel, WindowPinMode};

    #[test]
    fn panel_policy_covers_pinning_modes_and_temporary_lowering() {
        use WindowPinMode::{AlwaysOnTop, Normal};
        let dock = PanelLevel::Dock.value();
        let normal = PanelLevel::Normal.value();
        for (pinned, mode, lowered, expected) in [
            (false, AlwaysOnTop, false, (dock, true, false)),
            (false, AlwaysOnTop, true, (normal, false, false)),
            (false, Normal, false, (dock, true, false)),
            (false, Normal, true, (normal, false, false)),
            (true, AlwaysOnTop, false, (dock, true, true)),
            (true, AlwaysOnTop, true, (normal, false, true)),
            (true, Normal, false, (normal, false, false)),
            (true, Normal, true, (normal, false, false)),
        ] {
            assert_eq!(panel_policy(pinned, mode, lowered), expected);
        }
    }
}
