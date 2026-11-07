/// When true, `ExitRequested` is allowed to tear down the process (tray Quit etc.).
pub static ALLOW_APP_EXIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn request_app_exit(app: &tauri::AppHandle) {
    ALLOW_APP_EXIT.store(true, std::sync::atomic::Ordering::SeqCst);
    app.exit(0);
}
