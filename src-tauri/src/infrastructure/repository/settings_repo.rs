use crate::database::is_sensitive_key;
use crate::infrastructure::encryption;
use rusqlite::{params, Connection, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub trait SettingsRepository {
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn get_all(&self) -> Result<HashMap<String, String>>;
    /// Sensitive settings stored here that can't be decrypted on this device.
    fn undecryptable_keys(&self) -> Result<Vec<String>>;
    /// Deletes those of `keys` that are sensitive and can't be decrypted here
    /// (never a readable value); returns the ones removed.
    fn clear_undecryptable(&self, keys: &[String]) -> Result<Vec<String>>;
    fn clear(&self) -> Result<()>;
}

pub struct SqliteSettingsRepository {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteSettingsRepository {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn get_raw(conn: &Connection, key: &str) -> Result<Option<String>> {
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?")?;
        let mut rows = stmt.query(params![key])?;

        if let Some(row) = rows.next()? {
            let value: String = row.get(0)?;
            if is_sensitive_key(key) && encryption::is_encrypted_value(&value) {
                return Ok(Some(Self::decrypt_sensitive_value(key, &value)?));
            }
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn maybe_encrypt(&self, key: &str, value: &str) -> Result<String> {
        if is_sensitive_key(key) {
            return encryption::encrypt_for_storage(value).map_err(Self::encryption_error);
        }
        Ok(value.to_string())
    }

    fn maybe_decrypt(&self, key: &str, value: &str) -> Result<String> {
        if is_sensitive_key(key) && encryption::is_encrypted_value(value) {
            return Self::decrypt_sensitive_value(key, value);
        }
        Ok(value.to_string())
    }

    fn encryption_error(message: String) -> rusqlite::Error {
        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData, message,
        )))
    }

    fn decrypt_sensitive_value(key: &str, value: &str) -> Result<String> {
        encryption::decrypt_stored_value(value)
            .map_err(|e| Self::encryption_error(format!("sensitive setting '{key}': {e}")))
    }
}

impl SettingsRepository for SqliteSettingsRepository {
    fn set(&self, key: &str, value: &str) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if is_sensitive_key(key) && value.is_empty() {
            // An empty save must not replace a value that is only unreadable
            // for now (Keychain access denied, say). A new value is the user
            // replacing it on purpose, so it may: refusing that left a device
            // whose secret can't be decrypted unable to fix it.
            Self::get_raw(&tx, key)?;
        }
        let final_value = self.maybe_encrypt(key, value)?;

        tx.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
            params![key, final_value],
        )?;
        tx.commit()
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?")?;
        let mut rows = stmt.query(params![key])?;

        if let Some(row) = rows.next()? {
            let value: String = row.get(0)?;
            let decrypted = self.maybe_decrypt(key, &value)?;

            Ok(Some(decrypted))
        } else {
            Ok(None)
        }
    }

    fn get_all(&self) -> Result<HashMap<String, String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut settings = HashMap::new();
        for row in rows {
            let (key, value) = row?;
            match self.maybe_decrypt(&key, &value) {
                Ok(decrypted) => {
                    settings.insert(key, decrypted);
                }
                // One unreadable secret used to fail every setting, so the app
                // ran on defaults and settings sync stopped. It stays stored
                // untouched and is left out; see undecryptable_keys.
                Err(_) if is_sensitive_key(&key) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(settings)
    }

    fn undecryptable_keys(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        let mut keys = Vec::new();
        for row in rows {
            let (key, value) = row?;
            if self.maybe_decrypt(&key, &value).is_err() {
                keys.push(key);
            }
        }
        keys.sort();
        Ok(keys)
    }

    fn clear_undecryptable(&self, keys: &[String]) -> Result<Vec<String>> {
        let unreadable = self.undecryptable_keys()?;
        let conn = self.conn.lock().unwrap();
        let mut cleared = Vec::new();
        for key in keys.iter().filter(|key| is_sensitive_key(key) && unreadable.contains(key)) {
            conn.execute("DELETE FROM settings WHERE key = ?", params![key])?;
            cleared.push(key.clone());
        }
        Ok(cleared)
    }

    fn clear(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM settings", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteSettingsRepository;

    #[test]
    fn failed_reads_and_empty_saves_preserve_old_ciphertext() {
        use super::SettingsRepository;
        use std::sync::{Arc, Mutex};
        let conn = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()));
        conn.lock().unwrap().execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);").unwrap();
        let repo = SqliteSettingsRepository::new(conn.clone());
        for value in ["dpapi:not-base64", "enc1:not-base64"] {
            conn.lock().unwrap().execute("INSERT OR REPLACE INTO settings VALUES ('mqtt_password', ?1)", [value]).unwrap();
            assert!(repo.get("mqtt_password").is_err());
            assert!(repo.set("mqtt_password", "").is_err());
            let stored: String = conn.lock().unwrap().query_row("SELECT value FROM settings", [], |r| r.get(0)).unwrap();
            assert_eq!(stored, value);
        }
    }

    #[test]
    fn an_unreadable_secret_leaves_other_settings_readable_and_can_be_replaced() {
        use super::SettingsRepository;
        use std::sync::{Arc, Mutex};
        let conn = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()));
        conn.lock().unwrap().execute_batch(
            "CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO settings VALUES ('mqtt_password', 'enc1:not-base64');
             INSERT INTO settings VALUES ('app.theme', 'paper');",
        ).unwrap();
        let repo = SqliteSettingsRepository::new(conn.clone());

        let all = repo.get_all().expect("other settings still load");
        assert_eq!(all.get("app.theme").map(String::as_str), Some("paper"));
        assert!(!all.contains_key("mqtt_password"));
        assert_eq!(repo.undecryptable_keys().unwrap(), vec!["mqtt_password".to_string()]);

        // Clearing touches only unreadable secrets, never a readable value.
        conn.lock().unwrap().execute("INSERT INTO settings VALUES ('mqtt_username', 'plain:me')", []).unwrap();
        let cleared = repo
            .clear_undecryptable(&["mqtt_password".into(), "mqtt_username".into(), "app.theme".into()])
            .unwrap();
        assert_eq!(cleared, vec!["mqtt_password".to_string()]);
        assert_eq!(repo.get("mqtt_username").unwrap().as_deref(), Some("me"));
        assert_eq!(repo.get("app.theme").unwrap().as_deref(), Some("paper"));
        assert!(repo.undecryptable_keys().unwrap().is_empty());
        conn.lock().unwrap().execute("INSERT INTO settings VALUES ('mqtt_password', 'enc1:not-base64')", []).unwrap();

        // Typing a new password replaces the unreadable one.
        repo.set("mqtt_password", "new-secret").expect("replace with a new value");
        assert_eq!(repo.get("mqtt_password").unwrap().as_deref(), Some("new-secret"));
        assert!(repo.undecryptable_keys().unwrap().is_empty());
    }

    #[test]
    fn undecryptable_sensitive_values_are_never_returned_to_the_ui() {
        assert!(
            SqliteSettingsRepository::decrypt_sensitive_value(
                "mqtt_username",
                "enc1:not-valid-ciphertext"
            ).is_err()
        );
    }

    #[test]
    fn legacy_sensitive_values_still_decode() {
        assert_eq!(
            SqliteSettingsRepository::decrypt_sensitive_value(
                "mqtt_username",
                "plain:example-user"
            ).unwrap(),
            "example-user"
        );
    }
}
