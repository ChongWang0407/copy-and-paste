use crate::error::AppError;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    let Some(dir) = THUMBNAIL_DIR.get() else {
        return;
    };
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
    for ext in ["jpg", "png"] {
        let _ = fs::remove_file(dir.join(format!("{}.{}", key, ext)));
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Returns an absolute path to a thumbnail (longest side <= 320 px) for `path`.
/// Images that already fit return `path` unchanged; larger animated ones get a
/// still thumbnail of their first frame.
#[tauri::command]
pub async fn get_image_thumbnail(app: AppHandle, path: String) -> Result<String, AppError> {
    let cache_dir = thumbnail_dir(&app)?;
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
    let cache_dir = thumbnail_dir(&app)?;
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
            .args(["-s", &(THUMBNAIL_MAX_SIDE * 2).to_string()])
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

    // Cache hit: reuse without decoding anything.
    for ext in ["jpg", "png"] {
        let cached = cache_dir.join(format!("{}.{}", key, ext));
        if cached.is_file() {
            return Ok(cached.to_string_lossy().into_owned());
        }
    }

    let (width, height) = open_reader(&canonical)?
        .into_dimensions()
        .map_err(|_| decode_error())?;
    let Some((target_w, target_h)) = fit_within(width, height, THUMBNAIL_MAX_SIDE) else {
        return Ok(path.to_string());
    };
    // Large animated GIF/WebP get a still thumbnail of their first frame (the
    // decoder below reads only that one): the list plays them on hover only.
    // Every visible row decoding every frame at full size kept the CPU/GPU busy.

    let mut decoder = open_reader(&canonical)?
        .into_decoder()
        .map_err(|_| decode_error())?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(|_| decode_error())?;
    if let Some(orientation) = orientation {
        img.apply_orientation(orientation);
    }
    // Orientation may swap width/height; recompute from the oriented image.
    let (target_w, target_h) =
        fit_within(img.width(), img.height(), THUMBNAIL_MAX_SIDE).unwrap_or((target_w, target_h));
    // Once per image, off the main thread. thumbnail_exact is faster, but its
    // box sampling turns the small text of a screenshot to mush.
    let thumb = img.resize_exact(target_w, target_h, FilterType::Lanczos3);

    let keep_alpha = thumb.color().has_alpha() && thumb.to_rgba8().pixels().any(|p| p.0[3] < 255);
    let (ext, bytes) = if keep_alpha {
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(thumb.to_rgba8())
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
}
