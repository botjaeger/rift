//! ⌃Tab in most-recently-used order (`tab_switch_order = "mru"`, Windows
//! Terminal's tab switcher). Each tab carries when it was last the active
//! one (`TabState::last_used`), so moving, closing or dragging tabs keeps
//! the order. ⌃Tab with ⌃ held opens the switcher on the previous tab;
//! more ⌃Tab / ⌃⇧Tab (or ↑ ↓) move the highlight, releasing ⌃ (or ⏎)
//! switches, Esc cancels. A quick ⌃Tab tap so toggles between the two most
//! recent tabs. ⇧⌘] / ⇧⌘[ and `in_order` keep the strip's order. It lists
//! the tabs you can see (none folded in a collapsed group), and holds them
//! by their panes, so a tab that closes while it shows drops out of it.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// Stamps for `TabState::last_used` (every window shares the counter).
static CLOCK: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TabSwitcher {
    /// The tabs, last used first, each by its panes' ids (indices shift
    /// when a tab closes or moves).
    order: Vec<Vec<PaneId>>,
    /// Index into `order`.
    highlight: usize,
}

/// `live`: per entry of the switcher's order, the tab it still names. The
/// tabs left, and the highlight among them (an entry gone: the next one
/// left, else the last).
fn resolve(live: &[Option<usize>], highlight: usize) -> (Vec<usize>, usize) {
    let mut tabs = Vec::new();
    let mut at = None;
    for (k, tab) in live.iter().enumerate() {
        let Some(i) = *tab else { continue };
        if tabs.contains(&i) {
            continue;
        }
        if k >= highlight && at.is_none() {
            at = Some(tabs.len());
        }
        tabs.push(i);
    }
    let at = at.unwrap_or(tabs.len().saturating_sub(1));
    (tabs, at)
}

impl App {
    /// Stamp the active tab when it is not the most recently used one.
    pub(super) fn touch_tab_mru(&mut self) {
        let newest = self.win.tabs.iter().map(|t| t.last_used).max().unwrap_or(0);
        if let Some(tab) = self.win.tabs.get_mut(self.win.active)
            && (tab.last_used == 0 || tab.last_used < newest)
        {
            tab.last_used = CLOCK.fetch_add(1, Ordering::Relaxed) + 1;
        }
    }

    /// The window's shown tabs, last used first (never-used tabs keep
    /// the strip's order after the rest).
    fn tab_mru_order(&self) -> Vec<usize> {
        let mut order = self.visible_tabs();
        order.sort_by_key(|&i| std::cmp::Reverse(self.win.tabs[i].last_used));
        order
    }

    /// A tab's identity for the switcher: its panes.
    fn tab_panes(&self, i: usize) -> Vec<PaneId> {
        self.win.tabs[i]
            .panes
            .iter()
            .map(|p| p.session.id)
            .collect()
    }

    /// The switcher's tabs (indices now) and its highlight among them.
    fn switcher_tabs(&self) -> Option<(Vec<usize>, usize)> {
        let s = self.win.tab_switcher.as_ref()?;
        let live: Vec<Option<usize>> = s
            .order
            .iter()
            .map(|panes| {
                self.win
                    .tabs
                    .iter()
                    .position(|t| panes.iter().any(|&id| t.pane(id).is_some()))
            })
            .collect();
        Some(resolve(&live, s.highlight))
    }

    /// Move the switcher's highlight one tab on (or back) among the tabs
    /// still open.
    fn tab_switch_step(&mut self, forward: bool) {
        let Some((tabs, at)) = self.switcher_tabs() else {
            return;
        };
        let len = tabs.len().max(1);
        let at = (at + if forward { 1 } else { len - 1 }) % len;
        let order = (0..tabs.len()).map(|k| self.tab_panes(tabs[k])).collect();
        self.win.tab_switcher = Some(TabSwitcher {
            order,
            highlight: at,
        });
    }

    /// ⌃Tab / ⌃⇧Tab under `mru`: open the switcher or move its highlight.
    /// False: the strip's order applies (`in_order`, or a chord without ⌃
    /// such as ⇧⌘]).
    pub(super) fn tab_switch_mru(&mut self, forward: bool) -> bool {
        if self.config.tab_switch_order != rift_core::TabSwitchOrder::Mru
            || !self.win.modifiers.control_key()
        {
            return false;
        }
        self.touch_tab_mru();
        if self.win.tab_switcher.is_some() {
            self.tab_switch_step(forward);
            return true;
        }
        let order = self.tab_mru_order();
        let n = order.len();
        if n < 2 {
            return true;
        }
        let highlight = if forward { 1 } else { n - 1 };
        let order = order.into_iter().map(|i| self.tab_panes(i)).collect();
        self.win.tab_switcher = Some(TabSwitcher { order, highlight });
        true
    }

    /// ⌃ was released: switch to the highlighted tab.
    pub(super) fn tab_switcher_modifiers(&mut self) {
        if self.win.tab_switcher.is_some() && !self.win.modifiers.control_key() {
            self.commit_tab_switcher();
        }
    }

    fn commit_tab_switcher(&mut self) {
        let Some((tabs, at)) = self.switcher_tabs() else {
            return;
        };
        self.win.tab_switcher = None;
        if let Some(&i) = tabs.get(at)
            && i != self.win.active
        {
            self.switch_tab(i);
        }
        self.touch_tab_mru();
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Keys while the switcher shows: Esc cancels, ⏎ switches, ↑ ↓ move;
    /// ⌃Tab itself reaches `tab_switch_mru` as its action. True: used.
    pub(super) fn tab_switcher_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        if self.win.tab_switcher.is_none() || event.released {
            return false;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.win.tab_switcher = None,
            Key::Named(NamedKey::Enter) => self.commit_tab_switcher(),
            Key::Named(NamedKey::ArrowDown) => {
                self.tab_switch_step(true);
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.tab_switch_step(false);
            }
            Key::Named(NamedKey::Control | NamedKey::Shift | NamedKey::Alt | NamedKey::Super) => {}
            _ => {
                let action = input::app_action(event, self.win.modifiers, &self.keymap);
                return !matches!(action, Some(AppAction::TabNext | AppAction::TabPrev));
            }
        }
        true
    }

    /// The switcher's rows (title, directory) and highlight, for drawing.
    pub(super) fn tab_switcher_view(
        &self,
    ) -> Option<(Vec<crate::views::tab_switcher::Row>, usize)> {
        let (tabs, highlight) = self.switcher_tabs()?;
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let rows = tabs
            .iter()
            .filter_map(|&i| self.win.tabs.get(i))
            .map(|t| crate::views::tab_switcher::Row {
                title: t.display_title(),
                cwd: t
                    .active()
                    .and_then(|p| p.session.cwd.lock().unwrap().clone())
                    .map(|c| {
                        rift_core::config::tilde_path(std::path::Path::new(&c), home.as_deref())
                    })
                    .unwrap_or_default(),
            })
            .collect();
        Some((rows, highlight))
    }

    /// The switcher as an accessible list dialog.
    pub(super) fn tab_switcher_a11y(&self) -> Option<crate::a11y::Overlay> {
        let (rows, highlight) = self.tab_switcher_view()?;
        let controls = rows
            .iter()
            .enumerate()
            .map(|(i, r)| crate::a11y::Control {
                label: if r.cwd.is_empty() {
                    r.title.clone()
                } else {
                    format!("{}, {}", r.title, r.cwd)
                },
                role: accesskit::Role::ListBoxOption,
                toggled: None,
                selected: i == highlight,
                rect: self
                    .win
                    .tab_switcher_rows
                    .iter()
                    .find(|(j, _)| *j == i)
                    .map(|(_, r)| *r),
                disabled: false,
            })
            .collect();
        Some(crate::a11y::Overlay::Dialog {
            label: "Recent tabs".into(),
            input: None,
            status: Some(format!(
                "{} of {}, release Control to switch",
                highlight + 1,
                rows.len()
            )),
            controls,
            rect: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn a_tab_closing_under_the_switcher_drops_out() {
        // three tabs at 2, 0, 1; the highlight on the second entry
        assert_eq!(resolve(&[Some(2), Some(0), Some(1)], 1), (vec![2, 0, 1], 1));
        // the highlighted tab closed (the others shifted): the next one
        assert_eq!(resolve(&[Some(1), None, Some(0)], 1), (vec![1, 0], 1));
        // the last entry highlighted and gone: the last one left
        assert_eq!(resolve(&[Some(1), Some(0), None], 2), (vec![1, 0], 1));
        // a pane moved so two entries name one tab: it shows once
        assert_eq!(resolve(&[Some(0), Some(0), Some(1)], 2), (vec![0, 1], 1));
        assert_eq!(resolve(&[None], 0), (vec![], 0));
    }
}
