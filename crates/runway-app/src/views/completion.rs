use runway_core::completion::Completion;
use runway_core::{Color, Theme};
use runway_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use runway_renderer::{FONT_SANS, Layer, TextEngine, mono_family, text_color};

/// Live completion popup state for the active editor.
#[derive(Default)]
pub struct CompletionUi {
    pub items: Vec<Completion>,
    pub selected: usize,
    pub open: bool,
    /// Remainder previewed inline after the cursor.
    pub ghost: String,
}

impl CompletionUi {
    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.ghost.clear();
        self.selected = 0;
    }
}

const DROPDOWN_W: f32 = 560.0;
const ROW_H: f32 = 34.0;
const FOOTER_H: f32 = 30.0;
const MAX_ROWS: usize = 6;

fn sans(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    b.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(FONT_SANS)).weight(weight),
        Shaping::Advanced,
        None,
    );
    b
}

/// Draw the floating completion list ABOVE the editor zone (the editor sits
/// at the window bottom; design 04 shows it under the input line, flipped
/// here to stay on-screen). `anchor` = (x, y) of the editor input line.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    ui: &CompletionUi,
    anchor: (f32, f32),
    prefix_len: usize,
) {
    if !ui.open || ui.items.is_empty() {
        return;
    }
    let shown = ui.items.len().min(MAX_ROWS);
    let list_h = shown as f32 * ROW_H + FOOTER_H;
    let x = anchor.0;
    let y = anchor.1 - list_h - 10.0;

    layer.shadow(
        [x, y + 8.0],
        [DROPDOWN_W, list_h],
        Color::hex("#000000").unwrap().with_alpha(0.5),
        10.0,
        14.0,
    );
    layer.bordered(
        [x, y],
        [DROPDOWN_W, list_h],
        theme.surface,
        Color::rgba(1.0, 1.0, 1.0, 0.10),
        10.0,
        1.0,
    );

    let scroll = ui.selected.saturating_sub(MAX_ROWS - 1);
    for (row, item) in ui.items.iter().skip(scroll).take(shown).enumerate() {
        let idx = row + scroll;
        let ry = y + row as f32 * ROW_H;
        let selected = idx == ui.selected;
        if selected {
            layer.rect(
                [x, ry],
                [DROPDOWN_W, ROW_H],
                theme.accent.with_alpha(0.14),
                0.0,
            );
            layer.rect([x, ry], [2.0, ROW_H], theme.accent, 0.0);
        }

        // completion text: matched prefix in accent bold
        let name = &item.insert;
        let split = prefix_len.min(name.len());
        let (matched, rest) = name.split_at(split);
        let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(13.0, 17.0));
        let rich = vec![
            (
                matched,
                Attrs::new()
                    .family(Family::Name(mono_family()))
                    .weight(Weight::BOLD)
                    .color(text_color(theme.accent)),
            ),
            (
                rest,
                Attrs::new()
                    .family(Family::Name(mono_family()))
                    .color(text_color(if selected {
                        theme.text_bright
                    } else {
                        theme.text_ui
                    })),
            ),
        ];
        buf.set_rich_text(
            &mut engine.font_system,
            rich,
            &Attrs::new().family(Family::Name(mono_family())),
            Shaping::Advanced,
            None,
        );
        layer.text(buf, x + 14.0, ry + (ROW_H - 17.0) / 2.0, theme.text_ui);

        if !item.description.is_empty() {
            let dbuf = sans(engine, &item.description, 12.0, Weight::NORMAL);
            let dcolor = if selected {
                theme.text_dim
            } else {
                theme.text_muted
            };
            layer.text(dbuf, x + 14.0 + 144.0, ry + (ROW_H - 16.0) / 2.0, dcolor);
        }
    }

    // footer
    let fy = y + shown as f32 * ROW_H;
    layer.rect(
        [x, fy],
        [DROPDOWN_W, FOOTER_H],
        Color::hex("#000000").unwrap().with_alpha(0.18),
        0.0,
    );
    layer.hline(x, fy, DROPDOWN_W, 1.0, Color::rgba(1.0, 1.0, 1.0, 0.07));
    let hint = "⇥ accept   ↑↓ select";
    let hbuf = sans(engine, hint, 10.5, Weight::NORMAL);
    layer.text(hbuf, x + 14.0, fy + 8.0, theme.text_muted);
    let src = format!(
        "{} completions · {}",
        ui.items.len(),
        ui.items.first().map(|c| c.source).unwrap_or("shell")
    );
    let sw = engine.measure(&src, 10.5, FONT_SANS, Weight::NORMAL);
    let sbuf = sans(engine, &src, 10.5, Weight::NORMAL);
    layer.text(sbuf, x + DROPDOWN_W - sw - 14.0, fy + 8.0, theme.text_muted);
}
