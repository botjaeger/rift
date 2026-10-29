//! The vertical tab rail (`tab_layout = "vertical"`): a search field over
//! the rows (title, directory, branch, command, agent), a pinned section,
//! group header rows, one row per tab (or also one per pane of a split
//! tab, the "panes" view), each with its repository's `+added −removed`
//! and pull request. The app builds the rows (`app::rail`); this draws
//! them and reports their targets.

use rift_core::{Color, TabCloseButton, Theme};
use rift_pty::PaneId;
use rift_renderer::glyphon::Weight;
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::state::{ClickTarget, Hit};
use crate::tab_status::{Status, TabStatus};
use crate::views::glyphs;
use crate::views::icons::{self, Icon};
use crate::views::tabbar::{
    GroupHeader, RAIL_W, TabInfo, bar_h, draw_progress, elide, elide_start, make_text, status_mark,
};

/// One row of the rail.
#[derive(Debug, Clone)]
pub enum RailRow {
    /// A section label ("Pinned").
    Section(&'static str),
    Group(GroupHeader),
    /// Tab `.0` (index into the tabs).
    Tab(usize),
    Pane(PaneRow),
    /// The search matched nothing.
    NoMatch,
}

/// A pane of a split tab in the panes view.
#[derive(Debug, Clone)]
pub struct PaneRow {
    pub id: PaneId,
    pub title: String,
    /// The pane holds the focus of the active tab.
    pub active: bool,
    pub running: bool,
    pub read_only: bool,
    /// Its repository's uncommitted `(added, removed)` lines.
    pub diff: Option<(usize, usize)>,
    /// Its branch's pull request number.
    pub pr: Option<u64>,
}

/// The search field's state.
#[derive(Debug, Clone, Copy)]
pub struct Search<'a> {
    pub query: &'a str,
    pub focused: bool,
    /// The panes view is on (its toggle lights).
    pub panes: bool,
}

const ROW_H: f32 = 54.0;
const PIN_ROW_H: f32 = 32.0;
const GROUP_ROW_H: f32 = 28.0;
const PANE_ROW_H: f32 = 28.0;
const SECTION_H: f32 = 24.0;
const NO_MATCH_H: f32 = 44.0;
/// Height of a "+N above" / "+N more" line.
const MORE_H: f32 = 28.0;
const SEARCH_H: f32 = 26.0;

fn row_h(row: &RailRow, tabs: &[TabInfo]) -> f32 {
    match row {
        RailRow::Section(_) => SECTION_H,
        RailRow::Group(_) => GROUP_ROW_H,
        RailRow::Tab(i) if tabs.get(*i).is_some_and(|t| t.marks.pinned) => PIN_ROW_H,
        RailRow::Tab(_) => ROW_H,
        RailRow::Pane(_) => PANE_ROW_H,
        RailRow::NoMatch => NO_MATCH_H,
    }
}

/// The rows that fit `avail` px: all of them, else a run that holds row
/// `active`, from the top while it fits there, else ending at it; room is
/// kept for the "+N above" / "+N more" lines of `more_h` each.
pub fn window(heights: &[f32], active: usize, avail: f32, more_h: f32) -> std::ops::Range<usize> {
    let n = heights.len();
    if heights.iter().sum::<f32>() <= avail || n == 0 {
        return 0..n;
    }
    let active = active.min(n - 1);
    // from the top, room for "+N more"
    let mut used = 0.0;
    let mut end = 0;
    while end < n && used + heights[end] <= avail - more_h {
        used += heights[end];
        end += 1;
    }
    if active < end {
        return 0..end.max(1);
    }
    // ending at the active row, room for both lines
    let mut start = active + 1;
    let mut used = 0.0;
    while start > 0 && used + heights[start - 1] <= avail - 2.0 * more_h {
        used += heights[start - 1];
        start -= 1;
    }
    start.min(active)..active + 1
}

/// The marker an overflow line carries: the most urgent status among the
/// `hidden` tabs (as their own, so it reads as a dot, not a ring).
fn hidden_marker(hidden: &[&TabInfo]) -> Option<Status> {
    hidden
        .iter()
        .filter_map(|t| t.status)
        .map(|s| s.status)
        .max()
}

fn inside(r: [f32; 4], p: Option<(f32, f32)>) -> bool {
    p.is_some_and(|(x, y)| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3])
}

/// `+a −d` and `#n` right-aligned so they end at `right` on the line at
/// `y`; returns where they start (`right` when there is nothing).
fn draw_stats(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    diff: Option<(usize, usize)>,
    pr: Option<u64>,
    right: f32,
    y: f32,
) -> f32 {
    let mut parts: Vec<(String, Color)> = Vec::new();
    if let Some((a, d)) = diff.filter(|(a, d)| a + d > 0) {
        parts.push((format!("+{a}"), theme.success));
        parts.push((format!("−{d}"), theme.error));
    }
    if let Some(n) = pr {
        parts.push((format!("#{n}"), theme.accent));
    }
    let mut x = right;
    for (text, color) in parts.iter().rev() {
        let w = engine.measure(text, 11.0, mono_family(), Weight::MEDIUM);
        x -= w;
        let b = make_text(engine, text, 11.0, mono_family(), Weight::MEDIUM);
        layer.text(b, x, y, *color);
        x -= 5.0;
    }
    if parts.is_empty() { right } else { x }
}

/// Draw the rail under the chrome strip and report its targets.
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    tabs: &[TabInfo],
    rows: &[RailRow],
    active: usize,
    search: Search,
    h: f32,
    blink_on: bool,
    close_button: TabCloseButton,
    mouse: Option<(f32, f32)>,
    hits: &mut Vec<Hit>,
) {
    layer.rect(
        [0.0, bar_h()],
        [RAIL_W, h - bar_h()],
        theme.ground(theme.bg_chrome),
        0.0,
    );
    layer.rect(
        [RAIL_W - 1.0, bar_h()],
        [1.0, h - bar_h()],
        theme.border,
        0.0,
    );

    // the search field, and the panes-view toggle beside it
    let toggle = [RAIL_W - 8.0 - SEARCH_H, bar_h() + 8.0, SEARCH_H, SEARCH_H];
    let field = [8.0, bar_h() + 8.0, toggle[0] - 4.0 - 8.0, SEARCH_H];
    layer.bordered(
        [field[0], field[1]],
        [field[2], field[3]],
        theme.ground(theme.bg),
        if search.focused {
            theme.accent
        } else {
            theme.border
        },
        4.0,
        1.0,
    );
    icons::draw(
        layer,
        Icon::Search,
        [field[0] + 13.0, field[1] + SEARCH_H / 2.0],
        12.0,
        theme.text_dim,
    );
    let tx = field[0] + 24.0;
    let room = field[0] + field[2] - 6.0 - tx;
    if search.query.is_empty() {
        let b = make_text(engine, "Search tabs", 12.0, FONT_SANS, Weight::NORMAL);
        layer.text(b, tx, field[1] + 5.0, theme.text_muted);
        if search.focused && blink_on {
            layer.rect([tx - 1.0, field[1] + 6.0], [1.5, 14.0], theme.accent, 0.0);
        }
    } else {
        let shown = elide(engine, search.query, 12.0, room);
        let b = make_text(engine, &shown, 12.0, FONT_SANS, Weight::NORMAL);
        layer.text(b, tx, field[1] + 5.0, theme.text_bright);
        if search.focused && blink_on {
            let w = engine.measure(&shown, 12.0, FONT_SANS, Weight::NORMAL);
            layer.rect(
                [tx + w + 1.0, field[1] + 6.0],
                [1.5, 14.0],
                theme.accent,
                0.0,
            );
        }
    }
    hits.push(Hit {
        rect: field,
        target: ClickTarget::RailSearch,
    });
    if search.panes {
        layer.rect(
            [toggle[0], toggle[1]],
            [toggle[2], toggle[3]],
            theme.accent.with_alpha(0.14),
            4.0,
        );
    } else if inside(toggle, mouse) {
        layer.rect(
            [toggle[0], toggle[1]],
            [toggle[2], toggle[3]],
            theme.text_ui.with_alpha(0.08),
            4.0,
        );
    }
    icons::draw(
        layer,
        Icon::SplitDown,
        [toggle[0] + SEARCH_H / 2.0, toggle[1] + SEARCH_H / 2.0],
        14.0,
        if search.panes {
            theme.accent
        } else {
            theme.text_dim
        },
    );
    hits.push(Hit {
        rect: toggle,
        target: ClickTarget::RailPanesToggle,
    });

    let top = bar_h() + 8.0 + SEARCH_H + 6.0;
    let avail = (h - 36.0 - top).max(0.0);
    let heights: Vec<f32> = rows.iter().map(|r| row_h(r, tabs)).collect();
    let active_row = rows
        .iter()
        .position(|r| matches!(r, RailRow::Tab(i) if *i == active))
        .unwrap_or(0);
    let win = window(&heights, active_row, avail, MORE_H);
    let tabs_in = |range: &[RailRow]| -> Vec<usize> {
        range
            .iter()
            .filter_map(|r| match r {
                RailRow::Tab(i) => Some(*i),
                _ => None,
            })
            .collect()
    };
    let mut y = top;
    let above = tabs_in(&rows[..win.start]);
    if win.start > 0 && !above.is_empty() {
        let hidden: Vec<&TabInfo> = above.iter().filter_map(|i| tabs.get(*i)).collect();
        overflow(
            layer,
            engine,
            theme,
            &format!("+{} above", above.len()),
            &hidden,
            y,
            *above.last().unwrap_or(&0),
            hits,
        );
        y += MORE_H;
    }
    for (k, row) in rows.iter().enumerate().take(win.end).skip(win.start) {
        let rh = heights[k];
        match row {
            RailRow::Section(label) => {
                let b = make_text(engine, label, 10.5, FONT_SANS, Weight::MEDIUM);
                layer.text(b, 14.0, y + 6.0, theme.text_muted);
            }
            RailRow::NoMatch => {
                let b = make_text(engine, "No tabs match", 12.0, FONT_SANS, Weight::NORMAL);
                layer.text(b, 14.0, y + 12.0, theme.text_muted);
            }
            RailRow::Group(g) => group_row(layer, engine, theme, g, y, blink_on, mouse, hits),
            RailRow::Pane(p) => pane_row(layer, engine, theme, p, y, mouse, hits),
            RailRow::Tab(i) => {
                if let Some(tab) = tabs.get(*i) {
                    tab_row(
                        layer,
                        engine,
                        theme,
                        tab,
                        *i,
                        *i == active,
                        [0.0, y, RAIL_W - 1.0, rh],
                        blink_on,
                        close_button,
                        mouse.is_some_and(|(mx, my)| {
                            (0.0..RAIL_W - 1.0).contains(&mx) && (y..y + rh).contains(&my)
                        }),
                        hits,
                    );
                }
            }
        }
        y += rh;
    }
    let below = tabs_in(&rows[win.end..]);
    if !below.is_empty() {
        let hidden: Vec<&TabInfo> = below.iter().filter_map(|i| tabs.get(*i)).collect();
        overflow(
            layer,
            engine,
            theme,
            &format!("+{} more · palette lists all", below.len()),
            &hidden,
            y,
            below[0],
            hits,
        );
    }
}

/// A group's header row: a colour bar, its name and tab count, and a
/// chevron that says whether it is open.
#[allow(clippy::too_many_arguments)]
fn group_row(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    g: &GroupHeader,
    y: f32,
    blink_on: bool,
    mouse: Option<(f32, f32)>,
    hits: &mut Vec<Hit>,
) {
    let row = [0.0, y, RAIL_W - 1.0, GROUP_ROW_H];
    if inside(row, mouse) {
        layer.rect(
            [row[0], row[1]],
            [row[2], row[3]],
            theme.text_ui.with_alpha(0.05),
            0.0,
        );
    }
    let mid = y + GROUP_ROW_H / 2.0;
    super::chevron(
        layer,
        [14.0, mid],
        3.5,
        if g.collapsed {
            super::Dir::Right
        } else {
            super::Dir::Down
        },
        1.5,
        theme.text_dim,
    );
    layer.rect([24.0, mid - 6.0], [3.0, 12.0], g.color, 1.5);
    // up to the count; a name being typed shows its end, where the caret is
    let room = row[2] - 12.0 - 30.0 - 34.0;
    let label = match &g.editing {
        Some(e) => elide_start(engine, e, 12.0, room - 12.0),
        None => elide(engine, &g.name, 12.0, room),
    };
    let b = make_text(engine, &label, 12.0, FONT_SANS, Weight::MEDIUM);
    layer.text(b, 34.0, mid - 8.0, theme.text_bright);
    if g.editing.is_some() {
        let w = engine.measure(&label, 12.0, FONT_SANS, Weight::MEDIUM);
        layer.bordered(
            [30.0, mid - 10.0],
            [(w + 12.0).max(60.0), 20.0],
            Color::TRANSPARENT,
            theme.accent,
            4.0,
            1.0,
        );
        if blink_on {
            layer.rect([34.0 + w + 1.0, mid - 7.0], [1.5, 14.0], theme.accent, 0.0);
        }
    }
    let count = g.count.to_string();
    let cw = engine.measure(&count, 11.0, FONT_SANS, Weight::MEDIUM);
    let b = make_text(engine, &count, 11.0, FONT_SANS, Weight::MEDIUM);
    layer.text(b, row[2] - 12.0 - cw, mid - 7.0, theme.text_muted);
    hits.push(Hit {
        rect: row,
        target: ClickTarget::GroupHeader(g.id),
    });
}

/// A pane row of the panes view, indented under its tab.
fn pane_row(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    p: &PaneRow,
    y: f32,
    mouse: Option<(f32, f32)>,
    hits: &mut Vec<Hit>,
) {
    let row = [0.0, y, RAIL_W - 1.0, PANE_ROW_H];
    if p.active {
        layer.rect([28.0, y + 4.0], [2.0, PANE_ROW_H - 8.0], theme.accent, 1.0);
    } else if inside(row, mouse) {
        layer.rect(
            [row[0], row[1]],
            [row[2], row[3]],
            theme.text_ui.with_alpha(0.05),
            0.0,
        );
    }
    let mid = y + PANE_ROW_H / 2.0;
    let mut cx = 36.0;
    let ink = if p.active {
        theme.text_bright
    } else {
        theme.text_dim
    };
    if p.running {
        glyphs::draw_spinner(layer, [cx + 5.0, mid], 10.0, ink);
        cx += 16.0;
    } else if p.read_only {
        icons::draw(layer, Icon::Lock, [cx + 5.0, mid], 11.0, ink);
        cx += 16.0;
    }
    let stats_x = draw_stats(layer, engine, theme, p.diff, p.pr, row[2] - 10.0, mid - 7.0);
    let title = elide(engine, &p.title, 11.5, stats_x - 6.0 - cx);
    let b = make_text(engine, &title, 11.5, FONT_SANS, Weight::NORMAL);
    layer.text(b, cx, mid - 7.5, ink);
    hits.push(Hit {
        rect: row,
        target: ClickTarget::RailPane(p.id),
    });
}

/// One tab's row: its mark (alert, activity or colour dot; agent, spinner,
/// sync or lock), title, the directory and branch line with the pane count
/// and the repository's stats, and ×. A pinned tab's row is one line.
#[allow(clippy::too_many_arguments)]
fn tab_row(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    tab: &TabInfo,
    i: usize,
    is_active: bool,
    row: [f32; 4],
    blink_on: bool,
    close_button: TabCloseButton,
    hovered: bool,
    hits: &mut Vec<Hit>,
) {
    let compact = tab.marks.pinned;
    if is_active {
        // square row on the pane ground, edged in the accent
        layer.rect(
            [row[0], row[1]],
            [row[2], row[3]],
            theme.ground(theme.bg),
            0.0,
        );
        layer.rect(
            [row[0], row[1]],
            [2.0, row[3]],
            tab.color.or(tab.marks.group_color).unwrap_or(theme.accent),
            0.0,
        );
    } else if tab.marks.picked {
        layer.rect(
            [row[0], row[1]],
            [row[2], row[3]],
            theme.accent.with_alpha(0.14),
            0.0,
        );
    }
    // a grouped tab sits indented under its header, on its colour
    let indent = if tab.marks.group_color.is_some() {
        10.0
    } else {
        0.0
    };
    if let Some(c) = tab.marks.group_color.filter(|_| !is_active) {
        layer.rect([row[0] + 6.0, row[1] + 6.0], [2.0, row[3] - 12.0], c, 1.0);
    }
    let mut cx = row[0] + 14.0 + indent;
    let line1 = if compact {
        row[1] + (PIN_ROW_H - 17.0) / 2.0
    } else {
        row[1] + 9.0
    };
    // its status (the dot style's mark), else its colour
    if let Some(st) = tab.status {
        status_mark(layer, theme, st, tab.pulse, [cx + 7.0, line1 + 8.5]);
        cx += 14.0 + 8.0;
    } else if let Some(c) = tab.color {
        layer.circle([cx + 3.5, line1 + 8.0], 8.0, c);
        cx += 14.0;
    }
    let close_x = row[0] + row[2] - 20.0;
    let ink = if is_active {
        theme.text_ui
    } else {
        theme.text_dim
    };
    let icon = if tab.settings {
        Some((Icon::Settings, ink))
    } else if tab.sync {
        Some((Icon::Sync, theme.accent))
    } else if tab.marks.read_only {
        Some((Icon::Lock, ink))
    } else if compact {
        Some((Icon::Bookmark, ink))
    } else {
        None
    };
    if let Some((icon, icon_color)) = icon {
        icons::draw(layer, icon, [cx + 7.0, line1 + 8.5], 14.0, icon_color);
        cx += 14.0 + 8.0;
    }

    let title_color = if is_active {
        theme.text_bright
    } else {
        theme.text_ui
    };
    if let Some(edit) = &tab.editing {
        let fw = (close_x - 4.0 - cx).max(40.0);
        layer.bordered(
            [cx - 4.0, line1 - 2.0],
            [fw, 20.0],
            theme.bg,
            theme.accent,
            4.0,
            1.0,
        );
        // a name wider than the field shows its end, where the caret is
        let edit = &elide_start(engine, edit, 12.5, fw - 12.0);
        let tw = engine.measure(edit, 12.5, FONT_SANS, Weight::NORMAL);
        if tab.editing_selected && !edit.is_empty() {
            layer.rect([cx - 1.0, line1], [tw + 2.0, 16.0], theme.selection, 2.0);
        }
        if !edit.is_empty() {
            let b = make_text(engine, edit, 12.5, FONT_SANS, Weight::NORMAL);
            layer.text(b, cx, line1, theme.text_bright);
        }
        if blink_on {
            layer.rect([cx + tw + 1.0, line1 + 1.0], [1.5, 15.0], theme.accent, 0.0);
        }
    } else {
        let title = elide(engine, &tab.title, 12.5, close_x - 8.0 - cx);
        let weight = if is_active {
            Weight::MEDIUM
        } else {
            Weight::NORMAL
        };
        let b = make_text(engine, &title, 12.5, FONT_SANS, weight);
        layer.text(b, cx, line1, title_color);
    }

    if !compact {
        // second line: directory · branch, pane count, sync; the stats right
        let mut sub = tab.subtitle.clone();
        if let Some(p) = (tab.panes > 1).then(|| format!("{} panes", tab.panes)) {
            sub = if sub.is_empty() {
                p
            } else {
                format!("{sub} · {p}")
            };
        }
        if tab.sync {
            sub.push_str(" · synced");
        }
        let line2 = line1 + 20.0;
        let stats_x = draw_stats(
            layer,
            engine,
            theme,
            tab.marks.diff,
            tab.marks.pr,
            row[0] + row[2] - 12.0,
            line2,
        );
        let sx = row[0] + 14.0 + indent;
        let sub = elide_start(engine, &sub, 11.0, stats_x - 6.0 - sx);
        if !sub.is_empty() {
            let b = make_text(engine, &sub, 11.0, FONT_SANS, Weight::NORMAL);
            layer.text(b, sx, line2, theme.text_muted);
        }
    }

    // the × as on the strip (`tab_close_button`): the hovered row's, every
    // row's with `always` (a pinned one asks first); a held ⌘'s number
    // takes its place
    let shows_close = super::tabbar::shows_close(close_button, tab, hovered);
    if let Some(h) = &tab.hint {
        let w = engine.measure(h, 11.0, FONT_SANS, Weight::MEDIUM) + 8.0;
        let right = row[0] + row[2] - 8.0;
        layer.rect(
            [right - w, line1],
            [w, 16.0],
            theme.text_ui.with_alpha(0.09),
            3.0,
        );
        let b = make_text(engine, h, 11.0, FONT_SANS, Weight::MEDIUM);
        layer.text(b, right - w + 4.0, line1 + 0.8, ink);
    } else if shows_close {
        super::close_icon(
            layer,
            [close_x + 5.0, line1 + 7.0],
            3.5,
            theme
                .text_muted
                .with_alpha(if is_active { 1.0 } else { 0.7 }),
        );
        hits.push(Hit {
            rect: [close_x - 4.0, line1 - 3.0, 18.0, 20.0],
            target: if tab.settings {
                ClickTarget::SettingsClose
            } else {
                ClickTarget::TabClose(i)
            },
        });
    }
    if let Some(p) = tab.progress {
        draw_progress(layer, theme, row[0], row[1] + row[3], row[2], p);
    }
    hits.push(Hit {
        rect: row,
        target: ClickTarget::TabSelect(i),
    });
}

/// One overflow line: the hidden tabs' marker (same badge as a row), the
/// label, and a hit that selects the nearest hidden tab so the window
/// slides toward it.
#[allow(clippy::too_many_arguments)]
fn overflow(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    text: &str,
    hidden: &[&TabInfo],
    y: f32,
    nearest: usize,
    hits: &mut Vec<Hit>,
) {
    let row = [6.0, y, RAIL_W - 12.0, MORE_H - 4.0];
    let (mx, cy) = (row[0] + 17.5, y + 12.0);
    let mut tx = 16.0;
    if let Some(status) = hidden_marker(hidden) {
        let st = TabStatus {
            status,
            own: true,
            pane: 0,
        };
        status_mark(layer, theme, st, 1.0, [mx, cy]);
        tx += 20.0;
    }
    let b = make_text(engine, text, 11.0, FONT_SANS, Weight::NORMAL);
    layer.text(b, tx, y + 5.0, theme.text_muted);
    hits.push(Hit {
        rect: row,
        target: ClickTarget::TabSelect(nearest),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(n: usize, active: usize, fits: usize) -> std::ops::Range<usize> {
        // `fits` rows of 10 px fit, with room for the overflow lines (4 px)
        window(&vec![10.0; n], active, fits as f32 * 10.0 + 8.0, 4.0)
    }

    #[test]
    fn the_window_keeps_the_active_row_visible() {
        assert_eq!(uniform(8, 0, 4), 0..4);
        assert_eq!(uniform(8, 3, 4), 0..4);
        let w = uniform(8, 7, 4);
        assert!(w.contains(&7) && w.end == 8, "{w:?}");
        for active in 0..8 {
            let w = uniform(8, active, 3);
            assert!(w.contains(&active), "active {active} in {w:?}");
        }
    }

    #[test]
    fn the_window_shows_everything_that_fits() {
        assert_eq!(window(&[10.0, 10.0, 10.0], 2, 30.0, 4.0), 0..3);
        assert_eq!(window(&[], 0, 30.0, 4.0), 0..0);
        // no room at all still shows the active row
        assert_eq!(window(&[10.0; 5], 3, 0.0, 4.0), 3..4);
        // a stale active index past the end clamps
        assert!(window(&[10.0; 5], 9, 24.0, 4.0).contains(&4));
        // rows of different heights
        let w = window(&[54.0, 28.0, 54.0, 54.0, 54.0], 4, 120.0, 28.0);
        assert!(w.contains(&4), "{w:?}");
    }

    fn tab(status: Option<Status>) -> TabInfo {
        let mut t = TabInfo::plain("");
        t.status = status.map(|status| TabStatus {
            status,
            own: false,
            pane: 1,
        });
        t
    }

    #[test]
    fn overflow_marker_is_the_most_urgent_hidden_status() {
        let (a, b, c) = (
            tab(Some(Status::Done)),
            tab(Some(Status::NeedsYou)),
            tab(Some(Status::Failed)),
        );
        assert_eq!(hidden_marker(&[&a, &b, &c]), Some(Status::NeedsYou));
        let (d, e) = (tab(Some(Status::Activity)), tab(None));
        assert_eq!(hidden_marker(&[&d, &e]), Some(Status::Activity));
        assert_eq!(hidden_marker(&[&e]), None);
    }
}
