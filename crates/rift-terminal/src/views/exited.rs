//! The bar over a pane whose program exited and stays (`close_on_exit`):
//! "Process exited with code N", Restart (⏎) and Close (⌘W).

use rift_core::{Color, Theme};
use rift_renderer::glyphon::Weight;
use rift_renderer::{FONT_SANS, Layer, TextEngine};

use crate::views::tabbar::make_text;

const H: f32 = 36.0;
const MARGIN: f32 = 10.0;
const BTN_H: f32 = 24.0;
const BTN_PAD: f32 = 10.0;

/// What the bar's buttons do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Restart,
    Close,
}

/// The code of a pane whose session service stopped answering: its shell
/// may still run there, and the bar offers to reconnect rather than
/// restart.
pub const LOST: i32 = i32::MIN;

/// The bar's message for exit `code` (-1: none known; [`LOST`]).
pub fn message(code: i32) -> String {
    if code == LOST {
        "Lost the session service; the shell may still be running".into()
    } else if code < 0 {
        "Process exited".into()
    } else {
        format!("Process exited with code {code}")
    }
}

/// The main button's label: Reconnect for a lost pane, else Restart.
pub fn restart_label(code: i32) -> &'static str {
    if code == LOST { "Reconnect" } else { "Restart" }
}

/// Draw the bar over the bottom of `pane` and return the buttons' rects.
/// `close_keys` is the close shortcut (⌘W), shown on its button.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    pane: [f32; 4],
    code: i32,
    close_keys: &str,
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
    // a failure is edged in the error colour, a clean exit in the border's
    let edge = if code == 0 {
        theme.border
    } else {
        theme.error.with_alpha(0.7)
    };
    layer.bordered([x, y], [w, H], theme.surface, edge, 6.0, 1.0);
    let inside = |r: [f32; 4]| {
        mouse.is_some_and(|(mx, my)| {
            mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
        })
    };
    let mut hits = Vec::new();
    let by = y + (H - BTN_H) / 2.0;
    // right to left: Close, Restart
    let close_label = if close_keys.is_empty() {
        "Close".to_string()
    } else {
        format!("Close  {close_keys}")
    };
    let close_w = engine.measure(&close_label, 12.0, FONT_SANS, Weight::MEDIUM) + 2.0 * BTN_PAD;
    let close = [x + w - 8.0 - close_w, by, close_w, BTN_H];
    if inside(close) {
        layer.rect(
            [close[0], by],
            [close_w, BTN_H],
            theme.text_ui.with_alpha(0.08),
            4.0,
        );
    }
    let b = make_text(engine, &close_label, 12.0, FONT_SANS, Weight::MEDIUM);
    layer.text(b, close[0] + BTN_PAD, by + 5.0, theme.text_ui);
    hits.push((close, Part::Close));
    let restart_label = format!(
        "{}  {}",
        restart_label(code),
        rift_core::keybindings::cap_for("⏎", rift_core::keybindings::Platform::current())
    );
    let restart_label = restart_label.as_str();
    let restart_w = engine.measure(restart_label, 12.0, FONT_SANS, Weight::MEDIUM) + 2.0 * BTN_PAD;
    let restart = [close[0] - 8.0 - restart_w, by, restart_w, BTN_H];
    layer.rect(
        [restart[0], by],
        [restart_w, BTN_H],
        if inside(restart) {
            theme.accent
        } else {
            theme.accent.with_alpha(0.85)
        },
        4.0,
    );
    let b = make_text(engine, restart_label, 12.0, FONT_SANS, Weight::MEDIUM);
    layer.text(b, restart[0] + BTN_PAD, by + 5.0, theme.bg);
    hits.push((restart, Part::Restart));
    let room = restart[0] - 12.0 - (x + 14.0);
    let text = super::palette::elide_to(engine, &message(code), 13.0, FONT_SANS, room);
    let b = make_text(engine, &text, 13.0, FONT_SANS, Weight::NORMAL);
    layer.text(b, x + 14.0, y + (H - 16.9) / 2.0, theme.text_bright);
    hits
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_message_names_the_code_when_known() {
        assert_eq!(super::message(3), "Process exited with code 3");
        assert_eq!(super::message(0), "Process exited with code 0");
        assert_eq!(super::message(-1), "Process exited");
        assert!(super::message(super::LOST).starts_with("Lost the session service"));
        assert_eq!(super::restart_label(super::LOST), "Reconnect");
        assert_eq!(super::restart_label(1), "Restart");
    }
}
