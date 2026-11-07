//! Launch at login through SMAppService (macOS 13+).
//!
//! The old way, a plist written to ~/Library/LaunchAgents, is attributed to
//! the signing certificate: macOS's "Background Items Added" notice and Login
//! Items settings showed the developer's personal name instead of the app. An
//! agent shipped inside the app bundle (Contents/Library/LaunchAgents, see
//! tauri.macos.conf.json) and registered here is shown with the app's name and
//! icon, and still passes --minimized so the app starts hidden.
//!
//! None of this exists outside a bundled .app (dev runs), and SMAppService can
//! refuse an ad-hoc signed build. `set_launch_at_login` falls back to a plain
//! agent in ~/Library/LaunchAgents then. It is written here, not by the
//! autostart plugin, which doesn't escape XML: the `&` in "Copy & Paste.app"
//! made its plist invalid.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::msg_send;
use objc2_foundation::{NSError, NSString};
use std::path::PathBuf;

/// Must match the file name under Contents/Library/LaunchAgents.
const AGENT_PLIST: &str = "com.wangchong.copypaste.autostart.plist";

// SMAppServiceStatus
const STATUS_ENABLED: isize = 1;
const STATUS_REQUIRES_APPROVAL: isize = 2;

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

fn agent_service() -> Option<Retained<AnyObject>> {
    // SMAppService is only in macOS 13+; the framework itself is older.
    let class = AnyClass::get(c"SMAppService")?;
    let name = NSString::from_str(AGENT_PLIST);
    unsafe { msg_send![class, agentServiceWithPlistName: &*name] }
}

fn status(service: &AnyObject) -> isize {
    unsafe { msg_send![service, status] }
}

/// Whether this system and this build can use SMAppService. False before
/// macOS 13 and when the agent plist isn't in the bundle (dev builds).
pub fn available() -> bool {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent()?.parent().map(|contents| contents.join("Library/LaunchAgents").join(AGENT_PLIST)))
        .is_some_and(|plist| plist.exists());
    bundled && agent_service().is_some()
}

/// Registered and allowed to run. "Requires approval" means the user turned
/// it off in System Settings, so it counts as off.
pub fn is_enabled() -> bool {
    agent_service().is_some_and(|service| status(&service) == STATUS_ENABLED)
}

/// Registered, but switched off in System Settings › Login Items. Only the
/// user can switch it back on, over there; registering again changes nothing.
pub fn requires_approval() -> bool {
    agent_service().is_some_and(|service| status(&service) == STATUS_REQUIRES_APPROVAL)
}

/// Opens System Settings at Login Items.
pub fn open_settings() {
    if let Some(class) = AnyClass::get(c"SMAppService") {
        let _: () = unsafe { msg_send![class, openSystemSettingsLoginItems] };
    }
}

/// Turning it on fails while the user has it switched off in System Settings
/// (see `requires_approval`): it is registered but won't launch.
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let service = agent_service().ok_or("SMAppService unavailable")?;
    let current = status(&service);
    let registered = current == STATUS_ENABLED || current == STATUS_REQUIRES_APPROVAL;
    if enabled != registered {
        let result: Result<(), Retained<NSError>> = unsafe {
            if enabled {
                msg_send![&service, registerAndReturnError: _]
            } else {
                msg_send![&service, unregisterAndReturnError: _]
            }
        };
        result.map_err(|error| error.localizedDescription().to_string())?;
    }
    if enabled && status(&service) == STATUS_REQUIRES_APPROVAL {
        return Err("Launch at login is turned off in System Settings".to_string());
    }
    Ok(())
}

/// A dev run must not replace the installed app's agent.
fn plain_agent_path() -> Option<PathBuf> {
    let name = if cfg!(debug_assertions) { "com.wangchong.copypaste.dev.autostart.plist" } else { AGENT_PLIST };
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/LaunchAgents").join(name))
}

fn xml_escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn plain_agent_plist(label: &str, exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{}</string>
		<string>--minimized</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
	<key>ProcessType</key>
	<string>Interactive</string>
</dict>
</plist>
"#,
        xml_escape(label),
        xml_escape(exe)
    )
}

fn write_plain_agent() -> Result<(), String> {
    let path = plain_agent_path().ok_or("no home folder")?;
    let label = path.file_stem().and_then(|stem| stem.to_str()).ok_or("bad agent name")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, plain_agent_plist(label, &exe.to_string_lossy())).map_err(|e| e.to_string())
}

fn remove_plain_agent() -> Result<(), String> {
    match plain_agent_path().map(std::fs::remove_file) {
        Some(Err(error)) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
        _ => Ok(()),
    }
}

/// Starts at login, whichever way it was registered.
pub fn launch_at_login() -> bool {
    (available() && is_enabled()) || plain_agent_path().is_some_and(|path| path.exists())
}

pub fn set_launch_at_login(enabled: bool) -> Result<(), String> {
    if !enabled {
        remove_plain_agent()?;
        return if available() { set_enabled(false) } else { Ok(()) };
    }
    if available() {
        match set_enabled(true) {
            Ok(()) => return remove_plain_agent(),
            Err(error) if requires_approval() => return Err(error),
            Err(error) => crate::error!(">>> [AUTOSTART] {error}; using a launch agent instead"),
        }
    }
    write_plain_agent()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ampersand_in_the_app_path_stays_valid_xml() {
        let plist = plain_agent_plist("label", "/Applications/Copy & Paste.app/Contents/MacOS/copy-paste");
        assert!(plist.contains("<string>/Applications/Copy &amp; Paste.app/Contents/MacOS/copy-paste</string>"));
        assert!(!plist.contains("Copy & Paste"));
    }
}
