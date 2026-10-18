//! Mark mode (Windows Terminal): keyboard selection over the pane's rows,
//! scrollback included. `mark_mode` puts a block cursor on the screen's
//! cursor row (or the selected block's first row); arrows / hjkl move it,
//! ⇧ extends a selection from where it started, w / b step words, 0 / $
//! go to the row's ends, PgUp / PgDn by a screen, g / G to the first and
//! last row. ⏎ or ⌘C copies the selection and leaves; Esc leaves with
//! nothing selected; any other ⌘ / ⌃ shortcut leaves (nothing selected)
//! and runs. The cursor walks the rows the pane draws (collapsed
//! output and the hidden live prompt are skipped) and the viewport
//! (`view_offset`) follows it. The selection is the terminal's own, so
//! copying works as a mouse selection's does, on the primary screen and
//! in the alternate one.

use super::*;

pub(crate) struct MarkState {
    pane: PaneId,
    /// Absolute line and column of the cursor.
    cursor: (u64, usize),
    /// Where ⇧-motion started the selection.
    anchor: Option<(u64, usize)>,
}

impl MarkState {
    /// The cursor's (absolute line, column), for drawing.
    pub(super) fn cursor(&self) -> (PaneId, u64, usize) {
        (self.pane, self.cursor.0, self.cursor.1)
    }
}

/// A key's motion in mark mode.
#[derive(Clone, Copy, PartialEq)]
enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordNext,
    WordPrev,
    LineStart,
    LineEnd,
    PageUp,
    PageDown,
    Top,
    Bottom,
}

impl App {
    /// Enter mark mode in the active pane (again: leave it).
    pub(super) fn start_mark_mode(&mut self) {
        if self.win.mark.take().is_some() {
            self.clear_mark_selection();
            return;
        }
        let hide = self.win.editor_active;
        let Some(pane) = self.win.tabs.get(self.win.active).and_then(|t| t.active()) else {
            return;
        };
        let rows = Self::shown_rows(pane, hide);
        let Some(&last) = rows.last() else {
            return;
        };
        let scrolled = pane
            .session
            .scrolled
            .load(std::sync::atomic::Ordering::Acquire);
        let selected_start = pane.selected_block.and_then(|id| {
            let blocks = pane.session.blocks.lock().unwrap();
            blocks.iter().find(|b| b.id == id).map(|b| b.prompt_start)
        });
        let (line, col) = match selected_start {
            Some(start) => (
                rows.iter().copied().find(|&r| r >= start).unwrap_or(last),
                0,
            ),
            None => {
                let term = pane.session.term.lock();
                let p = term.grid().cursor.point;
                let abs = (scrolled as i64 + p.line.0 as i64).max(0) as u64;
                // the hidden live prompt: the last row drawn above it
                let line = rows.iter().copied().rfind(|&r| r <= abs).unwrap_or(last);
                (line, if line == abs { p.column.0 } else { 0 })
            }
        };
        let id = pane.session.id;
        self.win.mark = Some(MarkState {
            pane: id,
            cursor: (line, col),
            anchor: None,
        });
        if let Some(p) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        {
            p.clear_selection();
        }
        self.win.a11y_announcement = Some(
            "Mark mode: arrow keys move, Shift extends the selection, Enter copies, Escape exits"
                .into(),
        );
        self.reveal_mark();
    }

    /// Mark mode's keys; true when used (every key is, while it is on,
    /// except another app shortcut: it ends mark mode and runs).
    pub(super) fn mark_mode_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let Some(mark) = &self.win.mark else {
            return false;
        };
        // focus moved to another pane, or the pane closed: mark mode ends
        if self.active_pane_id() != Some(mark.pane) {
            self.win.mark = None;
            return false;
        }
        if event.released {
            return true;
        }
        let mods = self.win.modifiers;
        let action = input::app_action(event, mods, &self.keymap);
        if action == Some(AppAction::MarkMode) {
            self.win.mark = None;
            self.clear_mark_selection();
            return true;
        }
        if action == Some(AppAction::Copy) {
            self.copy_mark();
            return true;
        }
        // ⌘T, ⌘P, ⌃Tab…: mark mode ends, the shortcut runs
        if action.is_some() && (mods.super_key() || mods.control_key()) {
            self.win.mark = None;
            self.clear_mark_selection();
            return false;
        }
        let shift = mods.shift_key();
        let letter = match (&event.unmodified, &event.logical_key) {
            (Some(Key::Character(s)), _) | (None, Key::Character(s)) => s.to_lowercase(),
            _ => String::new(),
        };
        let text = event.text.as_deref().unwrap_or("");
        let motion = match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.win.mark = None;
                self.clear_mark_selection();
                self.win.a11y_announcement = Some("Mark mode off".into());
                return true;
            }
            Key::Named(NamedKey::Enter) => {
                self.copy_mark();
                return true;
            }
            Key::Named(NamedKey::ArrowLeft) => Some(Motion::Left),
            Key::Named(NamedKey::ArrowRight) => Some(Motion::Right),
            Key::Named(NamedKey::ArrowUp) => Some(Motion::Up),
            Key::Named(NamedKey::ArrowDown) => Some(Motion::Down),
            Key::Named(NamedKey::PageUp) => Some(Motion::PageUp),
            Key::Named(NamedKey::PageDown) => Some(Motion::PageDown),
            Key::Named(NamedKey::Home) => Some(Motion::LineStart),
            Key::Named(NamedKey::End) => Some(Motion::LineEnd),
            _ if mods.super_key() || mods.control_key() || mods.alt_key() => None,
            _ if text == "$" => Some(Motion::LineEnd),
            _ if text == "G" => Some(Motion::Bottom),
            _ => match letter.as_str() {
                "h" => Some(Motion::Left),
                "l" => Some(Motion::Right),
                "k" => Some(Motion::Up),
                "j" => Some(Motion::Down),
                "w" => Some(Motion::WordNext),
                "b" => Some(Motion::WordPrev),
                "0" => Some(Motion::LineStart),
                "g" => Some(Motion::Top),
                _ => None,
            },
        };
        if let Some(m) = motion {
            // `$` and `G` are typed with ⇧: they move, they do not extend
            let extend = shift && !matches!(text, "$" | "G");
            self.move_mark(m, extend);
        }
        true
    }

    fn move_mark(&mut self, motion: Motion, extend: bool) {
        let hide = self.win.editor_active;
        let Some(mark) = &self.win.mark else { return };
        let (id, (line, col), anchor) = (mark.pane, mark.cursor, mark.anchor);
        let Some(pane) = self.win.tabs.get(self.win.active).and_then(|t| t.pane(id)) else {
            return;
        };
        let rows = Self::shown_rows(pane, hide);
        if rows.is_empty() {
            return;
        }
        let page = pane.last_grid.map_or(24, |g| g.rows).max(1);
        let idx = rows
            .iter()
            .position(|&r| r >= line)
            .unwrap_or(rows.len() - 1);
        let (cols, text_of) = {
            use rift_pty::alacritty_terminal::grid::Dimensions;
            let term = pane.session.term.lock();
            let scrolled = pane
                .session
                .scrolled
                .load(std::sync::atomic::Ordering::Acquire);
            let cols = term.grid().columns();
            // the rows a word motion may cross (this one and its neighbours)
            let lo = idx.saturating_sub(1);
            let hi = (idx + 1).min(rows.len() - 1);
            let text: Vec<(u64, Vec<char>)> = rows[lo..=hi]
                .iter()
                .map(|&abs| {
                    let l = abs as i64 - scrolled as i64;
                    let in_grid = l >= -(term.grid().history_size() as i64)
                        && l < term.grid().screen_lines() as i64;
                    let chars = if in_grid {
                        let row = &term.grid()[ALine(l as i32)];
                        (0..cols).map(|c| row[ACol(c)].c).collect()
                    } else {
                        Vec::new()
                    };
                    (abs, chars)
                })
                .collect();
            (cols, text)
        };
        let row_text = |abs: u64| -> &[char] {
            text_of
                .iter()
                .find(|(a, _)| *a == abs)
                .map_or(&[][..], |(_, t)| t.as_slice())
        };
        let last_col = cols.saturating_sub(1);
        let is_word = |c: char| !c.is_whitespace() && c != '\0';
        let (mut line, mut col) = (rows[idx], col.min(last_col));
        match motion {
            Motion::Left if col > 0 => col -= 1,
            Motion::Left if idx > 0 => (line, col) = (rows[idx - 1], last_col),
            Motion::Right if col < last_col => col += 1,
            Motion::Right if idx + 1 < rows.len() => (line, col) = (rows[idx + 1], 0),
            Motion::Left | Motion::Right => {}
            Motion::Up => line = rows[idx.saturating_sub(1)],
            Motion::Down => line = rows[(idx + 1).min(rows.len() - 1)],
            Motion::PageUp => line = rows[idx.saturating_sub(page)],
            Motion::PageDown => line = rows[(idx + page).min(rows.len() - 1)],
            Motion::Top => (line, col) = (rows[0], 0),
            Motion::Bottom => (line, col) = (rows[rows.len() - 1], 0),
            Motion::LineStart => col = 0,
            Motion::LineEnd => {
                let t = row_text(line);
                col = t.iter().rposition(|&c| is_word(c)).unwrap_or(0);
            }
            Motion::WordNext => {
                let t = row_text(line);
                let mut c = col;
                while c < t.len() && is_word(t[c]) {
                    c += 1;
                }
                while c < t.len() && !is_word(t[c]) {
                    c += 1;
                }
                if c < t.len() {
                    col = c;
                } else if idx + 1 < rows.len() {
                    let next = rows[idx + 1];
                    let t = row_text(next);
                    line = next;
                    col = t.iter().position(|&ch| is_word(ch)).unwrap_or(0);
                }
            }
            Motion::WordPrev => {
                // the start of the word before the cursor on this row, else
                // of the last word on the row above
                let word_start = |t: &[char], from: usize| {
                    let mut c = from.min(t.len());
                    while c > 0 && !is_word(t[c - 1]) {
                        c -= 1;
                    }
                    while c > 0 && is_word(t[c - 1]) {
                        c -= 1;
                    }
                    (c < t.len() && is_word(t[c])).then_some(c)
                };
                match word_start(row_text(line), col).filter(|&c| c < col) {
                    Some(c) => col = c,
                    None if idx > 0 => {
                        let prev = rows[idx - 1];
                        let t = row_text(prev);
                        line = prev;
                        col = word_start(t, t.len()).unwrap_or(0);
                    }
                    None => col = 0,
                }
            }
        }
        let anchor = if extend {
            Some(anchor.unwrap_or(mark.cursor))
        } else {
            None
        };
        if let Some(m) = &mut self.win.mark {
            m.cursor = (line, col);
            m.anchor = anchor;
        }
        self.apply_mark_selection();
        self.reveal_mark();
    }

    /// The terminal selection from the anchor to the cursor (both cells
    /// included); none without an anchor.
    fn apply_mark_selection(&mut self) {
        let Some(mark) = &self.win.mark else { return };
        let (id, cursor, anchor) = (mark.pane, mark.cursor, mark.anchor);
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.pane_mut(id))
        else {
            return;
        };
        let Some(anchor) = anchor else {
            pane.clear_selection();
            return;
        };
        let scrolled = pane
            .session
            .scrolled
            .load(std::sync::atomic::Ordering::Acquire) as i64;
        let point = |(abs, col): (u64, usize)| {
            APoint::new(ALine((abs as i64 - scrolled) as i32), ACol(col))
        };
        let (start, end) = if anchor <= cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        };
        let mut sel = Selection::new(SelectionType::Simple, point(start), ASide::Left);
        sel.update(point(end), ASide::Right);
        pane.session.term.lock().selection = Some(sel);
        pane.has_selection = true;
    }

    fn clear_mark_selection(&mut self) {
        if let Some(p) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        {
            p.clear_selection();
        }
    }

    /// ⏎ / ⌘C: copy the selection (if any) and leave mark mode.
    fn copy_mark(&mut self) {
        if let Some(text) = self.selection_text() {
            self.copy_to_clipboard(text);
        }
        self.win.mark = None;
        self.clear_mark_selection();
    }

    /// Scroll the pane so the cursor's row is in view.
    fn reveal_mark(&mut self) {
        use crate::views::terminal::DisplayLine;
        let hide = self.win.editor_active;
        let Some(mark) = &self.win.mark else { return };
        let (id, line) = (mark.pane, mark.cursor.0);
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.pane_mut(id))
        else {
            return;
        };
        let (display, _) = Self::display_list(pane, hide);
        let Some(idx) = display.iter().position(|d| *d == DisplayLine::Grid(line)) else {
            return;
        };
        let rows = pane.last_grid.map_or(24, |g| g.rows);
        let len = display.len();
        let max_off = len.saturating_sub(rows);
        // visible display rows: [len - rows - off, len - 1 - off]
        let below = len - 1 - idx;
        let off = pane.view_offset.min(max_off);
        pane.view_offset = if below < off {
            below
        } else if below >= off + rows {
            below + 1 - rows
        } else {
            off
        }
        .min(max_off);
    }

    /// Mark mode's state for `dump`.
    pub(super) fn mark_dump(&self) -> String {
        match &self.win.mark {
            Some(m) => format!(
                "mark=on pane={} cursor={}:{} anchor={:?}",
                m.pane, m.cursor.0, m.cursor.1, m.anchor
            ),
            None => "mark=off".into(),
        }
    }
}
