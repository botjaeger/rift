//! Code review: a right panel listing the repository's staged and unstaged
//! changes (`shift_core::review`). Files expand into hunks and lines; each
//! file and hunk can be staged, unstaged or reverted. Revert goes through
//! the confirm dialog (it discards work). The model loads on a background
//! thread; the panel draws only the rows on screen.

use std::collections::HashSet;
use std::sync::Arc;

use shift_core::review::{FileDiff, HunkId, LineKind, Review};
use shift_core::{Color, Theme};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::views::explorer::status_color;

pub const REVIEW_W: f32 = 460.0;
pub const HEADER_H: f32 = 38.0;
const SECTION_H: f32 = 30.0;
const FILE_H: f32 = 28.0;
const HUNK_H: f32 = 24.0;
const LINE_H: f32 = 18.0;

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
}

/// Shown in place of a partial hunk's Stage/Unstage/Revert buttons.
const PARTIAL_HUNK_NOTE: &str = "Too long · use file actions";

/// What a click in the panel does.
#[derive(Debug, Clone, PartialEq)]
pub enum ReviewHit {
    Refresh,
    ToggleSection(bool),
    ToggleFile(bool, String),
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
    Hunk(bool, &'a FileDiff, usize),
    Line(&'a FileDiff, usize, usize),
    /// A hunk's n lines, before `visible` expands the ones on screen.
    Lines(&'a FileDiff, usize, usize),
    Note(String),
}

impl RowKind<'_> {
    fn height(&self) -> f32 {
        match self {
            RowKind::Section(..) => SECTION_H,
            RowKind::File(..) => FILE_H,
            RowKind::Hunk(..) => HUNK_H,
            RowKind::Line(..) | RowKind::Note(_) => LINE_H + 4.0,
            RowKind::Lines(_, _, n) => *n as f32 * (LINE_H + 4.0),
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
            if !state.expanded.contains(&(staged, f.path.clone())) {
                continue;
            }
            if f.binary {
                out.push(RowKind::Note("Binary file".into()));
                continue;
            }
            for (hi, h) in f.hunks.iter().enumerate() {
                out.push(RowKind::Hunk(staged, f, hi));
                // lines are expanded lazily by `visible`
                out.push(RowKind::Lines(f, hi, h.lines.len()));
            }
            if f.truncated {
                out.push(RowKind::Note(format!(
                    "Diff cut at {} lines",
                    shift_core::review::MAX_LINES_PER_FILE
                )));
            }
        }
    }
    out
}

/// The rows intersecting [top, bottom) (content coordinates) with their
/// y, and the total content height. A hunk's lines are counted, not
/// materialized, unless on screen: a 20 000-line diff costs no more per
/// frame than a short one.
fn visible<'a>(all: Vec<RowKind<'a>>, top: f32, bottom: f32) -> (Vec<(f32, RowKind<'a>)>, f32) {
    let mut out = Vec::new();
    let mut y = 0.0;
    for row in all {
        if let RowKind::Lines(f, hi, n) = row {
            let lh = LINE_H + 4.0;
            let block = n as f32 * lh;
            if y + block > top && y < bottom {
                let first = ((top - y) / lh).floor().max(0.0) as usize;
                let last = (((bottom - y) / lh).ceil().max(0.0) as usize).min(n);
                for li in first..last {
                    out.push((y + li as f32 * lh, RowKind::Line(f, hi, li)));
                }
            }
            y += block;
            continue;
        }
        let h = row.height();
        if y + h > top && y < bottom {
            out.push((y, row));
        }
        y += h;
    }
    (out, y)
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
        5.0,
    );
    layer.text(
        text(engine, label, 11.0, FONT_SANS, Weight::MEDIUM),
        r[0] + 7.0,
        r[1] + 3.0,
        color,
    );
    r
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
    layer.rect([x, y], [w, h], theme.bg_chrome, 0.0);
    layer.rect([x, y], [1.0, h], theme.border, 0.0);

    // ── header ──
    layer.text(
        text(engine, "CODE REVIEW", 11.0, FONT_SANS, Weight::SEMIBOLD),
        x + 16.0,
        y + 12.0,
        theme.text_dim,
    );
    let refresh = [x + w - 34.0, y + 7.0, 24.0, 24.0];
    layer.text(
        text(engine, "↻", 13.0, FONT_SANS, Weight::NORMAL),
        refresh[0] + 6.0,
        refresh[1] + 3.0,
        theme.text_muted,
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
    let (mut shown, total) = visible(rows(state, &review), lo, hi);
    state.content_h = total + 12.0;
    let before = state.scroll;
    state.scroll_by(0.0);
    if state.scroll != before {
        // clamped (the diff shrank): lay out the corrected window
        let (lo, hi) = window(state);
        shown = visible(rows(state, &review), lo, hi).0;
    }
    let bounds = TextBounds {
        left: (x + 1.0) as i32,
        top: body_top as i32,
        right: (x + w) as i32,
        bottom: body_bottom as i32,
    };
    let inside = |top: f32, rh: f32| top + rh > body_top && top < body_bottom;
    // quads can't be clipped: a band is drawn only fully inside the body
    let fully = |top: f32, rh: f32| top >= body_top && top + rh <= body_bottom;
    let mono = mono_family();
    for (cy, row) in &shown {
        let ry = body_top + 4.0 + cy - state.scroll;
        let rh = row.height();
        if !inside(ry, rh) {
            continue;
        }
        match row {
            RowKind::Section(staged, n) => {
                let open = !state.collapsed_sections.contains(staged);
                let title = if *staged { "STAGED" } else { "CHANGES" };
                layer.text_clipped(
                    text(
                        engine,
                        if open { "▾" } else { "▸" },
                        11.0,
                        mono_family(),
                        Weight::NORMAL,
                    ),
                    x + 14.0,
                    ry + 9.0,
                    theme.text_muted,
                    bounds,
                );
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
                if fully(ry, rh) && open {
                    layer.rect(
                        [x + 1.0, ry],
                        [w - 1.0, rh],
                        Color::WHITE.with_alpha(0.03),
                        0.0,
                    );
                }
                layer.text_clipped(
                    text(
                        engine,
                        if open { "▾" } else { "▸" },
                        11.0,
                        mono_family(),
                        Weight::NORMAL,
                    ),
                    x + 14.0,
                    ry + 8.0,
                    theme.text_muted,
                    bounds,
                );
                let letter = if f.status == '?' { 'U' } else { f.status };
                layer.text_clipped(
                    text(engine, &letter.to_string(), 11.0, mono, Weight::SEMIBOLD),
                    x + 28.0,
                    ry + 7.0,
                    status_color(theme, f.status),
                    bounds,
                );
                let (dir, name) = match f.path.rsplit_once('/') {
                    Some((d, n)) => (d, n),
                    None => ("", f.path.as_str()),
                };
                let name_w = engine.measure(name, 12.5, FONT_SANS, Weight::MEDIUM);
                // buttons first so the counts sit left of them
                let mut right = x + w - 12.0;
                let mut actions = Vec::new();
                if fully(ry, rh) {
                    let mut add = |label: &str, hit: ReviewHit, color: Color| {
                        let r = button(layer, engine, label, right, ry, rh, color);
                        right = r[0] - 4.0;
                        actions.push((r, hit));
                    };
                    if *staged {
                        add("−", ReviewHit::Unstage(f.path.clone(), None), theme.text_ui);
                    } else {
                        add("+", ReviewHit::Stage(f.path.clone(), None), theme.text_ui);
                        add("↺", ReviewHit::Revert(f.path.clone(), None), theme.error);
                    }
                    if f.status != 'D' {
                        add("↗", ReviewHit::Open(f.path.clone()), theme.text_dim);
                    }
                }
                let counts = format!("+{} −{}", f.added, f.removed);
                let cw = engine.measure(&counts, 11.0, mono, Weight::NORMAL);
                let cx = right - 6.0 - cw;
                layer.text_clipped(
                    text(engine, &counts, 11.0, mono, Weight::NORMAL),
                    cx,
                    ry + 7.5,
                    theme.text_muted,
                    bounds,
                );
                // a long name stops before the counts, ending in `…`
                let name_room = (cx - 8.0 - (x + 44.0)).max(10.0);
                let name_shown = if name_w > name_room {
                    super::palette::elide_to(engine, name, 12.5, FONT_SANS, name_room)
                } else {
                    name.to_string()
                };
                layer.text_clipped(
                    text(engine, &name_shown, 12.5, FONT_SANS, Weight::MEDIUM),
                    x + 44.0,
                    ry + 6.0,
                    theme.text_bright,
                    TextBounds {
                        right: (cx - 8.0) as i32,
                        ..bounds
                    },
                );
                if !dir.is_empty() && x + 44.0 + name_w + 8.0 < cx - 20.0 {
                    let dir_bounds = TextBounds {
                        right: (cx - 10.0) as i32,
                        ..bounds
                    };
                    layer.text_clipped(
                        text(engine, dir, 11.5, FONT_SANS, Weight::NORMAL),
                        x + 44.0 + name_w + 8.0,
                        ry + 7.0,
                        theme.text_muted,
                        dir_bounds,
                    );
                }
                // the row first, then its actions (screen reader order)
                hits.push((
                    [x, ry, (right - x).max(0.0), rh],
                    ReviewHit::ToggleFile(*staged, f.path.clone()),
                ));
                hits.extend(actions);
            }
            RowKind::Hunk(staged, f, hi) => {
                let hunk = &f.hunks[*hi];
                if fully(ry, rh) {
                    layer.rect(
                        [x + 1.0, ry],
                        [w - 1.0, rh],
                        theme.accent.with_alpha(0.07),
                        0.0,
                    );
                }
                let mut right = x + w - 12.0;
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
                let header_bounds = TextBounds {
                    right: (right - 6.0) as i32,
                    ..bounds
                };
                let header_room = (right - 6.0 - (x + 14.0)).max(10.0);
                let header_shown =
                    super::palette::elide_to(engine, &hunk.header, 11.0, mono, header_room);
                layer.text_clipped(
                    text(engine, &header_shown, 11.0, mono, Weight::NORMAL),
                    x + 14.0,
                    ry + 5.0,
                    theme.accent,
                    header_bounds,
                );
            }
            RowKind::Line(f, hi, li) => {
                let line = &f.hunks[*hi].lines[*li];
                let (tint, sign, color) = match line.kind {
                    LineKind::Added => (Some(theme.success), "+", theme.text_ui),
                    LineKind::Removed => (Some(theme.error), "−", theme.text_ui),
                    LineKind::Context => (None, " ", theme.text_dim),
                };
                if let Some(t) = tint
                    && fully(ry, rh)
                {
                    layer.rect([x + 1.0, ry], [w - 1.0, rh], t.with_alpha(0.12), 0.0);
                }
                // one mono buffer per line (gutters, sign, text): a screen
                // of diff is ~35 text areas instead of ~140
                let num = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
                let gutter = format!("{:>5} {:>5} ", num(line.old_no), num(line.new_no));
                // tabs as 4 spaces: glyphon's tab stops ignore the gutter
                let body = line.text.replace('\t', "    ");
                // a long line ends in `…` instead of being cut mid-glyph
                let adv = engine.measure("0", 12.0, mono, Weight::NORMAL).max(1.0);
                let max_chars = ((w - 4.0 - 10.0) / adv) as usize;
                let body_room = max_chars.saturating_sub(gutter.chars().count() + 2);
                let body = if body.chars().count() > body_room {
                    let keep = body_room.saturating_sub(1);
                    body.chars().take(keep).collect::<String>() + "…"
                } else {
                    body
                };
                let spans = [
                    (gutter.as_str(), theme.text_muted),
                    (sign, tint.unwrap_or(theme.text_muted)),
                    (" ", color),
                    (body.as_str(), color),
                ];
                layer.text_clipped(
                    rich(engine, &spans, 12.0, mono),
                    x + 4.0,
                    ry + 3.0,
                    color,
                    bounds,
                );
            }
            RowKind::Lines(..) => {} // expanded by `visible`
            RowKind::Note(s) => {
                layer.text_clipped(
                    text(engine, s, 11.5, FONT_SANS, Weight::NORMAL),
                    x + 44.0,
                    ry + 4.0,
                    theme.text_muted,
                    bounds,
                );
            }
        }
    }
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
