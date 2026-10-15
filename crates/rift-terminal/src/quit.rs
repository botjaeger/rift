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
