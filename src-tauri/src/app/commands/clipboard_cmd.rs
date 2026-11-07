use crate::app_state::{AppDataDir, SessionHistory};
use crate::database::{self, DbState};
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use tauri::{AppHandle, Emitter, Manager, State};

fn truncate_chars_with_suffix(text: &str, max_chars: usize, suffix: &str) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut = text
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    let mut out = String::with_capacity(cut + suffix.len());
    out.push_str(&text[..cut]);
    out.push_str(suffix);
    out
}

#[tauri::command]
pub fn toggle_clipboard_pin(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    app_data_dir: State<'_, AppDataDir>,
    id: i64,
    is_pinned: bool,
) -> AppResult<()> {
    let mut real_id = id;
    let mut entry_to_save = None;

    {
        let mut session_items = session.inner().0.lock().unwrap();
        if let Some(item) = session_items.iter_mut().find(|i| i.id == id) {
            item.is_pinned = is_pinned;
            if id < 0 && is_pinned {
                entry_to_save = Some(item.clone());
            }
        }
    }

    let mut persisted_id = None;
    let pin_result = {
        // Take other locks before the connection, never while holding it.
        let data_dir = entry_to_save
            .as_ref()
            .map(|_| app_data_dir.0.lock().unwrap().clone());
        let conn = state.conn.lock().unwrap();

        if let (Some(entry), Some(data_dir)) = (entry_to_save, data_dir) {
            if let Ok(new_id) = state
                .repo
                .save_with_conn(&conn, &entry, Some(&data_dir), None)
            {
                real_id = new_id;
                persisted_id = Some(new_id);
                if let Ok(deleted_ids) = state.repo.enforce_limit_with_conn(&conn, Some(&data_dir))
                {
                    for deleted_id in deleted_ids {
                        let _ = app_handle.emit("clipboard-removed", deleted_id);
                    }
                }
            }
        }

        if real_id > 0 {
            state.repo.toggle_pin_with_conn(&conn, real_id, is_pinned)
        } else {
            Ok(())
        }
    };

    if let Some(new_id) = persisted_id {
        let mut session_items = session.inner().0.lock().unwrap();
        // Once persisted, the database is the sole owner of this entry.
        session_items.retain(|item| item.id != id && item.id != new_id);
    }
    pin_result.map_err(AppError::from)?;

    let _ = app_handle.emit("clipboard-changed", ());
    Ok(())
}



#[tauri::command]
pub async fn add_manual_item(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    content: String,
    content_type: String,
    tags: Vec<String>,
) -> AppResult<i64> {
    let preview = truncate_chars_with_suffix(&content, 200, "...");
    let timestamp = chrono::Utc::now().timestamp_millis();

    let entry = database::ClipboardEntry {
        id: 0,
        content_type,
        content,
        html_content: None,
        source_app: "Manual".to_string(),
        source_app_path: None,
        note: String::new(),
        ocr_text: None,
        ocr_match: false,
        timestamp,
        created_at: timestamp,
        last_used_at: 0,
        sort_at: timestamp,
        preview,
        is_pinned: false,
        tags,
        use_count: 0,
        is_external: false,
        pinned_order: 0,
        file_preview_exists: true,
    };

    let app_data_dir = app_handle.state::<AppDataDir>();
    let data_dir = app_data_dir.0.lock().unwrap().clone();
    let new_id = state.repo.save(&entry, Some(&data_dir))?;
    let _ = app_handle.emit("clipboard-changed", ());
    Ok(new_id)
}

#[tauri::command]
pub async fn update_item_content(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    id: i64,
    new_content: String,
) -> AppResult<()> {
    let preview = truncate_chars_with_suffix(&new_content, 500, "...");

    {
        let mut session_items = session.inner().0.lock().unwrap();
        if let Some(item) = session_items.iter_mut().find(|i| i.id == id) {
            item.content = new_content.clone();
            item.preview = preview.clone();
            // Edited as plain text: the old rich format no longer matches.
            item.html_content = None;
            if item.content_type == "rich_text" {
                item.content_type = "text".to_string();
            }
        }
    }

    if id < 0 {
        let _ = app_handle.emit("clipboard-changed", ());
        return Ok(());
    }

    state
        .repo
        .update_entry_content(id, &new_content, &preview)
        .map_err(AppError::from)?;
    let _ = app_handle.emit("clipboard-changed", ());
    Ok(())
}
