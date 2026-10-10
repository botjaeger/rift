use shift_core::{Color, Theme};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::views::icons::{self, Icon};

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

/// Longest prefix of `value` (plus `…`) that fits `max_w`.
fn elide(
    engine: &mut TextEngine,
    value: &str,
    size: f32,
    family: &'static str,
    max_w: f32,
) -> String {
    if engine.measure(value, size, family, Weight::NORMAL) <= max_w {
        return value.to_string();
    }
    let mut out = value.to_string();
    while out.pop().is_some() {
        let candidate = format!("{}…", out.trim_end());
        if engine.measure(&candidate, size, family, Weight::NORMAL) <= max_w {
            return candidate;
        }
    }
    String::new()
}

/// Greedy word wrap of `value` into lines no wider than `max_w`.
fn wrap(engine: &mut TextEngine, value: &str, size: f32, max_w: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in value.split_whitespace() {
        let next = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if cur.is_empty() || engine.measure(&next, size, FONT_SANS, Weight::NORMAL) <= max_w {
            cur = next;
        } else {
            lines.push(std::mem::replace(&mut cur, word.to_string()));
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
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
        8.0,
        12.0,
    );
    layer.bordered(
        [x, y],
        [bw, 34.0],
        theme.surface,
        lift(theme, 0.12),
        8.0,
        1.0,
    );
    let msg = elide(engine, msg, 12.5, FONT_SANS, bw - 32.0);
    layer.text(
        text(engine, &msg, 12.5, FONT_SANS, Weight::NORMAL),
        x + 16.0,
        y + 8.5,
        theme.text_ui,
    );
}

fn lift(theme: &Theme, alpha: f32) -> Color {
    if theme.is_light() {
        Color::BLACK.with_alpha(alpha)
    } else {
        Color::WHITE.with_alpha(alpha)
    }
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

/// Confirmation modal (close with running processes, multiline paste): a
/// warning disc, title and description, the affected lines in a code box,
/// then Cancel (esc) and the confirm button (⏎). Returns [cancel, confirm]
/// hit rects.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    dialog: &Dialog,
    w: f32,
    h: f32,
) -> [[f32; 4]; 2] {
    let commands = dialog.lines;
    layer.rect([0.0, 0.0], [w, h], Color::BLACK.with_alpha(0.4), 0.0);

    let shown = commands.len().min(3);
    let modal_w = 440.0_f32.min((w - 32.0).max(120.0));
    let pad = 20.0;
    let text_x = pad + 32.0 + 12.0;
    let text_w = modal_w - text_x - pad;
    let desc_lines = wrap(engine, &dialog.description, 13.0, text_w);
    let note_lines = wrap(engine, dialog.note, 12.0, text_w);
    let more = commands.len() > shown;
    let box_h = if shown > 0 {
        16.0 + shown as f32 * 19.0 + if more { 19.0 } else { 0.0 }
    } else {
        0.0
    };
    let body_h = 22.0
        + desc_lines.len() as f32 * 18.0
        + 4.0
        + note_lines.len() as f32 * 16.0
        + if box_h > 0.0 { 12.0 + box_h } else { 0.0 };
    let modal_h = pad + body_h + 20.0 + 28.0 + pad;
    let x = (w - modal_w) / 2.0;
    let y = ((h - modal_h) / 2.0).max(8.0);

    layer.shadow(
        [x, y + 12.0],
        [modal_w, modal_h],
        Color::BLACK.with_alpha(0.55),
        8.0,
        24.0,
    );
    layer.bordered(
        [x, y],
        [modal_w, modal_h],
        theme.surface,
        lift(theme, 0.1),
        8.0,
        1.0,
    );

    // warning disc, title
    let (dx, dy) = (x + pad + 16.0, y + pad + 16.0);
    layer.circle([dx, dy], 32.0, theme.warning.with_alpha(0.14));
    icons::draw(layer, Icon::Warning, [dx, dy], 16.0, theme.warning);
    let title = elide(engine, dialog.title, 15.0, FONT_SANS, text_w);
    layer.text(
        text(engine, &title, 15.0, FONT_SANS, Weight::SEMIBOLD),
        x + text_x,
        y + pad,
        theme.text_bright,
    );
    let mut ty = y + pad + 24.0;
    for line in &desc_lines {
        layer.text(
            text(engine, line, 13.0, FONT_SANS, Weight::NORMAL),
            x + text_x,
            ty,
            theme.text_dim,
        );
        ty += 18.0;
    }
    ty += 4.0;
    for line in &note_lines {
        layer.text(
            text(engine, line, 12.0, FONT_SANS, Weight::NORMAL),
            x + text_x,
            ty,
            theme.text_muted,
        );
        ty += 16.0;
    }

    // the affected lines, in a code box
    if shown > 0 {
        ty += 12.0;
        layer.bordered(
            [x + text_x, ty],
            [text_w, box_h],
            theme.ground(theme.bg),
            lift(theme, 0.07),
            4.0,
            1.0,
        );
        let mut ly = ty + 8.0;
        for command in commands.iter().take(shown) {
            let label = if command.trim().is_empty() {
                dialog.empty_line
            } else {
                command.trim()
            };
            let clipped = elide(engine, label, 12.0, mono_family(), text_w - 20.0);
            layer.text(
                text(engine, &clipped, 12.0, mono_family(), Weight::NORMAL),
                x + text_x + 10.0,
                ly,
                theme.text_ui,
            );
            ly += 19.0;
        }
        if more {
            let more = format!("and {} more…", commands.len() - shown);
            layer.text(
                text(engine, &more, 11.5, FONT_SANS, Weight::NORMAL),
                x + text_x + 10.0,
                ly + 1.0,
                theme.text_muted,
            );
        }
    }

    // buttons: Cancel [esc], confirm [⏎]
    let by = y + modal_h - pad - 28.0;
    let cap_w = |engine: &mut TextEngine, k: &str| {
        (engine.measure(k, 10.5, FONT_SANS, Weight::MEDIUM) + 10.0).max(18.0)
    };
    let label_w = engine.measure(dialog.confirm_label, 12.5, FONT_SANS, Weight::MEDIUM);
    let confirm_w = 12.0 + label_w + 8.0 + cap_w(engine, "⏎") + 6.0;
    let cancel_lw = engine.measure("Cancel", 12.5, FONT_SANS, Weight::MEDIUM);
    let cancel_w = 12.0 + cancel_lw + 8.0 + cap_w(engine, "esc") + 6.0;
    let confirm = [x + modal_w - pad - confirm_w, by, confirm_w, 28.0];
    let cancel = [confirm[0] - 8.0 - cancel_w, by, cancel_w, 28.0];
    layer.bordered(
        [cancel[0], cancel[1]],
        [cancel[2], cancel[3]],
        lift(theme, 0.06),
        lift(theme, 0.12),
        4.0,
        1.0,
    );
    layer.text(
        text(engine, "Cancel", 12.5, FONT_SANS, Weight::MEDIUM),
        cancel[0] + 12.0,
        by + 6.0,
        theme.text_ui,
    );
    super::completion::key_cap(
        layer,
        engine,
        theme,
        "esc",
        cancel[0] + 12.0 + cancel_lw + 8.0,
        by + 14.0,
    );
    // red for closing running work (darkened so white text reads), else accent
    let (fill, ink) = if dialog.destructive {
        (theme.error.mix(Color::BLACK, 0.18), Color::WHITE)
    } else {
        (theme.accent, theme.bg)
    };
    layer.rect(
        [confirm[0], confirm[1]],
        [confirm[2], confirm[3]],
        fill,
        4.0,
    );
    layer.text(
        text(
            engine,
            dialog.confirm_label,
            12.5,
            FONT_SANS,
            Weight::MEDIUM,
        ),
        confirm[0] + 12.0,
        by + 6.0,
        ink,
    );
    let kx = confirm[0] + 12.0 + label_w + 8.0;
    let kw = cap_w(engine, "⏎");
    layer.bordered(
        [kx, by + 5.0],
        [kw, 18.0],
        Color::TRANSPARENT,
        ink.with_alpha(0.45),
        3.0,
        1.0,
    );
    let gw = engine.measure("⏎", 10.5, FONT_SANS, Weight::MEDIUM);
    layer.text(
        text(engine, "⏎", 10.5, FONT_SANS, Weight::MEDIUM),
        (kx + (kw - gw) / 2.0).round(),
        by + 6.5,
        ink,
    );

    [cancel, confirm]
}
