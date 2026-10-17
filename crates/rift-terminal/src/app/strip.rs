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

    /// Height the strip takes from the top of the window.
    pub(super) fn bar_h(&self) -> f32 {
        if self.strip_shown() { TAB_BAR_H } else { 0.0 }
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
                Some((_, y)) if self.win.bar_revealed && y < TAB_BAR_H => true,
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
        if self.strip_visible() { TAB_BAR_H } else { 0.0 }
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
