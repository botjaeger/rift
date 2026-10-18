//! A pane's own colour scheme: `theme` in its profile or launch, or "Pane
//! theme: NAME" in the palette (saved in the session through the pane's
//! launch). Its terminal content (ANSI palette, background, foreground,
//! cursor, selection, block rules) renders in that theme while the window's
//! chrome keeps the window theme. A pane theme is fixed: OS theme sync does
//! not change it.

use super::*;
use rift_core::profiles::{DEFAULT_PANE_THEME, PaneLaunch};

/// `name` loaded for a pane, with the window opacity applied.
pub(super) fn load(name: &str, opacity: f32) -> Result<(String, Box<Theme>), String> {
    let mut theme = rift_core::theme::try_load_by_name(name)
        .map_err(|e| format!("Pane theme {name:?}: {e}"))?;
    theme.ground_alpha = opacity;
    Ok((name.to_string(), Box::new(theme)))
}

/// The pane's colour queries (OSC 10/11/12, OSC 4) answer with its own
/// theme, else the window's.
pub(super) fn sync_query_colors(pane: &PaneState) {
    rift_pty::set_pane_colors(
        pane.session.id,
        pane.theme.as_ref().map(|(_, t)| terminal::theme_rgbs(t)),
    );
}

impl App {
    /// The colour scheme `launch` names (its own `theme`, else its
    /// profile's), loaded; a theme that does not load is logged and the
    /// pane keeps the window theme.
    pub(super) fn launch_theme(&self, launch: &PaneLaunch) -> Option<(String, Box<Theme>)> {
        let name = self.resolve_launch(launch).ok()?.theme?;
        load(&name, self.config.opacity())
            .inspect_err(|e| log::warn!("{e}; using the window theme"))
            .ok()
    }

    /// "Pane theme: NAME" (`Some`) or "Pane theme: default" (`None`, the
    /// window theme, also over the profile's) on the active pane.
    pub(super) fn set_active_pane_theme(&mut self, name: Option<String>) {
        let Some(launch) = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .map(|p| p.launch.clone())
        else {
            return;
        };
        let loaded = match name.as_deref().map(|n| load(n, self.config.opacity())) {
            Some(Err(e)) => {
                self.toast_error(e);
                return;
            }
            Some(Ok(t)) => Some(t),
            None => None,
        };
        // "default" is recorded only where the profile names a theme
        let profile_theme = self
            .resolve_launch(&PaneLaunch {
                theme: None,
                ..launch
            })
            .ok()
            .and_then(|r| r.theme);
        let own = match &name {
            Some(n) => Some(n.clone()),
            None => profile_theme.map(|_| DEFAULT_PANE_THEME.to_string()),
        };
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        else {
            return;
        };
        pane.launch.theme = own;
        pane.theme = loaded;
        sync_query_colors(pane);
        // the phone's screen of this pane takes the new colours
        pane.session
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.save_session();
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Load every pane's theme again (`apply_config`): the window opacity,
    /// an edited theme file. A theme that no longer loads stays as it was.
    pub(super) fn reload_pane_themes(&mut self) {
        let opacity = self.config.opacity();
        let tabs = self
            .win
            .tabs
            .iter_mut()
            .chain(self.parked.iter_mut().flat_map(|w| w.tabs.iter_mut()));
        for pane in tabs.flat_map(|t| t.panes.iter_mut()) {
            let Some((name, _)) = &pane.theme else {
                continue;
            };
            match load(name, opacity) {
                Ok(t) => pane.theme = Some(t),
                Err(e) => log::warn!("{e}; keeping the last one loaded"),
            }
            sync_query_colors(pane);
        }
    }
}
