#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

pub mod app;
pub mod app_state;
pub mod database;
pub mod domain;
pub mod error;
pub mod global_state;
pub mod infrastructure;
pub mod logger;
pub mod services;

use crate::app::setup;
use crate::error::AppResult;
use crate::global_state::*;

fn main() -> AppResult<()> {
    // reqwest is built without a crypto provider and panics if none is installed.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let context = tauri::generate_context!();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    setup::handle_global_shortcut(app, shortcut, event.state());
                })
                .build(),
        );

    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    let app = builder
        // Launching the app again brings the running instance forward: the way
        // back when the tray icon is hidden and the hotkey is forgotten.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A login-item start (--minimized / --autostart) is not the user
            // asking for the window. On macOS, registering the login item
            // starts one right away while the app is already running.
            if args.iter().any(|arg| arg == "--minimized" || arg == "--autostart") {
                return;
            }
            let app = app.clone();
            std::thread::spawn(move || {
                let _ = app::window_manager::focus_clipboard_window(app);
            });
        }))
        .setup(|app| {
            if let Err(error) = setup::init(app) {
                use tauri::Manager;
                use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                for window in app.webview_windows().values() {
                    let _ = window.hide();
                }
                app.dialog()
                    .message(format!("无法启动 Copy & Paste。\n\n{error}"))
                    .title("Copy & Paste 启动失败")
                    .kind(MessageDialogKind::Error)
                    .show(|_| std::process::exit(1));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app::window_manager::toggle_window_cmd,
            app::window_manager::hide_window_cmd,
            app::window_manager::escape_window_cmd,
            app::window_manager::activate_window_focus,
            app::window_manager::activate_app_focus,
            app::window_manager::focus_clipboard_window,
            app::window_manager::set_glass_level,
            app::window_manager::supports_liquid_glass,
            app::window_manager::set_side_panel,
            app::window_manager::begin_fixed_position_setup,
            app::window_manager::save_fixed_window_position,
            app::window_manager::cancel_fixed_position_setup,
            app::window_manager::has_fixed_window_position,
            app::capsule::preview_capsule,
            app::hooks::set_recording_mode,
            services::content_handler::open_content,
            services::content_handler::share_content,
            services::clipboard_ops::copy_to_clipboard,
            services::clipboard_ops::paste_text,
            services::clipboard_ops::paste_entry_as_markdown,
            services::clipboard_ops::paste_image_text,
            services::clipboard_ops::paste_batch,
            services::translate::translation_supported,
            services::translate::translation_status,
            services::translate::translate_text,
            app::commands::get_clipboard_history,
            app::commands::search_clipboard_history,
            app::commands::delete_clipboard_entry,
            app::commands::clear_clipboard_history,
            app::commands::get_clipboard_content,
            app::commands::set_sequential_mode,
            app::commands::set_sequential_hotkey,
            app::commands::set_deduplication,
            app::commands::save_setting,
            app::commands::acquire_blur_guard,
            app::commands::release_blur_guard,
            app::commands::set_window_pinned,
            app::commands::get_settings,
            app::commands::set_persistence,
            app::commands::set_silent_start,
            app::commands::set_delete_after_paste,
            app::commands::set_cleanup_rules,
            app::commands::set_app_cleanup_policies,
            app::commands::reset_settings,
            app::commands::set_tray_visible,
            app::commands::set_dock_visible,
            app::commands::get_data_path,
            app::commands::scan_installed_apps,
            app::commands::get_system_default_app,
            app::commands::get_associated_apps,
            app::commands::get_executable_icon,
            app::commands::get_file_icon,
            app::commands::open_folder,
            app::commands::set_data_path,
            app::commands::toggle_autostart,
            app::commands::is_autostart_enabled,
            app::commands::relaunch,
            app::commands::check_macos_permissions,
            app::commands::request_macos_permissions,
            app::window_manager::set_panel_lowered,
            app::commands::get_macos_accessibility_binary_path,
            app::commands::open_macos_accessibility_settings,
            app::commands::register_hotkey,
            app::commands::test_hotkey_available,
            app::commands::toggle_clipboard_pin,
            app::commands::add_manual_item,
            app::commands::update_item_content,
            app::commands::get_image_thumbnail,
            app::commands::get_video_thumbnail,
            app::commands::list_favorites,
            app::commands::add_favorite_from_entry,
            app::commands::add_favorite_text,
            app::commands::update_favorite_text,
            app::commands::delete_favorite,
            app::commands::reorder_favorites,
            app::commands::paste_favorite,
            services::paste_queue::paste_next_step,
        ])
        .on_window_event(|window, event| {
            setup::handle_window_event(window, event);
        })
        .build(context);

    match app {
        Ok(app) => {
            info!(">>> [STARTUP] Tauri app built successfully.");
            app.run(|app_handle, event| {
                if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                    if !crate::app::system::ALLOW_APP_EXIT.load(std::sync::atomic::Ordering::SeqCst)
                    {
                        // Keep the tray + clipboard daemon alive; only Quit exits.
                        api.prevent_exit();
                    }
                }
                #[cfg(target_os = "macos")]
                if let tauri::RunEvent::Reopen { .. } = event {
                    let _ = crate::app::window_manager::focus_clipboard_window(app_handle.clone());
                }
            });
            Ok(())
        }
        Err(e) => {
            error!(">>> [STARTUP] Failed to build tauri app: {}", e);
            Err(e.into())
        }
    }
}
