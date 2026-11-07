//! 翻译: Chinese <-> English on device, with Apple's Translation framework
//! (macOS 26+). The framework is Swift only; swift/Translate.swift wraps it
//! and build.rs links it in. Language packs are the system's: when one is
//! missing, the system's own download prompt shows over the main window.

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::mpsc;

use tauri::{AppHandle, Manager};

type Callback = extern "C" fn(*mut c_void, i32, *const c_char);

extern "C" {
    fn cp_translation_supported() -> i32;
    fn cp_translate(
        text: *const c_char,
        source: *const c_char,
        target: *const c_char,
        window: *mut c_void,
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

const LANGUAGES: [&str; 2] = ["zh-Hans", "en"];

/// Code and text from Swift: 0 ok, 1 old system, 2 language not offered, 3 failed.
type Reply = (i32, String);

extern "C" fn on_reply(context: *mut c_void, code: i32, text: *const c_char) {
    // Swift calls this exactly once per request.
    let sender = unsafe { Box::from_raw(context as *mut mpsc::Sender<Reply>) };
    let text = if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned()
    };
    let _ = sender.send((code, text));
}

/// Runs a Swift call that answers through `on_reply`, without blocking the
/// async runtime while it waits.
async fn call_swift(start: impl FnOnce(*mut c_void) + Send + 'static) -> Result<Reply, String> {
    let (sender, receiver) = mpsc::channel::<Reply>();
    let context = Box::into_raw(Box::new(sender)) as usize;
    tauri::async_runtime::spawn_blocking(move || {
        start(context as *mut c_void);
        receiver.recv().map_err(|_| "no_reply".to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn check_pair(source: &str, target: &str) -> Result<(CString, CString), String> {
    if source == target || !LANGUAGES.contains(&source) || !LANGUAGES.contains(&target) {
        return Err("unsupported_pair".into());
    }
    Ok((CString::new(source).unwrap(), CString::new(target).unwrap()))
}

/// Whether to show the 翻译 button: macOS 26 and later.
#[tauri::command]
pub fn translation_supported() -> bool {
    unsafe { cp_translation_supported() == 1 }
}

/// "installed", "download" (the system will ask first), "unavailable".
#[tauri::command]
pub async fn translation_status(source: String, target: String) -> Result<String, String> {
    let (source, target) = check_pair(&source, &target)?;
    let (_, text) = call_swift(move |context| unsafe {
        cp_translation_status(source.as_ptr(), target.as_ptr(), context, on_reply)
    })
    .await?;
    Ok(match text.as_str() {
        "0" => "installed",
        "1" => "download",
        _ => "unavailable",
    }
    .into())
}

/// Translates `text` between "zh-Hans" and "en". Errors are short codes the
/// panel turns into messages: unsupported_os, unsupported_pair, cancelled.
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
    let (source, target) = check_pair(&source, &target)?;
    let text = CString::new(text.replace('\0', "")).map_err(|error| error.to_string())?;
    // The window outlives the app's run; Swift only reads it on the main thread.
    let window = app
        .get_webview_window("main")
        .and_then(|window| window.ns_window().ok())
        .map(|pointer| pointer as usize)
        .unwrap_or(0);

    let (code, reply) = call_swift(move |context| unsafe {
        cp_translate(
            text.as_ptr(),
            source.as_ptr(),
            target.as_ptr(),
            window as *mut c_void,
            context,
            on_reply,
        )
    })
    .await?;
    match code {
        0 => Ok(reply),
        _ => Err(reply),
    }
}
