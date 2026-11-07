#[cfg(target_os = "macos")]
use objc2::rc::{autoreleasepool, Retained};
#[cfg(target_os = "macos")]
use objc2::runtime::ProtocolObject;
#[cfg(target_os = "macos")]
use objc2::ClassType;
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypePNG, NSPasteboardTypeRTF,
    NSPasteboardTypeRTFD, NSPasteboardTypeString, NSPasteboardWriting,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString, NSURL};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

pub struct ImageData {
    pub width: usize,
    pub height: usize,
    pub bytes: Vec<u8>,
}

static SEQ: AtomicU32 = AtomicU32::new(1);
static LAST_SIG: OnceLock<Mutex<u64>> = OnceLock::new();
const CLIPBOARD_SIG_MAX_BYTES: usize = 32 * 1024;

fn compute_clipboard_signature() -> u64 {
    let mut hasher = DefaultHasher::new();

    #[cfg(target_os = "macos")]
    autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        // We only hash a small part of the string for signature if it's text.
        // For performance, we prefer checking the change count first if available,
        // but for deduplication we still need a signature.
        if let Some(text) = unsafe { pb.stringForType(NSPasteboardTypeString) } {
            let s = text.to_string();
            let len = s.len().min(CLIPBOARD_SIG_MAX_BYTES);
            s[..len].hash(&mut hasher);
        } else if let Some(html) = unsafe { pb.stringForType(NSPasteboardTypeHTML) } {
            let s = html.to_string();
            let len = s.len().min(CLIPBOARD_SIG_MAX_BYTES);
            s[..len].hash(&mut hasher);
        } else if unsafe { pb.dataForType(NSPasteboardTypeRTF).is_some() } {
            "__RTF__".hash(&mut hasher);
        } else if unsafe { pb.dataForType(NSPasteboardTypeRTFD).is_some() } {
            "__RTFD__".hash(&mut hasher);
        } else {
            // Check for image types too
            if unsafe { pb.dataForType(NSPasteboardTypePNG).is_some() } {
                "__IMAGE_PNG__".hash(&mut hasher);
            }
        }
    });

    hasher.finish()
}

/// changeCount of a pasteboard write the app made that must not be recorded
/// (restoring the user's previous clipboard after a transient paste).
static OWN_CHANGE_COUNT: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(-1);

/// NSPasteboard's change counter (bumped on every write by any app).
pub fn change_count() -> isize {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| NSPasteboard::generalPasteboard().changeCount() as isize);

}

/// The pasteboard still holds the write restore_pasteboard marked as the app's own.
pub fn is_own_change() -> bool {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        NSPasteboard::generalPasteboard().changeCount() as isize == OWN_CHANGE_COUNT.load(Ordering::SeqCst)
    });

}

/// Every item on the pasteboard with all its types as raw data, so it can be
/// put back exactly (images, rich text and files included).
pub struct PasteboardSnapshot {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    items: Vec<Vec<(String, Vec<u8>)>>,
}

/// None when the content is larger than `max_bytes` in total.
pub fn snapshot_pasteboard(max_bytes: usize) -> Option<PasteboardSnapshot> {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        let mut total = 0usize;
        let mut items = Vec::new();
        if let Some(pasteboard_items) = pb.pasteboardItems() {
            for item in pasteboard_items.iter() {
                let mut entries = Vec::new();
                for pasteboard_type in item.types().iter() {
                    if let Some(data) = item.dataForType(&pasteboard_type) {
                        total += data.len();
                        if total > max_bytes {
                            return None;
                        }
                        entries.push((pasteboard_type.to_string(), data.to_vec()));
                    }
                }
                items.push(entries);
            }
        }
        Some(PasteboardSnapshot { items })
    });

}

/// Put a snapshot back. The write is marked as the app's own, so it is not
/// recorded as a new copy.
pub fn restore_pasteboard(snapshot: &PasteboardSnapshot) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        use objc2_app_kit::NSPasteboardItem;
        let pb = NSPasteboard::generalPasteboard();
        // Mark the write as ours before the data goes in: the listener can see
        // the new change count while large snapshots are still being copied.
        let count = pb.clearContents();
        OWN_CHANGE_COUNT.store(count as isize, Ordering::SeqCst);
        let mut objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = Vec::new();
        for entries in &snapshot.items {
            let item = NSPasteboardItem::new();
            for (pasteboard_type, data) in entries {
                item.setData_forType(&NSData::with_bytes(data), &NSString::from_str(pasteboard_type));
            }
            objects.push(ProtocolObject::from_retained(item));
        }
        let written = objects.is_empty() || pb.writeObjects(&NSArray::from_retained_slice(&objects));
        if written {
            Ok(())
        } else {
            Err("Failed to restore the clipboard".to_string())
        }
    });

}

/// Text that clipboard history apps, the app included, are asked not to record
/// (the nspasteboard.org concealed type): for secrets such as the LAN
/// transfer pairing link.
pub fn set_private_text(text: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let written = pb.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString })
            && pb.setString_forType(&NSString::from_str(""), &NSString::from_str("org.nspasteboard.ConcealedType"));
        if written {
            Ok(())
        } else {
            Err("Failed to write the clipboard".to_string())
        }
    });

}

pub fn get_clipboard_text() -> Option<String> {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        unsafe { pb.stringForType(NSPasteboardTypeString) }.map(|s| s.to_string())
    });

}

/// Pasteboard types that password managers and similar tools add so clipboard
/// history apps never record the content (see nspasteboard.org).
#[cfg(target_os = "macos")]
const PRIVATE_PASTEBOARD_TYPES: &[&str] = &[
    "org.nspasteboard.ConcealedType",
    "org.nspasteboard.TransientType",
    "org.nspasteboard.AutoGeneratedType",
    "de.petermaurer.TransientPasteboardType",
    "com.agilebits.onepassword",
];

pub fn is_clipboard_marked_private() -> bool {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        pb.types().is_some_and(|types| {
            types
                .iter()
                .any(|t| PRIVATE_PASTEBOARD_TYPES.contains(&t.to_string().as_str()))
        })
    });

}

pub fn get_clipboard_sequence_number() -> u32 {
    #[cfg(target_os = "macos")]
    {
        // Try getting the actual change count from NSPasteboard for maximum efficiency.
        let _change_count = autoreleasepool(|_| {
            let pb = NSPasteboard::generalPasteboard();
            pb.changeCount() as u32
        });

        // We still check the signature to avoid false positives with app-set clipboard
        // but change_count is our primary trigger.
        let sig = compute_clipboard_signature();
        let guard = LAST_SIG.get_or_init(|| Mutex::new(0));
        let mut last_sig = guard.lock().unwrap();

        if *last_sig != sig {
            *last_sig = sig;
            // Use change_count as a seed or just increment our own SEQ.
            // Using our own SEQ is safer for our internal logic.
            SEQ.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
        } else {
            SEQ.load(Ordering::Relaxed)
        }
    }

}

pub fn get_clipboard_image() -> Option<ImageData> {
    None
}

pub fn get_clipboard_files() -> Option<Vec<String>> {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        let class_objs = unsafe {
            let ns_url_class: *const objc2::runtime::AnyClass = NSURL::class();
            NSArray::from_retained_slice(&[Retained::from_raw(
                ns_url_class as *mut objc2::runtime::AnyClass,
            )
            .unwrap()])
        };
        let options = NSDictionary::<NSString, objc2::runtime::AnyObject>::new();
        let items = unsafe { pb.readObjectsForClasses_options(&class_objs, Some(&options)) };

        if let Some(urls) = items {
            let mut paths = Vec::new();
            for i in 0..urls.count() {
                let url_any = urls.objectAtIndex(i);
                let url: Retained<NSURL> = unsafe { Retained::cast_unchecked(url_any) };
                if url.isFileURL() {
                    if let Some(path) = url.path() {
                        paths.push(path.to_string());
                    }
                }
            }
            if paths.is_empty() {
                None
            } else {
                Some(paths)
            }
        } else {
            None
        }
    });

}

pub fn get_clipboard_raw_format(_name: &str) -> Option<Vec<u8>> {
    None
}

/// Image types read as-is, best first. GIF keeps its animation, PNG its alpha
/// and colour profile; decoding the TIFF that apps also offer (what arboard
/// reads) flattened GIFs to one frame and cost a full decode per copy.
#[cfg(target_os = "macos")]
const RAW_IMAGE_TYPES: &[(&str, &str)] = &[
    ("com.compuserve.gif", "gif"),
    ("public.png", "png"),
    ("public.jpeg", "jpg"),
    ("org.webmproject.webp", "webp"),
];

/// The clipboard image in its original encoding, with its file extension.
pub fn get_clipboard_image_bytes() -> Option<(Vec<u8>, &'static str)> {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        RAW_IMAGE_TYPES.iter().find_map(|(uti, ext)| {
            let data = pb.dataForType(&NSString::from_str(uti))?;
            let bytes = data.to_vec();
            (!bytes.is_empty()).then_some((bytes, *ext))
        })
    });

}

/// Put an image on the pasteboard the way browsers and Finder do: the original
/// bytes under their own type, a static PNG (and TIFF, for older apps) for
/// readers that want a bitmap, and optionally the file itself so apps that take
/// files (chat apps, Finder) get the original, e.g. an animated GIF.
pub fn set_clipboard_image(
    original: &[u8],
    extension: &str,
    static_png: Option<&[u8]>,
    file_path: Option<&str>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        use objc2_app_kit::{NSBitmapImageRep, NSPasteboardTypeFileURL, NSPasteboardTypeTIFF};
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();

        let original_type = match extension {
            "gif" => "com.compuserve.gif",
            "jpg" | "jpeg" => "public.jpeg",
            "webp" => "org.webmproject.webp",
            "bmp" => "com.microsoft.bmp",
            _ => "public.png",
        };
        let ns_type = NSString::from_str(original_type);
        if !pb.setData_forType(Some(&NSData::with_bytes(original)), &ns_type) {
            return Err("Failed to write image to pasteboard".to_string());
        }

        let png = static_png.or((original_type == "public.png").then_some(original));
        if let Some(png) = png {
            let png_data = NSData::with_bytes(png);
            if original_type != "public.png" {
                unsafe { pb.setData_forType(Some(&png_data), NSPasteboardTypePNG) };
            }
            if let Some(tiff) = NSBitmapImageRep::imageRepWithData(&png_data)
                .and_then(|rep| rep.TIFFRepresentation())
            {
                unsafe { pb.setData_forType(Some(&tiff), NSPasteboardTypeTIFF) };
            }
        }

        if let Some(path) = file_path {
            let url = NSURL::fileURLWithPath(&NSString::from_str(path));
            if let Some(url_string) = url.absoluteString() {
                unsafe { pb.setString_forType(&url_string, NSPasteboardTypeFileURL) };
            }
        }
        Ok(())
    });

}

pub fn get_clipboard_html() -> Option<String> {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        unsafe { pb.stringForType(NSPasteboardTypeHTML) }.map(|s| s.to_string())
    });

}

pub fn get_clipboard_rtf() -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        if let Some(data) = unsafe { pb.dataForType(NSPasteboardTypeRTF) } {
            return Some(data.to_vec());
        }
        unsafe { pb.dataForType(NSPasteboardTypeRTFD) }.map(|data| data.to_vec())
    });

}

pub fn set_clipboard_files(paths: Vec<String>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        if paths.is_empty() {
            return Ok(());
        }

        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();

        let mut objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
            Vec::with_capacity(paths.len());
        for path in &paths {
            let ns_path = NSString::from_str(path);
            let url = NSURL::fileURLWithPath(&ns_path);
            let proto = ProtocolObject::from_retained(url);
            objects.push(proto);
        }

        let ns_array = NSArray::from_retained_slice(&objects);
        if pb.writeObjects(&ns_array) {
            Ok(())
        } else {
            Err("Failed to write file objects to pasteboard".to_string())
        }
    });

}

pub fn set_clipboard_text_html_and_image(
    text: &str,
    html: &str,
    png_bytes: Option<Vec<u8>>,
) -> Result<(), String> {
    set_clipboard_text_html_rtf_and_image(text, html, None, png_bytes)
}

pub fn set_clipboard_text_html_rtf_and_image(
    text: &str,
    html: &str,
    rtf_bytes: Option<Vec<u8>>,
    png_bytes: Option<Vec<u8>>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();

        let ns_text = NSString::from_str(text);
        unsafe { pb.setString_forType(&ns_text, NSPasteboardTypeString) };

        if !html.is_empty() {
            let ns_html = NSString::from_str(html);
            unsafe { pb.setString_forType(&ns_html, NSPasteboardTypeHTML) };
        }

        if let Some(bytes) = rtf_bytes {
            let ns_data = NSData::from_vec(bytes);
            unsafe { pb.setData_forType(Some(&ns_data), NSPasteboardTypeRTF) };
        }

        // The fallback may be a GIF or JPEG; label it by its real type (it used
        // to go out as PNG), and give GIFs a static PNG for bitmap-only readers.
        if let Some(bytes) = png_bytes {
            let (uti, static_png) = if bytes.starts_with(b"GIF") {
                let first_frame = image::load_from_memory(&bytes).ok().and_then(|img| {
                    let mut png = Vec::new();
                    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                        .ok()
                        .map(|_| png)
                });
                ("com.compuserve.gif", first_frame)
            } else if bytes.starts_with(b"\xFF\xD8\xFF") {
                ("public.jpeg", None)
            } else {
                ("public.png", None)
            };
            let ns_data = NSData::from_vec(bytes);
            pb.setData_forType(Some(&ns_data), &NSString::from_str(uti));
            if let Some(png) = static_png {
                unsafe { pb.setData_forType(Some(&NSData::from_vec(png)), NSPasteboardTypePNG) };
            }
        }

        Ok(())
    });

}

pub fn set_clipboard_text_and_html(text: &str, html: &str) -> Result<(), String> {
    set_clipboard_text_html_rtf_and_image(text, html, None, None)
}

pub fn append_clipboard_text_and_html(_text: &str, _html: &str) -> Result<(), String> {
    Ok(())
}

pub fn set_clipboard_image_with_formats(
    _data: ImageData,
    _gif_data: Option<&Vec<u8>>,
    _png_data: Option<&Vec<u8>>,
    _file_path: Option<&str>,
) -> Result<Option<String>, String> {
    Ok(None)
}
pub fn convert_rtf_to_html(rtf_bytes: &[u8]) -> Option<String> {

    #[cfg(target_os = "macos")]
    {
        let mut child = Command::new("textutil")
            .args([
                "-convert",
                "html",
                "-stdin",
                "-stdout",
                "-format",
                "rtf",
                "-encoding",
                "UTF-8",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;

        if let Some(stdin) = child.stdin.as_mut() {
            if stdin.write_all(rtf_bytes).is_err() {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        } else {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }

        let output = child.wait_with_output().ok()?;
        if !output.status.success() {
            return None;
        }

        let html = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if html.is_empty() {
            None
        } else {
            Some(html)
        }
    }
}
