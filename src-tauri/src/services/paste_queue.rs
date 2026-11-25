use crate::app_state::{PasteQueue, SessionHistory};
use crate::app::commands::hotkey_cmd::sync_registered_hotkeys_on_main_thread;
use crate::database::DbState;
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use crate::services::clipboard_ops::{
    handle_post_paste_actions, handoff_macos_focus_for_paste, paste_to_target_and_then, PasteTarget,
};
use tauri::{Emitter, Manager};




#[tauri::command]
pub fn paste_next_step(app_handle: tauri::AppHandle) {
    // Run on async runtime so we can await focus-restoration delays.
    tauri::async_runtime::spawn(async move {
        paste_next_step_inner(app_handle).await;
    });
}

async fn paste_next_step_inner(app_handle: tauri::AppHandle) {
    // One step at a time: a second press waits until this item's keystroke is out.
    let _paste_guard = crate::services::clipboard_ops::PASTE_LOCK.lock().await;
    let state = app_handle.state::<PasteQueue>();
    let db_state = app_handle.state::<DbState>();
    let session = app_handle.state::<SessionHistory>();

    // Check if the app window is currently visible.
    // If the window is hidden (user pressed hotkey while the app was in the background),
    // focus is already in the target app - we must NOT do any focus restoration,
    // as that would steal focus FROM the target app to a wrong app.
    let window_was_visible = app_handle
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    let target = if window_was_visible {
        PasteTarget::previous_app()
    } else {
        PasteTarget::foreground_app()
    };

    // Keep the queue head until we know whether the target received the keystroke.
    let source = {
        let mut queue = state.inner().0.lock().unwrap();
        queue.last_action_was_paste = true;
        queue.items.front().copied()
    };

    if let Some(source) = source {
        let id = source.id;
        // 2. Get Content (DB Lock acquired here, safe because Queue lock is released)
        let entry_opt = if id < 0 {
            let s = session.inner().0.lock().unwrap();
            s.iter().find(|entry| source.matches(entry)).cloned()
        } else {
            db_state.repo.get_entry_by_id(id).unwrap_or(None).filter(|entry| source.matches(entry))
        };

        if let Some(entry) = entry_opt {
            let c_type = entry.content_type;
            let content = crate::services::clipboard_ops::with_current_entry_file(
                &app_handle,
                &c_type,
                entry.content,
            );
            let html_content = entry.html_content;
            let is_pinned = entry.is_pinned;
            let has_tags = !entry.tags.is_empty();

            // 3. Write content to system clipboard using the same payload logic as normal paste.
            match crate::services::clipboard_ops::prepare_clipboard_payload(
                &content,
                &c_type,
                html_content.as_deref(),
                // No click to choose from: rich entries keep their format (some must paste as exact text).
                crate::services::clipboard_ops::resolve_paste_with_format(
                    &content,
                    &c_type,
                    html_content.as_deref(),
                    None,
                ),
            )
            .await
            {
                Err(err) => {
                    eprintln!(
                        "[ERROR] Failed to prepare clipboard payload for sequential paste: {err}"
                    );
                    // A bad payload is skipped so the queue can continue on the next step.
                    let _ = app_handle.emit("queue-item-pasted", id);
                }
                Ok(()) => {
                    let delay = match target {
                        PasteTarget::PreviousApp(pid) => {
                            handoff_macos_focus_for_paste(pid);
                            60
                        }
                        PasteTarget::ForegroundApp(_) => 20,
                    };
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;

                    let result = paste_to_target_and_then(target, async {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

                        let settings_state = app_handle.state::<crate::app_state::SettingsState>();
                        let delete_after_paste = settings_state
                            .delete_after_paste
                            .load(std::sync::atomic::Ordering::Relaxed);

                        let delete = delete_after_paste && !is_pinned && !has_tags;
                        if id > 0 || delete {
                            match handle_post_paste_actions(
                                &app_handle, &db_state, Some(source), delete, Some(false),
                            ) {
                                Ok(true) => { let _ = app_handle.emit("clipboard-changed", ()); }
                                Ok(false) => {}
                                Err(err) => eprintln!("[ERROR] Sequential paste cleanup failed: {err}"),
                            }
                        }
                        Ok(())
                    })
                    .await;
                    if let Err(err) = result {
                        eprintln!("[ERROR] Sequential paste keystroke failed: {err}");
                        return;
                    }

                    let _ = app_handle.emit("queue-item-pasted", id);
                }
            }
        }
        let mut queue = state.inner().0.lock().unwrap();
        // A new copy or disabling the mode may have replaced the queue meanwhile.
        if queue.last_action_was_paste && queue.items.front() == Some(&source) {
            queue.items.pop_front();
        }
    } else {
        let _ = app_handle.emit("queue-finished", ());
    }

    let queue_empty = {
        let queue = state.inner().0.lock().unwrap();
        queue.items.is_empty()
    };
    if queue_empty {
        // This runs on the async runtime.
        sync_registered_hotkeys_on_main_thread(&app_handle);
    }
}
