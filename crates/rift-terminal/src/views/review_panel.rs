//! Code review: a right panel listing the repository's staged and unstaged
//! changes (`rift_core::review`). Each file is a card that expands into its
//! hunks: one column of line numbers, syntax-colored code, tinted added and
//! removed rows with the changed words highlighted, and "N unmodified lines"
//! rows between hunks that open the file's unchanged lines (Warp's diff
//! view). Each file can be staged, unstaged or reverted from its header, each
//! hunk from the toolbar a hovered line shows. Revert goes through the
//! confirm dialog (it discards work). Long lines scroll sideways per file.
//! Maximized, the panel covers the panes and lists the files on its right.
//! The model loads on a background thread; the panel draws only the rows on
//! screen.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use rift_core::code_highlight::{self, Kind, Lang};
use rift_core::review::{Base, FileDiff, HunkId, LineKind, Review};
use rift_core::{Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::views::icons::{self, Icon};

use crate::views::explorer::status_color;

pub const REVIEW_W: f32 = 460.0;
pub const HEADER_H: f32 = 38.0;
const SECTION_H: f32 = 30.0;
const FILE_H: f32 = 32.0;
const HUNK_H: f32 = 24.0;
const LINE_H: f32 = 18.0;
const ROW_H: f32 = LINE_H + 4.0;
/// File cards: inset from the panel edges, corner radius, gap below.
const CARD_X: f32 = 10.0;
const CARD_R: f32 = 6.0;
const CARD_GAP: f32 = 8.0;
/// Unmodified lines one click on a gap row opens; a gap of up to
/// `REVEAL_ALL` lines opens whole.
const REVEAL_STEP: u32 = 20;
const REVEAL_ALL: u32 = 40;
/// Columns a code line shows before it is cut with `…` (scrolled sideways).
const MAX_COLS: usize = 1000;
/// A hovered line's hunk toolbar: button size; the gutter fits three.
const TOOL_W: f32 = 20.0;
const GUTTER_MIN: f32 = 3.0 * TOOL_W + 12.0;
/// Maximized: the file list on the right, its rows, and the narrowest
/// panel that still shows it.
const SIDEBAR_W: f32 = 300.0;
const SIDE_ROW_H: f32 = 26.0;
const SIDEBAR_MIN_PANEL_W: f32 = 760.0;

#[derive(Default)]
pub enum Load {
    #[default]
    Idle,
    Loading,
    NotARepo,
    Failed(String),
    Ready(Arc<Review>),
}

#[derive(Default)]
pub struct ReviewPanelState {
    pub open: bool,
    pub load: Load,
    /// Bumped per request; a reply for an older one is dropped.
    pub generation: u64,
    /// Expanded files: (staged section, path).
    pub expanded: HashSet<(bool, String)>,
    /// Collapsed sections (staged = true).
    pub collapsed_sections: HashSet<bool>,
    /// Unmodified lines opened per gap: (staged, path, gap) → count. Gap
    /// `k` is above hunk `k`; gap `hunks.len()` is below the last one.
    pub revealed: HashMap<(bool, String, usize), u32>,
    pub scroll: f32,
    pub content_h: f32,
    pub view_h: f32,
    /// Last write's failure, shown under the header until the next load.
    pub notice: Option<String>,
    /// Covering the panes, with the file list on the right.
    pub maximized: bool,
    /// Sideways scroll of each expanded file's code: (staged, path) → px.
    pub scroll_x: HashMap<(bool, String), f32>,
    /// Pointer in window coordinates (`None` outside the window): a hovered
    /// line shows its hunk's toolbar, a hovered file-list row lights.
    pub pointer: Option<(f32, f32)>,
    /// File list scroll (maximized).
    pub side_scroll: f32,
    /// Per frame: each expanded card's code area on screen with its file
    /// and furthest sideways scroll; the file list's rect (zero = none);
    /// the rows the pointer lights (code lines, file-list rows).
    pub cards_x: Vec<([f32; 4], (bool, String), f32)>,
    pub side_rect: [f32; 4],
    side_max: f32,
    pub hover_rows: Vec<[f32; 4]>,
    /// Per frame: each hunk's toolbar actions on its band, for the
    /// accessibility tree (on screen they show only on a hovered line).
    pub latent: Vec<([f32; 4], ReviewHit)>,
    /// The panel has the keyboard (⌥⌘G): arrows move `focus`, s / u / r act
    /// on it, Esc hands the keys back to the command line.
    pub keyed: bool,
    /// The row the keyboard stands on; follows the list when it reloads.
    pub focus: Option<ReviewStop>,
    /// Where `focus` last was in `stops()`, to land near it when its row
    /// leaves the list (a staged file moves to the other section).
    focus_index: usize,
    /// The commit message being written (the bar at the panel's foot).
    pub commit_msg: String,
    /// The message field has the keyboard.
    pub commit_focus: bool,
    /// A commit or push runs (off the UI thread): its button spins.
    pub busy: Option<GitBusy>,
    /// Per frame: rows a right-click offers "Send to agent…" on (file
    /// headers, hunk bands and lines) with what they send.
    pub send_rows: Vec<([f32; 4], ReviewHit)>,
    /// The base asked for in this repository (the chip shows it while a
    /// load is in flight or failed).
    pub base: Base,
    /// The repository of the last review that loaded.
    pub root: Option<std::path::PathBuf>,
    /// While the base picker is open: the default branch and every branch.
    pub branch_list: Option<(Option<String>, Vec<String>)>,
}

/// The repository write in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitBusy {
    Commit,
    Push,
}

/// Why stage, unstage and revert are off while the panel compares against
/// a branch.
pub const READ_ONLY_REASON: &str =
    "Against a branch the changes include commits: stage, unstage and revert work in Uncommitted";

/// A row the keyboard can stand on: a section, a file, or a hunk of an
/// expanded file.
#[derive(Debug, Clone, PartialEq)]
pub enum ReviewStop {
    Section(bool),
    File(bool, String),
    Hunk(bool, String, HunkId),
}

/// What a key does to the focused row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StopAct {
    Stage,
    Unstage,
    Revert,
    Open,
    Copy,
    /// Send the hunk or file to an agent pane.
    Send,
}

impl ReviewPanelState {
    /// Comparing against a branch: the changes are read-only here.
    pub fn read_only(&self) -> bool {
        self.model().is_some_and(|m| m.base.is_some())
    }

    /// Why Commit is off, or `None` when it can run.
    pub fn commit_reason(&self) -> Option<&'static str> {
        let model = self.model()?;
        Some(if self.busy == Some(GitBusy::Commit) {
            "Committing…"
        } else if model.base.is_some() {
            "Commit from Uncommitted, which shows what is staged"
        } else if model.staged.is_empty() {
            "Nothing staged"
        } else if self.commit_msg.trim().is_empty() {
            "Write a commit message"
        } else {
            return None;
        })
    }

    /// What "Send to agent…" pastes for a file (all its hunks) or one hunk.
    pub fn send_parts(
        &self,
        staged: bool,
        path: &str,
        hunk: Option<HunkId>,
    ) -> Option<Vec<crate::views::send_agent::SendPart>> {
        let model = self.model()?;
        let files = if staged {
            &model.staged
        } else {
            &model.unstaged
        };
        let f = files.iter().find(|f| f.path == path)?;
        let mut parts: Vec<_> = f
            .hunks
            .iter()
            .filter(|h| hunk.is_none_or(|id| h.id == id))
            .map(crate::views::send_agent::hunk_part)
            .collect();
        // the whole file, but the line cap dropped hunks past the last one
        if hunk.is_none()
            && f.truncated
            && let Some(last) = parts.last_mut()
        {
            last.truncated = true;
        }
        (!parts.is_empty()).then_some(parts)
    }

    /// Every row the keyboard visits, top to bottom.
    pub fn stops(&self) -> Vec<ReviewStop> {
        let Some(review) = self.model() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for staged in [true, false] {
            let files = if staged {
                &review.staged
            } else {
                &review.unstaged
            };
            if files.is_empty() {
                continue;
            }
            out.push(ReviewStop::Section(staged));
            if self.collapsed_sections.contains(&staged) {
                continue;
            }
            for f in files {
                out.push(ReviewStop::File(staged, f.path.clone()));
                if !f.binary && self.expanded.contains(&(staged, f.path.clone())) {
                    out.extend(
                        f.hunks
                            .iter()
                            .map(|h| ReviewStop::Hunk(staged, f.path.clone(), h.id)),
                    );
                }
            }
        }
        out
    }

    /// Put the keyboard on the panel, on the first row when it has none.
    pub fn take_keys(&mut self) {
        self.keyed = true;
        self.sync_focus();
        if self.focus.is_none() {
            self.focus = self.stops().into_iter().next();
            self.focus_index = 0;
            self.scroll_to_focus();
        }
    }

    /// Hand the keys back.
    pub fn drop_keys(&mut self) {
        self.keyed = false;
        self.focus = None;
    }

    /// Keep `focus` on a row that exists: after a reload the same row, else
    /// the one now at its place in the list, else the first.
    pub fn sync_focus(&mut self) {
        // a reload in flight has no list to compare with: the focus waits
        if !self.keyed || self.model().is_none() {
            return;
        }
        let stops = self.stops();
        match &self.focus {
            Some(f) if stops.contains(f) => {}
            _ if stops.is_empty() => self.focus = None,
            gone => {
                // a staged hunk leaves the list: the file's nearest other
                // hunk, else the file; any other row: whatever is now at
                // its place
                let near = match gone {
                    Some(ReviewStop::Hunk(s, p, _)) => stops
                        .iter()
                        .enumerate()
                        .filter(
                            |(_, t)| matches!(t, ReviewStop::Hunk(s2, p2, _) if s2 == s && p2 == p),
                        )
                        .min_by_key(|(i, _)| i.abs_diff(self.focus_index))
                        .map(|(_, t)| t.clone())
                        .or_else(|| {
                            let file = ReviewStop::File(*s, p.clone());
                            stops.contains(&file).then_some(file)
                        }),
                    _ => None,
                };
                let at = self.focus_index.min(stops.len() - 1);
                self.focus = Some(near.unwrap_or_else(|| stops[at].clone()));
            }
        }
        if let Some(at) = self
            .focus
            .as_ref()
            .and_then(|f| stops.iter().position(|s| s == f))
        {
            self.focus_index = at;
        }
    }

    /// Move the focus `delta` rows (clamped) and scroll it into view; a
    /// `delta` past either end goes to that end.
    pub fn move_focus(&mut self, delta: isize) {
        self.sync_focus();
        let stops = self.stops();
        if stops.is_empty() {
            return;
        }
        let at = (self.focus_index as isize + delta).clamp(0, stops.len() as isize - 1) as usize;
        self.focus = Some(stops[at].clone());
        self.focus_index = at;
        self.scroll_to_focus();
    }

    /// Put the focus on `stop` (it must be in `stops()`).
    fn set_focus(&mut self, stop: ReviewStop) {
        if let Some(at) = self.stops().iter().position(|s| *s == stop) {
            self.focus_index = at;
            self.focus = Some(stop);
            self.scroll_to_focus();
        }
    }

    /// Scroll so the focused row is on screen (the next frame clamps).
    pub fn scroll_to_focus(&mut self) {
        let (Some(review), Some(focus)) = (self.model().cloned(), self.focus.clone()) else {
            return;
        };
        let mut y = 0.0;
        for row in rows(self, &review) {
            let here = match (&row, &focus) {
                (RowKind::Section(s, _), ReviewStop::Section(t)) => s == t,
                (RowKind::File(s, f), ReviewStop::File(t, p)) => s == t && f.path == *p,
                (RowKind::Hunk(s, f, hi, _), ReviewStop::Hunk(t, p, id)) => {
                    s == t && f.path == *p && f.hunks[*hi].id == *id
                }
                _ => false,
            };
            if here {
                let h = row.height();
                if y < self.scroll {
                    self.scroll = y;
                } else if y + h > self.scroll + self.view_h - 8.0 {
                    self.scroll = (y + h - self.view_h + 8.0).max(0.0);
                }
                return;
            }
            y += row.height();
        }
    }

    /// ← on the focus: fold its section or file, else step out to the row
    /// that holds it.
    pub fn focus_left(&mut self) {
        self.sync_focus();
        match self.focus.clone() {
            Some(ReviewStop::Section(s)) => {
                self.collapsed_sections.insert(s);
            }
            Some(ReviewStop::File(s, p)) => {
                if !self.expanded.remove(&(s, p)) {
                    self.set_focus(ReviewStop::Section(s));
                }
            }
            Some(ReviewStop::Hunk(s, p, _)) => self.set_focus(ReviewStop::File(s, p)),
            None => {}
        }
    }

    /// → on the focus: open its section or file, else step into it.
    pub fn focus_right(&mut self) {
        self.sync_focus();
        match self.focus.clone() {
            Some(ReviewStop::Section(s)) => {
                if !self.collapsed_sections.remove(&s) {
                    self.move_focus(1);
                }
            }
            Some(ReviewStop::File(s, p)) => {
                if self.expanded.insert((s, p)) {
                    self.scroll_to_focus();
                } else {
                    self.move_focus(1);
                }
            }
            _ => {}
        }
    }

    /// Enter / Space on the focus: fold or unfold a section or file.
    pub fn focus_toggle(&mut self) {
        self.sync_focus();
        match self.focus.clone() {
            Some(ReviewStop::Section(s)) => {
                if !self.collapsed_sections.remove(&s) {
                    self.collapsed_sections.insert(s);
                }
            }
            Some(ReviewStop::File(s, p)) if !self.expanded.remove(&(s, p.clone())) => {
                self.expanded.insert((s, p));
            }
            _ => {}
        }
    }

    /// The click-equivalent of `act` on the focused row; `None` when the row
    /// has no such action (stage on a staged file, anything on a cut hunk).
    pub fn focus_hit(&self, act: StopAct) -> Option<ReviewHit> {
        let review = self.model()?;
        let file = |staged: bool, path: &str| {
            let files = if staged {
                &review.staged
            } else {
                &review.unstaged
            };
            files.iter().find(|f| f.path == path)
        };
        if self.read_only() && matches!(act, StopAct::Stage | StopAct::Unstage | StopAct::Revert) {
            return None;
        }
        match (self.focus.as_ref()?, act) {
            (ReviewStop::File(s, p), StopAct::Send) => {
                file(*s, p).filter(|f| !f.binary && !f.hunks.is_empty())?;
                Some(ReviewHit::Send(*s, p.clone(), None))
            }
            (ReviewStop::Section(false), StopAct::Stage) => Some(ReviewHit::StageAll),
            (ReviewStop::Section(true), StopAct::Unstage) => Some(ReviewHit::UnstageAll),
            (ReviewStop::File(false, p), StopAct::Stage) => Some(ReviewHit::Stage(p.clone(), None)),
            (ReviewStop::File(true, p), StopAct::Unstage) => {
                Some(ReviewHit::Unstage(p.clone(), None))
            }
            (ReviewStop::File(false, p), StopAct::Revert) => {
                Some(ReviewHit::Revert(p.clone(), None))
            }
            (ReviewStop::File(s, p), StopAct::Open) => {
                file(*s, p).filter(|f| f.status != 'D')?;
                Some(ReviewHit::Open(p.clone()))
            }
            (ReviewStop::Hunk(s, p, id), act) => {
                let partial = file(*s, p)?.hunks.iter().any(|h| h.id == *id && h.partial);
                if partial {
                    return None;
                }
                match (*s, act) {
                    (false, StopAct::Stage) => Some(ReviewHit::Stage(p.clone(), Some(*id))),
                    (true, StopAct::Unstage) => Some(ReviewHit::Unstage(p.clone(), Some(*id))),
                    (false, StopAct::Revert) => Some(ReviewHit::Revert(p.clone(), Some(*id))),
                    (_, StopAct::Copy) => Some(ReviewHit::Copy(*s, p.clone(), *id)),
                    (_, StopAct::Send) => Some(ReviewHit::Send(*s, p.clone(), Some(*id))),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub fn model(&self) -> Option<&Arc<Review>> {
        match &self.load {
            Load::Ready(r) => Some(r),
            _ => None,
        }
    }

    pub fn scroll_by(&mut self, px: f32) {
        let max = (self.content_h - self.view_h).max(0.0);
        self.scroll = (self.scroll + px).clamp(0.0, max);
    }

    /// Scroll the code of the file card at window `y` sideways by `px`.
    pub fn scroll_x_by(&mut self, px: f32, y: f32) {
        let Some((key, max)) = self
            .cards_x
            .iter()
            .find(|(r, ..)| y >= r[1] && y < r[1] + r[3])
            .map(|(_, k, max)| (k.clone(), *max))
        else {
            return;
        };
        let s = self.scroll_x.entry(key).or_default();
        *s = (*s + px).clamp(0.0, max);
    }

    /// Scroll the file list (maximized).
    pub fn side_scroll_by(&mut self, px: f32) {
        self.side_scroll = (self.side_scroll + px).clamp(0.0, self.side_max);
    }

    /// Which lit row the pointer is on (changes repaint the panel).
    pub fn hover_index(&self) -> Option<usize> {
        let (mx, my) = self.pointer?;
        self.hover_rows
            .iter()
            .position(|r| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3])
    }

    /// Show a file from the file list: its section and card open, scrolled
    /// to the card's top (clamped by the next frame).
    pub fn jump(&mut self, staged: bool, path: &str) {
        self.collapsed_sections.remove(&staged);
        self.expanded.insert((staged, path.to_string()));
        let Some(review) = self.model().cloned() else {
            return;
        };
        let mut y = 0.0;
        for row in rows(self, &review) {
            if let RowKind::File(s, f) = &row
                && *s == staged
                && f.path == path
            {
                self.scroll = y;
                return;
            }
            y += row.height();
        }
    }

    /// Open more of gap `k`'s unmodified lines: all of a short gap, else
    /// `REVEAL_STEP` more.
    pub fn reveal(&mut self, staged: bool, path: &str, k: usize) {
        let Some(review) = self.model() else {
            return;
        };
        let files = if staged {
            &review.staged
        } else {
            &review.unstaged
        };
        let Some((start, end)) = files
            .iter()
            .find(|f| f.path == path)
            .and_then(|f| gap(f, k))
        else {
            return;
        };
        let size = end - start;
        let shown = self
            .revealed
            .entry((staged, path.to_string(), k))
            .or_default();
        let hidden = size.saturating_sub(*shown);
        *shown += if hidden <= REVEAL_ALL {
            hidden
        } else {
            REVEAL_STEP
        };
    }
}

/// New-side lines [start, end) (1-based) a hunk covers. A hunk without
/// new lines sits after line `new_start`.
fn hunk_span(id: &HunkId) -> (u32, u32) {
    let start = if id.new_lines == 0 {
        id.new_start + 1
    } else {
        id.new_start
    };
    (start, start + id.new_lines)
}

/// Unmodified new-side lines [start, end) of gap `k`: above hunk `k`, or
/// below the last hunk when `k == hunks.len()`. `None` when empty or when
/// the file's new side was not loaded.
pub fn gap(f: &FileDiff, k: usize) -> Option<(u32, u32)> {
    let src = f.source.as_ref()?;
    let last = src.len() as u32 + 1;
    let start = match k {
        0 => 1,
        _ => hunk_span(&f.hunks.get(k - 1)?.id).1,
    };
    let end = match f.hunks.get(k) {
        Some(h) => hunk_span(&h.id).0,
        // a cut diff does not know where its last hunk ends
        None if k == f.hunks.len() && !f.truncated => last,
        None => return None,
    };
    let end = end.min(last);
    (end > start).then_some((start, end))
}

/// Gap `k`'s open lines and how many stay hidden. The gap above the first
/// hunk opens upward from it; the others open downward from the hunk above.
fn gap_split(state: &ReviewPanelState, staged: bool, f: &FileDiff, k: usize) -> (Range<u32>, u32) {
    let Some((start, end)) = gap(f, k) else {
        return (0..0, 0);
    };
    let open = state
        .revealed
        .get(&(staged, f.path.clone(), k))
        .copied()
        .unwrap_or(0)
        .min(end - start);
    let shown = if k == 0 {
        end - open..end
    } else {
        start..start + open
    };
    (shown, end - start - open)
}

/// Shown in place of a partial hunk's Stage/Unstage/Revert buttons.
const PARTIAL_HUNK_NOTE: &str = "Too long · use file actions";

/// What a click in the panel does.
#[derive(Debug, Clone, PartialEq)]
pub enum ReviewHit {
    Refresh,
    ToggleSection(bool),
    ToggleFile(bool, String),
    /// Open more unmodified lines of (staged, path, gap).
    Reveal(bool, String, usize),
    Open(String),
    Stage(String, Option<HunkId>),
    Unstage(String, Option<HunkId>),
    Revert(String, Option<HunkId>),
    /// An unstaged file row's ⋮: open, stage, discard (out of Stage's
    /// reach: the destructive one is never a click beside it).
    Menu(String),
    StageAll,
    UnstageAll,
    /// Cover the panes / back beside them.
    Maximize,
    /// Copy a hunk as a unified diff.
    Copy(bool, String, HunkId),
    /// Show a file from the maximized file list.
    Jump(bool, String),
    /// The base chip: pick what the review compares against.
    Base,
    /// The commit message field.
    CommitField,
    Commit,
    Push,
    /// "Send to agent…" for (staged, path, one hunk or the whole file).
    Send(bool, String, Option<HunkId>),
}

enum RowKind<'a> {
    Section(bool, usize),
    File(bool, &'a FileDiff),
    /// The band above a hunk: the unmodified lines still hidden above it,
    /// or its `@@` header.
    Hunk(bool, &'a FileDiff, usize, u32),
    /// (staged, file, hunk, line)
    Line(bool, &'a FileDiff, usize, usize),
    /// A hunk's n lines, before `visible` expands the ones on screen.
    Lines(bool, &'a FileDiff, usize, usize),
    /// Open unmodified lines: new-side line number and count, expanded by
    /// `visible` like `Lines`.
    Source(bool, &'a FileDiff, u32, usize),
    SourceLine(bool, &'a FileDiff, u32),
    /// Unmodified lines still hidden below the last hunk.
    Tail(bool, &'a FileDiff, u32),
    Note(String),
    /// The bottom of a file card and the gap under it.
    CardEnd,
}

impl RowKind<'_> {
    fn height(&self) -> f32 {
        match self {
            RowKind::Section(..) => SECTION_H,
            RowKind::File(..) => FILE_H,
            RowKind::Hunk(..) | RowKind::Tail(..) => HUNK_H,
            RowKind::Line(..) | RowKind::SourceLine(..) | RowKind::Note(_) => ROW_H,
            RowKind::Lines(.., n) | RowKind::Source(.., n) => *n as f32 * ROW_H,
            RowKind::CardEnd => CARD_GAP + 2.0,
        }
    }
}

type TextKey = (String, u32, &'static str, u16);

thread_local! {
    /// Shaped labels by content: scrolling a long diff re-draws the same
    /// lines every frame, and shaping dominates the frame otherwise.
    static SHAPED: std::cell::RefCell<std::collections::HashMap<TextKey, Arc<Buffer>>> =
        Default::default();
}

thread_local! {
    static SHAPED_RICH: std::cell::RefCell<std::collections::HashMap<String, Arc<Buffer>>> =
        Default::default();
}

/// Colored spans in one buffer, cached by content and colors.
fn rich(
    engine: &mut TextEngine,
    spans: &[(&str, Color)],
    size: f32,
    family: &'static str,
) -> Arc<Buffer> {
    let mut key = format!("{size}{family}");
    for (s, c) in spans {
        key.push_str(&format!("\u{1}{c:?}\u{2}{s}"));
    }
    if let Some(b) = SHAPED_RICH.with(|c| c.borrow().get(&key).cloned()) {
        return b;
    }
    let base = Attrs::new().family(Family::Name(family));
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    b.set_rich_text(
        &mut engine.font_system,
        spans
            .iter()
            .map(|(s, c)| (*s, base.clone().color(rift_renderer::text_color(*c)))),
        &base,
        Shaping::Advanced,
        None,
    );
    let b = Arc::new(b);
    SHAPED_RICH.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= SHAPED_CAP {
            c.clear();
        }
        c.insert(key, b.clone());
    });
    b
}

/// Entries kept before the cache starts over (a few screens of diff).
const SHAPED_CAP: usize = 4096;

/// Section / file disclosure mark (▾ open, ▸ closed) centred on `c`.
fn chevron(layer: &mut Layer, theme: &Theme, open: bool, c: [f32; 2]) {
    let dir = if open {
        super::Dir::Down
    } else {
        super::Dir::Right
    };
    super::chevron(layer, c, 3.0, dir, 1.3, theme.text_dim);
}

fn text(
    engine: &mut TextEngine,
    s: &str,
    size: f32,
    family: &'static str,
    weight: Weight,
) -> Arc<Buffer> {
    let key = (s.to_string(), size.to_bits(), family, weight.0);
    if let Some(b) = SHAPED.with(|c| c.borrow().get(&key).cloned()) {
        return b;
    }
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    b.set_text(
        &mut engine.font_system,
        s,
        &Attrs::new().family(Family::Name(family)).weight(weight),
        Shaping::Advanced,
        None,
    );
    let b = Arc::new(b);
    SHAPED.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= SHAPED_CAP {
            c.clear();
        }
        c.insert(key, b.clone());
    });
    b
}

fn rows<'a>(state: &ReviewPanelState, review: &'a Review) -> Vec<RowKind<'a>> {
    let mut out = Vec::new();
    for staged in [true, false] {
        let files = if staged {
            &review.staged
        } else {
            &review.unstaged
        };
        if files.is_empty() {
            continue;
        }
        out.push(RowKind::Section(staged, files.len()));
        if state.collapsed_sections.contains(&staged) {
            continue;
        }
        for f in files {
            out.push(RowKind::File(staged, f));
            if state.expanded.contains(&(staged, f.path.clone())) {
                file_rows(state, staged, f, &mut out);
            }
            out.push(RowKind::CardEnd);
        }
    }
    out
}

/// An expanded file's rows, between its header and the card's end.
fn file_rows<'a>(
    state: &ReviewPanelState,
    staged: bool,
    f: &'a FileDiff,
    out: &mut Vec<RowKind<'a>>,
) {
    if f.binary {
        out.push(RowKind::Note("Binary file".into()));
        return;
    }
    let source = |out: &mut Vec<RowKind<'a>>, r: Range<u32>| {
        if !r.is_empty() {
            out.push(RowKind::Source(staged, f, r.start, r.len()));
        }
    };
    for (hi, h) in f.hunks.iter().enumerate() {
        let (shown, hidden) = gap_split(state, staged, f, hi);
        // the gap row sits on the hidden lines: open lines above it join
        // the hunk above, open lines of the first gap lead into hunk 0
        if hi > 0 {
            source(out, shown.clone());
        }
        out.push(RowKind::Hunk(staged, f, hi, hidden));
        if hi == 0 {
            source(out, shown);
        }
        // lines are expanded lazily by `visible`
        out.push(RowKind::Lines(staged, f, hi, h.lines.len()));
    }
    let (shown, hidden) = gap_split(state, staged, f, f.hunks.len());
    source(out, shown);
    if hidden > 0 {
        out.push(RowKind::Tail(staged, f, hidden));
    }
    if f.truncated {
        out.push(RowKind::Note(format!(
            "Diff cut at {} lines",
            rift_core::review::MAX_LINES_PER_FILE
        )));
    }
}

/// A file card crossing the window: content top and bottom, its file, and
/// whether it is expanded.
struct Card<'a> {
    top: f32,
    bottom: f32,
    staged: bool,
    file: &'a FileDiff,
    open: bool,
}

/// What `visible` lays out: rows on screen with their y, the file cards
/// crossing the window, and the total content height.
struct Layout<'a> {
    rows: Vec<(f32, RowKind<'a>)>,
    cards: Vec<Card<'a>>,
    height: f32,
}

/// The rows intersecting [top, bottom) (content coordinates). A hunk's
/// lines are counted, not materialized, unless on screen: a 20 000-line
/// diff costs no more per frame than a short one.
fn visible<'a>(all: Vec<RowKind<'a>>, top: f32, bottom: f32) -> Layout<'a> {
    let mut out = Layout {
        rows: Vec::new(),
        cards: Vec::new(),
        height: 0.0,
    };
    let mut y = 0.0;
    let mut card = None;
    for row in all {
        let lazy = match row {
            RowKind::Lines(.., n) | RowKind::Source(.., n) => Some(n),
            _ => None,
        };
        if let Some(n) = lazy {
            let block = n as f32 * ROW_H;
            if y + block > top && y < bottom {
                let first = ((top - y) / ROW_H).floor().max(0.0) as usize;
                let last = (((bottom - y) / ROW_H).ceil().max(0.0) as usize).min(n);
                for i in first..last {
                    let one = match row {
                        RowKind::Lines(s, f, hi, _) => RowKind::Line(s, f, hi, i),
                        RowKind::Source(s, f, start, _) => {
                            RowKind::SourceLine(s, f, start + i as u32)
                        }
                        _ => unreachable!("lazy rows only"),
                    };
                    out.rows.push((y + i as f32 * ROW_H, one));
                }
            }
            y += block;
            continue;
        }
        match row {
            RowKind::File(staged, file) => card = Some((y, staged, file)),
            RowKind::CardEnd => {
                let card_bottom = y + 2.0;
                if let Some((card_top, staged, file)) = card.take()
                    && card_bottom > top
                    && card_top < bottom
                {
                    out.cards.push(Card {
                        top: card_top,
                        bottom: card_bottom,
                        staged,
                        file,
                        open: y - card_top > FILE_H,
                    });
                }
            }
            _ => {}
        }
        let h = row.height();
        if y + h > top && y < bottom {
            out.rows.push((y, row));
        }
        y += h;
    }
    out.height = y;
    out
}

/// A small text button; returns its rect.
#[allow(clippy::too_many_arguments)]
fn button(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    label: &str,
    right: f32,
    y: f32,
    h: f32,
    color: Color,
) -> [f32; 4] {
    let tw = engine.measure(label, 11.0, FONT_SANS, Weight::MEDIUM);
    let bw = tw + 14.0;
    let r = [right - bw, y + (h - 20.0) / 2.0, bw, 20.0];
    // a lift of the panel in either theme (white on light was invisible)
    layer.rect(
        [r[0], r[1]],
        [r[2], r[3]],
        super::palette::lift(theme, 0.06),
        4.0,
    );
    layer.text(
        text(engine, label, 11.0, FONT_SANS, Weight::MEDIUM),
        r[0] + 7.0,
        r[1] + 3.0,
        color,
    );
    r
}

/// Digits in the file's largest new-side line number (at least 2).
fn number_digits(f: &FileDiff) -> usize {
    let last = f
        .hunks
        .last()
        .map_or(0, |h| hunk_span(&h.id).1)
        .max(f.source.as_ref().map_or(0, |s| s.len() as u32 + 1));
    last.to_string().len().max(2)
}

/// A code row's left gutter: the line number, and room for the hunk
/// toolbar a hovered line shows there.
struct Gutter {
    adv: f32,
    num_w: f32,
}

impl Gutter {
    fn new(engine: &mut TextEngine, f: &FileDiff) -> Self {
        let adv = engine
            .measure("0", 12.0, mono_family(), Weight::NORMAL)
            .max(1.0);
        Gutter {
            adv,
            num_w: (number_digits(f) as f32 * adv + 18.0).max(GUTTER_MIN),
        }
    }

    fn code_x(&self, cx: f32) -> f32 {
        cx + self.num_w + 8.0
    }
}

/// Display columns of a code line (tabs as 4).
fn columns(s: &str) -> usize {
    s.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

/// The widest line a file card shows (its hunks and opened unmodified
/// lines), in columns, at most `MAX_COLS`.
fn widest(state: &ReviewPanelState, staged: bool, f: &FileDiff) -> usize {
    let hunks = f
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .map(|l| columns(&l.text));
    let opened = (0..=f.hunks.len())
        .flat_map(|k| gap_split(state, staged, f, k).0)
        .filter_map(|n| f.source.as_ref()?.get(n as usize - 1))
        .map(|s| columns(s));
    hunks.chain(opened).max().unwrap_or(0).min(MAX_COLS)
}

/// A hunk as a unified diff (`@@` header, then ` `/`+`/`-` lines).
pub fn hunk_patch(f: &FileDiff, id: &HunkId) -> Option<String> {
    let h = f.hunks.iter().find(|h| h.id == *id)?;
    let mut out = format!("--- a/{0}\n+++ b/{0}\n{1}\n", f.path, h.header.trim_end());
    for l in &h.lines {
        out.push(match l.kind {
            LineKind::Added => '+',
            LineKind::Removed => '-',
            LineKind::Context => ' ',
        });
        out.push_str(&l.text);
        out.push('\n');
    }
    Some(out)
}

/// A run of code text in one color.
type Span = (String, Color);

fn syntax_color(theme: &Theme, kind: Kind) -> Color {
    match kind {
        Kind::Keyword => theme.syntax.command,
        Kind::String => theme.syntax.string,
        Kind::Comment => theme.text_muted,
        Kind::Number => theme.syntax.flag,
    }
}

/// One code line as syntax-colored spans, tabs as 4 spaces, cut to
/// `max_cols` columns with `…`; plus `emph` as display columns.
fn code_spans(
    theme: &Theme,
    lang: Lang,
    line: &str,
    emph: Option<&Range<usize>>,
    max_cols: usize,
) -> (Vec<Span>, Option<(usize, usize)>) {
    let mut segments: Vec<(Range<usize>, Color)> = Vec::new();
    let mut at = 0;
    for (r, kind) in code_highlight::line(lang, &mut Default::default(), line) {
        if r.start > at {
            segments.push((at..r.start, theme.fg));
        }
        at = r.end;
        segments.push((r, syntax_color(theme, kind)));
    }
    if at < line.len() {
        segments.push((at..line.len(), theme.fg));
    }
    let width = |s: &str| {
        s.chars()
            .map(|c| if c == '\t' { 4 } else { 1 })
            .sum::<usize>()
    };
    let total = width(line);
    let room = if total > max_cols {
        max_cols.saturating_sub(1)
    } else {
        total
    };
    let mut out = Vec::new();
    let mut used = 0;
    'segments: for (r, color) in segments {
        let mut s = String::new();
        for c in line.get(r).unwrap_or_default().chars() {
            let w = if c == '\t' { 4 } else { 1 };
            if used + w > room {
                out.push((s, color));
                break 'segments;
            }
            match c {
                '\t' => s.push_str("    "),
                c => s.push(c),
            }
            used += w;
        }
        out.push((s, color));
    }
    if total > max_cols {
        out.push(("…".into(), theme.text_muted));
    }
    let col = |byte: usize| width(line.get(..byte).unwrap_or(line)).min(room);
    (out, emph.map(|r| (col(r.start), col(r.end))))
}

/// A code row inside a card at `cx`, `cw` wide: change bar and tint, the
/// line number, the code scrolled `sx` px sideways, the changed words.
#[allow(clippy::too_many_arguments)]
fn code_row(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    f: &FileDiff,
    kind: LineKind,
    number: Option<u32>,
    line: &str,
    emph: Option<&Range<usize>>,
    (cx, cw, ry, sx): (f32, f32, f32, f32),
    bounds: TextBounds,
) {
    let mono = mono_family();
    let g = Gutter::new(engine, f);
    let (adv, num_w) = (g.adv, g.num_w);
    let tint = match kind {
        LineKind::Added => Some(theme.success),
        LineKind::Removed => Some(theme.error),
        LineKind::Context => None,
    };
    if let Some(t) = tint {
        layer.rect([cx + 1.0, ry], [cw - 2.0, ROW_H], t.with_alpha(0.13), 0.0);
        layer.rect([cx + 1.0, ry], [num_w, ROW_H], t.with_alpha(0.12), 0.0);
        layer.rect([cx + 1.0, ry], [3.0, ROW_H], t, 0.0);
    }
    if let Some(n) = number {
        let s = n.to_string();
        layer.text_clipped(
            text(engine, &s, 12.0, mono, Weight::NORMAL),
            cx + num_w - 8.0 - s.len() as f32 * adv,
            ry + 3.0,
            if tint.is_some() {
                theme.text_dim
            } else {
                theme.text_muted
            },
            bounds,
        );
    }
    let code_x = g.code_x(cx);
    let lang = code_highlight::lang_for(Path::new(&f.path));
    let (spans, emph) = code_spans(theme, lang, line, emph, MAX_COLS);
    if let (Some(t), Some((a, b))) = (tint, emph)
        && b > a
    {
        // the part right of the gutter
        let l = (code_x + a as f32 * adv - 1.0 - sx).max(code_x - 2.0);
        let r = (code_x + b as f32 * adv + 1.0 - sx).min(cx + cw - 1.0);
        if r > l {
            layer.rect([l, ry + 2.0], [r - l, ROW_H - 4.0], t.with_alpha(0.32), 2.0);
        }
    }
    let spans: Vec<(&str, Color)> = spans.iter().map(|(s, c)| (s.as_str(), *c)).collect();
    layer.text_clipped(
        rich(engine, &spans, 12.0, mono),
        code_x - sx,
        ry + 3.0,
        theme.fg,
        TextBounds {
            left: (code_x - 2.0) as i32,
            ..bounds
        },
    );
}

/// The band over hidden unmodified lines: arrows toward where they open,
/// and their count. `up`/`down` pick the arrows.
#[allow(clippy::too_many_arguments)]
fn gap_band(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    f: &FileDiff,
    hidden: u32,
    (up, down): (bool, bool),
    (cx, cw, ry): (f32, f32, f32),
    bounds: TextBounds,
) {
    layer.rect(
        [cx + 1.0, ry],
        [cw - 2.0, HUNK_H],
        theme.fg.with_alpha(0.04),
        0.0,
    );
    if hidden == 0 {
        return;
    }
    let g = Gutter::new(engine, f);
    let num_w = g.num_w;
    let ax = cx + num_w - 8.0 - g.adv;
    let cy = ry + HUNK_H / 2.0;
    match (up, down) {
        (true, true) => {
            icons::draw(layer, Icon::ChevronUp, [ax, cy - 3.5], 11.0, theme.text_dim);
            icons::draw(
                layer,
                Icon::ChevronDown,
                [ax, cy + 3.5],
                11.0,
                theme.text_dim,
            );
        }
        (true, false) => icons::draw(layer, Icon::ChevronUp, [ax, cy], 12.0, theme.text_dim),
        _ => icons::draw(layer, Icon::ChevronDown, [ax, cy], 12.0, theme.text_dim),
    }
    let label = if hidden == 1 {
        "1 unmodified line".to_string()
    } else {
        format!("{hidden} unmodified lines")
    };
    layer.text_clipped(
        text(engine, &label, 11.5, FONT_SANS, Weight::NORMAL),
        cx + num_w + 8.0,
        ry + 4.5,
        theme.text_dim,
        bounds,
    );
}

/// The toolbar a hovered line shows over the gutter for its hunk: stage or
/// unstage, revert (unstaged), copy; only copy against a branch
/// (`read_only`). Its actions; empty for a hunk the line cap cut (hunk
/// operations refuse it).
fn hunk_actions(staged: bool, f: &FileDiff, hi: usize, read_only: bool) -> Vec<(Icon, ReviewHit)> {
    let h = &f.hunks[hi];
    if h.partial {
        return Vec::new();
    }
    let path = f.path.clone();
    let mut out = if read_only {
        Vec::new()
    } else if staged {
        vec![(Icon::Minus, ReviewHit::Unstage(path.clone(), Some(h.id)))]
    } else {
        vec![
            (Icon::Plus, ReviewHit::Stage(path.clone(), Some(h.id))),
            (Icon::Revert, ReviewHit::Revert(path.clone(), Some(h.id))),
        ]
    };
    out.push((Icon::Copy, ReviewHit::Copy(staged, path, h.id)));
    out
}

/// Draw the panel into `rect` = [x, y, w, h]; returns click targets.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &mut ReviewPanelState,
    rect: [f32; 4],
) -> Vec<([f32; 4], ReviewHit)> {
    let [x, y, w, h] = rect;
    let mut hits = Vec::new();
    state.sync_focus();
    state.cards_x.clear();
    state.hover_rows.clear();
    state.latent.clear();
    state.send_rows.clear();
    state.side_rect = [0.0; 4];
    // solid: the panel never shows the window's translucency or wallpaper
    layer.rect([x, y], [w, h], theme.bg, 0.0);
    if !state.maximized {
        layer.rect([x, y], [1.0, h], theme.border, 0.0);
    }

    // ── header ──
    layer.text(
        text(engine, "CODE REVIEW", 11.0, FONT_SANS, Weight::SEMIBOLD),
        x + 16.0,
        y + 12.0,
        theme.text_dim,
    );
    let refresh = [x + w - 34.0, y + 7.0, 24.0, 24.0];
    icons::draw(
        layer,
        Icon::Refresh,
        [refresh[0] + 12.0, refresh[1] + 12.0],
        14.0,
        theme.text_dim,
    );
    hits.push((refresh, ReviewHit::Refresh));
    let maximize = [refresh[0] - 28.0, y + 7.0, 24.0, 24.0];
    icons::draw(
        layer,
        if state.maximized {
            Icon::Restore
        } else {
            Icon::Maximize
        },
        [maximize[0] + 12.0, maximize[1] + 12.0],
        14.0,
        theme.text_dim,
    );
    hits.push((maximize, ReviewHit::Maximize));
    let title_w = engine.measure("CODE REVIEW", 11.0, FONT_SANS, Weight::SEMIBOLD);
    // the counts' left edge: the base chip stops before it
    let mut header_room = maximize[0] - 10.0;
    if let Some(review) = state.model().cloned() {
        let (add, del) = review
            .staged
            .iter()
            .chain(&review.unstaged)
            .fold((0, 0), |(a, d), f| (a + f.added, d + f.removed));
        let both = format!("+{add}−{del}");
        header_room -= engine.measure(&both, 11.5, mono_family(), Weight::NORMAL) + 8.0 + 12.0;
    }
    let mut chip_x = x + 16.0 + title_w + 12.0;
    if let Some(review) = state.model().cloned() {
        // the branch, cut so the chip still fits
        let room = (header_room - chip_x - 110.0).max(40.0);
        let branch = super::palette::elide_to(engine, &review.branch, 11.5, mono_family(), room);
        layer.text(
            text(engine, &branch, 11.5, mono_family(), Weight::NORMAL),
            chip_x,
            y + 11.5,
            theme.success,
        );
        chip_x += engine.measure(&branch, 11.5, mono_family(), Weight::NORMAL) + 8.0;
    }
    if !matches!(state.load, Load::NotARepo) {
        // the base chip: what the review compares against (a menu)
        let label = match (&state.base, state.model().and_then(|m| m.base.as_ref())) {
            (_, Some(b)) => format!("vs {}", b.name),
            (Base::Uncommitted, None) => "Uncommitted".to_string(),
            (Base::DefaultBranch, None) => "vs default branch".to_string(),
            (Base::Branch(n), None) => format!("vs {n}"),
        };
        let room = (header_room - chip_x - 26.0).max(30.0);
        let label = super::palette::elide_to(engine, &label, 11.0, FONT_SANS, room);
        let lw = engine.measure(&label, 11.0, FONT_SANS, Weight::MEDIUM);
        let chip = [chip_x, y + 9.0, lw + 26.0, 20.0];
        let lit = state.pointer.is_some_and(|(px, py)| {
            px >= chip[0] && px < chip[0] + chip[2] && py >= chip[1] && py < chip[1] + chip[3]
        });
        layer.bordered(
            [chip[0], chip[1]],
            [chip[2], chip[3]],
            if lit {
                theme.fg.with_alpha(0.08)
            } else {
                theme.fg.with_alpha(0.04)
            },
            theme.border,
            4.0,
            1.0,
        );
        layer.text(
            text(engine, &label, 11.0, FONT_SANS, Weight::MEDIUM),
            chip[0] + 8.0,
            chip[1] + 3.0,
            theme.text_ui,
        );
        super::chevron(
            layer,
            [chip[0] + chip[2] - 10.0, chip[1] + 10.5],
            3.0,
            super::Dir::Down,
            1.3,
            theme.text_dim,
        );
        hits.push((chip, ReviewHit::Base));
    }
    if let Some(review) = state.model().cloned() {
        let (add, del) = review
            .staged
            .iter()
            .chain(&review.unstaged)
            .fold((0, 0), |(a, d), f| (a + f.added, d + f.removed));
        let del_s = format!("−{del}");
        let add_s = format!("+{add}");
        let del_w = engine.measure(&del_s, 11.5, mono_family(), Weight::NORMAL);
        let add_w = engine.measure(&add_s, 11.5, mono_family(), Weight::NORMAL);
        let dx = maximize[0] - 10.0 - del_w;
        layer.text(
            text(engine, &del_s, 11.5, mono_family(), Weight::NORMAL),
            dx,
            y + 11.5,
            theme.error,
        );
        layer.text(
            text(engine, &add_s, 11.5, mono_family(), Weight::NORMAL),
            dx - 8.0 - add_w,
            y + 11.5,
            theme.success,
        );
    }
    layer.hline(x + 1.0, y + HEADER_H, w - 1.0, 1.0, theme.border);

    let mut body_top = y + HEADER_H + 1.0;
    if let Some(notice) = state.notice.clone() {
        layer.rect(
            [x + 1.0, body_top],
            [w - 1.0, 26.0],
            theme.error.with_alpha(0.12),
            0.0,
        );
        layer.text(
            text(engine, &notice, 11.5, FONT_SANS, Weight::NORMAL),
            x + 16.0,
            body_top + 6.0,
            theme.error,
        );
        body_top += 26.0;
    }
    // against a branch: what it diffs from, and why the writes are off
    if let Some(base) = state.model().and_then(|m| m.base.clone()) {
        layer.rect(
            [x + 1.0, body_top],
            [w - 1.0, 26.0],
            theme.fg.with_alpha(0.04),
            0.0,
        );
        let head = format!("vs {} (merge-base {:.7})", base.name, base.merge_base);
        let head = super::palette::elide_to(engine, &head, 11.5, FONT_SANS, w - 32.0);
        let hw = engine.measure(&head, 11.5, FONT_SANS, Weight::NORMAL);
        layer.text(
            text(engine, &head, 11.5, FONT_SANS, Weight::NORMAL),
            x + 16.0,
            body_top + 6.0,
            theme.text_ui,
        );
        let room = w - 32.0 - hw - 12.0;
        if room > 60.0 {
            let why = super::palette::elide_to(
                engine,
                "· read-only: stage and revert need Uncommitted",
                11.5,
                FONT_SANS,
                room,
            );
            layer.text(
                text(engine, &why, 11.5, FONT_SANS, Weight::NORMAL),
                x + 16.0 + hw + 8.0,
                body_top + 6.0,
                theme.text_muted,
            );
        }
        body_top += 26.0;
    }
    // maximized: the file list takes the right side when there is room
    let side = state.maximized && w >= SIDEBAR_MIN_PANEL_W;
    let main_w = if side { w - SIDEBAR_W } else { w };
    // the commit bar at the foot, whenever there is a repository
    let bar_h = if state.model().is_some() {
        commit_bar_h(state)
    } else {
        0.0
    };
    let body_bottom = y + h - bar_h;
    if bar_h > 0.0 {
        commit_bar(
            layer,
            engine,
            theme,
            state,
            [x, body_bottom, main_w, bar_h],
            &mut hits,
        );
    }
    state.view_h = body_bottom - body_top;

    let message = match &state.load {
        Load::Idle | Load::Loading => Some("Loading changes…".to_string()),
        Load::NotARepo => Some("Not a git repository".to_string()),
        Load::Failed(e) => Some(e.clone()),
        Load::Ready(r) if r.staged.is_empty() && r.unstaged.is_empty() => Some(match &r.base {
            Some(b) => format!("No changes against {}", b.name),
            None => "No changes — working tree clean".to_string(),
        }),
        Load::Ready(_) => None,
    };
    if let Some(m) = message {
        state.content_h = 0.0;
        state.scroll = 0.0;
        layer.text(
            text(engine, &m, 12.5, FONT_SANS, Weight::NORMAL),
            x + 16.0,
            body_top + 16.0,
            theme.text_muted,
        );
        return hits;
    }
    let Some(review) = state.model().cloned() else {
        return hits;
    };
    let read_only = review.base.is_some();

    let window = |s: &ReviewPanelState| (s.scroll - 4.0, s.scroll - 4.0 + s.view_h);
    let (lo, hi) = window(state);
    let mut layout = visible(rows(state, &review), lo, hi);
    state.content_h = layout.height + 12.0;
    let before = state.scroll;
    state.scroll_by(0.0);
    if state.scroll != before {
        // clamped (the diff shrank): lay out the corrected window
        let (lo, hi) = window(state);
        layout = visible(rows(state, &review), lo, hi);
    }
    let bounds = TextBounds {
        left: (x + 1.0) as i32,
        top: body_top as i32,
        right: (x + main_w) as i32,
        bottom: body_bottom as i32,
    };
    let inside = |top: f32, rh: f32| top + rh > body_top && top < body_bottom;
    // buttons are drawn (and clickable) only fully inside the body
    let fully = |top: f32, rh: f32| top >= body_top && top + rh <= body_bottom;
    let pointer = state.pointer;
    let over = |r: [f32; 4]| {
        pointer.is_some_and(|(px, py)| {
            px >= r[0] && px < r[0] + r[2] && py >= r[1] && py < r[1] + r[3]
        })
    };
    let mono = mono_family();
    let scroll = state.scroll;
    let to_screen = move |cy: f32| body_top + 4.0 + cy - scroll;
    // file cards span the panel minus the scrollbar lane
    let cx = x + CARD_X;
    let cw = main_w - 2.0 * CARD_X;
    let card_bounds = TextBounds {
        left: (cx + 1.0) as i32,
        right: (cx + cw - 1.0) as i32,
        ..bounds
    };
    // marks on rows half under the header are cut at the body's edge
    layer.set_clip(Some((
        [x + 1.0, body_top, main_w - 1.0, body_bottom - body_top],
        0.0,
    )));
    // solid cards: the header on `surface`, the code on `bg_chrome`
    for card in &layout.cards {
        let (t, b) = (to_screen(card.top), to_screen(card.bottom));
        layer.bordered(
            [cx, t],
            [cw, b - t],
            theme.surface,
            theme.border,
            CARD_R,
            1.0,
        );
        let code_top = t + FILE_H;
        let code_h = b - 1.0 - code_top;
        if card.open && code_h > 0.0 {
            // rounded at the card's bottom corners, square under the header
            layer.rect(
                [cx + 1.0, code_top],
                [cw - 2.0, code_h],
                theme.bg_chrome,
                CARD_R - 1.0,
            );
            layer.rect(
                [cx + 1.0, code_top],
                [cw - 2.0, code_h.min(CARD_R)],
                theme.bg_chrome,
                0.0,
            );
            // sideways scroll: as far as the widest line shown
            let g = Gutter::new(engine, card.file);
            let code_w = cx + cw - 8.0 - g.code_x(cx);
            let max = (widest(state, card.staged, card.file) as f32 * g.adv - code_w).max(0.0);
            let key = (card.staged, card.file.path.clone());
            if let Some(s) = state.scroll_x.get_mut(&key) {
                *s = s.min(max);
            }
            let top = code_top.max(body_top);
            let bottom = (b - 1.0).min(body_bottom);
            if bottom > top {
                state.cards_x.push(([cx, top, cw, bottom - top], key, max));
            }
        }
    }
    let scroll_x = |staged: bool, f: &FileDiff| {
        state
            .scroll_x
            .get(&(staged, f.path.clone()))
            .copied()
            .unwrap_or(0.0)
    };
    let mut hover_rows = Vec::new();
    let mut send_rows = Vec::new();
    let mut latent = Vec::new();
    // the row the keyboard stands on gets the focus ring (DESIGN.md)
    let focus = state.keyed.then(|| state.focus.clone()).flatten();
    let mut focus_rect = None;

    // the hovered code line: its hunk's toolbar draws over the gutter
    let mut toolbar: Option<(f32, bool, &FileDiff, usize)> = None;
    for (cy, row) in &layout.rows {
        let ry = to_screen(*cy);
        let rh = row.height();
        if !inside(ry, rh) {
            continue;
        }
        match row {
            RowKind::Section(staged, n) => {
                if focus == Some(ReviewStop::Section(*staged)) {
                    focus_rect = Some([x + 6.0, ry + 1.0, main_w - 12.0, rh - 2.0]);
                }
                let open = !state.collapsed_sections.contains(staged);
                let title = if *staged { "STAGED" } else { "CHANGES" };
                chevron(layer, theme, open, [x + 17.5, ry + 15.0]);
                layer.text_clipped(
                    text(
                        engine,
                        &format!("{title}  {n}"),
                        11.0,
                        FONT_SANS,
                        Weight::SEMIBOLD,
                    ),
                    x + 28.0,
                    ry + 8.0,
                    theme.text_dim,
                    bounds,
                );
                hits.push((
                    [x, ry, main_w - 150.0, rh],
                    ReviewHit::ToggleSection(*staged),
                ));
                if fully(ry, rh) && !read_only {
                    let (label, hit) = if *staged {
                        ("Unstage all", ReviewHit::UnstageAll)
                    } else {
                        ("Stage all", ReviewHit::StageAll)
                    };
                    let r = button(
                        layer,
                        engine,
                        theme,
                        label,
                        x + main_w - 12.0,
                        ry,
                        rh,
                        theme.text_ui,
                    );
                    hits.push((r, hit));
                }
            }
            RowKind::File(staged, f) => {
                let key = (*staged, f.path.clone());
                let open = state.expanded.contains(&key);
                if focus == Some(ReviewStop::File(*staged, f.path.clone())) {
                    focus_rect = Some([cx, ry, cw, rh]);
                }
                if open {
                    layer.hline(cx + 1.0, ry + rh - 1.0, cw - 2.0, 1.0, theme.border);
                }
                chevron(layer, theme, open, [cx + 14.0, ry + rh / 2.0]);
                let letter = if f.status == '?' { 'U' } else { f.status };
                layer.text_clipped(
                    text(engine, &letter.to_string(), 11.0, mono, Weight::SEMIBOLD),
                    cx + 26.0,
                    ry + 9.0,
                    status_color(theme, f.status),
                    card_bounds,
                );
                let (dir, name) = match f.path.rsplit_once('/') {
                    Some((d, n)) => (d, n),
                    None => ("", f.path.as_str()),
                };
                let name_x = cx + 40.0;
                let name_w = engine.measure(name, 12.5, FONT_SANS, Weight::MEDIUM);
                // buttons first so the counts sit left of them
                let mut right = cx + cw - 8.0;
                let mut actions = Vec::new();
                if fully(ry, rh) {
                    let mut add = |icon: Icon, hit: ReviewHit, color: Color| {
                        let r = [right - 22.0, ry + (rh - 22.0) / 2.0, 22.0, 22.0];
                        icons::draw(layer, icon, [r[0] + 11.0, r[1] + 11.0], 14.0, color);
                        right = r[0] - 2.0;
                        actions.push((r, hit));
                    };
                    if read_only {
                        // against a branch: open only (READ_ONLY_REASON)
                    } else if *staged {
                        add(
                            Icon::Minus,
                            ReviewHit::Unstage(f.path.clone(), None),
                            theme.text_ui,
                        );
                    } else {
                        add(
                            Icon::Plus,
                            ReviewHit::Stage(f.path.clone(), None),
                            theme.text_ui,
                        );
                        // discard lives in this menu, not as a button
                        // beside Stage
                        add(Icon::More, ReviewHit::Menu(f.path.clone()), theme.text_dim);
                    }
                    if f.status != 'D' {
                        add(Icon::Open, ReviewHit::Open(f.path.clone()), theme.text_dim);
                    }
                }
                // +added −removed in a pill, colored per side
                // a zero side is left out (`+4`, not `+4 −0`); a file with
                // neither (a mode change) shows `±0`
                let mut sides: Vec<(String, Color)> = Vec::new();
                if f.added > 0 {
                    sides.push((format!("+{}", f.added), theme.success));
                }
                if f.removed > 0 {
                    sides.push((format!("−{}", f.removed), theme.error));
                }
                if sides.is_empty() {
                    sides.push(("±0".into(), theme.text_muted));
                }
                let widths: Vec<f32> = sides
                    .iter()
                    .map(|(s, _)| engine.measure(s, 11.0, mono, Weight::NORMAL))
                    .collect();
                let pill_w = widths.iter().sum::<f32>() + 6.0 * (sides.len() - 1) as f32 + 12.0;
                let pill_x = right - 6.0 - pill_w;
                layer.rect(
                    [pill_x, ry + (rh - 18.0) / 2.0],
                    [pill_w, 18.0],
                    theme.bg_chrome,
                    4.0,
                );
                let mut sx = pill_x + 6.0;
                for ((s, c), w) in sides.iter().zip(&widths) {
                    layer.text_clipped(
                        text(engine, s, 11.0, mono, Weight::NORMAL),
                        sx,
                        ry + 9.0,
                        *c,
                        card_bounds,
                    );
                    sx += w + 6.0;
                }
                // a long name stops before the counts, ending in `…`
                let name_room = (pill_x - 8.0 - name_x).max(10.0);
                let name_shown = if name_w > name_room {
                    super::palette::elide_to(engine, name, 12.5, FONT_SANS, name_room)
                } else {
                    name.to_string()
                };
                layer.text_clipped(
                    text(engine, &name_shown, 12.5, FONT_SANS, Weight::MEDIUM),
                    name_x,
                    ry + 8.0,
                    theme.text_bright,
                    TextBounds {
                        right: (pill_x - 8.0) as i32,
                        ..card_bounds
                    },
                );
                if !dir.is_empty() && name_x + name_w + 8.0 < pill_x - 20.0 {
                    layer.text_clipped(
                        text(engine, dir, 11.5, FONT_SANS, Weight::NORMAL),
                        name_x + name_w + 8.0,
                        ry + 9.0,
                        theme.text_muted,
                        TextBounds {
                            right: (pill_x - 10.0) as i32,
                            ..card_bounds
                        },
                    );
                }
                // the row first, then its actions (screen reader order)
                hits.push((
                    [cx, ry, (right - cx).max(0.0), rh],
                    ReviewHit::ToggleFile(*staged, f.path.clone()),
                ));
                hits.extend(actions);
                if !f.binary && !f.hunks.is_empty() {
                    send_rows.push((
                        [cx, ry, cw, rh],
                        ReviewHit::Send(*staged, f.path.clone(), None),
                    ));
                }
            }
            RowKind::Hunk(staged, f, hi, hidden) => {
                let hunk = &f.hunks[*hi];
                if focus == Some(ReviewStop::Hunk(*staged, f.path.clone(), hunk.id)) {
                    focus_rect = Some([cx + 2.0, ry, cw - 4.0, rh]);
                }
                send_rows.push((
                    [cx, ry, cw, rh],
                    ReviewHit::Send(*staged, f.path.clone(), Some(hunk.id)),
                ));
                gap_band(
                    layer,
                    engine,
                    theme,
                    f,
                    *hidden,
                    (true, *hi > 0),
                    (cx, cw, ry),
                    card_bounds,
                );
                let mut right = cx + cw - 8.0;
                if hunk.partial {
                    // cut by the diff line cap: hunk actions would apply a
                    // truncated patch (core refuses them); the file row's
                    // buttons act on the whole file
                    let note = PARTIAL_HUNK_NOTE;
                    let nw = engine.measure(note, 11.0, FONT_SANS, Weight::NORMAL);
                    right -= nw;
                    if fully(ry, rh) {
                        layer.text(
                            text(engine, note, 11.0, FONT_SANS, Weight::NORMAL),
                            right,
                            ry + (rh - 15.0) / 2.0,
                            theme.text_muted,
                        );
                    }
                    right -= 4.0;
                } else if fully(ry, rh) {
                    // the hover toolbar's actions, for screen readers
                    for (_, hit) in hunk_actions(*staged, f, *hi, read_only) {
                        latent.push(([cx, ry, cw, rh], hit));
                    }
                }
                if *hidden > 0 {
                    hits.push((
                        [cx, ry, (right - cx).max(0.0), rh],
                        ReviewHit::Reveal(*staged, f.path.clone(), *hi),
                    ));
                } else {
                    // nothing hidden above: the hunk's own range
                    let code_x = Gutter::new(engine, f).code_x(cx);
                    let room = (right - 6.0 - code_x).max(10.0);
                    let header = super::palette::elide_to(engine, &hunk.header, 11.0, mono, room);
                    layer.text_clipped(
                        text(engine, &header, 11.0, mono, Weight::NORMAL),
                        code_x,
                        ry + 5.0,
                        theme.text_dim,
                        TextBounds {
                            right: (right - 6.0) as i32,
                            ..card_bounds
                        },
                    );
                }
            }
            RowKind::Tail(staged, f, hidden) => {
                gap_band(
                    layer,
                    engine,
                    theme,
                    f,
                    *hidden,
                    (false, true),
                    (cx, cw, ry),
                    card_bounds,
                );
                hits.push((
                    [cx, ry, cw, rh],
                    ReviewHit::Reveal(*staged, f.path.clone(), f.hunks.len()),
                ));
            }
            RowKind::Line(staged, f, hi, li) => {
                let line = &f.hunks[*hi].lines[*li];
                let row_rect = [cx, ry, cw, rh];
                hover_rows.push(row_rect);
                send_rows.push((
                    row_rect,
                    ReviewHit::Send(*staged, f.path.clone(), Some(f.hunks[*hi].id)),
                ));
                let hovered = fully(ry, rh)
                    && over(row_rect)
                    && !hunk_actions(*staged, f, *hi, read_only).is_empty();
                if hovered {
                    toolbar = Some((ry, *staged, f, *hi));
                }
                code_row(
                    layer,
                    engine,
                    theme,
                    f,
                    line.kind,
                    // the toolbar covers the number
                    line.new_no.filter(|_| !hovered),
                    &line.text,
                    line.emph.as_ref(),
                    (cx, cw, ry, scroll_x(*staged, f)),
                    card_bounds,
                );
            }
            RowKind::SourceLine(staged, f, n) => {
                let line = f
                    .source
                    .as_ref()
                    .and_then(|s| s.get(*n as usize - 1))
                    .map_or("", String::as_str);
                code_row(
                    layer,
                    engine,
                    theme,
                    f,
                    LineKind::Context,
                    Some(*n),
                    line,
                    None,
                    (cx, cw, ry, scroll_x(*staged, f)),
                    card_bounds,
                );
            }
            RowKind::Lines(..) | RowKind::Source(..) | RowKind::CardEnd => {} // drawn above
            RowKind::Note(s) => {
                layer.text_clipped(
                    text(engine, s, 11.5, FONT_SANS, Weight::NORMAL),
                    cx + 40.0,
                    ry + 4.0,
                    theme.text_muted,
                    card_bounds,
                );
            }
        }
    }
    if let Some(r) = focus_rect {
        layer.bordered(
            [r[0] - 2.0, r[1] - 2.0],
            [r[2] + 4.0, r[3] + 4.0],
            theme.accent.with_alpha(0.0),
            theme.accent,
            6.0,
            2.0,
        );
    }
    if let Some((ry, staged, f, hi)) = toolbar {
        let actions = hunk_actions(staged, f, hi, read_only);
        let bar = [
            cx + 5.0,
            ry + 1.0,
            actions.len() as f32 * TOOL_W + 4.0,
            ROW_H - 2.0,
        ];
        layer.bordered(
            [bar[0], bar[1]],
            [bar[2], bar[3]],
            theme.surface,
            theme.border,
            4.0,
            1.0,
        );
        for (i, (icon, hit)) in actions.into_iter().enumerate() {
            let r = [bar[0] + 2.0 + i as f32 * TOOL_W, bar[1], TOOL_W, bar[3]];
            if over(r) {
                layer.rect(
                    [r[0] + 1.0, r[1] + 2.0],
                    [r[2] - 2.0, r[3] - 4.0],
                    theme.fg.with_alpha(0.08),
                    3.0,
                );
            }
            let color = if matches!(hit, ReviewHit::Revert(..)) {
                theme.error
            } else {
                theme.text_ui
            };
            icons::draw(
                layer,
                icon,
                [r[0] + TOOL_W / 2.0, r[1] + r[3] / 2.0],
                13.0,
                color,
            );
            // ahead of the row's other targets, in order
            hits.insert(i, (r, hit));
        }
    }
    layer.set_clip(None);
    state.hover_rows = hover_rows;
    state.send_rows = send_rows;
    state.latent = latent;
    // hits outside the body (scrolled under the header) never fire
    // and a row half under the header only takes clicks on its visible part
    let hits: Vec<([f32; 4], ReviewHit)> = hits
        .into_iter()
        .filter_map(|(mut r, hit)| {
            // the header and the commit bar sit outside the scrolled body
            if matches!(
                hit,
                ReviewHit::Refresh
                    | ReviewHit::Maximize
                    | ReviewHit::Base
                    | ReviewHit::CommitField
                    | ReviewHit::Commit
                    | ReviewHit::Push
            ) {
                return Some((r, hit));
            }
            let top = r[1].max(body_top);
            let bottom = (r[1] + r[3]).min(body_bottom);
            (bottom > top).then(|| {
                r[1] = top;
                r[3] = bottom - top;
                (r, hit)
            })
        })
        .collect();
    let mut hits = hits;

    if state.content_h > state.view_h {
        let track = state.view_h - 12.0;
        let thumb = (track * state.view_h / state.content_h).max(24.0);
        let max = state.content_h - state.view_h;
        let ty = body_top + 6.0 + (track - thumb) * (state.scroll / max);
        layer.rect(
            [x + main_w - 6.0, ty],
            [3.0, thumb],
            Color::WHITE.with_alpha(0.18),
            1.5,
        );
    }
    if side {
        file_list(
            layer,
            engine,
            theme,
            state,
            &review,
            [x + main_w, body_top, SIDEBAR_W, body_bottom - body_top],
            &mut hits,
        );
    }
    hits
}

/// Message lines the commit field shows before it stops growing.
const COMMIT_LINES: usize = 4;
const COMMIT_LINE_H: f32 = 17.0;

/// Height of the commit bar: the message field (one to four lines) and the
/// button row.
fn commit_bar_h(state: &ReviewPanelState) -> f32 {
    let lines = state.commit_msg.split('\n').count().clamp(1, COMMIT_LINES);
    8.0 + lines as f32 * COMMIT_LINE_H + 10.0 + 6.0 + 24.0 + 8.0
}

/// The commit bar at the panel's foot: the message field, Commit and Push,
/// and why Commit is off (or what is running).
fn commit_bar(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &ReviewPanelState,
    rect: [f32; 4],
    hits: &mut Vec<([f32; 4], ReviewHit)>,
) {
    let [x, y, w, h] = rect;
    layer.rect([x + 1.0, y], [w - 1.0, h], theme.bg, 0.0);
    layer.hline(x + 1.0, y, w - 1.0, 1.0, theme.border);
    let lines: Vec<&str> = state.commit_msg.split('\n').collect();
    let shown = lines.len().clamp(1, COMMIT_LINES);
    let field = [
        x + 10.0,
        y + 8.0,
        w - 20.0,
        shown as f32 * COMMIT_LINE_H + 10.0,
    ];
    layer.bordered(
        [field[0], field[1]],
        [field[2], field[3]],
        theme.bg_chrome,
        if state.commit_focus {
            theme.accent
        } else {
            theme.border
        },
        5.0,
        1.0,
    );
    hits.push((field, ReviewHit::CommitField));
    let bounds = TextBounds {
        left: (field[0] + 1.0) as i32,
        top: field[1] as i32,
        right: (field[0] + field[2] - 4.0) as i32,
        bottom: (field[1] + field[3]) as i32,
    };
    if state.commit_msg.is_empty() {
        // ⌘⏎, and Ctrl+Enter too (`handle_commit_key`): the one this
        // platform's keyboard has
        let commit_keys = rift_core::keybindings::primary_hint("enter");
        layer.text_clipped(
            text(
                engine,
                &format!("Commit message  ·  {commit_keys} commits"),
                12.0,
                FONT_SANS,
                Weight::NORMAL,
            ),
            field[0] + 8.0,
            field[1] + 5.5,
            theme.text_muted,
            bounds,
        );
    }
    // the last lines, so the caret's line is always in view
    let first = lines.len().saturating_sub(COMMIT_LINES);
    let mut caret = (field[0] + 8.0, field[1] + 5.0);
    for (i, line) in lines[first..].iter().enumerate() {
        let ly = field[1] + 5.0 + i as f32 * COMMIT_LINE_H;
        if !line.is_empty() {
            layer.text_clipped(
                text(engine, line, 12.0, FONT_SANS, Weight::NORMAL),
                field[0] + 8.0,
                ly,
                theme.text_bright,
                bounds,
            );
        }
        let lw = engine.measure(line, 12.0, FONT_SANS, Weight::NORMAL);
        caret = ((field[0] + 8.0 + lw).min(field[0] + field[2] - 6.0), ly);
    }
    if state.commit_focus {
        layer.rect(
            [caret.0 + 1.0, caret.1 + 1.0],
            [1.5, 15.0],
            theme.accent,
            0.0,
        );
    }

    // buttons, right-aligned: Commit at the right edge, Push left of it
    let by = field[1] + field[3] + 6.0;
    let reason = state.commit_reason();
    let commit_on = reason.is_none() && state.busy.is_none();
    let r = button(
        layer,
        engine,
        theme,
        "Commit",
        x + w - 10.0,
        by,
        24.0,
        if commit_on {
            theme.text_bright
        } else {
            theme.text_muted
        },
    );
    if commit_on {
        hits.push((r, ReviewHit::Commit));
    }
    let pushing = state.busy == Some(GitBusy::Push);
    let push_on = state.busy.is_none();
    let p = button(
        layer,
        engine,
        theme,
        if pushing { "Pushing…" } else { "Push" },
        r[0] - 6.0,
        by,
        24.0,
        if push_on {
            theme.text_ui
        } else {
            theme.text_muted
        },
    );
    if push_on {
        hits.push((p, ReviewHit::Push));
    }
    // what runs, else why Commit is off
    let mut tx = x + 14.0;
    if state.busy.is_some() {
        super::glyphs::draw_spinner(layer, [tx + 6.0, by + 12.0], 12.0, theme.text_dim);
        tx += 18.0;
    }
    let note = match state.busy {
        Some(GitBusy::Push) => Some("Pushing…"),
        _ => reason,
    };
    if let Some(note) = note {
        let room = p[0] - 10.0 - tx;
        if room > 30.0 {
            let note = super::palette::elide_to(engine, note, 11.5, FONT_SANS, room);
            layer.text(
                text(engine, &note, 11.5, FONT_SANS, Weight::NORMAL),
                tx,
                by + 4.5,
                theme.text_muted,
            );
        }
    }
}

/// The maximized panel's file list: every changed file (staged first) with
/// its folder and counts; a click shows the file.
fn file_list(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &mut ReviewPanelState,
    review: &Review,
    rect: [f32; 4],
    hits: &mut Vec<([f32; 4], ReviewHit)>,
) {
    let [x, y, w, h] = rect;
    state.side_rect = rect;
    layer.rect([x, y], [1.0, h], theme.border, 0.0);
    let files: Vec<(bool, &FileDiff)> = review
        .staged
        .iter()
        .map(|f| (true, f))
        .chain(review.unstaged.iter().map(|f| (false, f)))
        .collect();
    let content = files.len() as f32 * SIDE_ROW_H + 16.0;
    state.side_max = (content - h).max(0.0);
    state.side_scroll = state.side_scroll.clamp(0.0, state.side_max);
    let bounds = TextBounds {
        left: (x + 1.0) as i32,
        top: y as i32,
        right: (x + w - 8.0) as i32,
        bottom: (y + h) as i32,
    };
    let mono = mono_family();
    layer.set_clip(Some(([x + 1.0, y, w - 1.0, h], 0.0)));
    for (i, (staged, f)) in files.iter().enumerate() {
        let ry = y + 8.0 + i as f32 * SIDE_ROW_H - state.side_scroll;
        if ry + SIDE_ROW_H <= y || ry >= y + h {
            continue;
        }
        let row = [x + 8.0, ry, w - 16.0, SIDE_ROW_H];
        state.hover_rows.push(row);
        let lit = state.pointer.is_some_and(|(px, py)| {
            px >= row[0] && px < row[0] + row[2] && py >= row[1] && py < row[1] + row[3]
        });
        if lit {
            layer.rect(
                [row[0], row[1]],
                [row[2], row[3]],
                theme.fg.with_alpha(0.06),
                4.0,
            );
        }
        // counts at the right: only the sides that changed
        let mut right = row[0] + row[2] - 8.0;
        let mut counts = Vec::new();
        if f.removed > 0 {
            counts.push((format!("−{}", f.removed), theme.error));
        }
        if f.added > 0 || f.removed == 0 {
            counts.push((format!("+{}", f.added), theme.success));
        }
        for (s, color) in &counts {
            let cw = engine.measure(s, 11.0, mono, Weight::NORMAL);
            right -= cw;
            layer.text_clipped(
                text(engine, s, 11.0, mono, Weight::NORMAL),
                right,
                ry + 6.0,
                *color,
                bounds,
            );
            right -= 6.0;
        }
        let (dir, name) = match f.path.rsplit_once('/') {
            Some((d, n)) => (d, n),
            None => ("", f.path.as_str()),
        };
        let name_x = row[0] + 8.0;
        let room = (right - 6.0 - name_x).max(10.0);
        let name_w = engine.measure(name, 12.0, FONT_SANS, Weight::NORMAL);
        let shown = if name_w > room {
            super::palette::elide_to(engine, name, 12.0, FONT_SANS, room)
        } else {
            name.to_string()
        };
        let clip = TextBounds {
            right: (right - 6.0) as i32,
            ..bounds
        };
        layer.text_clipped(
            text(engine, &shown, 12.0, FONT_SANS, Weight::NORMAL),
            name_x,
            ry + 5.0,
            theme.text_bright,
            clip,
        );
        let mut after = name_x + name_w.min(room) + 6.0;
        if *staged {
            layer.text_clipped(
                text(engine, "staged", 11.0, FONT_SANS, Weight::NORMAL),
                after,
                ry + 6.0,
                theme.success,
                clip,
            );
            after += engine.measure("staged", 11.0, FONT_SANS, Weight::NORMAL) + 6.0;
        }
        if !dir.is_empty() && after < right - 20.0 {
            layer.text_clipped(
                text(engine, dir, 11.0, FONT_SANS, Weight::NORMAL),
                after,
                ry + 6.0,
                theme.text_muted,
                clip,
            );
        }
        let top = ry.max(y);
        let bottom = (ry + SIDE_ROW_H).min(y + h);
        hits.push((
            [row[0], top, row[2], bottom - top],
            ReviewHit::Jump(*staged, f.path.clone()),
        ));
    }
    layer.set_clip(None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rift_core::review::{Hunk, Line};

    fn hunk(new_start: u32, new_lines: u32) -> Hunk {
        Hunk {
            id: HunkId {
                old_start: new_start,
                old_lines: new_lines,
                new_start,
                new_lines,
                content_hash: 0,
            },
            header: String::new(),
            lines: vec![Line {
                kind: LineKind::Context,
                old_no: Some(new_start),
                new_no: Some(new_start),
                text: String::new(),
                emph: None,
            }],
            partial: false,
        }
    }

    fn file(hunks: Vec<Hunk>, lines: usize) -> FileDiff {
        FileDiff {
            path: "a.rs".into(),
            old_path: None,
            status: 'M',
            added: 0,
            removed: 0,
            binary: false,
            truncated: false,
            hunks,
            source: Some((1..=lines).map(|n| format!("line {n}")).collect()),
        }
    }

    fn keyed(staged: Vec<FileDiff>, unstaged: Vec<FileDiff>) -> ReviewPanelState {
        ReviewPanelState {
            load: Load::Ready(Arc::new(Review {
                staged,
                unstaged,
                ..Review::default()
            })),
            ..ReviewPanelState::default()
        }
    }

    #[test]
    fn keyboard_walks_sections_files_and_hunks_and_acts() {
        let mut state = keyed(vec![], vec![file(vec![hunk(5, 6), hunk(30, 6)], 50)]);
        state.take_keys();
        assert_eq!(state.focus, Some(ReviewStop::Section(false)));
        // a collapsed file hides its hunks from the walk
        state.move_focus(1);
        assert_eq!(state.focus, Some(ReviewStop::File(false, "a.rs".into())));
        state.move_focus(1);
        assert_eq!(state.focus, Some(ReviewStop::File(false, "a.rs".into())));
        state.focus_right();
        state.move_focus(1);
        let first = ReviewStop::Hunk(false, "a.rs".into(), hunk(5, 6).id);
        assert_eq!(state.focus, Some(first));
        assert_eq!(
            state.focus_hit(StopAct::Stage),
            Some(ReviewHit::Stage("a.rs".into(), Some(hunk(5, 6).id)))
        );
        assert_eq!(
            state.focus_hit(StopAct::Revert),
            Some(ReviewHit::Revert("a.rs".into(), Some(hunk(5, 6).id)))
        );
        // nothing to unstage in the unstaged section
        assert_eq!(state.focus_hit(StopAct::Unstage), None);
        // ← steps out to the file, then folds it
        state.focus_left();
        assert_eq!(state.focus, Some(ReviewStop::File(false, "a.rs".into())));
        state.focus_left();
        assert!(state.expanded.is_empty());
        // End and Home reach the ends
        state.move_focus(1_000_000);
        assert_eq!(state.focus, Some(ReviewStop::File(false, "a.rs".into())));
        state.move_focus(-1_000_000);
        assert_eq!(state.focus_hit(StopAct::Stage), Some(ReviewHit::StageAll));
        state.drop_keys();
        assert_eq!(state.focus, None);
    }

    #[test]
    fn focus_follows_a_staged_hunk_to_the_next_one() {
        let two = file(vec![hunk(5, 6), hunk(30, 6)], 50);
        let mut state = keyed(vec![], vec![two]);
        state.expanded.insert((false, "a.rs".into()));
        state.take_keys();
        state.move_focus(3); // section, file, hunk 1, hunk 2
        let second = ReviewStop::Hunk(false, "a.rs".into(), hunk(30, 6).id);
        assert_eq!(state.focus, Some(second));
        // hunk 2 got staged: it leaves the list, hunk 1 is the file's nearest
        state.load = Load::Ready(Arc::new(Review {
            staged: vec![file(vec![hunk(30, 6)], 50)],
            unstaged: vec![file(vec![hunk(5, 6)], 50)],
            ..Review::default()
        }));
        state.sync_focus();
        assert_eq!(
            state.focus,
            Some(ReviewStop::Hunk(false, "a.rs".into(), hunk(5, 6).id))
        );
    }

    #[test]
    fn against_a_branch_writes_are_off_and_send_stays() {
        let mut state = keyed(vec![], vec![file(vec![hunk(5, 6)], 50)]);
        state.expanded.insert((false, "a.rs".into()));
        let id = hunk(5, 6).id;
        state.take_keys();
        state.move_focus(2);
        assert_eq!(
            state.focus_hit(StopAct::Stage),
            Some(ReviewHit::Stage("a.rs".into(), Some(id)))
        );
        state.load = Load::Ready(Arc::new(Review {
            unstaged: vec![file(vec![hunk(5, 6)], 50)],
            base: Some(rift_core::review::BaseInfo {
                name: "main".into(),
                merge_base: "0123456789".into(),
            }),
            ..Review::default()
        }));
        assert!(state.read_only());
        for act in [StopAct::Stage, StopAct::Unstage, StopAct::Revert] {
            assert_eq!(state.focus_hit(act), None, "{act:?}");
        }
        assert_eq!(
            state.focus_hit(StopAct::Send),
            Some(ReviewHit::Send(false, "a.rs".into(), Some(id)))
        );
        let f = &state.model().unwrap().unstaged[0];
        let kinds: Vec<Icon> = hunk_actions(false, f, 0, true)
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        assert_eq!(kinds, [Icon::Copy]);
        assert_eq!(
            state.commit_reason(),
            Some("Commit from Uncommitted, which shows what is staged")
        );
    }

    #[test]
    fn commit_needs_staged_changes_and_a_message() {
        let mut state = keyed(vec![], vec![file(vec![hunk(5, 6)], 50)]);
        assert_eq!(state.commit_reason(), Some("Nothing staged"));
        state.load = Load::Ready(Arc::new(Review {
            staged: vec![file(vec![hunk(5, 6)], 50)],
            ..Review::default()
        }));
        state.commit_msg = "  \n".into();
        assert_eq!(state.commit_reason(), Some("Write a commit message"));
        state.commit_msg = "fix".into();
        assert_eq!(state.commit_reason(), None);
        state.busy = Some(GitBusy::Commit);
        assert_eq!(state.commit_reason(), Some("Committing…"));
    }

    #[test]
    fn focus_waits_out_a_reload() {
        let mut state = keyed(vec![], vec![file(vec![hunk(5, 6)], 50)]);
        state.take_keys();
        state.move_focus(1);
        let at = state.focus.clone();
        state.load = Load::Loading;
        state.sync_focus();
        assert_eq!(state.focus, at);
    }

    #[test]
    fn a_cut_hunk_has_no_hunk_actions() {
        let mut cut = hunk(5, 6);
        cut.partial = true;
        let mut state = keyed(vec![], vec![file(vec![cut.clone()], 50)]);
        state.expanded.insert((false, "a.rs".into()));
        state.take_keys();
        state.move_focus(2);
        assert_eq!(
            state.focus,
            Some(ReviewStop::Hunk(false, "a.rs".into(), cut.id))
        );
        assert_eq!(state.focus_hit(StopAct::Stage), None);
        assert_eq!(state.focus_hit(StopAct::Revert), None);
    }

    #[test]
    fn gaps_lie_between_hunks() {
        // lines 5–10 and 30–35 of 50
        let f = file(vec![hunk(5, 6), hunk(30, 6)], 50);
        assert_eq!(gap(&f, 0), Some((1, 5)));
        assert_eq!(gap(&f, 1), Some((11, 30)));
        assert_eq!(gap(&f, 2), Some((36, 51)));
        assert_eq!(gap(&f, 3), None);
        // a hunk at line 1 has nothing above it; a cut diff no tail
        let mut f = file(vec![hunk(1, 4)], 4);
        assert_eq!((gap(&f, 0), gap(&f, 1)), (None, None));
        f.truncated = true;
        f.source.as_mut().unwrap().push("more".into());
        assert_eq!(gap(&f, 1), None);
        // without the new side there is nothing to open
        f.source = None;
        assert_eq!(gap(&f, 0), None);
    }

    #[test]
    fn reveal_opens_short_gaps_whole_and_long_ones_in_steps() {
        let f = file(vec![hunk(5, 6), hunk(70, 6)], 80);
        let mut state = ReviewPanelState {
            load: Load::Ready(Arc::new(Review {
                unstaged: vec![f],
                ..Review::default()
            })),
            ..ReviewPanelState::default()
        };
        let model = state.model().unwrap().clone();
        let f = &model.unstaged[0];
        // gap 0 (lines 1–4) opens whole, upward from the first hunk
        state.reveal(false, "a.rs", 0);
        assert_eq!(gap_split(&state, false, f, 0), (1..5, 0));
        // gap 1 (lines 11–69, 59 lines) opens 20 at a time from the top
        state.reveal(false, "a.rs", 1);
        assert_eq!(gap_split(&state, false, f, 1), (11..31, 39));
        state.reveal(false, "a.rs", 1);
        assert_eq!(gap_split(&state, false, f, 1), (11..70, 0));
        // the row order: open lines of gap 1 join hunk 0, then the gap row
        let mut out = Vec::new();
        file_rows(&state, false, f, &mut out);
        let shape: Vec<String> = out
            .iter()
            .map(|r| match r {
                RowKind::Hunk(_, _, hi, hidden) => format!("hunk{hi}:{hidden}"),
                RowKind::Source(_, _, start, n) => format!("src{start}+{n}"),
                RowKind::Lines(_, _, hi, _) => format!("lines{hi}"),
                RowKind::Tail(_, _, hidden) => format!("tail:{hidden}"),
                _ => "other".into(),
            })
            .collect();
        assert_eq!(
            shape,
            [
                "hunk0:0", "src1+4", "lines0", "src11+59", "hunk1:0", "lines1", "tail:5"
            ]
        );
    }

    #[test]
    fn hunk_patch_is_a_unified_diff_and_jump_scrolls_to_the_card() {
        let mut f = file(vec![hunk(5, 1)], 9);
        f.hunks[0].header = "@@ -5 +5 @@ fn main".into();
        f.hunks[0].lines = vec![
            Line {
                kind: LineKind::Removed,
                old_no: Some(5),
                new_no: None,
                text: "old".into(),
                emph: None,
            },
            Line {
                kind: LineKind::Added,
                old_no: None,
                new_no: Some(5),
                text: "new".into(),
                emph: None,
            },
        ];
        let id = f.hunks[0].id;
        assert_eq!(
            hunk_patch(&f, &id).as_deref(),
            Some("--- a/a.rs\n+++ b/a.rs\n@@ -5 +5 @@ fn main\n-old\n+new\n")
        );
        // the toolbar: stage, revert, copy (unstaged); unstage, copy (staged)
        let kinds = |staged| -> Vec<Icon> {
            hunk_actions(staged, &f, 0, false)
                .into_iter()
                .map(|(i, _)| i)
                .collect()
        };
        assert_eq!(kinds(false), [Icon::Plus, Icon::Revert, Icon::Copy]);
        assert_eq!(kinds(true), [Icon::Minus, Icon::Copy]);
        let mut second = f.clone();
        second.path = "b.rs".into();
        let mut state = ReviewPanelState {
            load: Load::Ready(Arc::new(Review {
                unstaged: vec![f, second],
                ..Review::default()
            })),
            ..ReviewPanelState::default()
        };
        state.collapsed_sections.insert(false);
        state.jump(false, "b.rs");
        // the section opens; b.rs sits under its header and a.rs's card
        assert!(state.collapsed_sections.is_empty());
        assert!(state.expanded.contains(&(false, "b.rs".into())));
        assert_eq!(state.scroll, SECTION_H + FILE_H + CARD_GAP + 2.0);
    }

    #[test]
    fn code_spans_expand_tabs_cut_long_lines_and_map_changed_words() {
        let theme = Theme::default();
        let (spans, emph) = code_spans(&theme, Lang::Plain, "\tab cd", Some(&(4..6)), 80);
        let joined: String = spans.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!((joined.as_str(), emph), ("    ab cd", Some((7, 9))));
        let (spans, emph) = code_spans(&theme, Lang::Plain, "abcdefghij", Some(&(8..10)), 6);
        let joined: String = spans.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!((joined.as_str(), emph), ("abcde…", Some((5, 5))));
    }
}
