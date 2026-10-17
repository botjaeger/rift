//! Quitting: `event_loop.exit()` plus a watchdog that ends the process when
//! the event loop does not stop.
//!
//! On macOS winit stops the loop with `[NSApp stop:]`, which ends only the
//! outermost `NSApp.run`. While a nested AppKit loop is open — a menu the
//! menu bar opened (macOS 27 opens them from the run loop, outside any winit
//! handler), a tracking session — the stop never lands: winit re-requests it
//! after every pass, each pass wakes the menu loop again, and the process
//! spins at full CPU with its memory growing for as long as the menu stays
//! open (an update's window, 2026-10-15: never exited, so the new version
//! never started, and grew to 19 GB). Every caller saved the session before
//! quitting, so after [`DEADLINE`] the watchdog ends the process itself.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use winit::event_loop::ActiveEventLoop;

/// How long the event loop has to stop before the process ends anyway.
pub const DEADLINE: Duration = Duration::from_secs(3);

/// Set by `main` once `run_app` returned: teardown is under way, and the
/// watchdog leaves it alone.
pub static LOOP_RETURNED: AtomicBool = AtomicBool::new(false);
/// One watchdog per process.
static ARMED: AtomicBool = AtomicBool::new(false);

/// Stop the event loop, and end the process if it has not stopped within
/// [`DEADLINE`]. The caller saved what must survive (the session, an
/// update's restart file) first.
pub fn exit(event_loop: &ActiveEventLoop) {
    event_loop.exit();
    if ARMED.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("quit-watchdog".into())
        .spawn(|| {
            watch(DEADLINE, &LOOP_RETURNED, || {
                log::warn!("quit: the event loop did not stop in {DEADLINE:?}; exiting");
                rift_core::shell_complete::stop_all();
                std::process::exit(0)
            })
        });
    if spawned.is_err() {
        log::warn!("quit: could not start the watchdog");
    }
}

/// After `deadline`, `end` unless `returned` was set meanwhile. Returns
/// whether it ended.
fn watch(deadline: Duration, returned: &AtomicBool, end: impl FnOnce()) -> bool {
    std::thread::sleep(deadline);
    if returned.load(Ordering::SeqCst) {
        return false;
    }
    end();
    true
}

/// Route AppKit's own quits (the Dock's Quit, `quit app "Rift"` from
/// AppleScript) through the app's quit, which asks first while a phone is
/// connected and ends every shell: `ask` delivers the request to the event
/// loop. Without this, `terminate:` ends the process at once, like a crash,
/// and the next launch reattaches everything. A logout, restart or shutdown
/// (the quit event carries a reason) still ends the process at once: the
/// system cannot wait for a dialog, and the layout comes back afterwards.
#[cfg(target_os = "macos")]
pub fn route_app_quit(ask: impl Fn() + Send + Sync + 'static) {
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{MainThreadMarker, msg_send};
    use std::sync::OnceLock;

    static ASK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
    /// `NSApplicationTerminateReply`
    const CANCEL: usize = 0;
    const NOW: usize = 1;
    /// `keyAEQuitReason` ('why?'): set by loginwindow on a logout, restart
    /// or shutdown, absent on a quit the user chose in an app
    const QUIT_REASON: u32 = u32::from_be_bytes(*b"why?");

    extern "C-unwind" fn should_terminate(
        _this: *mut AnyObject,
        _cmd: Sel,
        _app: *mut AnyObject,
    ) -> usize {
        // the app's own quit is under way, or the system is logging out
        if ARMED.load(Ordering::SeqCst) || system_quit() {
            return NOW;
        }
        match ASK.get() {
            Some(ask) => {
                ask();
                CANCEL
            }
            None => NOW,
        }
    }

    fn system_quit() -> bool {
        // SAFETY: plain Foundation getters; a nil event or attribute is
        // checked before use
        unsafe {
            let Some(manager) = AnyClass::get(c"NSAppleEventManager") else {
                return false;
            };
            let manager: *mut AnyObject = msg_send![manager, sharedAppleEventManager];
            if manager.is_null() {
                return false;
            }
            let event: *mut AnyObject = msg_send![manager, currentAppleEvent];
            if event.is_null() {
                return false;
            }
            let reason: *mut AnyObject =
                msg_send![event, attributeDescriptorForKeyword: QUIT_REASON];
            !reason.is_null()
        }
    }

    if ASK.set(Box::new(ask)).is_err() {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    // SAFETY: `delegate` is winit's application delegate, alive for the
    // whole run; the replacement has the selector's exact signature
    // (`- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)`,
    // type "Q@:@"), and this process has one delegate of that class
    unsafe {
        let delegate: *mut AnyObject = msg_send![&*app, delegate];
        if delegate.is_null() {
            log::warn!("quit: no application delegate; the Dock's Quit ends shells without asking");
            return;
        }
        let cls = objc2::ffi::object_getClass(delegate) as *mut AnyClass;
        let imp: Imp = std::mem::transmute::<
            extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize,
            Imp,
        >(should_terminate);
        objc2::ffi::class_replaceMethod(
            cls,
            objc2::sel!(applicationShouldTerminate:),
            imp,
            c"Q@:@".as_ptr(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_that_did_not_stop_ends_the_process() {
        let returned = AtomicBool::new(false);
        let mut ended = false;
        assert!(watch(Duration::from_millis(10), &returned, || ended = true));
        assert!(ended);
    }

    #[test]
    fn every_quit_arms_the_watchdog() {
        // a bare `event_loop.exit()` hangs under an open menu (module doc)
        for (name, src) in [
            ("app.rs", include_str!("app.rs")),
            ("control.rs", include_str!("control.rs")),
            ("script.rs", include_str!("script.rs")),
            ("updater.rs", include_str!("updater.rs")),
        ] {
            assert!(
                !src.contains("event_loop.exit()"),
                "{name} quits without crate::quit::exit"
            );
        }
    }

    #[test]
    fn a_loop_that_stopped_tears_down_undisturbed() {
        let returned = AtomicBool::new(true);
        let mut ended = false;
        assert!(!watch(Duration::from_millis(10), &returned, || ended = true));
        assert!(!ended);
    }
}
