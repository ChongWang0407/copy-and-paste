mod apple;
mod config;
mod remote;

use crate::database::DbState;
use config::{Provider, TranslationConfig, TranslationDraft, TranslationService};
use tauri::{AppHandle, Manager, State};

fn check_pair(source: &str, target: &str) -> Result<(), String> {
    match (source, target) {
        ("en", "zh-Hans") | ("zh-Hans", "en") => Ok(()),
        _ => Err("unsupported_pair".into()),
    }
}

#[tauri::command]
pub fn get_translation_config(state: State<'_, DbState>) -> Result<TranslationConfig, String> {
    config::get_config(&state.conn.lock().unwrap(), apple::supported())
}

#[tauri::command]
pub async fn save_translation_config(
    app: AppHandle,
    draft: TranslationDraft,
) -> Result<TranslationConfig, String> {
    tauri::async_runtime::spawn_blocking(move || {
        config::save_config(&app.state::<DbState>().conn, draft, apple::supported())
    })
    .await
    .map_err(|_| "config_unavailable".to_string())?
}

#[tauri::command]
pub fn translation_supported(state: State<'_, DbState>) -> bool {
    get_translation_config(state).is_ok_and(|config| config.available)
}

/// Third-party services are ready without an Apple language pack.
#[tauri::command]
pub async fn translation_status(
    state: State<'_, DbState>,
    source: String,
    target: String,
) -> Result<String, String> {
    check_pair(&source, &target)?;
    let config = get_translation_config(state)?;
    match config.provider {
        Provider::Apple => apple::status(&source, &target).await,
        _ if config.available => Ok("installed".into()),
        _ => Err("missing_config".into()),
    }
}

async fn translate_request(
    app: AppHandle,
    draft: Option<TranslationDraft>,
    text: &str,
    source: &str,
    target: &str,
) -> Result<String, String> {
    let app_for_config = app.clone();
    let service = tauri::async_runtime::spawn_blocking(move || {
        config::service_for_request(&app_for_config.state::<DbState>().conn, draft)
    })
    .await
    .map_err(|_| "config_unavailable".to_string())??;
    match service {
        TranslationService::Apple if apple::supported() => {
            apple::translate(&app, text, source, target).await
        }
        TranslationService::Apple => Err("unsupported_os".into()),
        TranslationService::DeepL { key } => remote::deepl(&key, text, source, target).await,
        TranslationService::OpenAi {
            endpoint,
            model,
            key,
        } => remote::openai(endpoint, &model, key.as_deref(), text, source, target).await,
    }
}

#[tauri::command]
pub async fn translate_text(
    app: AppHandle,
    text: String,
    source: String,
    target: String,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Ok(String::new());
    }
    check_pair(&source, &target)?;
    translate_request(app, None, &text, &source, &target).await
}

#[tauri::command]
pub async fn test_translation_config(
    app: AppHandle,
    draft: TranslationDraft,
) -> Result<String, String> {
    translate_request(
        app,
        Some(draft),
        "Hello, this is a translation test.",
        "en",
        "zh-Hans",
    )
    .await
}
