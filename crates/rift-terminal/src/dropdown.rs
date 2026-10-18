//! The drop-down hotkey window's geometry and platform hooks
//! (`hotkey_window`): its frame on a screen edge, the slide, and on macOS
//! the window level and Spaces behaviour, the pointer's and numbered
//! screens, the reduced
//! motion setting and handing focus back to the app that had it. The app
//! side lives in `app/hotkey_window.rs`.

use std::time::Duration;

use rift_core::HotkeyEdge;

/// A rectangle in logical px, top-left origin (winit's window coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The screen used when none can be read (a display-less run).
pub const FALLBACK_SCREEN: Frame = Frame {
    x: 0.0,
    y: 0.0,
    w: 1280.0,
    h: 800.0,
};

/// How long the window takes to slide down or up.
pub const SLIDE: Duration = Duration::from_millis(150);
/// Time between two slide steps (about 120 per second).
pub const SLIDE_STEP: Duration = Duration::from_millis(8);

/// The window's frame on `screen`, against `edge`: `depth_pct` of the
/// screen away from the edge (its height at the top or bottom, its width
/// at the left or right) and `along_pct` along it, centred; both clamped
/// to 20–100 % and rounded to whole px.
pub fn frame(screen: Frame, edge: HotkeyEdge, depth_pct: u8, along_pct: u8) -> Frame {
    let pct = |v: u8| f64::from(v.clamp(20, 100)) / 100.0;
    let (depth, along) = (pct(depth_pct), pct(along_pct));
    match edge {
        HotkeyEdge::Top | HotkeyEdge::Bottom => {
            let w = (screen.w * along).round().max(1.0);
            let h = (screen.h * depth).round().max(1.0);
            Frame {
                x: screen.x + ((screen.w - w) / 2.0).round(),
                y: if edge == HotkeyEdge::Top {
                    screen.y
                } else {
                    screen.y + screen.h - h
                },
                w,
                h,
            }
        }
        HotkeyEdge::Left | HotkeyEdge::Right => {
            let w = (screen.w * depth).round().max(1.0);
            let h = (screen.h * along).round().max(1.0);
            Frame {
                x: if edge == HotkeyEdge::Left {
                    screen.x
                } else {
                    screen.x + screen.w - w
                },
                y: screen.y + ((screen.h - h) / 2.0).round(),
                w,
                h,
            }
        }
    }
}

/// The window's top-left corner `elapsed` into a slide toward `frame`
/// (showing) or away from it (hiding), and whether the slide is over. Off
/// screen is one window size past `edge`; showing eases out, hiding eases
/// in.
pub fn slide_pos(
    frame: Frame,
    edge: HotkeyEdge,
    elapsed: Duration,
    showing: bool,
) -> ((f64, f64), bool) {
    let t = (elapsed.as_secs_f64() / SLIDE.as_secs_f64()).clamp(0.0, 1.0);
    let shown = if showing {
        1.0 - (1.0 - t).powi(3)
    } else {
        1.0 - t.powi(3)
    };
    let off = 1.0 - shown;
    let pos = match edge {
        HotkeyEdge::Top => (frame.x, frame.y - frame.h * off),
        HotkeyEdge::Bottom => (frame.x, frame.y + frame.h * off),
        HotkeyEdge::Left => (frame.x - frame.w * off, frame.y),
        HotkeyEdge::Right => (frame.x + frame.w * off, frame.y),
    };
    (pos, t >= 1.0)
}

/// A monitor's rectangle in logical px.
pub fn monitor_frame(m: &winit::monitor::MonitorHandle) -> Frame {
    let scale = m.scale_factor();
    let pos = m.position().to_logical::<f64>(scale);
    let size = m.size().to_logical::<f64>(scale);
    Frame {
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
    }
}

/// The window slides (macOS, unless the user asked for reduced motion);
/// elsewhere it appears at once.
pub fn animate() -> bool {
    #[cfg(target_os = "macos")]
    {
        !mac::reduce_motion()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// The user asked for reduced motion (macOS Accessibility setting; Linux
/// has no such setting to read): pulsing badges hold still.
pub fn motion_reduced() -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::reduce_motion()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
pub use mac::*;

#[cfg(target_os = "macos")]
mod mac {
    use super::Frame;
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationOptions, NSEvent, NSFloatingWindowLevel,
        NSRunningApplication, NSScreen, NSView, NSWindow, NSWindowCollectionBehavior, NSWorkspace,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    fn ns_window(window: &winit::window::Window) -> Option<Retained<NSWindow>> {
        MainThreadMarker::new()?;
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return None;
        };
        // SAFETY: winit's AppKit handle is a live NSView owned by `window`
        let view: &NSView = unsafe { appkit.ns_view.cast().as_ref() };
        view.window()
    }

    /// Above other apps' windows, on every Space and over full-screen apps,
    /// and left in place by Mission Control.
    pub fn float_everywhere(window: &winit::window::Window) {
        let Some(w) = ns_window(window) else {
            return;
        };
        w.setLevel(NSFloatingWindowLevel);
        w.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary,
        );
    }

    /// The visible part (below the menu bar, beside the Dock) of the screen
    /// that has the mouse pointer, in winit's coordinates (top-left origin
    /// at the menu-bar screen's top-left).
    pub fn pointer_screen() -> Option<Frame> {
        let mtm = MainThreadMarker::new()?;
        let screens = NSScreen::screens(mtm);
        // the first screen is the one with the menu bar: Cocoa's origin
        let primary_h = screens.firstObject()?.frame().size.height;
        let p = NSEvent::mouseLocation();
        let screen = screens
            .iter()
            .find(|s| {
                let f = s.frame();
                p.x >= f.origin.x
                    && p.x < f.origin.x + f.size.width
                    && p.y >= f.origin.y
                    && p.y <= f.origin.y + f.size.height
            })
            .or_else(|| NSScreen::mainScreen(mtm))?;
        let v = screen.visibleFrame();
        Some(Frame {
            x: v.origin.x,
            y: primary_h - (v.origin.y + v.size.height),
            w: v.size.width,
            h: v.size.height,
        })
    }

    /// The visible part of screen `index` (0 is the one with the menu bar),
    /// in winit's coordinates as [`pointer_screen`].
    pub fn screen_at(index: usize) -> Option<Frame> {
        let mtm = MainThreadMarker::new()?;
        let screens = NSScreen::screens(mtm);
        let primary_h = screens.firstObject()?.frame().size.height;
        let v = screens.iter().nth(index)?.visibleFrame();
        Some(Frame {
            x: v.origin.x,
            y: primary_h - (v.origin.y + v.size.height),
            w: v.size.width,
            h: v.size.height,
        })
    }

    /// System Settings › Accessibility › Display › Reduce motion.
    pub fn reduce_motion() -> bool {
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    }

    /// The frontmost app when it is not Rift (its pid), so hiding can hand
    /// focus back to it.
    pub fn frontmost_other_app() -> Option<i32> {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let pid = app.processIdentifier();
        (pid != NSRunningApplication::currentApplication().processIdentifier()).then_some(pid)
    }

    /// Bring app `pid` back to the front.
    pub fn activate_app(pid: i32) {
        if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            app.activateWithOptions(NSApplicationActivationOptions::empty());
        }
    }

    /// Unhide Rift when it is hidden (⌘H): a hidden app shows no window.
    pub fn unhide_app() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        if app.isHidden() {
            app.unhide(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Frame = Frame {
        x: 0.0,
        y: 25.0,
        w: 1440.0,
        h: 875.0,
    };

    #[test]
    fn frame_spans_the_screen_width_at_its_top_edge() {
        let f = frame(SCREEN, HotkeyEdge::Top, 40, 100);
        assert_eq!(
            f,
            Frame {
                x: 0.0,
                y: 25.0,
                w: 1440.0,
                h: 350.0
            }
        );
        // a second screen left of and below the first
        let side = Frame {
            x: -1920.0,
            y: 300.0,
            w: 1920.0,
            h: 1080.0,
        };
        assert_eq!(
            frame(side, HotkeyEdge::Top, 50, 100),
            Frame {
                x: -1920.0,
                y: 300.0,
                w: 1920.0,
                h: 540.0
            }
        );
    }

    #[test]
    fn frame_height_clamps_to_20_through_100_percent() {
        let h = |pct| frame(SCREEN, HotkeyEdge::Top, pct, 100).h;
        assert_eq!(h(0), 175.0);
        assert_eq!(h(19), 175.0);
        assert_eq!(h(100), 875.0);
        assert_eq!(h(255), 875.0);
        // rounded to whole px
        assert_eq!(h(33), 289.0);
    }

    #[test]
    fn frame_hangs_from_each_edge_centred_along_it() {
        let f = |edge| frame(SCREEN, edge, 40, 50);
        let at = |f: Frame| [f.x, f.y, f.w, f.h];
        assert_eq!(at(f(HotkeyEdge::Top)), [360.0, 25.0, 720.0, 350.0]);
        assert_eq!(at(f(HotkeyEdge::Bottom)), [360.0, 550.0, 720.0, 350.0]);
        // at the sides the depth is a width
        assert_eq!(at(f(HotkeyEdge::Left)), [0.0, 244.0, 576.0, 438.0]);
        assert_eq!(at(f(HotkeyEdge::Right)), [864.0, 244.0, 576.0, 438.0]);
        // the size along the edge clamps like the depth
        assert_eq!(frame(SCREEN, HotkeyEdge::Top, 40, 0).w, 288.0);
        assert_eq!(frame(SCREEN, HotkeyEdge::Top, 40, 200).w, 1440.0);
    }

    #[test]
    fn slide_starts_off_screen_and_ends_on_the_frame() {
        let f = frame(SCREEN, HotkeyEdge::Top, 40, 100);
        let y = |elapsed, showing| {
            let ((x, y), done) = slide_pos(f, HotkeyEdge::Top, elapsed, showing);
            assert_eq!(x, 0.0);
            (y, done)
        };
        assert_eq!(y(Duration::ZERO, true), (25.0 - 350.0, false));
        let (mid, done) = y(SLIDE / 2, true);
        assert!(!done && mid > 25.0 - 350.0 && mid < 25.0, "{mid}");
        assert_eq!(y(SLIDE, true), (25.0, true));
        assert_eq!(y(SLIDE * 3, true), (25.0, true));
        // hiding runs the other way
        assert_eq!(y(Duration::ZERO, false), (25.0, false));
        assert_eq!(y(SLIDE, false), (25.0 - 350.0, true));
    }

    #[test]
    fn slide_comes_in_past_its_own_edge() {
        let start = |edge| {
            let f = frame(SCREEN, edge, 40, 50);
            (slide_pos(f, edge, Duration::ZERO, true).0, f)
        };
        let ((x, y), f) = start(HotkeyEdge::Bottom);
        assert_eq!((x, y), (f.x, f.y + f.h));
        let ((x, y), f) = start(HotkeyEdge::Left);
        assert_eq!((x, y), (f.x - f.w, f.y));
        let ((x, y), f) = start(HotkeyEdge::Right);
        assert_eq!((x, y), (f.x + f.w, f.y));
    }
}
