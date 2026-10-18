//! The vertical rail's search and panes view (`views::rail` draws them):
//! the rows it shows for the window's tabs, the search field's keys, and
//! its clicks. The rows read what the panes already know (title, cwd, git
//! chip data, PR chip, blocks, agent): no git work happens here.

use super::*;
use crate::views::rail::{PaneRow, RailRow};

/// The rail's search field and view.
#[derive(Debug, Default)]
pub struct RailUi {
    pub query: String,
    /// The field has the keyboard.
    pub focused: bool,
    /// One row per pane of a split tab under its tab row.
    pub panes: bool,
    /// The rows drawn last frame (the accessibility tree reads them).
    pub rows: Vec<RailRow>,
}

/// A row as `dump` reports it: `pinned`, `group:<name>:<open|collapsed>`,
/// `tab:<i>[:+a-d][:#pr]`, `pane:<id>[:active][:+a-d][:#pr]`, `nomatch`.
pub(super) fn describe(row: &RailRow) -> String {
    let stats = |diff: Option<(usize, usize)>, pr: Option<u64>| {
        let mut s = String::new();
        if let Some((a, d)) = diff {
            s.push_str(&format!(":+{a}-{d}"));
        }
        if let Some(n) = pr {
            s.push_str(&format!(":#{n}"));
        }
        s
    };
    match row {
        RailRow::Section(label) => label.to_lowercase(),
        RailRow::Group(g) => format!(
            "group:{}:{}",
            g.name,
            if g.collapsed { "collapsed" } else { "open" }
        ),
        RailRow::Tab(i) => format!("tab:{i}"),
        RailRow::Pane(p) => format!(
            "pane:{}{}{}",
            p.id,
            if p.active { ":active" } else { "" },
            stats(p.diff, p.pr)
        ),
        RailRow::NoMatch => "nomatch".into(),
    }
}

/// Whether `query` (lowercase) is in any of `fields`, case-insensitively.
pub(crate) fn matches(query: &str, fields: &[&str]) -> bool {
    query.is_empty() || fields.iter().any(|f| f.to_lowercase().contains(query))
}

impl App {
    /// What a search matches a pane by: its title, directory, branch, the
    /// command it runs or last ran, and its coding agent; masked while
    /// `mask_secrets` is on (a search must not find what the rail hides).
    fn pane_fields(p: &PaneState) -> Vec<String> {
        let mut out = vec![p.title.clone()];
        if let Some(cwd) = p.session.cwd.lock().unwrap().clone() {
            out.push(cwd);
        }
        if let Some(g) = &p.git {
            out.push(g.branch.clone());
            if let Some(wt) = &g.worktree {
                out.push(wt.name.clone());
            }
        }
        let command = {
            let blocks = p.session.blocks.lock().unwrap();
            blocks
                .iter()
                .rev()
                .find(|b| !b.command.trim().is_empty())
                .map(|b| b.command.clone())
        };
        out.extend(command);
        if let Some(a) = p
            .agent_status
            .map(|s| s.agent)
            .or_else(|| Self::pane_agent(p))
        {
            out.push(a.name().to_string());
        }
        out.into_iter()
            .map(rift_core::secrets::mask_owned)
            .collect()
    }

    /// Tab `i` matches `query` (lowercase): its title or one of its panes.
    fn tab_matches(&self, i: usize, query: &str) -> bool {
        let t = &self.win.tabs[i];
        matches(query, &[&rift_core::secrets::mask(&t.display_title())])
            || t.panes.iter().any(|p| Self::pane_matches(p, query))
    }

    fn pane_matches(p: &PaneState, query: &str) -> bool {
        let fields = Self::pane_fields(p);
        let refs: Vec<&str> = fields.iter().map(String::as_str).collect();
        matches(query, &refs)
    }

    /// A pane row's title: its program's title (masked while
    /// `mask_secrets` is on), else its directory.
    fn pane_row_title(p: &PaneState) -> String {
        if !p.title.is_empty() {
            return rift_core::secrets::mask_owned(p.title.clone());
        }
        p.session
            .cwd
            .lock()
            .unwrap()
            .as_deref()
            .map(editor::shorten_home)
            .unwrap_or_else(|| "shell".into())
    }

    /// A pane's repository stats for its row (uncommitted lines, PR).
    pub(super) fn pane_stats(p: &PaneState) -> (Option<(usize, usize)>, Option<u64>) {
        (
            p.git
                .as_ref()
                .map(|g| (g.added, g.removed))
                .filter(|(a, d)| a + d > 0),
            p.pr.as_ref().map(|pr| pr.number),
        )
    }

    /// The rail's rows for `tabs` (this window's, in order; the Settings tab
    /// last when it shows): the pinned section, group headers, tab rows and,
    /// in the panes view, pane rows of split tabs. A search shows the
    /// matching tabs (and panes) whatever their groups' folding, or a
    /// "no match" row.
    pub(super) fn rail_rows(&self, tabs: &[tabbar::TabInfo]) -> Vec<RailRow> {
        let query = self.win.rail.query.trim().to_lowercase();
        let searching = !query.is_empty();
        let panes_view = self.win.rail.panes;
        let active_pane = self.active_pane_id();
        let mut rows = Vec::new();
        let n = self.win.tabs.len();
        let tab_matches = |i: usize| self.tab_matches(i, &query);
        let shown: Vec<usize> = (0..n)
            .filter(|&i| {
                if searching {
                    tab_matches(i)
                } else {
                    !tabs.get(i).is_some_and(|t| t.marks.hidden)
                }
            })
            .collect();
        let pinned = shown.iter().filter(|&&i| self.win.tabs[i].pinned).count();
        if pinned > 0 {
            rows.push(RailRow::Section("PINNED"));
        }
        let mut last_header = None;
        let mut tabs_section = false;
        for i in 0..n {
            let t = &self.win.tabs[i];
            let is_shown = shown.contains(&i);
            // a folded group's tabs hide behind its header; a search shows
            // the headers of groups with a match only
            let header = t
                .group
                .filter(|g| last_header != Some(*g) && (is_shown || !searching));
            if !is_shown && header.is_none() {
                continue;
            }
            if pinned > 0 && !t.pinned && !tabs_section {
                tabs_section = true;
                rows.push(RailRow::Section("TABS"));
            }
            if let Some(g) = header {
                last_header = Some(g);
                let first = (0..n).find(|&j| self.win.tabs[j].group == Some(g));
                if let Some(mut h) = first.and_then(|j| tabs.get(j)?.marks.header.clone()) {
                    if searching {
                        h.collapsed = false;
                    }
                    rows.push(RailRow::Group(h));
                }
            }
            if !is_shown {
                continue;
            }
            rows.push(RailRow::Tab(i));
            if panes_view && t.panes.len() > 1 {
                for leaf in t.root.leaves() {
                    let Some(p) = t.pane(leaf) else {
                        continue;
                    };
                    if searching && !Self::pane_matches(p, &query) {
                        continue;
                    }
                    let (diff, pr) = Self::pane_stats(p);
                    rows.push(RailRow::Pane(PaneRow {
                        id: leaf,
                        title: Self::pane_row_title(p),
                        active: i == self.win.active && Some(leaf) == active_pane,
                        running: p.running(),
                        read_only: p.session.read_only(),
                        diff,
                        pr,
                    }));
                }
            }
        }
        // the Settings tab (after the shells) is a page, not searched
        if !searching && tabs.len() > n {
            rows.push(RailRow::Tab(n));
        }
        if searching && !rows.iter().any(|r| matches!(r, RailRow::Tab(_))) {
            rows.push(RailRow::NoMatch);
        }
        rows
    }

    /// Give the rail's search field the keyboard (⇧⌘F, a click). Without
    /// the rail, the session switcher searches the tabs instead.
    pub(super) fn focus_rail_search(&mut self, event_loop: &ActiveEventLoop) {
        if self.rail_w() > 0.0 {
            self.win.rail.focused = true;
        } else {
            self.handle_action(AppAction::SessionSwitcher, event_loop);
        }
    }

    /// Keys while the search field has the keyboard: typing filters, Esc
    /// clears it and lets go, Enter opens the first tab that matches.
    pub(super) fn handle_rail_search_key(&mut self, event: &input::KeyPress) {
        use winit::keyboard::{Key, NamedKey};
        if event.released {
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.win.rail.query.clear();
                self.win.rail.focused = false;
            }
            // Enter opens the first match; in an empty field it only lets go
            Key::Named(NamedKey::Enter) => {
                let query = self.win.rail.query.trim().to_lowercase();
                let first = (0..self.win.tabs.len())
                    .find(|&i| !query.is_empty() && self.tab_matches(i, &query));
                self.win.rail.focused = false;
                if let Some(i) = first {
                    self.switch_tab(i);
                }
            }
            Key::Named(NamedKey::Backspace) => {
                crate::state::pop_grapheme(&mut self.win.rail.query);
            }
            _ => {
                let mods = self.win.modifiers;
                if let Some(t) = event
                    .text
                    .as_deref()
                    .filter(|t| !t.chars().any(char::is_control))
                    && !mods.super_key()
                    && !mods.control_key()
                    && self.win.rail.query.chars().count() < 80
                {
                    self.win.rail.query.push_str(t);
                }
            }
        }
    }

    /// Tab `i` is left out by the rail's search (last frame's rows).
    pub(super) fn rail_filters_out(&self, i: usize) -> bool {
        self.rail_w() > 0.0
            && !self.win.rail.query.trim().is_empty()
            && !self
                .win
                .rail
                .rows
                .iter()
                .any(|r| matches!(r, RailRow::Tab(j) if *j == i))
    }

    /// The rail for screen readers: its search field, the panes-view
    /// toggle and the pane rows (title, state, diff stats, PR).
    pub(super) fn rail_a11y(&self) -> Option<crate::a11y::RailNode> {
        use accesskit::Role;
        if self.rail_w() <= 0.0 {
            return None;
        }
        let hit = |want: &ClickTarget| {
            self.win
                .hits
                .iter()
                .find(|h| h.target == *want)
                .map(|h| h.rect)
        };
        let mut controls = vec![crate::a11y::Control {
            label: "Show panes under their tabs".into(),
            role: Role::Button,
            toggled: Some(self.win.rail.panes),
            selected: false,
            rect: hit(&ClickTarget::RailPanesToggle),
        }];
        for row in &self.win.rail.rows {
            let RailRow::Pane(p) = row else { continue };
            let mut label = format!("{}, pane", p.title);
            if p.active {
                label.push_str(", active");
            }
            if p.running {
                label.push_str(", running");
            }
            if p.read_only {
                label.push_str(", read-only");
            }
            if let Some((a, d)) = p.diff {
                label.push_str(&format!(", {a} lines added, {d} removed"));
            }
            if let Some(n) = p.pr {
                label.push_str(&format!(", pull request {n}"));
            }
            controls.push(crate::a11y::Control {
                label,
                role: Role::ListItem,
                toggled: None,
                selected: p.active,
                rect: hit(&ClickTarget::RailPane(p.id)),
            });
        }
        Some(crate::a11y::RailNode {
            query: self.win.rail.query.clone(),
            focused: self.win.rail.focused,
            search_rect: hit(&ClickTarget::RailSearch),
            controls,
        })
    }

    /// A pane row of the panes view: its tab, and that pane focused.
    pub(super) fn focus_rail_pane(&mut self, pane: PaneId) {
        let Some(i) = self.win.tabs.iter().position(|t| t.pane(pane).is_some()) else {
            return;
        };
        if i != self.win.active {
            self.switch_tab(i);
        }
        let tab = &mut self.win.tabs[i];
        if tab.zoomed.is_some_and(|z| z != pane) {
            tab.zoomed = None;
        }
        tab.active_pane = pane;
        self.win.viewer_focus = false;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_search_matches_any_field_ignoring_case() {
        use super::matches;
        assert!(matches("", &[]));
        assert!(matches("main", &["~/src", "Main"]));
        assert!(matches("claude", &["Claude Code"]));
        assert!(!matches("npm", &["cargo test", "~/src"]));
    }
}
