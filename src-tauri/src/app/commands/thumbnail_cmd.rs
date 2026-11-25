use crate::error::AppError;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::UNIX_EPOCH;
use tauri::{AppHandle, Manager};

/// Longest side (px) of a generated thumbnail. A card shows at most ~320 x 280
/// points, 640 x 560 pixels on a Retina screen. At 320 here the text of a
/// screenshot went blurry; at 1280 every card decoded far more than it shows,
/// and scrolling stuttered.
pub const THUMBNAIL_MAX_SIDE: u32 = 800;
/// A video card shows only a small cover.
const VIDEO_THUMBNAIL_MAX_SIDE: u32 = 320;
const THUMBNAIL_JPEG_QUALITY: u8 = 90;
/// Bump to invalidate every cached thumbnail when the output format changes.
const THUMBNAIL_CACHE_VERSION: &str = "v1";
/// Thumbnails not refreshed for this long are dropped at startup…
const THUMBNAIL_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 60 * 60);
/// …and the folder is trimmed, oldest first, to this size.
const THUMBNAIL_CACHE_MAX_BYTES: u64 = 300 * 1024 * 1024;

static THUMBNAIL_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub(crate) fn thumbnail_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    if let Some(dir) = THUMBNAIL_DIR.get() {
        return Ok(dir.clone());
    }
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| AppError::IO(format!("cache dir unavailable: {}", e)))?
        .join("thumbnails");
    Ok(THUMBNAIL_DIR.get_or_init(|| dir).clone())
}

fn session_thumbnail_dir(data_dir: &Path, path: &Path) -> Option<PathBuf> {
    let session = data_dir.join("session");
    let attachments = session.join("attachments");
    let in_session = path.starts_with(&attachments)
        || fs::canonicalize(path)
            .ok()
            .zip(fs::canonicalize(&attachments).ok())
            .is_some_and(|(path, root)| path.starts_with(root));
    in_session.then(|| session.join("thumbnails"))
}

pub(crate) fn thumbnail_dir_for_path(app: &AppHandle, path: &str) -> Result<PathBuf, AppError> {
    if let Some(dir) = crate::services::content_handler::app_data_dir(app)
        .and_then(|dir| session_thumbnail_dir(&dir, Path::new(path)))
    {
        return Ok(dir);
    }
    thumbnail_dir(app)
}

/// At startup: remember where thumbnails live (so deletes can remove them) and
/// prune the cache in the background. Edited images get a new key and deleted
/// ones used to leave their thumbnails behind forever.
pub fn init_thumbnail_cache(app: &AppHandle) {
    if let Ok(dir) = thumbnail_dir(app) {
        std::thread::spawn(move || prune_thumbnail_cache(&dir));
    }
}

fn prune_thumbnail_cache(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(PathBuf, std::time::SystemTime, u64)> = entries
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            meta.is_file().then(|| (entry.path(), meta.modified().unwrap_or(UNIX_EPOCH), meta.len()))
        })
        .collect();
    let now = std::time::SystemTime::now();
    files.retain(|(path, modified, _)| {
        let stale = now.duration_since(*modified).map_or(false, |age| age > THUMBNAIL_MAX_AGE);
        if stale {
            let _ = fs::remove_file(path);
        }
        !stale
    });
    let mut total: u64 = files.iter().map(|(_, _, size)| size).sum();
    files.sort_by_key(|(_, modified, _)| *modified);
    for (path, _, size) in files {
        if total <= THUMBNAIL_CACHE_MAX_BYTES {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

/// Remove the cached thumbnails of an image that is about to be deleted, so a
/// deleted (possibly sensitive) screenshot doesn't live on as a thumbnail.
pub fn remove_cached_thumbnails(path: &Path) {
    let Ok(canonical) = fs::canonicalize(path) else {
        return;
    };
    let Ok(meta) = fs::metadata(&canonical) else {
        return;
    };
    let mtime_nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let key = thumbnail_cache_key(&canonical, meta.len(), mtime_nanos);
    let session_cache = path
        .parent()
        .and_then(Path::parent)
        .filter(|dir| dir.file_name().is_some_and(|name| name == "session"))
        .map(|dir| dir.join("thumbnails"));
    for dir in THUMBNAIL_DIR
        .get()
        .into_iter()
        .chain(session_cache.as_ref())
    {
        for ext in ["jpg", "png"] {
            let _ = fs::remove_file(dir.join(format!("{key}.{ext}")));
        }
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn thumbnail_generation(cache_dir: &Path, key: &str) -> Arc<Mutex<()>> {
    static GENERATIONS: LazyLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let mut generations = GENERATIONS.lock().unwrap();
    let key = cache_dir.join(key);
    if let Some(lock) = generations.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    generations.retain(|_, lock| lock.strong_count() != 0);
    let lock = Arc::new(Mutex::new(()));
    generations.insert(key, Arc::downgrade(&lock));
    lock
}

/// Returns an absolute path to a thumbnail (longest side <= 800 px) for `path`.
/// Images that already fit return `path` unchanged; larger animated ones get a
/// still thumbnail of their first frame.
#[tauri::command]
pub async fn get_image_thumbnail(app: AppHandle, path: String) -> Result<String, AppError> {
    let cache_dir = thumbnail_dir_for_path(&app, &path)?;
    tauri::async_runtime::spawn_blocking(move || build_thumbnail(&path, &cache_dir))
        .await
        .map_err(|e| AppError::Internal(format!("thumbnail task failed: {}", e)))?
}

/// Returns an absolute path to a still of a video (longest side <= 320 px),
/// made by the system's own thumbnailer: QuickLook on macOS, the shell's
/// thumbnail providers on Windows. Those cover what Finder and Explorer can
/// preview; the WebView decodes only a few codecs (no MKV, AVI, often HEVC).
#[tauri::command]
pub async fn get_video_thumbnail(app: AppHandle, path: String) -> Result<String, AppError> {
    let cache_dir = thumbnail_dir_for_path(&app, &path)?;
    tauri::async_runtime::spawn_blocking(move || build_video_thumbnail(&path, &cache_dir))
        .await
        .map_err(|e| AppError::Internal(format!("video thumbnail task failed: {}", e)))?
}

fn build_video_thumbnail(path: &str, cache_dir: &Path) -> Result<String, AppError> {
    let canonical =
        fs::canonicalize(path).map_err(|e| AppError::IO(format!("video not accessible: {}", e)))?;
    let meta = fs::metadata(&canonical)
        .map_err(|e| AppError::IO(format!("video not accessible: {}", e)))?;
    if !meta.is_file() {
        return Err(AppError::IO("video path is not a file".to_string()));
    }
    let mtime_nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let key = format!("video-{}", thumbnail_cache_key(&canonical, meta.len(), mtime_nanos));
    let generation = thumbnail_generation(cache_dir, &key);
    let _generation = generation.lock().unwrap();
    let final_path = cache_dir.join(format!("{}.jpg", key));
    if final_path.is_file() {
        return Ok(final_path.to_string_lossy().into_owned());
    }
    // A video the system couldn't preview (QuickLook stalls on MKV until the
    // timeout) isn't retried on every mount or launch.
    let failed_marker = cache_dir.join(format!("{}.none", key));
    if failed_marker.is_file() {
        return Err(AppError::Internal("system has no thumbnail for this video".to_string()));
    }

    fs::create_dir_all(cache_dir)?;
    let frame = match system_video_frame(&canonical, cache_dir, &key) {
        Ok(frame) => frame,
        Err(err) => {
            let _ = fs::write(&failed_marker, b"");
            return Err(err);
        }
    };
    let frame = match fit_within(frame.width(), frame.height(), VIDEO_THUMBNAIL_MAX_SIDE) {
        Some((w, h)) => frame.thumbnail_exact(w, h),
        None => frame,
    };
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, THUMBNAIL_JPEG_QUALITY)
        .encode_image(&frame.to_rgb8())
        .map_err(|e| AppError::Internal(format!("video thumbnail encode failed: {}", e)))?;
    write_atomically(&final_path, &bytes)?;
    Ok(final_path.to_string_lossy().into_owned())
}

/// QuickLook via `qlmanage`, which ships with every macOS. It writes
/// `<file name>.png` into the output folder.
#[cfg(target_os = "macos")]
fn system_video_frame(path: &Path, cache_dir: &Path, key: &str) -> Result<DynamicImage, AppError> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let out_dir = cache_dir.join(format!(".ql-{}", key));
    let _ = fs::remove_dir_all(&out_dir);
    fs::create_dir_all(&out_dir)?;
    let result = (|| {
        let mut child = Command::new("/usr/bin/qlmanage")
            .arg("-t")
            .args(["-s", &VIDEO_THUMBNAIL_MAX_SIDE.to_string()])
            .arg("-o")
            .arg(&out_dir)
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| AppError::Internal(format!("qlmanage failed to start: {}", e)))?;
        // QuickLook can stall on odd files; don't hold a worker thread forever.
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AppError::Internal("qlmanage timed out".to_string()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let png = fs::read_dir(&out_dir)?
            .flatten()
            .map(|entry| entry.path())
            .find(|p| p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("png")))
            .ok_or_else(|| AppError::Internal("QuickLook made no thumbnail".to_string()))?;
        image::open(&png).map_err(|_| decode_error())
    })();
    let _ = fs::remove_dir_all(&out_dir);
    result
}

/// The shell's IShellItemImageFactory: the same thumbnails Explorer shows,
/// including codecs installed system-wide.

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system_video_frame(_path: &Path, _cache_dir: &Path, _key: &str) -> Result<DynamicImage, AppError> {
    Err(AppError::Internal("no system video thumbnailer".to_string()))
}

/// Target size that keeps the aspect ratio with the longest side <= `max_side`.
/// `None` means the image already fits and should be used as-is.
pub fn fit_within(width: u32, height: u32, max_side: u32) -> Option<(u32, u32)> {
    if width <= max_side && height <= max_side {
        return None;
    }
    let scale = max_side as f64 / width.max(height) as f64;
    let scaled = |v: u32| ((v as f64 * scale).round() as u32).clamp(1, max_side);
    Some((scaled(width), scaled(height)))
}

/// Cache file stem derived from (canonical path, size, mtime); any edit changes it.
pub fn thumbnail_cache_key(canonical_path: &Path, file_size: u64, mtime_nanos: u128) -> String {
    let mut hasher = Sha256::new();
    hasher.update(THUMBNAIL_CACHE_VERSION.as_bytes());
    hasher.update(THUMBNAIL_MAX_SIDE.to_le_bytes());
    hasher.update(canonical_path.to_string_lossy().as_bytes());
    hasher.update([0u8]);
    hasher.update(file_size.to_le_bytes());
    hasher.update(mtime_nanos.to_le_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn build_thumbnail(path: &str, cache_dir: &Path) -> Result<String, AppError> {
    let canonical =
        fs::canonicalize(path).map_err(|e| AppError::IO(format!("image not accessible: {}", e)))?;
    let meta = fs::metadata(&canonical)
        .map_err(|e| AppError::IO(format!("image not accessible: {}", e)))?;
    if !meta.is_file() {
        return Err(AppError::IO("image path is not a file".to_string()));
    }
    let mtime_nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let key = thumbnail_cache_key(&canonical, meta.len(), mtime_nanos);
    let generation = thumbnail_generation(cache_dir, &key);
    let _generation = generation.lock().unwrap();

    // Cache hit: reuse without decoding anything.
    for ext in ["jpg", "png"] {
        let cached = cache_dir.join(format!("{}.{}", key, ext));
        if cached.is_file() {
            return Ok(cached.to_string_lossy().into_owned());
        }
    }

    let mut decoder = open_reader(&canonical)?
        .into_decoder()
        .map_err(|_| decode_error())?;
    let (width, height) = decoder.dimensions();
    if fit_within(width, height, THUMBNAIL_MAX_SIDE).is_none() {
        return Ok(path.to_string());
    }
    #[cfg(target_os = "macos")]
    let native = native_thumbnail::decode(&canonical, THUMBNAIL_MAX_SIDE);
    #[cfg(not(target_os = "macos"))]
    let native: Option<DynamicImage> = None;
    let thumb = if let Some(native) = native {
        native
    } else {
        // Keep support for formats the Rust decoder can read on its own.
        let orientation = decoder.orientation().ok();
        let mut image = DynamicImage::from_decoder(decoder).map_err(|_| decode_error())?;
        if let Some(orientation) = orientation {
            image.apply_orientation(orientation);
        }
        if let Some((width, height)) = fit_within(image.width(), image.height(), THUMBNAIL_MAX_SIDE)
        {
            image.resize_exact(width, height, FilterType::Lanczos3)
        } else {
            image
        }
    };

    let rgba = thumb.color().has_alpha().then(|| thumb.to_rgba8());
    let (ext, bytes) = if let Some(rgba) = rgba.filter(|rgba| rgba.pixels().any(|p| p.0[3] < 255)) {
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(rgba)
            .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
            .map_err(|e| AppError::Internal(format!("thumbnail encode failed: {}", e)))?;
        ("png", out)
    } else {
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, THUMBNAIL_JPEG_QUALITY)
            .encode_image(&thumb.to_rgb8())
            .map_err(|e| AppError::Internal(format!("thumbnail encode failed: {}", e)))?;
        ("jpg", out)
    };

    fs::create_dir_all(cache_dir)?;
    let final_path = cache_dir.join(format!("{}.{}", key, ext));
    write_atomically(&final_path, &bytes)?;
    Ok(final_path.to_string_lossy().into_owned())
}

fn open_reader(path: &Path) -> Result<ImageReader<BufReader<fs::File>>, AppError> {
    ImageReader::open(path)
        .map_err(|e| AppError::IO(format!("image not accessible: {}", e)))?
        .with_guessed_format()
        .map_err(|e| AppError::IO(format!("image not accessible: {}", e)))
}

fn decode_error() -> AppError {
    AppError::Internal("failed to decode image".to_string())
}

#[cfg(target_os = "macos")]
mod native_thumbnail {
    use std::ffi::c_void;
    use std::path::Path;
    use std::ptr::{null, NonNull};

    use image::DynamicImage;
    use objc2::rc::{autoreleasepool, Retained};
    use objc2_foundation::{NSDictionary, NSMutableData, NSNumber, NSString, NSURL};

    type Ref = *const c_void;

    #[link(name = "ImageIO", kind = "framework")]
    extern "C" {
        static kCGImageSourceCreateThumbnailFromImageAlways: Ref;
        static kCGImageSourceCreateThumbnailWithTransform: Ref;
        static kCGImageSourceThumbnailMaxPixelSize: Ref;
        fn CGImageSourceCreateWithURL(url: Ref, options: Ref) -> *mut c_void;
        fn CGImageSourceCreateThumbnailAtIndex(
            source: Ref,
            index: usize,
            options: Ref,
        ) -> *mut c_void;
        fn CGImageDestinationCreateWithData(
            data: Ref,
            format: Ref,
            count: usize,
            options: Ref,
        ) -> *mut c_void;
        fn CGImageDestinationAddImage(destination: Ref, image: Ref, options: Ref);
        fn CGImageDestinationFinalize(destination: Ref) -> u8;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(object: Ref);
    }

    struct Owned(NonNull<c_void>);

    impl Owned {
        fn new(ptr: *mut c_void) -> Option<Self> {
            NonNull::new(ptr).map(Self)
        }

        fn as_ref(&self) -> Ref {
            self.0.as_ptr()
        }
    }

    impl Drop for Owned {
        fn drop(&mut self) {
            // ImageIO create functions return a retained Core Foundation object.
            unsafe { CFRelease(self.as_ref()) };
        }
    }

    pub(super) fn decode(path: &Path, max_side: u32) -> Option<DynamicImage> {
        autoreleasepool(|_| {
            let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
            let yes = NSNumber::new_bool(true);
            let pixels = NSNumber::new_u32(max_side);
            let data = NSMutableData::new();
            let png = NSString::from_str("public.png");
            // NSURL, NSDictionary, NSString and NSMutableData bridge to the
            // corresponding Core Foundation types. All outlive the calls.
            unsafe {
                let options = NSDictionary::<NSString, NSNumber>::from_slices(
                    &[
                        &*kCGImageSourceCreateThumbnailFromImageAlways.cast::<NSString>(),
                        &*kCGImageSourceCreateThumbnailWithTransform.cast::<NSString>(),
                        &*kCGImageSourceThumbnailMaxPixelSize.cast::<NSString>(),
                    ],
                    &[&*yes, &*yes, &*pixels],
                );
                let source = Owned::new(CGImageSourceCreateWithURL(
                    Retained::as_ptr(&url).cast(),
                    null(),
                ))?;
                let image = Owned::new(CGImageSourceCreateThumbnailAtIndex(
                    source.as_ref(),
                    0,
                    Retained::as_ptr(&options).cast(),
                ))?;
                let destination = Owned::new(CGImageDestinationCreateWithData(
                    Retained::as_ptr(&data).cast(),
                    Retained::as_ptr(&png).cast(),
                    1,
                    null(),
                ))?;
                CGImageDestinationAddImage(destination.as_ref(), image.as_ref(), null());
                if CGImageDestinationFinalize(destination.as_ref()) == 0 {
                    return None;
                }
            }
            image::load_from_memory_with_format(&data.to_vec(), image::ImageFormat::Png).ok()
        })
    }
}

/// Write via a temp file + rename so concurrent callers never see a partial file.
fn write_atomically(final_path: &PathBuf, bytes: &[u8]) -> Result<(), AppError> {
    let temp_path = final_path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temp_path, bytes)?;
    if let Err(e) = fs::rename(&temp_path, final_path) {
        let _ = fs::remove_file(&temp_path);
        // Another caller may have produced the same thumbnail first.
        if !final_path.is_file() {
            return Err(e.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_within_keeps_aspect_ratio() {
        assert_eq!(fit_within(320, 320, 320), None);
        assert_eq!(fit_within(100, 50, 320), None);
        assert_eq!(fit_within(3840, 2160, 320), Some((320, 180)));
        assert_eq!(fit_within(1080, 2400, 320), Some((144, 320)));
        assert_eq!(fit_within(321, 100, 320), Some((320, 100)));
        // Extreme panoramas never collapse to 0 px.
        assert_eq!(fit_within(100_000, 10, 320), Some((320, 1)));
    }

    #[test]
    fn cache_key_changes_with_size_and_mtime() {
        let path = Path::new("/tmp/shot.png");
        let base = thumbnail_cache_key(path, 1024, 1_700_000_000_000_000_000);
        assert_eq!(
            base,
            thumbnail_cache_key(path, 1024, 1_700_000_000_000_000_000)
        );
        assert_ne!(
            base,
            thumbnail_cache_key(path, 1025, 1_700_000_000_000_000_000)
        );
        assert_ne!(
            base,
            thumbnail_cache_key(path, 1024, 1_700_000_000_000_000_001)
        );
        assert_ne!(
            base,
            thumbnail_cache_key(Path::new("/tmp/other.png"), 1024, 1_700_000_000_000_000_000)
        );
        assert_eq!(base.len(), 64);
    }

    #[test]
    fn builds_and_reuses_cached_thumbnail() {
        let root = std::env::temp_dir().join(format!(
            "cp-thumb-test-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let cache = root.join("thumbnails");
        fs::create_dir_all(&root).unwrap();

        let big = root.join("big.png");
        image::RgbImage::from_pixel(2000, 1000, image::Rgb([10, 20, 30]))
            .save(&big)
            .unwrap();
        let first = build_thumbnail(big.to_str().unwrap(), &cache).unwrap();
        assert!(first.ends_with(".jpg"));
        assert_eq!(image::image_dimensions(&first).unwrap(), (800, 400));
        assert_eq!(
            build_thumbnail(big.to_str().unwrap(), &cache).unwrap(),
            first
        );

        let alpha = root.join("alpha.png");
        image::RgbaImage::from_pixel(1400, 1400, image::Rgba([0, 0, 0, 0]))
            .save(&alpha)
            .unwrap();
        assert!(build_thumbnail(alpha.to_str().unwrap(), &cache)
            .unwrap()
            .ends_with(".png"));

        let small = root.join("small.png");
        image::RgbImage::new(100, 80).save(&small).unwrap();
        let small_str = small.to_str().unwrap();
        assert_eq!(build_thumbnail(small_str, &cache).unwrap(), small_str);

        assert!(matches!(
            build_thumbnail(root.join("missing.png").to_str().unwrap(), &cache),
            Err(AppError::IO(_))
        ));
        let broken = root.join("broken.png");
        fs::write(&broken, b"not an image").unwrap();
        assert!(matches!(
            build_thumbnail(broken.to_str().unwrap(), &cache),
            Err(AppError::Internal(_))
        ));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn imageio_decodes_to_the_requested_size() {
        let path = std::env::temp_dir().join(format!("cp-imageio-{}.png", uuid::Uuid::new_v4()));
        image::RgbaImage::from_pixel(2400, 1200, image::Rgba([50, 100, 150, 128]))
            .save(&path)
            .unwrap();
        let thumbnail = native_thumbnail::decode(&path, 320).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!((thumbnail.width(), thumbnail.height()), (320, 160));
        assert_eq!(thumbnail.to_rgba8().get_pixel(0, 0).0[3], 128);
    }

    #[test]
    fn session_thumbnail_cache_lives_with_its_attachments() {
        let root = std::env::temp_dir().join(format!("cp-session-thumb-{}", uuid::Uuid::new_v4()));
        let attachment = root.join("session/attachments/shot.png");
        fs::create_dir_all(attachment.parent().unwrap()).unwrap();
        image::RgbImage::new(900, 450).save(&attachment).unwrap();
        let cache = session_thumbnail_dir(&root, &attachment).unwrap();
        let thumbnail = build_thumbnail(attachment.to_str().unwrap(), &cache).unwrap();
        assert!(Path::new(&thumbnail).starts_with(root.join("session/thumbnails")));
        remove_cached_thumbnails(&attachment);
        assert!(!Path::new(&thumbnail).exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
