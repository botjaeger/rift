//! ⌃Tab's most-recently-used tab switcher (`tab_switch_order = "mru"`):
//! a card of the window's tabs, last used first, while ⌃ is held. Each row
//! shows the tab's title and its directory; the highlighted row is the tab
//! releasing ⌃ switches to.

use rift_core::{Color, Theme};
use rift_renderer::glyphon::Weight;
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use super::palette::{elide_to, lift, sans};

const CARD_W: f32 = 460.0;
const ROW_H: f32 = 44.0;
const PAD: f32 = 6.0;
const HEAD_H: f32 = 30.0;

/// One row: the tab's title and its directory (may be empty).
pub struct Row {
    pub title: String,
    pub cwd: String,
}

/// Draw the card centred under the tab bar; the drawn rows (index, rect).
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    rows: &[Row],
    highlight: usize,
    w: f32,
    h: f32,
) -> Vec<(usize, [f32; 4])> {
    let card_w = CARD_W.min((w - 32.0).max(160.0));
    // as many rows as fit; the highlighted one stays in view
    let room = h - crate::views::tabbar::TAB_BAR_H - 54.0 - HEAD_H - PAD * 2.0 - 16.0;
    let max_rows = ((room / ROW_H).max(1.0) as usize).max(1);
    let shown = rows.len().min(max_rows);
    let start = (highlight + 1)
        .saturating_sub(max_rows)
        .min(rows.len().saturating_sub(shown));
    let card_h = HEAD_H + PAD * 2.0 + ROW_H * shown as f32;
    let x = (w - card_w) / 2.0;
    let y = crate::views::tabbar::TAB_BAR_H + 54.0;
    layer.shadow(
        [x, y + 12.0],
        [card_w, card_h],
        Color::BLACK.with_alpha(0.55),
        8.0,
        24.0,
    );
    layer.bordered(
        [x, y],
        [card_w, card_h],
        theme.surface,
        lift(theme, 0.1),
        8.0,
        1.0,
    );
    layer.text(
        sans(engine, "RECENT TABS", 11.0, Weight::SEMIBOLD),
        x + 16.0,
        y + 9.0,
        theme.text_muted,
    );
    layer.hline(x + 1.0, y + HEAD_H, card_w - 2.0, 1.0, theme.border);
    let mut out = Vec::with_capacity(shown);
    let mut ry = y + HEAD_H + PAD;
    for (i, row) in rows.iter().enumerate().skip(start).take(shown) {
        let r = [x + PAD, ry, card_w - PAD * 2.0, ROW_H];
        let on = i == highlight;
        if on {
            layer.rect(
                [r[0], r[1]],
                [r[2], r[3]],
                theme.accent.with_alpha(0.16),
                4.0,
            );
        }
        let text_w = r[2] - 20.0;
        let title = elide_to(engine, &row.title, 13.5, FONT_SANS, text_w);
        layer.text(
            sans(engine, &title, 13.5, Weight::NORMAL),
            r[0] + 10.0,
            ry + 5.0,
            if on { theme.text_bright } else { theme.text_ui },
        );
        if !row.cwd.is_empty() {
            let cwd = elide_to(engine, &row.cwd, 11.0, mono_family(), text_w);
            layer.text(
                super::palette::mono(engine, &cwd, 11.0, Weight::NORMAL),
                r[0] + 10.0,
                ry + 24.0,
                theme.text_muted,
            );
        }
        out.push((i, r));
        ry += ROW_H;
    }
    out
}
