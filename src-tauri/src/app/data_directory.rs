//! Move the default data folder without changing the application's macOS identity.
//! The single-instance plugin runs before setup. A journal travels with the atomic
//! directory rename so a crash can resume before the database or background jobs open.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};

const JOURNAL: &str = ".data-directory-migration-v1";

pub fn default_directory(legacy: &Path, development: bool) -> PathBuf {
    legacy.with_file_name(if development {
        "com.copypaste.dev"
    } else {
        "com.copypaste"
    })
}

fn error(context: &str, err: impl std::fmt::Display) -> String {
    format!("数据目录迁移失败（{context}）：{err}。原有数据已保留，请解决问题后重新启动。")
}

/// Never merge databases or overwrite a destination containing unknown data.
pub fn prepare(legacy: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        if !destination.is_dir() {
            return Err(error("目标目录", "目标位置不是文件夹"));
        }
        if destination.join(JOURNAL).exists() {
            return Ok(());
        }
        let empty = std::fs::read_dir(destination)
            .map_err(|e| error("检查目标目录", e))?
            .next()
            .is_none();
        if !empty {
            if destination.join("clipboard.db").is_file()
                || destination.join("datapath.txt").is_file()
            {
                if destination.join("clipboard.db").is_file() {
                    check_database(&destination.join("clipboard.db"))?;
                }
                return Ok(());
            }
            return Err(error("目标目录", "已有其他文件，未合并或覆盖"));
        }
        if !legacy.exists() {
            return Ok(());
        }
        std::fs::remove_dir(destination).map_err(|e| error("准备空目标目录", e))?;
    }
    if !legacy.exists() {
        return std::fs::create_dir_all(destination).map_err(|e| error("创建目录", e));
    }
    if !legacy.is_dir() {
        return Err(error("旧目录", "旧位置不是文件夹"));
    }
    // Fail before moving a corrupt database, and do not create an empty replacement.
    if legacy.join("clipboard.db").exists() {
        check_database(&legacy.join("clipboard.db"))?;
    }
    let journal = legacy.join(JOURNAL);
    if !journal.exists() {
        use std::io::Write;
        let temporary = legacy.join(format!("{JOURNAL}.tmp"));
        let mut file = std::fs::File::create(&temporary).map_err(|e| error("创建迁移日志", e))?;
        file.write_all(b"1\n")
            .and_then(|_| file.sync_all())
            .map_err(|e| error("保存迁移日志", e))?;
        std::fs::rename(&temporary, &journal).map_err(|e| error("提交迁移日志", e))?;
    }
    // Both directories have the same parent/filesystem. This carries WAL/SHM,
    // attachments, favorites, dev keys, file permissions and user-owned files together.
    std::fs::rename(legacy, destination).map_err(|e| error("移动目录", e))
}

fn check_database(path: &Path) -> Result<(), String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| error("读取原数据库", e))?;
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(|e| error("检查数据库", e))?;
    if check != "ok" {
        return Err(error("检查数据库", check));
    }
    Ok(())
}

pub fn redirected_directory(default: &Path) -> Result<PathBuf, String> {
    let redirect = default.join("datapath.txt");
    let value = match std::fs::read_to_string(&redirect) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(default.into()),
        Err(e) => return Err(error("读取自定义目录", e)),
    };
    let path = PathBuf::from(value.trim());
    if value.trim().is_empty() {
        return Ok(default.into());
    }
    if !path.is_dir() || value.trim().to_ascii_lowercase().ends_with(".app") {
        return Err(error(
            "自定义目录",
            "保存的位置当前不可用，未创建空白记录库",
        ));
    }
    Ok(path)
}

/// Called after encryption::init and before init_db. On failure, the journal
/// remains and SQL transactions roll back. Repeating after a crash is idempotent.
pub fn finish(legacy: &Path, destination: &Path, effective: &Path) -> Result<(), String> {
    finish_with(
        legacy,
        destination,
        effective,
        |value| crate::infrastructure::encryption::decrypt_stored_value(value),
        |value| crate::infrastructure::encryption::encrypt_for_storage(value),
    )
}

fn finish_with<D, E>(
    legacy: &Path,
    destination: &Path,
    effective: &Path,
    decrypt: D,
    encrypt: E,
) -> Result<(), String>
where
    D: Fn(&str) -> Result<String, String>,
    E: Fn(&str) -> Result<String, String>,
{
    let journal = destination.join(JOURNAL);
    if !journal.exists() {
        return Ok(());
    }
    if std::fs::read_to_string(&journal)
        .map_err(|e| error("读取迁移日志", e))?
        .trim()
        != "1"
    {
        return Err(error("迁移日志", "日志格式无效"));
    }
    for directory in [destination, effective] {
        let path = directory.join("clipboard.db");
        if path.exists() {
            rewrite_database(&path, legacy, destination, &decrypt, &encrypt)?;
            check_database(&path)?;
        }
        if directory == effective {
            break;
        }
    }
    std::fs::remove_file(journal).map_err(|e| error("完成迁移", e))
}

pub fn migrate_redirect(legacy: &Path, destination: &Path) -> Result<(), String> {
    if !destination.join(JOURNAL).exists() {
        return Ok(());
    }
    let path = destination.join("datapath.txt");
    if !path.exists() {
        return Ok(());
    }
    let value = std::fs::read_to_string(&path).map_err(|e| error("读取目录配置", e))?;
    let previous = Path::new(value.trim());
    let Ok(relative) = previous.strip_prefix(legacy) else {
        return Ok(());
    };
    let updated = destination.join(relative);
    let temporary = destination.join("datapath.txt.migrating");
    std::fs::write(&temporary, updated.to_string_lossy().as_bytes())
        .map_err(|e| error("写入目录配置", e))?;
    std::fs::rename(temporary, path).map_err(|e| error("更新目录配置", e))
}

fn rewrite_text(value: &str, legacy: &Path, destination: &Path) -> String {
    let old = legacy.to_string_lossy();
    let new = destination.to_string_lossy();
    let encoded = |s: &str| {
        s.split('/')
            .map(|part| urlencoding::encode(part).into_owned())
            .collect::<Vec<_>>()
            .join("/")
    };
    let mut result = value.to_owned();
    for (from, to) in [
        (old.to_string(), new.to_string()),
        (encoded(&old), encoded(&new)),
        (
            urlencoding::encode(&old).into_owned(),
            urlencoding::encode(&new).into_owned(),
        ),
    ] {
        if result == from {
            result = to;
        } else {
            result = result.replace(&format!("{from}/"), &format!("{to}/"));
            result = result.replace(&format!("{from}%2F"), &format!("{to}%2F"));
        }
    }
    result
}

fn rewrite_value<D, E>(
    value: &str,
    legacy: &Path,
    destination: &Path,
    decrypt: &D,
    encrypt: &E,
) -> Result<String, String>
where
    D: Fn(&str) -> Result<String, String>,
    E: Fn(&str) -> Result<String, String>,
{
    if crate::infrastructure::encryption::is_encrypted_value(value) {
        let plain = decrypt(value).map_err(|e| error("解密路径", e))?;
        let rewritten = rewrite_text(&plain, legacy, destination);
        if plain == rewritten {
            return Ok(value.into());
        }
        return encrypt(&rewritten).map_err(|e| error("加密路径", e));
    }
    Ok(rewrite_text(value, legacy, destination))
}

fn rewrite_database<D, E>(
    path: &Path,
    legacy: &Path,
    destination: &Path,
    decrypt: &D,
    encrypt: &E,
) -> Result<(), String>
where
    D: Fn(&str) -> Result<String, String>,
    E: Fn(&str) -> Result<String, String>,
{
    let mut conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| error("打开数据库", e))?;
    conn.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|e| error("锁定数据库", e))?;
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| error("开始路径迁移", e))?;
    for table in ["clipboard_history", "favorites"] {
        let mut statement = transaction
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|e| error("检查数据字段", e))?;
        let columns = statement
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(|e| error("检查数据字段", e))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| error("读取数据字段", e))?;
        drop(statement);
        if columns.is_empty() {
            continue;
        }
        for column in ["content", "html_content"] {
            if !columns.iter().any(|v| v == column) {
                continue;
            }
            let condition = if column == "content" {
                if columns.iter().any(|v| v == "is_external") {
                    "(is_external = 1 OR content_type IN ('image','video','file','audio'))"
                } else {
                    "content_type IN ('image','video','file','audio')"
                }
            } else {
                "1 = 1"
            };
            let mut statement = transaction
                .prepare(&format!(
                    "SELECT id, {column} FROM {table} WHERE {column} IS NOT NULL AND {condition}"
                ))
                .map_err(|e| error("读取媒体路径", e))?;
            let rows = statement
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
                .map_err(|e| error("读取媒体路径", e))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| error("读取媒体路径", e))?;
            drop(statement);
            for (id, value) in rows {
                let updated = rewrite_value(&value, legacy, destination, decrypt, encrypt)?;
                if updated != value {
                    transaction
                        .execute(
                            &format!("UPDATE {table} SET {column} = ?1 WHERE id = ?2"),
                            rusqlite::params![updated, id],
                        )
                        .map_err(|e| error("更新媒体路径", e))?;
                }
            }
        }
    }
    let settings_exist: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='settings')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| error("检查设置", e))?;
    if settings_exist {
        let value: Option<String> = transaction
            .query_row(
                "SELECT value FROM settings WHERE key='app.custom_background'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| error("读取背景路径", e))?;
        if let Some(value) = value {
            let updated = rewrite_value(&value, legacy, destination, decrypt, encrypt)?;
            if updated != value {
                transaction
                    .execute(
                        "UPDATE settings SET value=?1 WHERE key='app.custom_background'",
                        [updated],
                    )
                    .map_err(|e| error("更新背景路径", e))?;
            }
        }
    }
    transaction.commit().map_err(|e| error("提交路径迁移", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("copypaste-directory-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn old(&self) -> PathBuf {
            self.0.join("com.wangchong.copypaste")
        }
        fn new_path(&self) -> PathBuf {
            self.0.join("com.copypaste")
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn database(root: &Path) -> Connection {
        std::fs::create_dir_all(root).unwrap();
        let conn = Connection::open(root.join("clipboard.db")).unwrap();
        conn.execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY, content_type TEXT, content TEXT, html_content TEXT, is_external INTEGER); CREATE TABLE favorites(id INTEGER PRIMARY KEY, content_type TEXT, content TEXT, html_content TEXT); CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT);").unwrap();
        conn
    }
    fn finish_plain(old: &Path, new: &Path) -> Result<(), String> {
        finish_with(
            old,
            new,
            new,
            |v| Ok(v.trim_start_matches("enc1:").into()),
            |v| Ok(format!("enc1:{v}")),
        )
    }

    #[test]
    fn fresh_install_and_dev_are_separate() {
        let f = Folder::new();
        prepare(&f.old(), &f.new_path()).unwrap();
        assert!(f.new_path().is_dir());
        assert!(!f.old().exists());
        assert_eq!(
            default_directory(&f.old(), true),
            f.0.join("com.copypaste.dev")
        );
        assert_eq!(default_directory(&f.old(), false), f.new_path());
    }

    #[test]
    fn moves_all_files_rewrites_paths_and_preserves_plain_text_and_relative_favorites() {
        let f = Folder::new();
        let old = f.old();
        let new = f.new_path();
        let conn = database(&old);
        let image = old.join("attachments/image.png");
        std::fs::create_dir_all(image.parent().unwrap()).unwrap();
        std::fs::write(&image, b"image").unwrap();
        std::fs::create_dir(old.join("favorites")).unwrap();
        std::fs::write(old.join("favorites/star.png"), b"favorite").unwrap();
        std::fs::write(old.join("master.key"), [42; 32]).unwrap();
        conn.execute(
            "INSERT INTO clipboard_history VALUES(1,'image',?1,?2,1)",
            rusqlite::params![
                image.to_string_lossy(),
                format!("<img src='file://{}'>", image.display())
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clipboard_history VALUES(2,'text',?1,NULL,0)",
            [old.join("notes.txt").to_string_lossy()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO favorites VALUES(1,'image','star.png',NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO settings VALUES('app.custom_background',?1)",
            [old.join("background.png").to_string_lossy()],
        )
        .unwrap();
        drop(conn);
        prepare(&old, &new).unwrap();
        finish_plain(&old, &new).unwrap();
        assert!(!old.exists());
        assert_eq!(
            std::fs::read(new.join("attachments/image.png")).unwrap(),
            b"image"
        );
        assert_eq!(
            std::fs::read(new.join("favorites/star.png")).unwrap(),
            b"favorite"
        );
        assert_eq!(std::fs::read(new.join("master.key")).unwrap(), [42; 32]);
        let conn = Connection::open(new.join("clipboard.db")).unwrap();
        let (content, html): (String, String) = conn
            .query_row(
                "SELECT content,html_content FROM clipboard_history WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(content, new.join("attachments/image.png").to_string_lossy());
        assert!(html.contains(&content));
        assert_eq!(
            conn.query_row(
                "SELECT content FROM clipboard_history WHERE id=2",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            old.join("notes.txt").to_string_lossy()
        );
        assert_eq!(
            conn.query_row("SELECT content FROM favorites", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "star.png"
        );
        assert_eq!(
            conn.query_row("SELECT value FROM settings", [], |r| r.get::<_, String>(0))
                .unwrap(),
            new.join("background.png").to_string_lossy()
        );
        prepare(&old, &new).unwrap();
        finish_plain(&old, &new).unwrap();
        assert!(!new.join(JOURNAL).exists());
    }

    #[test]
    fn interrupted_rename_and_rewrite_resume_without_losing_encryption() {
        let f = Folder::new();
        let old = f.old();
        let new = f.new_path();
        let conn = database(&old);
        let value = format!("enc1:{}/attachments/private.png", old.display());
        conn.execute(
            "INSERT INTO clipboard_history VALUES(1,'image',?1,NULL,1)",
            [value],
        )
        .unwrap();
        drop(conn);
        std::fs::write(old.join(JOURNAL), "1\n").unwrap();
        prepare(&old, &new).unwrap();
        // A denied key must not downgrade encrypted paths to plaintext or clear the journal.
        let failed = finish_with(
            &old,
            &new,
            &new,
            |_| Err("key denied".into()),
            |v| Ok(v.into()),
        );
        assert!(failed.is_err());
        assert!(new.join(JOURNAL).exists());
        finish_plain(&old, &new).unwrap();
        let conn = Connection::open(new.join("clipboard.db")).unwrap();
        assert_eq!(
            conn.query_row("SELECT content FROM clipboard_history", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            format!("enc1:{}/attachments/private.png", new.display())
        );
    }

    #[test]
    fn new_database_wins_and_unknown_destination_is_not_overwritten() {
        let f = Folder::new();
        drop(database(&f.old()));
        drop(database(&f.new_path()));
        prepare(&f.old(), &f.new_path()).unwrap();
        assert!(f.old().join("clipboard.db").exists());
        let g = Folder::new();
        drop(database(&g.old()));
        std::fs::create_dir(g.new_path()).unwrap();
        std::fs::write(g.new_path().join("user-file"), "keep").unwrap();
        assert!(prepare(&g.old(), &g.new_path()).is_err());
        assert!(g.old().join("clipboard.db").exists());
    }

    #[test]
    fn custom_directory_is_retained_and_nested_redirect_follows_the_move() {
        let f = Folder::new();
        let custom = f.0.join("custom");
        drop(database(&custom));
        std::fs::create_dir(f.old()).unwrap();
        std::fs::write(f.old().join("datapath.txt"), custom.to_str().unwrap()).unwrap();
        prepare(&f.old(), &f.new_path()).unwrap();
        migrate_redirect(&f.old(), &f.new_path()).unwrap();
        assert_eq!(redirected_directory(&f.new_path()).unwrap(), custom);
        finish_with(
            &f.old(),
            &f.new_path(),
            &custom,
            |v| Ok(v.into()),
            |v| Ok(v.into()),
        )
        .unwrap();
        assert!(custom.join("clipboard.db").exists());
        let g = Folder::new();
        std::fs::create_dir_all(g.old().join("nested")).unwrap();
        std::fs::write(
            g.old().join("datapath.txt"),
            g.old().join("nested").to_str().unwrap(),
        )
        .unwrap();
        prepare(&g.old(), &g.new_path()).unwrap();
        migrate_redirect(&g.old(), &g.new_path()).unwrap();
        assert_eq!(
            redirected_directory(&g.new_path()).unwrap(),
            g.new_path().join("nested")
        );
    }

    #[test]
    fn invalid_custom_path_and_corrupt_database_fail_instead_of_starting_empty() {
        let f = Folder::new();
        std::fs::create_dir(f.old()).unwrap();
        std::fs::write(
            f.old().join("datapath.txt"),
            f.0.join("missing").to_str().unwrap(),
        )
        .unwrap();
        prepare(&f.old(), &f.new_path()).unwrap();
        migrate_redirect(&f.old(), &f.new_path()).unwrap();
        assert!(redirected_directory(&f.new_path()).is_err());
        let g = Folder::new();
        std::fs::create_dir(g.old()).unwrap();
        std::fs::write(g.old().join("clipboard.db"), b"not sqlite").unwrap();
        assert!(prepare(&g.old(), &g.new_path()).is_err());
        assert!(g.old().join("clipboard.db").exists());
    }

    #[test]
    fn rewrites_encoded_paths_without_rewriting_neighbor_directories() {
        let old = Path::new("/Users/A B/com.wangchong.copypaste");
        let new = Path::new("/Users/A B/com.copypaste");
        assert_eq!(
            rewrite_text(
                "file:///Users/A%20B/com.wangchong.copypaste/attachments/a.png",
                old,
                new
            ),
            "file:///Users/A%20B/com.copypaste/attachments/a.png"
        );
        assert_eq!(
            rewrite_text("/Users/A B/com.wangchong.copypaste-other/a", old, new),
            "/Users/A B/com.wangchong.copypaste-other/a"
        );
    }
    #[test]
    fn failure_after_an_update_rolls_back_and_can_resume() {
        let f = Folder::new();
        let old = f.old();
        let new = f.new_path();
        let conn = database(&old);
        conn.execute(
            "INSERT INTO clipboard_history VALUES(1,'image',?1,NULL,1)",
            [old.join("one.png").to_string_lossy()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clipboard_history VALUES(2,'image','enc1:locked',NULL,1)",
            [],
        )
        .unwrap();
        drop(conn);
        prepare(&old, &new).unwrap();
        assert!(finish_with(&old, &new, &new, |_| Err("denied".into()), |v| Ok(v.into())).is_err());
        let conn = Connection::open(new.join("clipboard.db")).unwrap();
        let value: String = conn
            .query_row(
                "SELECT content FROM clipboard_history WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(value, old.join("one.png").to_string_lossy());
        assert!(new.join(JOURNAL).exists());
        drop(conn);
        finish_plain(&old, &new).unwrap();
    }

    #[test]
    fn committed_wal_rows_survive_directory_move() {
        let f = Folder::new();
        let old = f.old();
        let new = f.new_path();
        let conn = database(&old);
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
        conn.execute(
            "INSERT INTO clipboard_history VALUES(1,'image',?1,NULL,1)",
            [old.join("wal.png").to_string_lossy()],
        )
        .unwrap();
        // Simulate a terminated process leaving committed pages in WAL.
        std::mem::forget(conn);
        assert!(old.join("clipboard.db-wal").is_file());
        prepare(&old, &new).unwrap();
        finish_plain(&old, &new).unwrap();
        let conn = Connection::open(new.join("clipboard.db")).unwrap();
        let value: String = conn
            .query_row("SELECT content FROM clipboard_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(value, new.join("wal.png").to_string_lossy());
    }

    #[cfg(not(feature = "portable"))]
    #[test]
    fn real_encrypted_media_remains_decryptable_after_migration() {
        use crate::infrastructure::encryption;
        let f = Folder::new();
        let old = f.old();
        let new = f.new_path();
        let conn = database(&old);
        let encrypted =
            encryption::encrypt_for_storage(&old.join("secret.png").to_string_lossy()).unwrap();
        conn.execute(
            "INSERT INTO clipboard_history VALUES(1,'image',?1,NULL,1)",
            [encrypted],
        )
        .unwrap();
        drop(conn);
        prepare(&old, &new).unwrap();
        finish(&old, &new, &new).unwrap();
        let conn = Connection::open(new.join("clipboard.db")).unwrap();
        let value: String = conn
            .query_row("SELECT content FROM clipboard_history", [], |r| r.get(0))
            .unwrap();
        assert!(value.starts_with("enc1:"));
        assert_eq!(
            encryption::decrypt_stored_value(&value).unwrap(),
            new.join("secret.png").to_string_lossy()
        );
    }
}
