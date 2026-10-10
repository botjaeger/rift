//! macOS window translucency, as Warp sets it up (`warpui` `window.m`): the
//! NSWindow background is clear at alpha 0.01, and the content behind the
//! window is blurred through the private `CGSSetWindowBackgroundBlurRadius`
//! (radius ≥ 1; 1 is effectively none). The window keeps its shadow. What
//! the frame itself draws is `Theme::window_ground`.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSColor, NSView};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Apply the background colour and blur radius to `window`'s NSWindow.
pub fn apply(window: &winit::window::Window, blur: u8) {
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
    // alpha 0 with the drop shadow leaves a gap between the window border
    // and its contents (an AppKit quirk Warp works around the same way)
    let clear = NSColor::clearColor().colorWithAlphaComponent(0.01);
    ns_window.setBackgroundColor(Some(&clear));
    set_blur(ns_window.windowNumber(), blur);
}

type ConnectionFn = unsafe extern "C" fn() -> i32;
type SetBlurFn = unsafe extern "C" fn(i32, i32, i32) -> i32;

/// The private SkyLight entry points, looked up once; `None` when a macOS
/// release drops them (blur then silently does nothing, as in Warp).
fn blur_fns() -> Option<(ConnectionFn, SetBlurFn)> {
    static FNS: std::sync::OnceLock<Option<(ConnectionFn, SetBlurFn)>> = std::sync::OnceLock::new();
    *FNS.get_or_init(|| {
        let find = |name: &std::ffi::CStr| {
            // SAFETY: dlsym with a NUL-terminated name; null when absent
            let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
            (!p.is_null()).then_some(p)
        };
        let conn = find(c"CGSDefaultConnectionForThread")?;
        let set = find(c"CGSSetWindowBackgroundBlurRadius")?;
        // SAFETY: the documented-by-use C signatures of these functions
        Some(unsafe {
            (
                std::mem::transmute::<*mut libc::c_void, ConnectionFn>(conn),
                std::mem::transmute::<*mut libc::c_void, SetBlurFn>(set),
            )
        })
    })
}

fn set_blur(window_number: isize, radius: u8) {
    let Ok(window_number) = i32::try_from(window_number) else {
        return;
    };
    let Some((connection, set)) = blur_fns() else {
        return;
    };
    // SAFETY: plain C calls on this process's window-server connection
    unsafe {
        let con = connection();
        if con != 0 {
            set(con, window_number, i32::from(radius.max(1)));
        }
    }
}
