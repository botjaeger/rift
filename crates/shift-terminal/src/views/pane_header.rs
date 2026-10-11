//! Title bar of a split pane (Warp pane header): a terminal icon, the
//! pane's title centred (a spinner before it while a command runs), and split / ⋮ (pane menu) / × (close) on the right
//! while the pane is focused or hovered. Pressing the bar and dragging moves
//! the pane (see `App::drop_pane`); the drop zone and the dragged chip are
//! drawn here too.

use shift_core::{Color, Theme};
use shift_pty::PaneId;
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use shift_renderer::{FONT_SANS, Layer, TextEngine};

use crate::state::{ClickTarget, Hit};
use crate::views::glyphs;
use crate::views::icons::{self, Icon};

const TITLE_SIZE: f32 = 12.0;
/// Hit square of the header buttons.
const BUTTON: f32 = 24.0;

fn sans(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.3));
    buf.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(FONT_SANS)).weight(weight),
        Shaping::Advanced,
        None,
    );
    buf
}

/// Hover lift: white on dark themes, black on light ones.
fn lift(theme: &Theme, alpha: f32) -> Color {
    if theme.is_light() {
        Color::BLACK.with_alpha(alpha)
    } else {
        Color::WHITE.with_alpha(alpha)
    }
}

/// Draw the bar over `rect` (the pane's top `rect[3]` px) and report its
/// targets: the buttons first, then the whole bar as the drag handle.
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    rect: [f32; 4],
    title: &str,
    active: bool,
    running: bool,
    mouse: Option<(f32, f32)>,
    id: PaneId,
    hits: &mut Vec<Hit>,
) {
    let [x, y, w, h] = rect;
    let cy = y + h / 2.0;
    let over = |r: [f32; 4]| {
        mouse.is_some_and(|(mx, my)| {
            mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
        })
    };
    let hovered = over(rect);

    // a shade under the pane ground; the focused pane's carries an accent edge
    layer.rect([x, y], [w, h], Color::BLACK.with_alpha(0.08), 0.0);
    if active {
        layer.rect([x, y], [w, 2.0], theme.accent, 0.0);
    }
    icons::draw(
        layer,
        Icon::Terminal,
        [x + 8.0 + 7.0, cy],
        14.0,
        if active { theme.accent } else { theme.text_dim },
    );

    // split, ⋮ and × at the right edge: shown while focused or hovered
    let close = [x + w - 4.0 - BUTTON, cy - BUTTON / 2.0, BUTTON, BUTTON];
    let menu = [close[0] - 2.0 - BUTTON, close[1], BUTTON, BUTTON];
    let split = [menu[0] - 2.0 - BUTTON, close[1], BUTTON, BUTTON];
    if active || hovered {
        for (r, icon, size, target) in [
            (split, Icon::SplitRight, 14.0, ClickTarget::PaneSplit(id)),
            (menu, Icon::More, 14.0, ClickTarget::PaneMenu(id)),
            (close, Icon::Close, 11.0, ClickTarget::PaneClose(id)),
        ] {
            let hot = over(r);
            if hot {
                layer.rect([r[0], r[1]], [r[2], r[3]], lift(theme, 0.08), 4.0);
            }
            icons::draw(
                layer,
                icon,
                [r[0] + BUTTON / 2.0, cy],
                size,
                if hot { theme.text_ui } else { theme.text_dim },
            );
            hits.push(Hit { rect: r, target });
        }
    }

    // the title centres on the pane, elided clear of the buttons; a running
    // command's spinner leads it
    let spin_w = if running { 13.0 + 6.0 } else { 0.0 };
    let room = (w - 2.0 * (x + w - split[0]) - 8.0 - spin_w).max(0.0);
    let measure =
        |engine: &mut TextEngine, s: &str| engine.measure(s, TITLE_SIZE, FONT_SANS, Weight::NORMAL);
    let label = super::editor::elide_head(title, room, |s| measure(engine, s));
    if room > 0.0 && !label.is_empty() {
        let tw = measure(engine, &label);
        let color = if active {
            theme.text_bright
        } else {
            theme.text_dim
        };
        let lx = (x + (w - tw - spin_w) / 2.0).round();
        if running {
            glyphs::draw_spinner(layer, [lx + 6.5, cy], 13.0, color);
        }
        layer.text(
            sans(engine, &label, TITLE_SIZE, Weight::NORMAL),
            lx + spin_w,
            (cy - TITLE_SIZE * 1.3 / 2.0).round(),
            color,
        );
    }
    hits.push(Hit {
        rect,
        target: ClickTarget::PaneHeader(id),
    });
}

/// The drop zone under a dragged pane: an accent wash and edge, with what
/// the drop does (`Split right`, `Swap`, `New tab`) on a pill in its middle.
pub fn drop_zone(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    zone: [f32; 4],
    label: &str,
) {
    let [zx, zy, zw, zh] = zone;
    layer.bordered(
        [zx + 1.0, zy + 1.0],
        [zw - 2.0, zh - 2.0],
        theme.accent.with_alpha(0.16),
        theme.accent,
        0.0,
        1.5,
    );
    if label.is_empty() {
        return;
    }
    let tw = engine.measure(label, 12.0, FONT_SANS, Weight::SEMIBOLD);
    let (pw, ph) = (tw + 20.0, 24.0);
    if pw > zw - 8.0 || ph > zh - 8.0 {
        return;
    }
    let (px, py) = (zx + (zw - pw) / 2.0, zy + (zh - ph) / 2.0);
    layer.rect([px, py], [pw, ph], theme.accent, 4.0);
    layer.text(
        sans(engine, label, 12.0, Weight::SEMIBOLD),
        (px + 10.0).round(),
        (py + (ph - 15.6) / 2.0).round(),
        theme.bg,
    );
}

/// The chip that follows the pointer while a pane is dragged: a grip, the
/// pane's icon and its title, kept inside the window.
#[allow(clippy::too_many_arguments)]
pub fn drag_ghost(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    title: &str,
    viewer: bool,
    at: (f32, f32),
    w: f32,
    h: f32,
) {
    let (gw, gh) = (220.0, 34.0);
    let x = at.0.min(w - gw - 4.0).max(4.0);
    let y = at.1.min(h - gh - 4.0).max(4.0);
    layer.shadow([x, y], [gw, gh], Color::BLACK.with_alpha(0.55), 4.0, 14.0);
    layer.bordered([x, y], [gw, gh], theme.surface, lift(theme, 0.14), 4.0, 1.0);
    let cy = y + gh / 2.0;
    icons::draw(layer, Icon::Grip, [x + 17.0, cy], 14.0, theme.text_dim);
    icons::draw(
        layer,
        if viewer { Icon::File } else { Icon::Terminal },
        [x + 35.0, cy],
        14.0,
        theme.accent,
    );
    let room = gw - 52.0 - 10.0;
    let measure =
        |engine: &mut TextEngine, s: &str| engine.measure(s, TITLE_SIZE, FONT_SANS, Weight::NORMAL);
    let label = super::editor::elide_head(title, room, |s| measure(engine, s));
    layer.text(
        sans(engine, &label, TITLE_SIZE, Weight::NORMAL),
        x + 50.0,
        (cy - TITLE_SIZE * 1.3 / 2.0).round(),
        theme.text_ui,
    );
}
