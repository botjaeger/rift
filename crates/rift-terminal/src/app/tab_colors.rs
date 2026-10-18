//! Tab colours beyond the six named ones: any `#rrggbb` (the prompt of
//! `set_tab_color` and the tab menu's "Custom color…"), `cycle_tab_color`,
//! and `[[tab_color_rules]]`, which colour a tab by its directory while the
//! tab has no colour of its own.

use super::*;
use rift_core::tab_color::{self, TabColor};

impl App {
    /// The colour tab `t` shows: its own, else the deepest directory rule
    /// matching its active pane's (local) directory.
    pub(super) fn shown_tab_color(&self, t: &TabState) -> Option<TabColor> {
        t.color.or_else(|| {
            if self.tab_rules.is_empty() {
                return None;
            }
            let cwd = t.active()?.session.local_cwd()?;
            // resolved at config load: `~` expanded, real paths added
            tab_color::for_dir(&self.tab_rules, &cwd, None)
        })
    }

    /// `cycle_tab_color`: the active tab's own colour moves on (none, red …
    /// cyan, none).
    pub(super) fn cycle_tab_color(&mut self) {
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        tab.color = TabColor::cycle(tab.color);
        self.save_session();
    }

    /// Ask for tab `i`'s colour (the palette's text input).
    pub(super) fn open_tab_color_prompt(&mut self, i: usize) {
        let Some(pane) = self.win.tabs.get(i).map(|t| t.active_pane) else {
            return;
        };
        self.win.tab_color_target = Some(pane);
        self.open_palette(crate::views::palette::PaletteMode::TabColor);
        // a hex colour already set starts the query, selected
        if let Some(c @ TabColor::Hex(_)) = self.win.tabs.get(i).and_then(|t| t.color) {
            self.win.palette.query = c.to_config();
            self.win.palette.query_selected = true;
            self.refresh_palette();
        }
    }

    /// A row of the prompt: the tab it was opened on takes the colour.
    pub(super) fn set_prompted_tab_color(&mut self, color: Option<TabColor>) {
        let Some(pane) = self.win.tab_color_target.take() else {
            return;
        };
        let Some(tab) = self.win.tabs.iter_mut().find(|t| t.pane(pane).is_some()) else {
            return;
        };
        tab.color = color;
        self.save_session();
    }
}
