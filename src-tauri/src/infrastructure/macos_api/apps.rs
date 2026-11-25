use base64::Engine;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[cfg(target_os = "macos")]
use objc2::rc::autoreleasepool;
#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSImage, NSWorkspace,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSDictionary, NSString};

static EXECUTABLE_ICON_CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();
static FILE_ICON_CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();

/// Icons kept per cache; file icons are keyed by path, so without a limit the
/// cache grew with every file ever copied.
const ICON_CACHE_LIMIT: usize = 256;
/// Largest icon side sent to the page: cards show icons at 16-30 pt (2x).
const ICON_PIXELS: u32 = 64;

/// Icons by key, the oldest dropped first past the limit.
#[derive(Default)]
struct IconCache {
    icons: HashMap<String, Option<String>>,
    order: std::collections::VecDeque<String>,
}

impl IconCache {
    fn get(&self, key: &str) -> Option<Option<String>> {
        self.icons.get(key).cloned()
    }

    fn insert(&mut self, key: String, icon: Option<String>) {
        if self.icons.insert(key.clone(), icon).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > ICON_CACHE_LIMIT {
            if let Some(oldest) = self.order.pop_front() {
                self.icons.remove(&oldest);
            }
        }
    }
}

/// The icon at display size. NSWorkspace icons carry up to 1024 px
/// representations, which made each one tens to hundreds of KB of base64.
fn shrink_icon_png(png: Vec<u8>) -> Vec<u8> {
    let Ok(image) = image::load_from_memory_with_format(&png, image::ImageFormat::Png) else {
        return png;
    };
    if image.width() <= ICON_PIXELS && image.height() <= ICON_PIXELS {
        return png;
    }
    let small = image.resize(ICON_PIXELS, ICON_PIXELS, image::imageops::FilterType::Lanczos3);
    let mut out = Vec::new();
    match small.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png) {
        Ok(()) => out,
        Err(_) => png,
    }
}

pub async fn launch_uwp_with_file(
    _package: &str,
    _file: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // macOS equivalent: open -a or similar
    Ok(())
}

pub fn get_system_default_app(ext: &str) -> String {
    // Lightweight fallback mapping on macOS.
    // A proper LaunchServices lookup can replace this later.
    let normalized = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    match normalized.as_str() {
        "txt" | "text" | "md" | "code" | "json" | "xml" | "csv" => "TextEdit".to_string(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "heic" | "tiff" => "Preview".to_string(),
        "mp4" | "mov" | "mkv" | "avi" | "wmv" | "webm" | "m4v" => "QuickTime Player".to_string(),
        "html" | "htm" | "url" => "Safari".to_string(),
        _ => String::new(),
    }
}

fn collect_app_bundles(root: &Path, seen: &mut HashSet<String>, output: &mut Vec<Value>) {
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let is_app = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("app"))
            .unwrap_or(false);
        if !is_app {
            continue;
        }

        let path_str = path.to_string_lossy().to_string();
        if !seen.insert(path_str.clone()) {
            continue;
        }

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string();

        output.push(json!({
            "name": name,
            "path": path_str
        }));
    }
}

pub fn scan_installed_apps() -> Vec<Value> {
    let mut roots: Vec<PathBuf> = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];

    if let Ok(home) = std::env::var("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }

    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for root in roots {
        collect_app_bundles(&root, &mut seen, &mut apps);
    }

    apps.sort_by(|a, b| {
        let an = a
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let bn = b
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        an.cmp(&bn)
    });
    apps
}

pub fn get_associated_apps(_ext: &str) -> Vec<Value> {
    // Fallback on macOS: expose installed apps as candidates.
    // Proper LaunchServices association lookup can be added later.
    scan_installed_apps()
}

fn bundle_path_from_target(path: &Path) -> Option<PathBuf> {
    let path_str = path.to_string_lossy();
    if let Some(idx) = path_str.find(".app/") {
        return Some(PathBuf::from(&path_str[..idx + 4]));
    }

    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("app"))
        .unwrap_or(false)
    {
        return Some(path.to_path_buf());
    }

    None
}

fn normalize_icon_cache_key(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let candidate = Path::new(trimmed);
    if let Ok(resolved) = std::fs::canonicalize(candidate) {
        if let Some(bundle) = bundle_path_from_target(&resolved) {
            return bundle.to_string_lossy().to_string();
        }
        return resolved.to_string_lossy().to_string();
    }

    if let Some(bundle) = bundle_path_from_target(candidate) {
        return bundle.to_string_lossy().to_string();
    }

    trimmed.to_string()
}

fn normalize_file_icon_cache_key(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let candidate = Path::new(trimmed);
    if candidate.exists() {
        return normalize_icon_cache_key(trimmed);
    }

    candidate
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(|value| format!(".{}", value.to_ascii_lowercase()))
        .unwrap_or_else(|| trimmed.to_string())
}

#[cfg(target_os = "macos")]
fn image_to_data_url(image: &NSImage) -> Result<Option<String>, String> {
    autoreleasepool(|_| {
        let Some(tiff_data) = image.TIFFRepresentation() else {
            return Ok(None);
        };
        let Some(bitmap) = NSBitmapImageRep::imageRepWithData(&tiff_data) else {
            return Ok(None);
        };

        let properties = NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::from_slices(
            &[] as &[&NSBitmapImageRepPropertyKey],
            &[] as &[&AnyObject],
        );

        let png_data = unsafe {
            bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
        };
        let Some(png_data) = png_data else {
            return Ok(None);
        };

        Ok(Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(shrink_icon_png(png_data.to_vec()))
        )))
    })
}

#[cfg(target_os = "macos")]
fn get_workspace_icon_for_file(path: &str) -> Result<Option<String>, String> {
    let normalized = normalize_icon_cache_key(path);
    if normalized.is_empty() {
        return Ok(None);
    }

    let candidate = Path::new(&normalized);
    if !candidate.exists() {
        return Ok(None);
    }

    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let ns_path = NSString::from_str(&normalized);
        let image = workspace.iconForFile(&ns_path);
        image_to_data_url(&image)
    })
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn get_workspace_icon_for_file_type(path_or_ext: &str) -> Result<Option<String>, String> {
    let key = normalize_file_icon_cache_key(path_or_ext);
    if key.is_empty() {
        return Ok(None);
    }

    let file_type = key.trim_start_matches('.').trim();
    if file_type.is_empty() {
        return Ok(None);
    }

    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let ns_type = NSString::from_str(file_type);
        let image = workspace.iconForFileType(&ns_type);
        image_to_data_url(&image)
    })
}

pub fn get_executable_icon(executable_path: String) -> Result<Option<String>, String> {
    let cache_key = normalize_icon_cache_key(&executable_path);
    if cache_key.is_empty() {
        return Ok(None);
    }

    let cache = EXECUTABLE_ICON_CACHE.get_or_init(|| Mutex::new(IconCache::default()));
    if let Some(cached) = cache.lock().map_err(|e| e.to_string())?.get(&cache_key) {
        return Ok(cached);
    }

    let icon = get_workspace_icon_for_file(&executable_path)?;
    cache
        .lock()
        .map_err(|e| e.to_string())?
        .insert(cache_key, icon.clone());
    Ok(icon)
}

pub fn get_file_icon(file_path: String) -> Result<Option<String>, String> {
    let cache_key = normalize_file_icon_cache_key(&file_path);
    if cache_key.is_empty() {
        return Ok(None);
    }

    let cache = FILE_ICON_CACHE.get_or_init(|| Mutex::new(IconCache::default()));
    if let Some(cached) = cache.lock().map_err(|e| e.to_string())?.get(&cache_key) {
        return Ok(cached);
    }

    let icon = if Path::new(file_path.trim()).exists() {
        get_workspace_icon_for_file(&file_path)?
    } else {
        get_workspace_icon_for_file_type(&file_path)?
    };

    cache
        .lock()
        .map_err(|e| e.to_string())?
        .insert(cache_key, icon.clone());
    Ok(icon)
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub fn activate_app_by_pid(pid: i32) -> bool {
    use objc2_app_kit::NSApplicationActivationOptions;
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let apps = workspace.runningApplications();
        for i in 0..apps.count() {
            let app = apps.objectAtIndex(i);
            if app.processIdentifier() == pid {
                return app.activateWithOptions(
                    NSApplicationActivationOptions::ActivateIgnoringOtherApps,
                );
            }
        }
        false
    })
}

/// Soft activation for overlay show — restores keyboard without yanking windows.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub fn activate_app_by_pid_soft(pid: i32) -> bool {
    use objc2_app_kit::NSApplicationActivationOptions;
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let apps = workspace.runningApplications();
        for i in 0..apps.count() {
            let app = apps.objectAtIndex(i);
            if app.processIdentifier() == pid {
                // Empty options: prefer keyboard restore without forcing our
                // panel under other apps or stealing the next click.
                return app.activateWithOptions(NSApplicationActivationOptions::empty());
            }
        }
        false
    })
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub fn activate_app_by_name(name: &str) -> bool {
    use objc2_app_kit::NSApplicationActivationOptions;
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let apps = workspace.runningApplications();
        let target_name = name.to_lowercase();
        for i in 0..apps.count() {
            let app = apps.objectAtIndex(i);
            if let Some(bundle_name) = app.localizedName().map(|s| s.to_string()) {
                if bundle_name.to_lowercase() == target_name {
                    return app.activateWithOptions(
                        NSApplicationActivationOptions::ActivateIgnoringOtherApps,
                    );
                }
            }
        }
        false
    })
}

#[cfg(test)]
mod icon_tests {
    use super::{shrink_icon_png, IconCache, ICON_CACHE_LIMIT, ICON_PIXELS};

    fn png(side: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::new_rgba8(side, side)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn icons_are_sent_at_display_size() {
        let small = image::load_from_memory(&shrink_icon_png(png(512))).unwrap();
        assert_eq!((small.width(), small.height()), (ICON_PIXELS, ICON_PIXELS));
        let tiny = png(32);
        assert_eq!(shrink_icon_png(tiny.clone()), tiny);
        assert_eq!(shrink_icon_png(b"not a png".to_vec()), b"not a png".to_vec());
    }

    #[test]
    fn the_icon_cache_drops_the_oldest_past_its_limit() {
        let mut cache = IconCache::default();
        for index in 0..ICON_CACHE_LIMIT + 10 {
            cache.insert(format!("/file/{index}"), None);
        }
        assert_eq!(cache.icons.len(), ICON_CACHE_LIMIT);
        assert!(cache.get("/file/0").is_none());
        assert!(cache.get(&format!("/file/{}", ICON_CACHE_LIMIT + 9)).is_some());
        // Replacing a key does not count twice.
        cache.insert(format!("/file/{}", ICON_CACHE_LIMIT + 9), Some("icon".into()));
        assert_eq!(cache.order.len(), ICON_CACHE_LIMIT);
    }
}
