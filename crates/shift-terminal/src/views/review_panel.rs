//! Code review: a right panel listing the repository's staged and unstaged
//! changes (`shift_core::review`). Each file is a card that expands into its
//! hunks: one column of line numbers, syntax-colored code, tinted added and
//! removed rows with the changed words highlighted, and "N unmodified lines"
//! rows between hunks that open the file's unchanged lines (Warp's diff
//! view). Each file and hunk can be staged, unstaged or reverted. Revert
//! goes through the confirm dialog (it discards work). The model loads on a
//! background thread; the panel draws only the rows on screen.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use shift_core::code_highlight::{self, Kind, Lang};
use shift_core::review::{FileDiff, HunkId, LineKind, Review};
use shift_core::{Color, Theme};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

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
}

impl ReviewPanelState {
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
    StageAll,
    UnstageAll,
}

enum RowKind<'a> {
    Section(bool, usize),
    File(bool, &'a FileDiff),
    /// A hunk's actions, on the row that counts the unmodified lines still
    /// hidden above it.
    Hunk(bool, &'a FileDiff, usize, u32),
    Line(&'a FileDiff, usize, usize),
    /// A hunk's n lines, before `visible` expands the ones on screen.
    Lines(&'a FileDiff, usize, usize),
    /// Open unmodified lines: new-side line number and count, expanded by
    /// `visible` like `Lines`.
    Source(&'a FileDiff, u32, usize),
    SourceLine(&'a FileDiff, u32),
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
            RowKind::Lines(_, _, n) | RowKind::Source(_, _, n) => *n as f32 * ROW_H,
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
            .map(|(s, c)| (*s, base.clone().color(shift_renderer::text_color(*c)))),
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
            out.push(RowKind::Source(f, r.start, r.len()));
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
        out.push(RowKind::Lines(f, hi, h.lines.len()));
    }
    let (shown, hidden) = gap_split(state, staged, f, f.hunks.len());
    source(out, shown);
    if hidden > 0 {
        out.push(RowKind::Tail(staged, f, hidden));
    }
    if f.truncated {
        out.push(RowKind::Note(format!(
            "Diff cut at {} lines",
            shift_core::review::MAX_LINES_PER_FILE
        )));
    }
}

/// What `visible` lays out: rows on screen with their y, the file cards
/// crossing the window as (top, bottom), and the total content height.
struct Layout<'a> {
    rows: Vec<(f32, RowKind<'a>)>,
    cards: Vec<(f32, f32)>,
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
    let mut card_top = 0.0;
    for row in all {
        let lazy = match row {
            RowKind::Lines(_, _, n) | RowKind::Source(_, _, n) => Some(n),
            _ => None,
        };
        if let Some(n) = lazy {
            let block = n as f32 * ROW_H;
            if y + block > top && y < bottom {
                let first = ((top - y) / ROW_H).floor().max(0.0) as usize;
                let last = (((bottom - y) / ROW_H).ceil().max(0.0) as usize).min(n);
                for i in first..last {
                    let one = match row {
                        RowKind::Lines(f, hi, _) => RowKind::Line(f, hi, i),
                        RowKind::Source(f, start, _) => RowKind::SourceLine(f, start + i as u32),
                        _ => unreachable!("lazy rows only"),
                    };
                    out.rows.push((y + i as f32 * ROW_H, one));
                }
            }
            y += block;
            continue;
        }
        match row {
            RowKind::File(..) => card_top = y,
            RowKind::CardEnd => {
                let card_bottom = y + 2.0;
                if card_bottom > top && card_top < bottom {
                    out.cards.push((card_top, card_bottom));
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
fn button(
    layer: &mut Layer,
    engine: &mut TextEngine,
    label: &str,
    right: f32,
    y: f32,
    h: f32,
    color: Color,
) -> [f32; 4] {
    let tw = engine.measure(label, 11.0, FONT_SANS, Weight::MEDIUM);
    let bw = tw + 14.0;
    let r = [right - bw, y + (h - 20.0) / 2.0, bw, 20.0];
    layer.rect(
        [r[0], r[1]],
        [r[2], r[3]],
        Color::WHITE.with_alpha(0.06),
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
/// line number, the code, the changed words.
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
    (cx, cw, ry): (f32, f32, f32),
    bounds: TextBounds,
) {
    let mono = mono_family();
    let adv = engine.measure("0", 12.0, mono, Weight::NORMAL).max(1.0);
    let num_w = number_digits(f) as f32 * adv + 18.0;
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
    let code_x = cx + num_w + 8.0;
    let max_cols = ((cx + cw - 8.0 - code_x) / adv).max(0.0) as usize;
    let lang = code_highlight::lang_for(Path::new(&f.path));
    let (spans, emph) = code_spans(theme, lang, line, emph, max_cols);
    if let (Some(t), Some((a, b))) = (tint, emph)
        && b > a
    {
        layer.rect(
            [code_x + a as f32 * adv - 1.0, ry + 2.0],
            [(b - a) as f32 * adv + 2.0, ROW_H - 4.0],
            t.with_alpha(0.32),
            2.0,
        );
    }
    let spans: Vec<(&str, Color)> = spans.iter().map(|(s, c)| (s.as_str(), *c)).collect();
    layer.text_clipped(
        rich(engine, &spans, 12.0, mono),
        code_x,
        ry + 3.0,
        theme.fg,
        bounds,
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
    let adv = engine
        .measure("0", 12.0, mono_family(), Weight::NORMAL)
        .max(1.0);
    let num_w = number_digits(f) as f32 * adv + 18.0;
    let ax = cx + num_w - 8.0 - adv;
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
    layer.rect([x, y], [w, h], theme.ground(theme.bg_chrome), 0.0);
    layer.rect([x, y], [1.0, h], theme.border, 0.0);

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
    if let Some(review) = state.model().cloned() {
        let title_w = engine.measure("CODE REVIEW", 11.0, FONT_SANS, Weight::SEMIBOLD);
        layer.text(
            text(engine, &review.branch, 11.5, mono_family(), Weight::NORMAL),
            x + 16.0 + title_w + 12.0,
            y + 11.5,
            theme.success,
        );
        let (add, del) = review
            .staged
            .iter()
            .chain(&review.unstaged)
            .fold((0, 0), |(a, d), f| (a + f.added, d + f.removed));
        let del_s = format!("−{del}");
        let add_s = format!("+{add}");
        let del_w = engine.measure(&del_s, 11.5, mono_family(), Weight::NORMAL);
        let add_w = engine.measure(&add_s, 11.5, mono_family(), Weight::NORMAL);
        let dx = refresh[0] - 10.0 - del_w;
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
    let body_bottom = y + h;
    state.view_h = body_bottom - body_top;

    let message = match &state.load {
        Load::Idle | Load::Loading => Some("Loading changes…".to_string()),
        Load::NotARepo => Some("Not a git repository".to_string()),
        Load::Failed(e) => Some(e.clone()),
        Load::Ready(r) if r.staged.is_empty() && r.unstaged.is_empty() => {
            Some("No changes — working tree clean".to_string())
        }
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
        right: (x + w) as i32,
        bottom: body_bottom as i32,
    };
    let inside = |top: f32, rh: f32| top + rh > body_top && top < body_bottom;
    // buttons are drawn (and clickable) only fully inside the body
    let fully = |top: f32, rh: f32| top >= body_top && top + rh <= body_bottom;
    let mono = mono_family();
    let to_screen = |cy: f32| body_top + 4.0 + cy - state.scroll;
    // file cards span the panel minus the scrollbar lane
    let cx = x + CARD_X;
    let cw = w - 2.0 * CARD_X;
    let card_bounds = TextBounds {
        left: (cx + 1.0) as i32,
        right: (cx + cw - 1.0) as i32,
        ..bounds
    };
    // marks on rows half under the header are cut at the body's edge
    layer.set_clip(Some((
        [x + 1.0, body_top, w - 1.0, body_bottom - body_top],
        0.0,
    )));
    for &(top, bottom) in &layout.cards {
        let (t, b) = (to_screen(top), to_screen(bottom));
        layer.bordered(
            [cx, t],
            [cw, b - t],
            theme.ground(theme.bg),
            theme.border,
            CARD_R,
            1.0,
        );
    }
    for (cy, row) in &layout.rows {
        let ry = to_screen(*cy);
        let rh = row.height();
        if !inside(ry, rh) {
            continue;
        }
        match row {
            RowKind::Section(staged, n) => {
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
                hits.push(([x, ry, w - 150.0, rh], ReviewHit::ToggleSection(*staged)));
                if fully(ry, rh) {
                    let (label, hit) = if *staged {
                        ("Unstage all", ReviewHit::UnstageAll)
                    } else {
                        ("Stage all", ReviewHit::StageAll)
                    };
                    let r = button(layer, engine, label, x + w - 12.0, ry, rh, theme.text_ui);
                    hits.push((r, hit));
                }
            }
            RowKind::File(staged, f) => {
                let key = (*staged, f.path.clone());
                let open = state.expanded.contains(&key);
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
                    if *staged {
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
                        add(
                            Icon::Revert,
                            ReviewHit::Revert(f.path.clone(), None),
                            theme.error,
                        );
                    }
                    if f.status != 'D' {
                        add(Icon::Open, ReviewHit::Open(f.path.clone()), theme.text_dim);
                    }
                }
                // +added −removed in a pill, colored per side
                let add_s = format!("+{}", f.added);
                let del_s = format!("−{}", f.removed);
                let add_w = engine.measure(&add_s, 11.0, mono, Weight::NORMAL);
                let del_w = engine.measure(&del_s, 11.0, mono, Weight::NORMAL);
                let pill_w = add_w + del_w + 6.0 + 12.0;
                let pill_x = right - 6.0 - pill_w;
                layer.rect(
                    [pill_x, ry + (rh - 18.0) / 2.0],
                    [pill_w, 18.0],
                    theme.fg.with_alpha(0.06),
                    4.0,
                );
                layer.text_clipped(
                    text(engine, &add_s, 11.0, mono, Weight::NORMAL),
                    pill_x + 6.0,
                    ry + 9.0,
                    theme.success,
                    card_bounds,
                );
                layer.text_clipped(
                    text(engine, &del_s, 11.0, mono, Weight::NORMAL),
                    pill_x + 6.0 + add_w + 6.0,
                    ry + 9.0,
                    theme.error,
                    card_bounds,
                );
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
            }
            RowKind::Hunk(staged, f, hi, hidden) => {
                let hunk = &f.hunks[*hi];
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
                    let mut add = |label: &str, hit: ReviewHit, color: Color| {
                        let r = button(layer, engine, label, right, ry, rh, color);
                        right = r[0] - 4.0;
                        hits.push((r, hit));
                    };
                    if *staged {
                        add(
                            "Unstage",
                            ReviewHit::Unstage(f.path.clone(), Some(hunk.id)),
                            theme.text_ui,
                        );
                    } else {
                        add(
                            "Stage",
                            ReviewHit::Stage(f.path.clone(), Some(hunk.id)),
                            theme.text_ui,
                        );
                        add(
                            "Revert",
                            ReviewHit::Revert(f.path.clone(), Some(hunk.id)),
                            theme.error,
                        );
                    }
                }
                if *hidden > 0 {
                    hits.push((
                        [cx, ry, (right - cx).max(0.0), rh],
                        ReviewHit::Reveal(*staged, f.path.clone(), *hi),
                    ));
                } else {
                    // nothing hidden above: the hunk's own range
                    let adv = engine.measure("0", 12.0, mono, Weight::NORMAL).max(1.0);
                    let code_x = cx + number_digits(f) as f32 * adv + 26.0;
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
            RowKind::Line(f, hi, li) => {
                let line = &f.hunks[*hi].lines[*li];
                code_row(
                    layer,
                    engine,
                    theme,
                    f,
                    line.kind,
                    line.new_no,
                    &line.text,
                    line.emph.as_ref(),
                    (cx, cw, ry),
                    card_bounds,
                );
            }
            RowKind::SourceLine(f, n) => {
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
                    (cx, cw, ry),
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
    layer.set_clip(None);
    // hits outside the body (scrolled under the header) never fire
    // and a row half under the header only takes clicks on its visible part
    let hits: Vec<([f32; 4], ReviewHit)> = hits
        .into_iter()
        .filter_map(|(mut r, hit)| {
            if matches!(hit, ReviewHit::Refresh) {
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

    if state.content_h > state.view_h {
        let track = state.view_h - 12.0;
        let thumb = (track * state.view_h / state.content_h).max(24.0);
        let max = state.content_h - state.view_h;
        let ty = body_top + 6.0 + (track - thumb) * (state.scroll / max);
        layer.rect(
            [x + w - 6.0, ty],
            [3.0, thumb],
            Color::WHITE.with_alpha(0.18),
            1.5,
        );
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use shift_core::review::{Hunk, Line};

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
                RowKind::Source(_, start, n) => format!("src{start}+{n}"),
                RowKind::Lines(_, hi, _) => format!("lines{hi}"),
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
