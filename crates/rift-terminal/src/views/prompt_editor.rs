//! "Edit prompt" (Warp): pick and order the Rift prompt's context chips, in
//! the arranger dialog (`arranger.rs`): the chips not in the prompt sit in a
//! palette, the prompt's chips in one dashed bucket.

use rift_core::{PromptChip, Theme};
use rift_renderer::{Layer, TextEngine};

use crate::views::arranger::{self, Arranger, Look};
use crate::views::editor::{CHIP_H, chip_width, draw_chip};
use crate::views::prompt_chips::sample;

pub type PromptEditor = Arranger<PromptChip>;
pub type Layout = arranger::Layout<PromptChip>;
pub use arranger::Target;

/// The dialog over the prompt's `chips`.
pub fn new(chips: &[PromptChip]) -> PromptEditor {
    Arranger::new(
        vec![chips.to_vec()],
        vec![PromptChip::defaults()],
        PromptChip::ALL.to_vec(),
    )
}

/// The prompt's chips as the draft holds them.
pub fn chips(ed: &PromptEditor) -> &[PromptChip] {
    ed.buckets.first().map_or(&[], |b| b.as_slice())
}

/// Draw the dialog centred in a `w`×`h` window over a dim backdrop.
pub fn render(
    layer: &mut Layer,
    over: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    ed: &PromptEditor,
    size: [f32; 2],
    mouse: (f32, f32),
) -> Layout {
    let width = |engine: &mut TextEngine, c: PromptChip| chip_width(engine, &sample(c, theme));
    let draw = |layer: &mut Layer,
                engine: &mut TextEngine,
                theme: &Theme,
                at: [f32; 2],
                c: PromptChip,
                trail: f32| {
        // the chip's border reaches around its × in the bucket
        draw_chip(
            layer,
            engine,
            theme,
            at,
            &sample(c, theme),
            (trail - 4.0).max(0.0),
        );
    };
    let look = Look {
        title: "Edit prompt",
        palette_label: "Not in your prompt",
        palette_empty: "Every chip is in your prompt.",
        buckets_label: "Your prompt",
        bucket_names: &[],
        bucket_empty: "Drag chips here, or click one above",
        item_h: CHIP_H,
        framed: false,
        width: &width,
        draw: &draw,
    };
    arranger::render(layer, over, engine, theme, ed, &look, size, mouse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_is_one_bucket_of_every_chip() {
        let mut ed = new(&PromptChip::defaults());
        assert!(!ed.changed() && ed.is_default());
        assert!(!ed.palette().contains(&PromptChip::Cwd));
        assert_eq!(ed.palette().len() + chips(&ed).len(), PromptChip::ALL.len());
        ed.add(PromptChip::Time24, 0);
        assert_eq!(chips(&ed).last(), Some(&PromptChip::Time24));
        assert!(ed.changed() && !ed.is_default());
        ed.restore_default();
        assert!(!ed.changed());
    }
}
