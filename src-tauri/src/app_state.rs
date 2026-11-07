use crate::domain::models::ClipboardEntry;
use crate::services::encryption_queue::EncryptionQueue;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

pub struct SettingsState {
    pub deduplicate: AtomicBool,
    pub persistent: AtomicBool,
    /// Always on in Copy & Paste: the 文件 / 视频 tabs need file copies.
    pub capture_files: AtomicBool,
    /// Always on in Copy & Paste: 原格式 / Markdown paste needs the HTML.
    pub capture_rich_text: AtomicBool,
    pub silent_start: AtomicBool,
    pub delete_after_paste: AtomicBool,
    pub cleanup_rules: Mutex<String>,
    pub app_cleanup_policies: Mutex<String>,
    pub sequential_mode: AtomicBool,
    pub sequential_paste_hotkey: Mutex<String>,
    pub hide_tray_icon: AtomicBool,
    pub hide_dock_icon: AtomicBool,
    pub arrow_key_selection: AtomicBool,
    pub main_hotkey: Mutex<String>,
    /// `follow_mouse` | `last` | `fixed`
    pub window_position_mode: Mutex<String>,
    /// 0 (clearest) – 100 (most tinted).
    pub glass_level: AtomicU64,
    /// 复制提示弹窗: the pill that pops up at the top of the screen on each copy.
    pub capsule_enabled: AtomicBool,
    /// `glass` | `island` | `black`
    pub capsule_style: Mutex<String>,
    /// How long it stays out, in ms.
    pub capsule_duration_ms: AtomicU64,
}

impl SettingsState {
    /// Takes every value from `fresh` (「恢复初始设置」). No `..` below: a new
    /// field won't compile until it's handled here.
    pub fn take_values_from(&self, fresh: SettingsState) {
        let SettingsState {
            deduplicate,
            persistent,
            capture_files,
            capture_rich_text,
            silent_start,
            delete_after_paste,
            cleanup_rules,
            app_cleanup_policies,
            sequential_mode,
            sequential_paste_hotkey,
            hide_tray_icon,
            hide_dock_icon,
            arrow_key_selection,
            main_hotkey,
            window_position_mode,
            glass_level,
            capsule_enabled,
            capsule_style,
            capsule_duration_ms,
        } = fresh;
        self.deduplicate.store(deduplicate.into_inner(), Ordering::Relaxed);
        self.persistent.store(persistent.into_inner(), Ordering::Relaxed);
        self.capture_files.store(capture_files.into_inner(), Ordering::Relaxed);
        self.capture_rich_text.store(capture_rich_text.into_inner(), Ordering::Relaxed);
        self.silent_start.store(silent_start.into_inner(), Ordering::Relaxed);
        self.delete_after_paste.store(delete_after_paste.into_inner(), Ordering::Relaxed);
        *self.cleanup_rules.lock().unwrap() = cleanup_rules.into_inner().unwrap();
        *self.app_cleanup_policies.lock().unwrap() = app_cleanup_policies.into_inner().unwrap();
        self.sequential_mode.store(sequential_mode.into_inner(), Ordering::Relaxed);
        *self.sequential_paste_hotkey.lock().unwrap() = sequential_paste_hotkey.into_inner().unwrap();
        self.hide_tray_icon.store(hide_tray_icon.into_inner(), Ordering::Relaxed);
        self.hide_dock_icon.store(hide_dock_icon.into_inner(), Ordering::Relaxed);
        self.arrow_key_selection.store(arrow_key_selection.into_inner(), Ordering::Relaxed);
        *self.main_hotkey.lock().unwrap() = main_hotkey.into_inner().unwrap();
        *self.window_position_mode.lock().unwrap() = window_position_mode.into_inner().unwrap();
        self.glass_level.store(glass_level.into_inner(), Ordering::Relaxed);
        self.capsule_enabled.store(capsule_enabled.into_inner(), Ordering::Relaxed);
        *self.capsule_style.lock().unwrap() = capsule_style.into_inner().unwrap();
        self.capsule_duration_ms.store(capsule_duration_ms.into_inner(), Ordering::Relaxed);
    }
}

#[derive(Default)]
pub struct PasteQueueState {
    pub items: VecDeque<i64>,
    pub last_action_was_paste: bool,
    pub last_pasted_content: Option<String>,
    pub last_pasted_fingerprint: Option<String>,
    pub last_paste_timestamp_ms: u64,
}

#[derive(Default)]
pub struct PasteQueue(pub Mutex<PasteQueueState>);

pub struct SessionHistory(pub Mutex<VecDeque<ClipboardEntry>>);

pub struct AppDataDir(pub Mutex<std::path::PathBuf>);

pub struct EncryptionQueueState(pub EncryptionQueue);

#[cfg(test)]
mod tests {
    use crate::app::setup::{load_settings, settings_state_from};
    use crate::infrastructure::repository::settings_repo::{SettingsRepository, SqliteSettingsRepository};
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_settings_reset_reaches_the_running_state() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .unwrap();
        let repo = SqliteSettingsRepository::new(Arc::new(Mutex::new(conn)));
        let defaults = settings_state_from(&load_settings(&repo));
        repo.set("app.delete_after_paste", "true").unwrap();
        repo.set("app.cleanup_rules", "secret => ").unwrap();
        repo.set("app.hotkey", "Ctrl+Shift+K").unwrap();
        let running = settings_state_from(&load_settings(&repo));
        assert!(running.delete_after_paste.load(Ordering::Relaxed));

        repo.clear().unwrap();
        running.take_values_from(settings_state_from(&load_settings(&repo)));

        assert!(!running.delete_after_paste.load(Ordering::Relaxed));
        assert_eq!(*running.cleanup_rules.lock().unwrap(), *defaults.cleanup_rules.lock().unwrap());
        assert_eq!(*running.main_hotkey.lock().unwrap(), *defaults.main_hotkey.lock().unwrap());
    }
}
