use crate::app_state::AppDataDir;
use crate::error::{AppError, AppResult};
use serde_json;
use tauri::{AppHandle, Manager, State};

fn open_path_in_file_manager(path: &std::path::Path) -> AppResult<()> {
    if !path.exists() {
        return Err(AppError::Validation(format!(
            "Directory does not exist: {}",
            path.to_string_lossy()
        )));
    }

    use std::process::Command;

    #[cfg(target_os = "macos")]
    {
        let reveal = Command::new("open")
            .arg("-R")
            .arg(path)
            .output()
            .map_err(|e| AppError::Internal(format!("Failed to open folder: {}", e)))?;
        if reveal.status.success() {
            return Ok(());
        }

        let fallback = Command::new("open")
            .arg(path)
            .output()
            .map_err(|e| AppError::Internal(format!("Failed to open folder: {}", e)))?;
        if fallback.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&reveal.stderr).trim().to_string();
        let fallback_stderr = String::from_utf8_lossy(&fallback.stderr).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !fallback_stderr.is_empty() {
            fallback_stderr
        } else {
            "Unknown Finder error".to_string()
        };

        return Err(AppError::Internal(format!(
            "Failed to open folder: {}",
            detail
        )));
    }

    // explorer.exe exits with 1 even when the folder opened, so its exit code
    // reported every success as a failure. ShellExecute says whether it worked.

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let output = Command::new("xdg-open")
            .arg(path)
            .output()
            .map_err(|e| AppError::Internal(format!("Failed to open folder: {}", e)))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let detail = if stderr.is_empty() {
                "xdg-open returned a non-zero exit status".to_string()
            } else {
                stderr
            };
            return Err(AppError::Internal(format!(
                "Failed to open folder: {}",
                detail
            )));
        }
        return Ok(());
    }
}

fn content_type_to_ext(content_type: &str) -> String {
    let normalized = content_type.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "text" | "code" => "txt".to_string(),
        "rich_text" => "html".to_string(),
        "image" => "png".to_string(),
        "video" => "mp4".to_string(),
        "url" => "html".to_string(),
        other if !other.is_empty() => other.to_string(),
        _ => "txt".to_string(),
    }
}

#[tauri::command]
pub async fn scan_installed_apps() -> Vec<serde_json::Value> {
    #[cfg(target_os = "macos")]
    {
        crate::infrastructure::macos_api::apps::scan_installed_apps()
    }
}

#[tauri::command]
#[allow(non_snake_case)]
pub fn get_system_default_app(contentType: String) -> String {
    let ext = content_type_to_ext(&contentType);
    #[cfg(target_os = "macos")]
    {
        return crate::infrastructure::macos_api::apps::get_system_default_app(&ext);
    }
}

#[tauri::command]
pub async fn get_associated_apps(extension: String) -> Vec<serde_json::Value> {
    #[cfg(target_os = "macos")]
    {
        crate::infrastructure::macos_api::apps::get_associated_apps(&extension)
    }
}

#[tauri::command]
pub fn get_executable_icon(executable_path: String) -> AppResult<Option<String>> {
    #[cfg(target_os = "macos")]
    {
        return crate::infrastructure::macos_api::apps::get_executable_icon(executable_path)
            .map_err(AppError::Internal);
    }
}

#[tauri::command]
pub fn get_file_icon(file_path: String) -> AppResult<Option<String>> {
    #[cfg(target_os = "macos")]
    {
        return crate::infrastructure::macos_api::apps::get_file_icon(file_path).map_err(AppError::Internal);
    }
}

#[tauri::command]
pub fn get_data_path(state: State<'_, AppDataDir>) -> AppResult<String> {
    let path = state.0.lock().unwrap();
    Ok(path.to_string_lossy().to_string())
}


#[tauri::command]
pub fn open_folder(path: String) -> AppResult<()> {
    open_path_in_file_manager(std::path::Path::new(&path))
}




#[tauri::command]
pub fn toggle_autostart(enabled: bool) -> AppResult<()> {
    use crate::infrastructure::macos_api::login_item;
    let result = apply_autostart(enabled);
    if enabled && result.is_err() && login_item::available() && login_item::requires_approval() {
        // Switched off in System Settings, where only the user can switch
        // it back on: take them there instead of failing silently.
        login_item::open_settings();
    }
    result
}

/// `toggle_autostart` without sending the user anywhere; for the app's own use.
pub(crate) fn apply_autostart(enabled: bool) -> AppResult<()> {
    crate::infrastructure::macos_api::login_item::set_launch_at_login(enabled).map_err(AppError::Internal)
}

#[tauri::command]
pub fn is_autostart_enabled() -> AppResult<bool> {
    Ok(crate::infrastructure::macos_api::login_item::launch_at_login())
}

#[tauri::command]
pub fn set_windows_clipboard_history(enabled: bool) -> AppResult<()> {
    {
        let _ = enabled;
        Ok(())
    }
}

#[tauri::command]
pub fn get_windows_clipboard_history() -> AppResult<bool> {
    {
        Ok(true)
    }
}

#[tauri::command]
pub fn set_win_clipboard_disabled(disabled: bool) -> AppResult<()> {
    set_windows_clipboard_history(!disabled)
}

#[tauri::command]
pub fn trigger_registry_win_v_optimization(enable: bool) -> AppResult<bool> {
    {
        let _ = enable;
        Ok(false)
    }
}

#[tauri::command]
pub fn is_registry_win_v_optimized() -> AppResult<bool> {
    Ok(get_registry_win_v_optimized_status())
}

pub fn get_registry_win_v_optimized_status() -> bool {
    false
}

#[tauri::command]
pub fn restart_explorer() -> AppResult<()> {
    Ok(())
}



fn launch_after_exit(pid: u32, replacement: &std::path::Path) -> std::process::Command {
    // Wait for the old database connections and singleton socket to close.
    let mut command = std::process::Command::new("/bin/sh");
    command
        .args(["-c", r#"while kill -0 "$1" 2>/dev/null; do sleep 0.1; done; shift; exec "$@""#, "copypaste-relaunch"])
        .arg(pid.to_string())
        .arg(replacement)
        .stdin(std::process::Stdio::null());
    command
}

#[tauri::command]
pub fn relaunch(app: AppHandle) -> AppResult<()> {
    launch_after_exit(std::process::id(), &std::env::current_exe()?).spawn()?;
    crate::app::system::request_app_exit(&app);
    Ok(())
}

#[tauri::command]
pub fn check_is_admin() -> bool {
    false
}

#[tauri::command]
pub fn set_data_path(app_handle: AppHandle, new_path: String) -> AppResult<()> {
    if crate::app::setup::portable_data_dir().is_some() {
        return Err(AppError::Validation(
            "便携版的数据保存在程序旁的 data 文件夹，不能更换位置。".to_string(),
        ));
    }
    let source = app_handle.state::<AppDataDir>().0.lock().unwrap().clone();
    let config = crate::app::setup::default_app_data_dir(app_handle.path())?;
    crate::app::data_directory::request_move(&config, &source, std::path::Path::new(new_path.trim()))
        .map_err(AppError::from)
}

#[tauri::command]
pub fn check_macos_permissions() -> bool {
    #[cfg(target_os = "macos")]
    {
        return crate::infrastructure::macos_api::permissions::has_accessibility_permission();
    }
}

#[tauri::command]
pub fn request_macos_permissions(
    app_handle: AppHandle,
) -> crate::infrastructure::macos_api::permissions::PermissionRequest {
    #[cfg(target_os = "macos")]
    {
        return crate::infrastructure::macos_api::permissions::request_accessibility_permission(
            &app_handle.config().identifier,
        );
    }
}

#[tauri::command]
pub fn get_macos_accessibility_binary_path() -> String {
    #[cfg(target_os = "macos")]
    {
        return std::env::current_exe()
            .ok()
            .and_then(|path| path.into_os_string().into_string().ok())
            .unwrap_or_else(|| "copy-paste".to_string());
    }
}

#[tauri::command]
pub fn open_macos_accessibility_settings() -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        const URLS: &[&str] = &[
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Accessibility",
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            "x-apple.systempreferences:com.apple.preference.security",
        ];
        for url in URLS {
            if Command::new("open")
                .arg(url)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                return Ok(());
            }
        }
        if Command::new("open")
            .args(["-b", "com.apple.systempreferences"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Ok(());
        }
        return Err(AppError::Internal(
            "请手动打开系统「辅助功能」设置。".to_string(),
        ));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn launcher_waits_until_the_original_process_has_exited() {
        let folder = std::env::temp_dir().join(format!("copypaste-launch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&folder).unwrap();
        let marker = folder.join("launched");
        let mut original = std::process::Command::new("/bin/sleep").arg("0.25").spawn().unwrap();
        let mut replacement = super::launch_after_exit(original.id(), std::path::Path::new("/usr/bin/touch"))
            .arg(&marker).spawn().unwrap();
        assert!(!marker.exists());
        assert!(original.wait().unwrap().success());
        assert!(replacement.wait().unwrap().success());
        assert!(marker.is_file());
        std::fs::remove_dir_all(folder).unwrap();
    }
}
