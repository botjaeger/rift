//! macOS: centre the native traffic lights on the tab strip. With a
//! transparent full-size titlebar AppKit pins them to its own 28pt titlebar,
//! above the middle of the 42px strip. The buttons are moved inside their
//! container and the container is grown to the strip height (the Tauri /
//! Zed approach); AppKit lays the titlebar out again on every resize, so this
//! runs after each one.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSView, NSWindowButton};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Centre the buttons vertically in the top `bar_h` points, the close button
/// at `first_center_x` and each next one `spacing` further right (the drawn
/// Linux buttons use the same centres, `tabbar::TRAFFIC_X`).
pub fn place(window: &winit::window::Window, bar_h: f64, first_center_x: f64, spacing: f64) {
    if MainThreadMarker::new().is_none() || window.fullscreen().is_some() {
        return;
    }
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    // SAFETY: winit's AppKit handle is a live NSView owned by `window`
    let view: &NSView = unsafe { appkit.ns_view.cast().as_ref() };
    keep_window_still(view);
    let Some(ns_window) = view.window() else {
        return;
    };
    let buttons = [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ]
    .map(|b| ns_window.standardWindowButton(b));
    let [Some(close), _, _] = &buttons else {
        return;
    };
    // button → NSTitlebarView → NSTitlebarContainerView
    // SAFETY: AppKit keeps both superviews alive while the window lives
    let Some(container) = (unsafe { close.superview().and_then(|v| v.superview()) }) else {
        return;
    };
    let size = close.frame().size;
    // the container is bottom-up and anchored to the window top: as tall as
    // the strip, with the buttons vertically centred in it
    let mut frame = container.frame();
    frame.size.height = bar_h;
    frame.origin.y = ns_window.frame().size.height - bar_h;
    // only as wide as the buttons: the titlebar view zooms the window on a
    // double-click and drags it on a press, which must not happen over the
    // tabs (the app drags and zooms from the empty strip itself)
    frame.origin.x = 0.0;
    frame.size.width = first_center_x + 2.0 * spacing + size.width;
    container.setFrame(frame);
    let y = ((bar_h - size.height) / 2.0).round();
    for (i, b) in buttons.iter().enumerate() {
        if let Some(b) = b {
            let x = first_center_x + i as f64 * spacing - size.width / 2.0;
            b.setFrameOrigin(objc2_foundation::NSPoint::new(x.round(), y));
        }
    }
}

/// Hide or show the buttons (they go with a hidden tab strip, `tab_bar`).
pub fn set_hidden(window: &winit::window::Window, hidden: bool) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    // SAFETY: winit's AppKit handle is a live NSView owned by `window`
    let view: &NSView = unsafe { appkit.ns_view.cast().as_ref() };
    let Some(ns_window) = view.window() else {
        return;
    };
    for b in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(b) = ns_window.standardWindowButton(b) {
            b.setHidden(hidden);
        }
    }
}

/// AppKit treats a press in the titlebar band (the top of a full-size
/// content view) on any view whose `mouseDownCanMoveWindow` is YES as a
/// titlebar gesture: drag moves the window, double-click zooms it. winit's
/// layer-backed view is not opaque, so it says YES, and a double-click on a
/// tab zoomed the window. The view's class answers NO from now on; the app
/// drags and zooms from the empty strip itself (`ClickTarget::WindowDrag`).
fn keep_window_still(view: &NSView) {
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use std::sync::Once;

    extern "C-unwind" fn cannot_move_window(_this: *mut AnyObject, _cmd: Sel) -> Bool {
        Bool::NO
    }

    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let obj: *const AnyObject = (view as *const NSView).cast();
        // SAFETY: `obj` is a live NSView; the replacement has the selector's
        // exact signature (`- (BOOL)mouseDownCanMoveWindow`, type "B@:"),
        // and every view of this class is the window's content view
        unsafe {
            let cls = objc2::ffi::object_getClass(obj) as *mut AnyClass;
            let imp: Imp = std::mem::transmute::<
                extern "C-unwind" fn(*mut AnyObject, Sel) -> Bool,
                Imp,
            >(cannot_move_window);
            objc2::ffi::class_replaceMethod(
                cls,
                objc2::sel!(mouseDownCanMoveWindow),
                imp,
                if cfg!(target_arch = "aarch64") {
                    c"B@:".as_ptr()
                } else {
                    c"c@:".as_ptr()
                },
            );
        }
    });
    // SAFETY: plain BOOL getter on a live NSView
    let can_move: bool = unsafe { objc2::msg_send![view, mouseDownCanMoveWindow] };
    log::debug!("content view mouseDownCanMoveWindow={can_move}");
}
