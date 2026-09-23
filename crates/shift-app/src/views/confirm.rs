use shift_core::{Color, Theme};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

fn text(
    engine: &mut TextEngine,
    value: &str,
    size: f32,
    family: &'static str,
    weight: Weight,
) -> Buffer {
    let mut buffer = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    buffer.set_text(
        &mut engine.font_system,
        value,
        &Attrs::new().family(Family::Name(family)).weight(weight),
        Shaping::Advanced,
        None,
    );
    buffer
}

/// Bottom-center status toast (saved files, copied text, …).
pub fn toast(layer: &mut Layer, engine: &mut TextEngine, theme: &Theme, msg: &str, w: f32, h: f32) {
    let tw = engine.measure(msg, 12.5, FONT_SANS, Weight::NORMAL);
    let bw = (tw + 32.0).min(w - 32.0);
    let x = (w - bw) / 2.0;
    let y = h - 64.0;
    layer.shadow(
        [x, y + 6.0],
        [bw, 34.0],
        Color::BLACK.with_alpha(0.45),
        10.0,
        12.0,
    );
    layer.bordered(
        [x, y],
        [bw, 34.0],
        theme.surface,
        theme.accent.with_alpha(0.45),
        10.0,
        1.0,
    );
    layer.text(
        text(engine, msg, 12.5, FONT_SANS, Weight::NORMAL),
        x + 16.0,
        y + 8.5,
        theme.text_ui,
    );
}

/// Contents of the confirm modal.
pub struct Dialog<'a> {
    pub title: &'a str,
    pub description: String,
    pub note: &'a str,
    /// Up to three are listed (running commands, pasted lines).
    pub lines: &'a [String],
    /// Shown for a blank entry in `lines`.
    pub empty_line: &'a str,
    pub confirm_label: &'a str,
    /// Red confirm button (close/quit) instead of the accent one.
    pub destructive: bool,
}

/// Confirmation modal (close with running processes, multiline paste).
/// Returns [cancel, confirm] hit rects.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    dialog: &Dialog,
    w: f32,
    h: f32,
) -> [[f32; 4]; 2] {
    let (title, commands, destructive_label) = (dialog.title, dialog.lines, dialog.confirm_label);
    layer.rect(
        [0.0, 0.0],
        [w, h],
        Color::hex("#0d0d0f").unwrap().with_alpha(0.72),
        0.0,
    );

    let shown = commands.len().min(3);
    let modal_w = 520.0;
    let modal_h = 190.0 + shown as f32 * 25.0;
    let x = (w - modal_w) / 2.0;
    let y = (h - modal_h) / 2.0;

    layer.shadow(
        [x, y + 10.0],
        [modal_w, modal_h],
        Color::BLACK.with_alpha(0.62),
        14.0,
        20.0,
    );
    layer.bordered(
        [x, y],
        [modal_w, modal_h],
        theme.surface,
        Color::WHITE.with_alpha(0.12),
        14.0,
        1.0,
    );

    layer.circle([x + 34.0, y + 34.0], 22.0, theme.warning.with_alpha(0.18));
    layer.text(
        text(engine, "!", 16.0, FONT_SANS, Weight::BOLD),
        x + 31.2,
        y + 21.5,
        theme.warning,
    );
    layer.text(
        text(engine, title, 19.0, FONT_SANS, Weight::SEMIBOLD),
        x + 56.0,
        y + 20.0,
        theme.text_bright,
    );

    layer.text(
        text(engine, &dialog.description, 13.0, FONT_SANS, Weight::NORMAL),
        x + 24.0,
        y + 64.0,
        theme.text_ui,
    );
    layer.text(
        text(engine, dialog.note, 12.0, FONT_SANS, Weight::NORMAL),
        x + 24.0,
        y + 87.0,
        theme.text_dim,
    );

    let mut command_y = y + 116.0;
    for command in commands.iter().take(shown) {
        layer.circle([x + 30.0, command_y + 7.0], 5.0, theme.warning);
        let label = if command.trim().is_empty() {
            dialog.empty_line
        } else {
            command.trim()
        };
        let clipped: String = label.chars().take(56).collect();
        layer.text(
            text(engine, &clipped, 12.0, mono_family(), Weight::NORMAL),
            x + 42.0,
            command_y,
            theme.text_ui,
        );
        command_y += 25.0;
    }
    if commands.len() > shown {
        let more = format!("and {} more…", commands.len() - shown);
        layer.text(
            text(engine, &more, 11.5, FONT_SANS, Weight::NORMAL),
            x + 42.0,
            command_y,
            theme.text_muted,
        );
    }

    let button_y = y + modal_h - 52.0;
    let destructive_w = 126.0;
    let cancel_w = 88.0;
    let destructive = [
        x + modal_w - destructive_w - 20.0,
        button_y,
        destructive_w,
        32.0,
    ];
    let cancel = [destructive[0] - cancel_w - 10.0, button_y, cancel_w, 32.0];
    layer.bordered(
        [cancel[0], cancel[1]],
        [cancel[2], cancel[3]],
        Color::WHITE.with_alpha(0.06),
        Color::WHITE.with_alpha(0.10),
        7.0,
        1.0,
    );
    layer.text(
        text(engine, "Cancel", 12.5, FONT_SANS, Weight::MEDIUM),
        cancel[0] + 22.0,
        cancel[1] + 7.0,
        theme.text_ui,
    );
    layer.rect(
        [destructive[0], destructive[1]],
        [destructive[2], destructive[3]],
        if dialog.destructive {
            theme.error
        } else {
            theme.accent
        },
        7.0,
    );
    let label_w = engine.measure(destructive_label, 12.5, FONT_SANS, Weight::SEMIBOLD);
    layer.text(
        text(engine, destructive_label, 12.5, FONT_SANS, Weight::SEMIBOLD),
        destructive[0] + (destructive[2] - label_w) / 2.0,
        destructive[1] + 7.0,
        Color::WHITE,
    );

    [cancel, destructive]
}
