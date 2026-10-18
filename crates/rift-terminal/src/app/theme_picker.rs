//! The theme picker (`theme_picker`): the palette over every theme, the
//! selected row applied live as the selection moves (keys, typing, a
//! hover-free list), nothing saved. Enter keeps the theme and saves it;
//! anything else that closes the palette (Esc, a click outside, another
//! mode) puts the theme it opened over back. With `sync_os_theme` on and
//! the system in light mode it previews and keeps `theme_light`, the theme
//! on screen then (and follows a light/dark switch while it is open).
//!
//! The preview lives in the config in memory only: one picker is open at a
//! time across windows, a save from elsewhere writes the original theme,
//! a hot reload moves the original to what the file now says, and a window
//! closing mid-preview puts the original back.

use super::*;
use crate::views::palette::{PaletteAction, PaletteMode};

/// The picker's state while it is open.
#[derive(Debug, Clone)]
pub(crate) struct Preview {
    /// The theme in force when it opened (put back on revert).
    original: String,
    /// It picks `theme_light` (else `theme`).
    light: bool,
}

impl App {
    /// The config field the picker previews: the one on screen.
    fn picked_theme_field(&mut self, light: bool) -> &mut String {
        picked_field(&mut self.config, light)
    }

    pub(super) fn open_theme_picker(&mut self) {
        // a picker already open (here, or in another window) puts its
        // original back first: its preview is not what to revert to
        self.revert_theme_preview();
        let elsewhere: Vec<Preview> = self
            .parked
            .iter_mut()
            .filter_map(|w| {
                let p = w.theme_preview.take()?;
                if w.palette.mode == PaletteMode::Themes {
                    w.palette.open = false;
                }
                if let Some(win) = &w.window {
                    win.request_redraw();
                }
                Some(p)
            })
            .collect();
        for p in elsewhere {
            *self.picked_theme_field(p.light) = p.original;
            self.apply_config();
        }
        let light = self.config.sync_os_theme && self.os_light;
        let original = self.picked_theme_field(light).clone();
        self.open_palette(PaletteMode::Themes);
        self.win.theme_preview = Some(Preview {
            original: original.clone(),
            light,
        });
        // start on the theme in use
        let at =
            self.win.palette_results.iter().position(
                |m| matches!(&m.item.action, PaletteAction::PickTheme(n) if *n == original),
            );
        self.win.palette.selected = at.unwrap_or(0);
    }

    pub(super) fn theme_picker_items(&self) -> Vec<crate::views::palette::MatchedItem> {
        let names = rift_core::theme::available_themes();
        let labels = rift_core::theme::picker_labels(&names);
        let current = self
            .win
            .theme_preview
            .as_ref()
            .map(|p| p.original.clone())
            .unwrap_or_else(|| self.config.theme.clone());
        crate::views::palette::theme_items(&names, &labels, &current, &self.win.palette.query)
    }

    /// Enter on a row: keep it (saved), the picker closed.
    pub(super) fn keep_theme(&mut self, name: String) {
        let light = self
            .win
            .theme_preview
            .take()
            .map(|p| p.light)
            .unwrap_or(self.config.sync_os_theme && self.os_light);
        *self.picked_theme_field(light) = name;
        self.save_config();
        self.apply_config();
    }

    /// This window's preview ends (it closes mid-preview, or a picker
    /// opens again): the original theme comes back.
    pub(super) fn revert_theme_preview(&mut self) {
        if let Some(p) = self.win.theme_preview.take() {
            *self.picked_theme_field(p.light) = p.original;
            self.apply_config();
        }
    }

    /// The picker's preview in any window (one at a time).
    fn any_theme_preview(&self) -> Option<&Preview> {
        std::iter::once(&*self.win)
            .chain(self.parked.iter().map(|w| &**w))
            .find_map(|w| w.theme_preview.as_ref())
    }

    /// The config `save_config` writes: the one in force, with a preview
    /// in progress swapped back for the theme it opened over.
    pub(super) fn config_to_save(&self) -> std::borrow::Cow<'_, rift_core::config::Config> {
        match self.any_theme_preview() {
            Some(p) => {
                let mut c = self.config.clone();
                *picked_field(&mut c, p.light) = p.original.clone();
                std::borrow::Cow::Owned(c)
            }
            None => std::borrow::Cow::Borrowed(&self.config),
        }
    }

    /// config.toml was reloaded while a picker is open: its revert goes
    /// to the theme the file now names (the next frame previews again).
    pub(super) fn rebase_theme_preview(&mut self) {
        let config = &self.config;
        for w in std::iter::once(&mut *self.win).chain(self.parked.iter_mut().map(|w| &mut **w)) {
            if let Some(p) = w.theme_preview.as_mut() {
                p.original = if p.light {
                    config.theme_light.clone()
                } else {
                    config.theme.clone()
                };
            }
        }
    }

    /// Once per frame: preview the selected row while the picker is open;
    /// once it closed without a pick, put the original back.
    pub(super) fn sync_theme_preview(&mut self) {
        let Some(mut preview) = self.win.theme_preview.clone() else {
            return;
        };
        // the system switched light/dark meanwhile: preview the theme now
        // on screen instead, the other one back as it was
        let light = self.config.sync_os_theme && self.os_light;
        if light != preview.light {
            *self.picked_theme_field(preview.light) = preview.original.clone();
            preview = Preview {
                original: self.picked_theme_field(light).clone(),
                light,
            };
            self.win.theme_preview = Some(preview.clone());
            self.apply_config();
        }
        let picking = self.win.palette.open && self.win.palette.mode == PaletteMode::Themes;
        let want = if picking {
            match self
                .win
                .palette_results
                .get(self.win.palette.selected)
                .map(|m| &m.item.action)
            {
                Some(PaletteAction::PickTheme(name)) => name.clone(),
                // nothing matches the query: show the original meanwhile
                _ => preview.original.clone(),
            }
        } else {
            self.win.theme_preview = None;
            preview.original.clone()
        };
        if *self.picked_theme_field(preview.light) != want {
            *self.picked_theme_field(preview.light) = want;
            self.apply_config();
        }
    }
}

/// `config`'s theme field for the light (`theme_light`) or dark side.
fn picked_field(config: &mut rift_core::config::Config, light: bool) -> &mut String {
    if light {
        &mut config.theme_light
    } else {
        &mut config.theme
    }
}
