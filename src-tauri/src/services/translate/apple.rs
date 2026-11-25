//! Swift bridge for Apple Translation and its system download prompt.

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::mpsc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

type Callback = extern "C" fn(*mut c_void, i32, *const c_char);

extern "C" {
    fn cp_translation_supported() -> i32;
    fn cp_translate(
        text: *const c_char,
        source: *const c_char,
        target: *const c_char,
        view: *mut c_void,
        context: *mut c_void,
        callback: Callback,
    );
    fn cp_translation_status(
        source: *const c_char,
        target: *const c_char,
        context: *mut c_void,
        callback: Callback,
    );
}

// Swift expires requests after 60 seconds and owns the context until it replies.
const SWIFT_REPLY_TIMEOUT: Duration = Duration::from_secs(65);
type Reply = (i32, String);

extern "C" fn on_reply(context: *mut c_void, code: i32, text: *const c_char) {
    let sender = unsafe { Box::from_raw(context as *mut mpsc::Sender<Reply>) };
    let text = if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    };
    let _ = sender.send((code, text));
}

fn wait_for_reply(receiver: mpsc::Receiver<Reply>, timeout: Duration) -> Result<Reply, String> {
    receiver.recv_timeout(timeout).map_err(|error| match error {
        mpsc::RecvTimeoutError::Timeout => "timeout".into(),
        mpsc::RecvTimeoutError::Disconnected => "no_reply".into(),
    })
}

async fn call_swift(
    start: impl FnOnce(mpsc::Sender<Reply>) -> Result<(), String>,
) -> Result<Reply, String> {
    let (sender, receiver) = mpsc::channel::<Reply>();
    start(sender)?;
    tauri::async_runtime::spawn_blocking(move || wait_for_reply(receiver, SWIFT_REPLY_TIMEOUT))
        .await
        .map_err(|_| "no_reply".to_string())?
}

pub fn supported() -> bool {
    unsafe { cp_translation_supported() == 1 }
}

pub async fn status(source: &str, target: &str) -> Result<String, String> {
    let source = CString::new(source).unwrap();
    let target = CString::new(target).unwrap();
    let (code, text) = call_swift(move |sender| {
        unsafe {
            cp_translation_status(
                source.as_ptr(),
                target.as_ptr(),
                Box::into_raw(Box::new(sender)).cast(),
                on_reply,
            );
        }
        Ok(())
    })
    .await?;
    if code != 0 {
        return Err(text);
    }
    Ok(match text.as_str() {
        "0" => "installed",
        "1" => "download",
        _ => "unavailable",
    }
    .into())
}

pub async fn translate(
    app: &AppHandle,
    text: &str,
    source: &str,
    target: &str,
) -> Result<String, String> {
    let text = CString::new(text.replace('\0', "")).unwrap();
    let source = CString::new(source).unwrap();
    let target = CString::new(target).unwrap();
    let window = app.get_webview_window("main").ok_or("no_window")?;
    // with_webview runs on the main thread. Swift resolves the attached window
    // before returning, while the platform webview handle is still valid.
    let (code, reply) = call_swift(move |sender| {
        window
            .with_webview(move |view| unsafe {
                cp_translate(
                    text.as_ptr(),
                    source.as_ptr(),
                    target.as_ptr(),
                    view.inner(),
                    Box::into_raw(Box::new(sender)).cast(),
                    on_reply,
                );
            })
            .map_err(|_| "no_window".into())
    })
    .await?;
    if code == 0 {
        Ok(reply)
    } else {
        Err(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swift_reply_times_out_when_callback_does_not_arrive() {
        let (sender, receiver) = mpsc::channel();
        let disconnect = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(sender);
        });
        let result = wait_for_reply(receiver, Duration::ZERO);
        disconnect.join().unwrap();
        assert_eq!(result, Err("timeout".into()));
    }

    #[test]
    fn swift_translate_reports_a_missing_window_without_waiting_for_language_packs() {
        if !supported() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let input = CString::new("Hello").unwrap();
        let source = CString::new("en").unwrap();
        let target = CString::new("zh-Hans").unwrap();
        unsafe {
            cp_translate(
                input.as_ptr(),
                source.as_ptr(),
                target.as_ptr(),
                std::ptr::null_mut(),
                Box::into_raw(Box::new(sender)).cast(),
                on_reply,
            );
        }
        assert_eq!(
            wait_for_reply(receiver, Duration::ZERO),
            Ok((3, "no_window".into()))
        );
    }
}
