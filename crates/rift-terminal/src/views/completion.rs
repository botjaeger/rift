use rift_core::completion::Completion;
use rift_core::{Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family, text_color};

use crate::state::{ClickTarget, Hit};
use crate::views::icons::{self, Icon};

/// Live completion panel state for the active editor.
#[derive(Default)]
pub struct CompletionUi {
    /// The rows shown: `all`, narrowed to `filter`'s source.
    pub items: Vec<Completion>,
    /// Every candidate of the last query.
    pub all: Vec<Completion>,
    /// Source tab picked with ⇧⇥ or a click (`None` = all sources).
    pub filter: Option<&'static str>,
    pub selected: usize,
    pub open: bool,
    /// Remainder previewed inline after the cursor.
    pub ghost: String,
}

impl CompletionUi {
    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.all.clear();
        self.filter = None;
        self.ghost.clear();
        self.selected = 0;
    }

    /// New candidates; the picked source tab stays while it still has rows.
    pub fn set(&mut self, items: Vec<Completion>) {
        self.all = items;
        self.refilter();
    }

    fn refilter(&mut self) {
        if self
            .filter
            .is_some_and(|s| !self.all.iter().any(|c| c.source == s))
        {
            self.filter = None;
        }
        self.items = match self.filter {
            Some(s) => self.all.iter().filter(|c| c.source == s).cloned().collect(),
            None => self.all.clone(),
        };
        self.selected = 0;
        self.open = !self.items.is_empty();
    }

    /// The source tabs: every source among the candidates with its count,
    /// in order of first appearance.
    pub fn sources(&self) -> Vec<(&'static str, usize)> {
        let mut out: Vec<(&'static str, usize)> = Vec::new();
        for c in &self.all {
            match out.iter_mut().find(|(s, _)| *s == c.source) {
                Some(entry) => entry.1 += 1,
                None => out.push((c.source, 1)),
            }
        }
        out
    }

    /// ⇧⇥: the next source tab (All, then each source, wrapping).
    pub fn cycle_source(&mut self) {
        let sources = self.sources();
        if sources.len() < 2 {
            return;
        }
        self.filter = match self.filter {
            None => Some(sources[0].0),
            Some(s) => sources
                .iter()
                .position(|(x, _)| *x == s)
                .and_then(|i| sources.get(i + 1))
                .map(|(x, _)| *x),
        };
        self.refilter();
    }

    /// A click on source tab `i` (0 = All).
    pub fn pick_source(&mut self, i: usize) {
        self.filter = i
            .checked_sub(1)
            .and_then(|k| self.sources().get(k).map(|(s, _)| *s));
        self.refilter();
    }
}

const HEADER_H: f32 = 32.0;
const ROW_H: f32 = 28.0;
const FOOTER_H: f32 = 30.0;
const MAX_ROWS: usize = 8;
const PAD_X: f32 = 16.0;

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

fn lift(theme: &Theme, alpha: f32) -> Color {
    if theme.is_light() {
        Color::BLACK.with_alpha(alpha)
    } else {
        Color::WHITE.with_alpha(alpha)
    }
}

/// A key cap: the glyph on a hairline-bordered chip; returns its width.
pub fn key_cap(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    key: &str,
    x: f32,
    cy: f32,
) -> f32 {
    let kw = engine.measure(key, 10.5, FONT_SANS, Weight::MEDIUM);
    let w = (kw + 10.0).max(18.0);
    layer.bordered(
        [x, cy - 9.0],
        [w, 18.0],
        lift(theme, 0.05),
        lift(theme, 0.14),
        3.0,
        1.0,
    );
    layer.text(
        sans(engine, key, 10.5, Weight::MEDIUM),
        (x + (w - kw) / 2.0).round(),
        cy - 7.5,
        theme.text_dim,
    );
    w
}

/// Footer of a docked panel: key caps and what they do, from `x`.
pub fn key_hints(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    hints: &[(&[&str], &str)],
    x: f32,
    cy: f32,
    right: f32,
) {
    let mut kx = x;
    for (keys, label) in hints {
        let lw = engine.measure(label, 11.5, FONT_SANS, Weight::NORMAL);
        let keys_w: f32 = keys
            .iter()
            .map(|k| (engine.measure(k, 10.5, FONT_SANS, Weight::MEDIUM) + 10.0).max(18.0) + 4.0)
            .sum();
        if kx + keys_w + 2.0 + lw > right {
            break;
        }
        for k in keys.iter() {
            kx += key_cap(layer, engine, theme, k, kx, cy) + 4.0;
        }
        kx += 2.0;
        layer.text(
            sans(engine, label, 11.5, Weight::NORMAL),
            kx,
            cy - 7.8,
            theme.text_muted,
        );
        kx += lw + 16.0;
    }
}

/// The row icon of a candidate's source.
fn kind_icon(item: &Completion) -> Icon {
    match item.source {
        "path" if item.insert.ends_with('/') => Icon::Folder,
        "path" => Icon::File,
        "history" => Icon::History,
        "git" => Icon::Branch,
        _ => Icon::Terminal,
    }
}

/// Draw the completion list as a panel docked to the editor, full pane
/// width (Warp's completion pane): source tabs with counts, the rows, and a
/// key-hint footer. It sits on the editor zone's top edge (input at the
/// bottom) or under it (input at the top); the output moves out from under
/// it (`terminal::beside_panel`). `zone` =
/// the editor rect, `pane` = the active pane's body; `prefix_len` is the
/// typed part of a token candidate, `line_len` of a history line.
/// Height of the open panel: the terminal output moves out from under it.
pub fn panel_height(ui: &CompletionUi) -> f32 {
    HEADER_H + ui.items.len().min(MAX_ROWS) as f32 * ROW_H + FOOTER_H
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    ui: &CompletionUi,
    zone: [f32; 4],
    pane: [f32; 4],
    prefix_len: usize,
    line_len: usize,
    mouse: Option<(f32, f32)>,
    hits: &mut Vec<Hit>,
) {
    if !ui.open || ui.items.is_empty() {
        return;
    }
    let shown = ui.items.len().min(MAX_ROWS);
    let panel_h = panel_height(ui);
    let (x, w) = (pane[0], pane[2]);
    // above the editor when it sits at the pane bottom, else below it
    let at_bottom = zone[1] + zone[3] >= pane[1] + pane[3] - 2.0;
    let y = if at_bottom {
        (zone[1] - panel_h).max(pane[1])
    } else {
        zone[1] + zone[3]
    };

    layer.rect([x, y], [w, panel_h], theme.bg, 0.0);
    layer.hline(x, y, w, 1.0, lift(theme, 0.1));
    if !at_bottom {
        layer.hline(x, y + panel_h - 1.0, w, 1.0, lift(theme, 0.1));
    }
    layer.set_clip(Some(([x, y, w, panel_h], 0.0)));

    // header: source tabs (All, then each source) with their counts
    // (one source: just its name, nothing to switch)
    let sources = ui.sources();
    let mut tabs: Vec<(String, usize, bool)> = if sources.len() > 1 {
        vec![("ALL".into(), ui.all.len(), ui.filter.is_none())]
    } else {
        Vec::new()
    };
    for (s, n) in sources {
        tabs.push((s.to_uppercase(), n, ui.filter == Some(s) || tabs.is_empty()));
    }
    let mut tx = x + 8.0;
    for (i, (label, n, on)) in tabs.iter().enumerate() {
        let count = n.to_string();
        let lw =
            engine.measure(label, 11.0, FONT_SANS, Weight::SEMIBOLD) + 0.66 * label.len() as f32;
        let cw = engine.measure(&count, 11.0, FONT_SANS, Weight::MEDIUM);
        let tab_w = 10.0 + lw + 5.0 + cw + 10.0;
        let r = [tx, y, tab_w, HEADER_H];
        if *on {
            layer.rect(
                [tx + 8.0, y + HEADER_H - 2.0],
                [tab_w - 16.0, 2.0],
                theme.accent,
                0.0,
            );
        }
        let mut lb = Buffer::new(&mut engine.font_system, Metrics::new(11.0, 14.0));
        lb.set_text(
            &mut engine.font_system,
            label,
            &Attrs::new()
                .family(Family::Name(FONT_SANS))
                .weight(Weight::SEMIBOLD)
                .letter_spacing(0.06),
            Shaping::Advanced,
            None,
        );
        layer.text(
            lb,
            tx + 10.0,
            y + (HEADER_H - 14.0) / 2.0,
            if *on {
                theme.text_bright
            } else {
                theme.text_dim
            },
        );
        layer.text(
            sans(engine, &count, 11.0, Weight::MEDIUM),
            tx + 10.0 + lw + 5.0,
            y + (HEADER_H - 14.85) / 2.0,
            theme.text_muted,
        );
        if tabs.len() > 1 {
            hits.push(Hit {
                rect: r,
                target: ClickTarget::CompletionSource(i as u8),
            });
        }
        tx += tab_w;
    }
    if tabs.len() > 2 {
        let hint = "switch source";
        let hw = engine.measure(hint, 11.5, FONT_SANS, Weight::NORMAL);
        let caps_x = x + w - PAD_X - hw - 4.0 - 2.0 * 22.0;
        if caps_x > tx + 12.0 {
            key_hints(
                layer,
                engine,
                theme,
                &[(&["⇧", "⇥"], hint)],
                caps_x,
                y + HEADER_H / 2.0,
                x + w,
            );
        }
    }
    layer.hline(x, y + HEADER_H, w, 1.0, theme.border);

    // rows
    let scroll = ui.selected.saturating_sub(MAX_ROWS - 1);
    let cell = engine
        .measure("0", 12.5, mono_family(), Weight::NORMAL)
        .max(1.0);
    let rows_top = y + HEADER_H + 1.0;
    for (row, item) in ui.items.iter().skip(scroll).take(shown).enumerate() {
        let idx = row + scroll;
        let ry = rows_top + row as f32 * ROW_H;
        let rect = [x, ry, w, ROW_H];
        let selected = idx == ui.selected;
        let hot = mouse.is_some_and(|(mx, my)| {
            mx >= rect[0] && mx < rect[0] + rect[2] && my >= rect[1] && my < rect[1] + rect[3]
        });
        if selected {
            layer.rect([x, ry], [w, ROW_H], lift(theme, 0.07), 0.0);
        } else if hot {
            layer.rect([x, ry], [w, ROW_H], lift(theme, 0.04), 0.0);
        }
        hits.push(Hit {
            rect,
            target: ClickTarget::CompletionRow(idx),
        });
        icons::draw(
            layer,
            kind_icon(item),
            [x + PAD_X + 7.0, ry + ROW_H / 2.0],
            14.0,
            theme.text_dim,
        );

        // description on the right (muted), the name left of it
        let desc_w = if item.description.is_empty() {
            0.0
        } else {
            engine.measure(&item.description, 12.0, FONT_SANS, Weight::NORMAL)
        };
        let name_x = x + PAD_X + 14.0 + 12.0;
        let desc_x = x + w - PAD_X - desc_w;
        let name_room = (desc_x - 16.0 - name_x).max(cell * 4.0);
        let fits = (name_room / cell).floor().max(1.0) as usize;
        let name = &item.insert;
        let chars = name.chars().count();
        let typed = if item.source == "history" {
            line_len
        } else {
            prefix_len
        };
        // too long (a deep path): keep the tail, which tells candidates apart
        let (matched, rest): (String, String) = if chars > fits {
            let tail: String = name.chars().skip(chars - fits.saturating_sub(1)).collect();
            (String::new(), format!("…{tail}"))
        } else {
            let split = typed.min(name.len());
            let split = (0..=split)
                .rev()
                .find(|&i| name.is_char_boundary(i))
                .unwrap_or(0);
            (name[..split].to_string(), name[split..].to_string())
        };
        let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(12.5, 16.0));
        let rich = vec![
            (
                matched.as_str(),
                Attrs::new()
                    .family(Family::Name(mono_family()))
                    .weight(Weight::BOLD)
                    .color(text_color(theme.accent)),
            ),
            (
                rest.as_str(),
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
        layer.text_clipped(
            buf,
            name_x,
            ry + (ROW_H - 16.0) / 2.0,
            theme.text_ui,
            TextBounds {
                left: name_x as i32,
                top: ry as i32,
                right: (name_x + name_room) as i32,
                bottom: (ry + ROW_H) as i32,
            },
        );
        if desc_w > 0.0 && desc_x > name_x + 40.0 {
            let dbuf = sans(engine, &item.description, 12.0, Weight::NORMAL);
            layer.text(
                dbuf,
                desc_x,
                ry + (ROW_H - 16.2) / 2.0,
                if selected {
                    theme.text_dim
                } else {
                    theme.text_muted
                },
            );
        }
    }

    // footer: key caps and what they do
    let fy = y + panel_h - FOOTER_H;
    layer.hline(x, fy, w, 1.0, theme.border);
    let mut hints: Vec<(&[&str], &str)> = vec![(&["↑", "↓"], "navigate"), (&["⇥"], "accept")];
    if !ui.ghost.is_empty() {
        hints.push((&["→"], "accept ghost"));
    }
    hints.push((&["esc"], "dismiss"));
    key_hints(
        layer,
        engine,
        theme,
        &hints,
        x + PAD_X,
        fy + FOOTER_H / 2.0,
        x + w - PAD_X,
    );
    layer.set_clip(None);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(insert: &str, source: &'static str) -> Completion {
        Completion {
            insert: insert.into(),
            description: String::new(),
            source,
        }
    }

    #[test]
    fn source_tabs_filter_and_cycle() {
        let mut ui = CompletionUi::default();
        ui.set(vec![
            c("a/", "path"),
            c("b/", "path"),
            c("cd a/x", "history"),
        ]);
        assert_eq!(ui.sources(), vec![("path", 2), ("history", 1)]);
        assert_eq!(ui.items.len(), 3);
        ui.cycle_source();
        assert_eq!(ui.filter, Some("path"));
        assert_eq!(ui.items.len(), 2);
        ui.cycle_source();
        assert_eq!(ui.items.len(), 1);
        ui.cycle_source();
        assert_eq!(ui.filter, None);
        ui.pick_source(2);
        assert_eq!(ui.filter, Some("history"));
        // the picked source survives new candidates while it still has rows
        ui.set(vec![c("a/", "path"), c("cd a/y", "history")]);
        assert_eq!(ui.filter, Some("history"));
        ui.set(vec![c("a/", "path")]);
        assert_eq!(ui.filter, None);
        assert!(ui.open);
    }
}
