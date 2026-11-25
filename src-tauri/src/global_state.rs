// Global state module
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

pub static GLOBAL_APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
pub static HOTKEY_STRING: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub static IS_RECORDING: AtomicBool = AtomicBool::new(false);
/// Legacy one-shot blur suppression. New code should use owner-based guards below.
pub static IGNORE_BLUR: AtomicBool = AtomicBool::new(false);
pub static WINDOW_PINNED: AtomicBool = AtomicBool::new(false);

/// Who has the clipboard monitor paused (hotkey recording, a batch paste).
/// It stays paused until every owner has let go: one finishing first used to
/// resume it under the other.
static MONITOR_PAUSE_OWNERS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

pub const PAUSE_HOTKEY_RECORDING: &str = "hotkey_recording";
pub const PAUSE_BATCH_PASTE: &str = "batch_paste";

pub fn pause_clipboard_monitor(owner: &'static str) {
    let mut owners = MONITOR_PAUSE_OWNERS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !owners.contains(&owner) {
        owners.push(owner);
    }
}

pub fn resume_clipboard_monitor(owner: &'static str) {
    MONITOR_PAUSE_OWNERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .retain(|held| *held != owner);
}

pub fn is_clipboard_monitor_paused() -> bool {
    !MONITOR_PAUSE_OWNERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_empty()
}

static BLUR_GUARD_OWNERS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn blur_guard_owners() -> &'static Mutex<HashSet<String>> {
    BLUR_GUARD_OWNERS.get_or_init(|| Mutex::new(HashSet::new()))
}

pub fn acquire_blur_guard(owner: &str) -> usize {
    let mut owners = blur_guard_owners()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    owners.insert(owner.to_string());
    owners.len()
}

pub fn release_blur_guard(owner: &str) -> usize {
    let mut owners = blur_guard_owners()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    owners.remove(owner);
    owners.len()
}

pub fn is_blur_ignored() -> bool {
    if IGNORE_BLUR.load(Ordering::Relaxed) {
        return true;
    }
    !blur_guard_owners()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_empty()
}

#[cfg(test)]
mod blur_guard_tests {
    use super::*;

    #[test]
    fn monitor_stays_paused_until_every_owner_resumes() {
        resume_clipboard_monitor(PAUSE_HOTKEY_RECORDING);
        resume_clipboard_monitor(PAUSE_BATCH_PASTE);
        assert!(!is_clipboard_monitor_paused());

        pause_clipboard_monitor(PAUSE_BATCH_PASTE);
        pause_clipboard_monitor(PAUSE_HOTKEY_RECORDING);
        // Recording ends first: the batch paste still holds the pause.
        resume_clipboard_monitor(PAUSE_HOTKEY_RECORDING);
        assert!(is_clipboard_monitor_paused());
        // A repeated start is the same owner, not a second hold.
        pause_clipboard_monitor(PAUSE_HOTKEY_RECORDING);
        pause_clipboard_monitor(PAUSE_HOTKEY_RECORDING);
        resume_clipboard_monitor(PAUSE_HOTKEY_RECORDING);
        assert!(is_clipboard_monitor_paused());
        resume_clipboard_monitor(PAUSE_BATCH_PASTE);
        assert!(!is_clipboard_monitor_paused());
    }

    #[test]
    fn owner_guards_are_nested_and_idempotent() {
        const FIRST: &str = "test:blur-guard:first";
        const SECOND: &str = "test:blur-guard:second";

        IGNORE_BLUR.store(false, Ordering::Relaxed);
        release_blur_guard(FIRST);
        release_blur_guard(SECOND);

        assert_eq!(acquire_blur_guard(FIRST), 1);
        assert_eq!(acquire_blur_guard(FIRST), 1);
        assert_eq!(acquire_blur_guard(SECOND), 2);
        assert!(is_blur_ignored());

        assert_eq!(release_blur_guard(FIRST), 1);
        assert!(is_blur_ignored());
        assert_eq!(release_blur_guard(SECOND), 0);
        assert!(!is_blur_ignored());
    }
}

// Store the frontmost app before we show our window,
// so we can re-activate it before pasting.
pub static LAST_ACTIVE_APP_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static LAST_ACTIVE_APP_NAME: std::sync::OnceLock<std::sync::Mutex<String>> =
    std::sync::OnceLock::new();

pub fn get_last_active_app_name() -> String {
    LAST_ACTIVE_APP_NAME
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clone()
}

pub fn set_last_active_app_name(name: String) {
    let cell = LAST_ACTIVE_APP_NAME.get_or_init(|| std::sync::Mutex::new(String::new()));
    *cell.lock().unwrap() = name;
}

pub static LAST_TOGGLE_TIMESTAMP: AtomicU64 = AtomicU64::new(0);
pub static LAST_SHOW_TIMESTAMP: AtomicU64 = AtomicU64::new(0);

pub static NAVIGATION_ENABLED: AtomicBool = AtomicBool::new(false);
pub static NAVIGATION_MODE_ACTIVE: AtomicBool = AtomicBool::new(false);
