//! Launch profiles in the app (`rift_core::profiles` resolves them): what a
//! new pane runs, the + button's right-click menu and the palette rows that
//! open a profile, and the identity a pane keeps for duplicate, save-layout
//! and restore. A profile that cannot open says why in a toast and nothing
//! is spawned; only paths that cannot refuse (restore, links, the first tab
//! of a window) fall back to the global shell, and say so.

use super::*;
use crate::views::palette::ProfileOpen;
use rift_core::profiles::{LaunchError, PaneLaunch, Profile, Resolved};

impl App {
    /// The user's profiles, then the discovered shells.
    pub(super) fn profiles(&self) -> Vec<Profile> {
        rift_core::profiles::effective(&self.config.profiles, self.config.discover_shells)
    }

    pub(super) fn resolve_launch(&self, launch: &PaneLaunch) -> Result<Resolved, LaunchError> {
        let path = std::env::var_os("PATH");
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        rift_core::profiles::resolve(
            &self.profiles(),
            self.config.default_profile.as_deref(),
            launch,
            path.as_deref(),
            home.as_deref(),
        )
    }

    /// What the next spawn runs (`next_launch`, else the default launch):
    /// the resolved launch, the identity the pane keeps, the program's path,
    /// and the error when it did not resolve (the global shell then runs).
    pub(super) fn take_launch(&mut self) -> (Resolved, PaneLaunch, String, Option<LaunchError>) {
        let launch = self.next_launch.take().unwrap_or_default();
        let (resolved, identity, error) = match self.resolve_launch(&launch) {
            Ok(r) => {
                let identity = r.identity(&launch);
                (r, identity, None)
            }
            // the pane keeps what it was meant to be: a later restore tries
            // that again instead of recording the fallback
            Err(e) => (Resolved::default(), launch, Some(e)),
        };
        let program = resolved.program.clone().unwrap_or_else(|| {
            rift_core::shell_integration::resolve_shell(self.config.shell.as_deref())
        });
        (resolved, identity, program, error)
    }

    /// The program a pane runs (its own, else the global shell's).
    pub(super) fn pane_program(&self, pane: &PaneState) -> String {
        if pane.program.is_empty() {
            rift_core::shell_integration::resolve_shell(self.config.shell.as_deref())
        } else {
            pane.program.clone()
        }
    }

    /// Open `name` (`None`: the default launch) as a tab or split of the
    /// active pane. A profile that does not resolve is refused with a toast.
    pub(super) fn open_profile(&mut self, name: Option<&str>, how: ProfileOpen) {
        let launch = name.map(PaneLaunch::profile).unwrap_or_default();
        self.open_launch(launch, how);
    }

    pub(super) fn open_launch(&mut self, launch: PaneLaunch, how: ProfileOpen) {
        let resolved = match self.resolve_launch(&launch) {
            Ok(r) => r,
            Err(e) => {
                self.toast_error(e.to_string());
                return;
            }
        };
        // the profile's directory, else the global one
        let cwd = resolved
            .cwd
            .clone()
            .filter(|d| d.is_dir())
            .or_else(|| self.start_dir());
        self.next_launch = Some(launch);
        match how {
            ProfileOpen::Tab => {
                self.spawn_new_tab_at(cwd);
                self.style_active_tab(&resolved);
            }
            ProfileOpen::SplitRight => self.split_active_at(Axis::Horizontal, cwd),
            ProfileOpen::SplitDown => self.split_active_at(Axis::Vertical, cwd),
        }
    }

    /// The active tab takes `resolved`'s title and color (a profile without
    /// them leaves the tab as it is).
    pub(super) fn style_active_tab(&mut self, resolved: &Resolved) {
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        if let Some(title) = &resolved.title {
            tab.custom_title = Some(title.clone());
        }
        if let Some(c) = &resolved.color {
            tab.color = crate::state::TAB_COLORS.iter().position(|n| n == c);
        }
        self.save_session();
    }

    /// A tab of the next launch (`next_launch`, else the default profile)
    /// for paths that cannot refuse (the first tab, links, the command
    /// line, ⌘N, ctl, the phone, the drop-down window's replacement tab):
    /// in `folder` when one was named, else the profile's directory, else
    /// `fallback`; with the profile's title and color. A profile that does
    /// not resolve runs the global shell and says so (`spawn_session`).
    pub(super) fn spawn_profile_tab(
        &mut self,
        folder: Option<std::path::PathBuf>,
        fallback: Option<std::path::PathBuf>,
    ) {
        let launch = self.next_launch.clone().unwrap_or_default();
        let resolved = self.resolve_launch(&launch).unwrap_or_default();
        let cwd = folder
            .filter(|d| d.is_dir())
            .or_else(|| resolved.cwd.clone().filter(|d| d.is_dir()))
            .or(fallback);
        self.spawn_tab_at(cwd);
        self.style_active_tab(&resolved);
    }

    /// Every pane of `cfg` resolves; else the first error, and nothing of
    /// the configuration opens.
    pub(super) fn check_launch_config(
        &self,
        cfg: &rift_core::launch::LaunchConfig,
    ) -> Result<(), LaunchError> {
        fn leaves<'a>(l: &'a rift_core::launch::LaunchLayout, out: &mut Vec<&'a PaneLaunch>) {
            match l {
                rift_core::launch::LaunchLayout::Pane { launch, .. } => out.push(launch),
                rift_core::launch::LaunchLayout::Split { panes, .. } => {
                    panes.iter().for_each(|p| leaves(p, out))
                }
            }
        }
        let mut all = Vec::new();
        cfg.tabs.iter().for_each(|t| leaves(&t.layout, &mut all));
        all.into_iter()
            .try_for_each(|l| self.resolve_launch(l).map(|_| ()))
    }

    /// The + button's ⌄ (and right-click on +): one row per profile, the
    /// default one bold, then Settings, the command palette and About.
    pub(super) fn open_profile_menu(&mut self, pos: (f32, f32)) {
        let default = self
            .config
            .default_profile
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string);
        let mut items: Vec<MenuItem> = self
            .profiles()
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                let glyph = p.icon.as_deref().and_then(crate::views::glyphs::by_name);
                let bold = default.as_deref() == Some(p.name.trim());
                MenuItem::new(p.name, MenuAction::NewTabProfile(i))
                    .bold(bold)
                    .glyph(glyph)
            })
            .collect();
        let first_extra = MenuItem::new("Settings", MenuAction::OpenSettings);
        items.push(if items.is_empty() {
            first_extra
        } else {
            first_extra.sep()
        });
        items.push(MenuItem::new("Command Palette", MenuAction::OpenPalette));
        items.push(MenuItem::new("About Rift", MenuAction::OpenAbout));
        self.win.context_menu = ContextMenuState {
            open: true,
            pane: 0,
            block: 0,
            pos,
            items,
            selected: None,
        };
    }

    /// The menu row `i` (the profiles listed when it opened).
    pub(super) fn open_profile_at(&mut self, i: usize) {
        match self.profiles().get(i).map(|p| p.name.clone()) {
            Some(name) => self.open_profile(Some(&name), ProfileOpen::Tab),
            None => self.toast_error("That profile is no longer in config.toml".into()),
        }
    }
}

/// The basename of the pane's program (`fish`), else `global` (the global
/// shell's name) for a pane whose program is not known.
pub(super) fn program_name(pane: &PaneState, global: &str) -> String {
    if pane.program.is_empty() {
        global.to_string()
    } else {
        rift_core::profiles::basename(&pane.program).to_string()
    }
}

/// The launch of the first leaf of `layout` (the pane spawned for it).
pub(super) fn first_leaf_launch(layout: &rift_core::launch::LaunchLayout) -> PaneLaunch {
    match layout {
        rift_core::launch::LaunchLayout::Pane { launch, .. } => launch.clone(),
        rift_core::launch::LaunchLayout::Split { panes, .. } => {
            panes.first().map(first_leaf_launch).unwrap_or_default()
        }
    }
}
