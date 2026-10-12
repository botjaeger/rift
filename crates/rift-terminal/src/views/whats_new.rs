//! What's New: a modal over the window listing `CHANGELOG.md` releases,
//! newest first. Opens from Help → What's New, the palette, or once after
//! an update. The body scrolls; Esc, Enter, ✕, "Got it" or a click outside
//! the card closes it.
//!
//! The body is shaped once per width and theme (`BodyLayout`) and reused
//! while scrolling; each frame only offsets it and drops what is off-screen,
//! so a wheel tick costs no text shaping.

use std::sync::Arc;

use rift_core::changelog::Release;
use rift_core::{Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight, Wrap};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family, text_color};

#[derive(Default)]
pub struct WhatsNewState {
    pub open: bool,
    /// Vertical scroll of the body (logical px, ≥ 0).
    pub scroll: f32,
    /// Height of the last rendered body content and viewport.
    pub content_h: f32,
    pub view_h: f32,
    layout: Option<BodyLayout>,
}

impl WhatsNewState {
    /// Launch state: open once after an update (never on a first install),
    /// then record the running version as seen.
    pub fn on_launch() -> Self {
        use rift_core::changelog;
        let open =
            changelog::opens_after_update(changelog::last_seen().as_deref(), changelog::VERSION);
        changelog::mark_seen();
        Self {
            open,
            ..Self::default()
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.scroll = 0.0;
    }

    /// Scroll by `px` (> 0 = down), clamped to the content.
    pub fn scroll_by(&mut self, px: f32) {
        if self.layout.is_none() {
            // not laid out yet: keep the input, the first render clamps it
            self.scroll = (self.scroll + px).max(0.0);
            return;
        }
        let max = (self.content_h - self.view_h).max(0.0);
        self.scroll = (self.scroll + px).clamp(0.0, max);
    }

    /// One viewport minus a line of overlap (PageUp/PageDown).
    pub fn page(&self) -> f32 {
        (self.view_h - 40.0).max(40.0)
    }
}

/// Click targets of the last frame.
#[derive(Default, Clone, Copy)]
pub struct WhatsNewHits {
    pub card: [f32; 4],
    pub close: [f32; 4],
    pub done: [f32; 4],
}

/// One drawable of the body, positioned relative to the content top.
enum Piece {
    Text {
        buffer: Arc<Buffer>,
        x: f32,
        y: f32,
        h: f32,
        color: Color,
    },
    Rule {
        y: f32,
    },
    Dot {
        y: f32,
        color: Color,
    },
}

/// The shaped body; valid while the text width and theme colors match.
struct BodyLayout {
    width: f32,
    colors: [Color; 7],
    pieces: Vec<Piece>,
    height: f32,
}

const MODAL_W: f32 = 620.0;
const PAD_X: f32 = 28.0;
const HEADER_H: f32 = 68.0;
const FOOTER_H: f32 = 64.0;
const BODY_SIZE: f32 = 13.0;
const BODY_LINE: f32 = 19.0;
/// Space above the first and below the last body line.
const BODY_PAD: f32 = 20.0;

fn label(
    engine: &mut TextEngine,
    text: &str,
    size: f32,
    family: &'static str,
    weight: Weight,
) -> Buffer {
    let mut buffer = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    buffer.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(family)).weight(weight),
        Shaping::Advanced,
        None,
    );
    buffer
}

/// Word-wrapped rich text at `width`; returns the buffer and its height.
fn paragraph(
    engine: &mut TextEngine,
    spans: &[(&str, Weight, Color)],
    width: f32,
) -> (Buffer, f32) {
    let fs = &mut engine.font_system;
    let mut buffer = Buffer::new(fs, Metrics::new(BODY_SIZE, BODY_LINE));
    buffer.set_wrap(fs, Wrap::Word);
    buffer.set_size(fs, Some(width), None);
    buffer.set_rich_text(
        fs,
        spans.iter().map(|(text, weight, color)| {
            (
                *text,
                Attrs::new()
                    .family(Family::Name(FONT_SANS))
                    .weight(*weight)
                    .color(text_color(*color)),
            )
        }),
        &Attrs::new().family(Family::Name(FONT_SANS)),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(fs, false);
    let lines = buffer.layout_runs().count().max(1);
    (buffer, lines as f32 * BODY_LINE)
}

/// Section title color: meaning, not decoration (DESIGN.md).
fn section_color(theme: &Theme, title: &str) -> Color {
    match title.to_ascii_lowercase().as_str() {
        "new" | "added" => theme.accent,
        "improved" | "changed" => theme.success,
        "fixed" => theme.warning,
        _ => theme.text_muted,
    }
}

fn layout_colors(theme: &Theme) -> [Color; 7] {
    [
        theme.accent,
        theme.success,
        theme.warning,
        theme.text_muted,
        theme.text_bright,
        theme.text_dim,
        theme.text_ui,
    ]
}

/// Shape every release at `width` (the text column, x = 0).
fn build_layout(
    engine: &mut TextEngine,
    theme: &Theme,
    releases: &[Release],
    width: f32,
) -> BodyLayout {
    let mut pieces = Vec::new();
    let mut y = 0.0;
    for (ri, release) in releases.iter().enumerate() {
        if ri > 0 {
            y += 12.0;
            pieces.push(Piece::Rule { y });
            y += 20.0;
            let heading = if release.date.is_empty() {
                format!("v{}", release.version)
            } else {
                format!("v{} · {}", release.version, release.date)
            };
            pieces.push(Piece::Text {
                buffer: Arc::new(label(engine, &heading, 14.0, FONT_SANS, Weight::SEMIBOLD)),
                x: 0.0,
                y,
                h: 19.0,
                color: theme.text_bright,
            });
            y += 28.0;
        }
        if !release.summary.is_empty() {
            let (buffer, h) = paragraph(
                engine,
                &[(&release.summary, Weight::NORMAL, theme.text_ui)],
                width,
            );
            pieces.push(Piece::Text {
                buffer: Arc::new(buffer),
                x: 0.0,
                y,
                h,
                color: theme.text_ui,
            });
            y += h + 16.0;
        }
        for section in &release.sections {
            let color = section_color(theme, &section.title);
            if !section.title.is_empty() {
                let title = section.title.to_uppercase();
                pieces.push(Piece::Text {
                    buffer: Arc::new(label(engine, &title, 11.0, FONT_SANS, Weight::SEMIBOLD)),
                    x: 0.0,
                    y,
                    h: 15.0,
                    color,
                });
                y += 24.0;
            }
            for item in &section.items {
                let mut spans: Vec<(&str, Weight, Color)> = Vec::new();
                if !item.title.is_empty() {
                    spans.push((&item.title, Weight::SEMIBOLD, theme.text_bright));
                    spans.push((" — ", Weight::NORMAL, theme.text_dim));
                }
                spans.push((&item.text, Weight::NORMAL, theme.text_ui));
                let (buffer, h) = paragraph(engine, &spans, width - 18.0);
                pieces.push(Piece::Dot { y: y + 9.5, color });
                pieces.push(Piece::Text {
                    buffer: Arc::new(buffer),
                    x: 18.0,
                    y,
                    h,
                    color: theme.text_ui,
                });
                y += h + 10.0;
            }
            y += 10.0;
        }
    }
    BodyLayout {
        width,
        colors: layout_colors(theme),
        pieces,
        height: y,
    }
}

pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &mut WhatsNewState,
    releases: &[Release],
    w: f32,
    h: f32,
) -> WhatsNewHits {
    layer.rect(
        [0.0, 0.0],
        [w, h],
        Color::hex("#0d0d0f").unwrap().with_alpha(0.72),
        0.0,
    );

    let modal_w = MODAL_W.min(w - 32.0);
    let modal_h = 600.0_f32.min(h - 72.0).max(HEADER_H + FOOTER_H + 80.0);
    let x = ((w - modal_w) / 2.0).round();
    let y = ((h - modal_h) / 2.0).round();
    layer.shadow(
        [x, y + 10.0],
        [modal_w, modal_h],
        Color::BLACK.with_alpha(0.62),
        8.0,
        20.0,
    );
    layer.bordered(
        [x, y],
        [modal_w, modal_h],
        theme.surface,
        Color::WHITE.with_alpha(0.12),
        8.0,
        1.0,
    );

    // ── header: title, version pill, close ──
    layer.text(
        label(engine, "What's new", 19.0, FONT_SANS, Weight::SEMIBOLD),
        x + PAD_X,
        y + 22.0,
        theme.text_bright,
    );
    if let Some(latest) = releases.first() {
        let pill = if latest.date.is_empty() {
            format!("v{}", latest.version)
        } else {
            format!("v{} · {}", latest.version, latest.date)
        };
        let title_w = engine.measure("What's new", 19.0, FONT_SANS, Weight::SEMIBOLD);
        let pill_w = engine.measure(&pill, 11.0, mono_family(), Weight::MEDIUM) + 16.0;
        let px = x + PAD_X + title_w + 12.0;
        layer.rect(
            [px, y + 25.0],
            [pill_w, 20.0],
            theme.accent.with_alpha(0.14),
            4.0,
        );
        layer.text(
            label(engine, &pill, 11.0, mono_family(), Weight::MEDIUM),
            px + 8.0,
            y + 27.5,
            theme.accent,
        );
    }
    let close = [x + modal_w - 16.0 - 28.0, y + 20.0, 28.0, 28.0];
    layer.rect(
        [close[0], close[1]],
        [close[2], close[3]],
        Color::WHITE.with_alpha(0.06),
        4.0,
    );
    super::close_icon(
        layer,
        [close[0] + close[2] / 2.0, close[1] + close[3] / 2.0],
        4.0,
        theme.text_muted,
    );
    layer.hline(x + 1.0, y + HEADER_H, modal_w - 2.0, 1.0, theme.border);

    // ── body: the cached layout, scrolled, culled and clipped ──
    let body_top = y + HEADER_H + 1.0;
    let body_bottom = y + modal_h - FOOTER_H;
    let view_h = body_bottom - body_top;
    let text_x = x + PAD_X;
    let text_w = modal_w - PAD_X * 2.0;
    let stale = state
        .layout
        .as_ref()
        .is_none_or(|l| l.width != text_w || l.colors != layout_colors(theme));
    if stale {
        state.layout = Some(build_layout(engine, theme, releases, text_w));
    }
    state.content_h = state.layout.as_ref().map_or(0.0, |l| l.height) + BODY_PAD * 2.0;
    state.view_h = view_h;
    state.scroll_by(0.0); // re-clamp after a resize
    let layout = state.layout.as_ref().expect("built above");

    let bounds = TextBounds {
        left: x as i32,
        top: body_top as i32,
        right: (x + modal_w) as i32,
        bottom: body_bottom as i32,
    };
    let origin = body_top + BODY_PAD - state.scroll;
    for piece in &layout.pieces {
        match piece {
            Piece::Text {
                buffer,
                x: dx,
                y: dy,
                h: ph,
                color,
            } => {
                let top = origin + dy;
                if top + ph > body_top && top < body_bottom {
                    layer.text_clipped(buffer.clone(), text_x + dx, top, *color, bounds);
                }
            }
            Piece::Rule { y: dy } => {
                let top = origin + dy;
                if top > body_top && top < body_bottom {
                    layer.hline(text_x, top, text_w, 1.0, theme.border);
                }
            }
            // quads can't be clipped: draw a dot only when fully inside
            Piece::Dot { y: dy, color } => {
                let center = origin + dy;
                if center - 3.0 > body_top && center + 3.0 < body_bottom {
                    layer.circle([text_x + 3.0, center], 5.0, *color);
                }
            }
        }
    }

    // scrollbar when the body overflows
    if state.content_h > view_h {
        let track = view_h - 16.0;
        let thumb = (track * view_h / state.content_h).max(24.0);
        let max = state.content_h - view_h;
        let ty = body_top + 8.0 + (track - thumb) * (state.scroll / max);
        layer.rect(
            [x + modal_w - 8.0, ty],
            [3.0, thumb],
            Color::WHITE.with_alpha(0.18),
            1.5,
        );
    }

    // ── footer ──
    layer.hline(x, body_bottom, modal_w, 1.0, theme.border);
    layer.text(
        label(engine, "Esc to close", 11.5, FONT_SANS, Weight::NORMAL),
        x + PAD_X,
        body_bottom + 23.0,
        theme.text_muted,
    );
    let done_w = 96.0;
    let done = [
        x + modal_w - done_w - 20.0,
        body_bottom + 16.0,
        done_w,
        32.0,
    ];
    layer.rect([done[0], done[1]], [done[2], done[3]], theme.accent, 7.0);
    let done_label_w = engine.measure("Got it", 12.5, FONT_SANS, Weight::SEMIBOLD);
    layer.text(
        label(engine, "Got it", 12.5, FONT_SANS, Weight::SEMIBOLD),
        done[0] + (done[2] - done_label_w) / 2.0,
        done[1] + 7.0,
        Color::WHITE,
    );

    WhatsNewHits {
        card: [x, y, modal_w, modal_h],
        close,
        done,
    }
}
