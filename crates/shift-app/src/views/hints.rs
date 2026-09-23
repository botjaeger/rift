use shift_core::{Color, Theme};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use shift_renderer::{Layer, TextEngine, mono_family};

/// Hint mode (⇧⌘H): a label pill at each target's start, and an underline
/// under links and paths (block headers are already a visible target). Labels that no longer match what was typed are hidden; the
/// typed prefix of the rest is dimmed.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    items: &[(&str, [f32; 4], bool)],
    typed: &str,
) {
    for (label, rect, underline) in items {
        if !label.starts_with(typed) {
            continue;
        }
        if *underline {
            layer.rect(
                [rect[0], rect[1] + rect[3] - 2.0],
                [rect[2], 1.5],
                theme.warning.with_alpha(0.8),
                0.0,
            );
        }
        let w = label.chars().count() as f32 * 8.0 + 8.0;
        let x = rect[0] - 2.0;
        let y = rect[1] + (rect[3] - 17.0) / 2.0;
        layer.shadow(
            [x, y + 2.0],
            [w, 17.0],
            Color::BLACK.with_alpha(0.4),
            4.0,
            6.0,
        );
        layer.rect([x, y], [w, 17.0], theme.warning, 4.0);
        let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(12.0, 15.0));
        let rest = &label[typed.len()..];
        let dark = Color::hex("#101010").unwrap();
        let spans: Vec<(&str, Attrs)> = [(typed, dark.with_alpha(0.45)), (rest, dark)]
            .into_iter()
            .filter(|(s, _)| !s.is_empty())
            .map(|(s, c)| {
                (
                    s,
                    Attrs::new()
                        .family(Family::Name(mono_family()))
                        .weight(Weight::BOLD)
                        .color(shift_renderer::text_color(c)),
                )
            })
            .collect();
        buf.set_rich_text(
            &mut engine.font_system,
            spans,
            &Attrs::new().family(Family::Name(mono_family())),
            Shaping::Advanced,
            None,
        );
        layer.text(buf, x + 4.0, y + 1.0, dark);
    }
}
