use std::sync::Arc;

/// Run captures on a worker thread. The returned closure marks the clipboard
/// as changed: the worker captures as soon as it is free, and changes seen
/// while it is busy collapse into one capture of the latest value.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn spawn_capture_worker(
    callback: Arc<dyn Fn() + Send + Sync + 'static>,
) -> Arc<dyn Fn() + Send + Sync + 'static> {
    use std::sync::{Condvar, Mutex};
    let pending = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_pending = pending.clone();
    std::thread::spawn(move || loop {
        {
            let (lock, ready) = &*worker_pending;
            let mut changed = lock.lock().unwrap_or_else(|p| p.into_inner());
            while !*changed {
                changed = ready.wait(changed).unwrap_or_else(|p| p.into_inner());
            }
            *changed = false;
        }
        // A panic in one capture must not end clipboard recording.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback())).is_err() {
            eprintln!(">>> [CLIPBOARD] capture panicked; continuing");
        }
    });
    Arc::new(move || {
        let (lock, ready) = &*pending;
        *lock.lock().unwrap_or_else(|p| p.into_inner()) = true;
        ready.notify_one();
    })
}

pub fn listen_clipboard(callback: Arc<dyn Fn() + Send + Sync + 'static>) {
    // macOS has no clipboard notification, so changeCount is polled. The
    // clipboard-rs watcher polled every 500 ms and ran the capture on its own
    // polling thread: several quick copies, or any copy made while a capture
    // was busy (e.g. fetching a web page's images), ended up as one entry.
    // Now the poll is fast and captures run on a worker; a change seen while
    // the worker is busy is captured as soon as it is free. Not debounced:
    // a later clipboard value can't be rebuilt from a dropped event, and the
    // monitor drops genuine duplicate events by payload hash.
    #[cfg(target_os = "macos")]
    {
        let notify = spawn_capture_worker(callback);
        std::thread::spawn(move || {
            let mut last = crate::infrastructure::macos_api::clipboard::change_count();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(80));
                let current = crate::infrastructure::macos_api::clipboard::change_count();
                if current != last {
                    last = current;
                    notify();
                }
            }
        });
        return;
    }

    // Windows: WM_CLIPBOARDUPDATE only signals the worker. The capture (with
    // its settle sleeps, up to ~1.4 s for some sources) used to run inside the
    // window procedure and held up the listener's message loop.


    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = callback;
        eprintln!(">>> [CLIPBOARD] listener unsupported on this platform");
    }
}

