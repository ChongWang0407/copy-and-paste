use std::sync::atomic::Ordering;

use tauri::AppHandle;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::global_state::*;

/// While a hotkey is being recorded the clipboard is ignored, and registered
/// shortcuts step aside so pressing one reaches the recorder.
#[tauri::command]
pub fn set_recording_mode(app_handle: AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        app_handle.global_shortcut().unregister_all().map_err(|error| error.to_string())?;
        update_recording_flags(true);
    } else if update_recording_flags(false) {
        crate::app::commands::sync_registered_hotkeys(&app_handle).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn update_recording_flags(enabled: bool) -> bool {
    if enabled {
        pause_clipboard_monitor(PAUSE_HOTKEY_RECORDING);
    } else {
        resume_clipboard_monitor(PAUSE_HOTKEY_RECORDING);
    }
    IS_RECORDING.swap(enabled, Ordering::SeqCst)
}
