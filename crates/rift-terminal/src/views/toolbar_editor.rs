//! "Edit toolbar": put the strip's toolbar buttons left of the tabs or at its
//! right end, in order, or take them off, in the arranger dialog
//! (`arranger.rs`): the hidden buttons sit in a palette, the shown ones in
//! two dashed buckets, Left and Right, side by side.

use rift_core::{Config, Theme, ToolbarItem};
use rift_renderer::glyphon::Weight;
use rift_renderer::{FONT_SANS, Layer, TextEngine};

use crate::views::arranger::{self, Arranger, Look};
use crate::views::palette::sans;

pub type ToolbarEditor = Arranger<ToolbarItem>;
pub type Layout = arranger::Layout<ToolbarItem>;

/// The Left and Right buckets' places.
pub const LEFT: usize = 0;
pub const RIGHT: usize = 1;

const ITEM_H: f32 = 28.0;
const ICON_W: f32 = 16.0;

/// The dialog over the toolbar as `config` has it.
pub fn new(config: &Config) -> ToolbarEditor {
    let (left, right) = config.toolbar();
    let defaults = Config::default().toolbar();
    Arranger::new(
        vec![left, right],
        vec![defaults.0, defaults.1],
        ToolbarItem::ALL.to_vec(),
    )
}

/// The draft as `(toolbar_left, toolbar_right)`.
pub fn sides(ed: &ToolbarEditor) -> (Vec<ToolbarItem>, Vec<ToolbarItem>) {
    (
        ed.buckets.get(LEFT).cloned().unwrap_or_default(),
        ed.buckets.get(RIGHT).cloned().unwrap_or_default(),
    )
}

/// Draw `item`'s toolbar mark centred on `c`: its icon, or ± for code
/// review (the strip's own button shows `+added −removed` there).
pub fn draw_mark(layer: &mut Layer, item: ToolbarItem, c: [f32; 2], color: rift_core::Color) {
    match crate::views::tabbar::toolbar_icon(item) {
        Some(icon) => crate::views::icons::draw(layer, icon, c, ICON_W, color),
        None => crate::views::tabbar::draw_plus_minus(layer, c, color),
    }
}

/// Draw the dialog centred in a `w`×`h` window over a dim backdrop.
pub fn render(
    layer: &mut Layer,
    over: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    ed: &ToolbarEditor,
    size: [f32; 2],
    mouse: (f32, f32),
) -> Layout {
    let width = |engine: &mut TextEngine, item: ToolbarItem| {
        ICON_W + 8.0 + engine.measure(item.label(), 12.5, FONT_SANS, Weight::NORMAL)
    };
    let draw = |layer: &mut Layer,
                engine: &mut TextEngine,
                theme: &Theme,
                [x, y]: [f32; 2],
                item: ToolbarItem,
                _trail: f32| {
        draw_mark(
            layer,
            item,
            [x + ICON_W / 2.0, y + ITEM_H / 2.0],
            theme.text_dim,
        );
        layer.text(
            sans(engine, item.label(), 12.5, Weight::NORMAL),
            x + ICON_W + 8.0,
            y + (ITEM_H - 16.5) / 2.0,
            theme.text_ui,
        );
    };
    let look = Look {
        title: "Edit toolbar",
        palette_label: "Not in the toolbar",
        palette_empty: "Every button is in your toolbar.",
        buckets_label: "Your toolbar",
        bucket_names: &["Left", "Right"],
        bucket_empty: "Drop a button here",
        item_h: ITEM_H,
        framed: true,
        width: &width,
        draw: &draw,
    };
    arranger::render(layer, over, engine, theme, ed, &look, size, mouse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toolbar_is_two_buckets_and_saves_as_its_sides() {
        let config = Config {
            toolbar_left: vec![ToolbarItem::Settings],
            toolbar_right: vec![ToolbarItem::Review],
            ..Default::default()
        };
        let mut ed = new(&config);
        assert!(!ed.changed() && !ed.is_default());
        assert_eq!(
            ed.palette(),
            [
                ToolbarItem::Explorer,
                ToolbarItem::Notifications,
                ToolbarItem::Palette
            ]
        );
        // a palette click goes to the right end
        ed.add(ToolbarItem::Palette, RIGHT);
        ed.place(ToolbarItem::Review, LEFT, 0);
        assert_eq!(
            sides(&ed),
            (
                vec![ToolbarItem::Review, ToolbarItem::Settings],
                vec![ToolbarItem::Palette]
            )
        );
        ed.restore_default();
        assert_eq!(sides(&ed), Config::default().toolbar());
    }
}
