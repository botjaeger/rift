use shift_core::{Color, Theme};
use shift_renderer::glyphon::Weight;
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::state::{ClickTarget, Hit};

pub const TAB_BAR_H: f32 = 42.0;
/// Where the first tab starts (room for the traffic lights).
const TABS_X: f32 = 78.0;
/// Narrowest a tab gets before titles are elided to nothing.
const MIN_TAB_W: f32 = 96.0;

pub struct TabInfo {
    pub title: String,
    /// Pane count; > 1 shows the design-02 "N panes" pill after the title.
    pub panes: usize,
    pub shell_label: String,
    pub activity: bool,
    /// Tab color (Warp tab colors): tints the underline and a leading dot.
    pub color: Option<Color>,
    /// Synchronized inputs are on for this tab.
    pub sync: bool,
    /// Inline rename in progress: the edited text.
    pub editing: Option<String>,
    /// The rename text is fully selected (first keystroke replaces it).
    pub editing_selected: bool,
}

fn make_text(
    engine: &mut TextEngine,
    text: &str,
    size: f32,
    family: &'static str,
    weight: Weight,
) -> shift_renderer::glyphon::Buffer {
    use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping};
    let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.3));
    buf.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(family)).weight(weight),
        Shaping::Advanced,
        None,
    );
    buf
}

/// Longest prefix of `text` (plus `…`) that fits `max_w`.
fn elide(engine: &mut TextEngine, text: &str, size: f32, max_w: f32) -> String {
    if engine.measure(text, size, FONT_SANS, Weight::NORMAL) <= max_w {
        return text.to_string();
    }
    let mut out: String = text.to_string();
    while !out.is_empty() {
        out.pop();
        let candidate = format!("{}…", out.trim_end());
        if engine.measure(&candidate, size, FONT_SANS, Weight::NORMAL) <= max_w {
            return candidate;
        }
    }
    String::new()
}

/// Draw the 42px tab strip (design 01) and report click targets.
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    tabs: &[TabInfo],
    hint: &str,
    hint_pill: bool,
    active: usize,
    w: f32,
    blink_on: bool,
    hits: &mut Vec<Hit>,
) {
    layer.rect([0.0, 0.0], [w, TAB_BAR_H], theme.bg_chrome, 0.0);
    layer.hline(0.0, TAB_BAR_H - 1.0, w, 1.0, theme.border);

    // traffic lights: native on macOS, drawn (and clickable) on Linux
    #[cfg(not(target_os = "macos"))]
    {
        for (i, c) in ["#ff5f57", "#febc2e", "#28c840"].iter().enumerate() {
            let cx = 12.0 + 6.0 + i as f32 * 20.0;
            layer.circle([cx, TAB_BAR_H / 2.0], 12.0, Color::hex(c).unwrap());
            hits.push(Hit {
                rect: [cx - 8.0, TAB_BAR_H / 2.0 - 8.0, 16.0, 16.0],
                target: ClickTarget::TrafficLight(i as u8),
            });
        }
    }

    // right: palette / split hint (dropped when the strip gets crowded)
    let hint_w = engine.measure(hint, 11.5, mono_family(), Weight::NORMAL)
        + if hint_pill { 14.0 } else { 0.0 };

    // natural widths, then shrink evenly when the tabs overflow the strip
    let natural: Vec<f32> = tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            let shell_w = engine.measure(&tab.shell_label, 12.0, mono_family(), Weight::NORMAL);
            let title = tab.editing.as_deref().unwrap_or(&tab.title);
            let title_w = engine
                .measure(title, 12.5, FONT_SANS, Weight::NORMAL)
                .max(if tab.editing.is_some() { 90.0 } else { 0.0 });
            let dot_w = if (tab.activity && i != active) || tab.color.is_some() {
                15.0
            } else {
                0.0
            };
            let sync_w = if tab.sync { 20.0 } else { 0.0 };
            let panes_w = panes_label(tab)
                .map(|l| engine.measure(&l, 11.0, FONT_SANS, Weight::NORMAL) + 12.0 + 6.0)
                .unwrap_or(0.0);
            (16.0 + dot_w + shell_w + 8.0 + title_w + panes_w + sync_w + 10.0 + 8.0 + 16.0)
                .min(300.0)
        })
        .collect();
    let avail = (w - TABS_X - 40.0 - hint_w - 28.0).max(MIN_TAB_W);
    let total: f32 = natural.iter().sum();
    let (widths, show_hint): (Vec<f32>, bool) = if total <= avail {
        (natural, true)
    } else {
        // without the hint first, then squeeze
        let avail = (w - TABS_X - 40.0).max(MIN_TAB_W);
        let each = (avail / tabs.len().max(1) as f32).max(MIN_TAB_W);
        (natural.iter().map(|n| n.min(each)).collect(), false)
    };

    let mut x = TABS_X;
    for (i, tab) in tabs.iter().enumerate() {
        let is_active = i == active;
        let tab_w = widths[i];

        if is_active {
            // gradient hint + 2px underline (the tab color wins)
            layer.rect(
                [x, 0.0],
                [tab_w, TAB_BAR_H],
                tab.color.unwrap_or(theme.accent).with_alpha(0.07),
                0.0,
            );
        }
        match (tab.color, is_active) {
            (Some(c), _) => layer.rect([x, TAB_BAR_H - 2.0], [tab_w, 2.0], c, 0.0),
            (None, true) => layer.rect([x, TAB_BAR_H - 2.0], [tab_w, 2.0], theme.accent, 0.0),
            _ => {}
        }

        let mut cx = x + 16.0;
        let text_color = if is_active {
            theme.text_ui
        } else {
            theme.text_dim
        };

        if tab.activity && !is_active {
            layer.circle([cx + 3.5, TAB_BAR_H / 2.0], 7.0, theme.success);
            cx += 15.0;
        } else if let Some(c) = tab.color {
            layer.circle([cx + 3.5, TAB_BAR_H / 2.0], 8.0, c);
            cx += 15.0;
        }

        let shell_w = engine.measure(&tab.shell_label, 12.0, mono_family(), Weight::NORMAL);
        let close_x = x + tab_w - 16.0 - 8.0;
        let room = (close_x - 10.0 - (cx + shell_w + 8.0)).max(0.0);
        if room > 0.0 || tab.editing.is_some() {
            let shell_buf = make_text(
                engine,
                &tab.shell_label,
                12.0,
                mono_family(),
                Weight::NORMAL,
            );
            layer.text(shell_buf, cx, (TAB_BAR_H - 16.0) / 2.0, theme.accent);
            cx += shell_w + 8.0;
        }

        if let Some(edit) = &tab.editing {
            // inline rename field
            let fw = (close_x - 6.0 - cx).max(60.0);
            layer.bordered(
                [cx - 5.0, 9.0],
                [fw, TAB_BAR_H - 18.0],
                theme.bg,
                theme.accent,
                5.0,
                1.0,
            );
            let tw = engine.measure(edit, 12.5, FONT_SANS, Weight::NORMAL);
            if tab.editing_selected && !edit.is_empty() {
                layer.rect(
                    [cx - 1.0, 12.0],
                    [tw + 2.0, TAB_BAR_H - 24.0],
                    theme.selection,
                    2.0,
                );
            }
            if !edit.is_empty() {
                let b = make_text(engine, edit, 12.5, FONT_SANS, Weight::NORMAL);
                layer.text(b, cx, (TAB_BAR_H - 17.0) / 2.0, theme.text_bright);
            }
            if blink_on {
                layer.rect(
                    [cx + tw + 1.0, 13.0],
                    [1.5, TAB_BAR_H - 26.0],
                    theme.accent,
                    0.0,
                );
            }
        } else {
            let sync_w = if tab.sync { 20.0 } else { 0.0 };
            let panes = panes_label(tab);
            let panes_w = panes
                .as_ref()
                .map(|l| engine.measure(l, 11.0, FONT_SANS, Weight::NORMAL) + 12.0)
                .unwrap_or(0.0);
            let title = elide(
                engine,
                &tab.title,
                12.5,
                close_x - 10.0 - sync_w - panes_w - cx,
            );
            if !title.is_empty() {
                let title_w = engine.measure(&title, 12.5, FONT_SANS, Weight::NORMAL);
                let title_buf = make_text(engine, &title, 12.5, FONT_SANS, Weight::NORMAL);
                layer.text(title_buf, cx, (TAB_BAR_H - 17.0) / 2.0, text_color);
                cx += title_w + 6.0;
            }
            // "2 panes" pill (design 02: 11px muted on a faint lift)
            if let Some(label) = panes
                && cx + panes_w <= close_x - 6.0
            {
                let lift = if theme.is_light() {
                    Color::BLACK
                } else {
                    Color::WHITE
                };
                layer.rect(
                    [cx, (TAB_BAR_H - 17.0) / 2.0],
                    [panes_w, 17.0],
                    lift.with_alpha(0.06),
                    4.0,
                );
                let b = make_text(engine, &label, 11.0, FONT_SANS, Weight::NORMAL);
                layer.text(b, cx + 6.0, (TAB_BAR_H - 14.5) / 2.0, theme.text_muted);
                cx += panes_w + 6.0;
            }
            if tab.sync {
                // two linked rings: inputs are broadcast across panes
                let y = TAB_BAR_H / 2.0 - 3.5;
                for dx in [0.0, 5.0] {
                    layer.bordered(
                        [cx + dx, y],
                        [8.0, 7.0],
                        Color::TRANSPARENT,
                        theme.warning,
                        3.5,
                        1.3,
                    );
                }
            }
        }

        // close ×
        let close_color = if is_active {
            theme.text_muted
        } else {
            theme.text_muted.with_alpha(0.7)
        };
        let close_buf = make_text(engine, "×", 14.0, FONT_SANS, Weight::NORMAL);
        layer.text(close_buf, close_x, (TAB_BAR_H - 19.0) / 2.0, close_color);
        hits.push(Hit {
            rect: [close_x - 4.0, 6.0, 18.0, TAB_BAR_H - 12.0],
            target: ClickTarget::TabClose(i),
        });

        hits.push(Hit {
            rect: [x, 0.0, tab_w - 26.0, TAB_BAR_H],
            target: ClickTarget::TabSelect(i),
        });

        // hairline between inactive neighbours
        if !is_active && i + 1 < tabs.len() && i + 1 != active {
            layer.rect(
                [x + tab_w - 0.5, 12.0],
                [1.0, TAB_BAR_H - 24.0],
                theme.border,
                0.0,
            );
        }

        x += tab_w;
    }

    // + new tab
    let plus_buf = make_text(engine, "+", 17.0, FONT_SANS, Weight::NORMAL);
    layer.text(plus_buf, x + 12.0, (TAB_BAR_H - 22.0) / 2.0, theme.text_dim);
    hits.push(Hit {
        rect: [x + 2.0, 6.0, 28.0, TAB_BAR_H - 12.0],
        target: ClickTarget::TabNew,
    });

    // empty strip drags the window (Linux has no native titlebar)
    #[cfg(not(target_os = "macos"))]
    hits.push(Hit {
        rect: [x + 34.0, 0.0, (w - x - 140.0).max(0.0), TAB_BAR_H],
        target: ClickTarget::WindowDrag,
    });

    if show_hint && !hint.is_empty() {
        let hx = w - hint_w - 14.0;
        if hint_pill {
            let lift = if theme.is_light() {
                Color::BLACK
            } else {
                Color::WHITE
            };
            layer.rect(
                [hx, (TAB_BAR_H - 19.0) / 2.0],
                [hint_w, 19.0],
                lift.with_alpha(0.05),
                5.0,
            );
        }
        let hint_buf = make_text(engine, hint, 11.5, mono_family(), Weight::NORMAL);
        layer.text(
            hint_buf,
            hx + if hint_pill { 7.0 } else { 0.0 },
            (TAB_BAR_H - 15.0) / 2.0,
            theme.text_muted,
        );
    }
}

fn panes_label(tab: &TabInfo) -> Option<String> {
    (tab.panes > 1).then(|| format!("{} panes", tab.panes))
}
