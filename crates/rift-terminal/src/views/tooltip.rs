//! Hover tooltips: what an icon control does (with its shortcut from the live
//! keymap), and the full text of a label the chrome cuts short (tab title and
//! directory, explorer path), and a hover card for a prompt chip: its name
//! over what it shows and what a click does. Shown after the pointer rests on one control for
//! [`DELAY`]; moving straight to a neighbour while one shows switches at once
//! (macOS). A press, key or wheel hides it until the pointer moves on.

use std::time::Duration;

use rift_core::{Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight, Wrap};
use rift_renderer::{FONT_SANS, Layer, TextEngine};

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
/// A card's button: height, label padding, gap above it.
const BTN_H: f32 = 24.0;
const BTN_PAD_X: f32 = 12.0;
const BTN_GAP: f32 = 8.0;

/// One control's tooltip: `rect` is the control, `text` what it does or its
/// full label, `detail` the shortcut or a path, drawn muted at the right.
#[derive(Debug, Clone, PartialEq)]
pub struct Tip {
    pub rect: [f32; 4],
    pub text: String,
    pub detail: Option<String>,
    /// Wrapped muted lines under `text` (a hover card).
    pub body: Option<String>,
    /// The label of a button at the foot of a hover card. The card is then
    /// interactive: it stays open while the pointer moves from the control
    /// into it.
    pub action: Option<String>,
}

impl Tip {
    pub fn new(rect: [f32; 4], text: impl Into<String>, detail: Option<String>) -> Self {
        Tip {
            rect,
            text: text.into(),
            detail,
            body: None,
            action: None,
        }
    }

    /// A hover card: `text` as its title over `body`.
    pub fn card(rect: [f32; 4], text: impl Into<String>, body: impl Into<String>) -> Self {
        Tip {
            body: Some(body.into()),
            ..Tip::new(rect, text, None)
        }
    }

    /// A hover card with a button.
    pub fn with_action(mut self, label: impl Into<String>) -> Self {
        self.action = Some(label.into());
        self
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        let [rx, ry, rw, rh] = self.rect;
        x >= rx && x < rx + rw && y >= ry && y < ry + rh
    }
}

/// Whether the pointer at (`x`, `y`) is on the control `rect`, on the shown
/// card `card`, or in the gap between them: an interactive card stays open
/// while the pointer travels from the control into it.
pub fn bridged(rect: [f32; 4], card: [f32; 4], x: f32, y: f32) -> bool {
    let inside = |r: [f32; 4]| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3];
    let left = rect[0].min(card[0]);
    let right = (rect[0] + rect[2]).max(card[0] + card[2]);
    let (top, bottom) = if card[1] >= rect[1] {
        (rect[1] + rect[3], card[1])
    } else {
        (card[1] + card[3], rect[1])
    };
    inside(rect) || inside(card) || inside([left, top, right - left, (bottom - top).max(0.0)])
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

/// Where a tooltip was drawn: its rect, and its button's when it has one.
pub struct Drawn {
    pub rect: [f32; 4],
    pub action: Option<[f32; 4]>,
}

/// Draw `tip` on `layer` (the topmost) in a `w`×`h` window; `mouse` lights
/// the button under it.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    tip: &Tip,
    (w, h): (f32, f32),
    mouse: (f32, f32),
) -> Drawn {
    if let Some(body) = &tip.body {
        return render_card(layer, engine, theme, tip, body, (w, h), mouse);
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
    Drawn {
        rect: [x, y, size[0], size[1]],
        action: None,
    }
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
    (w, h): (f32, f32),
    mouse: (f32, f32),
) -> Drawn {
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
    let btn_w = tip
        .action
        .as_ref()
        .map(|a| engine.measure(a, FONT, FONT_SANS, Weight::MEDIUM) + 2.0 * BTN_PAD_X);
    let foot = if btn_w.is_some() {
        BTN_GAP + BTN_H + 6.0 - pad_y
    } else {
        0.0
    };
    let size = [
        CARD_W,
        2.0 * pad_y + line_h + BODY_GAP + lines * line_h + foot,
    ];
    let [x, y] = place(tip.rect, size, w, h);
    plate(layer, theme, [x, y], size);
    layer.text(
        sans(engine, &tip.text, Weight::MEDIUM),
        x + PAD_X,
        y + pad_y,
        theme.text_bright,
    );
    layer.text(b, x + PAD_X, y + pad_y + line_h + BODY_GAP, theme.text_dim);
    // a secondary button (DESIGN.md): 6 % lift, 12 % hairline, medium label
    let action = btn_w.zip(tip.action.as_deref()).map(|(bw, label)| {
        let r = [x + PAD_X, y + size[1] - 6.0 - BTN_H, bw, BTN_H];
        let on =
            mouse.0 >= r[0] && mouse.0 < r[0] + r[2] && mouse.1 >= r[1] && mouse.1 < r[1] + r[3];
        let lift = if theme.is_light() {
            Color::BLACK.with_alpha(if on { 0.14 } else { 0.06 })
        } else {
            Color::WHITE.with_alpha(if on { 0.14 } else { 0.06 })
        };
        let edge = if theme.is_light() {
            Color::BLACK.with_alpha(0.12)
        } else {
            Color::WHITE.with_alpha(0.12)
        };
        layer.bordered([r[0], r[1]], [r[2], r[3]], lift, edge, 4.0, 1.0);
        layer.text(
            sans(engine, label, Weight::MEDIUM),
            r[0] + BTN_PAD_X,
            r[1] + (BTN_H - FONT * 1.35) / 2.0,
            theme.text_bright,
        );
        r
    });
    Drawn {
        rect: [x, y, size[0], size[1]],
        action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_stays_open_from_the_control_into_it() {
        let chip = [100.0, 10.0, 60.0, 20.0];
        let card = [60.0, 36.0, 300.0, 90.0];
        assert!(bridged(chip, card, 110.0, 20.0)); // on the chip
        assert!(bridged(chip, card, 110.0, 33.0)); // in the gap
        assert!(bridged(chip, card, 200.0, 100.0)); // on the card
        assert!(!bridged(chip, card, 400.0, 33.0)); // beside the gap
        assert!(!bridged(chip, card, 110.0, 200.0)); // below the card
        // a card above the chip
        let above = [60.0, 100.0, 300.0, 50.0];
        let chip = [100.0, 160.0, 60.0, 20.0];
        assert!(bridged(chip, above, 120.0, 155.0));
        assert!(!bridged(chip, above, 120.0, 190.0));
    }

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
