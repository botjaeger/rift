//! OSC 9;4 progress across the app's windows: whether a frame must follow
//! for an indeterminate sweep, and the Dock tile badge (see
//! `crate::progress`).

use super::*;

impl App {
    /// A tab of the window being served shows an indeterminate progress bar
    /// (it sweeps every frame).
    pub(super) fn progress_sweeps(&self) -> bool {
        self.win.tabs.iter().any(|t| {
            crate::progress::of_panes(&t.panes, true)
                .is_some_and(|p| p.state == rift_core::ProgressState::Indeterminate)
        })
    }

    /// After a frame of the focused window (or the only one, before any
    /// took focus): badge the Dock tile with its active tab's progress;
    /// cleared when it has none or `show_progress` is off.
    pub(super) fn update_dock_badge(&mut self) {
        if self
            .focused_window
            .is_some_and(|id| self.win_id() != Some(id))
        {
            return;
        }
        let label = self.win.tabs.get(self.win.active).and_then(|t| {
            crate::progress::badge(crate::progress::of_panes(
                &t.panes,
                self.config.show_progress,
            ))
        });
        if label != self.dock_badge {
            crate::progress::set_dock_badge(label.as_deref());
            self.dock_badge = label;
        }
    }
}
