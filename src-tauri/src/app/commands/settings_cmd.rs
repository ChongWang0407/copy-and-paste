use crate::app::commands::hotkey_cmd::sync_registered_hotkeys;
use crate::app_state::{PasteQueue, SettingsState};
use crate::database::DbState;
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn set_sequential_mode(
    app_handle: AppHandle,
    state: State<'_, crate::app_state::SettingsState>,
    enabled: bool,
) {
    state.sequential_mode.store(enabled, Ordering::Relaxed);
    let db_state = app_handle.state::<DbState>();
    let _ = db_state
        .settings_repo
        .set("app.sequential_mode", &enabled.to_string());

    if !enabled {
        let paste_queue = app_handle.state::<PasteQueue>();
        let mut queue = paste_queue.inner().0.lock().unwrap();
        queue.items.clear();
        queue.last_action_was_paste = false;
        queue.last_pasted_content = None;
        queue.last_pasted_fingerprint = None;
    }

    let _ = sync_registered_hotkeys(&app_handle);
}

#[tauri::command]
pub fn set_sequential_hotkey(
    app_handle: AppHandle,
    state: State<'_, SettingsState>,
    hotkey: String,
) -> AppResult<()> {
    if let Ok(mut guard) = state.sequential_paste_hotkey.lock() {
        *guard = hotkey.clone();
    }

    let db_state = app_handle.state::<DbState>();
    db_state
        .settings_repo
        .set("app.sequential_hotkey", &hotkey)
        .map_err(AppError::from)?;

    sync_registered_hotkeys(&app_handle)
}




#[tauri::command]
pub fn set_deduplication(
    app_handle: AppHandle,
    state: State<'_, crate::app_state::SettingsState>,
    enabled: bool,
) {
    state.deduplicate.store(enabled, Ordering::Relaxed);
    let db_state = app_handle.state::<DbState>();
    let _ = db_state
        .settings_repo
        .set("app.deduplicate", &enabled.to_string());
}

#[tauri::command]
pub fn save_setting(
    app_handle: AppHandle,
    db_state: State<'_, DbState>,
    settings_state: State<'_, crate::app_state::SettingsState>,
    key: String,
    value: String,
) -> AppResult<()> {
    match key.as_str() {
        "app.arrow_key_selection" => {
            settings_state
                .arrow_key_selection
                .store(value == "true", Ordering::Relaxed);
        }
        "app.sequential_mode" => {
            settings_state
                .sequential_mode
                .store(value == "true", Ordering::Relaxed);
        }
        "app.persistent" => {
            settings_state
                .persistent
                .store(value != "false", Ordering::Relaxed);
        }
        "app.silent_start" => {
            settings_state
                .silent_start
                .store(value == "true", Ordering::Relaxed);
        }
        "app.delete_after_paste" => {
            settings_state
                .delete_after_paste
                .store(value == "true", Ordering::Relaxed);
        }
        "app.hide_tray_icon" => {
            settings_state
                .hide_tray_icon
                .store(value == "true", Ordering::Relaxed);
        }
        "app.hide_dock_icon" => {
            settings_state
                .hide_dock_icon
                .store(value == "true", Ordering::Relaxed);
        }
        "app.window_position_mode" => {
            if let Ok(mut guard) = settings_state.window_position_mode.lock() {
                *guard = crate::app::window_manager::normalize_position_mode(&value).to_string();
            }
        }
        crate::app::capsule::ENABLED_KEY => {
            let enabled = value != "false";
            settings_state.capsule_enabled.store(enabled, Ordering::Relaxed);
            if !enabled {
                crate::app::capsule::dismiss(&app_handle);
            }
        }
        crate::app::capsule::DURATION_KEY => {
            settings_state
                .capsule_duration_ms
                .store(crate::app::capsule::normalize_duration(&value), Ordering::Relaxed);
        }
        crate::app::capsule::STYLE_KEY => {
            if let Ok(mut guard) = settings_state.capsule_style.lock() {
                *guard = crate::app::capsule::normalize_style(&value).to_string();
            }
        }
        _ => {}
    }

    db_state
        .settings_repo
        .set(&key, &value)
        .map_err(AppError::from)
}


#[tauri::command]
pub fn acquire_blur_guard(owner: String) -> Result<usize, String> {
    let owner = owner.trim();
    if owner.is_empty() {
        return Err("Blur guard owner cannot be empty".to_string());
    }
    Ok(crate::global_state::acquire_blur_guard(owner))
}

#[tauri::command]
pub fn release_blur_guard(owner: String) -> Result<usize, String> {
    let owner = owner.trim();
    if owner.is_empty() {
        return Err("Blur guard owner cannot be empty".to_string());
    }
    Ok(crate::global_state::release_blur_guard(owner))
}

#[tauri::command]
pub fn set_window_pinned(app_handle: AppHandle, pinned: bool) {
    set_pinned_state(&app_handle, pinned);
}

/// Pinned = above every window, survives losing focus, and only Esc closes it.
pub fn set_pinned_state(app_handle: &AppHandle, pinned: bool) {
    crate::WINDOW_PINNED.store(pinned, Ordering::Relaxed);
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.set_always_on_top(pinned);
        #[cfg(target_os = "macos")]
        {
            // NSPanel focusability cannot be changed through Tauri after panel
            // conversion. In pinned mode, let ordinary clicks pass through
            // without making the clipboard key; editable controls opt in via
            // the shared focus coordinator.
            let _ = crate::infrastructure::macos_api::window::set_clipboard_panel_pinned_interaction(
                app_handle,
                pinned,
            );
        }
    }
    if let Some(state) = app_handle.try_state::<DbState>() {
        let _ = state
            .settings_repo
            .set("app.window_pinned", &pinned.to_string());
    }
}

#[tauri::command]
pub fn get_settings(
    state: State<'_, DbState>,
) -> AppResult<std::collections::HashMap<String, String>> {
    state.settings_repo.get_all().map_err(AppError::from)
}






#[tauri::command]
pub fn set_persistence(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.persistent.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.persistent", &enabled.to_string())
        .map_err(AppError::from)
}





#[tauri::command]
pub fn set_silent_start(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.silent_start.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.silent_start", &enabled.to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_delete_after_paste(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.delete_after_paste.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.delete_after_paste", &enabled.to_string())
        .map_err(AppError::from)
}




#[tauri::command]
pub fn set_cleanup_rules(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    rules: String,
) -> AppResult<()> {
    let mut guard = state.cleanup_rules.lock().unwrap();
    *guard = rules.clone();
    db_state
        .settings_repo
        .set("app.cleanup_rules", &rules)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_app_cleanup_policies(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    policies: String,
) -> AppResult<()> {
    let mut guard = state.app_cleanup_policies.lock().unwrap();
    *guard = policies.clone();
    db_state
        .settings_repo
        .set("app.app_cleanup_policies", &policies)
        .map_err(AppError::from)
}












#[tauri::command]
pub fn reset_settings(
    app: AppHandle,
    state: State<'_, DbState>,
    settings_state: State<'_, crate::app_state::SettingsState>,
) -> AppResult<()> {
    use crate::database::seed_defaults;

    state.settings_repo.clear().map_err(AppError::from)?;
    {
        let conn = state.conn.lock().unwrap();
        seed_defaults(&conn).map_err(AppError::from)?;
    }
    // Running state follows the defaults now, not after the next start.
    let fresh = crate::app::setup::load_settings(&state.settings_repo);
    settings_state.take_values_from(crate::app::setup::settings_state_from(&fresh));
    *crate::global_state::HOTKEY_STRING.lock().unwrap() = fresh.main_hotkey.clone();
    // The window applies the rest when it reloads, but not these two.
    let _ = set_tray_visible(app.clone(), app.state(), !fresh.hide_tray_icon);
    let _ = set_dock_visible(app.clone(), app.state(), !fresh.hide_dock_icon);

    sync_registered_hotkeys(&app)
}

#[tauri::command]
pub fn set_tray_visible(
    app_handle: AppHandle,
    state: State<'_, crate::app_state::SettingsState>,
    visible: bool,
) -> AppResult<()> {
    state.hide_tray_icon.store(!visible, Ordering::Relaxed);
    if let Some(tray) = app_handle.tray_by_id("main_tray") {
        let _ = tray.set_visible(visible);
    }
    let db_state = app_handle.state::<DbState>();
    db_state
        .settings_repo
        .set("app.hide_tray_icon", &(!visible).to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_dock_visible(
    app_handle: AppHandle,
    state: State<'_, crate::app_state::SettingsState>,
    visible: bool,
) -> AppResult<()> {
    state.hide_dock_icon.store(!visible, Ordering::Relaxed);
    #[cfg(target_os = "macos")]
    {
        crate::infrastructure::macos_api::window::set_dock_visibility_preserving_panel(
            &app_handle, visible,
        ).map_err(AppError::from)?;
        if !visible {
            // Tao intentionally drops a hide within one second of showing the
            // Dock icon. Retry beyond that interval, but respect a newer toggle.
            let app = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
                let app_on_main = app.clone();
                let _ = app.run_on_main_thread(move || {
                    let settings = app_on_main.state::<crate::app_state::SettingsState>();
                    if settings.hide_dock_icon.load(Ordering::Relaxed) {
                        if let Err(error) = crate::infrastructure::macos_api::window::set_dock_visibility_preserving_panel(&app_on_main, false) {
                            eprintln!("Failed to retry hiding Dock icon: {error}");
                        }
                    }
                });
            });
        }
    }
    let db_state = app_handle.state::<DbState>();
    db_state
        .settings_repo
        .set("app.hide_dock_icon", &(!visible).to_string())
        .map_err(AppError::from)
}


