use shift_core::{Color, Theme};
use shift_renderer::glyphon::Weight;
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::state::{ClickTarget, Hit};

pub const TAB_BAR_H: f32 = 42.0;
/// Where the first tab starts (room for the traffic lights).
const TABS_X: f32 = 78.0;
/// Narrowest a tab gets before titles are elided to nothing.
const MIN_TAB_W: f32 = 96.0;
/// Width of the `‹N` / `N›` chips for tabs scrolled out of the strip.
const OVERFLOW_CHIP_W: f32 = 34.0;

pub struct TabInfo {
    pub title: String,
    /// Pane count; > 1 shows the design-02 "N panes" pill after the title.
    pub panes: usize,
    pub shell_label: String,
    pub activity: bool,
    /// Kind of the tab's newest unread notification (shown in place of the
    /// activity dot).
    pub alert: Option<crate::views::notifications::Kind>,
    /// Tab color (Warp tab colors): tints the underline and a leading dot.
    pub color: Option<Color>,
    /// Synchronized inputs are on for this tab.
    pub sync: bool,
    /// Inline rename in progress: the edited text.
    pub editing: Option<String>,
    /// The rename text is fully selected (first keystroke replaces it).
    pub editing_selected: bool,
    /// Directory and git branch of the tab's focused pane (vertical rail).
    pub subtitle: String,
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

/// State of the top-bar panel buttons.
#[derive(Debug, Clone, Copy, Default)]
pub struct PanelButtons {
    pub explorer_open: bool,
    pub review_open: bool,
    /// Working-tree `(added, removed)` lines; `None` outside a repository.
    pub diff: Option<(usize, usize)>,
    pub notifications_open: bool,
    /// Unread notifications (dot on the inbox button).
    pub unread: usize,
}

/// A window outline with its left column filled (sidebar toggle).
fn sidebar_icon(layer: &mut Layer, theme: &Theme, r: [f32; 4], color: Color) {
    let (iw, ih) = (16.0, 13.0);
    let (ix, iy) = (r[0] + (r[2] - iw) / 2.0, r[1] + (r[3] - ih) / 2.0);
    layer.bordered(
        [ix, iy],
        [iw, ih],
        theme.bg_chrome.with_alpha(0.0),
        color,
        2.5,
        1.3,
    );
    layer.rect([ix, iy], [5.5, ih], color, 2.0);
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
    panels: PanelButtons,
    hits: &mut Vec<Hit>,
) {
    layer.rect(
        [0.0, 0.0],
        [w, TAB_BAR_H],
        theme.ground(theme.bg_chrome),
        0.0,
    );
    layer.hline(0.0, TAB_BAR_H - 1.0, w, 1.0, theme.border);

    // Warp layout: the sidebar (file explorer) toggle right after the
    // traffic lights, the code review button at the far right showing the
    // working tree's `+added −removed`
    let size = 28.0;
    let by = (TAB_BAR_H - size) / 2.0;
    let explorer = [TABS_X + 2.0, by, size, size];
    if panels.explorer_open {
        layer.rect(
            [explorer[0], by],
            [size, size],
            theme.accent.with_alpha(0.14),
            6.0,
        );
    }
    sidebar_icon(
        layer,
        theme,
        explorer,
        if panels.explorer_open {
            theme.accent
        } else {
            theme.text_dim
        },
    );
    hits.push(Hit {
        rect: explorer,
        target: ClickTarget::ToggleExplorer,
    });
    let tabs_x = explorer[0] + size + 8.0;

    let review_color = if panels.review_open {
        theme.accent
    } else {
        theme.text_dim
    };
    let counts = panels
        .diff
        .filter(|(a, d)| a + d > 0)
        .map(|(a, d)| (format!("+{a}"), format!("−{d}")));
    let review_w = match &counts {
        Some((a, d)) => {
            14.0 + 12.0
                + 6.0
                + engine.measure(a, 11.5, mono_family(), Weight::MEDIUM)
                + 6.0
                + engine.measure(d, 11.5, mono_family(), Weight::MEDIUM)
                + 10.0
        }
        None => size,
    };
    let review = [w - 10.0 - review_w, by, review_w, size];
    if panels.review_open {
        layer.rect(
            [review[0], by],
            [review_w, size],
            theme.accent.with_alpha(0.14),
            6.0,
        );
    }
    // ± glyph as quads (crisp at any scale): alone when the tree is clean,
    // followed by the counts otherwise
    let gx = match &counts {
        Some(_) => review[0] + 10.0,
        None => review[0] + (size - 12.0) / 2.0,
    };
    let gy = by + 8.0;
    layer.rect([gx + 5.25, gy], [1.5, 8.0], review_color, 0.5);
    layer.rect([gx + 2.0, gy + 3.25], [8.0, 1.5], review_color, 0.5);
    layer.rect([gx + 2.0, gy + 10.5], [8.0, 1.5], review_color, 0.5);
    if let Some((a, d)) = &counts {
        let mut tx = gx + 12.0 + 6.0;
        for (s, c) in [(a, theme.success), (d, theme.error)] {
            let b = make_text(engine, s, 11.5, mono_family(), Weight::MEDIUM);
            layer.text(b, tx, (TAB_BAR_H - 15.0) / 2.0, c);
            tx += engine.measure(s, 11.5, mono_family(), Weight::MEDIUM) + 6.0;
        }
    }
    hits.push(Hit {
        rect: review,
        target: ClickTarget::ToggleReview,
    });

    // notifications inbox, left of the review button (Warp layout)
    let inbox = [review[0] - 6.0 - size, by, size, size];
    if panels.notifications_open {
        layer.rect(
            [inbox[0], by],
            [size, size],
            Color::WHITE.with_alpha(0.12),
            6.0,
        );
    }
    crate::views::notifications::inbox_icon(
        layer,
        inbox,
        if panels.notifications_open {
            theme.text_bright
        } else {
            theme.text_dim
        },
    );
    if panels.unread > 0 {
        let c = [inbox[0] + size - 7.0, by + 7.0];
        layer.circle(c, 8.5, theme.ground(theme.bg_chrome));
        layer.circle(c, 6.0, theme.accent);
    }
    hits.push(Hit {
        rect: inbox,
        target: ClickTarget::ToggleNotifications,
    });
    // everything else on the right lays out left of these buttons
    let w = inbox[0] - 6.0;

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
            let dot_w =
                if (tab.activity && i != active) || tab.color.is_some() || tab.alert.is_some() {
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
    let avail = (w - tabs_x - 40.0 - hint_w - 28.0).max(MIN_TAB_W);
    let total: f32 = natural.iter().sum();
    let n = tabs.len();
    // visible window `start..end`; everything outside it collapses into the
    // `‹N` / `N›` chips so the active tab always stays on screen
    let (mut start, mut end) = (0, n);
    let (widths, show_hint): (Vec<f32>, bool) = if total <= avail {
        (natural, true)
    } else {
        // without the hint first, then squeeze, then scroll around the active tab
        let avail = (w - tabs_x - 40.0).max(MIN_TAB_W);
        if MIN_TAB_W * n as f32 <= avail {
            let each = avail / n.max(1) as f32;
            (natural.iter().map(|x| x.min(each)).collect(), false)
        } else {
            let k = (((avail - 2.0 * OVERFLOW_CHIP_W) / MIN_TAB_W) as usize).clamp(1, n);
            start = active.min(n - 1).saturating_sub(k / 2).min(n - k);
            end = start + k;
            let chips = (start > 0) as u8 as f32 + (end < n) as u8 as f32;
            let each = (avail - chips * OVERFLOW_CHIP_W) / k as f32;
            (natural.iter().map(|x| x.min(each)).collect(), false)
        }
    };

    let mut x = tabs_x;
    let chip = |layer: &mut Layer,
                engine: &mut TextEngine,
                hits: &mut Vec<Hit>,
                x: f32,
                label: String,
                to: usize| {
        let tw = engine.measure(&label, 11.5, mono_family(), Weight::NORMAL);
        let b = make_text(engine, &label, 11.5, mono_family(), Weight::NORMAL);
        layer.text(
            b,
            x + (OVERFLOW_CHIP_W - tw) / 2.0,
            (TAB_BAR_H - 15.0) / 2.0,
            theme.text_dim,
        );
        hits.push(Hit {
            rect: [x, 0.0, OVERFLOW_CHIP_W, TAB_BAR_H],
            target: ClickTarget::TabSelect(to),
        });
    };
    if start > 0 {
        chip(layer, engine, hits, x, format!("‹{start}"), start - 1);
        x += OVERFLOW_CHIP_W;
    }
    for (i, tab) in tabs.iter().enumerate().skip(start).take(end - start) {
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

        if let Some(kind) = tab.alert {
            crate::views::notifications::tab_badge(layer, theme, kind, cx + 3.5, TAB_BAR_H / 2.0);
            cx += 15.0;
        } else if tab.activity && !is_active {
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
        if !is_active && i + 1 < end && i + 1 != active {
            layer.rect(
                [x + tab_w - 0.5, 12.0],
                [1.0, TAB_BAR_H - 24.0],
                theme.border,
                0.0,
            );
        }

        x += tab_w;
    }

    if end < n {
        chip(layer, engine, hits, x, format!("{}›", n - end), end);
        x += OVERFLOW_CHIP_W;
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

/// Width of the vertical tab rail (`tab_layout = "vertical"`).
pub const RAIL_W: f32 = 220.0;
const RAIL_ROW_H: f32 = 54.0;

/// Vertical tab rail: one row per tab under the chrome strip, with the
/// title, the directory and git branch, pane count, activity and color.
#[allow(clippy::too_many_arguments)]
pub fn render_rail(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    tabs: &[TabInfo],
    active: usize,
    h: f32,
    blink_on: bool,
    hits: &mut Vec<Hit>,
) {
    layer.rect(
        [0.0, TAB_BAR_H],
        [RAIL_W, h - TAB_BAR_H],
        theme.ground(theme.bg_chrome),
        0.0,
    );
    layer.rect(
        [RAIL_W - 1.0, TAB_BAR_H],
        [1.0, h - TAB_BAR_H],
        theme.border,
        0.0,
    );

    let top = TAB_BAR_H + 6.0;
    let avail = (h - 36.0 - top).max(0.0);
    let fits = (avail / RAIL_ROW_H).floor() as usize;
    // overflowing: the "+N above" line takes a slot of its own at the top;
    // "+N more" sits in the bottom margin under the last row
    let capacity = if tabs.len() <= fits {
        fits
    } else {
        ((avail - RAIL_MORE_H) / RAIL_ROW_H).floor().max(1.0) as usize
    };
    let window = rail_window(tabs.len(), active, capacity);
    let mut y = top;
    if window.start > 0 {
        rail_overflow(
            layer,
            engine,
            theme,
            &format!("+{} above", window.start),
            &tabs[..window.start],
            y,
            window.start - 1,
            hits,
        );
        y += RAIL_MORE_H;
    }
    for (i, tab) in tabs.iter().enumerate().take(window.end).skip(window.start) {
        let is_active = i == active;
        let row = [6.0, y, RAIL_W - 12.0, RAIL_ROW_H - 4.0];
        if is_active {
            layer.rect(
                [row[0], row[1]],
                [row[2], row[3]],
                tab.color.unwrap_or(theme.accent).with_alpha(0.10),
                7.0,
            );
            layer.rect(
                [row[0], row[1] + 8.0],
                [2.5, row[3] - 16.0],
                tab.color.unwrap_or(theme.accent),
                1.0,
            );
        }
        let mut cx = row[0] + 14.0;
        let line1 = row[1] + 9.0;
        if let Some(kind) = tab.alert {
            crate::views::notifications::tab_badge(layer, theme, kind, cx + 3.5, line1 + 8.0);
            cx += 14.0;
        } else if tab.activity && !is_active {
            layer.circle([cx + 3.5, line1 + 8.0], 7.0, theme.success);
            cx += 14.0;
        } else if let Some(c) = tab.color {
            layer.circle([cx + 3.5, line1 + 8.0], 8.0, c);
            cx += 14.0;
        }
        let close_x = row[0] + row[2] - 20.0;
        let shell_w = engine.measure(&tab.shell_label, 11.5, mono_family(), Weight::NORMAL);
        let b = make_text(
            engine,
            &tab.shell_label,
            11.5,
            mono_family(),
            Weight::NORMAL,
        );
        layer.text(b, cx, line1 + 1.0, theme.accent);
        cx += shell_w + 7.0;

        let title_color = if is_active {
            theme.text_bright
        } else {
            theme.text_ui
        };
        if let Some(edit) = &tab.editing {
            let fw = (close_x - 4.0 - cx).max(40.0);
            layer.bordered(
                [cx - 4.0, line1 - 2.0],
                [fw, 20.0],
                theme.bg,
                theme.accent,
                4.0,
                1.0,
            );
            let tw = engine.measure(edit, 12.5, FONT_SANS, Weight::NORMAL);
            if tab.editing_selected && !edit.is_empty() {
                layer.rect([cx - 1.0, line1], [tw + 2.0, 16.0], theme.selection, 2.0);
            }
            if !edit.is_empty() {
                let b = make_text(engine, edit, 12.5, FONT_SANS, Weight::NORMAL);
                layer.text(b, cx, line1, theme.text_bright);
            }
            if blink_on {
                layer.rect([cx + tw + 1.0, line1 + 1.0], [1.5, 15.0], theme.accent, 0.0);
            }
        } else {
            let title = elide(engine, &tab.title, 12.5, close_x - 8.0 - cx);
            let weight = if is_active {
                Weight::MEDIUM
            } else {
                Weight::NORMAL
            };
            let b = make_text(engine, &title, 12.5, FONT_SANS, weight);
            layer.text(b, cx, line1, title_color);
        }

        // second line: directory · branch, pane count, sync
        let mut sub = tab.subtitle.clone();
        if let Some(p) = panes_label(tab) {
            sub = if sub.is_empty() {
                p
            } else {
                format!("{sub} · {p}")
            };
        }
        if tab.sync {
            sub.push_str(" · synced");
        }
        let sub = elide(engine, &sub, 11.0, row[2] - 28.0);
        if !sub.is_empty() {
            let b = make_text(engine, &sub, 11.0, FONT_SANS, Weight::NORMAL);
            layer.text(b, row[0] + 14.0, line1 + 20.0, theme.text_muted);
        }

        let close_buf = make_text(engine, "×", 14.0, FONT_SANS, Weight::NORMAL);
        layer.text(
            close_buf,
            close_x,
            line1 - 1.0,
            theme
                .text_muted
                .with_alpha(if is_active { 1.0 } else { 0.7 }),
        );
        hits.push(Hit {
            rect: [close_x - 4.0, line1 - 3.0, 18.0, 20.0],
            target: ClickTarget::TabClose(i),
        });
        hits.push(Hit {
            rect: row,
            target: ClickTarget::TabSelect(i),
        });
        y += RAIL_ROW_H;
    }
    if window.end < tabs.len() {
        let hidden = &tabs[window.end..];
        rail_overflow(
            layer,
            engine,
            theme,
            &format!("+{} more · palette lists all", hidden.len()),
            hidden,
            y,
            window.end,
            hits,
        );
    }
}

/// Height of a "+N above" / "+N more" line in the rail.
const RAIL_MORE_H: f32 = 28.0;

/// The tabs the rail shows: at most `capacity` consecutive indices that
/// always include `active`. Starts at tab 0 while `active` fits on the first
/// page, then slides so `active` is the last visible row.
pub fn rail_window(len: usize, active: usize, capacity: usize) -> std::ops::Range<usize> {
    if len <= capacity {
        return 0..len;
    }
    let cap = capacity.max(1);
    let active = active.min(len - 1);
    let start = (active + 1).saturating_sub(cap).min(len - cap);
    start..start + cap
}

/// Severity order for the overflow marker: what most needs the user.
fn alert_rank(kind: crate::views::notifications::Kind) -> u8 {
    use crate::views::notifications::Kind;
    match kind {
        Kind::Request => 4,
        Kind::Failed => 3,
        Kind::Bell => 2,
        Kind::Notify => 1,
        Kind::Finished => 0,
    }
}

/// The marker an overflow line carries: the worst alert among `hidden`, or
/// activity when none has an alert.
fn hidden_marker(hidden: &[TabInfo]) -> Option<Option<crate::views::notifications::Kind>> {
    let worst = hidden
        .iter()
        .filter_map(|t| t.alert)
        .max_by_key(|k| alert_rank(*k));
    match worst {
        Some(kind) => Some(Some(kind)),
        None => hidden.iter().any(|t| t.activity).then_some(None),
    }
}

/// One overflow line: the hidden tabs' marker (same badge as a row), the
/// label, and a hit that selects the nearest hidden tab so the window
/// slides toward it.
#[allow(clippy::too_many_arguments)]
fn rail_overflow(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    text: &str,
    hidden: &[TabInfo],
    y: f32,
    nearest: usize,
    hits: &mut Vec<Hit>,
) {
    let row = [6.0, y, RAIL_W - 12.0, RAIL_MORE_H - 4.0];
    let (mx, cy) = (row[0] + 17.5, y + 12.0);
    let mut tx = 16.0;
    match hidden_marker(hidden) {
        Some(Some(kind)) => {
            crate::views::notifications::tab_badge(layer, theme, kind, mx, cy);
            tx += 20.0;
        }
        Some(None) => {
            layer.circle([mx, cy], 7.0, theme.success);
            tx += 20.0;
        }
        None => {}
    }
    let b = make_text(engine, text, 11.0, FONT_SANS, Weight::NORMAL);
    layer.text(b, tx, y + 5.0, theme.text_muted);
    hits.push(Hit {
        rect: row,
        target: ClickTarget::TabSelect(nearest),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::notifications::Kind;

    #[test]
    fn rail_window_keeps_the_active_tab_visible() {
        let w = rail_window(8, 7, 4);
        assert!(w.contains(&7));
        assert_eq!(w, 4..8);
        assert_eq!(rail_window(8, 0, 4), 0..4);
        assert_eq!(rail_window(8, 3, 4), 0..4);
        assert_eq!(rail_window(8, 5, 4), 2..6);
        for active in 0..8 {
            let w = rail_window(8, active, 3);
            assert!(w.contains(&active), "active {active} in {w:?}");
            assert_eq!(w.len(), 3);
        }
    }

    #[test]
    fn rail_window_shows_everything_when_it_fits() {
        assert_eq!(rail_window(3, 2, 4), 0..3);
        assert_eq!(rail_window(4, 3, 4), 0..4);
        assert_eq!(rail_window(0, 0, 4), 0..0);
        // no room at all still shows the active tab
        assert_eq!(rail_window(5, 3, 0), 3..4);
        // stale active index past the end clamps
        assert_eq!(rail_window(5, 9, 2), 3..5);
    }

    fn tab(alert: Option<Kind>, activity: bool) -> TabInfo {
        TabInfo {
            title: String::new(),
            panes: 1,
            shell_label: String::new(),
            activity,
            alert,
            color: None,
            sync: false,
            editing: None,
            editing_selected: false,
            subtitle: String::new(),
        }
    }

    #[test]
    fn overflow_marker_is_the_worst_hidden_alert() {
        let hidden = [
            tab(Some(Kind::Finished), false),
            tab(Some(Kind::Request), false),
            tab(Some(Kind::Failed), true),
        ];
        assert_eq!(hidden_marker(&hidden), Some(Some(Kind::Request)));
        assert_eq!(
            hidden_marker(&[tab(None, true), tab(None, false)]),
            Some(None)
        );
        assert_eq!(hidden_marker(&[tab(None, false)]), None);
    }
}
