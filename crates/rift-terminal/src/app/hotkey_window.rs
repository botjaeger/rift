//! The drop-down hotkey window (`hotkey_window = true`, Warp's dedicated
//! hotkey window): a window kind the global hotkey (or the script's
//! `hotkey` step) creates on first use, then shows and hides. It spans the
//! width of the screen with the pointer, hangs from its top edge, floats
//! above other apps on every Space, and slides down and up. Hiding keeps
//! its shells running. It is not saved in the session file: a relaunch
//! does not bring back its tabs, and an update's restart ends its shells
//! (`end_hotkey_window_shells`) rather than leave them unreachable in the
//! session service. It does not keep the app open: closing the last
//! ordinary window quits (`App::quits`). Turning `hotkey_window` off closes
//! it unless a command runs in it (`retire_hotkey_window`).

use super::*;
use crate::dropdown::{self, Frame};

/// What a drop-down window adds to its `WindowState`.
#[derive(Debug, Default)]
pub(super) struct Dropdown {
    /// On screen (or sliding down).
    pub(super) shown: bool,
    /// It took keyboard focus since it last showed: only then does a focus
    /// loss hide it (a window being created or ordered in can report one
    /// first).
    pub(super) had_focus: bool,
    /// The screen it was last placed on and its frame there (logical px).
    pub(super) screen: Option<Frame>,
    pub(super) frame: Option<Frame>,
    /// A slide in progress: when it started, and whether it shows the
    /// window (else hides it).
    pub(super) slide: Option<(Instant, bool)>,
    /// macOS: the app that was frontmost before the window showed; the
    /// hotkey hands focus back to it.
    pub(super) prev_app: Option<i32>,
}

impl App {
    /// The global hotkey (or the script's `hotkey` step): the drop-down
    /// window with `hotkey_window`, else the whole app.
    pub(super) fn on_global_hotkey(&mut self, event_loop: &ActiveEventLoop) {
        if self.config.hotkey_window {
            self.toggle_hotkey_window(event_loop);
        } else {
            self.toggle_visibility();
        }
    }

    /// The drop-down window's id, if it exists.
    pub(super) fn dropdown_id(&self) -> Option<WindowId> {
        std::iter::once(&*self.win)
            .chain(self.parked.iter().map(|w| &**w))
            .find(|w| w.dropdown.is_some())
            .and_then(|w| w.window.as_ref().map(|x| x.id()))
    }

    /// Show the drop-down window (creating it on first use), or hide it when
    /// it is already shown and focused.
    pub(super) fn toggle_hotkey_window(&mut self, event_loop: &ActiveEventLoop) {
        let screen = self.active_screen();
        let Some(id) = self.dropdown_id() else {
            self.open_hotkey_window(event_loop, screen);
            return;
        };
        self.enter_window(id);
        let shown = self.win.dropdown.as_ref().is_some_and(|d| d.shown);
        // headless windows never take focus: the toggle alone decides
        if shown && (self.win.focused || self.headless) {
            self.hide_hotkey_window(true);
        } else {
            self.show_hotkey_window(Some(screen));
        }
    }

    /// The screen the window drops onto: the one with the pointer (macOS),
    /// else the served window's, else the primary one.
    fn active_screen(&self) -> Frame {
        #[cfg(target_os = "macos")]
        if let Some(s) = dropdown::pointer_screen() {
            return s;
        }
        self.win
            .window
            .as_ref()
            .and_then(|w| {
                w.current_monitor()
                    .or_else(|| w.primary_monitor())
                    .or_else(|| w.available_monitors().next())
            })
            .map(|m| dropdown::monitor_frame(&m))
            .unwrap_or(dropdown::FALLBACK_SCREEN)
    }

    /// Create the drop-down window (hidden, at its frame), give it a tab,
    /// then show it.
    fn open_hotkey_window(&mut self, event_loop: &ActiveEventLoop, screen: Frame) {
        let cwd = self.start_dir();
        let f = dropdown::frame(screen, self.config.hotkey_window_height());
        let old = std::mem::replace(&mut self.win, Box::new(WindowState::new(false)));
        if old.window.is_some() {
            self.parked.push(old);
        }
        self.win.dropdown = Some(Dropdown {
            screen: Some(screen),
            frame: Some(f),
            ..Default::default()
        });
        self.create_window(
            event_loop,
            Some(rift_core::session_state::SavedWindow {
                x: f.x,
                y: f.y,
                width: f.w,
                height: f.h,
                maximized: false,
                fullscreen: false,
            }),
        );
        self.apply_window_config();
        self.spawn_tab_at(cwd);
        self.show_hotkey_window(Some(screen));
    }

    /// Show the drop-down window (the one being served) on `screen` (else
    /// where it was), sliding down unless motion is reduced, and focus it.
    pub(super) fn show_hotkey_window(&mut self, screen: Option<Frame>) {
        let headless = self.headless;
        let height = self.config.hotkey_window_height();
        let Some(window) = self.win.window.clone() else {
            return;
        };
        let Some(d) = self.win.dropdown.as_mut() else {
            return;
        };
        let screen = screen.or(d.screen).unwrap_or(dropdown::FALLBACK_SCREEN);
        let f = dropdown::frame(screen, height);
        let animate = !headless && dropdown::animate();
        d.shown = true;
        d.had_focus = false;
        d.screen = Some(screen);
        d.frame = Some(f);
        d.slide = animate.then(|| (Instant::now(), true));
        let y = if animate { f.y - f.h } else { f.y };
        let _ = window.request_inner_size(LogicalSize::new(f.w, f.h));
        window.set_outer_position(winit::dpi::LogicalPosition::new(f.x, y));
        if !headless {
            #[cfg(target_os = "macos")]
            {
                d.prev_app = dropdown::frontmost_other_app();
                dropdown::float_everywhere(&window);
                dropdown::unhide_app();
            }
            window.set_visible(true);
            window.focus_window();
        }
        if let Some(id) = self.win_id() {
            self.note_window_focus(id);
        }
        window.request_redraw();
    }

    /// The window being served is the hidden drop-down window: show it, so
    /// what brings one of its panes forward (`ctl focus`, the switcher, a
    /// notification) never leaves keyboard focus in an invisible window.
    pub(super) fn reveal_hotkey_window(&mut self) {
        if self.win.dropdown.as_ref().is_some_and(|d| !d.shown) {
            self.show_hotkey_window(None);
        }
    }

    /// An exit that keeps durable shells for the next launch (an update's
    /// restart): the drop-down window is not saved, so nothing would
    /// reattach its shells; they end instead of running on unseen in the
    /// session service.
    pub(super) fn end_hotkey_window_shells(&self) {
        let dropdown = std::iter::once(&*self.win)
            .chain(self.parked.iter().map(|w| &**w))
            .filter(|w| w.dropdown.is_some());
        for p in dropdown.flat_map(|w| w.tabs.iter().flat_map(|t| &t.panes)) {
            p.session.terminate();
        }
    }

    /// `hotkey_window` was turned off: the drop-down window closes (its
    /// shells end) unless a command runs in one of them. Then it stays and
    /// drops down, so the command stays in view; it no longer hides on a
    /// focus loss and closes like any window (⇧⌘W, its last tab).
    pub(super) fn retire_hotkey_window(&mut self) {
        if self.config.hotkey_window {
            return;
        }
        let Some(id) = self.dropdown_id() else {
            return;
        };
        let back = self.win_id().filter(|b| *b != id);
        self.enter_window(id);
        let busy = self
            .win
            .tabs
            .iter()
            .flat_map(|t| &t.panes)
            .any(|p| p.session.has_foreground_job());
        if busy {
            self.reveal_hotkey_window();
        } else if self.close_window() {
            log::info!("hotkey window closed: hotkey_window is off");
        }
        if let Some(b) = back {
            self.enter_window(b);
        }
    }

    /// Hide the drop-down window (the one being served), sliding up unless
    /// motion is reduced; its shells keep running. `hand_back`: focus goes
    /// back to the app that had it before the window showed (the hotkey;
    /// not a focus loss, which already went somewhere).
    pub(super) fn hide_hotkey_window(&mut self, hand_back: bool) {
        let headless = self.headless;
        let Some(d) = self.win.dropdown.as_mut().filter(|d| d.shown) else {
            return;
        };
        let animate = !headless && dropdown::animate();
        d.shown = false;
        d.slide = animate.then(|| (Instant::now(), false));
        let prev_app = d.prev_app.take();
        if !animate
            && !headless
            && let Some(w) = &self.win.window
        {
            w.set_visible(false);
        }
        self.win.focused = false;
        // menu commands, the control socket and script steps act in the
        // ordinary window focused last
        let me = self.win_id();
        self.focused_window = self
            .focus_history
            .iter()
            .rev()
            .chain(self.window_order.iter())
            .find(|id| Some(**id) != me)
            .copied();
        #[cfg(target_os = "macos")]
        if hand_back
            && !headless
            && let Some(pid) = prev_app
        {
            dropdown::activate_app(pid);
        }
        let _ = (hand_back, prev_app);
    }

    /// The window being served took (`true`) or lost keyboard focus: a
    /// shown drop-down window that had focus hides on the loss with
    /// `hotkey_window_hide_on_blur`.
    pub(super) fn hotkey_window_focus(&mut self, focused: bool) {
        let Some(d) = self.win.dropdown.as_mut().filter(|d| d.shown) else {
            return;
        };
        if focused {
            d.had_focus = true;
        } else if d.had_focus && self.config.hotkey_window && self.config.hotkey_window_hide_on_blur
        {
            self.hide_hotkey_window(false);
        }
    }

    /// Move the window being served one step along its slide; the next
    /// step's time while it lasts.
    pub(super) fn step_slide(&mut self, now: Instant) -> Option<Instant> {
        let d = self.win.dropdown.as_mut()?;
        let (start, showing) = d.slide?;
        let f = d.frame?;
        let (y, done) = dropdown::slide_y(f, now.saturating_duration_since(start), showing);
        let window = self.win.window.as_ref()?;
        window.set_outer_position(winit::dpi::LogicalPosition::new(f.x, y));
        if !done {
            return Some(now + dropdown::SLIDE_STEP);
        }
        d.slide = None;
        if !showing {
            window.set_visible(false);
        }
        None
    }

    /// A changed `hotkey_window_height` re-places a shown window.
    pub(super) fn place_hotkey_window(&mut self) {
        let shown = self
            .win
            .dropdown
            .as_ref()
            .is_some_and(|d| d.shown && d.slide.is_none());
        let height = self.config.hotkey_window_height();
        let (Some(d), Some(window)) = (self.win.dropdown.as_mut(), self.win.window.as_ref()) else {
            return;
        };
        let Some(screen) = d.screen.filter(|_| shown) else {
            return;
        };
        let f = dropdown::frame(screen, height);
        if d.frame != Some(f) {
            d.frame = Some(f);
            let _ = window.request_inner_size(LogicalSize::new(f.w, f.h));
            window.set_outer_position(winit::dpi::LogicalPosition::new(f.x, f.y));
        }
    }

    /// The dump's description of a window's kind: `kind=normal`, or the
    /// drop-down's visibility, frame, screen and actual size.
    pub(super) fn window_kind_dump(w: &WindowState) -> String {
        let Some(d) = &w.dropdown else {
            return "kind=normal".into();
        };
        let r = |f: Option<Frame>| f.map(|f| [f.x, f.y, f.w, f.h]);
        let size = w.window.as_ref().map(|x| {
            let s = x.inner_size().to_logical::<f64>(x.scale_factor());
            [s.width.round(), s.height.round()]
        });
        format!(
            "kind=dropdown shown={} sliding={} frame={:?} screen={:?} size={:?}",
            d.shown,
            d.slide.is_some(),
            r(d.frame),
            r(d.screen),
            size
        )
    }
}
