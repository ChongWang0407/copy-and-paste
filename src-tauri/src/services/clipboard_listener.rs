#[cfg(target_os = "macos")]
fn has_pending_change(last: isize, current: isize, paused: bool) -> bool {
    !paused && current != last
}

#[cfg(target_os = "macos")]
pub fn listen_clipboard<T: Send + 'static>(
    mut snapshot: impl FnMut(isize) -> Result<Option<T>, ()> + Send + 'static,
    mut capture: impl FnMut(T) + Send + 'static,
) {
    use crate::global_state::is_clipboard_monitor_paused;
    use crate::infrastructure::macos_api::clipboard::change_count;
    use std::sync::mpsc;
    use std::time::Duration;

    let (sender, receiver) = mpsc::channel();
    let mut last = change_count();
    std::thread::spawn(move || {
        for payload in receiver {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| capture(payload))).is_err()
            {
                eprintln!(">>> [CLIPBOARD] capture panicked; continuing");
            }
        }
    });

    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(80));
        loop {
            let current = change_count();
            // Leave the latest change pending until every pause owner has resumed.
            if !has_pending_change(last, current, is_clipboard_monitor_paused()) {
                break;
            }
            // Some sources add formats immediately after declaring the new value.
            std::thread::sleep(Duration::from_millis(8));
            let payload = snapshot(current);
            if is_clipboard_monitor_paused() {
                break;
            }
            match payload {
                Ok(payload) => {
                    last = current;
                    if let Some(payload) = payload {
                        if sender.send(payload).is_err() {
                            return;
                        }
                    }
                    break;
                }
                // A newer change replaced the data before it could be queued.
                Err(()) => continue,
            }
        }
    });
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::has_pending_change;

    #[test]
    fn clipboard_change_remains_pending_until_monitor_resumes() {
        let mut last = 10;
        for current in [11, 12] {
            if has_pending_change(last, current, true) {
                last = current;
            }
        }
        assert!(has_pending_change(last, 12, false));
        last = 12;
        assert!(!has_pending_change(last, 12, false));
    }
}

