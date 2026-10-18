//! Pinned tabs and tab groups (Warp). A pinned tab sits first, compact,
//! and ⌘W asks before closing it. A group is a named, coloured run of
//! tabs behind a header (a pill in the strip, a row in the rail) that
//! folds them away. The order rules live in [`arrange`]: pinned tabs first,
//! then every group's tabs together where its first tab is. A pinned tab
//! is in no group; the active tab is never folded away (a group holding it
//! opens); ⌘1–9, ⌃Tab and the arrows count the tabs you can see.

use super::*;
use crate::views::context_menu::{ContextMenuState, MenuAction, MenuItem};

/// A tab group of one window.
#[derive(Debug, Clone, PartialEq)]
pub struct TabGroup {
    pub id: u64,
    pub name: String,
    /// Index into `TAB_COLORS`.
    pub color: usize,
    pub collapsed: bool,
}

/// A window's tab groups, and the header being renamed.
#[derive(Debug, Default)]
pub struct TabGroups {
    pub list: Vec<TabGroup>,
    next: u64,
    /// Inline rename of a group header: (group id, edited name).
    pub rename: Option<(u64, String)>,
}

impl TabGroups {
    pub fn get(&self, id: u64) -> Option<&TabGroup> {
        self.list.iter().find(|g| g.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut TabGroup> {
        self.list.iter_mut().find(|g| g.id == id)
    }

    /// Whether `group` folds its tabs away.
    pub fn collapsed(&self, group: Option<u64>) -> bool {
        group.and_then(|g| self.get(g)).is_some_and(|g| g.collapsed)
    }

    /// A new group: "Group N" (the smallest N not taken), the next colour
    /// of the tab palette.
    fn add(&mut self) -> u64 {
        self.next += 1;
        let id = self.next;
        let names: Vec<&str> = self.list.iter().map(|g| g.name.as_str()).collect();
        let name = next_name(&names);
        let color = (self.next as usize - 1) % crate::state::TAB_COLORS.len();
        self.list.push(TabGroup {
            id,
            name,
            color,
            collapsed: false,
        });
        id
    }
}

/// "Group N" for the smallest N no name in `taken` has.
pub(crate) fn next_name(taken: &[&str]) -> String {
    (1..)
        .map(|n| format!("Group {n}"))
        .find(|n| !taken.contains(&n.as_str()))
        .unwrap_or_default()
}

/// The order tabs take, as old indices, from each tab's (pinned, group):
/// pinned tabs first in their order, then the rest in their order with
/// every group's tabs gathered where its first tab is.
pub(crate) fn arrange(meta: &[(bool, Option<u64>)]) -> Vec<usize> {
    let mut out: Vec<usize> = (0..meta.len()).filter(|&i| meta[i].0).collect();
    let mut placed = vec![false; meta.len()];
    for i in 0..meta.len() {
        if meta[i].0 || placed[i] {
            continue;
        }
        match meta[i].1 {
            Some(g) => {
                for j in i..meta.len() {
                    if !meta[j].0 && meta[j].1 == Some(g) {
                        placed[j] = true;
                        out.push(j);
                    }
                }
            }
            None => out.push(i),
        }
    }
    out
}

/// The tabs that show, in order: none folded in a collapsed group, except
/// the active one.
pub(crate) fn visible(
    meta: &[(bool, Option<u64>)],
    collapsed: impl Fn(u64) -> bool,
    active: usize,
) -> Vec<usize> {
    (0..meta.len())
        .filter(|&i| i == active || !meta[i].1.is_some_and(&collapsed))
        .collect()
}

impl App {
    fn tab_meta(&self) -> Vec<(bool, Option<u64>)> {
        self.win.tabs.iter().map(|t| (t.pinned, t.group)).collect()
    }

    /// The tabs that show in the strip and the rail, in order.
    pub(super) fn visible_tabs(&self) -> Vec<usize> {
        let groups = &self.win.groups;
        visible(
            &self.tab_meta(),
            |g| groups.collapsed(Some(g)),
            self.win.active,
        )
    }

    /// Put the tabs in [`arrange`]'s order (the active tab stays active),
    /// drop groups left without tabs and picks of tabs that went.
    pub(super) fn arrange_tabs(&mut self) {
        let order = arrange(&self.tab_meta());
        if order.iter().enumerate().any(|(i, &j)| i != j) {
            let active = order
                .iter()
                .position(|&j| j == self.win.active)
                .unwrap_or(0);
            let mut slots: Vec<Option<TabState>> = std::mem::take(&mut self.win.tabs)
                .into_iter()
                .map(Some)
                .collect();
            self.win.tabs = order.iter().filter_map(|&j| slots[j].take()).collect();
            self.win.active = active;
            self.win.rename = None;
        }
        let used: Vec<u64> = self.win.tabs.iter().filter_map(|t| t.group).collect();
        self.win.groups.list.retain(|g| used.contains(&g.id));
        if self
            .win
            .groups
            .rename
            .as_ref()
            .is_some_and(|(id, _)| !used.contains(id))
        {
            self.win.groups.rename = None;
        }
    }

    /// The active tab is never folded away: its group opens.
    pub(super) fn reveal_active_tab(&mut self) {
        let group = self.win.tabs.get(self.win.active).and_then(|t| t.group);
        if let Some(g) = group.and_then(|g| self.win.groups.get_mut(g))
            && g.collapsed
        {
            g.collapsed = false;
            self.save_session();
        }
    }

    /// Pin tab `i`, or unpin it. A pinned tab leaves its group.
    pub(super) fn toggle_pin(&mut self, i: usize) {
        let Some(t) = self.win.tabs.get_mut(i) else {
            return;
        };
        t.pinned = !t.pinned;
        if t.pinned {
            t.group = None;
        }
        self.arrange_tabs();
        self.save_session();
    }

    /// "Group tabs": tab `i` and the picked ones (⌘-click / ⇧-click) form
    /// a new group; pinned tabs stay out of it.
    pub(super) fn group_tabs(&mut self, i: usize) {
        let mut members: Vec<usize> = self
            .win
            .tabs
            .iter()
            .enumerate()
            .filter(|(j, t)| (t.picked || *j == i) && !t.pinned)
            .map(|(j, _)| j)
            .collect();
        members.dedup();
        if members.is_empty() {
            self.toast("Unpin the tab to group it".into());
            return;
        }
        let id = self.win.groups.add();
        for j in members {
            self.win.tabs[j].group = Some(id);
        }
        self.clear_picks();
        self.arrange_tabs();
        self.save_session();
    }

    /// Tab `i` leaves its group (the group goes with its last tab).
    pub(super) fn leave_group(&mut self, i: usize) {
        if let Some(t) = self.win.tabs.get_mut(i) {
            t.group = None;
        }
        self.arrange_tabs();
        self.save_session();
    }

    /// "Ungroup": the group goes, its tabs stay where they are.
    pub(super) fn ungroup(&mut self, id: u64) {
        for t in self.win.tabs.iter_mut().filter(|t| t.group == Some(id)) {
            t.group = None;
        }
        self.arrange_tabs();
        self.save_session();
    }

    /// A click on a group header: fold its tabs away, or show them. Folding
    /// the group of the active tab moves to the nearest tab outside it
    /// (after the group, else before); with none there it stays open.
    pub(super) fn toggle_group(&mut self, id: u64) {
        let Some(g) = self.win.groups.get(id) else {
            return;
        };
        if !g.collapsed && self.win.tabs.get(self.win.active).and_then(|t| t.group) == Some(id) {
            let outside = |j: &usize| {
                let t = &self.win.tabs[*j];
                t.group != Some(id) && !self.win.groups.collapsed(t.group)
            };
            let after = (self.win.active..self.win.tabs.len()).find(outside);
            let before = (0..self.win.active).rev().find(outside);
            match after.or(before) {
                Some(j) => self.switch_tab(j),
                None => return,
            }
        }
        if let Some(g) = self.win.groups.get_mut(id) {
            g.collapsed = !g.collapsed;
        }
        self.save_session();
    }

    /// The tab indices of group `id`.
    pub(super) fn group_tabs_of(&self, id: u64) -> Vec<usize> {
        (0..self.win.tabs.len())
            .filter(|&j| self.win.tabs[j].group == Some(id))
            .collect()
    }

    pub(super) fn start_group_rename(&mut self, id: u64) {
        if let Some(g) = self.win.groups.get(id) {
            self.win.groups.rename = Some((id, g.name.clone()));
            self.win.rename_fresh = true;
        }
    }

    /// Keys while a group header is renamed: Enter keeps the name (an empty
    /// one is not taken), Esc drops the edit.
    pub(super) fn handle_group_rename_key(&mut self, event: &input::KeyPress) {
        use winit::keyboard::{Key, NamedKey};
        if event.released {
            return;
        }
        let fresh = std::mem::replace(&mut self.win.rename_fresh, false);
        let Some((id, text)) = self.win.groups.rename.as_mut() else {
            return;
        };
        let id = *id;
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => {
                let name = text.trim().to_string();
                self.win.groups.rename = None;
                if let Some(g) = self.win.groups.get_mut(id)
                    && !name.is_empty()
                {
                    g.name = name;
                }
                self.save_session();
            }
            Key::Named(NamedKey::Escape) => self.win.groups.rename = None,
            Key::Named(NamedKey::Backspace) => {
                if fresh {
                    text.clear();
                } else {
                    crate::state::pop_grapheme(text);
                }
            }
            _ => {
                if let Some(t) = event
                    .text
                    .as_deref()
                    .filter(|t| !t.chars().any(char::is_control))
                    && !self.win.modifiers.super_key()
                    && !self.win.modifiers.control_key()
                {
                    if fresh {
                        text.clear();
                    }
                    if text.chars().count() < 40 {
                        text.push_str(t);
                    }
                } else {
                    // a shortcut keeps the rename's freshness
                    self.win.rename_fresh = fresh;
                }
            }
        }
    }

    /// The close asks first: it would close a pinned tab.
    pub(super) fn closes_pinned_tab(&self, scope: CloseScope) -> bool {
        match scope {
            CloseScope::Tab(i) => self.win.tabs.get(i).is_some_and(|t| t.pinned),
            CloseScope::Pane(id) => self
                .win
                .tabs
                .iter()
                .find(|t| t.pane(id).is_some())
                .is_some_and(|t| t.pinned && t.panes.len() == 1),
            _ => false,
        }
    }

    fn clear_picks(&mut self) {
        for t in &mut self.win.tabs {
            t.picked = false;
        }
    }

    /// ⌘-click (`range` false) adds tab `i` to the picked tabs or takes it
    /// out; ⇧-click picks every shown tab from the active one to `i`. The
    /// first ⌘-click picks the active tab too.
    pub(super) fn pick_tab(&mut self, i: usize, range: bool) {
        if i >= self.win.tabs.len() {
            return;
        }
        if range {
            let shown = self.visible_tabs();
            let (Some(a), Some(b)) = (
                shown.iter().position(|&j| j == self.win.active),
                shown.iter().position(|&j| j == i),
            ) else {
                return;
            };
            self.clear_picks();
            for &j in &shown[a.min(b)..=a.max(b)] {
                self.win.tabs[j].picked = true;
            }
            return;
        }
        if !self.win.tabs.iter().any(|t| t.picked) {
            let active = self.win.active;
            if let Some(t) = self.win.tabs.get_mut(active) {
                t.picked = true;
            }
        }
        self.win.tabs[i].picked = !self.win.tabs[i].picked;
    }

    /// A plain click on a tab drops the picks.
    pub(super) fn drop_picks(&mut self) {
        self.clear_picks();
    }

    /// Whether tabs `i` and `j` may swap (drag, ⌃⇧← / ⌃⇧→): the same
    /// zone, pinned or not, and the same group.
    pub(super) fn same_zone(&self, i: usize, j: usize) -> bool {
        match (self.win.tabs.get(i), self.win.tabs.get(j)) {
            (Some(a), Some(b)) => a.pinned == b.pinned && a.group == b.group,
            _ => false,
        }
    }

    /// The `n`th shown tab for ⌘1–⌘8, the last shown for ⌘9.
    pub(super) fn nth_shown_tab(&self, n: u8) -> Option<usize> {
        let shown = self.visible_tabs();
        if n == 9 {
            shown.last().copied()
        } else {
            shown.get(n as usize - 1).copied()
        }
    }

    /// The shown tab `step` places from the active one, wrapping.
    pub(super) fn shown_tab_from_active(&self, step: isize) -> Option<usize> {
        let shown = self.visible_tabs();
        let at = shown.iter().position(|&j| j == self.win.active)? as isize;
        let n = shown.len() as isize;
        shown.get((at + step).rem_euclid(n) as usize).copied()
    }

    /// A new tab opened from tab `from` joins its group.
    pub(super) fn join_group_of(&mut self, from: usize, new: usize) {
        let group = self.win.tabs.get(from).and_then(|t| t.group);
        if let (Some(g), Some(t)) = (group, self.win.tabs.get_mut(new)) {
            t.group = Some(g);
            self.arrange_tabs();
        }
    }

    /// The tab menu's pin and group rows for tab `i`.
    pub(super) fn tab_group_items(&self, i: usize) -> Vec<MenuItem> {
        let Some(t) = self.win.tabs.get(i) else {
            return Vec::new();
        };
        let mut items = vec![
            MenuItem::new(
                if t.pinned { "Unpin tab" } else { "Pin tab" },
                MenuAction::PinTab(i),
            )
            .sep(),
        ];
        let picked = self.win.tabs.iter().filter(|t| t.picked).count();
        if !t.pinned {
            items.push(MenuItem::new(
                if picked > 1 {
                    "Group picked tabs"
                } else {
                    "Group tab"
                },
                MenuAction::GroupTabs(i),
            ));
        }
        if let Some(g) = t.group {
            items.push(MenuItem::new(
                "Remove from group",
                MenuAction::LeaveGroup(i),
            ));
            items.extend(self.group_items(g));
        }
        items
    }

    /// A group header's menu rows.
    fn group_items(&self, id: u64) -> Vec<MenuItem> {
        let collapsed = self.win.groups.collapsed(Some(id));
        vec![
            MenuItem::new(
                if collapsed {
                    "Expand group"
                } else {
                    "Collapse group"
                },
                MenuAction::ToggleGroup(id),
            )
            .sep(),
            MenuItem::new("Rename group", MenuAction::RenameGroup(id)),
            MenuItem::new("Ungroup", MenuAction::Ungroup(id)),
            MenuItem::new("Close group", MenuAction::CloseGroup(id)),
        ]
    }

    /// Right-click on a group header.
    pub(super) fn open_group_menu(&mut self, id: u64, pos: (f32, f32)) {
        let mut items = self.group_items(id);
        if let Some(first) = items.first_mut() {
            first.separator = false;
        }
        self.win.context_menu = ContextMenuState {
            open: true,
            pane: 0,
            block: 0,
            pos,
            items,
            selected: None,
        };
    }

    /// The window's groups as the session file stores them, and each tab's
    /// group as an index into them.
    pub(super) fn saved_groups(
        win: &WindowState,
    ) -> (
        Vec<rift_core::session_state::SavedGroup>,
        Vec<Option<usize>>,
    ) {
        let groups = win
            .groups
            .list
            .iter()
            .map(|g| rift_core::session_state::SavedGroup {
                name: g.name.clone(),
                color: crate::state::TAB_COLORS.get(g.color).map(|s| s.to_string()),
                collapsed: g.collapsed,
            })
            .collect();
        let of_tab = win
            .tabs
            .iter()
            .map(|t| {
                t.group
                    .and_then(|id| win.groups.list.iter().position(|g| g.id == id))
            })
            .collect();
        (groups, of_tab)
    }

    /// After a window's `saved` tabs were restored (the last of its tabs):
    /// their pins and groups.
    pub(super) fn restore_tab_groups(
        &mut self,
        groups: &[rift_core::session_state::SavedGroup],
        saved: &[rift_core::session_state::SavedTab],
    ) {
        let ids: Vec<u64> = groups
            .iter()
            .map(|g| {
                let id = self.win.groups.add();
                if let Some(new) = self.win.groups.get_mut(id) {
                    if !g.name.trim().is_empty() {
                        new.name = g.name.clone();
                    }
                    if let Some(c) = g
                        .color
                        .as_deref()
                        .and_then(|c| crate::state::TAB_COLORS.iter().position(|n| *n == c))
                    {
                        new.color = c;
                    }
                    new.collapsed = g.collapsed;
                }
                id
            })
            .collect();
        let first = self.win.tabs.len().saturating_sub(saved.len());
        for (k, s) in saved.iter().enumerate() {
            if let Some(t) = self.win.tabs.get_mut(first + k) {
                t.pinned = s.pinned;
                t.group = s
                    .group
                    .and_then(|g| ids.get(g).copied())
                    .filter(|_| !s.pinned);
            }
        }
        self.arrange_tabs();
        self.reveal_active_tab();
    }

    /// What the strip and the rail draw for tab `i` beyond its title.
    pub(super) fn tab_marks(&self, i: usize, theme: &rift_core::Theme) -> tabbar::TabMarks {
        let Some(t) = self.win.tabs.get(i) else {
            return Default::default();
        };
        let group = t.group.and_then(|g| self.win.groups.get(g));
        let first_of_group = group.is_some() && (i == 0 || self.win.tabs[i - 1].group != t.group);
        tabbar::TabMarks {
            pinned: t.pinned,
            header: group
                .filter(|_| first_of_group)
                .map(|g| tabbar::GroupHeader {
                    id: g.id,
                    name: g.name.clone(),
                    color: context_menu::tab_color(theme, g.color),
                    collapsed: g.collapsed,
                    count: self.group_tabs_of(g.id).len(),
                    editing: self
                        .win
                        .groups
                        .rename
                        .as_ref()
                        .filter(|(id, _)| *id == g.id)
                        .map(|(_, s)| s.clone()),
                }),
            group_color: group.map(|g| context_menu::tab_color(theme, g.color)),
            hidden: group.is_some_and(|g| g.collapsed) && i != self.win.active,
            picked: t.picked,
            read_only: t.panes.iter().any(|p| p.session.read_only()),
            diff: t.active().and_then(|p| Self::pane_stats(p).0),
            pr: t.active().and_then(|p| Self::pane_stats(p).1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrange_puts_pinned_first_and_gathers_groups() {
        // u, g1, p, u, g1, g2, p, g2
        let meta = [
            (false, None),
            (false, Some(1)),
            (true, None),
            (false, None),
            (false, Some(1)),
            (false, Some(2)),
            (true, None),
            (false, Some(2)),
        ];
        assert_eq!(arrange(&meta), vec![2, 6, 0, 1, 4, 3, 5, 7]);
        // already in order: unchanged
        let sorted = [
            (true, None),
            (false, Some(3)),
            (false, Some(3)),
            (false, None),
        ];
        assert_eq!(arrange(&sorted), vec![0, 1, 2, 3]);
        assert!(arrange(&[]).is_empty());
    }

    #[test]
    fn visible_folds_collapsed_groups_but_keeps_the_active_tab() {
        let meta = [
            (true, None),
            (false, Some(1)),
            (false, Some(1)),
            (false, None),
        ];
        let collapsed = |g: u64| g == 1;
        assert_eq!(visible(&meta, collapsed, 0), vec![0, 3]);
        assert_eq!(visible(&meta, collapsed, 2), vec![0, 2, 3]);
        assert_eq!(visible(&meta, |_| false, 0), vec![0, 1, 2, 3]);
    }

    #[test]
    fn group_names_count_up_past_taken_ones() {
        assert_eq!(next_name(&[]), "Group 1");
        assert_eq!(next_name(&["Group 1", "Group 3"]), "Group 2");
        assert_eq!(next_name(&["Servers"]), "Group 1");
        let mut g = TabGroups::default();
        let a = g.add();
        let b = g.add();
        assert_ne!(a, b);
        assert_eq!(g.get(b).unwrap().name, "Group 2");
        assert_ne!(g.get(a).unwrap().color, g.get(b).unwrap().color);
    }
}
