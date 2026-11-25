use crate::database::{
    TRANSLATE_DEEPL_KEY, TRANSLATE_OPENAI_BASE_URL, TRANSLATE_OPENAI_KEY, TRANSLATE_OPENAI_MODEL,
    TRANSLATE_PROVIDER,
};
use crate::infrastructure::repository::settings_repo::SqliteSettingsRepository;
use reqwest::Url;
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, MutexGuard};

use super::remote::openai_endpoint;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-4o-mini";

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Apple,
    DeepL,
    OpenAi,
}

impl Provider {
    fn setting(self) -> &'static str {
        match self {
            Self::Apple => "apple",
            Self::DeepL => "deepl",
            Self::OpenAi => "openai",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationConfig {
    pub provider: Provider,
    openai_base_url: String,
    openai_model: String,
    deepl_key_set: bool,
    openai_key_set: bool,
    apple_supported: bool,
    pub available: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationDraft {
    provider: Provider,
    openai_base_url: String,
    openai_model: String,
    deepl_key: Option<String>,
    openai_key: Option<String>,
    #[serde(default)]
    clear_deepl_key: bool,
    #[serde(default)]
    clear_openai_key: bool,
}

pub enum TranslationService {
    Apple,
    DeepL {
        key: String,
    },
    OpenAi {
        endpoint: Url,
        model: String,
        key: Option<String>,
    },
}

fn read(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    SqliteSettingsRepository::get_stored(conn, key).map_err(|_| "config_unavailable".into())
}

fn read_provider(conn: &Connection) -> Result<Provider, String> {
    Ok(match read(conn, TRANSLATE_PROVIDER)?.as_deref() {
        Some("deepl") => Provider::DeepL,
        Some("openai") => Provider::OpenAi,
        _ => Provider::Apple,
    })
}

fn read_base_url(conn: &Connection) -> Result<String, String> {
    Ok(read(conn, TRANSLATE_OPENAI_BASE_URL)?.unwrap_or_else(|| DEFAULT_BASE_URL.into()))
}

pub fn get_config(conn: &Connection, apple_supported: bool) -> Result<TranslationConfig, String> {
    let provider = read_provider(conn)?;
    let openai_base_url = read_base_url(conn)?;
    let openai_model = read(conn, TRANSLATE_OPENAI_MODEL)?.unwrap_or_else(|| DEFAULT_MODEL.into());
    let deepl_key_set = read(conn, TRANSLATE_DEEPL_KEY)?.is_some_and(|value| !value.is_empty());
    let openai_key_set = read(conn, TRANSLATE_OPENAI_KEY)?.is_some_and(|value| !value.is_empty());
    let available = match provider {
        Provider::Apple => apple_supported,
        Provider::DeepL => deepl_key_set,
        Provider::OpenAi => {
            !openai_model.trim().is_empty() && openai_endpoint(&openai_base_url).is_ok()
        }
    };
    Ok(TranslationConfig {
        provider,
        openai_base_url,
        openai_model,
        deepl_key_set,
        openai_key_set,
        apple_supported,
        available,
    })
}

enum KeyChange<'a> {
    Keep,
    Replace(&'a str),
    Clear,
}

fn key_change(new: Option<&str>, clear: bool) -> KeyChange<'_> {
    if clear {
        return KeyChange::Clear;
    }
    match new.map(str::trim).filter(|key| !key.is_empty()) {
        Some(key) => KeyChange::Replace(key),
        None => KeyChange::Keep,
    }
}

/// Same scheme, host and port: a key is never reused for another service,
/// including a different port on the same machine.
fn same_host(saved_base_url: &str, next_endpoint: Option<&Url>) -> bool {
    match (openai_endpoint(saved_base_url).ok(), next_endpoint) {
        (Some(saved), Some(next)) => saved.origin() == next.origin(),
        _ => false,
    }
}

fn encode_new_key(key_name: &str, change: KeyChange<'_>) -> Result<Option<String>, String> {
    match change {
        KeyChange::Replace(key) => SqliteSettingsRepository::encode_value(key_name, key)
            .map(Some)
            .map_err(|_| "secret_storage_unavailable".into()),
        _ => Ok(None),
    }
}

fn store(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
        params![key, value],
    )
    .map(|_| ())
    .map_err(|_| "config_unavailable".into())
}

fn store_key(
    conn: &Connection,
    key: &str,
    encrypted: Option<String>,
    clear: bool,
) -> Result<(), String> {
    if let Some(encrypted) = encrypted {
        store(conn, key, &encrypted)
    } else if clear {
        conn.execute("DELETE FROM settings WHERE key = ?", [key])
            .map(|_| ())
            .map_err(|_| "config_unavailable".into())
    } else {
        Ok(())
    }
}

pub fn save_config(
    conn: &Mutex<Connection>,
    draft: TranslationDraft,
    apple_supported: bool,
) -> Result<TranslationConfig, String> {
    let base_url = draft.openai_base_url.trim();
    let model = draft.openai_model.trim();
    let next_endpoint = if base_url.is_empty() {
        None
    } else {
        Some(openai_endpoint(base_url)?)
    };
    let deepl = key_change(draft.deepl_key.as_deref(), draft.clear_deepl_key);
    let openai = key_change(draft.openai_key.as_deref(), draft.clear_openai_key);
    let clear_deepl = matches!(deepl, KeyChange::Clear);
    let clear_openai = matches!(openai, KeyChange::Clear);
    // Keychain may prompt. Do this before locking the shared database.
    let encrypted_deepl = encode_new_key(TRANSLATE_DEEPL_KEY, deepl)?;
    let encrypted_openai = encode_new_key(TRANSLATE_OPENAI_KEY, openai)?;

    let mut conn = conn.lock().unwrap();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| "config_unavailable".to_string())?;
    let saved_base_url = read_base_url(&tx)?;
    let host_changed = !same_host(&saved_base_url, next_endpoint.as_ref());
    store(&tx, TRANSLATE_PROVIDER, draft.provider.setting())?;
    store(&tx, TRANSLATE_OPENAI_BASE_URL, base_url)?;
    store(&tx, TRANSLATE_OPENAI_MODEL, model)?;
    store_key(&tx, TRANSLATE_DEEPL_KEY, encrypted_deepl, clear_deepl)?;
    store_key(
        &tx,
        TRANSLATE_OPENAI_KEY,
        encrypted_openai,
        clear_openai || host_changed,
    )?;
    let config = get_config(&tx, apple_supported)?;
    tx.commit().map_err(|_| "config_unavailable".to_string())?;
    Ok(config)
}

// Snapshot the routing parameters and stored key together. Decrypt after
// releasing the database lock, and only for the service making the request.
pub fn service_for_request(
    conn: &Mutex<Connection>,
    draft: Option<TranslationDraft>,
) -> Result<TranslationService, String> {
    let conn = conn.lock().unwrap();
    let provider = match &draft {
        Some(draft) => draft.provider,
        None => read_provider(&conn)?,
    };
    match provider {
        Provider::Apple => Ok(TranslationService::Apple),
        Provider::DeepL => {
            let change = draft
                .as_ref()
                .map(|draft| key_change(draft.deepl_key.as_deref(), draft.clear_deepl_key))
                .unwrap_or(KeyChange::Keep);
            let key = request_key(conn, TRANSLATE_DEEPL_KEY, change)?.ok_or("missing_config")?;
            Ok(TranslationService::DeepL { key })
        }
        Provider::OpenAi => {
            let saved_base_url = read_base_url(&conn)?;
            let (base_url, model, change) = match draft.as_ref() {
                Some(draft) => (
                    draft.openai_base_url.trim(),
                    draft.openai_model.trim().to_string(),
                    key_change(draft.openai_key.as_deref(), draft.clear_openai_key),
                ),
                None => (
                    saved_base_url.trim(),
                    read(&conn, TRANSLATE_OPENAI_MODEL)?.unwrap_or_else(|| DEFAULT_MODEL.into()),
                    KeyChange::Keep,
                ),
            };
            if base_url.is_empty() || model.trim().is_empty() {
                return Err("missing_config".into());
            }
            let endpoint = openai_endpoint(base_url)?;
            let change = if matches!(change, KeyChange::Keep)
                && !same_host(&saved_base_url, Some(&endpoint))
            {
                KeyChange::Clear
            } else {
                change
            };
            let key = request_key(conn, TRANSLATE_OPENAI_KEY, change)?;
            Ok(TranslationService::OpenAi {
                endpoint,
                model,
                key,
            })
        }
    }
}

fn request_key(
    conn: MutexGuard<'_, Connection>,
    key_name: &str,
    change: KeyChange<'_>,
) -> Result<Option<String>, String> {
    match change {
        KeyChange::Replace(key) => Ok(Some(key.to_string())),
        KeyChange::Clear => Ok(None),
        KeyChange::Keep => {
            let stored = read(&conn, key_name)?;
            drop(conn);
            Ok(stored
                .map(|value| {
                    SqliteSettingsRepository::decode_value(key_name, value)
                        .map_err(|_| "secret_storage_unavailable".to_string())
                })
                .transpose()?
                .filter(|key| !key.is_empty()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::encryption;
    use crate::infrastructure::repository::settings_repo::SettingsRepository;
    use std::sync::Arc;

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        conn
    }

    fn draft(
        provider: Provider,
        openai_base_url: &str,
        openai_key: Option<&str>,
    ) -> TranslationDraft {
        serde_json::from_value(serde_json::json!({
            "provider": provider,
            "openaiBaseUrl": openai_base_url,
            "openaiModel": DEFAULT_MODEL,
            "openaiKey": openai_key,
        }))
        .unwrap()
    }

    #[test]
    fn get_settings_omits_both_translation_keys() {
        let conn = database();
        store(&conn, TRANSLATE_DEEPL_KEY, "plain:example:fx").unwrap();
        store(
            &conn,
            &TRANSLATE_OPENAI_KEY.to_uppercase(),
            "plain:example-openai",
        )
        .unwrap();
        store(&conn, TRANSLATE_PROVIDER, "deepl").unwrap();
        let repo = SqliteSettingsRepository::new(Arc::new(Mutex::new(conn)));
        assert_eq!(
            repo.get_all().unwrap().into_iter().collect::<Vec<_>>(),
            vec![(TRANSLATE_PROVIDER.into(), "deepl".into())],
        );
    }

    #[test]
    fn config_availability_changes_when_saved_provider_or_key_changes() {
        let conn = Mutex::new(database());
        assert!(!get_config(&conn.lock().unwrap(), false).unwrap().available);
        assert!(get_config(&conn.lock().unwrap(), true).unwrap().available);

        let deepl =
            save_config(&conn, draft(Provider::DeepL, DEFAULT_BASE_URL, None), true).unwrap();
        assert!(!deepl.available);
        assert!(!deepl.deepl_key_set);
        let local = save_config(
            &conn,
            draft(Provider::OpenAi, "http://localhost:11434/v1", None),
            false,
        )
        .unwrap();
        assert!(local.available);
        assert!(!local.openai_key_set);

        let schema = serde_json::to_value(local).unwrap();
        assert_eq!(
            schema,
            serde_json::json!({
                "provider": "openai",
                "openaiBaseUrl": "http://localhost:11434/v1",
                "openaiModel": DEFAULT_MODEL,
                "deeplKeySet": false,
                "openaiKeySet": false,
                "appleSupported": false,
                "available": true,
            })
        );
    }

    #[test]
    fn empty_key_saves_keep_ciphertext_and_explicit_clear_removes_it() {
        let conn = Mutex::new(database());
        let mut first = draft(Provider::DeepL, DEFAULT_BASE_URL, Some("example-openai"));
        first.deepl_key = Some("example:fx".into());
        let saved = save_config(&conn, first, false).unwrap();
        assert!(saved.available && saved.deepl_key_set && saved.openai_key_set);
        let service = service_for_request(&conn, None).unwrap();
        assert!(matches!(service, TranslationService::DeepL { key } if key == "example:fx"));
        let before = {
            let conn = conn.lock().unwrap();
            [
                read(&conn, TRANSLATE_DEEPL_KEY).unwrap().unwrap(),
                read(&conn, TRANSLATE_OPENAI_KEY).unwrap().unwrap(),
            ]
        };
        assert!(before
            .iter()
            .all(|value| encryption::is_modern_ciphertext(value)));
        let mut next = draft(
            Provider::DeepL,
            "https://api.openai.com/proxy/v1",
            Some("  "),
        );
        next.deepl_key = Some(String::new());
        save_config(&conn, next, false).unwrap();
        {
            let conn = conn.lock().unwrap();
            assert_eq!(
                read(&conn, TRANSLATE_DEEPL_KEY).unwrap().as_ref(),
                Some(&before[0])
            );
            assert_eq!(
                read(&conn, TRANSLATE_OPENAI_KEY).unwrap().as_ref(),
                Some(&before[1])
            );
        }

        let mut clear = draft(Provider::DeepL, "https://api.openai.com/proxy/v1", None);
        clear.clear_deepl_key = true;
        let cleared = save_config(&conn, clear, false).unwrap();
        assert!(!cleared.available && !cleared.deepl_key_set);
        assert!(cleared.openai_key_set);
    }

    #[test]
    fn openai_host_change_drops_old_key_for_testing_and_after_saving() {
        let conn = database();
        store(&conn, TRANSLATE_OPENAI_KEY, "plain:example-old-host").unwrap();
        store(&conn, TRANSLATE_PROVIDER, "openai").unwrap();
        let conn = Mutex::new(conn);
        let same = service_for_request(
            &conn,
            Some(draft(
                Provider::OpenAi,
                "https://API.OPENAI.COM/proxy/v1",
                None,
            )),
        )
        .unwrap();
        assert!(
            matches!(same, TranslationService::OpenAi { key: Some(key), .. } if key == "example-old-host")
        );
        let different = "http://localhost:11434/v1";
        let tested =
            service_for_request(&conn, Some(draft(Provider::OpenAi, different, None))).unwrap();
        assert!(matches!(
            tested,
            TranslationService::OpenAi { key: None, .. }
        ));
        assert!(
            get_config(&conn.lock().unwrap(), false)
                .unwrap()
                .openai_key_set
        );
        let replaced = service_for_request(
            &conn,
            Some(draft(Provider::OpenAi, different, Some("example-new-host"))),
        )
        .unwrap();
        assert!(
            matches!(replaced, TranslationService::OpenAi { key: Some(key), .. } if key == "example-new-host")
        );
        let saved = save_config(&conn, draft(Provider::OpenAi, different, None), false).unwrap();
        assert!(!saved.openai_key_set);
        assert!(read(&conn.lock().unwrap(), TRANSLATE_OPENAI_KEY)
            .unwrap()
            .is_none());
    }

    #[test]
    fn translation_settings_roll_back_when_database_rejects_a_write() {
        let conn = database();
        store(&conn, TRANSLATE_OPENAI_KEY, "plain:example-old-host").unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_model BEFORE INSERT ON settings
             WHEN NEW.key = 'app.translate_openai_model'
             BEGIN SELECT RAISE(ABORT, 'read only'); END;",
        )
        .unwrap();
        let conn = Mutex::new(conn);
        let result = save_config(
            &conn,
            draft(Provider::OpenAi, "http://localhost:11434/v1", None),
            false,
        );
        assert!(matches!(result, Err(error) if error == "config_unavailable"));
        let config = get_config(&conn.lock().unwrap(), false).unwrap();
        assert!(config.provider == Provider::Apple && config.openai_key_set);
        assert_eq!(config.openai_base_url, DEFAULT_BASE_URL);
    }
}
