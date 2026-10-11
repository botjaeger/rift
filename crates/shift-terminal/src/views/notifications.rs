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
// Metrics match Warp's inbox at the same card width (13 px header and
// details, 12 px meta line, 24 px badges).
const PAD_X: f32 = 18.0;
const HEADER_H: f32 = 44.0;
const CHIPS_H: f32 = 38.0;
const CHIP_H: f32 = 26.0;
const ICON: f32 = 24.0;
const TEXT_X: f32 = PAD_X + ICON + 12.0;
const TITLE_SIZE: f32 = 13.0;
const TITLE_LINE: f32 = 18.0;
const TITLE_MAX_LINES: usize = 4;
const ROW_PAD: f32 = 11.0;
const META_SIZE: f32 = 12.0;
const META_LINE: f32 = 18.0;
const DETAIL_SIZE: f32 = 12.5;
const DETAIL_LINE: f32 = 17.0;

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
            let w = engine.measure(a.name(), META_SIZE, FONT_SANS, Weight::MEDIUM);
            (
                Arc::new(label(engine, a.name(), META_SIZE, Weight::MEDIUM)),
                w,
            )
        });
        let branch = e
            .branch
            .as_ref()
            .map(|b| Arc::new(label(engine, b, META_SIZE, Weight::NORMAL)));
        // meta line (agent, branch, age); without agent or branch the age
        // shares the title's first line, so the title wraps clear of the
        // widest age
        let meta = source.is_some() || branch.is_some();
        let title_w = if meta {
            h += META_LINE;
            text_w
        } else {
            text_w - engine.measure(AGE_WIDEST, META_SIZE, FONT_SANS, Weight::NORMAL) - 10.0
        };
        let (title_buf, title_h) = wrapped(
            engine,
            display_title(e),
            TITLE_SIZE,
            TITLE_LINE,
            Weight::SEMIBOLD,
            title_w,
            TITLE_MAX_LINES,
        );
        h += title_h;
        let detail = (!e.detail.is_empty()).then(|| {
            let (b, dh) = wrapped(
                engine,
                &e.detail,
                DETAIL_SIZE,
                DETAIL_LINE,
                Weight::NORMAL,
                text_w,
                2,
            );
            h += 2.0 + dh;
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

/// The widest `relative_time` text a row reserves room for.
const AGE_WIDEST: &str = "59 minutes ago";

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

/// The Claude mark (Simple Icons `claude`, 24×24 viewBox). Nerd Fonts has
/// no Claude glyph, so the badge rasterizes this path instead.
const CLAUDE_PATH: &str = "m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z";

/// Sprite key space of the Claude mark (inline images count up from 1).
const CLAUDE_KEY: u64 = 0xC1A0_DE00_0000_0000;

/// Size of the Claude mark relative to its disc.
const CLAUDE_MARK: f32 = 0.62;

/// The Claude mark in white, `px` device pixels square, premultiplied RGBA
/// (rasterized once per size and cut). Sprites draw over the layer's
/// quads, so the pixels under a badge's kind dot (`cut`: centre and radius,
/// in mark px) are cleared to keep the dot on top, as in Warp.
fn claude_rgba(px: u32, cut: Option<[f32; 3]>) -> Arc<Vec<u8>> {
    use std::sync::Mutex;
    type Key = (u32, Option<[u32; 3]>);
    static CACHE: Mutex<Vec<(Key, Arc<Vec<u8>>)>> = Mutex::new(Vec::new());
    let key = (px, cut.map(|c| c.map(f32::to_bits)));
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, rgba)) = cache.iter().find(|(k, _)| *k == key) {
        return rgba.clone();
    }
    let s = px as f32 / 24.0;
    let (alpha, _) = zeno::Mask::new(CLAUDE_PATH)
        .size(px, px)
        .transform(Some(zeno::Transform::scale(s, s)))
        .render();
    let [cx, cy, r] = cut.unwrap_or([-1e6, -1e6, 0.0]);
    let rgba = alpha
        .iter()
        .enumerate()
        .flat_map(|(i, &a)| {
            let (x, y) = ((i as u32 % px) as f32 + 0.5, (i as u32 / px) as f32 + 0.5);
            // one-pixel soft edge
            let keep = ((x - cx).hypot(y - cy) - r).clamp(0.0, 1.0);
            let a = (a as f32 * keep).round() as u8;
            [a, a, a, a]
        })
        .collect();
    let rgba = Arc::new(rgba);
    cache.push((key, Arc::clone(&rgba)));
    rgba
}

/// Draw the Claude mark for a `disc` px disc centred on `c`, clear of the
/// kind dot (`dot`: centre and radius, logical px) when there is one.
fn claude_mark(
    layer: &mut Layer,
    c: [f32; 2],
    disc: f32,
    dot: Option<([f32; 2], f32)>,
    scale: f32,
    clip: [f32; 4],
) {
    let size = disc * CLAUDE_MARK;
    let px = (size * scale).round().max(1.0) as u32;
    let k = px as f32 / size;
    let origin = [c[0] - size / 2.0, c[1] - size / 2.0];
    let cut = dot.map(|(d, r)| [(d[0] - origin[0]) * k, (d[1] - origin[1]) * k, r * k]);
    layer.sprites.push(shift_renderer::Sprite {
        key: CLAUDE_KEY | px as u64 | u64::from(cut.is_some()) << 16,
        rgba: claude_rgba(px, cut),
        size: [px, px],
        rect: [c[0] - size / 2.0, c[1] - size / 2.0, size, size],
        src: [0.0, 0.0, px as f32, px as f32],
        clip,
        under_text: true,
    });
}

/// An agent's brand disc with its mark, `d` px across, centred on `c`
/// (the quit dialog's process rows). `clip` bounds the Claude sprite.
pub fn agent_disc(
    layer: &mut Layer,
    engine: &mut TextEngine,
    agent: Agent,
    c: [f32; 2],
    d: f32,
    scale: f32,
    clip: [f32; 4],
) {
    let (disc, glyph, ink) = agent_style(agent);
    layer.circle(c, d, disc);
    if agent == Agent::ClaudeCode {
        claude_mark(layer, c, d, None, scale, clip);
    } else {
        let size = d * if glyph.len() > 1 { 0.42 } else { 0.62 };
        mark(layer, engine, glyph, size, c[0], c[1], ink);
    }
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
        // drawn as the Claude mark sprite (`claude_mark`), not a glyph
        Agent::ClaudeCode => (hex("#d97757"), "", Color::WHITE),
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
/// with a check, cross, `!` or bell, centered at `c`. `clip` bounds
/// the Claude mark sprite (the card or toast).
#[allow(clippy::too_many_arguments)]
fn badge(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    kind: Kind,
    agent: Option<Agent>,
    c: [f32; 2],
    scale: f32,
    clip: [f32; 4],
) {
    let [cx, cy] = c;
    let ring = theme.surface;
    let color = kind_color(theme, kind);
    if let Some(agent) = agent {
        let (disc, glyph, ink) = agent_style(agent);
        layer.circle([cx, cy], ICON, disc);
        // kind dot, ringed in the card color (Warp's status overlay)
        let (dx, dy) = (cx + ICON / 2.0 - 3.0, cy + ICON / 2.0 - 3.0);
        if agent == Agent::ClaudeCode {
            claude_mark(layer, [cx, cy], ICON, Some(([dx, dy], 6.0)), scale, clip);
        } else {
            let size = if glyph.len() > 1 { 10.0 } else { 15.0 };
            mark(layer, engine, glyph, size, cx, cy, ink);
        }
        layer.circle([dx, dy], 12.0, ring);
        layer.circle([dx, dy], 8.0, color);
        return;
    }
    layer.circle([cx, cy], ICON, color.with_alpha(0.16));
    match kind {
        Kind::Finished => mark(layer, engine, "✓", 12.0, cx, cy, color),
        Kind::Failed => mark(layer, engine, "✕", 12.0, cx, cy, color),
        Kind::Request => mark(layer, engine, "!", 13.0, cx, cy, color),
        // the default notification icon: a bell (filled for a program's
        // notification, outlined for a terminal bell)
        Kind::Notify => super::glyphs::draw(layer, engine, super::glyphs::BELL, c, 12.0, color),
        Kind::Bell => {
            super::glyphs::draw(layer, engine, super::glyphs::BELL_OUTLINE, c, 12.0, color)
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
    scale: f32,
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
        8.0,
        18.0,
    );
    layer.bordered(
        [x, y],
        [card_w, card_h],
        theme.surface,
        Color::WHITE.with_alpha(0.12),
        8.0,
        1.0,
    );

    // ── header: title, "Mark all as read", ✕ ──
    layer.text(
        label(engine, "Notifications", 13.5, Weight::MEDIUM),
        x + PAD_X,
        y + 14.0,
        theme.text_bright,
    );
    let close = [x + card_w - 10.0 - 24.0, y + 10.0, 24.0, 24.0];
    super::close_icon(
        layer,
        [close[0] + 12.0, close[1] + 12.0],
        4.0,
        theme.text_ui,
    );
    let unread = entries.iter().filter(|e| e.unread).count();
    let mut mark_read = [0.0; 4];
    if unread > 0 {
        let text = "Mark all as read";
        let tw = engine.measure(text, META_SIZE, FONT_SANS, Weight::MEDIUM);
        mark_read = [close[0] - 6.0 - tw - 16.0, y + 11.0, tw + 16.0, 22.0];
        layer.text(
            label(engine, text, META_SIZE, Weight::MEDIUM),
            mark_read[0] + 8.0,
            mark_read[1] + 3.0,
            theme.accent,
        );
    }

    // ── filter chips ──
    let mut chips = Vec::new();
    let mut cx = x + PAD_X;
    for filter in Filter::ALL {
        let n = entries.iter().filter(|e| filter.matches(e)).count();
        let text = format!("{} ({n})", filter.label());
        let tw = engine.measure(&text, META_SIZE, FONT_SANS, Weight::MEDIUM);
        let r = [cx, y + HEADER_H, tw + 16.0, CHIP_H];
        let on = state.filter == filter;
        if on {
            layer.rect(
                [r[0], r[1]],
                [r[2], r[3]],
                theme.accent.with_alpha(0.16),
                4.0,
            );
        }
        layer.text(
            label(engine, &text, META_SIZE, Weight::MEDIUM),
            r[0] + 8.0,
            r[1] + (CHIP_H - META_SIZE * 1.35) / 2.0,
            if on {
                theme.text_bright
            } else {
                theme.text_dim
            },
        );
        chips.push((r, filter));
        cx += r[2] + 4.0;
    }
    layer.hline(x + 1.0, list_top - 1.0, card_w - 2.0, 1.0, theme.border);

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
        let mw = engine.measure(msg, DETAIL_SIZE, FONT_SANS, Weight::NORMAL);
        layer.text(
            label(engine, msg, DETAIL_SIZE, Weight::NORMAL),
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
                4.0,
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
                scale,
                [x, list_top, card_w, card_h - (list_top - y)],
            );
            if entry.unread {
                layer.circle([x + 9.0, icon_top + ICON / 2.0], 6.0, theme.accent);
            }
        }
        let tx = x + TEXT_X;
        let ago = relative_time(entry.age_secs);
        let ago_w = engine.measure(&ago, META_SIZE, FONT_SANS, Weight::NORMAL);
        let ago_x = x + card_w - PAD_X - ago_w;
        let meta = row.source.is_some() || row.branch.is_some();
        // on the title's first line, the age sits on the title's baseline
        let ago_y = if meta { ty } else { ty + 1.5 };
        if fully_in(ago_y, META_LINE) {
            layer.text_clipped(
                label(engine, &ago, META_SIZE, Weight::NORMAL),
                ago_x,
                ago_y,
                theme.text_muted,
                clip,
            );
        }
        if meta {
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
                if fully_in(ty, META_LINE) && sx + 12.0 < ago_x - 10.0 {
                    super::glyphs::draw(
                        layer,
                        engine,
                        super::glyphs::BRANCH,
                        [sx + 6.0, ty + META_SIZE * 1.35 / 2.0],
                        META_SIZE,
                        theme.text_dim,
                    );
                }
                layer.text_clipped(b.clone(), sx + 16.0, ty, theme.text_dim, bounds);
            }
            ty += META_LINE;
        }
        layer.text_clipped(row.title.clone(), tx, ty, theme.text_bright, clip);
        ty += row.title_h;
        if let Some((d, _)) = &row.detail {
            layer.text_clipped(d.clone(), tx, ty + 2.0, theme.text_dim, clip);
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
    scale: f32,
) -> Vec<ToastHit> {
    let tw = TOAST_W.min(w - 24.0);
    let x = (w - tw - 12.0).round();
    let mut y = top.round();
    let text_x = x + 14.0 + ICON + 12.0;
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
            TITLE_SIZE,
            TITLE_LINE,
            Weight::SEMIBOLD,
            text_w,
            2,
        );
        let detail = (!e.detail.is_empty()).then(|| {
            wrapped(
                engine,
                &e.detail,
                DETAIL_SIZE,
                DETAIL_LINE,
                Weight::NORMAL,
                text_w,
                1,
            )
        });
        let src_h = if source.is_empty() { 0.0 } else { META_LINE };
        let content_h = src_h + title_h + detail.as_ref().map_or(0.0, |d| d.1 + 2.0);
        let h = (content_h + 24.0).max(ICON + 24.0);
        layer.shadow(
            [x, y + 6.0],
            [tw, h],
            Color::BLACK.with_alpha(0.5),
            8.0,
            14.0,
        );
        layer.bordered(
            [x, y],
            [tw, h],
            theme.surface,
            Color::WHITE.with_alpha(0.12),
            8.0,
            1.0,
        );
        // badge and text block both centred on the card's height
        badge(
            layer,
            engine,
            theme,
            e.kind,
            e.agent,
            [x + 14.0 + ICON / 2.0, y + h / 2.0],
            scale,
            [x, y, tw, h],
        );
        let mut ty = y + (h - content_h) / 2.0;
        if !source.is_empty() {
            let b = label(engine, &source, META_SIZE, Weight::MEDIUM);
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
        super::close_icon(
            layer,
            [close[0] + 11.0, close[1] + 11.0],
            3.5,
            theme.text_muted,
        );
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
    fn claude_mark_rasterizes_clear_of_the_kind_dot() {
        let alpha = |rgba: &[u8], px: u32, x: u32, y: u32| rgba[((y * px + x) * 4 + 3) as usize];
        let whole = claude_rgba(32, None);
        assert_eq!(whole.len(), 32 * 32 * 4);
        assert!(
            alpha(&whole, 32, 16, 16) > 200,
            "the mark's centre is solid"
        );
        // premultiplied white: every channel equals alpha
        assert!(whole.chunks(4).all(|p| p[0] == p[3] && p[1] == p[3]));
        let cut = claude_rgba(30, Some([15.0, 15.0, 4.0]));
        assert_eq!(alpha(&cut, 30, 15, 15), 0, "the dot's area is cleared");
        assert!(cut.chunks(4).any(|p| p[3] > 200), "the rest stays");
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
