//! The system share menu (NSSharingServicePicker), the one Finder shows for
//! 共享…: AirDrop, 信息, 邮件, 备忘录 and so on.

use std::cell::RefCell;
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread, MainThreadMarker};
use objc2_app_kit::{NSSharingServicePicker, NSView};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};
use tauri::WebviewWindow;

/// What gets shared. Files go as file URLs, the way Finder shares them.
pub enum ShareItem {
    Files(Vec<PathBuf>),
    Url(String),
    Text(String),
}

/// The anchor in page (CSS) pixels, measured from the top-left of the WebView.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct AnchorRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

thread_local! {
    // The picker must outlive this call; the next one replaces it.
    static CURRENT_PICKER: RefCell<Option<Retained<NSSharingServicePicker>>> = const { RefCell::new(None) };
}

fn ns_items(item: &ShareItem) -> Vec<Retained<AnyObject>> {
    match item {
        ShareItem::Files(paths) => paths
            .iter()
            .filter_map(|path| path.to_str())
            .map(|path| Retained::into_super(Retained::into_super(NSURL::fileURLWithPath(&NSString::from_str(path)))))
            .collect(),
        ShareItem::Url(url) => match NSURL::URLWithString(&NSString::from_str(url.trim())) {
            Some(ns_url) => vec![Retained::into_super(Retained::into_super(ns_url))],
            None => vec![Retained::into_super(Retained::into_super(NSString::from_str(url)))],
        },
        ShareItem::Text(text) => vec![Retained::into_super(Retained::into_super(NSString::from_str(text)))],
    }
}

/// Opens the share menu below `anchor` (the button that was clicked).
pub fn show_share_picker(window: &WebviewWindow, item: ShareItem, anchor: AnchorRect) -> Result<(), String> {
    window
        .with_webview(move |webview| {
            let Some(_mtm) = MainThreadMarker::new() else { return };
            let objects = ns_items(&item);
            if objects.is_empty() {
                return;
            }
            let refs: Vec<&AnyObject> = objects.iter().map(|object| &**object).collect();
            let array = NSArray::from_slice(&refs);
            let view = unsafe { &*(webview.inner() as *const NSView) };

            // AppKit counts y from the bottom unless the view is flipped, and
            // the edge is in the view's own coordinates: the menu goes below.
            let flipped = view.isFlipped();
            let y = if flipped {
                anchor.y
            } else {
                view.bounds().size.height - anchor.y - anchor.height
            };
            let below = if flipped { NSRectEdge::MaxY } else { NSRectEdge::MinY };
            let rect = NSRect::new(NSPoint::new(anchor.x, y), NSSize::new(anchor.width, anchor.height));

            let picker = unsafe { NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &array) };
            picker.showRelativeToRect_ofView_preferredEdge(rect, view, below);
            CURRENT_PICKER.with(|slot| *slot.borrow_mut() = Some(picker));
        })
        .map_err(|error| error.to_string())
}
