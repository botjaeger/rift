//! Stitching a full-screen program's screens into one page the phone
//! scrolls natively (feature `stitch`, `docs/remote-protocol.md`).
//!
//! A program on the alternate screen draws only the rows in view; what is
//! above or below exists nowhere until it draws it. The gateway scrolls the
//! program itself (each wheel step a local round trip), reads each screen
//! once it rests, and joins the screens: [`learn`] finds the rectangle that
//! scrolls (the rows whose text moves, the columns that change: a sticky
//! header, a "jump to bottom" hint, a status line or a pane beside it stay
//! out), and [`Page::extend`] appends what a screen adds past its longest
//! overlap with the page so far. A screen that overlaps by less than
//! [`MIN_OVERLAP`] rows is never joined: a frame caught mid-redraw makes
//! the page stop growing, never wrong.
//!
//! Columns are counted in characters, one cell each.

use std::ops::Range;

use crate::protocol::Run;

/// Fewest rows a screen must share with the page to be joined to it.
pub(crate) const MIN_OVERLAP: usize = 4;
/// Unchanged columns bridged inside the changing band (spaces in a sentence).
const BRIDGE: usize = 3;

/// One screen as rows of runs, with each row's text padded to the width.
pub(crate) struct Frame {
    rows: Vec<Vec<Run>>,
    text: Vec<Vec<char>>,
}

impl Frame {
    /// `lines` as the gateway's `Screen` holds them: row 0 first, trailing
    /// blank rows left out.
    pub(crate) fn new(lines: &[Vec<Run>], rows: u16, cols: u16) -> Frame {
        let (rows, cols) = (rows as usize, cols as usize);
        let mut all: Vec<Vec<Run>> = lines.iter().take(rows).cloned().collect();
        all.resize(rows, Vec::new());
        let text = all
            .iter()
            .map(|r| {
                let mut t: Vec<char> = r
                    .iter()
                    .flat_map(|run| run.text.chars())
                    .take(cols)
                    .collect();
                t.resize(cols, ' ');
                t
            })
            .collect();
        Frame { rows: all, text }
    }

    fn band(&self, row: usize, cols: &Range<usize>) -> String {
        self.text[row][cols.clone()].iter().collect()
    }

    fn same_shape(&self, other: &Frame) -> bool {
        self.text.len() == other.text.len()
            && self.text.first().map(Vec::len) == other.text.first().map(Vec::len)
    }
}

/// The rectangle of the screen that scrolls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Area {
    pub rows: Range<usize>,
    pub cols: Range<usize>,
}

impl Area {
    fn height(&self) -> usize {
        self.rows.len()
    }
}

/// The scrolling rectangle of a program scrolled through `frames` (each a
/// rested screen after one step, the first before any): the widest band of
/// columns that changed, then the longest run of rows whose text sat on
/// another row of the frame before in at least a third of the steps. None
/// while too little moved to tell (fewer than [`MIN_OVERLAP`] + 1 rows).
pub(crate) fn learn(frames: &[Frame]) -> Option<Area> {
    let first = frames.first()?;
    if frames.len() < 2 || frames.iter().any(|f| !f.same_shape(first)) {
        return None;
    }
    let (rows, cols) = (first.text.len(), first.text.first()?.len());
    let changed_row: Vec<bool> = (0..rows)
        .map(|i| frames.iter().any(|f| f.text[i] != first.text[i]))
        .collect();
    let changed_col: Vec<bool> = (0..cols)
        .map(|c| {
            frames
                .iter()
                .any(|f| (0..rows).any(|i| changed_row[i] && f.text[i][c] != first.text[i][c]))
        })
        .collect();
    let band = widest_run(&changed_col, BRIDGE)?;
    // a row scrolls when its text came from another row, or went to one (the
    // rows at the edge the program brings rows in at only ever do the latter)
    // and it changes on most steps: a sticky header (the prompt a conversation
    // is in), a hint or a status line changes only now and then, even when its
    // new text sat on another row before
    let mut moved = vec![0usize; rows];
    let mut changed = vec![0usize; rows];
    for pair in frames.windows(2) {
        let a: Vec<String> = (0..rows).map(|i| pair[0].band(i, &band)).collect();
        let b: Vec<String> = (0..rows).map(|i| pair[1].band(i, &band)).collect();
        let in_a: std::collections::HashSet<&String> = a.iter().collect();
        let in_b: std::collections::HashSet<&String> = b.iter().collect();
        for i in 0..rows {
            let came = !b[i].trim().is_empty() && b[i] != a[i] && in_a.contains(&b[i]);
            let went = !a[i].trim().is_empty() && a[i] != b[i] && in_b.contains(&a[i]);
            moved[i] += usize::from(came || went);
            changed[i] += usize::from(a[i] != b[i]);
        }
    }
    let steps = frames.len() - 1;
    let need = (steps / 3).max(1);
    let scrolls: Vec<bool> = (0..rows)
        .map(|i| moved[i] >= need && changed[i] * 5 >= steps * 3)
        .collect();
    let rows = widest_run(&scrolls, 0)?;
    if rows.len() <= MIN_OVERLAP {
        return None;
    }
    // the band holds what changed; the document's columns run on both sides
    // of it (a prefix every line shares, line ends that stayed the same) up to
    // the screen's edge or a pane's border
    let border = |c: usize| -> bool {
        let ch = frames[0].text[rows.start][c];
        is_border(ch)
            && frames
                .iter()
                .all(|f| rows.clone().all(|i| f.text[i][c] == ch))
    };
    let mut start = band.start;
    while start > 0 && !border(start - 1) {
        start -= 1;
    }
    let mut end = band.end;
    while end < cols && !border(end) {
        end += 1;
    }
    Some(Area {
        rows,
        cols: start..end,
    })
}

/// A character a full-screen program draws a pane's edge with.
fn is_border(ch: char) -> bool {
    matches!(ch, '│' | '┃' | '║' | '|' | '▏' | '▕' | '┆' | '┊' | '╎')
}

/// The widest run of `true`, runs `bridge` apart or closer joined.
fn widest_run(flags: &[bool], bridge: usize) -> Option<Range<usize>> {
    let mut best: Option<Range<usize>> = None;
    let mut start: Option<usize> = None;
    let mut last = 0;
    for (i, &on) in flags.iter().enumerate() {
        if on {
            if start.is_some_and(|_| i - last - 1 > bridge) {
                consider(&mut best, start.take().unwrap()..last + 1);
            }
            start.get_or_insert(i);
            last = i;
        }
    }
    if let Some(s) = start {
        consider(&mut best, s..last + 1);
    }
    best
}

fn consider(best: &mut Option<Range<usize>>, run: Range<usize>) {
    if best.as_ref().is_none_or(|b| run.len() > b.len()) {
        *best = Some(run);
    }
}

/// Which way the program was scrolled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dir {
    /// Towards the start: the rows above come in at the top.
    Up,
    /// Towards the end: the rows below come in at the bottom.
    Down,
}

/// What [`Page::extend`] did with a screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Joined {
    /// `count` new rows at page index `from` on.
    Added { from: i64, count: usize },
    /// The screen shows rows the page already has where it is: nothing new.
    Same,
    /// Too little in common to join it safely.
    Unstitched,
}

/// The rows of the scrolling rectangle in document order. Indices are
/// signed: 0 is the first row of the area on the screen the page started
/// from; rows above it are negative.
pub(crate) struct Page {
    area: Area,
    top: i64,
    rows: Vec<Vec<Run>>,
    text: Vec<String>,
    /// Index of the row the program shows first in the area now.
    at: i64,
}

impl Page {
    /// The area's rows of `frame`, at index 0.
    pub(crate) fn new(area: Area, frame: &Frame) -> Page {
        let rows = area
            .rows
            .clone()
            .map(|i| cut(&frame.rows[i], &area.cols))
            .collect();
        let text = area
            .rows
            .clone()
            .map(|i| frame.band(i, &area.cols))
            .collect();
        Page {
            area,
            top: 0,
            rows,
            text,
            at: 0,
        }
    }

    pub(crate) fn area(&self) -> &Area {
        &self.area
    }
    pub(crate) fn top(&self) -> i64 {
        self.top
    }
    pub(crate) fn bottom(&self) -> i64 {
        self.top + self.rows.len() as i64
    }
    pub(crate) fn at(&self) -> i64 {
        self.at
    }
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }
    /// Rows `from..to` (page indices, clamped).
    pub(crate) fn slice(&self, from: i64, to: i64) -> &[Vec<Run>] {
        let a = (from - self.top).clamp(0, self.rows.len() as i64) as usize;
        let b = (to - self.top).clamp(a as i64, self.rows.len() as i64) as usize;
        &self.rows[a..b]
    }

    fn area_text(&self, frame: &Frame) -> Vec<String> {
        self.area
            .rows
            .clone()
            .map(|i| frame.band(i, &self.area.cols))
            .collect()
    }

    /// Where `frame`'s area rows sit in the page (the first such index), if
    /// they do, exactly.
    pub(crate) fn locate(&self, frame: &Frame) -> Option<i64> {
        let s = self.area_text(frame);
        let h = s.len();
        (0..=self.text.len().checked_sub(h)?)
            .find(|&i| self.text[i..i + h] == s[..])
            .map(|i| self.top + i as i64)
    }

    /// Joins `frame`, the screen after a step `dir`: what it shows past its
    /// longest overlap (at least [`MIN_OVERLAP`] rows) with the page's end
    /// on that side is added there, and the program is then at that end.
    pub(crate) fn extend(&mut self, frame: &Frame, dir: Dir) -> Joined {
        if frame.text.len() < self.area.rows.end
            || frame.text.first().map(Vec::len).unwrap_or(0) < self.area.cols.end
        {
            return Joined::Unstitched;
        }
        let s = self.area_text(frame);
        let h = s.len();
        let n = self.text.len();
        // still inside the rows the page holds (on the way to its end): only
        // where the program is changes (the first match from the end it
        // moves toward, where a repeat is likelier the right one)
        let mut found =
            (0..=n.saturating_sub(h)).filter(|&i| n >= h && self.text[i..i + h] == s[..]);
        let found = match dir {
            Dir::Down => found.next_back(),
            Dir::Up => found.next(),
        };
        if let Some(i) = found {
            self.at = self.top + i as i64;
            return Joined::Same;
        }
        match dir {
            Dir::Down => {
                let Some(k) = (MIN_OVERLAP..h.min(n) + 1)
                    .rev()
                    .find(|&k| self.text[n - k..] == s[..k])
                else {
                    return Joined::Unstitched;
                };
                let from = self.bottom();
                for (i, text) in s.iter().enumerate().skip(k) {
                    self.rows
                        .push(cut(&frame.rows[self.area.rows.start + i], &self.area.cols));
                    self.text.push(text.clone());
                }
                self.at = self.bottom() - h as i64;
                Joined::Added { from, count: h - k }
            }
            Dir::Up => {
                let Some(k) = (MIN_OVERLAP..h.min(n) + 1)
                    .rev()
                    .find(|&k| self.text[..k] == s[h - k..])
                else {
                    return Joined::Unstitched;
                };
                let count = h - k;
                let rows: Vec<Vec<Run>> = (0..count)
                    .map(|i| cut(&frame.rows[self.area.rows.start + i], &self.area.cols))
                    .collect();
                self.rows.splice(0..0, rows);
                self.text.splice(0..0, s[..count].iter().cloned());
                self.top -= count as i64;
                self.at = self.top;
                Joined::Added {
                    from: self.top,
                    count,
                }
            }
        }
    }

    /// The program's view moved to `at` (a seek that landed there).
    pub(crate) fn set_at(&mut self, at: i64) {
        self.at = at;
    }

    /// The area's height in rows.
    pub(crate) fn height(&self) -> usize {
        self.area.height()
    }
}

/// The cells `cols` of a row of runs, as runs (one cell per character).
fn cut(row: &[Run], cols: &Range<usize>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let mut at = 0usize;
    for run in row {
        let len = run.text.chars().count();
        let (a, b) = (at.max(cols.start), (at + len).min(cols.end));
        if a < b {
            let text: String = run.text.chars().skip(a - at).take(b - a).collect();
            out.push(Run {
                text,
                ..run.clone()
            });
        }
        at += len;
        if at >= cols.end {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: usize = 12;
    const COLS: usize = 40;

    fn run(t: &str) -> Run {
        Run {
            text: t.into(),
            fg: None,
            bg: None,
            flags: 0,
        }
    }

    /// The document's line `n` (a few repeat, as real output does).
    fn line(n: i64) -> String {
        if n % 7 == 3 {
            "  stage.".into()
        } else {
            format!(
                "answer {} line {} {}",
                n / 6,
                n % 6,
                "x".repeat((n % 5) as usize + 7)
            )
        }
    }

    /// A Claude Code–like screen with the document scrolled to `top`: a
    /// sticky header (row 0), the document (rows 1..10), a hint (row 10)
    /// when not at the end, and a status row (11) that ticks; columns 30..
    /// a side panel that never moves.
    fn screen(top: i64, end: i64, tick: u32) -> Frame {
        let mut lines = Vec::new();
        for r in 0..ROWS {
            let left = match r {
                0 => format!("❯ question {}", top / 6),
                1..=9 => line(top + r as i64 - 1),
                10 if top + 9 < end => "   jump to bottom ↓".into(),
                10 => String::new(),
                _ => format!("status {tick}"),
            };
            let mut left: String = left.chars().take(29).collect();
            while left.chars().count() < 30 {
                left.push(' ');
            }
            lines.push(vec![run(&left), run(&format!("│panel {r}"))]);
        }
        Frame::new(&lines, ROWS as u16, COLS as u16)
    }

    fn texts(p: &Page) -> Vec<String> {
        p.text.iter().map(|t| t.trim_end().to_string()).collect()
    }

    fn scrolled(starts: &[i64]) -> Vec<Frame> {
        starts
            .iter()
            .enumerate()
            .map(|(i, &t)| screen(t, 400, i as u32))
            .collect()
    }

    #[test]
    fn learns_the_document_rows_and_columns_without_header_hint_status_or_panel() {
        let area = learn(&scrolled(&[200, 203, 208, 212, 215, 220, 224])).unwrap();
        assert_eq!(
            area,
            Area {
                rows: 1..10,
                cols: 0..30
            }
        );
    }

    #[test]
    fn a_sticky_header_that_changes_now_and_then_stays_out_with_few_frames() {
        // Claude Code's header shows the prompt the top row is in: it changes when the
        // view crosses a prompt, every few steps, and its new text sat on a row before
        let frames: Vec<Frame> = [200i64, 203, 206, 209, 212, 215, 218, 221]
            .iter()
            .map(|&t| {
                let mut f = screen(t, 400, 0);
                // the header shows a document line that was on screen two steps before
                let sticky = line(t - t % 7);
                f.text[0] = format!("{sticky:<30}│panel 0").chars().collect();
                f.text[0].resize(COLS, ' ');
                f
            })
            .collect();
        assert_eq!(learn(&frames).map(|a| a.rows), Some(1..10));
    }

    #[test]
    fn too_little_movement_is_no_area() {
        assert_eq!(learn(&scrolled(&[200])), None);
        assert_eq!(learn(&scrolled(&[200, 200, 200])), None);
    }

    #[test]
    fn joins_downward_steps_of_any_size_into_the_document_in_order() {
        let frames = scrolled(&[200, 203, 208, 212, 215, 220, 224]);
        let mut page = Page::new(learn(&frames).unwrap(), &frames[0]);
        for f in &frames[1..] {
            assert!(matches!(page.extend(f, Dir::Down), Joined::Added { .. }));
        }
        let want: Vec<String> = (200..233).map(line).collect();
        assert_eq!(texts(&page), want);
        assert_eq!((page.top(), page.bottom(), page.at()), (0, 33, 24));
    }

    #[test]
    fn joins_upward_steps_at_the_top_with_negative_indices() {
        let frames = scrolled(&[200, 197, 192, 187]);
        let area = Area {
            rows: 1..10,
            cols: 0..29,
        };
        let mut page = Page::new(area, &frames[0]);
        for f in &frames[1..] {
            page.extend(f, Dir::Up);
        }
        let want: Vec<String> = (187..209).map(line).collect();
        assert_eq!(texts(&page), want);
        assert_eq!((page.top(), page.at()), (-13, -13));
    }

    #[test]
    fn a_screen_that_did_not_move_is_same_and_a_half_drawn_one_is_never_joined() {
        let area = Area {
            rows: 1..10,
            cols: 0..29,
        };
        let frames = scrolled(&[200, 203]);
        let mut page = Page::new(area, &frames[0]);
        assert_eq!(page.extend(&screen(200, 400, 9), Dir::Down), Joined::Same);
        // a redraw caught half way: the top half new, the bottom half old
        let mut half = screen(260, 400, 1);
        for r in 5..10 {
            half.text[r] = screen(200, 400, 1).text[r].clone();
        }
        assert_eq!(page.extend(&half, Dir::Down), Joined::Unstitched);
        assert_eq!(page.len(), 9);
        // a jump too far for any overlap
        assert_eq!(
            page.extend(&screen(300, 400, 2), Dir::Down),
            Joined::Unstitched
        );
    }

    #[test]
    fn a_screen_inside_the_page_moves_the_program_there_without_joining() {
        let area = Area {
            rows: 1..10,
            cols: 0..30,
        };
        let mut page = Page::new(area, &screen(200, 400, 0));
        for t in [203, 207, 211] {
            page.extend(&screen(t, 400, 0), Dir::Down);
        }
        assert_eq!((page.len(), page.at()), (20, 11));
        // back up inside the page, then down again past its end
        assert_eq!(page.extend(&screen(204, 400, 0), Dir::Up), Joined::Same);
        assert_eq!((page.len(), page.at()), (20, 4));
        assert_eq!(page.extend(&screen(209, 400, 0), Dir::Down), Joined::Same);
        assert_eq!(page.at(), 9);
        assert_eq!(
            page.extend(&screen(213, 400, 0), Dir::Down),
            Joined::Added { from: 20, count: 2 }
        );
        assert_eq!(page.at(), 13);
    }

    #[test]
    fn a_repeated_row_alone_never_joins_a_far_screen() {
        // the page ends on a "  stage." row; a screen 15 rows on starts with another one
        let area = Area {
            rows: 1..10,
            cols: 0..28,
        };
        let mut page = Page::new(area, &screen(198, 400, 0));
        assert_eq!(line(206), line(213));
        assert_eq!(
            page.extend(&screen(213, 400, 1), Dir::Down),
            Joined::Unstitched
        );
        assert_eq!(page.len(), 9);
    }

    #[test]
    fn locate_finds_the_programs_view_in_the_page() {
        let frames = scrolled(&[200, 205, 210]);
        let mut page = Page::new(
            Area {
                rows: 1..10,
                cols: 0..29,
            },
            &frames[0],
        );
        page.extend(&frames[1], Dir::Down);
        page.extend(&frames[2], Dir::Down);
        assert_eq!(page.locate(&screen(204, 400, 0)), Some(4));
        assert_eq!(page.locate(&screen(250, 400, 0)), None);
    }

    #[test]
    fn rows_keep_their_runs_cut_to_the_area() {
        let r = vec![
            run("ab"),
            Run {
                text: "cdef".into(),
                fg: Some("ff0000".into()),
                bg: None,
                flags: 1,
            },
            run("gh"),
        ];
        let out = cut(&r, &(1..5));
        assert_eq!(
            out.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(),
            ["b", "cde"]
        );
        assert_eq!(out[1].fg.as_deref(), Some("ff0000"));
    }
}
