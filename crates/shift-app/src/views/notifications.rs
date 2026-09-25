//! Notifications (Warp's inbox and toasts): the attention center's entries —
//! finished and failed commands out of view, program and coding-agent
//! notifications, password prompts, bells.
//!
//! - The popover drops from the inbox button in the tab bar, newest first.
//!   Each row shows a kind badge (or the agent's badge with a kind dot), the
//!   agent and the pane's git branch, the title (wrapped, at most four
//!   lines), its detail and a relative time. Chips filter to all, unread or
//!   errors; "Mark all as read" clears the unread state. A click on a row
//!   jumps to its pane and block.
//! - Toasts stack in the top-right corner while Shift is focused and the
//!   source pane is out of view: at most two, hovering keeps them, a click
//!   jumps there, ✕ dismisses.
//!
//! Popover rows are shaped once per width and entry list (`ListLayout`) and
//! reused while scrolling; each frame only draws what is in view.

use std::sync::Arc;

use shift_core::notifications::Agent;
use shift_core::{Color, Theme};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight, Wrap};
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

/// What a row reports (drives its badge).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Finished,
    Failed,
    /// Waiting on you: an agent asking for input or approval, a password
    /// prompt.
    Request,
    Notify,
    Bell,
}

/// One row, as the app hands it over (newest first).
#[derive(Debug, Clone)]
pub struct Entry {
    /// Stable id of the attention entry (click target).
    pub id: u64,
    pub kind: Kind,
    pub agent: Option<Agent>,
    pub branch: Option<String>,
    pub title: String,
    pub detail: String,
    /// Seconds since it happened.
    pub age_secs: u64,
    pub unread: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Filter {
    #[default]
    All,
    Unread,
    Errors,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::All, Filter::Unread, Filter::Errors];

    pub fn matches(self, e: &Entry) -> bool {
        match self {
            Filter::All => true,
            Filter::Unread => e.unread,
            Filter::Errors => e.kind == Kind::Failed,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All tabs",
            Filter::Unread => "Unread",
            Filter::Errors => "Errors",
        }
    }

    /// Next (or previous) chip, wrapping (Tab / ⇧Tab).
    pub fn cycle(self, delta: isize) -> Filter {
        let i = Filter::ALL.iter().position(|f| *f == self).unwrap_or(0) as isize;
        let n = Filter::ALL.len() as isize;
        Filter::ALL[(i + delta).rem_euclid(n) as usize]
    }
}

#[derive(Default)]
pub struct NotificationsState {
    pub open: bool,
    pub filter: Filter,
    /// Vertical scroll of the list (logical px, ≥ 0).
    pub scroll: f32,
    pub content_h: f32,
    pub view_h: f32,
    /// Keyboard-highlighted row (position in the filtered list).
    pub selected: Option<usize>,
    layout: Option<ListLayout>,
}

impl NotificationsState {
    pub fn open(&mut self) {
        self.open = true;
        self.scroll = 0.0;
        self.selected = None;
        self.filter = Filter::All;
    }

    pub fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        self.scroll = 0.0;
        self.selected = None;
    }

    /// Scroll by `px` (> 0 = down), clamped to the content.
    pub fn scroll_by(&mut self, px: f32) {
        let max = (self.content_h - self.view_h).max(0.0);
        self.scroll = (self.scroll + px).clamp(0.0, max);
    }

    /// Move the keyboard highlight by `delta` rows within `len` and keep it
    /// in view.
    pub fn move_selection(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.selected = None;
            return;
        }
        let next = match self.selected {
            None if delta > 0 => 0,
            None => len - 1,
            Some(i) => (i as isize + delta).clamp(0, len as isize - 1) as usize,
        };
        self.selected = Some(next);
        if let Some(l) = &self.layout
            && let Some(row) = l.rows.get(next)
        {
            if row.y < self.scroll {
                self.scroll = row.y;
            } else if row.y + row.h > self.scroll + self.view_h {
                self.scroll = row.y + row.h - self.view_h;
            }
        }
    }
}

/// Click targets of the last frame.
#[derive(Default, Clone)]
pub struct NotificationsHits {
    pub card: [f32; 4],
    pub close: [f32; 4],
    /// "Mark all as read" (zero-sized when nothing is unread).
    pub mark_read: [f32; 4],
    pub chips: Vec<([f32; 4], Filter)>,
    /// (rect, attention id), visible rows only.
    pub rows: Vec<([f32; 4], u64)>,
}

struct RowLayout {
    y: f32,
    h: f32,
    source: Option<(Arc<Buffer>, f32)>,
    branch: Option<Arc<Buffer>>,
    title: Arc<Buffer>,
    title_h: f32,
    detail: Option<(Arc<Buffer>, f32)>,
}

/// Shaped rows; valid while width and entries match.
struct ListLayout {
    key: (u32, Vec<u64>),
    rows: Vec<RowLayout>,
    height: f32,
}

pub const CARD_W: f32 = 420.0;
pub const TOAST_W: f32 = 360.0;
/// Toasts shown at once (Warp: two).
pub const MAX_TOASTS: usize = 2;
const PAD_X: f32 = 20.0;
const HEADER_H: f32 = 52.0;
const CHIPS_H: f32 = 44.0;
const ICON: f32 = 28.0;
const TEXT_X: f32 = PAD_X + ICON + 14.0;
const TITLE_SIZE: f32 = 14.0;
const TITLE_LINE: f32 = 20.0;
const TITLE_MAX_LINES: usize = 4;
const ROW_PAD: f32 = 14.0;

fn label(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut buffer = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    buffer.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(FONT_SANS)).weight(weight),
        Shaping::Advanced,
        None,
    );
    buffer
}

fn mono_label(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.3));
    b.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new()
            .family(Family::Name(mono_family()))
            .weight(weight),
        Shaping::Advanced,
        None,
    );
    b
}

/// Word-wrapped text at `width`, cut to `max_lines` with a trailing `…`.
fn wrapped(
    engine: &mut TextEngine,
    text: &str,
    size: f32,
    line: f32,
    weight: Weight,
    width: f32,
    max_lines: usize,
) -> (Buffer, f32) {
    let fs = &mut engine.font_system;
    let attrs = Attrs::new().family(Family::Name(FONT_SANS)).weight(weight);
    let mut buffer = Buffer::new(fs, Metrics::new(size, line));
    buffer.set_wrap(fs, Wrap::WordOrGlyph);
    buffer.set_size(fs, Some(width), None);
    buffer.set_text(fs, text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(fs, false);
    let mut lines = buffer.layout_runs().count().max(1);
    if lines > max_lines {
        // keep the text of the first lines, then trim until "…" fits
        let mut end = buffer
            .layout_runs()
            .take(max_lines)
            .last()
            .and_then(|run| run.glyphs.last().map(|g| g.end))
            .unwrap_or(text.len())
            .min(text.len());
        loop {
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            let cut = format!("{}…", text[..end].trim_end());
            buffer.set_text(fs, &cut, &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(fs, false);
            lines = buffer.layout_runs().count().max(1);
            if lines <= max_lines || end == 0 {
                break;
            }
            end = end.saturating_sub(4);
        }
    }
    (buffer, lines.min(max_lines) as f32 * line)
}

fn display_title(e: &Entry) -> &str {
    if e.title.trim().is_empty() {
        "(empty command)"
    } else {
        e.title.trim()
    }
}

fn build_layout(engine: &mut TextEngine, entries: &[Entry], width: f32) -> ListLayout {
    let text_w = width - TEXT_X - PAD_X;
    let mut rows = Vec::with_capacity(entries.len());
    let mut y = 6.0;
    for e in entries {
        let mut h = ROW_PAD;
        let source = e.agent.map(|a| {
            let w = engine.measure(a.name(), 13.0, FONT_SANS, Weight::MEDIUM);
            (Arc::new(label(engine, a.name(), 13.0, Weight::MEDIUM)), w)
        });
        let branch = e
            .branch
            .as_ref()
            .map(|b| Arc::new(label(engine, b, 13.0, Weight::NORMAL)));
        if source.is_some() || branch.is_some() {
            h += 20.0;
        }
        let (title_buf, title_h) = wrapped(
            engine,
            display_title(e),
            TITLE_SIZE,
            TITLE_LINE,
            Weight::SEMIBOLD,
            text_w,
            TITLE_MAX_LINES,
        );
        h += title_h;
        let detail = (!e.detail.is_empty()).then(|| {
            let (b, dh) = wrapped(engine, &e.detail, 13.0, 18.0, Weight::NORMAL, text_w, 2);
            h += 4.0 + dh;
            (Arc::new(b), dh)
        });
        h += ROW_PAD;
        rows.push(RowLayout {
            y,
            h,
            source,
            branch,
            title: Arc::new(title_buf),
            title_h,
            detail,
        });
        y += h;
    }
    ListLayout {
        key: (width.to_bits(), entries.iter().map(|e| e.id).collect()),
        rows,
        height: y + 6.0,
    }
}

/// "Just now", "4 minutes ago", "2 hours ago", "3 days ago".
pub fn relative_time(secs: u64) -> String {
    let plural = |n: u64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match secs {
        s if s < 60 => "Just now".into(),
        s if s < 3600 => plural(s / 60, "minute"),
        s if s < 86_400 => plural(s / 3600, "hour"),
        s => plural(s / 86_400, "day"),
    }
}

/// Inbox tray outline (tab-bar button), centered in `r`.
pub fn inbox_icon(layer: &mut Layer, r: [f32; 4], color: Color) {
    let (iw, ih) = (16.0, 13.0);
    let (ix, iy) = (
        (r[0] + (r[2] - iw) / 2.0).round(),
        (r[1] + (r[3] - ih) / 2.0).round(),
    );
    let t = 1.3;
    layer.bordered([ix, iy], [iw, ih], color.with_alpha(0.0), color, 3.0, t);
    // the tray lip: a shelf with a dip in the middle
    let (shelf, dip, side) = (iy + 6.0, 3.2, 4.8);
    layer.rect([ix, shelf], [side, t], color, 0.0);
    layer.rect([ix + iw - side, shelf], [side, t], color, 0.0);
    layer.rect([ix + side - t, shelf], [t, dip], color, 0.0);
    layer.rect([ix + iw - side, shelf], [t, dip], color, 0.0);
    layer.rect(
        [ix + side - t, shelf + dip - t],
        [iw - 2.0 * side + 2.0 * t, t],
        color,
        0.0,
    );
}

/// Git branch glyph: two stacked commits and a side branch.
fn branch_icon(layer: &mut Layer, x: f32, y: f32, color: Color) {
    let t = 1.2;
    let ring = |layer: &mut Layer, rx: f32, ry: f32| {
        layer.bordered([rx, ry], [5.0, 5.0], color.with_alpha(0.0), color, 2.5, t);
    };
    layer.rect([x + 2.4, y + 3.0], [t, 8.0], color, 0.0);
    ring(layer, x + 0.5, y);
    ring(layer, x + 0.5, y + 9.5);
    ring(layer, x + 7.0, y + 2.0);
    layer.rect([x + 8.9, y + 6.5], [t, 2.5], color, 0.0);
    layer.rect([x + 3.0, y + 8.8], [7.1, t], color, 0.0);
}

/// Semantic color of a kind (DESIGN.md: meaning, not decoration).
pub fn kind_color(theme: &Theme, kind: Kind) -> Color {
    match kind {
        Kind::Finished => theme.success,
        Kind::Failed => theme.error,
        Kind::Request => theme.warning,
        Kind::Notify => theme.accent,
        Kind::Bell => theme.text_dim,
    }
}

/// Brand disc and mark of an agent's badge.
fn agent_style(agent: Agent) -> (Color, &'static str, Color) {
    let hex = |s: &str| Color::hex(s).unwrap_or(Color::WHITE);
    match agent {
        Agent::ClaudeCode => (hex("#d97757"), "*", Color::WHITE),
        Agent::Codex => (hex("#f2f2f2"), ">_", hex("#111111")),
        Agent::OpenCode => (hex("#3b3b40"), "oc", Color::WHITE),
        Agent::Gemini => (hex("#4285f4"), "G", Color::WHITE),
    }
}

/// Centered mark in the mono font.
fn mark(
    layer: &mut Layer,
    engine: &mut TextEngine,
    text: &str,
    size: f32,
    cx: f32,
    cy: f32,
    c: Color,
) {
    let w = engine.measure(text, size, mono_family(), Weight::BOLD);
    let b = mono_label(engine, text, size, Weight::BOLD);
    layer.text(b, cx - w / 2.0, cy - size * 0.65, c);
}

/// Row badge: the agent's disc with a small kind dot, or a tinted kind disc
/// with a check, cross, `!`, dot or bell, centered at `c`.
fn badge(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    kind: Kind,
    agent: Option<Agent>,
    c: [f32; 2],
) {
    let [cx, cy] = c;
    let ring = theme.surface;
    let color = kind_color(theme, kind);
    if let Some(agent) = agent {
        let (disc, glyph, ink) = agent_style(agent);
        layer.circle([cx, cy], ICON, disc);
        let size = if glyph.len() > 1 { 11.0 } else { 17.0 };
        let dy = if glyph == "*" { 3.0 } else { 0.0 };
        mark(layer, engine, glyph, size, cx, cy + dy, ink);
        // kind dot, ringed in the card color (Warp's status overlay)
        let (dx, dy) = (cx + ICON / 2.0 - 4.0, cy + ICON / 2.0 - 4.0);
        layer.circle([dx, dy], 13.0, ring);
        layer.circle([dx, dy], 9.0, color);
        return;
    }
    layer.circle([cx, cy], ICON, color.with_alpha(0.16));
    match kind {
        Kind::Finished => mark(layer, engine, "✓", 14.0, cx, cy, color),
        Kind::Failed => mark(layer, engine, "✕", 14.0, cx, cy, color),
        Kind::Request => mark(layer, engine, "!", 15.0, cx, cy, color),
        Kind::Notify => layer.circle([cx, cy], 8.0, color),
        Kind::Bell => {
            layer.rect([cx - 4.5, cy - 5.0], [9.0, 8.0], color, 4.0);
            layer.rect([cx - 6.0, cy + 2.0], [12.0, 1.8], color, 0.9);
            layer.circle([cx, cy + 5.0], 3.2, color);
        }
    }
}

/// Tab-strip marker for a tab's newest unread entry: a dot in the kind's
/// color with a soft halo (replaces the activity dot).
pub fn tab_badge(layer: &mut Layer, theme: &Theme, kind: Kind, cx: f32, cy: f32) {
    let c = kind_color(theme, kind);
    layer.circle([cx, cy], 12.0, c.with_alpha(0.25));
    layer.circle([cx, cy], 7.0, c);
}

/// ✕ from two diagonal dot runs (crisp, no font dependency).
fn cross(layer: &mut Layer, cx: f32, cy: f32, half: i32, color: Color) {
    for i in -half..=half {
        let f = i as f32;
        layer.rect([cx + f - 0.75, cy + f - 0.75], [1.5, 1.5], color, 0.5);
        layer.rect([cx + f - 0.75, cy - f - 0.75], [1.5, 1.5], color, 0.5);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &mut NotificationsState,
    entries: &[Entry],
    anchor_right: f32,
    top: f32,
    w: f32,
    h: f32,
) -> NotificationsHits {
    let card_w = CARD_W.min(w - 16.0);
    let x = (anchor_right - card_w)
        .clamp(8.0, (w - card_w - 8.0).max(8.0))
        .round();
    let y = top.round();

    let shown: Vec<Entry> = entries
        .iter()
        .filter(|e| state.filter.matches(e))
        .cloned()
        .collect();

    let key = (card_w.to_bits(), shown.iter().map(|e| e.id).collect());
    if state.layout.as_ref().is_none_or(|l| l.key != key) {
        state.layout = Some(build_layout(engine, &shown, card_w));
    }
    let layout = state.layout.as_ref().expect("built above");

    let list_top = y + HEADER_H + CHIPS_H + 1.0;
    let max_h = (h - y - 12.0).max(HEADER_H + CHIPS_H + 80.0);
    let empty_h = 96.0;
    let body_h = if shown.is_empty() {
        empty_h
    } else {
        layout.height
    };
    let card_h = (HEADER_H + CHIPS_H + 1.0 + body_h).min(max_h).min(620.0);
    let view_h = card_h - (list_top - y);
    state.view_h = view_h;
    state.content_h = if shown.is_empty() { 0.0 } else { layout.height };
    state.scroll = state.scroll.clamp(0.0, (state.content_h - view_h).max(0.0));
    if state.selected.is_some_and(|s| s >= shown.len()) {
        state.selected = shown.len().checked_sub(1);
    }

    layer.shadow(
        [x, y + 8.0],
        [card_w, card_h],
        Color::BLACK.with_alpha(0.55),
        12.0,
        18.0,
    );
    layer.bordered(
        [x, y],
        [card_w, card_h],
        theme.surface,
        Color::WHITE.with_alpha(0.12),
        12.0,
        1.0,
    );

    // ── header: title, "Mark all as read", ✕ ──
    layer.text(
        label(engine, "Notifications", 16.0, Weight::MEDIUM),
        x + PAD_X,
        y + 16.0,
        theme.text_bright,
    );
    let close = [x + card_w - 12.0 - 28.0, y + 12.0, 28.0, 28.0];
    cross(layer, close[0] + 14.0, close[1] + 14.0, 4, theme.text_ui);
    let unread = entries.iter().filter(|e| e.unread).count();
    let mut mark_read = [0.0; 4];
    if unread > 0 {
        let text = "Mark all as read";
        let tw = engine.measure(text, 12.5, FONT_SANS, Weight::MEDIUM);
        mark_read = [close[0] - 8.0 - tw - 16.0, y + 14.0, tw + 16.0, 24.0];
        layer.text(
            label(engine, text, 12.5, Weight::MEDIUM),
            mark_read[0] + 8.0,
            mark_read[1] + 4.0,
            theme.accent,
        );
    }

    // ── filter chips ──
    let mut chips = Vec::new();
    let mut cx = x + PAD_X;
    for filter in Filter::ALL {
        let n = entries.iter().filter(|e| filter.matches(e)).count();
        let text = format!("{} ({n})", filter.label());
        let tw = engine.measure(&text, 13.0, FONT_SANS, Weight::MEDIUM);
        let r = [cx, y + HEADER_H, tw + 22.0, 30.0];
        let on = state.filter == filter;
        if on {
            layer.rect(
                [r[0], r[1]],
                [r[2], r[3]],
                Color::WHITE.with_alpha(0.14),
                7.0,
            );
        }
        layer.text(
            label(engine, &text, 13.0, Weight::MEDIUM),
            r[0] + 11.0,
            r[1] + 6.5,
            if on {
                theme.text_bright
            } else {
                theme.text_dim
            },
        );
        chips.push((r, filter));
        cx += r[2] + 6.0;
    }
    layer.hline(x, list_top - 1.0, card_w, 1.0, theme.border);

    // ── rows ──
    let clip = TextBounds {
        left: x as i32,
        top: list_top as i32,
        right: (x + card_w) as i32,
        bottom: (y + card_h - 1.0) as i32,
    };
    let mut rows = Vec::new();
    if shown.is_empty() {
        let msg = match state.filter {
            Filter::All => "No notifications",
            Filter::Unread => "Nothing unread",
            Filter::Errors => "No errors",
        };
        let mw = engine.measure(msg, 13.0, FONT_SANS, Weight::NORMAL);
        layer.text(
            label(engine, msg, 13.0, Weight::NORMAL),
            x + (card_w - mw) / 2.0,
            list_top + (empty_h - 18.0) / 2.0,
            theme.text_muted,
        );
    }
    let bottom = y + card_h;
    for (pos, (row, entry)) in layout.rows.iter().zip(&shown).enumerate() {
        let ry = list_top + row.y - state.scroll;
        if ry + row.h < list_top || ry > bottom {
            continue;
        }
        let rect = [
            x + 1.0,
            ry.max(list_top),
            card_w - 2.0,
            (ry + row.h).min(bottom) - ry.max(list_top),
        ];
        if state.selected == Some(pos) {
            layer.rect(
                [rect[0] + 5.0, rect[1]],
                [rect[2] - 10.0, rect[3]],
                Color::WHITE.with_alpha(0.06),
                8.0,
            );
        }
        rows.push((rect, entry.id));
        let fully_in = |top: f32, height: f32| top >= list_top && top + height <= bottom - 1.0;
        let mut ty = ry + ROW_PAD;
        // badge + unread dot: quads cannot be clipped, so only when whole
        let icon_top = ry + ROW_PAD + 2.0;
        if fully_in(icon_top, ICON + 4.0) {
            badge(
                layer,
                engine,
                theme,
                entry.kind,
                entry.agent,
                [x + PAD_X + ICON / 2.0, icon_top + ICON / 2.0],
            );
            if entry.unread {
                layer.circle([x + 9.0, icon_top + ICON / 2.0], 6.0, theme.accent);
            }
        }
        let tx = x + TEXT_X;
        let ago = relative_time(entry.age_secs);
        let ago_w = engine.measure(&ago, 13.0, FONT_SANS, Weight::NORMAL);
        let ago_x = x + card_w - PAD_X - ago_w;
        if fully_in(ty, 18.0) {
            layer.text_clipped(
                label(engine, &ago, 13.0, Weight::NORMAL),
                ago_x,
                ty,
                theme.text_muted,
                clip,
            );
        }
        if row.source.is_some() || row.branch.is_some() {
            let bounds = TextBounds {
                right: (ago_x - 10.0) as i32,
                ..clip
            };
            let mut sx = tx;
            if let Some((b, bw)) = &row.source {
                layer.text_clipped(b.clone(), sx, ty, theme.text_ui, bounds);
                sx += bw + 10.0;
            }
            if let Some(b) = &row.branch {
                if fully_in(ty + 2.0, 14.0) && sx + 14.0 < ago_x - 10.0 {
                    branch_icon(layer, sx, ty + 2.0, theme.text_dim);
                }
                layer.text_clipped(b.clone(), sx + 17.0, ty, theme.text_dim, bounds);
            }
            ty += 20.0;
        }
        layer.text_clipped(row.title.clone(), tx, ty, theme.text_bright, clip);
        ty += row.title_h;
        if let Some((d, _)) = &row.detail {
            layer.text_clipped(d.clone(), tx, ty + 4.0, theme.text_dim, clip);
        }
    }

    NotificationsHits {
        card: [x, y, card_w, card_h],
        close,
        mark_read,
        chips,
        rows,
    }
}

/// Click targets of one toast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToastHit {
    pub id: u64,
    pub rect: [f32; 4],
    pub close: [f32; 4],
}

/// In-window toasts, newest on top, right-aligned under `top`.
pub fn render_toasts(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    toasts: &[Entry],
    top: f32,
    w: f32,
) -> Vec<ToastHit> {
    let tw = TOAST_W.min(w - 24.0);
    let x = (w - tw - 12.0).round();
    let mut y = top.round();
    let text_x = x + 16.0 + ICON + 12.0;
    let text_w = tw - (text_x - x) - 36.0;
    let mut hits = Vec::new();
    for e in toasts.iter().take(MAX_TOASTS) {
        let source = match (&e.agent, &e.branch) {
            (Some(a), Some(b)) => format!("{} · {b}", a.name()),
            (Some(a), None) => a.name().to_string(),
            (None, Some(b)) => b.clone(),
            (None, None) => String::new(),
        };
        let (title, title_h) = wrapped(
            engine,
            display_title(e),
            13.5,
            19.0,
            Weight::SEMIBOLD,
            text_w,
            2,
        );
        let detail = (!e.detail.is_empty())
            .then(|| wrapped(engine, &e.detail, 12.5, 17.0, Weight::NORMAL, text_w, 1));
        let src_h = if source.is_empty() { 0.0 } else { 18.0 };
        let h = (14.0 + src_h + title_h + detail.as_ref().map_or(0.0, |d| d.1 + 2.0) + 14.0)
            .max(ICON + 28.0);
        layer.shadow(
            [x, y + 6.0],
            [tw, h],
            Color::BLACK.with_alpha(0.5),
            12.0,
            14.0,
        );
        layer.bordered(
            [x, y],
            [tw, h],
            theme.surface,
            Color::WHITE.with_alpha(0.12),
            12.0,
            1.0,
        );
        // kind stripe on the left edge
        layer.rect(
            [x + 1.0, y + 10.0],
            [3.0, h - 20.0],
            kind_color(theme, e.kind),
            1.5,
        );
        badge(
            layer,
            engine,
            theme,
            e.kind,
            e.agent,
            [x + 16.0 + ICON / 2.0, y + 14.0 + ICON / 2.0],
        );
        let mut ty = y + 14.0;
        if !source.is_empty() {
            let b = label(engine, &source, 12.0, Weight::MEDIUM);
            let bounds = TextBounds {
                left: text_x as i32,
                top: y as i32,
                right: (text_x + text_w) as i32,
                bottom: (y + h) as i32,
            };
            layer.text_clipped(b, text_x, ty, theme.text_dim, bounds);
            ty += src_h;
        }
        layer.text(title, text_x, ty, theme.text_bright);
        ty += title_h;
        if let Some((d, _)) = detail {
            layer.text(d, text_x, ty + 2.0, theme.text_dim);
        }
        let close = [x + tw - 30.0, y + 8.0, 22.0, 22.0];
        cross(layer, close[0] + 11.0, close[1] + 11.0, 3, theme.text_muted);
        hits.push(ToastHit {
            id: e.id,
            rect: [x, y, tw, h],
            close,
        });
        y += h + 8.0;
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, kind: Kind, unread: bool) -> Entry {
        Entry {
            id,
            kind,
            agent: None,
            branch: None,
            title: String::new(),
            detail: String::new(),
            age_secs: 0,
            unread,
        }
    }

    #[test]
    fn relative_time_buckets() {
        assert_eq!(relative_time(0), "Just now");
        assert_eq!(relative_time(59), "Just now");
        assert_eq!(relative_time(60), "1 minute ago");
        assert_eq!(relative_time(4 * 60 + 30), "4 minutes ago");
        assert_eq!(relative_time(3600), "1 hour ago");
        assert_eq!(relative_time(2 * 86_400), "2 days ago");
    }

    #[test]
    fn selection_moves_and_clamps() {
        let mut s = NotificationsState::default();
        s.move_selection(1, 3);
        assert_eq!(s.selected, Some(0));
        s.move_selection(5, 3);
        assert_eq!(s.selected, Some(2));
        s.move_selection(-1, 3);
        assert_eq!(s.selected, Some(1));
        s.move_selection(1, 0);
        assert_eq!(s.selected, None);
    }

    #[test]
    fn filters_match_and_cycle() {
        let es = [
            entry(1, Kind::Finished, false),
            entry(2, Kind::Failed, true),
            entry(3, Kind::Request, true),
            entry(4, Kind::Failed, false),
            entry(5, Kind::Bell, false),
        ];
        let ids = |f: Filter| {
            es.iter()
                .filter(|e| f.matches(e))
                .map(|e| e.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(Filter::All), [1, 2, 3, 4, 5]);
        assert_eq!(ids(Filter::Unread), [2, 3]);
        assert_eq!(ids(Filter::Errors), [2, 4]);
        assert_eq!(Filter::All.cycle(1), Filter::Unread);
        assert_eq!(Filter::Errors.cycle(1), Filter::All);
        assert_eq!(Filter::All.cycle(-1), Filter::Errors);
    }
}
