//! Hover tooltips: what an icon control does (with its shortcut from the live
//! keymap), and the full text of a label the chrome cuts short (tab title and
//! directory, explorer path), and a hover card for a prompt chip: its name
//! over what it shows and what a click does. Shown after the pointer rests on one control for
//! [`DELAY`]; moving straight to a neighbour while one shows switches at once
//! (macOS). A press, key or wheel hides it until the pointer moves on.

use std::time::Duration;

use shift_core::{Color, Theme};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight, Wrap};
use shift_renderer::{FONT_SANS, Layer, TextEngine};

/// Rest time before a tooltip shows.
pub const DELAY: Duration = Duration::from_millis(600);
/// After one hides, a tooltip for the next control shows without delay
/// within this window.
pub const WARM: Duration = Duration::from_millis(400);

const FONT: f32 = 12.0;
const PAD_X: f32 = 8.0;
const H: f32 = 24.0;
/// Gap between the control and the tooltip.
const GAP: f32 = 6.0;
const MAX_W: f32 = 420.0;
/// Width of a card with a body, and the gap between its title and body.
const CARD_W: f32 = 300.0;
const BODY_GAP: f32 = 4.0;
const MARGIN: f32 = 8.0;

/// One control's tooltip: `rect` is the control, `text` what it does or its
/// full label, `detail` the shortcut or a path, drawn muted at the right.
#[derive(Debug, Clone, PartialEq)]
pub struct Tip {
    pub rect: [f32; 4],
    pub text: String,
    pub detail: Option<String>,
    /// Wrapped muted lines under `text` (a hover card).
    pub body: Option<String>,
}

impl Tip {
    pub fn new(rect: [f32; 4], text: impl Into<String>, detail: Option<String>) -> Self {
        Tip {
            rect,
            text: text.into(),
            detail,
            body: None,
        }
    }

    /// A hover card: `text` as its title over `body`.
    pub fn card(rect: [f32; 4], text: impl Into<String>, body: impl Into<String>) -> Self {
        Tip {
            body: Some(body.into()),
            ..Tip::new(rect, text, None)
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        let [rx, ry, rw, rh] = self.rect;
        x >= rx && x < rx + rw && y >= ry && y < ry + rh
    }
}

/// Top-left of a `size` tooltip for the control `rect` in a `w`×`h` window:
/// centred under the control, above it when there is no room below, kept
/// inside the window.
pub fn place(rect: [f32; 4], size: [f32; 2], w: f32, h: f32) -> [f32; 2] {
    let [rx, ry, rw, rh] = rect;
    let x = (rx + rw / 2.0 - size[0] / 2.0)
        .min(w - MARGIN - size[0])
        .max(MARGIN);
    let below = ry + rh + GAP;
    let y = if below + size[1] <= h - MARGIN {
        below
    } else {
        (ry - GAP - size[1]).max(MARGIN)
    };
    [x, y]
}

fn sans(engine: &mut TextEngine, text: &str, weight: Weight) -> Buffer {
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(FONT, FONT * 1.35));
    b.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(FONT_SANS)).weight(weight),
        Shaping::Advanced,
        None,
    );
    b
}

/// Draw `tip` on `layer` (the topmost) in a `w`×`h` window; returns its rect.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    tip: &Tip,
    w: f32,
    h: f32,
) -> [f32; 4] {
    if let Some(body) = &tip.body {
        return render_card(layer, engine, theme, tip, body, w, h);
    }
    let detail = tip.detail.as_deref().unwrap_or("");
    let gap = if detail.is_empty() { 0.0 } else { 10.0 };
    // a detail (often a path) keeps its end, the label its start
    let detail = crate::views::editor::elide_head(detail, MAX_W / 2.0, |s| {
        engine.measure(s, FONT, FONT_SANS, Weight::NORMAL)
    });
    let detail_w = if detail.is_empty() {
        0.0
    } else {
        engine.measure(&detail, FONT, FONT_SANS, Weight::NORMAL)
    };
    let room = (MAX_W - 2.0 * PAD_X - gap - detail_w).max(40.0);
    let text = crate::views::palette::elide_to(engine, &tip.text, FONT, FONT_SANS, room);
    let text_w = engine.measure(&text, FONT, FONT_SANS, Weight::NORMAL);
    let size = [text_w + gap + detail_w + 2.0 * PAD_X, H];
    let [x, y] = place(tip.rect, size, w, h);

    plate(layer, theme, [x, y], size);
    let ty = y + (H - FONT * 1.35) / 2.0;
    layer.text(
        sans(engine, &text, Weight::NORMAL),
        x + PAD_X,
        ty,
        theme.text_bright,
    );
    if !detail.is_empty() {
        layer.text(
            sans(engine, &detail, Weight::NORMAL),
            x + PAD_X + text_w + gap,
            ty,
            theme.text_muted,
        );
    }
    [x, y, size[0], size[1]]
}

/// The surface, shadow and hairline every tooltip sits on.
fn plate(layer: &mut Layer, theme: &Theme, [x, y]: [f32; 2], size: [f32; 2]) {
    layer.shadow(
        [x, y + 3.0],
        size,
        Color::BLACK.with_alpha(if theme.is_light() { 0.2 } else { 0.4 }),
        4.0,
        10.0,
    );
    let edge = if theme.is_light() {
        Color::BLACK.with_alpha(0.12)
    } else {
        Color::WHITE.with_alpha(0.1)
    };
    layer.bordered([x, y], size, theme.surface, edge, 4.0, 1.0);
}

/// A hover card: the title in medium weight, the body wrapped under it.
fn render_card(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    tip: &Tip,
    body: &str,
    w: f32,
    h: f32,
) -> [f32; 4] {
    let line_h = FONT * 1.35;
    let inner = CARD_W - 2.0 * PAD_X;
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(FONT, line_h));
    b.set_wrap(&mut engine.font_system, Wrap::Word);
    b.set_size(&mut engine.font_system, Some(inner), None);
    b.set_text(
        &mut engine.font_system,
        body,
        &Attrs::new().family(Family::Name(FONT_SANS)),
        Shaping::Advanced,
        None,
    );
    b.shape_until_scroll(&mut engine.font_system, false);
    let lines = b.layout_runs().count().max(1) as f32;
    let pad_y = (H - line_h) / 2.0;
    let size = [CARD_W, 2.0 * pad_y + line_h + BODY_GAP + lines * line_h];
    let [x, y] = place(tip.rect, size, w, h);
    plate(layer, theme, [x, y], size);
    layer.text(
        sans(engine, &tip.text, Weight::MEDIUM),
        x + PAD_X,
        y + pad_y,
        theme.text_bright,
    );
    layer.text(b, x + PAD_X, y + pad_y + line_h + BODY_GAP, theme.text_dim);
    [x, y, size[0], size[1]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sits_under_the_control_inside_the_window() {
        // centred under a 28px button
        assert_eq!(
            place([100.0, 10.0, 28.0, 28.0], [60.0, 24.0], 800.0, 600.0),
            [84.0, 44.0]
        );
        // at the right edge: pulled in
        assert_eq!(
            place([780.0, 10.0, 20.0, 20.0], [100.0, 24.0], 800.0, 600.0),
            [692.0, 36.0]
        );
        // at the left edge: pushed in
        assert_eq!(
            place([0.0, 10.0, 20.0, 20.0], [100.0, 24.0], 800.0, 600.0),
            [8.0, 36.0]
        );
        // no room below: above the control
        assert_eq!(
            place([100.0, 560.0, 20.0, 30.0], [60.0, 24.0], 800.0, 600.0),
            [80.0, 530.0]
        );
    }
}
