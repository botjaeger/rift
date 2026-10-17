pub mod chip_menu;
pub mod completion;
pub mod confirm;
pub mod context_menu;
pub mod editor;
pub mod explorer;
pub mod glyphs;
pub mod hints;
pub mod history;
pub mod icons;
pub mod markdown_view;
pub mod notifications;
pub mod palette;
pub mod pane_header;
pub mod prompt_chips;
pub mod prompt_editor;
pub mod review_panel;
pub mod search;
pub mod settings;
pub mod subshell;
pub mod tabbar;
pub mod terminal;
pub mod tooltip;
pub mod viewer;
pub mod welcome;
pub mod whats_new;

use rift_core::Color;
use rift_renderer::Layer;

/// A close "×" drawn as two strokes centred on `c`, each arm `half` px long
/// (glyph crosses sit off-centre: their ink box is not the font's line box).
pub fn close_icon(layer: &mut Layer, c: [f32; 2], half: f32, color: Color) {
    let (x, y) = (c[0], c[1]);
    layer.line([x - half, y - half], [x + half, y + half], 1.5, color);
    layer.line([x - half, y + half], [x + half, y - half], 1.5, color);
}

/// Which way a [`chevron`] points.
#[derive(Clone, Copy)]
pub enum Dir {
    Down,
    Right,
}

/// A stroked chevron centred on `c`: `half` px from the tip to either arm
/// along the pointing axis, arms `2 × half` apart (▾ ▸ as strokes, not
/// glyphs, so it sits on the pixel grid the same way in every font).
pub fn chevron(layer: &mut Layer, c: [f32; 2], half: f32, dir: Dir, width: f32, color: Color) {
    let (x, y) = (c[0], c[1]);
    let h = half / 2.0;
    let (a, tip, b) = match dir {
        Dir::Down => ([x - half, y - h], [x, y + h], [x + half, y - h]),
        Dir::Right => ([x - h, y - half], [x + h, y], [x - h, y + half]),
    };
    layer.line(a, tip, width, color);
    layer.line(tip, b, width, color);
}
