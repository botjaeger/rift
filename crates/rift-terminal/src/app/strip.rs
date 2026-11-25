//! When the tab strip shows (`tab_bar`, Warp "Show the tab bar"): always,
//! only outside full screen, or only on hover. A hidden strip gives its
//! height to the panes; the pointer at the window's top edge slides it in
//! over them until the pointer leaves it. The vertical tab layout keeps its
//! strip (the rail hangs under it).

use super::*;
use rift_core::TabBarVisibility;

/// How close to the top edge the pointer reveals a hidden strip.
const REVEAL_EDGE: f32 = 4.0;

/// Whether the strip takes its height in this layout, mode and window state.
fn shown(layout: rift_core::TabLayout, mode: TabBarVisibility, fullscreen: bool) -> bool {
    layout == rift_core::TabLayout::Vertical
        || match mode {
            TabBarVisibility::Always => true,
            TabBarVisibility::HideFullscreen => !fullscreen,
            TabBarVisibility::OnHover => false,
        }
}

impl App {
    /// The strip takes its height from the window (else it is hidden or
    /// slides in over the panes).
    pub(super) fn strip_shown(&self) -> bool {
        let fullscreen = self
            .win
            .window
            .as_ref()
            .is_some_and(|w| w.fullscreen().is_some());
        shown(self.config.tab_layout, self.config.tab_bar, fullscreen)
    }

    /// Windows: the window buttons the strip draws, while it is the title
    /// bar (`crate::win_frame`). `None` elsewhere, in the drop-down window
    /// (no frame) and while the strip is hidden (Windows' caption is back).
    pub(super) fn caption_buttons(&self) -> Option<crate::win_frame::Caption> {
        #[cfg(windows)]
        if self.win.dropdown.is_none()
            && self.strip_shown()
            && let Some(window) = &self.win.window
            && window.fullscreen().is_none()
        {
            return Some(crate::win_frame::caption(window));
        }
        None
    }

    /// Windows: after a frame is laid out, tell the window frame which
    /// parts of the strip are Rift's (`self.win.hits`) and the rest the
    /// caption, and give the frame the theme's colours.
    #[cfg(windows)]
    pub(super) fn sync_caption(&self, w: f32, scale: f32) {
        use crate::win_frame::{Regions, button_rects, to_physical};
        let Some(window) = &self.win.window else {
            return;
        };
        if self.win.dropdown.is_some() {
            return;
        }
        // full screen has no window frame: no buttons, no resize band
        let regions = if self.strip_shown() && window.fullscreen().is_none() {
            let bar = tabbar::bar_h();
            Regions {
                strip_h: (bar * scale).round() as i32,
                buttons: button_rects(w, bar).map(|r| to_physical(r, scale)),
                controls: self
                    .win
                    .hits
                    .iter()
                    .filter(|h| h.rect[1] < bar && !matches!(h.target, ClickTarget::WindowDrag))
                    .map(|h| to_physical(h.rect, scale))
                    .collect(),
            }
        } else {
            Regions::default()
        };
        crate::win_frame::set_regions(window, regions);
        let c = self.theme.bg_chrome;
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        crate::win_frame::set_theme(
            window,
            !self.theme.is_light(),
            [byte(c.r), byte(c.g), byte(c.b)],
        );
    }

    /// Height the strip takes from the top of the window.
    pub(super) fn bar_h(&self) -> f32 {
        if self.strip_shown() {
            tabbar::bar_h()
        } else {
            0.0
        }
    }

    /// The hidden strip is slid in over the panes.
    pub(super) fn strip_overlay(&self) -> bool {
        self.win.bar_revealed && !self.strip_shown()
    }

    /// The strip is on screen (taking its height or slid in): the pointer
    /// in its band is over the strip.
    pub(super) fn strip_visible(&self) -> bool {
        self.strip_shown() || self.win.bar_revealed
    }

    /// Follow the pointer (`None`: it left the window): the top edge
    /// reveals a hidden strip, leaving it hides it again (an open menu
    /// keeps it). True when that changed.
    pub(super) fn track_strip(&mut self, pointer: Option<(f32, f32)>) -> bool {
        let revealed = if self.strip_shown() {
            false
        } else {
            match pointer {
                Some((_, y)) if y <= REVEAL_EDGE => true,
                Some((_, y)) if self.win.bar_revealed && y < tabbar::bar_h() => true,
                // a menu opened from it, or a tab dragged from it, keeps it
                _ => {
                    self.win.bar_revealed
                        && (self.win.context_menu.open || self.win.tab_drag.is_some())
                }
            }
        };
        if revealed == self.win.bar_revealed {
            return false;
        }
        self.win.bar_revealed = revealed;
        self.sync_window_buttons();
        true
    }

    /// macOS: the native window buttons show with the strip (full screen
    /// hides them itself) and sit centred on it.
    pub(super) fn sync_window_buttons(&self) {
        #[cfg(target_os = "macos")]
        if let Some(w) = &self.win.window
            && self.win.dropdown.is_none()
        {
            crate::traffic_lights::set_hidden(w, !self.strip_visible());
        }
    }
}

impl App {
    /// Height of the strip's band while it is on screen (taking its height
    /// or slid in), else 0: the pointer above it is over the strip.
    pub(super) fn strip_h(&self) -> f32 {
        if self.strip_visible() {
            tabbar::bar_h()
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rift_core::TabLayout::{Horizontal, Vertical};

    #[test]
    fn the_strip_hides_per_mode() {
        use TabBarVisibility::*;
        for full in [false, true] {
            assert!(shown(Horizontal, Always, full));
            assert!(!shown(Horizontal, OnHover, full));
            // the rail hangs under the strip: it stays
            assert!(shown(Vertical, OnHover, full));
        }
        assert!(shown(Horizontal, HideFullscreen, false));
        assert!(!shown(Horizontal, HideFullscreen, true));
    }
}
