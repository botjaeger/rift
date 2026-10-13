use rift_core::{Color, Theme};
use rift_renderer::glyphon::Weight;
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::state::{ClickTarget, Hit};
use crate::views::glyphs;
use crate::views::icons::{self, Icon};

pub const TAB_BAR_H: f32 = 42.0;
/// Where the first tab starts (room for the traffic lights).
const TABS_X: f32 = 78.0;
/// Centre of the first traffic light and the distance between centres
/// (native buttons on macOS are moved there, Linux draws its own).
pub const TRAFFIC_X: f32 = 20.0;
pub const TRAFFIC_GAP: f32 = 20.0;
/// Narrowest a tab gets before titles are elided to nothing.
const MIN_TAB_W: f32 = 120.0;
/// Width of the `‹N` / `N›` chips for tabs scrolled out of the strip.
const OVERFLOW_CHIP_W: f32 = 34.0;

pub struct TabInfo {
    pub title: String,
    /// Pane count; > 1 shows the count chip after the title.
    pub panes: usize,
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
    /// The Settings tab (a page, not a shell): settings icon, × closes it.
    pub settings: bool,
    /// A command runs in one of its panes: a spinner leads the title (in
    /// place of the sync icon).
    pub running: bool,
}

fn make_text(
    engine: &mut TextEngine,
    text: &str,
    size: f32,
    family: &'static str,
    weight: Weight,
) -> rift_renderer::glyphon::Buffer {
    use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping};
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
#[derive(Debug, Clone, Default)]
pub struct PanelButtons {
    pub explorer_open: bool,
    pub review_open: bool,
    /// Working-tree `(added, removed)` lines; `None` outside a repository.
    pub diff: Option<(usize, usize)>,
    pub notifications_open: bool,
    /// Unread notifications (dot on the inbox button).
    pub unread: usize,
    /// A new release is available: the "Update Rift" chip.
    pub update: bool,
    /// A phone is connected: the remote chip's label ("Remote · Sam's
    /// iPhone", "Remote · 2 devices").
    pub remote: Option<String>,
}

/// Height of the "Update Rift" chip.
const UPDATE_CHIP_H: f32 = 22.0;
/// Widest the remote chip's label grows (a long device name is cut).
const REMOTE_LABEL_MAX_W: f32 = 200.0;

/// Warp's "Update Warp" pill: an outline and text in `color` on no fill, a
/// hover lift, ending at `right`. Returns its rect; `None` (nothing drawn)
/// when it would start left of `min_x` (a narrow window keeps its tabs).
#[allow(clippy::too_many_arguments)]
fn chrome_chip(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    label: &str,
    color: Color,
    right: f32,
    min_x: f32,
    mouse: Option<(f32, f32)>,
) -> Option<[f32; 4]> {
    let tw = engine.measure(label, 12.0, FONT_SANS, Weight::MEDIUM);
    let w = (tw + 20.0).round();
    if right - w < min_x {
        return None;
    }
    let r = [
        right - w,
        ((TAB_BAR_H - UPDATE_CHIP_H) / 2.0).round(),
        w,
        UPDATE_CHIP_H,
    ];
    let fill = if inside(r, mouse) {
        lift(theme, 0.08)
    } else {
        Color::TRANSPARENT
    };
    layer.bordered(
        [r[0], r[1]],
        [r[2], r[3]],
        fill,
        color,
        UPDATE_CHIP_H / 2.0,
        1.0,
    );
    let b = make_text(engine, label, 12.0, FONT_SANS, Weight::MEDIUM);
    layer.text(b, r[0] + 10.0, r[1] + (UPDATE_CHIP_H - 15.6) / 2.0, color);
    Some(r)
}

/// A 26 px chrome icon button: hover lift, accent tint while `on`.
#[allow(clippy::too_many_arguments)]
fn icon_button(
    layer: &mut Layer,
    theme: &Theme,
    r: [f32; 4],
    icon: Icon,
    size: f32,
    on: bool,
    hover: bool,
) {
    if on {
        layer.rect(
            [r[0], r[1]],
            [r[2], r[3]],
            theme.accent.with_alpha(0.14),
            4.0,
        );
    } else if hover {
        layer.rect([r[0], r[1]], [r[2], r[3]], lift(theme, 0.08), 4.0);
    }
    let color = if on {
        theme.accent
    } else if hover {
        theme.text_ui
    } else {
        theme.text_dim
    };
    icons::draw(
        layer,
        icon,
        [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0],
        size,
        color,
    );
}

/// White on dark themes, black on light ones, at `alpha` (hover lifts).
fn lift(theme: &Theme, alpha: f32) -> Color {
    if theme.is_light() {
        Color::BLACK.with_alpha(alpha)
    } else {
        Color::WHITE.with_alpha(alpha)
    }
}

fn inside(r: [f32; 4], p: Option<(f32, f32)>) -> bool {
    p.is_some_and(|(x, y)| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3])
}

/// The tab being dragged: its index and the pointer-following left edge.
#[derive(Clone, Copy, Debug)]
pub struct Lifted {
    pub index: usize,
    pub x: f32,
}

/// Draw the 42 px tab strip and report click targets. `mouse` drives hover
/// states; a `lifted` tab is drawn on `over` (a layer above the strip) so
/// it floats over its neighbours, while its slot shows the drop caret.
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    over: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    tabs: &[TabInfo],
    active: usize,
    w: f32,
    blink_on: bool,
    panels: PanelButtons,
    mouse: Option<(f32, f32)>,
    lifted: Option<Lifted>,
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
    let size = 26.0;
    let by = (TAB_BAR_H - size) / 2.0;
    let explorer = [TABS_X + 2.0, by, size, size];
    icon_button(
        layer,
        theme,
        explorer,
        Icon::Sidebar,
        16.0,
        panels.explorer_open,
        inside(explorer, mouse),
    );
    hits.push(Hit {
        rect: explorer,
        target: ClickTarget::ToggleExplorer,
    });
    let tabs_x = explorer[0] + size + 8.0;

    let counts = panels
        .diff
        .filter(|(a, d)| a + d > 0)
        .map(|(a, d)| (format!("+{a}"), format!("−{d}")));
    let review_w = match &counts {
        Some((a, d)) => {
            16.0 + engine.measure(a, 12.0, mono_family(), Weight::MEDIUM)
                + 6.0
                + engine.measure(d, 12.0, mono_family(), Weight::MEDIUM)
        }
        None => size,
    };
    let review = [w - 10.0 - review_w, by, review_w, size];
    let review_hover = inside(review, mouse);
    if panels.review_open {
        layer.rect(
            [review[0], by],
            [review_w, size],
            theme.accent.with_alpha(0.14),
            4.0,
        );
    } else if review_hover {
        layer.rect([review[0], by], [review_w, size], lift(theme, 0.08), 4.0);
    }
    match &counts {
        Some((a, d)) => {
            let mut tx = review[0] + 8.0;
            for (s, c) in [(a, theme.success), (d, theme.error)] {
                let b = make_text(engine, s, 12.0, mono_family(), Weight::MEDIUM);
                layer.text(b, tx, (TAB_BAR_H - 15.6) / 2.0, c);
                tx += engine.measure(s, 12.0, mono_family(), Weight::MEDIUM) + 6.0;
            }
        }
        None => {
            // ± as quads (crisp at any scale) when the tree is clean
            let color = if panels.review_open {
                theme.accent
            } else {
                theme.text_dim
            };
            let (gx, gy) = (review[0] + (size - 12.0) / 2.0, by + 7.0);
            layer.rect([gx + 5.25, gy], [1.5, 8.0], color, 0.5);
            layer.rect([gx + 2.0, gy + 3.25], [8.0, 1.5], color, 0.5);
            layer.rect([gx + 2.0, gy + 10.5], [8.0, 1.5], color, 0.5);
        }
    }
    hits.push(Hit {
        rect: review,
        target: ClickTarget::ToggleReview,
    });

    // notifications inbox, left of the review button (Warp layout)
    let inbox = [review[0] - 4.0 - size, by, size, size];
    icon_button(
        layer,
        theme,
        inbox,
        Icon::Inbox,
        16.0,
        panels.notifications_open,
        inside(inbox, mouse),
    );
    if panels.unread > 0 {
        let c = [inbox[0] + size - 7.0, by + 7.0];
        layer.circle(c, 9.0, theme.ground(theme.bg_chrome));
        layer.circle(c, 6.0, theme.accent);
    }
    hits.push(Hit {
        rect: inbox,
        target: ClickTarget::ToggleNotifications,
    });
    // a new release: the chip left of the inbox
    let mut w = inbox[0] - 6.0;
    let mut chip_right = inbox[0] - 8.0;
    // room for one tab and + stays
    let min_x = tabs_x + MIN_TAB_W + 40.0 + 6.0;
    if panels.update
        && let Some(chip) = chrome_chip(
            layer,
            engine,
            theme,
            "Update Rift",
            theme.warning,
            chip_right,
            min_x,
            mouse,
        )
    {
        hits.push(Hit {
            rect: chip,
            target: ClickTarget::UpdateChip,
        });
        w = chip[0] - 6.0;
        chip_right = chip[0] - 6.0;
    }
    // a connected phone: its chip left of those (accent: something is
    // attached to this session right now)
    if let Some(label) = &panels.remote {
        let label = super::palette::elide_to(engine, label, 12.0, FONT_SANS, REMOTE_LABEL_MAX_W);
        if let Some(chip) = chrome_chip(
            layer,
            engine,
            theme,
            &label,
            theme.accent,
            chip_right,
            min_x,
            mouse,
        ) {
            hits.push(Hit {
                rect: chip,
                target: ClickTarget::RemoteChip,
            });
            w = chip[0] - 6.0;
        }
    }
    // everything else on the right lays out left of these buttons
    let w = w;

    // traffic lights: native on macOS, drawn (and clickable) on Linux
    #[cfg(not(target_os = "macos"))]
    {
        for (i, c) in ["#ff5f57", "#febc2e", "#28c840"].iter().enumerate() {
            let cx = TRAFFIC_X + i as f32 * TRAFFIC_GAP;
            layer.circle([cx, TAB_BAR_H / 2.0], 12.0, Color::hex(c).unwrap());
            hits.push(Hit {
                rect: [cx - 8.0, TAB_BAR_H / 2.0 - 8.0, 16.0, 16.0],
                target: ClickTarget::TrafficLight(i as u8),
            });
        }
    }

    // natural widths, then shrink evenly when the tabs overflow the strip
    let natural: Vec<f32> = tabs
        .iter()
        .map(|tab| {
            let title = tab.editing.as_deref().unwrap_or(&tab.title);
            let title_w = engine
                .measure(title, 12.5, FONT_SANS, Weight::NORMAL)
                .max(if tab.editing.is_some() { 90.0 } else { 0.0 });
            let chip_w = chip_label(tab)
                .map(|l| engine.measure(&l, 10.5, FONT_SANS, Weight::MEDIUM) + 12.0 + 6.0)
                .unwrap_or(0.0);
            let icon_w = if tab.settings || tab.sync {
                14.0 + 8.0
            } else {
                0.0
            };
            (TAB_PAD_L + icon_w + title_w + chip_w + 8.0 + CLOSE_BOX + 8.0).clamp(TAB_W, MAX_TAB_W)
        })
        .collect();
    let avail = (w - tabs_x - 40.0).max(MIN_TAB_W);
    let total: f32 = natural.iter().sum();
    let n = tabs.len();
    // visible window `start..end`; everything outside it collapses into the
    // `‹N` / `N›` chips so the active tab always stays on screen
    let (mut start, mut end) = (0, n);
    let widths: Vec<f32> = if total <= avail {
        natural
    } else if MIN_TAB_W * n as f32 <= avail {
        let each = avail / n.max(1) as f32;
        natural.iter().map(|x| x.min(each)).collect()
    } else {
        let k = (((avail - 2.0 * OVERFLOW_CHIP_W) / MIN_TAB_W) as usize).clamp(1, n);
        start = active.min(n - 1).saturating_sub(k / 2).min(n - k);
        end = start + k;
        let chips = (start > 0) as u8 as f32 + (end < n) as u8 as f32;
        let each = (avail - chips * OVERFLOW_CHIP_W) / k as f32;
        natural.iter().map(|x| x.min(each)).collect()
    };

    let mut x = tabs_x;
    let chip = |layer: &mut Layer,
                engine: &mut TextEngine,
                hits: &mut Vec<Hit>,
                x: f32,
                label: String,
                to: usize| {
        let tw = engine.measure(&label, 11.5, FONT_SANS, Weight::MEDIUM);
        let b = make_text(engine, &label, 11.5, FONT_SANS, Weight::MEDIUM);
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
        chip(layer, engine, hits, x, format!("‹ {start}"), start - 1);
        x += OVERFLOW_CHIP_W;
    }
    for (i, tab) in tabs.iter().enumerate().skip(start).take(end - start) {
        let tab_w = widths[i];
        let slot = [x, 0.0, tab_w, TAB_BAR_H];
        let lift_x = lifted
            .filter(|l| l.index == i)
            .map(|l| l.x.clamp(tabs_x, (w - tab_w).max(tabs_x)));
        let hovered = lifted.is_none() && inside(slot, mouse);
        match lift_x {
            Some(lx) => {
                // the slot it will land in: an empty well with a caret
                layer.rect([x, 0.0], [tab_w, TAB_BAR_H - 1.0], lift(theme, 0.02), 0.0);
                layer.rect([x, 6.0], [2.0, TAB_BAR_H - 12.0], theme.accent, 1.0);
                draw_tab(
                    over,
                    engine,
                    theme,
                    tab,
                    [lx, -4.0, tab_w, TAB_BAR_H],
                    i == active,
                    true,
                    true,
                    blink_on,
                );
            }
            None => draw_tab(
                layer,
                engine,
                theme,
                tab,
                slot,
                i == active,
                hovered,
                false,
                blink_on,
            ),
        }
        let close = close_rect(slot);
        if i == active || hovered || tab.editing.is_some() {
            hits.push(Hit {
                rect: close,
                target: close_target(tab, i),
            });
        }
        hits.push(Hit {
            rect: slot,
            target: ClickTarget::TabSelect(i),
        });
        x += tab_w;
    }

    if end < n {
        chip(layer, engine, hits, x, format!("{} ›", n - end), end);
        x += OVERFLOW_CHIP_W;
    }

    // + new tab
    let plus = [x + 6.0, by, size, size];
    icon_button(
        layer,
        theme,
        plus,
        Icon::Plus,
        14.0,
        false,
        inside(plus, mouse),
    );
    hits.push(Hit {
        rect: plus,
        target: ClickTarget::TabNew,
    });

    // the empty strip drags the window (macOS: the native titlebar ends at
    // the traffic lights; Linux has none)
    hits.push(Hit {
        rect: [x + 34.0, 0.0, (w - x - 34.0).max(0.0), TAB_BAR_H],
        target: ClickTarget::WindowDrag,
    });
}

/// Left padding, the close button box and the tab width bounds.
const TAB_PAD_L: f32 = 14.0;
const CLOSE_BOX: f32 = 20.0;
const TAB_W: f32 = 150.0;
const MAX_TAB_W: f32 = 240.0;

fn close_rect(slot: [f32; 4]) -> [f32; 4] {
    [
        slot[0] + slot[2] - 8.0 - CLOSE_BOX,
        (TAB_BAR_H - CLOSE_BOX) / 2.0,
        CLOSE_BOX,
        CLOSE_BOX,
    ]
}

/// One tab: terminal (or sync) icon, title, pane count chip, then × when
/// active or hovered, else its alert or activity dot.
#[allow(clippy::too_many_arguments)]
fn draw_tab(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    tab: &TabInfo,
    r: [f32; 4],
    is_active: bool,
    hovered: bool,
    floating: bool,
    blink_on: bool,
) {
    let [x, y, tw, th] = r;
    let mid = y + th / 2.0;
    if floating {
        layer.shadow([x, y], [tw, th], Color::BLACK.with_alpha(0.55), 0.0, 10.0);
        layer.bordered([x, y], [tw, th], theme.bg, lift(theme, 0.12), 0.0, 1.0);
    } else if is_active {
        // joins the pane below: covers the strip's bottom hairline
        layer.rect([x, y], [tw, th], theme.ground(theme.bg), 0.0);
    } else if hovered {
        layer.rect([x, y], [tw, th - 1.0], lift(theme, 0.03), 0.0);
    }
    // top edge: accent on the active tab, the tab color on any colored one
    match (tab.color, is_active) {
        (Some(c), _) => layer.rect([x, y], [tw, 2.0], c, 0.0),
        (None, true) => layer.rect([x, y], [tw, 2.0], theme.accent, 0.0),
        _ => {}
    }
    if !floating {
        // both edges: the left one lands on the previous tab's right edge,
        // and gives the first tab a left edge against the sidebar button
        layer.rect([x - 1.0, y], [1.0, th - 1.0], theme.border, 0.0);
        layer.rect([x + tw - 1.0, y], [1.0, th - 1.0], theme.border, 0.0);
    }

    let text_color = if is_active || hovered {
        theme.text_ui
    } else {
        theme.text_dim
    };
    let mut cx = x + TAB_PAD_L;
    // shell tabs carry no icon; Settings and synced tabs say what they are,
    // and one running a command spins
    let icon = if tab.settings {
        Some((Icon::Settings, text_color))
    } else if tab.running {
        glyphs::draw_spinner(layer, [cx + 7.0, mid], 13.0, text_color);
        cx += 14.0 + 8.0;
        None
    } else if tab.sync {
        Some((Icon::Sync, theme.accent))
    } else {
        None
    };
    if let Some((icon, icon_color)) = icon {
        icons::draw(layer, icon, [cx + 7.0, mid], 14.0, icon_color);
        cx += 14.0 + 8.0;
    }

    let close = close_rect(r);
    let close = [close[0], mid - CLOSE_BOX / 2.0, CLOSE_BOX, CLOSE_BOX];
    if let Some(edit) = &tab.editing {
        // inline rename field
        let fw = (close[0] - 6.0 - cx).max(60.0);
        layer.bordered(
            [cx - 5.0, mid - 12.0],
            [fw, 24.0],
            theme.bg,
            theme.accent,
            3.0,
            1.0,
        );
        let ew = engine.measure(edit, 12.5, FONT_SANS, Weight::NORMAL);
        if tab.editing_selected && !edit.is_empty() {
            layer.rect(
                [cx - 1.0, mid - 9.0],
                [ew + 2.0, 18.0],
                theme.selection,
                2.0,
            );
        }
        if !edit.is_empty() {
            let b = make_text(engine, edit, 12.5, FONT_SANS, Weight::NORMAL);
            layer.text(b, cx, mid - 8.5, theme.text_bright);
        }
        if blink_on {
            layer.rect([cx + ew + 1.0, mid - 8.0], [1.5, 16.0], theme.accent, 0.0);
        }
    } else {
        let chip = chip_label(tab);
        let chip_w = chip
            .as_ref()
            .map(|l| engine.measure(l, 10.5, FONT_SANS, Weight::MEDIUM) + 12.0)
            .unwrap_or(0.0);
        let room = close[0] - 8.0 - cx - if chip_w > 0.0 { chip_w + 6.0 } else { 0.0 };
        let title = elide(engine, &tab.title, 12.5, room);
        if !title.is_empty() {
            let title_w = engine.measure(&title, 12.5, FONT_SANS, Weight::NORMAL);
            let b = make_text(engine, &title, 12.5, FONT_SANS, Weight::NORMAL);
            layer.text(b, cx, mid - 8.5, text_color);
            cx += title_w + 6.0;
        }
        if let Some(label) = chip
            && cx + chip_w <= close[0] - 4.0
        {
            layer.rect([cx, mid - 8.0], [chip_w, 16.0], lift(theme, 0.07), 3.0);
            let b = make_text(engine, &label, 10.5, FONT_SANS, Weight::MEDIUM);
            layer.text(b, cx + 6.0, mid - 7.0, theme.text_dim);
        }
    }

    // right end: × while active or hovered, else the tab's alert / activity
    let (ccx, ccy) = (close[0] + CLOSE_BOX / 2.0, mid);
    if is_active || hovered || tab.editing.is_some() {
        icons::draw(
            layer,
            Icon::Close,
            [ccx, ccy],
            10.0,
            if hovered && !floating {
                theme.text_ui
            } else {
                theme.text_dim
            },
        );
    } else if let Some(kind) = tab.alert {
        crate::views::notifications::tab_badge(layer, theme, kind, ccx, ccy);
    } else if tab.activity {
        layer.circle([ccx, ccy], 6.0, theme.accent.with_alpha(0.85));
    }
}

/// The × of a tab: the Settings tab closes itself, a shell tab its panes.
fn close_target(tab: &TabInfo, i: usize) -> ClickTarget {
    if tab.settings {
        ClickTarget::SettingsClose
    } else {
        ClickTarget::TabClose(i)
    }
}

/// The pane count chip (`3`) of a split tab.
fn chip_label(tab: &TabInfo) -> Option<String> {
    (tab.panes > 1).then(|| tab.panes.to_string())
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
        let row = [0.0, y, RAIL_W - 1.0, RAIL_ROW_H];
        if is_active {
            // square row on the pane ground, edged in the accent
            layer.rect(
                [row[0], row[1]],
                [row[2], row[3]],
                theme.ground(theme.bg),
                0.0,
            );
            layer.rect(
                [row[0], row[1]],
                [2.0, row[3]],
                tab.color.unwrap_or(theme.accent),
                0.0,
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
        let icon = if tab.settings {
            Some((
                Icon::Settings,
                if is_active {
                    theme.text_ui
                } else {
                    theme.text_dim
                },
            ))
        } else if tab.running {
            let color = if is_active {
                theme.text_ui
            } else {
                theme.text_dim
            };
            let c = [cx + 7.0, line1 + 8.5];
            glyphs::draw_spinner(layer, c, 13.0, color);
            cx += 14.0 + 8.0;
            None
        } else if tab.sync {
            Some((Icon::Sync, theme.accent))
        } else {
            None
        };
        if let Some((icon, icon_color)) = icon {
            icons::draw(layer, icon, [cx + 7.0, line1 + 8.5], 14.0, icon_color);
            cx += 14.0 + 8.0;
        }

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
        if let Some(p) = (tab.panes > 1).then(|| format!("{} panes", tab.panes)) {
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

        super::close_icon(
            layer,
            [close_x + 5.0, line1 + 7.0],
            3.5,
            theme
                .text_muted
                .with_alpha(if is_active { 1.0 } else { 0.7 }),
        );
        hits.push(Hit {
            rect: [close_x - 4.0, line1 - 3.0, 18.0, 20.0],
            target: close_target(tab, i),
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
            activity,
            alert,
            color: None,
            sync: false,
            editing: None,
            editing_selected: false,
            subtitle: String::new(),
            settings: false,
            running: false,
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
