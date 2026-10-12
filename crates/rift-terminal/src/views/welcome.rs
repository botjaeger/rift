//! Welcome list on a fresh session: what the shell is, then five ways in
//! (split, palette, history, explorer, settings) with their shortcuts. It
//! shows until the pane's first command runs.

use rift_core::{Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine};

use crate::views::icons::{self, Icon};

/// One entry: icon, name, what it is for, shortcut chips.
pub struct Entry {
    pub icon: Icon,
    pub title: &'static str,
    pub detail: &'static str,
    pub keys: Vec<String>,
}

const WIDTH: f32 = 560.0;
const ROW_H: f32 = 36.0;

fn sans(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.3));
    b.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(FONT_SANS)).weight(weight),
        Shaping::Advanced,
        None,
    );
    b
}

fn lift(theme: &Theme, alpha: f32) -> Color {
    if theme.is_light() {
        Color::BLACK.with_alpha(alpha)
    } else {
        Color::WHITE.with_alpha(alpha)
    }
}

/// The app icon (DESIGN.md "App icon"): a `surface` tile with the `❯` in
/// `accent` and a block cursor in `cursor`, from the 1024-unit icon grid
/// (tile 100–924). At 48 px and below it has no hairline, as the icon does.
pub fn logo(layer: &mut Layer, theme: &Theme, at: [f32; 2], size: f32) {
    let s = size / 824.0;
    let p = |gx: f32, gy: f32| [at[0] + (gx - 100.0) * s, at[1] + (gy - 100.0) * s];
    if size > 48.0 {
        layer.bordered(
            at,
            [size, size],
            theme.surface,
            lift(theme, 0.1),
            185.0 * s,
            1.0,
        );
    } else {
        layer.rect(at, [size, size], theme.surface, 185.0 * s);
    }
    let stroke = 96.0 * s;
    layer.line(p(336.0, 360.0), p(506.0, 512.0), stroke, theme.accent);
    layer.line(p(506.0, 512.0), p(336.0, 664.0), stroke, theme.accent);
    let c0 = p(596.0, 372.0);
    let c1 = p(736.0, 652.0);
    layer.rect(c0, [c1[0] - c0[0], c1[1] - c0[1]], theme.cursor, 16.0 * s);
}

/// The height the list needs; callers skip it when the pane is shorter.
pub fn height(entries: usize) -> f32 {
    40.0 + 20.0 + entries as f32 * (ROW_H + 2.0) + 20.0 + 16.0
}

/// Draw the list centred in `area` (the pane above its command line) and
/// return each row's rect, in `entries` order.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    area: [f32; 4],
    subtitle: &str,
    entries: &[Entry],
    mouse: Option<(f32, f32)>,
) -> Vec<[f32; 4]> {
    let w = WIDTH.min(area[2] - 32.0);
    let x = area[0] + (area[2] - w) / 2.0;
    let total = height(entries.len());
    let mut y = area[1] + ((area[3] - total) * 0.4).max(16.0);

    // a backing of the pane ground keeps the text readable over a theme's
    // background image or a translucent window (invisible on a plain ground)
    layer.rect(
        [x - 20.0, y - 20.0],
        [w + 40.0, total + 24.0],
        theme.bg.with_alpha(0.94),
        8.0,
    );

    // header: the Rift logo, "New session", the shell and where it is
    logo(layer, theme, [x, y], 40.0);
    layer.text(
        sans(engine, "New session", 17.0, Weight::SEMIBOLD),
        x + 52.0,
        y + 1.0,
        theme.text_bright,
    );
    let sub = super::palette::elide_to(engine, subtitle, 12.5, FONT_SANS, w - 52.0);
    layer.text(
        sans(engine, &sub, 12.5, Weight::NORMAL),
        x + 52.0,
        y + 23.0,
        theme.text_dim,
    );
    y += 40.0 + 20.0;

    let mut rects = Vec::with_capacity(entries.len());
    for e in entries {
        let r = [x, y, w, ROW_H];
        let hot = mouse.is_some_and(|(mx, my)| {
            mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
        });
        if hot {
            layer.rect([r[0], r[1]], [r[2], r[3]], lift(theme, 0.05), 4.0);
        }
        let cy = y + ROW_H / 2.0;
        layer.rect([x + 12.0, cy - 14.0], [28.0, 28.0], lift(theme, 0.05), 4.0);
        icons::draw(layer, e.icon, [x + 26.0, cy], 16.0, theme.text_dim);
        layer.text(
            sans(engine, e.title, 13.0, Weight::NORMAL),
            x + 52.0,
            cy - 8.5,
            theme.text_ui,
        );
        // shortcut chips at the right, the description between
        let mut kx = x + w - 12.0;
        for k in e.keys.iter().rev() {
            let kw = (engine.measure(k, 11.0, FONT_SANS, Weight::NORMAL) + 12.0).max(20.0);
            kx -= kw;
            layer.bordered(
                [kx, cy - 10.0],
                [kw, 20.0],
                lift(theme, 0.05),
                lift(theme, 0.14),
                3.0,
                1.0,
            );
            let tw = engine.measure(k, 11.0, FONT_SANS, Weight::NORMAL);
            layer.text(
                sans(engine, k, 11.0, Weight::NORMAL),
                (kx + (kw - tw) / 2.0).round(),
                cy - 7.0,
                theme.text_dim,
            );
            kx -= 4.0;
        }
        let tx = x + 52.0 + engine.measure(e.title, 13.0, FONT_SANS, Weight::NORMAL) + 12.0;
        let room = kx - 8.0 - tx;
        if room > 40.0 {
            let d = super::palette::elide_to(engine, e.detail, 13.0, FONT_SANS, room);
            layer.text(
                sans(engine, &d, 13.0, Weight::NORMAL),
                tx,
                cy - 8.5,
                theme.text_dim,
            );
        }
        rects.push(r);
        y += ROW_H + 2.0;
    }
    layer.text(
        sans(
            engine,
            "Shown on a new session until the first command runs.",
            12.0,
            Weight::NORMAL,
        ),
        x,
        y + 18.0,
        theme.text_dim,
    );
    rects
}
