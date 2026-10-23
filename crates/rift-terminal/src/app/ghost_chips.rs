//! The ghost text's chips (Warp): the accept-key hint, whose menu changes
//! the key that takes the ghost text (`tab_behavior`, the
//! `accept_autosuggestion` binding), and the ✕ that ignores the suggested
//! command line (`rift_core::ignored_suggestions`).

use super::*;
use crate::views::editor::AcceptKey;

impl App {
    /// The hint chip's menu is the open context menu.
    pub(super) fn accept_key_menu_open(&self) -> bool {
        self.win.context_menu.open
            && self
                .win
                .context_menu
                .items
                .iter()
                .any(|i| matches!(i.action, MenuAction::AcceptKey(_)))
    }

    /// Click on the hint chip of `pane`: the keys that can accept the ghost
    /// text, the current one checked; above the chip (below it when the
    /// command line is at the top).
    pub(super) fn open_accept_key_menu(&mut self, pane: PaneId) {
        let Some(chip) = self
            .win
            .hits
            .iter()
            .find(|h| h.target == ClickTarget::GhostKey(pane))
            .map(|h| h.rect)
        else {
            return;
        };
        let current = AcceptKey::current(&self.config);
        let items: Vec<_> = AcceptKey::ALL
            .iter()
            .map(|&k| MenuItem::new(k.label(), MenuAction::AcceptKey(k)).radio(k == current))
            .collect();
        let menu_h = items.len() as f32 * crate::views::context_menu::ROW_H + 8.0;
        let above = chip[1] - menu_h - 4.0;
        let y = if above >= 8.0 {
            above
        } else {
            chip[1] + chip[3] + 4.0
        };
        let selected = AcceptKey::ALL.iter().position(|&k| k == current);
        self.win.context_menu = ContextMenuState {
            open: true,
            pane,
            block: 0,
            pos: (chip[0], y),
            items,
            selected,
        };
    }

    /// A row of the hint chip's menu: → is the default (Tab completes, no
    /// binding); Tab gives Tab the ghost text; ⇧→ binds
    /// `accept_autosuggestion`; Custom… opens Settings › Keybindings at it.
    /// Written to config.toml (hot reload as any Settings change).
    pub(super) fn pick_accept_key(&mut self, key: AcceptKey) {
        const ACTION: &str = "accept_autosuggestion";
        let c = &mut self.config;
        match key {
            AcceptKey::Right => {
                c.tab_behavior = rift_core::TabBehavior::Completions;
                c.keybindings.remove(ACTION);
            }
            AcceptKey::Tab => {
                c.tab_behavior = rift_core::TabBehavior::Autosuggestion;
                c.keybindings.remove(ACTION);
            }
            AcceptKey::ShiftRight => {
                c.tab_behavior = rift_core::TabBehavior::Completions;
                c.keybindings.insert(ACTION.into(), "shift+right".into());
            }
            AcceptKey::Custom => {
                self.win
                    .settings
                    .open_page(crate::views::settings::Page::Keybindings);
                self.win.settings.kb_filter = ACTION.into();
                return;
            }
        }
        self.save_config();
        self.apply_config();
    }

    /// Click on the ✕ of `pane`'s ghost text: its whole suggested line is
    /// never suggested again; the next suggestion takes its place.
    pub(super) fn ignore_ghost(&mut self, pane: PaneId) {
        let Some(p) = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.pane(pane))
        else {
            return;
        };
        let ghost = &self.win.completion.ghost;
        if ghost.is_empty() || !p.editor.ghost_at_end() {
            return;
        }
        let line = format!("{}{ghost}", p.editor.buffer);
        self.ignored.add(&line);
        self.win.completion.ghost.clear();
        self.update_completions();
    }
}
