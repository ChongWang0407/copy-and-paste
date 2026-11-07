#[cfg(target_os = "macos")]
use std::ffi::c_void;

#[cfg(target_os = "macos")]
type CFAllocatorRef = *const c_void;
#[cfg(target_os = "macos")]
type CFMutableDictionaryRef = *mut c_void;
#[cfg(target_os = "macos")]
type CFDictionaryRef = *const c_void;
#[cfg(target_os = "macos")]
type CFStringRef = *const c_void;
#[cfg(target_os = "macos")]
type CFTypeRef = *const c_void;
#[cfg(target_os = "macos")]
type Boolean = u8;

#[cfg(target_os = "macos")]
type CGEventSourceRef = *mut c_void;
#[cfg(target_os = "macos")]
type CGEventRef = *mut c_void;
#[cfg(target_os = "macos")]
type CGEventTapLocation = u32;
#[cfg(target_os = "macos")]
type CGEventFlags = u64;
#[cfg(target_os = "macos")]
type CGKeyCode = u16;
#[cfg(target_os = "macos")]
type CGEventSourceStateID = i32;

#[cfg(target_os = "macos")]
const K_CG_HID_EVENT_TAP: CGEventTapLocation = 0;
#[cfg(target_os = "macos")]
const K_CG_EVENT_FLAG_MASK_COMMAND: CGEventFlags = 1 << 20;
#[cfg(target_os = "macos")]
const K_CG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE: CGEventSourceStateID = 1;
#[cfg(target_os = "macos")]
pub(crate) const V_KEY_CODE: CGKeyCode = 9;

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> Boolean;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> Boolean;
    static kAXTrustedCheckOptionPrompt: CFStringRef;

    fn CGPreflightPostEventAccess() -> bool;
    fn CGEventSourceCreate(state_id: CGEventSourceStateID) -> CGEventSourceRef;
    fn CGEventCreateKeyboardEvent(
        source: CGEventSourceRef,
        virtual_key: CGKeyCode,
        key_down: Boolean,
    ) -> CGEventRef;
    fn CGEventSetFlags(event: CGEventRef, flags: CGEventFlags);
    fn CGEventPost(tap: CGEventTapLocation, event: CGEventRef);
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDictionaryCreateMutable(
        allocator: CFAllocatorRef,
        capacity: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFMutableDictionaryRef;
    fn CFDictionaryAddValue(
        the_dict: CFMutableDictionaryRef,
        key: *const c_void,
        value: *const c_void,
    );
    fn CFRelease(cf: CFTypeRef);
    static kCFBooleanTrue: CFTypeRef;
}

#[cfg(target_os = "macos")]
fn trusted_with_prompt(prompt: bool) -> bool {
    if !prompt {
        return unsafe { AXIsProcessTrusted() != 0 };
    }

    let dict = unsafe {
        CFDictionaryCreateMutable(std::ptr::null(), 1, std::ptr::null(), std::ptr::null())
    };
    if dict.is_null() {
        return unsafe { AXIsProcessTrusted() != 0 };
    }

    unsafe {
        CFDictionaryAddValue(
            dict,
            kAXTrustedCheckOptionPrompt as *const c_void,
            kCFBooleanTrue as *const c_void,
        );
    }

    let trusted = unsafe { AXIsProcessTrustedWithOptions(dict as CFDictionaryRef) != 0 };
    unsafe {
        CFRelease(dict as CFTypeRef);
    }
    trusted
}

/// Pasting only needs to post Cmd+V. System Settings shows one switch, but
/// macOS records it under two services, so either grant counts.
pub fn has_accessibility_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        return trusted_with_prompt(false) || unsafe { CGPreflightPostEventAccess() };
    }

}

/// What asking for the permission led to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionRequest {
    Granted,
    /// macOS shows its own dialog; its button opens System Settings.
    Prompted,
    /// macOS may stay silent: the caller should open System Settings itself.
    Failed,
}

/// Clear what macOS remembers about this app for `service`.
#[cfg(target_os = "macos")]
fn forget_permission_record(service: &str, bundle_id: &str) -> bool {
    match std::process::Command::new("/usr/bin/tccutil")
        .args(["reset", service, bundle_id])
        .output()
    {
        Ok(output) if output.status.success() => {
            crate::info!(">>> [PERMISSION] cleared the {service} record");
            true
        }
        Ok(output) => {
            crate::error!(
                ">>> [PERMISSION] tccutil {service}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            false
        }
        Err(error) => {
            crate::error!(">>> [PERMISSION] tccutil {service}: {error}");
            false
        }
    }
}

/// Put this app in the Accessibility list of System Settings. It lands there
/// switched off: only the user can switch it on.
///
/// macOS asks once per app. A record for an older build (its signature no
/// longer matches) or a past "Don't Allow" makes the request do nothing, and
/// the app stays out of the list. Without the permission there is nothing to
/// lose by clearing that record first. Exactly one request: each one is a
/// system dialog of its own.
pub fn request_accessibility_permission(bundle_id: &str) -> PermissionRequest {
    #[cfg(target_os = "macos")]
    {
        if has_accessibility_permission() {
            return PermissionRequest::Granted;
        }
        let accessibility_cleared = forget_permission_record("Accessibility", bundle_id);
        let post_event_cleared = forget_permission_record("PostEvent", bundle_id);
        if trusted_with_prompt(true) {
            return PermissionRequest::Granted;
        }
        let outcome = if accessibility_cleared && post_event_cleared {
            PermissionRequest::Prompted
        } else {
            PermissionRequest::Failed
        };
        crate::info!(">>> [PERMISSION] requested: {outcome:?}");
        return outcome;
    }

}

pub fn send_command_v() -> bool {
    #[cfg(target_os = "macos")]
    {
        let source = unsafe { CGEventSourceCreate(K_CG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE) };
        if source.is_null() {
            return false;
        }

        let key_down = unsafe { CGEventCreateKeyboardEvent(source, V_KEY_CODE, 1) };
        let key_up = unsafe { CGEventCreateKeyboardEvent(source, V_KEY_CODE, 0) };
        if key_down.is_null() || key_up.is_null() {
            if !key_down.is_null() {
                unsafe { CFRelease(key_down as CFTypeRef) };
            }
            if !key_up.is_null() {
                unsafe { CFRelease(key_up as CFTypeRef) };
            }
            unsafe { CFRelease(source as CFTypeRef) };
            return false;
        }

        unsafe {
            CGEventSetFlags(key_down, K_CG_EVENT_FLAG_MASK_COMMAND);
            CGEventSetFlags(key_up, K_CG_EVENT_FLAG_MASK_COMMAND);
            CGEventPost(K_CG_HID_EVENT_TAP, key_down);
            CGEventPost(K_CG_HID_EVENT_TAP, key_up);
            CFRelease(key_down as CFTypeRef);
            CFRelease(key_up as CFTypeRef);
            CFRelease(source as CFTypeRef);
        }
        true
    }

}
