//! The rich-input composer (`crate::composer`): the command editor's look,
//! docked at the bottom of a pane over the running program, with a
//! "Compose for …" header in place of the prompt chips and a hint row.

use rift_core::{Config, PromptMode, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping};
use rift_renderer::{Layer, TextEngine};

use crate::state::PaneState;
use crate::views::editor;

/// The hint row under the input.
const HINT_H: f32 = 14.0;

/// The composer's height for its draft `buffer` soft-wrapped at `cols`.
pub fn height(buffer: &str, cols: usize) -> f32 {
    editor::editor_height(buffer, PromptMode::Rift, cols, 1) + HINT_H
}

/// Draw the composer in `area` ([x, y, w, h], the editor zone's inset).
/// Returns the input line's (x, y) and the cursor's top-left, as
/// `editor::render` does.
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    config: &Config,
    pane: &PaneState,
    area: [f32; 4],
    cell_w: f32,
    blink_on: bool,
    focused: bool,
    cols: usize,
) -> ((f32, f32), (f32, f32)) {
    let [x, y, w, h] = area;
    // no fill: the pane's own ground, as under the command editor (the
    // program's rows moved out from under it, `terminal::beside_panel`)
    let (anchor, cursor, _, _) = editor::render(
        layer,
        engine,
        theme,
        config,
        pane,
        None,
        [x, y, w, h - HINT_H],
        cell_w,
        blink_on,
        focused,
        false,
        "",
        None,
        None,
        cols,
        Some(("Compose for", pane.composer.label.as_str())),
        editor::GhostUi::default(),
    );
    let hint = crate::composer::hint(config.rich_input_submit);
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(11.0, HINT_H));
    b.set_text(
        &mut engine.font_system,
        hint,
        &Attrs::new().family(Family::Name(rift_renderer::FONT_SANS)),
        Shaping::Advanced,
        None,
    );
    // the row under the input, below its bottom padding's middle
    layer.text(b, x + editor::PAD_X, y + h - HINT_H - 6.0, theme.text_muted);
    (anchor, cursor)
}
