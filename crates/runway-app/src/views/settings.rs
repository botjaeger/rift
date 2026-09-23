use runway_core::{Bell, Color, Config, CursorStyle, InputPosition, Keymap, PromptMode, Theme};
use runway_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use runway_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::state::{ClickTarget, Hit};
use crate::views::tabbar::TAB_BAR_H;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    General,
    #[default]
    Appearance,
    Features,
    Keybindings,
    Themes,
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::General,
        Page::Appearance,
        Page::Features,
        Page::Keybindings,
        Page::Themes,
    ];
}

#[derive(Default)]
pub struct SettingsState {
    pub open: bool,
    pub page: Page,
    /// Vertical scroll of the page body (logical px, ≥ 0).
    pub scroll: f32,
    /// Height of the last rendered page body (for scroll clamping).
    pub content_h: f32,
    /// Keybindings page: type-to-filter query over action names/shortcuts.
    pub kb_filter: String,
    /// Keybindings page: action waiting for its new shortcut.
    pub recording: Option<String>,
}

impl SettingsState {
    pub fn open_page(&mut self, page: Page) {
        if self.page != page {
            self.scroll = 0.0;
        }
        self.page = page;
        self.open = true;
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

const SIDEBAR_W: f32 = 248.0;
const ROW_H: f32 = 52.0;

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
/// black on light ones) so cards and chips read on both.
fn lift(theme: &Theme, alpha: f32) -> Color {
    if theme.is_light() {
        Color::BLACK.with_alpha(alpha)
    } else {
        Color::WHITE.with_alpha(alpha)
    }
}

/// Full-window settings overlay (design 05).
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &SettingsState,
    config: &Config,
    keymap: &Keymap,
    w: f32,
    h: f32,
    hits: &mut Vec<Hit>,
) -> f32 {
    let top = TAB_BAR_H;

    // ── sidebar ──
    layer.rect([0.0, top], [SIDEBAR_W, h - top], theme.bg_chrome, 0.0);
    layer.rect([SIDEBAR_W - 1.0, top], [1.0, h - top], theme.border, 0.0);
    layer.rect([SIDEBAR_W, top], [w - SIDEBAR_W, h - top], theme.bg, 0.0);

    // header: ❯ logo + "Settings"
    let hx = 22.0;
    let hy = top + 22.0;
    layer.rect([hx, hy], [26.0, 26.0], theme.accent, 7.0);
    layer.text(
        mono(engine, "❯", 13.0, Weight::BOLD),
        hx + 7.5,
        hy + 4.5,
        Color::WHITE,
    );
    layer.text(
        sans(engine, "Settings", 16.0, Weight::SEMIBOLD),
        hx + 36.0,
        hy + 3.0,
        theme.text_bright,
    );

    // nav
    let labels = ["General", "Appearance", "Features", "Keybindings", "Themes"];
    let mut ny = hy + 46.0;
    for (i, page) in Page::ALL.iter().enumerate() {
        let selected = state.page == *page;
        let row = [14.0, ny, SIDEBAR_W - 28.0, 38.0];
        if selected {
            layer.rect(
                [row[0], row[1]],
                [row[2], row[3]],
                theme.accent.with_alpha(0.15),
                8.0,
            );
            layer.rect([row[0], row[1]], [2.0, row[3]], theme.accent, 1.0);
        }
        // icon: simple glyph box
        let icon_color = if selected {
            theme.accent
        } else {
            theme.text_dim
        };
        layer.bordered(
            [row[0] + 11.0, ny + 11.0],
            [16.0, 16.0],
            Color::TRANSPARENT,
            icon_color,
            if matches!(page, Page::Themes | Page::General) {
                8.0
            } else {
                4.0
            },
            1.6,
        );
        let color = if selected {
            theme.text_bright
        } else {
            theme.text_dim
        };
        layer.text(
            sans(
                engine,
                labels[i],
                13.5,
                if selected {
                    Weight::MEDIUM
                } else {
                    Weight::NORMAL
                },
            ),
            row[0] + 38.0,
            ny + 10.0,
            color,
        );
        hits.push(Hit {
            rect: row,
            target: ClickTarget::SettingsNav(i as u8),
        });
        ny += 41.0;
    }

    // sidebar footer: config path
    let footer = "~/.config/runway/config.toml";
    layer.hline(14.0, h - 44.0, SIDEBAR_W - 28.0, 1.0, theme.border);
    layer.text(
        mono(engine, footer, 11.0, Weight::NORMAL),
        22.0,
        h - 32.0,
        theme.text_muted,
    );

    // ── content header ──
    let cx = SIDEBAR_W + 32.0;
    let (title, subtitle) = match state.page {
        Page::General => ("General", "Shell and session behavior"),
        Page::Appearance => ("Appearance", "Fonts, cursor, window and theme"),
        Page::Features => ("Features", "Input, blocks, notifications"),
        Page::Keybindings => (
            "Keybindings",
            "Click a shortcut to record a new one · type to filter",
        ),
        Page::Themes => (
            "Themes",
            "Built-in themes plus ~/.config/runway/themes (TOML or Warp YAML)",
        ),
    };
    layer.text(
        sans(engine, title, 19.0, Weight::SEMIBOLD),
        cx,
        top + 24.0,
        theme.text_bright,
    );
    layer.text(
        sans(engine, subtitle, 12.5, Weight::NORMAL),
        cx,
        top + 52.0,
        theme.text_dim,
    );

    // close ✕
    let close = [w - 46.0, top + 24.0, 30.0, 30.0];
    layer.rect(
        [close[0], close[1]],
        [close[2], close[3]],
        lift(theme, 0.05),
        8.0,
    );
    layer.text(
        sans(engine, "✕", 15.0, Weight::NORMAL),
        close[0] + 8.5,
        close[1] + 5.0,
        theme.text_dim,
    );
    hits.push(Hit {
        rect: close,
        target: ClickTarget::SettingsClose,
    });
    layer.hline(
        SIDEBAR_W,
        top + 76.0,
        w - SIDEBAR_W,
        1.0,
        theme.border.with_alpha(0.05),
    );

    let body_y = top + 100.0;
    let text_start = layer.texts.len();
    let hit_start = hits.len();
    let mut form = Form {
        layer,
        engine,
        theme,
        x: cx,
        y: body_y - state.scroll,
        card_w: w - cx - 32.0,
        clip_top: body_y - 12.0,
        clip_bottom: h,
        hits,
    };
    match state.page {
        Page::Appearance => appearance_page(&mut form, config),
        Page::General => general_page(&mut form, config),
        Page::Features => features_page(&mut form, config),
        Page::Keybindings => keybindings_page(&mut form, keymap, config, state),
        Page::Themes => themes_page(&mut form, config),
    }
    let content_h = form.y - (body_y - state.scroll) + 24.0;
    // scrolled content stays inside the body: clip text, drop hits above it
    for t in &mut layer.texts[text_start..] {
        t.bounds = Some(runway_renderer::glyphon::TextBounds {
            left: SIDEBAR_W as i32,
            top: (body_y - 12.0) as i32,
            right: w as i32,
            bottom: h as i32,
        });
    }
    let kept: Vec<Hit> = hits
        .drain(hit_start..)
        .filter(|hh| hh.rect[1] + hh.rect[3] > body_y - 12.0)
        .collect();
    hits.extend(kept);
    // full overlay eats every other click (lowest priority: pushed last)
    hits.push(Hit {
        rect: [0.0, top, w, h - top],
        target: ClickTarget::SettingsNoop,
    });
    content_h
}

/// Page layout cursor: group labels, cards and rows of controls.
struct Form<'a> {
    layer: &'a mut Layer,
    engine: &'a mut TextEngine,
    theme: &'a Theme,
    x: f32,
    y: f32,
    card_w: f32,
    clip_top: f32,
    clip_bottom: f32,
    hits: &'a mut Vec<Hit>,
}

impl Form<'_> {
    fn right(&self) -> f32 {
        self.x + self.card_w - 18.0
    }

    fn visible(&self, y: f32, h: f32) -> bool {
        y + h > self.clip_top && y < self.clip_bottom
    }

    fn group(&mut self, label: &str) {
        if self.visible(self.y, 20.0) {
            self.layer.text(
                sans(self.engine, &label.to_uppercase(), 11.0, Weight::SEMIBOLD),
                self.x,
                self.y,
                self.theme.text_muted,
            );
        }
        self.y += 22.0;
    }

    /// Card background for `rows` rows; returns the first row's y.
    fn card(&mut self, rows: usize) -> f32 {
        let h = rows as f32 * ROW_H + 4.0;
        // clamp to the clip so a scrolled card never paints over the header
        let top = self.y.max(self.clip_top);
        let bottom = (self.y + h).min(self.clip_bottom);
        if bottom > top {
            self.layer.bordered(
                [self.x, top],
                [self.card_w, bottom - top],
                lift(self.theme, 0.025),
                lift(self.theme, 0.05),
                11.0,
                1.0,
            );
        }
        let first = self.y + 2.0;
        self.y += h + 26.0;
        first
    }

    fn divider(&mut self, y: f32) {
        if self.visible(y, 1.0) {
            self.layer.hline(
                self.x + 18.0,
                y,
                self.card_w - 36.0,
                1.0,
                lift(self.theme, 0.05),
            );
        }
    }

    fn label(&mut self, text: &str, y: f32) {
        if self.visible(y, ROW_H) {
            self.layer.text(
                sans(self.engine, text, 13.5, Weight::NORMAL),
                self.x + 18.0,
                y + 17.0,
                self.theme.text_ui,
            );
        }
    }

    fn push_hit(&mut self, rect: [f32; 4], target: ClickTarget) {
        if self.visible(rect[1], rect[3]) {
            self.hits.push(Hit { rect, target });
        }
    }

    fn toggle(&mut self, y: f32, label: &str, on: bool, id: &'static str) {
        self.label(label, y);
        let ty = y + (ROW_H - 23.0) / 2.0;
        let x = self.right() - 40.0;
        if self.visible(ty, 23.0) {
            let bg = if on {
                self.theme.accent
            } else {
                lift(self.theme, 0.15)
            };
            self.layer.rect([x, ty], [40.0, 23.0], bg, 12.0);
            let knob_x = if on { x + 40.0 - 20.5 } else { x + 2.5 };
            self.layer
                .circle([knob_x + 9.0, ty + 11.5], 18.0, Color::WHITE);
        }
        self.push_hit(
            [x - 4.0, ty - 4.0, 48.0, 31.0],
            ClickTarget::SettingsToggle(id),
        );
    }

    fn stepper(&mut self, y: f32, label: &str, value: &str, unit: &str, id: &'static str) {
        self.label(label, y);
        let ry = y + (ROW_H - 36.0) / 2.0;
        let right = self.right();
        let unit_w = if unit.is_empty() {
            0.0
        } else {
            self.engine.measure(unit, 11.5, FONT_SANS, Weight::NORMAL) + 14.0
        };
        let step_w = 122.0;
        let sx = right - unit_w - step_w;
        if self.visible(ry, 36.0) {
            self.layer.bordered(
                [sx, ry],
                [step_w, 36.0],
                Color::TRANSPARENT,
                lift(self.theme, 0.08),
                8.0,
                1.0,
            );
            self.layer
                .rect([sx, ry], [34.0, 36.0], lift(self.theme, 0.04), 8.0);
            self.layer.rect(
                [sx + step_w - 34.0, ry],
                [34.0, 36.0],
                lift(self.theme, 0.04),
                8.0,
            );
            self.layer.text(
                sans(self.engine, "−", 16.0, Weight::NORMAL),
                sx + 12.0,
                ry + 6.0,
                self.theme.text_dim,
            );
            self.layer.text(
                sans(self.engine, "+", 16.0, Weight::NORMAL),
                sx + step_w - 24.0,
                ry + 6.0,
                self.theme.text_dim,
            );
            let vw = self
                .engine
                .measure(value, 13.0, mono_family(), Weight::NORMAL);
            self.layer.text(
                mono(self.engine, value, 13.0, Weight::NORMAL),
                sx + 34.0 + (step_w - 68.0 - vw) / 2.0,
                ry + 9.0,
                self.theme.text_bright,
            );
            if !unit.is_empty() {
                self.layer.text(
                    sans(self.engine, unit, 11.5, Weight::NORMAL),
                    right - unit_w + 10.0,
                    ry + 10.0,
                    self.theme.text_muted,
                );
            }
        }
        self.push_hit([sx, ry, 34.0, 36.0], ClickTarget::SettingsStep(id, -1));
        self.push_hit(
            [sx + step_w - 34.0, ry, 34.0, 36.0],
            ClickTarget::SettingsStep(id, 1),
        );
    }

    fn segment(&mut self, y: f32, label: &str, options: &[&str], active: usize, id: &'static str) {
        self.label(label, y);
        let sy = y + (ROW_H - 34.0) / 2.0;
        let seg_w = if options.len() > 3 { 70.0 } else { 82.0 };
        let total = seg_w * options.len() as f32 + 3.0 * (options.len() as f32 + 1.0);
        let sx = self.right() - total;
        if self.visible(sy, 34.0) {
            self.layer.bordered(
                [sx, sy],
                [total, 34.0],
                lift(self.theme, 0.05),
                lift(self.theme, 0.07),
                9.0,
                1.0,
            );
        }
        for (i, opt) in options.iter().enumerate() {
            let bx = sx + 3.0 + i as f32 * (seg_w + 3.0);
            if self.visible(sy, 34.0) {
                if i == active {
                    self.layer
                        .rect([bx, sy + 3.0], [seg_w, 28.0], self.theme.accent, 6.0);
                }
                let color = if i == active {
                    Color::WHITE
                } else {
                    self.theme.text_dim
                };
                let tw = self.engine.measure(opt, 12.5, FONT_SANS, Weight::NORMAL);
                self.layer.text(
                    sans(self.engine, opt, 12.5, Weight::NORMAL),
                    bx + (seg_w - tw) / 2.0,
                    sy + 8.0,
                    color,
                );
            }
            self.push_hit(
                [bx, sy, seg_w, 34.0],
                ClickTarget::SettingsSegment(id, i as u8),
            );
        }
    }

    /// Select-style control; clicking cycles to the next value.
    fn select(&mut self, y: f32, label: &str, value: &str, id: &'static str) {
        self.label(label, y);
        let ry = y + (ROW_H - 36.0) / 2.0;
        let sel_w = 240.0;
        let sx = self.right() - sel_w;
        if self.visible(ry, 36.0) {
            self.layer.bordered(
                [sx, ry],
                [sel_w, 36.0],
                lift(self.theme, 0.05),
                lift(self.theme, 0.08),
                8.0,
                1.0,
            );
            let mut shown = value.to_string();
            while shown.chars().count() > 26 {
                shown.pop();
            }
            if shown.len() < value.len() {
                shown.push('…');
            }
            self.layer.text(
                mono(self.engine, &shown, 13.0, Weight::NORMAL),
                sx + 12.0,
                ry + 9.0,
                self.theme.text_bright,
            );
            self.layer.text(
                sans(self.engine, "▾", 11.0, Weight::NORMAL),
                sx + sel_w - 22.0,
                ry + 10.0,
                self.theme.text_dim,
            );
        }
        self.push_hit([sx, ry, sel_w, 36.0], ClickTarget::SettingsCycle(id));
    }

    fn slider(&mut self, y: f32, label: &str, t: f32, value: &str, id: &'static str) {
        self.label(label, y);
        let ry = y + (ROW_H - 36.0) / 2.0;
        let slider_w = 170.0;
        let sx = self.right() - slider_w - 44.0;
        let t = t.clamp(0.0, 1.0);
        if self.visible(ry, 36.0) {
            self.layer.rect(
                [sx, ry + 16.0],
                [slider_w, 4.0],
                lift(self.theme, 0.13),
                2.0,
            );
            self.layer
                .rect([sx, ry + 16.0], [slider_w * t, 4.0], self.theme.accent, 2.0);
            self.layer
                .circle([sx + slider_w * t, ry + 18.0], 15.0, Color::WHITE);
            self.layer.text(
                mono(self.engine, value, 13.0, Weight::NORMAL),
                self.right() - 34.0,
                ry + 9.0,
                self.theme.text_bright,
            );
        }
        self.push_hit(
            [sx - 8.0, ry, slider_w + 16.0, 36.0],
            ClickTarget::SettingsSlider(id),
        );
    }

    fn display(&mut self, y: f32, label: &str, value: &str) {
        self.label(label, y);
        if self.visible(y, ROW_H) {
            let vw = self
                .engine
                .measure(value, 13.0, mono_family(), Weight::NORMAL);
            self.layer.text(
                mono(self.engine, value, 13.0, Weight::NORMAL),
                self.right() - vw,
                y + 17.0,
                self.theme.text_bright,
            );
        }
    }
}

/// Row `i` of a card whose first row starts at `first`; draws the divider
/// above every row but the first.
fn row(form: &mut Form, first: f32, i: usize) -> f32 {
    let y = first + i as f32 * ROW_H;
    if i > 0 {
        form.divider(y - 2.0);
    }
    y
}

fn appearance_page(form: &mut Form, config: &Config) {
    form.group("Font");
    let r = form.card(3);
    let y = row(form, r, 0);
    form.select(y, "Font family", &config.font_family, "font_family");
    let y = row(form, r, 1);
    form.stepper(
        y,
        "Font size",
        &format!("{:.0}", config.font_size),
        "px",
        "font_size",
    );
    let y = row(form, r, 2);
    form.slider(
        y,
        "Line height",
        (config.line_height - 1.0) / 1.2,
        &format!("{:.2}", config.line_height),
        "line_height",
    );

    form.group("Cursor");
    let r = form.card(2);
    let y = row(form, r, 0);
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
    let y = row(form, r, 1);
    form.toggle(y, "Cursor blink", config.cursor_blink, "cursor_blink");

    form.group("Window & theme");
    let r = form.card(6);
    let y = row(form, r, 0);
    form.select(y, "Theme", &config.theme, "theme");
    let y = row(form, r, 1);
    form.toggle(
        y,
        "Sync with OS appearance",
        config.sync_os_theme,
        "sync_os_theme",
    );
    let y = row(form, r, 2);
    form.select(
        y,
        "Light theme (OS sync)",
        &config.theme_light,
        "theme_light",
    );
    let y = row(form, r, 3);
    form.slider(
        y,
        "Window opacity",
        (config.opacity() - 0.3) / 0.7,
        &format!("{:.0}%", config.opacity() * 100.0),
        "window_opacity",
    );
    let y = row(form, r, 4);
    form.stepper(
        y,
        "Window padding",
        &format!("{:.0}", config.padding()),
        "px",
        "window_padding",
    );
    let y = row(form, r, 5);
    form.toggle(y, "Font ligatures", config.ligatures, "ligatures");
}

fn general_page(form: &mut Form, config: &Config) {
    form.group("Shell");
    let r = form.card(2);
    let shell = config
        .shell
        .clone()
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/zsh".into());
    let y = row(form, r, 0);
    form.display(y, "Shell path", &shell);
    let y = row(form, r, 1);
    form.stepper(
        y,
        "Scrollback",
        &format!("{}", config.scrollback),
        "lines",
        "scrollback",
    );

    form.group("Session");
    let r = form.card(3);
    let y = row(form, r, 0);
    form.toggle(
        y,
        "Restore tabs on launch",
        config.restore_session,
        "restore_session",
    );
    let y = row(form, r, 1);
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
    let y = row(form, r, 2);
    form.toggle(
        y,
        "Warn before closing running commands",
        config.confirm_close_running,
        "confirm_close_running",
    );

    form.group("Terminal");
    let rows = if cfg!(target_os = "macos") { 2 } else { 1 };
    let r = form.card(rows);
    let y = row(form, r, 0);
    form.toggle(
        y,
        "Mouse reporting to programs (Shift selects)",
        config.mouse_reporting,
        "mouse_reporting",
    );
    if cfg!(target_os = "macos") {
        let y = row(form, r, 1);
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
    let r = form.card(9);
    let y = row(form, r, 0);
    form.toggle(y, "Vim keybindings", config.vim_mode, "vim_mode");
    let y = row(form, r, 1);
    form.toggle(
        y,
        "Autosuggestions (ghost text)",
        config.autosuggestions,
        "autosuggestions",
    );
    let y = row(form, r, 2);
    form.toggle(
        y,
        "Completions menu",
        config.completions_menu,
        "completions_menu",
    );
    let y = row(form, r, 3);
    form.toggle(
        y,
        "Autocomplete quotes and brackets",
        config.autocomplete_pairs,
        "autocomplete_pairs",
    );
    let y = row(form, r, 4);
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
    let y = row(form, r, 5);
    form.segment(
        y,
        "Prompt",
        &["Runway", "Shell (PS1)"],
        match config.prompt {
            PromptMode::Runway => 0,
            PromptMode::Shell => 1,
        },
        "prompt",
    );
    let y = row(form, r, 6);
    form.toggle(
        y,
        "Expand aliases as you type",
        config.alias_expansion,
        "alias_expansion",
    );
    let y = row(form, r, 7);
    form.toggle(
        y,
        "Underline unknown commands",
        config.error_underline,
        "error_underline",
    );
    let y = row(form, r, 8);
    form.toggle(
        y,
        "Suggest corrections for failed commands",
        config.command_corrections,
        "command_corrections",
    );

    form.group("Blocks & notifications");
    let r = form.card(4);
    let y = row(form, r, 0);
    form.toggle(y, "Copy on select", config.copy_on_select, "copy_on_select");
    let y = row(form, r, 1);
    form.toggle(
        y,
        "Notify when a long command finishes",
        config.notify_on_completion,
        "notify_on_completion",
    );
    let y = row(form, r, 2);
    form.stepper(
        y,
        "Notify after",
        &format!("{}", config.notify_threshold_secs),
        "s",
        "notify_threshold_secs",
    );
    let y = row(form, r, 3);
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
    // filter bar
    form.group("Filter");
    let fy = form.card(1);
    if form.visible(fy, ROW_H) {
        let (text, color) = if state.kb_filter.is_empty() {
            ("Type to filter actions or shortcuts", form.theme.text_muted)
        } else {
            (state.kb_filter.as_str(), form.theme.text_bright)
        };
        form.layer.text(
            mono(form.engine, text, 13.0, Weight::NORMAL),
            form.x + 18.0,
            fy + 17.0,
            color,
        );
    }

    form.group("Shortcuts");
    let rows = filtered_bindings(keymap, &state.kb_filter);
    let row_h = 40.0;
    let h = rows.len().max(1) as f32 * row_h + 4.0;
    let top = form.y.max(form.clip_top);
    let bottom = (form.y + h).min(form.clip_bottom);
    if bottom > top {
        form.layer.bordered(
            [form.x, top],
            [form.card_w, bottom - top],
            lift(form.theme, 0.025),
            lift(form.theme, 0.05),
            11.0,
            1.0,
        );
    }
    let fy = form.y;
    if rows.is_empty() && form.visible(fy, row_h) {
        form.layer.text(
            sans(form.engine, "No matching actions", 13.0, Weight::NORMAL),
            form.x + 18.0,
            fy + 12.0,
            form.theme.text_muted,
        );
    }
    for (i, (action, chips)) in rows.iter().enumerate() {
        let ry = fy + 2.0 + i as f32 * row_h;
        if !form.visible(ry, row_h) {
            continue;
        }
        let recording = state.recording.as_deref() == Some(action.as_str());
        let custom = config.keybindings.contains_key(action);
        if recording {
            form.layer.rect(
                [form.x + 6.0, ry + 2.0],
                [form.card_w - 12.0, row_h - 4.0],
                form.theme.accent.with_alpha(0.12),
                7.0,
            );
        }
        form.layer.text(
            mono(form.engine, action, 12.5, Weight::NORMAL),
            form.x + 18.0,
            ry + 11.0,
            form.theme.text_ui,
        );
        let mut chip_x = form.x + form.card_w - 18.0;
        // reset to default (custom rows only)
        if custom && !recording {
            let rw = 22.0;
            chip_x -= rw;
            form.layer.text(
                sans(form.engine, "↺", 14.0, Weight::NORMAL),
                chip_x + 5.0,
                ry + 9.0,
                form.theme.text_dim,
            );
            form.push_hit(
                [chip_x, ry + 6.0, rw, 26.0],
                ClickTarget::SettingsKeybindReset(action.clone()),
            );
            chip_x -= 8.0;
        }
        if recording {
            let hint = "Press a shortcut · Esc cancels · ⌫ unbinds";
            let tw = form.engine.measure(hint, 12.0, FONT_SANS, Weight::NORMAL);
            form.layer.text(
                sans(form.engine, hint, 12.0, Weight::NORMAL),
                chip_x - tw,
                ry + 12.0,
                form.theme.accent,
            );
        } else if chips.is_empty() {
            let tw = form
                .engine
                .measure("unbound", 12.0, FONT_SANS, Weight::NORMAL);
            form.layer.text(
                sans(form.engine, "unbound", 12.0, Weight::NORMAL),
                chip_x - tw,
                ry + 12.0,
                form.theme.text_muted,
            );
        } else {
            for key in chips.iter().rev() {
                let kw = form
                    .engine
                    .measure(key, 11.0, mono_family(), Weight::NORMAL);
                let cw = kw + 14.0;
                chip_x -= cw;
                let edge = if custom {
                    form.theme.accent.with_alpha(0.5)
                } else {
                    lift(form.theme, 0.08)
                };
                form.layer.bordered(
                    [chip_x, ry + 8.0],
                    [cw, 22.0],
                    lift(form.theme, 0.08),
                    edge,
                    5.0,
                    1.0,
                );
                form.layer.text(
                    mono(form.engine, key, 11.0, Weight::NORMAL),
                    chip_x + 7.0,
                    ry + 12.0,
                    form.theme.text_ui,
                );
                chip_x -= 5.0;
            }
        }
        // the rest of the row starts recording (reset button wins: pushed first)
        form.push_hit(
            [form.x, ry, form.card_w, row_h],
            ClickTarget::SettingsKeybind(action.clone()),
        );
        if i + 1 < rows.len() {
            form.layer.hline(
                form.x + 18.0,
                ry + row_h - 2.0,
                form.card_w - 36.0,
                1.0,
                lift(form.theme, 0.04),
            );
        }
    }
    form.y += h + 26.0;
}

fn themes_page(form: &mut Form, config: &Config) {
    form.group("Available themes");
    let names = runway_core::theme::available_themes();
    let r = form.card(names.len());
    for (i, name) in names.iter().enumerate() {
        let ry = row(form, r, i);
        if !form.visible(ry, ROW_H) {
            continue;
        }
        let t = runway_core::theme::load_by_name(name);
        // swatches
        let mut sx = form.x + 18.0;
        for c in [t.bg, t.accent, t.success, t.error] {
            form.layer.bordered(
                [sx, ry + 17.0],
                [16.0, 16.0],
                c,
                lift(form.theme, 0.2),
                4.0,
                1.0,
            );
            sx += 22.0;
        }
        form.layer.text(
            sans(form.engine, &t.name, 13.5, Weight::MEDIUM),
            sx + 12.0,
            ry + 16.0,
            form.theme.text_bright,
        );
        let tag = if config.theme == *name {
            Some("active")
        } else if config.sync_os_theme && config.theme_light == *name {
            Some("light")
        } else {
            None
        };
        if let Some(tag) = tag {
            let aw = form.engine.measure(tag, 11.5, FONT_SANS, Weight::NORMAL);
            form.layer.rect(
                [form.x + form.card_w - aw - 36.0, ry + 14.0],
                [aw + 18.0, 24.0],
                form.theme.accent.with_alpha(0.15),
                6.0,
            );
            form.layer.text(
                sans(form.engine, tag, 11.5, Weight::NORMAL),
                form.x + form.card_w - aw - 27.0,
                ry + 18.0,
                form.theme.accent,
            );
        }
        form.push_hit(
            [form.x, ry, form.card_w, ROW_H],
            ClickTarget::SettingsTheme(i as u8),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Page, SettingsState};

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
