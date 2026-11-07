use std::sync::atomic::Ordering;

use tauri::AppHandle;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::global_state::*;

/// While a hotkey is being recorded the clipboard is ignored, and registered
/// shortcuts step aside so pressing one reaches the recorder.
#[tauri::command]
pub fn set_recording_mode(app_handle: AppHandle, enabled: bool) -> Result<(), String> {
    IS_RECORDING.store(enabled, Ordering::SeqCst);
    CLIPBOARD_MONITOR_PAUSED.store(enabled, Ordering::Relaxed);
    if enabled {
        let _ = app_handle.global_shortcut().unregister_all();
        Ok(())
    } else {
        crate::app::commands::sync_registered_hotkeys(&app_handle).map_err(|e| e.to_string())
    }
}
