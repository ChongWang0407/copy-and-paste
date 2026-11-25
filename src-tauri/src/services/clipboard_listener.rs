#[cfg(target_os = "macos")]
pub fn listen_clipboard<T: Send + 'static>(
    mut snapshot: impl FnMut(isize) -> Result<Option<T>, ()> + Send + 'static,
    mut capture: impl FnMut(T) + Send + 'static,
) {
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
            if current == last {
                break;
            }
            // Some sources add formats immediately after declaring the new value.
            std::thread::sleep(Duration::from_millis(8));
            match snapshot(current) {
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

