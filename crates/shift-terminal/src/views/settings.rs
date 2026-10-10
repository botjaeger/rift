use shift_core::{
    Bell, Color, Config, CursorStyle, InputPosition, Keymap, PromptMode, TabLayout, Theme,
};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::state::{ClickTarget, Hit};
use crate::views::icons::{self, Icon};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    General,
    #[default]
    Appearance,
    Features,
    Notifications,
    Keybindings,
    Themes,
}

impl Page {
    pub const ALL: [Page; 6] = [
        Page::General,
        Page::Appearance,
        Page::Features,
        Page::Notifications,
        Page::Keybindings,
        Page::Themes,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Appearance => "Appearance",
            Page::Features => "Input",
            Page::Notifications => "Notifications",
            Page::Keybindings => "Keybindings",
            Page::Themes => "Themes",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Page::General => "Shell and session behavior.",
            Page::Appearance => "Fonts, cursor, window and theme. Changes apply as you make them.",
            Page::Features => "The command line and blocks.",
            Page::Notifications => {
                "Toasts, desktop notifications and sounds; everything also lands in the inbox."
            }
            Page::Keybindings => "Click a shortcut to record a new one · type to filter.",
            Page::Themes => "Built-in themes plus ~/.config/shift/themes (TOML or Warp YAML).",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Page::General => Icon::Settings,
            Page::Appearance => Icon::Appearance,
            Page::Features => Icon::Terminal,
            Page::Notifications => Icon::Inbox,
            Page::Keybindings => Icon::Keyboard,
            Page::Themes => Icon::Themes,
        }
    }
}

/// An open select's option list (Settings selects open a list; they never
/// cycle on click).
#[derive(Debug, Clone, Default)]
pub struct Dropdown {
    pub id: &'static str,
    /// (config value, shown label)
    pub options: Vec<(String, String)>,
    /// Option under the keyboard (↑ ↓ ⏎) or the pointer.
    pub highlight: usize,
    /// The current value's option.
    pub current: Option<usize>,
}

#[derive(Default)]
pub struct SettingsState {
    /// The Settings tab is shown (it stays in the tab strip while `tab`).
    pub open: bool,
    /// Settings has a tab in the strip.
    pub tab: bool,
    pub page: Page,
    /// Vertical scroll of the page body (logical px, ≥ 0).
    pub scroll: f32,
    /// Height of the last rendered page body (for scroll clamping).
    pub content_h: f32,
    /// Sidebar search: rows of every page whose name or description match.
    pub query: String,
    /// Keybindings page: type-to-filter query over action names/shortcuts.
    pub kb_filter: String,
    /// Keybindings page: action waiting for its new shortcut.
    pub recording: Option<String>,
    /// Page control with keyboard focus (Tab / ⇧Tab); `None` = the arrow
    /// keys move between pages.
    pub focus: Option<ClickTarget>,
    /// The select whose list is open.
    pub dropdown: Option<Dropdown>,
}

impl SettingsState {
    /// Show the Settings tab on `page` (adding the tab when missing).
    pub fn open_page(&mut self, page: Page) {
        if self.page != page {
            self.scroll = 0.0;
            self.focus = None;
        }
        self.page = page;
        self.query.clear();
        self.dropdown = None;
        self.open = true;
        self.tab = true;
    }

    /// Remove the Settings tab.
    pub fn close_tab(&mut self) {
        self.open = false;
        self.tab = false;
        self.focus = None;
        self.dropdown = None;
        self.recording = None;
    }

    /// Keyboard sidebar navigation, clamped at the first/last page.
    pub fn move_page(&mut self, delta: isize) {
        let current = Page::ALL
            .iter()
            .position(|page| *page == self.page)
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, Page::ALL.len() as isize - 1) as usize;
        self.open_page(Page::ALL[next]);
    }

    /// Wheel scroll; `lines` > 0 scrolls up (toward the top).
    pub fn scroll_by(&mut self, lines: f32, viewport_h: f32) {
        let max = (self.content_h - viewport_h).max(0.0);
        self.scroll = (self.scroll - lines * 24.0).clamp(0.0, max);
    }
}

/// The page's keyboard-focusable controls in reading order (top to bottom,
/// left to right): every body control of this frame's `hits`.
pub fn focus_order(hits: &[Hit]) -> Vec<(ClickTarget, [f32; 4])> {
    let mut order: Vec<_> = hits
        .iter()
        .filter(|h| {
            matches!(
                h.target,
                ClickTarget::SettingsToggle(_)
                    | ClickTarget::SettingsStep(..)
                    | ClickTarget::SettingsSegment(..)
                    | ClickTarget::SettingsCycle(_)
                    | ClickTarget::SettingsSlider(_)
                    | ClickTarget::SettingsTheme(_)
                    | ClickTarget::SettingsKeybind(_)
                    | ClickTarget::SettingsKeybindReset(_)
                    | ClickTarget::SettingsReset(_)
                    | ClickTarget::SettingsOpenConfig
            )
        })
        .map(|h| (h.target.clone(), h.rect))
        .collect();
    order.sort_by(|a, b| {
        (a.1[1].round(), a.1[0].round())
            .partial_cmp(&(b.1[1].round(), b.1[0].round()))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    order.dedup_by(|a, b| a.0 == b.0);
    order
}

/// Focus after Tab (`delta` 1) or ⇧Tab (−1), wrapping; from no focus, Tab
/// starts at the first control and ⇧Tab at the last.
pub fn next_focus(
    order: &[(ClickTarget, [f32; 4])],
    current: Option<&ClickTarget>,
    delta: isize,
) -> Option<ClickTarget> {
    if order.is_empty() {
        return None;
    }
    let n = order.len() as isize;
    let i = match current.and_then(|c| order.iter().position(|(t, _)| t == c)) {
        Some(i) => (i as isize + delta).rem_euclid(n),
        None if delta < 0 => n - 1,
        None => 0,
    };
    Some(order[i as usize].0.clone())
}

/// `config` as a TOML table (changed-from-default markers and resets).
pub fn config_table(config: &Config) -> toml::Table {
    toml::Table::try_from(config).unwrap_or_default()
}

/// The value at a dotted setting id (`notifications.toasts`).
fn lookup<'a>(table: &'a toml::Table, id: &str) -> Option<&'a toml::Value> {
    let mut parts = id.split('.');
    let mut v = table.get(parts.next()?)?;
    for p in parts {
        v = v.as_table()?.get(p)?;
    }
    Some(v)
}

/// The setting `id` differs from its default.
pub fn is_changed(current: &toml::Table, defaults: &toml::Table, id: &str) -> bool {
    lookup(current, id) != lookup(defaults, id)
}

/// `config` with setting `id` back at its default (`None` if the id is not
/// a config key or the result does not load).
pub fn reset_setting(config: &Config, id: &str) -> Option<Config> {
    let mut table = config_table(config);
    let defaults = config_table(&Config::default());
    let (path, key) = match id.rsplit_once('.') {
        Some((path, key)) => (Some(path), key),
        None => (None, id),
    };
    let target = match path {
        Some(p) => table.get_mut(p)?.as_table_mut()?,
        None => &mut table,
    };
    match lookup(&defaults, id) {
        Some(v) => {
            target.insert(key.to_string(), v.clone());
        }
        None => {
            target.remove(key);
        }
    }
    toml::Value::Table(table).try_into().ok()
}

const SIDEBAR_W: f32 = 232.0;
/// Icon-only sidebar used when the window is too narrow for labels.
const SIDEBAR_COMPACT_W: f32 = 56.0;
/// Below this width the sidebar collapses to icons.
const COMPACT_BELOW_W: f32 = 700.0;
const ROW_H: f32 = 52.0;
/// Widest the page body grows.
const BODY_MAX_W: f32 = 720.0;
/// Default width reserved for a row's control (toggle, stepper) when its label is elided.
const DEFAULT_RESERVE: f32 = 130.0;
const CONTROL_H: f32 = 28.0;
const MENU_ROW_H: f32 = 28.0;
const MENU_MAX_ROWS: usize = 12;

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

fn mono(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    b.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new()
            .family(Family::Name(mono_family()))
            .weight(weight),
        Shaping::Advanced,
        None,
    );
    b
}

/// Overlay tint that lifts toward the text color (white on dark themes,
/// black on light ones) so controls read on both.
fn lift(theme: &Theme, alpha: f32) -> Color {
    if theme.is_light() {
        Color::BLACK.with_alpha(alpha)
    } else {
        Color::WHITE.with_alpha(alpha)
    }
}

/// The Settings tab's content in `area` (below the tab strip): a sidebar
/// with search and the pages, the page (or the search results) in a body
/// at most 720 px wide, and an open select's list on top. Returns the
/// body's content height (scroll clamping).
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    // the open select's list (a layer above the page)
    over: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &SettingsState,
    config: &Config,
    keymap: &Keymap,
    area: [f32; 4],
    mouse: Option<(f32, f32)>,
    blink_on: bool,
    hits: &mut Vec<Hit>,
    names: &mut Vec<([f32; 4], String)>,
) -> f32 {
    names.clear();
    let [x0, top, aw, ah] = area;
    let (right, bottom) = (x0 + aw, top + ah);
    let compact = aw < COMPACT_BELOW_W;
    let sidebar_w = if compact {
        SIDEBAR_COMPACT_W
    } else {
        SIDEBAR_W
    };
    let hot = |r: [f32; 4]| {
        mouse.is_some_and(|(mx, my)| {
            mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
        })
    };

    // ── sidebar ──
    layer.rect(
        [x0, top],
        [sidebar_w, ah],
        theme.ground(theme.bg_chrome),
        0.0,
    );
    layer.rect([x0 + sidebar_w - 1.0, top], [1.0, ah], theme.border, 0.0);
    layer.rect(
        [x0 + sidebar_w, top],
        [aw - sidebar_w, ah],
        theme.ground(theme.bg),
        0.0,
    );

    let mut ny = top + 12.0;
    if !compact {
        // search field: type anywhere (outside Keybindings) to search
        let r = [x0 + 8.0, ny, sidebar_w - 16.0, 30.0];
        let searching = !state.query.is_empty();
        layer.bordered(
            [r[0], r[1]],
            [r[2], r[3]],
            theme.ground(theme.bg),
            if searching {
                theme.accent
            } else {
                lift(theme, 0.1)
            },
            4.0,
            1.0,
        );
        icons::draw(
            layer,
            Icon::Search,
            [r[0] + 16.0, r[1] + 15.0],
            14.0,
            theme.text_muted,
        );
        let (text, color) = if searching {
            (state.query.as_str(), theme.text_bright)
        } else {
            ("Search settings", theme.text_muted)
        };
        let shown = super::palette::elide_to(engine, text, 12.5, FONT_SANS, r[2] - 40.0);
        layer.text(
            sans(engine, &shown, 12.5, Weight::NORMAL),
            r[0] + 30.0,
            r[1] + 6.5,
            color,
        );
        if searching && blink_on {
            let tw = engine.measure(&shown, 12.5, FONT_SANS, Weight::NORMAL);
            layer.rect(
                [r[0] + 31.0 + tw, r[1] + 7.0],
                [1.5, 16.0],
                theme.accent,
                0.0,
            );
        }
        names.push((r, format!("Search settings: {}", state.query)));
        ny += 30.0 + 10.0;
    }

    for (i, page) in Page::ALL.iter().enumerate() {
        let selected = state.page == *page && state.query.is_empty();
        let row = [x0 + 8.0, ny, sidebar_w - 16.0, 32.0];
        if selected {
            layer.rect([row[0], row[1]], [row[2], row[3]], lift(theme, 0.07), 4.0);
        } else if hot(row) {
            layer.rect([row[0], row[1]], [row[2], row[3]], lift(theme, 0.04), 4.0);
        }
        let color = if selected {
            theme.text_bright
        } else {
            theme.text_dim
        };
        let icon_x = if compact {
            row[0] + row[2] / 2.0
        } else {
            row[0] + 10.0 + 8.0
        };
        icons::draw(layer, page.icon(), [icon_x, ny + 16.0], 16.0, color);
        if !compact {
            layer.text(
                sans(engine, page.label(), 13.0, Weight::NORMAL),
                row[0] + 36.0,
                ny + 7.5,
                color,
            );
        }
        hits.push(Hit {
            rect: row,
            target: ClickTarget::SettingsNav(i as u8),
        });
        names.push((
            row,
            format!(
                "{}{}",
                page.label(),
                if selected { ", current page" } else { "" }
            ),
        ));
        ny += 34.0;
    }

    // sidebar foot: open config.toml in the reading pane
    let foot = [x0 + 8.0, bottom - 40.0, sidebar_w - 16.0, 32.0];
    if foot[1] > ny + 8.0 {
        if hot(foot) {
            layer.rect(
                [foot[0], foot[1]],
                [foot[2], foot[3]],
                lift(theme, 0.04),
                4.0,
            );
        }
        let icon_x = if compact {
            foot[0] + foot[2] / 2.0
        } else {
            foot[0] + 18.0
        };
        icons::draw(
            layer,
            Icon::File,
            [icon_x, foot[1] + 16.0],
            16.0,
            theme.text_muted,
        );
        if !compact {
            layer.text(
                sans(engine, "Open config.toml", 12.5, Weight::NORMAL),
                foot[0] + 36.0,
                foot[1] + 8.0,
                theme.text_muted,
            );
        }
        hits.push(Hit {
            rect: foot,
            target: ClickTarget::SettingsOpenConfig,
        });
        names.push((foot, "Open config.toml".into()));
    }

    // ── body ──
    let pad = if compact { 20.0 } else { 48.0 };
    let cx = x0 + sidebar_w + pad;
    let card_w = (right - pad - cx).clamp(120.0, BODY_MAX_W);
    let searching = !state.query.trim().is_empty();
    let (title, subtitle) = if searching {
        (
            format!("Results for “{}”", state.query.trim()),
            "Settings on every page whose name or description match.",
        )
    } else {
        (state.page.label().to_string(), state.page.subtitle())
    };
    let body_top = top + 28.0;
    // everything from the title down scrolls and clips to the body
    let text_start = layer.texts.len();
    let title = super::palette::elide_to(engine, &title, 22.0, FONT_SANS, card_w);
    layer.text(
        sans(engine, &title, 22.0, Weight::SEMIBOLD),
        cx,
        body_top - state.scroll,
        theme.text_bright,
    );
    let subtitle = super::palette::elide_to(engine, subtitle, 13.0, FONT_SANS, card_w);
    layer.text(
        sans(engine, &subtitle, 13.0, Weight::NORMAL),
        cx,
        body_top + 34.0 - state.scroll,
        theme.text_dim,
    );

    let body_y = body_top + 70.0;
    let hit_start = hits.len();
    let current = config_table(config);
    let defaults = config_table(&Config::default());
    let mut form = Form {
        layer,
        engine,
        theme,
        x: cx,
        y: body_y - state.scroll,
        card_w,
        clip_top: top,
        clip_bottom: bottom - 34.0,
        hits,
        names,
        row_label: String::new(),
        reserve: DEFAULT_RESERVE,
        filter: state.query.trim().to_lowercase(),
        crumb: String::new(),
        page: state.page.label(),
        hidden: false,
        groups: 0,
        label_end: cx,
        label_mid: 0.0,
        current: &current,
        defaults: &defaults,
        mouse,
        rows: 0,
    };
    if searching {
        for page in [
            Page::General,
            Page::Appearance,
            Page::Features,
            Page::Notifications,
        ] {
            form.page = page.label();
            match page {
                Page::General => general_page(&mut form, config),
                Page::Appearance => appearance_page(&mut form, config),
                Page::Features => features_page(&mut form, config),
                _ => notifications_page(&mut form, config),
            }
        }
        if form.rows == 0 && form.visible_band(form.y, ROW_H) {
            form.layer.text(
                sans(form.engine, "No settings match", 13.0, Weight::NORMAL),
                form.x,
                form.y + 16.0,
                form.theme.text_muted,
            );
            form.y += ROW_H;
        }
    } else {
        match state.page {
            Page::Appearance => appearance_page(&mut form, config),
            Page::General => general_page(&mut form, config),
            Page::Features => features_page(&mut form, config),
            Page::Notifications => notifications_page(&mut form, config),
            Page::Keybindings => keybindings_page(&mut form, keymap, config, state),
            Page::Themes => themes_page(&mut form, config),
        }
    }
    let content_h = form.y - (body_top - state.scroll) + 24.0;

    // the select whose list is open, over the page
    let mut menu_hits = Vec::new();
    if let Some(dd) = &state.dropdown
        && let Some(anchor) = form
            .hits
            .iter()
            .find(|h| h.target == ClickTarget::SettingsCycle(dd.id))
            .map(|h| h.rect)
    {
        menu_hits = dropdown(over, form.engine, theme, dd, anchor, area, mouse);
        for h in &menu_hits {
            if let ClickTarget::SettingsOption(i) = h.target
                && let Some((_, label)) = dd.options.get(i)
            {
                form.names.push((h.rect, label.clone()));
            }
        }
    }

    // scrolled content stays inside the body: clip text, drop hits above it
    let layer = form.layer;
    let hits = form.hits;
    let engine = form.engine;
    for t in &mut layer.texts[text_start..] {
        t.bounds = Some(shift_renderer::glyphon::TextBounds {
            left: (x0 + sidebar_w) as i32,
            top: top as i32,
            right: right as i32,
            bottom: (bottom - 34.0) as i32,
        });
    }
    let kept: Vec<Hit> = hits
        .drain(hit_start..)
        .filter(|hh| hh.rect[1] + hh.rect[3] > top && hh.rect[1] < bottom - 34.0)
        .collect();
    hits.extend(kept);
    // the open list wins over everything under it
    for (i, h) in menu_hits.into_iter().enumerate() {
        hits.insert(i, h);
    }

    // footer: where the settings live
    layer.rect(
        [x0 + sidebar_w, bottom - 34.0],
        [aw - sidebar_w, 34.0],
        theme.ground(theme.bg),
        0.0,
    );
    layer.hline(
        x0 + sidebar_w,
        bottom - 34.0,
        aw - sidebar_w,
        1.0,
        theme.border,
    );
    layer.text(
        mono(
            engine,
            "~/.config/shift/config.toml · saved",
            11.5,
            Weight::NORMAL,
        ),
        cx,
        bottom - 24.0,
        theme.text_muted,
    );

    // the Settings tab eats every other click in its area (pushed last)
    hits.push(Hit {
        rect: area,
        target: ClickTarget::SettingsNoop,
    });
    content_h
}

/// The open select's list under (or above) its field: options with a check
/// on the current one; returns the option targets.
fn dropdown(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    dd: &Dropdown,
    anchor: [f32; 4],
    area: [f32; 4],
    mouse: Option<(f32, f32)>,
) -> Vec<Hit> {
    let n = dd.options.len();
    let shown = n.min(MENU_MAX_ROWS);
    let start = dd
        .highlight
        .saturating_sub(shown.saturating_sub(1))
        .min(n.saturating_sub(shown));
    let mw = anchor[2].max(240.0);
    let mh = shown as f32 * MENU_ROW_H + 8.0;
    let mx = (anchor[0] + anchor[2] - mw).max(area[0] + 4.0);
    let below = anchor[1] + anchor[3] + 4.0;
    let my = if below + mh <= area[1] + area[3] - 4.0 {
        below
    } else {
        (anchor[1] - 4.0 - mh).max(area[1] + 4.0)
    };
    layer.shadow(
        [mx, my + 6.0],
        [mw, mh],
        Color::BLACK.with_alpha(0.5),
        4.0,
        14.0,
    );
    layer.bordered(
        [mx, my],
        [mw, mh],
        theme.surface,
        lift(theme, 0.1),
        4.0,
        1.0,
    );
    let mut out = Vec::with_capacity(shown + 1);
    for (row, (value, label)) in dd.options.iter().enumerate().skip(start).take(shown) {
        let ry = my + 4.0 + (row - start) as f32 * MENU_ROW_H;
        let r = [mx + 4.0, ry, mw - 8.0, MENU_ROW_H];
        let over = mouse
            .is_some_and(|(x, y)| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]);
        if row == dd.highlight || over {
            layer.rect(
                [r[0], r[1]],
                [r[2], r[3]],
                theme.accent.with_alpha(0.18),
                3.0,
            );
        }
        let cy = ry + MENU_ROW_H / 2.0;
        if Some(row) == dd.current {
            icons::draw(layer, Icon::Check, [r[0] + 14.0, cy], 14.0, theme.accent);
        }
        let mut tx = r[0] + 28.0;
        // themes: their ground ringed in their accent
        if dd.id.starts_with("theme") {
            let t = shift_core::theme::load_by_name(value);
            layer.bordered([tx, cy - 7.0], [14.0, 14.0], t.bg, t.accent, 3.0, 2.0);
            tx += 22.0;
        }
        let label =
            super::palette::elide_to(engine, label, 13.0, FONT_SANS, r[0] + r[2] - 10.0 - tx);
        layer.text(
            sans(engine, &label, 13.0, Weight::NORMAL),
            tx,
            cy - 8.8,
            if row == dd.highlight {
                theme.text_bright
            } else {
                theme.text_ui
            },
        );
        out.push(Hit {
            rect: r,
            target: ClickTarget::SettingsOption(row),
        });
    }
    if start > 0 || start + shown < n {
        let more = format!("{}–{} of {n}", start + 1, start + shown);
        let w = engine.measure(&more, 11.0, FONT_SANS, Weight::NORMAL);
        layer.text(
            sans(engine, &more, 11.0, Weight::NORMAL),
            mx + mw - 10.0 - w,
            my + 8.0,
            theme.text_muted,
        );
    }
    // the rest of the list swallows clicks
    out.push(Hit {
        rect: [mx, my, mw, mh],
        target: ClickTarget::SettingsNoop,
    });
    out
}

/// Page layout cursor: group labels and flat rows of controls, filtered by
/// the sidebar search.
struct Form<'a> {
    layer: &'a mut Layer,
    engine: &'a mut TextEngine,
    theme: &'a Theme,
    x: f32,
    /// Top of the next row.
    y: f32,
    card_w: f32,
    clip_top: f32,
    clip_bottom: f32,
    hits: &'a mut Vec<Hit>,
    /// Accessible name per pushed hit rect (screen readers).
    names: &'a mut Vec<([f32; 4], String)>,
    /// Label of the row being drawn; names its controls.
    row_label: String,
    /// Width the next row's control claims on the right; its label stops
    /// short of it on narrow bodies (reset to the default after each label).
    reserve: f32,
    /// Lowercased search query; empty = the page as it is.
    filter: String,
    /// `Page › Group` of the rows being drawn (shown under search results).
    crumb: String,
    page: &'static str,
    /// The current row does not match the search: draw and target nothing.
    hidden: bool,
    groups: usize,
    /// Right end of the current row's label (the changed dot follows it).
    label_end: f32,
    /// Vertical middle of the current row's label line.
    label_mid: f32,
    current: &'a toml::Table,
    defaults: &'a toml::Table,
    mouse: Option<(f32, f32)>,
    /// Rows drawn (search: zero means no match).
    rows: usize,
}

impl Form<'_> {
    fn right(&self) -> f32 {
        self.x + self.card_w
    }

    fn visible_band(&self, y: f32, h: f32) -> bool {
        y + h > self.clip_top && y < self.clip_bottom
    }

    fn visible(&self, y: f32, h: f32) -> bool {
        !self.hidden && self.visible_band(y, h)
    }

    fn hot(&self, r: [f32; 4]) -> bool {
        self.mouse.is_some_and(|(mx, my)| {
            mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
        })
    }

    fn group(&mut self, label: &str) {
        self.crumb = format!("{} › {label}", self.page);
        if !self.filter.is_empty() {
            return;
        }
        if self.groups > 0 {
            self.y += 28.0;
        }
        self.groups += 1;
        if self.visible_band(self.y, 20.0) {
            let mut b = Buffer::new(&mut self.engine.font_system, Metrics::new(11.0, 14.0));
            b.set_text(
                &mut self.engine.font_system,
                &label.to_uppercase(),
                &Attrs::new()
                    .family(Family::Name(FONT_SANS))
                    .weight(Weight::SEMIBOLD)
                    .letter_spacing(0.06),
                Shaping::Advanced,
                None,
            );
            self.layer.text(b, self.x, self.y, self.theme.text_muted);
        }
        self.y += 22.0;
        self.layer.hline(
            self.x,
            self.y - 1.0,
            self.card_w,
            1.0,
            lift(self.theme, 0.06),
        );
    }

    fn label(&mut self, text: &str, y: f32) {
        self.label_desc(text, "", y);
    }

    /// Row label, with an optional muted description line under it (the
    /// page › group trail under search results). Advances to the next row;
    /// a row the search does not match is skipped (nothing drawn, no y).
    fn label_desc(&mut self, text: &str, desc: &str, y: f32) {
        self.row_label = text.to_string();
        let reserve = std::mem::replace(&mut self.reserve, DEFAULT_RESERVE);
        let f = &self.filter;
        self.hidden =
            !(f.is_empty() || text.to_lowercase().contains(f) || desc.to_lowercase().contains(f));
        if self.hidden {
            return;
        }
        self.rows += 1;
        self.y = y + ROW_H;
        let desc = if self.filter.is_empty() {
            desc.to_string()
        } else {
            self.crumb.clone()
        };
        if self.visible(y, ROW_H) {
            let ly = if desc.is_empty() { y + 17.0 } else { y + 9.0 };
            // keep room for the control on the right
            let max_w = (self.card_w - reserve - 8.0).max(60.0);
            let shown = super::palette::elide_to(self.engine, text, 13.0, FONT_SANS, max_w);
            self.label_end = self.x + self.engine.measure(&shown, 13.0, FONT_SANS, Weight::NORMAL);
            self.label_mid = ly + 8.8;
            self.layer.text(
                sans(self.engine, &shown, 13.0, Weight::NORMAL),
                self.x,
                ly,
                self.theme.text_ui,
            );
            if !desc.is_empty() {
                let desc = super::palette::elide_to(self.engine, &desc, 12.0, FONT_SANS, max_w);
                self.layer.text(
                    sans(self.engine, &desc, 12.0, Weight::NORMAL),
                    self.x,
                    y + 28.0,
                    self.theme.text_muted,
                );
            }
            self.layer.hline(
                self.x,
                y + ROW_H - 1.0,
                self.card_w,
                1.0,
                lift(self.theme, 0.06),
            );
        }
    }

    fn push_hit(&mut self, rect: [f32; 4], target: ClickTarget) {
        if self.visible(rect[1], rect[3]) {
            self.hits.push(Hit { rect, target });
            self.names.push((rect, self.row_label.clone()));
        }
    }

    /// Accessible name of the control `push_hit` just added (if visible).
    fn name_last(&mut self, name: String) {
        if let (Some(hit), Some(last)) = (self.hits.last(), self.names.last_mut())
            && hit.rect == last.0
        {
            last.1 = name;
        }
    }

    /// A setting changed from its default: an accent dot after the label
    /// and ↺ (reset) left of the control at `control_x`.
    fn changed(&mut self, id: &'static str, y: f32, control_x: f32) {
        if self.hidden || !is_changed(self.current, self.defaults, id) {
            return;
        }
        if self.visible(y, ROW_H) {
            self.layer.circle(
                [self.label_end + 9.0, self.label_mid],
                6.0,
                self.theme.accent,
            );
        }
        let r = [control_x - 32.0, y + (ROW_H - 24.0) / 2.0, 24.0, 24.0];
        if self.visible(r[1], r[3]) {
            if self.hot(r) {
                self.layer
                    .rect([r[0], r[1]], [24.0, 24.0], lift(self.theme, 0.08), 4.0);
            }
            icons::draw(
                self.layer,
                Icon::Revert,
                [r[0] + 12.0, r[1] + 12.0],
                14.0,
                self.theme.text_dim,
            );
        }
        let label = self.row_label.clone();
        self.push_hit(r, ClickTarget::SettingsReset(id));
        self.name_last(format!("Reset {label} to default"));
    }

    fn toggle(&mut self, y: f32, label: &str, on: bool, id: &'static str) {
        self.toggle_desc(y, label, "", on, id);
    }

    fn toggle_desc(&mut self, y: f32, label: &str, desc: &str, on: bool, id: &'static str) {
        self.reserve = 34.0 + 40.0;
        self.label_desc(label, desc, y);
        let ty = y + (ROW_H - 20.0) / 2.0;
        let x = self.right() - 34.0;
        if self.visible(ty, 20.0) {
            let bg = if on {
                self.theme.accent
            } else {
                lift(self.theme, 0.14)
            };
            self.layer.rect([x, ty], [34.0, 20.0], bg, 10.0);
            let knob_x = if on { x + 17.0 } else { x + 3.0 };
            // white on the accent and on light themes; a light grey off on dark
            let knob = if on || self.theme.is_light() {
                Color::WHITE
            } else {
                self.theme.text_ui
            };
            self.layer.circle([knob_x + 7.0, ty + 10.0], 14.0, knob);
        }
        self.changed(id, y, x);
        self.push_hit(
            [x - 4.0, ty - 4.0, 42.0, 28.0],
            ClickTarget::SettingsToggle(id),
        );
    }

    fn stepper(&mut self, y: f32, label: &str, value: &str, unit: &str, id: &'static str) {
        self.stepper_desc(y, label, "", value, unit, id);
    }

    fn stepper_desc(
        &mut self,
        y: f32,
        label: &str,
        desc: &str,
        value: &str,
        unit: &str,
        id: &'static str,
    ) {
        let field_w = self
            .engine
            .measure(&format!("{value} {unit}"), 12.5, FONT_SANS, Weight::NORMAL)
            .max(36.0)
            + 20.0;
        let step_w = 28.0 * 2.0 + field_w;
        self.reserve = step_w + 40.0;
        self.label_desc(label, desc, y);
        let ry = y + (ROW_H - CONTROL_H) / 2.0;
        let sx = self.right() - step_w;
        if self.visible(ry, CONTROL_H) {
            self.layer.bordered(
                [sx, ry],
                [step_w, CONTROL_H],
                Color::TRANSPARENT,
                lift(self.theme, 0.12),
                4.0,
                1.0,
            );
            for bx in [sx + 28.0, sx + step_w - 28.0] {
                self.layer.rect(
                    [bx, ry + 1.0],
                    [1.0, CONTROL_H - 2.0],
                    lift(self.theme, 0.08),
                    0.0,
                );
            }
            for (bx, icon) in [(sx, Icon::Minus), (sx + step_w - 28.0, Icon::Plus)] {
                let r = [bx, ry, 28.0, CONTROL_H];
                if self.hot(r) {
                    self.layer.rect(
                        [r[0] + 1.0, r[1] + 1.0],
                        [26.0, CONTROL_H - 2.0],
                        lift(self.theme, 0.06),
                        3.0,
                    );
                }
                icons::draw(
                    self.layer,
                    icon,
                    [bx + 14.0, ry + 14.0],
                    12.0,
                    self.theme.text_dim,
                );
            }
            let shown = if unit.is_empty() {
                value.to_string()
            } else {
                format!("{value} {unit}")
            };
            let vw = self.engine.measure(&shown, 12.5, FONT_SANS, Weight::NORMAL);
            self.layer.text(
                sans(self.engine, &shown, 12.5, Weight::NORMAL),
                sx + 28.0 + (field_w - vw) / 2.0,
                ry + 6.0,
                self.theme.text_bright,
            );
        }
        self.changed(id, y, sx);
        self.push_hit([sx, ry, 28.0, CONTROL_H], ClickTarget::SettingsStep(id, -1));
        self.name_last(
            format!("Decrease {label}, {value} {unit}")
                .trim()
                .to_string(),
        );
        self.push_hit(
            [sx + step_w - 28.0, ry, 28.0, CONTROL_H],
            ClickTarget::SettingsStep(id, 1),
        );
        self.name_last(
            format!("Increase {label}, {value} {unit}")
                .trim()
                .to_string(),
        );
    }

    fn segment(&mut self, y: f32, label: &str, options: &[&str], active: usize, id: &'static str) {
        let widths: Vec<f32> = options
            .iter()
            .map(|o| self.engine.measure(o, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0)
            .collect();
        let total: f32 = widths.iter().sum();
        self.reserve = total + 40.0;
        self.label(label, y);
        let sy = y + (ROW_H - CONTROL_H) / 2.0;
        let sx = self.right() - total;
        if self.visible(sy, CONTROL_H) {
            self.layer.bordered(
                [sx, sy],
                [total, CONTROL_H],
                Color::TRANSPARENT,
                lift(self.theme, 0.12),
                4.0,
                1.0,
            );
        }
        let mut bx = sx;
        for (i, opt) in options.iter().enumerate() {
            let bw = widths[i];
            if self.visible(sy, CONTROL_H) {
                let r = [bx, sy, bw, CONTROL_H];
                if i == active {
                    self.layer.rect(
                        [bx + 1.0, sy + 1.0],
                        [bw - 2.0, CONTROL_H - 2.0],
                        lift(self.theme, 0.11),
                        3.0,
                    );
                } else if self.hot(r) {
                    self.layer.rect(
                        [bx + 1.0, sy + 1.0],
                        [bw - 2.0, CONTROL_H - 2.0],
                        lift(self.theme, 0.05),
                        3.0,
                    );
                }
                if i > 0 {
                    self.layer.rect(
                        [bx, sy + 1.0],
                        [1.0, CONTROL_H - 2.0],
                        lift(self.theme, 0.08),
                        0.0,
                    );
                }
                let color = if i == active {
                    self.theme.text_bright
                } else {
                    self.theme.text_dim
                };
                self.layer.text(
                    sans(self.engine, opt, 12.5, Weight::MEDIUM),
                    bx + 12.0,
                    sy + 6.0,
                    color,
                );
            }
            self.push_hit(
                [bx, sy, bw, CONTROL_H],
                ClickTarget::SettingsSegment(id, i as u8),
            );
            self.name_last(format!(
                "{label}: {opt}{}",
                if i == active { ", selected" } else { "" }
            ));
            bx += bw;
        }
        self.changed(id, y, sx);
    }

    /// A select: the value and a chevron; a click opens its option list.
    fn select(&mut self, y: f32, label: &str, value: &str, id: &'static str) {
        self.select_with(y, label, value, None, id);
    }

    /// A theme select: the theme's ground ringed in its accent, its name.
    fn theme_select(&mut self, y: f32, label: &str, name: &str, id: &'static str) {
        let t = shift_core::theme::load_by_name(name);
        self.select_with(y, label, &t.name, Some((t.bg, t.accent)), id);
    }

    fn select_with(
        &mut self,
        y: f32,
        label: &str,
        value: &str,
        swatch: Option<(Color, Color)>,
        id: &'static str,
    ) {
        let sel_w = (self.card_w * 0.45).clamp(140.0, 220.0);
        self.reserve = sel_w + 40.0;
        self.label(label, y);
        let ry = y + (ROW_H - CONTROL_H) / 2.0;
        let sx = self.right() - sel_w;
        let r = [sx, ry, sel_w, CONTROL_H];
        if self.visible(ry, CONTROL_H) {
            self.layer.bordered(
                [sx, ry],
                [sel_w, CONTROL_H],
                lift(self.theme, if self.hot(r) { 0.07 } else { 0.04 }),
                lift(self.theme, 0.12),
                4.0,
                1.0,
            );
            let mut tx = sx + 10.0;
            if let Some((bg, accent)) = swatch {
                self.layer
                    .bordered([tx, ry + 7.0], [14.0, 14.0], bg, accent, 3.0, 2.0);
                tx += 22.0;
            }
            let shown = super::palette::elide_to(
                self.engine,
                value,
                13.0,
                FONT_SANS,
                sx + sel_w - 30.0 - tx,
            );
            self.layer.text(
                sans(self.engine, &shown, 13.0, Weight::NORMAL),
                tx,
                ry + 5.2,
                self.theme.text_bright,
            );
            icons::draw(
                self.layer,
                Icon::ChevronDown,
                [sx + sel_w - 16.0, ry + 14.0],
                12.0,
                self.theme.text_dim,
            );
        }
        self.changed(id, y, sx);
        self.push_hit(r, ClickTarget::SettingsCycle(id));
        self.name_last(format!("{label}: {value}"));
    }

    fn slider(&mut self, y: f32, label: &str, t: f32, value: &str, id: &'static str) {
        let slider_w = (self.card_w * 0.3).clamp(90.0, 160.0);
        let value_w = self
            .engine
            .measure(value, 12.5, FONT_SANS, Weight::NORMAL)
            .max(28.0)
            + 16.0;
        self.reserve = slider_w + 12.0 + value_w + 40.0;
        self.label(label, y);
        let cy = y + ROW_H / 2.0;
        let sx = self.right() - value_w - 12.0 - slider_w;
        let t = t.clamp(0.0, 1.0);
        if self.visible(cy - 14.0, 28.0) {
            self.layer
                .rect([sx, cy - 2.0], [slider_w, 4.0], lift(self.theme, 0.14), 2.0);
            self.layer
                .rect([sx, cy - 2.0], [slider_w * t, 4.0], self.theme.accent, 2.0);
            self.layer.circle(
                [sx + slider_w * t, cy + 1.0],
                17.0,
                Color::BLACK.with_alpha(0.35),
            );
            self.layer
                .circle([sx + slider_w * t, cy], 16.0, Color::WHITE);
            let vx = self.right() - value_w;
            self.layer.bordered(
                [vx, cy - 12.0],
                [value_w, 24.0],
                Color::TRANSPARENT,
                lift(self.theme, 0.12),
                4.0,
                1.0,
            );
            let vw = self.engine.measure(value, 12.5, FONT_SANS, Weight::NORMAL);
            self.layer.text(
                sans(self.engine, value, 12.5, Weight::NORMAL),
                vx + (value_w - vw) / 2.0,
                cy - 8.5,
                self.theme.text_bright,
            );
        }
        self.changed(id, y, sx - 8.0);
        self.push_hit(
            [sx - 8.0, cy - 14.0, slider_w + 16.0, 28.0],
            ClickTarget::SettingsSlider(id),
        );
        self.name_last(format!("{label}: {value}"));
    }

    fn display(&mut self, y: f32, label: &str, value: &str) {
        self.reserve = 260.0;
        self.label(label, y);
        if self.visible(y, ROW_H) {
            let shown = super::palette::elide_to(self.engine, value, 12.5, mono_family(), 260.0);
            let vw = self
                .engine
                .measure(&shown, 12.5, mono_family(), Weight::NORMAL);
            self.layer.text(
                mono(self.engine, &shown, 12.5, Weight::NORMAL),
                self.right() - vw,
                y + 17.0,
                self.theme.text_dim,
            );
        }
    }
}

/// `10000` → `10,000` (design 05 scrollback field).
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn appearance_page(form: &mut Form, config: &Config) {
    form.group("Font");
    let y = form.y;
    form.select(y, "Font family", &config.font_family, "font_family");
    let y = form.y;
    form.stepper(
        y,
        "Font size",
        &format!("{:.0}", config.font_size),
        "px",
        "font_size",
    );
    let y = form.y;
    form.slider(
        y,
        "Line height",
        (config.line_height - 1.0) / 1.2,
        &format!("{:.2}", config.line_height),
        "line_height",
    );

    form.group("Cursor");
    let y = form.y;
    let active = match config.cursor_style {
        CursorStyle::Block => 0,
        CursorStyle::Bar => 1,
        CursorStyle::Underline => 2,
    };
    form.segment(
        y,
        "Cursor style",
        &["Block", "Bar", "Underline"],
        active,
        "cursor_style",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Cursor blink",
        "Pulse the cursor when the editor is focused",
        config.cursor_blink,
        "cursor_blink",
    );

    form.group("Window & theme");
    // blur exists on macOS only (Warp: not supported on Linux)
    let blur = cfg!(target_os = "macos");
    let y = form.y;
    form.theme_select(y, "Active theme", &config.theme, "theme");
    let y = form.y;
    form.toggle(
        y,
        "Sync with OS appearance",
        config.sync_os_theme,
        "sync_os_theme",
    );
    let y = form.y;
    form.theme_select(
        y,
        "Light theme (OS sync)",
        &config.theme_light,
        "theme_light",
    );
    let y = form.y;
    form.slider(
        y,
        "Window opacity",
        opacity_slider(config.opacity()),
        &format!("{:.0}%", config.opacity() * 100.0),
        "window_opacity",
    );
    if blur {
        let y = form.y;
        form.slider(
            y,
            "Window blur radius",
            blur_slider(config.blur()),
            &format!("{} px", config.blur()),
            "window_blur",
        );
    }
    let y = form.y;
    form.stepper(
        y,
        "Window padding",
        &format!("{:.0}", config.padding()),
        "px",
        "window_padding",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Font ligatures",
        "Render -> >= != as combined glyphs",
        config.ligatures,
        "ligatures",
    );
    let y = form.y;
    form.segment(
        y,
        "Tabs",
        &["Top", "Side"],
        match config.tab_layout {
            TabLayout::Horizontal => 0,
            TabLayout::Vertical => 1,
        },
        "tab_layout",
    );
}

fn general_page(form: &mut Form, config: &Config) {
    form.group("Shell");
    let shell = config
        .shell
        .clone()
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/zsh".into());
    let y = form.y;
    form.display(y, "Shell path", &shell);
    let y = form.y;
    form.stepper_desc(
        y,
        "Scrollback",
        "Maximum lines kept in the buffer",
        &thousands(config.scrollback),
        "lines",
        "scrollback",
    );

    form.group("Session");
    let y = form.y;
    form.toggle(
        y,
        "Restore tabs on launch",
        config.restore_session,
        "restore_session",
    );
    let y = form.y;
    let wd = match config.working_directory.trim() {
        "" | "previous" => 0,
        "home" => 1,
        _ => 2,
    };
    form.segment(
        y,
        "New tabs start in",
        &["Previous", "Home", "Custom"],
        wd,
        "working_directory",
    );
    let y = form.y;
    form.toggle(
        y,
        "Warn before closing running commands",
        config.confirm_close_running,
        "confirm_close_running",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Save block output for restore",
        "Output can contain secrets; turning this off deletes it",
        config.save_transcripts,
        "save_transcripts",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Keep shells running when Shift quits",
        "Session service; relaunch reattaches (new shells)",
        config.durable_sessions,
        "durable_sessions",
    );

    form.group("Automation");
    let y = form.y;
    form.segment(
        y,
        "Control socket",
        &["Off", "Navigate", "Full"],
        match config.control_api.as_str() {
            "off" => 0,
            "full" => 2,
            _ => 1,
        },
        "control_api",
    );

    form.group("Terminal");
    let y = form.y;
    form.toggle(
        y,
        "Mouse reporting to programs (Shift selects)",
        config.mouse_reporting,
        "mouse_reporting",
    );
    let y = form.y;
    form.toggle(
        y,
        "Ask before pasting lines that would run",
        config.confirm_multiline_paste,
        "confirm_multiline_paste",
    );
    if cfg!(target_os = "macos") {
        let y = form.y;
        form.toggle(
            y,
            "Option key sends Meta",
            config.option_as_meta,
            "option_as_meta",
        );
    }
}

fn features_page(form: &mut Form, config: &Config) {
    form.group("Input");
    let y = form.y;
    form.toggle(y, "Vim keybindings", config.vim_mode, "vim_mode");
    let y = form.y;
    form.toggle(
        y,
        "Autosuggestions (ghost text)",
        config.autosuggestions,
        "autosuggestions",
    );
    let y = form.y;
    form.toggle(
        y,
        "Completions menu",
        config.completions_menu,
        "completions_menu",
    );
    let y = form.y;
    form.toggle(
        y,
        "Autocomplete quotes and brackets",
        config.autocomplete_pairs,
        "autocomplete_pairs",
    );
    let y = form.y;
    form.segment(
        y,
        "Input position",
        &["Bottom", "Top"],
        match config.input_position {
            InputPosition::Bottom => 0,
            InputPosition::Top => 1,
        },
        "input_position",
    );
    let y = form.y;
    form.segment(
        y,
        "Prompt",
        &["Shift", "Shell (PS1)"],
        match config.prompt {
            PromptMode::Shift => 0,
            PromptMode::Shell => 1,
        },
        "prompt",
    );
    let y = form.y;
    form.toggle(
        y,
        "Expand aliases as you type",
        config.alias_expansion,
        "alias_expansion",
    );
    let y = form.y;
    form.toggle(
        y,
        "Underline unknown commands",
        config.error_underline,
        "error_underline",
    );
    let y = form.y;
    form.toggle(
        y,
        "Suggest corrections for failed commands",
        config.command_corrections,
        "command_corrections",
    );

    form.group("Blocks");
    let y = form.y;
    form.toggle(y, "Copy on select", config.copy_on_select, "copy_on_select");
}

/// Shown value of a sound setting (`""` = follow the main sound).
pub fn sound_label(value: &str) -> String {
    match value.trim() {
        "" => "Same as Sound".into(),
        v if v.eq_ignore_ascii_case("default") => "Default".into(),
        v if v.eq_ignore_ascii_case("none") => "None".into(),
        v => v.rsplit('/').next().unwrap_or(v).to_string(),
    }
}

fn notifications_page(form: &mut Form, config: &Config) {
    let n = &config.notifications;
    form.group("Alerts");
    let y = form.y;
    form.toggle_desc(
        y,
        "Notifications",
        "Toasts while Shift is focused, desktop notifications while it is not",
        n.enabled,
        "notifications.enabled",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "In-window toasts",
        "Top-right cards for panes you are not looking at",
        n.toasts,
        "notifications.toasts",
    );
    let y = form.y;
    form.stepper(
        y,
        "Toast duration",
        &format!("{}", n.toast_duration().as_secs()),
        "s",
        "notifications.toast_secs",
    );

    form.group("Notify me when");
    let y = form.y;
    form.toggle(
        y,
        "A long command finishes",
        config.notify_on_completion,
        "notify_on_completion",
    );
    let y = form.y;
    form.stepper(
        y,
        "Long command threshold",
        &format!("{}", config.notify_threshold_secs),
        "s",
        "notify_threshold_secs",
    );
    let y = form.y;
    form.toggle(
        y,
        "A command fails",
        n.command_failed,
        "notifications.command_failed",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "An agent finishes its task",
        "Claude Code, Codex, OpenCode, Gemini CLI",
        n.agent_task_completed,
        "notifications.agent_task_completed",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "A program or agent needs attention",
        "OSC 9 / 777 / 99, approval and input requests",
        n.needs_attention,
        "notifications.needs_attention",
    );
    let y = form.y;
    form.toggle(
        y,
        "A password prompt appears",
        n.password_prompt,
        "notifications.password_prompt",
    );

    form.group("Sounds");
    let y = form.y;
    form.toggle(
        y,
        "Play notification sounds",
        n.play_sound,
        "notifications.play_sound",
    );
    let y = form.y;
    form.select(y, "Sound", &sound_label(&n.sound), "notifications.sound");
    let y = form.y;
    form.select(
        y,
        "Finished",
        &sound_label(&n.sound_finished),
        "notifications.sound_finished",
    );
    let y = form.y;
    form.select(
        y,
        "Failed",
        &sound_label(&n.sound_failed),
        "notifications.sound_failed",
    );
    let y = form.y;
    form.select(
        y,
        "Needs attention",
        &sound_label(&n.sound_attention),
        "notifications.sound_attention",
    );

    form.group("Terminal bell");
    let y = form.y;
    form.segment(
        y,
        "Bell",
        &["Visual", "Audible", "Both", "None"],
        match config.bell {
            Bell::Visual => 0,
            Bell::Audible => 1,
            Bell::Both => 2,
            Bell::None => 3,
        },
        "bell",
    );
}

/// Keybindings rows that match the type-to-filter query (action name with
/// spaces for underscores, or the shortcut chips).
pub fn filtered_bindings(keymap: &Keymap, filter: &str) -> Vec<(String, Vec<String>)> {
    let q = filter.trim().to_lowercase();
    keymap
        .display_rows()
        .into_iter()
        .filter(|(action, chips)| {
            q.is_empty()
                || action.replace('_', " ").contains(&q)
                || action.contains(&q)
                || chips.concat().to_lowercase().contains(&q)
        })
        .collect()
}

fn keybindings_page(form: &mut Form, keymap: &Keymap, config: &Config, state: &SettingsState) {
    // filter field
    let fy = form.y;
    if form.visible_band(fy, 32.0) {
        form.layer.bordered(
            [form.x, fy],
            [form.card_w, 32.0],
            form.theme.ground(form.theme.bg),
            if state.kb_filter.is_empty() {
                lift(form.theme, 0.12)
            } else {
                form.theme.accent
            },
            4.0,
            1.0,
        );
        icons::draw(
            form.layer,
            Icon::Filter,
            [form.x + 16.0, fy + 16.0],
            14.0,
            form.theme.text_muted,
        );
        let (text, color) = if state.kb_filter.is_empty() {
            ("Type to filter actions or shortcuts", form.theme.text_muted)
        } else {
            (state.kb_filter.as_str(), form.theme.text_bright)
        };
        form.layer.text(
            sans(form.engine, text, 13.0, Weight::NORMAL),
            form.x + 32.0,
            fy + 7.0,
            color,
        );
    }
    form.y += 32.0 + 8.0;

    form.group("Shortcuts");
    let rows = filtered_bindings(keymap, &state.kb_filter);
    let row_h = 40.0;
    let fy = form.y;
    if rows.is_empty() && form.visible_band(fy, row_h) {
        form.layer.text(
            sans(form.engine, "No matching actions", 13.0, Weight::NORMAL),
            form.x,
            fy + 12.0,
            form.theme.text_muted,
        );
    }
    for (i, (action, chips)) in rows.iter().enumerate() {
        let ry = fy + i as f32 * row_h;
        if !form.visible_band(ry, row_h) {
            continue;
        }
        let recording = state.recording.as_deref() == Some(action.as_str());
        let custom = config.keybindings.contains_key(action);
        form.row_label = format!(
            "{action}: {}",
            if chips.is_empty() {
                "unbound".to_string()
            } else {
                chips.join(" ")
            }
        );
        let r = [form.x, ry, form.card_w, row_h];
        if recording {
            form.layer.rect(
                [r[0], r[1]],
                [r[2], r[3] - 1.0],
                form.theme.accent.with_alpha(0.12),
                0.0,
            );
        } else if form.hot(r) {
            form.layer.rect(
                [r[0], r[1]],
                [r[2], r[3] - 1.0],
                lift(form.theme, 0.03),
                0.0,
            );
        }
        // the action name stops before the shortcut caps / recording hint
        let hint = "Press a shortcut · Esc cancels · ⌫ unbinds";
        let caps_w: f32 = chips
            .iter()
            .map(|k| {
                (form.engine.measure(k, 10.5, FONT_SANS, Weight::MEDIUM) + 10.0).max(18.0) + 4.0
            })
            .sum();
        let reserve = if recording {
            form.engine.measure(hint, 12.0, FONT_SANS, Weight::NORMAL)
        } else if chips.is_empty() {
            60.0
        } else {
            caps_w + if custom { 32.0 } else { 0.0 }
        };
        let name_w = (form.card_w - reserve - 24.0).max(40.0);
        let action_shown =
            super::palette::elide_to(form.engine, action, 12.5, mono_family(), name_w);
        form.layer.text(
            mono(form.engine, &action_shown, 12.5, Weight::NORMAL),
            form.x + 8.0,
            ry + 11.5,
            form.theme.text_ui,
        );
        let mut cx = form.right() - 8.0;
        // reset to default (custom rows only)
        if custom && !recording {
            let rr = [cx - 24.0, ry + 8.0, 24.0, 24.0];
            if form.hot(rr) {
                form.layer
                    .rect([rr[0], rr[1]], [24.0, 24.0], lift(form.theme, 0.08), 4.0);
            }
            icons::draw(
                form.layer,
                Icon::Revert,
                [rr[0] + 12.0, rr[1] + 12.0],
                14.0,
                form.theme.text_dim,
            );
            form.push_hit(rr, ClickTarget::SettingsKeybindReset(action.clone()));
            form.name_last(format!("Reset {action} to default"));
            cx -= 32.0;
        }
        if recording {
            let tw = form.engine.measure(hint, 12.0, FONT_SANS, Weight::NORMAL);
            form.layer.text(
                sans(form.engine, hint, 12.0, Weight::NORMAL),
                cx - tw,
                ry + 12.0,
                form.theme.accent,
            );
        } else if chips.is_empty() {
            let tw = form
                .engine
                .measure("unbound", 12.0, FONT_SANS, Weight::NORMAL);
            form.layer.text(
                sans(form.engine, "unbound", 12.0, Weight::NORMAL),
                cx - tw,
                ry + 12.0,
                form.theme.text_muted,
            );
        } else {
            let mut kx = cx - caps_w + 4.0;
            for key in chips {
                let w = super::completion::key_cap(
                    form.layer,
                    form.engine,
                    form.theme,
                    key,
                    kx,
                    ry + row_h / 2.0,
                );
                if custom {
                    // overridden: the caps carry an accent edge
                    form.layer.bordered(
                        [kx, ry + row_h / 2.0 - 9.0],
                        [w, 18.0],
                        Color::TRANSPARENT,
                        form.theme.accent.with_alpha(0.6),
                        3.0,
                        1.0,
                    );
                }
                kx += w + 4.0;
            }
        }
        // the rest of the row starts recording (reset button wins: pushed first)
        form.push_hit(r, ClickTarget::SettingsKeybind(action.clone()));
        form.layer.hline(
            form.x,
            ry + row_h - 1.0,
            form.card_w,
            1.0,
            lift(form.theme, 0.06),
        );
    }
    form.y = fy + rows.len().max(1) as f32 * row_h + 16.0;
}

fn themes_page(form: &mut Form, config: &Config) {
    form.group("Available themes");
    let names = shift_core::theme::available_themes();
    for (i, name) in names.iter().enumerate() {
        let ry = form.y;
        form.y += ROW_H;
        if !form.visible_band(ry, ROW_H) {
            continue;
        }
        let t = shift_core::theme::load_by_name(name);
        let r = [form.x, ry, form.card_w, ROW_H];
        if form.hot(r) {
            form.layer.rect(
                [r[0], r[1]],
                [r[2], r[3] - 1.0],
                lift(form.theme, 0.03),
                0.0,
            );
        }
        // swatches
        let mut sx = form.x + 8.0;
        for c in [t.bg, t.accent, t.success, t.error] {
            form.layer.bordered(
                [sx, ry + 18.0],
                [16.0, 16.0],
                c,
                lift(form.theme, 0.2),
                3.0,
                1.0,
            );
            sx += 22.0;
        }
        // the name stops before the "active" chip
        let name_w = (form.x + form.card_w - 100.0 - (sx + 12.0)).max(30.0);
        let shown = super::palette::elide_to(form.engine, &t.name, 13.0, FONT_SANS, name_w);
        form.layer.text(
            sans(form.engine, &shown, 13.0, Weight::MEDIUM),
            sx + 10.0,
            ry + 17.0,
            form.theme.text_bright,
        );
        let tag = if config.theme == *name {
            Some("active")
        } else if config.sync_os_theme && config.theme_light == *name {
            Some("light")
        } else {
            None
        };
        form.row_label = match tag {
            Some(tag) => format!("{}, {tag}", t.name),
            None => t.name.clone(),
        };
        if let Some(tag) = tag {
            let aw = form.engine.measure(tag, 11.5, FONT_SANS, Weight::MEDIUM);
            form.layer.rect(
                [form.right() - aw - 18.0 - 8.0, ry + 15.0],
                [aw + 18.0, 22.0],
                form.theme.accent.with_alpha(0.16),
                4.0,
            );
            form.layer.text(
                sans(form.engine, tag, 11.5, Weight::MEDIUM),
                form.right() - aw - 9.0 - 8.0,
                ry + 18.5,
                form.theme.accent,
            );
        }
        form.layer.hline(
            form.x,
            ry + ROW_H - 1.0,
            form.card_w,
            1.0,
            lift(form.theme, 0.06),
        );
        form.push_hit(r, ClickTarget::SettingsTheme(i as u8));
    }
}

/// `window_opacity` (0.01–1.0) as a slider position.
pub fn opacity_slider(opacity: f32) -> f32 {
    (opacity - 0.01) / 0.99
}

/// `window_blur` (1–64 px) as a slider position.
pub fn blur_slider(blur: u8) -> f32 {
    (f32::from(blur) - 1.0) / 63.0
}

#[cfg(test)]
mod tests {
    use super::{
        Page, SettingsState, config_table, focus_order, is_changed, next_focus, reset_setting,
        thousands,
    };
    use crate::state::{ClickTarget, Hit};

    #[test]
    fn tab_walks_controls_in_reading_order() {
        let hit = |target, x, y| Hit {
            rect: [x, y, 40.0, 20.0],
            target,
        };
        let hits = vec![
            hit(ClickTarget::SettingsNav(0), 0.0, 0.0),
            hit(ClickTarget::SettingsToggle("b"), 300.0, 200.0),
            hit(ClickTarget::SettingsSegment("seg", 1), 360.0, 100.0),
            hit(ClickTarget::SettingsSegment("seg", 0), 300.0, 100.0),
            hit(ClickTarget::SettingsClose, 900.0, 0.0),
        ];
        let order = focus_order(&hits);
        let targets: Vec<_> = order.iter().map(|(t, _)| t.clone()).collect();
        assert_eq!(
            targets,
            [
                ClickTarget::SettingsSegment("seg", 0),
                ClickTarget::SettingsSegment("seg", 1),
                ClickTarget::SettingsToggle("b"),
            ]
        );
        let first = next_focus(&order, None, 1);
        assert_eq!(first, Some(ClickTarget::SettingsSegment("seg", 0)));
        assert_eq!(
            next_focus(&order, None, -1),
            Some(ClickTarget::SettingsToggle("b"))
        );
        let wrapped = next_focus(&order, Some(&ClickTarget::SettingsToggle("b")), 1);
        assert_eq!(wrapped, first);
        assert_eq!(next_focus(&[], None, 1), None);
        // changing page drops the focus
        let mut state = SettingsState {
            focus: first,
            ..Default::default()
        };
        state.move_page(1);
        assert_eq!(state.focus, None);
    }

    #[test]
    fn changed_settings_reset_to_their_defaults() {
        let mut c = shift_core::Config::default();
        let d = config_table(&c);
        c.font_size = 18.0;
        c.notifications.toasts = !c.notifications.toasts;
        let t = config_table(&c);
        assert!(is_changed(&t, &d, "font_size"));
        assert!(is_changed(&t, &d, "notifications.toasts"));
        assert!(!is_changed(&t, &d, "theme"));
        let c = reset_setting(&c, "font_size").expect("loads");
        assert_eq!(c.font_size, shift_core::Config::default().font_size);
        let c = reset_setting(&c, "notifications.toasts").expect("loads");
        assert!(!is_changed(&config_table(&c), &d, "notifications.toasts"));
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(10000), "10,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn keyboard_page_navigation_clamps_and_resets_scroll() {
        let mut state = SettingsState {
            page: Page::Appearance,
            scroll: 120.0,
            ..Default::default()
        };
        state.move_page(1);
        assert_eq!(state.page, Page::Features);
        assert_eq!(state.scroll, 0.0);

        state.move_page(99);
        assert_eq!(state.page, Page::Themes);
        state.move_page(-99);
        assert_eq!(state.page, Page::General);
    }
}
