//! The subshell offer (Warp "Warpify subshell"): a bar at the bottom of a
//! pane whose command started a subshell Rift's hooks do not reach
//! (`crate::subshell`): enable them there (⌘⏎), never ask for this command
//! again, or dismiss.

use rift_core::{Color, Theme};
use rift_renderer::glyphon::Weight;
use rift_renderer::{FONT_SANS, Layer, TextEngine};

use crate::views::tabbar::make_text;

const H: f32 = 36.0;
const MARGIN: f32 = 10.0;
const BTN_H: f32 = 24.0;
const BTN_PAD: f32 = 10.0;

/// What the bar's controls do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Enable,
    Never,
    Dismiss,
}

/// Draw the bar over the bottom of `pane` (its grid area) and return the
/// controls' rects.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    pane: [f32; 4],
    enable_keys: &str,
    mouse: Option<(f32, f32)>,
) -> Vec<([f32; 4], Part)> {
    let [px, py, pw, ph] = pane;
    let w = (pw - 2.0 * MARGIN).min(640.0);
    if w < 200.0 || ph < H + 2.0 * MARGIN {
        return Vec::new();
    }
    let (x, y) = (px + (pw - w) / 2.0, py + ph - MARGIN - H);
    layer.shadow(
        [x, y + 4.0],
        [w, H],
        Color::BLACK.with_alpha(0.45),
        6.0,
        10.0,
    );
    // opaque over the rows under it, whatever the window's opacity
    layer.bordered(
        [x, y],
        [w, H],
        theme.surface,
        theme.accent.with_alpha(0.6),
        6.0,
        1.0,
    );
    let inside = |r: [f32; 4]| {
        mouse.is_some_and(|(mx, my)| {
            mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
        })
    };
    let mut hits = Vec::new();
    // right to left: ✕, Don't ask again, Enable
    let by = y + (H - BTN_H) / 2.0;
    let close = [x + w - 8.0 - 20.0, y + (H - 20.0) / 2.0, 20.0, 20.0];
    super::close_icon(
        layer,
        [close[0] + 10.0, close[1] + 10.0],
        4.0,
        if inside(close) {
            theme.text_bright
        } else {
            theme.text_dim
        },
    );
    hits.push((close, Part::Dismiss));
    let never_label = "Don't ask again";
    let never_w = engine.measure(never_label, 12.0, FONT_SANS, Weight::MEDIUM) + 2.0 * BTN_PAD;
    let never = [close[0] - 6.0 - never_w, by, never_w, BTN_H];
    if inside(never) {
        layer.rect(
            [never[0], by],
            [never_w, BTN_H],
            theme.text_ui.with_alpha(0.08),
            4.0,
        );
    }
    let b = make_text(engine, never_label, 12.0, FONT_SANS, Weight::MEDIUM);
    layer.text(b, never[0] + BTN_PAD, by + 5.0, theme.text_ui);
    hits.push((never, Part::Never));
    let enable_label = format!("Enable  {enable_keys}");
    let enable_w = engine.measure(&enable_label, 12.0, FONT_SANS, Weight::MEDIUM) + 2.0 * BTN_PAD;
    let enable = [never[0] - 8.0 - enable_w, by, enable_w, BTN_H];
    layer.rect(
        [enable[0], by],
        [enable_w, BTN_H],
        if inside(enable) {
            theme.accent
        } else {
            theme.accent.with_alpha(0.85)
        },
        4.0,
    );
    let b = make_text(engine, &enable_label, 12.0, FONT_SANS, Weight::MEDIUM);
    layer.text(b, enable[0] + BTN_PAD, by + 5.0, theme.bg);
    hits.push((enable, Part::Enable));
    // the question, cut to the room left of the buttons
    let room = enable[0] - 12.0 - (x + 14.0);
    let text = super::palette::elide_to(
        engine,
        "Use Rift's blocks and editor in this subshell?",
        13.0,
        FONT_SANS,
        room,
    );
    let b = make_text(engine, &text, 13.0, FONT_SANS, Weight::NORMAL);
    layer.text(b, x + 14.0, y + (H - 16.9) / 2.0, theme.text_bright);
    hits
}
