use std::sync::atomic::{AtomicU32, Ordering};

use rift_core::{Color, TabCloseButton, TabStyle, Theme, ToolbarItem};
use rift_renderer::glyphon::Weight;
use rift_renderer::{FONT_SANS, Layer, TextEngine};

use crate::state::{ClickTarget, Hit};
use crate::tab_status::{Status, TabStatus};
use crate::views::glyphs;
use crate::views::icons::{self, Icon};

/// Warp's tab strip height.
pub const TAB_BAR_H: f32 = 34.0;
/// The strip under `tab_style = "detailed"`: room for the second line.
pub const DETAILED_BAR_H: f32 = 44.0;

/// The strip's height in force (`set_style`), read wherever the layout
/// starts below the strip.
static BAR_H: AtomicU32 = AtomicU32::new(TAB_BAR_H.to_bits());

/// Height of the tab strip: 34 px, 44 px for the detailed tab style.
pub fn bar_h() -> f32 {
    f32::from_bits(BAR_H.load(Ordering::Relaxed))
}

/// Follow `tab_style`; true when the strip's height changed. The vertical
/// rail's top strip holds no tabs and keeps 34 px.
pub fn set_style(style: TabStyle, horizontal: bool) -> bool {
    let h = if style == TabStyle::Detailed && horizontal {
        DETAILED_BAR_H
    } else {
        TAB_BAR_H
    };
    BAR_H.swap(h.to_bits(), Ordering::Relaxed) != h.to_bits()
}
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
    /// What the tab shows (`tab_status::titles`): its name, or its focused
    /// pane's folder.
    pub title: String,
    /// Pane count (the rail's second line).
    pub panes: usize,
    /// Its one status (`tab_status`): which, and whether its focused pane
    /// holds it.
    pub status: Option<TabStatus>,
    /// Pulse phase of a needs-you dot this frame, 0 (smallest) to 1 (always
    /// 1 under reduced motion).
    pub pulse: f32,
    /// The detailed style's second line while nothing happens: the part of
    /// the path the title leaves out (the whole path on a renamed tab).
    pub idle_line: String,
    /// The folder of the pane the status comes from when it is not the
    /// focused one (the detailed style names it: `atlas · Needs you`).
    pub status_pane: String,
    /// Its `tab_select_N` chord while the modifier is held (`⌘3`).
    pub hint: Option<String>,
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
    /// The OSC 9;4 progress its panes report (`progress::of_panes`): a bar
    /// along the tab's bottom edge.
    pub progress: Option<rift_core::Progress>,
    /// Pinned, its group, picks and locks (`app::tab_groups`).
    pub marks: TabMarks,
}

impl TabInfo {
    /// A tab with nothing to say but `title` (tests, the Settings tab).
    pub fn plain(title: impl Into<String>) -> Self {
        TabInfo {
            title: title.into(),
            panes: 1,
            status: None,
            pulse: 1.0,
            idle_line: String::new(),
            status_pane: String::new(),
            hint: None,
            color: None,
            sync: false,
            editing: None,
            editing_selected: false,
            subtitle: String::new(),
            settings: false,
            progress: None,
            marks: TabMarks::default(),
        }
    }
}

/// What a tab carries beyond its title: pinned (compact, first), the
/// header of its group drawn before its group's first tab, folded away in
/// a collapsed group, picked for "Group tabs", a read-only pane.
#[derive(Debug, Clone, Default)]
pub struct TabMarks {
    pub pinned: bool,
    pub header: Option<GroupHeader>,
    /// Its group's colour: the tab's top edge when it has no colour of
    /// its own.
    pub group_color: Option<Color>,
    pub hidden: bool,
    pub picked: bool,
    pub read_only: bool,
    /// The focused pane's repository: uncommitted `(added, removed)` lines
    /// (rail).
    pub diff: Option<(usize, usize)>,
    /// Its branch's pull request number (rail).
    pub pr: Option<u64>,
}

/// A tab group's header: a pill in the strip, a row in the rail.
#[derive(Debug, Clone)]
pub struct GroupHeader {
    pub id: u64,
    pub name: String,
    pub color: Color,
    pub collapsed: bool,
    /// Tabs in the group.
    pub count: usize,
    /// Inline rename in progress: the edited name.
    pub editing: Option<String>,
}

/// A pinned tab's width in the strip (its mark only; the title is its
/// tooltip).
const PIN_W: f32 = 44.0;
/// Height of a group pill.
const PILL_H: f32 = 20.0;

/// The pill's label: the name, and the tab count while folded.
fn pill_label(h: &GroupHeader) -> String {
    match &h.editing {
        Some(e) => e.clone(),
        None if h.collapsed => format!("{}  {}", h.name, h.count),
        None => h.name.clone(),
    }
}

/// Width of a group's pill (with its margins).
fn pill_w(engine: &mut TextEngine, h: &GroupHeader) -> f32 {
    let label = pill_label(h);
    (engine.measure(&label, 11.5, FONT_SANS, Weight::MEDIUM) + 22.0).max(40.0) + 8.0
}

/// A group's pill at `x` in the strip; returns its rect.
fn draw_pill(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    h: &GroupHeader,
    x: f32,
    w: f32,
    blink_on: bool,
) -> [f32; 4] {
    let r = [x + 4.0, (bar_h() - PILL_H) / 2.0, w - 8.0, PILL_H];
    let fill = if h.collapsed {
        h.color.with_alpha(0.85)
    } else {
        h.color.with_alpha(0.18)
    };
    layer.bordered([r[0], r[1]], [r[2], r[3]], fill, h.color, PILL_H / 2.0, 1.0);
    let label = pill_label(h);
    let ink = if h.collapsed {
        theme.bg
    } else {
        theme.text_bright
    };
    let b = make_text(engine, &label, 11.5, FONT_SANS, Weight::MEDIUM);
    layer.text(b, r[0] + 11.0, r[1] + (PILL_H - 15.0) / 2.0, ink);
    if h.editing.is_some() && blink_on {
        let tw = engine.measure(&label, 11.5, FONT_SANS, Weight::MEDIUM);
        layer.rect(
            [r[0] + 11.0 + tw + 1.0, r[1] + 3.0],
            [1.5, PILL_H - 6.0],
            ink,
            0.0,
        );
    }
    r
}

/// A status's colour: needs you warning, failed error, done success, new
/// output and working accent.
pub(crate) fn status_color(theme: &Theme, status: Status) -> Color {
    match status {
        Status::NeedsYou => theme.warning,
        Status::Failed => theme.error,
        Status::Done => theme.success,
        Status::Working | Status::Activity => theme.accent,
    }
}

/// A tab's status mark centred on `c` (the dot style, the rail, a pinned
/// tab): the spinner while working, else an 8 px dot that pulses while it
/// needs you. Another pane's status is a ring, a faded spinner.
pub(crate) fn status_mark(
    layer: &mut Layer,
    theme: &Theme,
    st: TabStatus,
    pulse: f32,
    c: [f32; 2],
) {
    let color = status_color(theme, st.status);
    if st.status == Status::Working {
        let ink = if st.own {
            color
        } else {
            color.with_alpha(0.45)
        };
        glyphs::draw_spinner(layer, c, 13.0, ink);
        return;
    }
    let d = if st.status == Status::NeedsYou {
        6.5 + 1.5 * pulse.clamp(0.0, 1.0)
    } else {
        8.0
    };
    if st.own {
        layer.circle(c, d, color);
    } else {
        let d = d + 1.0;
        layer.bordered(
            [c[0] - d / 2.0, c[1] - d / 2.0],
            [d, d],
            Color::TRANSPARENT,
            color,
            d / 2.0,
            1.5,
        );
    }
}

/// The underline style's line: `w` wide from `x` at `y`, in the status
/// colour; working sweeps an accent third across a faint track (standing
/// with reduced motion); another pane's status is dashed (faded while it
/// works).
fn status_underline(layer: &mut Layer, theme: &Theme, st: TabStatus, x: f32, y: f32, w: f32) {
    const H: f32 = 2.0;
    // a one-letter title (`~`) still gets a line its sweep shows on
    const MIN_W: f32 = 24.0;
    let (x, w) = if w < MIN_W {
        (x - (MIN_W - w) / 2.0, MIN_W)
    } else {
        (x, w)
    };
    let color = status_color(theme, st.status);
    if st.status == Status::Working {
        let a = if st.own { 1.0 } else { 0.45 };
        layer.rect([x, y], [w, H], color.with_alpha(0.2 * a), 1.0);
        let seg = (w / 3.0).max(6.0);
        if crate::progress::reduce_motion() {
            layer.rect([x + (w - seg) / 2.0, y], [seg, H], color.with_alpha(a), 1.0);
        } else {
            let t = (glyphs::epoch().elapsed().as_secs_f32() / SWEEP_PERIOD).fract();
            let start = x - seg + (w + seg) * t;
            let (l, r) = (start.max(x), (start + seg).min(x + w));
            if r > l {
                layer.rect([l, y], [r - l, H], color.with_alpha(a), 1.0);
            }
        }
        return;
    }
    if st.own {
        layer.rect([x, y], [w, H], color, 1.0);
        return;
    }
    // another pane's: 4 on, 3 off
    let mut dx = 0.0;
    while dx < w {
        layer.rect([x + dx, y], [4.0f32.min(w - dx), H], color, 1.0);
        dx += 7.0;
    }
}

/// Height of a tab's progress bar.
const PROGRESS_H: f32 = 2.0;
/// One pass of the indeterminate sweep, and the share of the width it lights.
const SWEEP_PERIOD: f32 = 1.4;
const SWEEP_W: f32 = 0.3;

/// A tab's OSC 9;4 progress: a 2 px bar along the bottom edge of the tab
/// spanning `x..x + w`, ending at `bottom`. `accent` for normal and
/// indeterminate, `error` for an error, `warning` when paused, on a track of
/// the same colour at 25 %. Without a value an error or paused bar is full;
/// indeterminate sweeps a third of the width across, or stands full at 60 %
/// with reduced motion on.
pub(crate) fn draw_progress(
    layer: &mut Layer,
    theme: &Theme,
    x: f32,
    bottom: f32,
    w: f32,
    p: rift_core::Progress,
) {
    use rift_core::ProgressState;
    let y = bottom - PROGRESS_H;
    let color = match p.state {
        ProgressState::Error => theme.error,
        ProgressState::Paused => theme.warning,
        ProgressState::Normal | ProgressState::Indeterminate => theme.accent,
    };
    layer.rect([x, y], [w, PROGRESS_H], color.with_alpha(0.25), 0.0);
    match (p.state, p.pct) {
        (ProgressState::Indeterminate, _) if crate::progress::reduce_motion() => {
            layer.rect([x, y], [w, PROGRESS_H], color.with_alpha(0.6), 0.0);
        }
        (ProgressState::Indeterminate, _) => {
            let seg = w * SWEEP_W;
            let t = (glyphs::epoch().elapsed().as_secs_f32() / SWEEP_PERIOD).fract();
            // from just off the left edge to just off the right, clipped
            let start = x - seg + (w + seg) * t;
            let (l, r) = (start.max(x), (start + seg).min(x + w));
            if r > l {
                layer.rect([l, y], [r - l, PROGRESS_H], color, 0.0);
            }
        }
        (_, pct) => {
            let filled = w * f32::from(pct.unwrap_or(100)) / 100.0;
            if filled > 0.0 {
                layer.rect([x, y], [filled, PROGRESS_H], color, 0.0);
            }
        }
    }
}

pub(crate) fn make_text(
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

/// Longest prefix of `text` (plus `…`) that fits `max_w`. The width grows
/// with the prefix, so the cut is found by bisection.
pub(crate) fn elide(engine: &mut TextEngine, text: &str, size: f32, max_w: f32) -> String {
    if engine.measure(text, size, FONT_SANS, Weight::NORMAL) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let candidate = |keep: usize| {
        let head: String = chars[..keep].iter().collect();
        format!("{}…", head.trim_end())
    };
    let mut fits =
        |keep: usize| engine.measure(&candidate(keep), size, FONT_SANS, Weight::NORMAL) <= max_w;
    if chars.is_empty() || !fits(0) {
        return String::new();
    }
    // `keep` chars fit; `hi` and beyond do not (the whole text was tested)
    let (mut keep, mut hi) = (0, chars.len());
    while hi - keep > 1 {
        let mid = keep + (hi - keep) / 2;
        if fits(mid) {
            keep = mid;
        } else {
            hi = mid;
        }
    }
    candidate(keep)
}

/// `…` plus the longest suffix of `text` that fits `max_w` (a path keeps
/// its last folders). Bisection, as [`elide`].
pub(crate) fn elide_start(engine: &mut TextEngine, text: &str, size: f32, max_w: f32) -> String {
    if engine.measure(text, size, FONT_SANS, Weight::NORMAL) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let candidate = |skip: usize| {
        let tail: String = chars[skip..].iter().collect();
        format!("…{}", tail.trim_start())
    };
    let mut fits =
        |skip: usize| engine.measure(&candidate(skip), size, FONT_SANS, Weight::NORMAL) <= max_w;
    if chars.len() < 2 || !fits(chars.len() - 1) {
        return String::new();
    }
    // `skip` chars cut does not fit; `hi` and beyond do (the last one was tested)
    let (mut lo, mut hi) = (0, chars.len() - 1);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if fits(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    candidate(hi)
}

/// State of the top-bar panel buttons.
#[derive(Debug, Clone, Default)]
pub struct PanelButtons {
    pub explorer_open: bool,
    pub review_open: bool,
    /// Working-tree `(added, removed)` lines; `None` outside a repository.
    pub diff: Option<(usize, usize)>,
    /// The Settings tab is shown: no repository, so the review button is a
    /// dimmed ± that does nothing.
    pub review_unavailable: bool,
    pub notifications_open: bool,
    /// Unread notifications (dot on the inbox button).
    pub unread: usize,
    /// A new release is available: the "Update Rift" chip.
    pub update: bool,
    /// A phone is connected: the remote chip's label ("Remote · Sam's
    /// iPhone", "Remote · 2 devices").
    pub remote: Option<String>,
    /// The drop-down hotkey window has no window buttons (Linux draws
    /// them; macOS shows none on a window without a title bar).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub no_window_buttons: bool,
    /// The toolbar's buttons left of the tabs and at the right end, in
    /// order (`Config::toolbar`).
    pub left: Vec<ToolbarItem>,
    pub right: Vec<ToolbarItem>,
    pub palette_open: bool,
    pub settings_open: bool,
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
        ((bar_h() - UPDATE_CHIP_H) / 2.0).round(),
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

/// Width of a toolbar button, the review button's ± included: the
/// `+added −removed` counts live in the prompt's chip, not here too.
const TOOLBAR_ITEM_W: f32 = 26.0;

/// A toolbar button's icon (`None` for code review: its ± is drawn).
pub fn toolbar_icon(item: ToolbarItem) -> Option<Icon> {
    match item {
        ToolbarItem::Explorer => Some(Icon::Sidebar),
        ToolbarItem::Notifications => Some(Icon::Inbox),
        ToolbarItem::Palette => Some(Icon::Search),
        ToolbarItem::Settings => Some(Icon::Settings),
        ToolbarItem::Review => None,
    }
}

/// The ± of a clean tree, centred on `c`, as quads (crisp at any scale).
pub fn draw_plus_minus(layer: &mut Layer, c: [f32; 2], color: Color) {
    let (gx, gy) = (c[0] - 6.0, c[1] - 6.0);
    layer.rect([gx + 5.25, gy], [1.5, 8.0], color, 0.5);
    layer.rect([gx + 2.0, gy + 3.25], [8.0, 1.5], color, 0.5);
    layer.rect([gx + 2.0, gy + 10.5], [8.0, 1.5], color, 0.5);
}

/// Draw one toolbar button starting at `x` and report its click target;
/// returns its rect.
#[allow(clippy::too_many_arguments)]
fn toolbar_item(
    layer: &mut Layer,
    theme: &Theme,
    panels: &PanelButtons,
    item: ToolbarItem,
    x: f32,
    by: f32,
    mouse: Option<(f32, f32)>,
    hits: &mut Vec<Hit>,
) -> [f32; 4] {
    let size = 26.0;
    let rect = [x, by, TOOLBAR_ITEM_W, size];
    let hover = inside(rect, mouse);
    let (icon, on, target) = match item {
        ToolbarItem::Explorer => (
            Icon::Sidebar,
            panels.explorer_open,
            ClickTarget::ToggleExplorer,
        ),
        ToolbarItem::Notifications => (
            Icon::Inbox,
            panels.notifications_open,
            ClickTarget::ToggleNotifications,
        ),
        ToolbarItem::Palette => (
            Icon::Search,
            panels.palette_open,
            ClickTarget::TogglePalette,
        ),
        ToolbarItem::Settings => (
            Icon::Settings,
            panels.settings_open,
            ClickTarget::ToggleSettings,
        ),
        ToolbarItem::Review => {
            let unavailable = panels.review_unavailable;
            review_button(layer, theme, panels, rect, hover && !unavailable);
            hits.push(Hit {
                rect,
                target: if unavailable {
                    ClickTarget::ReviewUnavailable
                } else {
                    ClickTarget::ToggleReview
                },
            });
            return rect;
        }
    };
    icon_button(layer, theme, rect, icon, 16.0, on, hover);
    if item == ToolbarItem::Notifications && panels.unread > 0 {
        let c = [x + size - 7.0, by + 7.0];
        layer.circle(c, 9.0, theme.ground(theme.bg_chrome));
        layer.circle(c, 6.0, theme.accent);
    }
    hits.push(Hit { rect, target });
    rect
}

/// The code review button: a ±, brighter while the working tree has
/// changes (their counts show in the prompt's chip and the panel).
fn review_button(
    layer: &mut Layer,
    theme: &Theme,
    panels: &PanelButtons,
    review: [f32; 4],
    hover: bool,
) {
    let [rx, by, review_w, size] = review;
    if panels.review_open {
        layer.rect(
            [rx, by],
            [review_w, size],
            theme.accent.with_alpha(0.14),
            4.0,
        );
    } else if hover {
        layer.rect([rx, by], [review_w, size], lift(theme, 0.08), 4.0);
    }
    let dirty = panels.diff.is_some_and(|(a, d)| a + d > 0);
    let color = if panels.review_unavailable {
        theme.text_muted.with_alpha(0.45)
    } else if panels.review_open {
        theme.accent
    } else if dirty {
        theme.text_ui
    } else {
        theme.text_dim
    };
    // ± as quads (crisp at any scale)
    draw_plus_minus(layer, [rx + size / 2.0, by + 13.0], color);
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
    close_button: TabCloseButton,
    close_left: bool,
    style: TabStyle,
    hits: &mut Vec<Hit>,
) {
    let bar = bar_h();
    layer.rect([0.0, 0.0], [w, bar], theme.ground(theme.bg_chrome), 0.0);
    layer.hline(0.0, bar - 1.0, w, 1.0, theme.border);

    // Warp layout: the toolbar's left buttons right after the traffic
    // lights (default: the file explorer toggle), its right buttons at the
    // far right (default: the inbox, then code review showing the working
    // tree's `+added −removed`); `toolbar_left` / `toolbar_right` choose
    let size = 26.0;
    let by = (bar - size) / 2.0;
    let mut lx = TABS_X + 2.0;
    for item in &panels.left {
        // a narrow window keeps room for a tab and + (later buttons go)
        if lx + TOOLBAR_ITEM_W > w - MIN_TAB_W - 40.0 {
            break;
        }
        let rect = toolbar_item(layer, theme, &panels, *item, lx, by, mouse, hits);
        lx = rect[0] + rect[2] + 4.0;
    }
    let tabs_x = if panels.left.is_empty() {
        TABS_X + 2.0
    } else {
        lx + 4.0
    };
    // right to left from the strip's end
    let mut right_x = w - 10.0;
    for item in panels.right.iter().rev() {
        let width = TOOLBAR_ITEM_W;
        // never over the tabs: the leftmost ones go first in a narrow window
        if right_x - width < tabs_x + MIN_TAB_W + 40.0 {
            break;
        }
        let rect = toolbar_item(
            layer,
            theme,
            &panels,
            *item,
            right_x - width,
            by,
            mouse,
            hits,
        );
        right_x = rect[0] - 4.0;
    }
    // a new release: the chip left of the right buttons
    let mut w = right_x - 2.0;
    let mut chip_right = right_x - 4.0;
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
    if !panels.no_window_buttons {
        for (i, c) in ["#ff5f57", "#febc2e", "#28c840"].iter().enumerate() {
            let cx = TRAFFIC_X + i as f32 * TRAFFIC_GAP;
            layer.circle([cx, bar / 2.0], 12.0, Color::hex(c).unwrap());
            hits.push(Hit {
                rect: [cx - 8.0, bar / 2.0 - 8.0, 16.0, 16.0],
                target: ClickTarget::TrafficLight(i as u8),
            });
        }
    }

    // one fixed width (Warp), shrunk evenly when the tabs overflow the
    // strip; pinned tabs are compact, folded ones take no room, and a
    // group's pill sits before its first tab
    let pills: Vec<f32> = tabs
        .iter()
        .map(|t| t.marks.header.as_ref().map_or(0.0, |h| pill_w(engine, h)))
        .collect();
    let natural: Vec<f32> = tabs
        .iter()
        .map(|t| match (t.marks.hidden, t.marks.pinned) {
            (true, _) => 0.0,
            (false, true) => PIN_W,
            (false, false) => TAB_W,
        })
        .collect();
    let fixed: f32 =
        pills.iter().sum::<f32>() + natural.iter().filter(|w| **w == PIN_W).sum::<f32>();
    let avail = (w - tabs_x - 40.0 - CHEVRON_W - fixed).max(MIN_TAB_W);
    let total: f32 = natural.iter().filter(|w| **w == TAB_W).sum();
    let regular = natural.iter().filter(|w| **w == TAB_W).count();
    let n = tabs.len();
    // visible window `start..end`; everything outside it collapses into the
    // `‹N` / `N›` chips so the active tab always stays on screen
    let (mut start, mut end) = (0, n);
    let widths: Vec<f32> = if total <= avail {
        natural
    } else if MIN_TAB_W * regular as f32 <= avail {
        let each = avail / regular.max(1) as f32;
        natural
            .iter()
            .map(|x| if *x == TAB_W { x.min(each) } else { *x })
            .collect()
    } else {
        let k = (((avail - 2.0 * OVERFLOW_CHIP_W) / MIN_TAB_W) as usize).clamp(1, n);
        start = active.min(n - 1).saturating_sub(k / 2).min(n - k);
        end = start + k;
        let chips = (start > 0) as u8 as f32 + (end < n) as u8 as f32;
        let each = (avail - chips * OVERFLOW_CHIP_W) / k as f32;
        natural
            .iter()
            .map(|x| if *x == TAB_W { x.min(each) } else { *x })
            .collect()
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
            (bar - 15.0) / 2.0,
            theme.text_dim,
        );
        hits.push(Hit {
            rect: [x, 0.0, OVERFLOW_CHIP_W, bar],
            target: ClickTarget::TabSelect(to),
        });
    };
    if start > 0 {
        chip(layer, engine, hits, x, format!("‹ {start}"), start - 1);
        x += OVERFLOW_CHIP_W;
    }
    for (i, tab) in tabs.iter().enumerate().skip(start).take(end - start) {
        if let Some(h) = &tab.marks.header {
            let pr = draw_pill(layer, engine, theme, h, x, pills[i], blink_on);
            hits.push(Hit {
                rect: pr,
                target: ClickTarget::GroupHeader(h.id),
            });
            x += pills[i];
        }
        if tab.marks.hidden {
            continue;
        }
        let tab_w = widths[i];
        let slot = [x, 0.0, tab_w, bar];
        let lift_x = lifted
            .filter(|l| l.index == i)
            .map(|l| l.x.clamp(tabs_x, (w - tab_w).max(tabs_x)));
        let hovered = lifted.is_none() && inside(slot, mouse);
        // a pinned tab has no × (⌘W asks first); a held ⌘'s number takes
        // the ×'s place
        let shows_close =
            !tab.marks.pinned && tab.hint.is_none() && shows_close(close_button, tab, hovered);
        let look = Look {
            active: i == active,
            hovered,
            floating: false,
            shows_close,
            close_left,
            blink_on,
            style,
        };
        match lift_x {
            Some(lx) => {
                // the slot it will land in: an empty well with a caret
                layer.rect([x, 0.0], [tab_w, bar - 1.0], lift(theme, 0.02), 0.0);
                layer.rect([x, 6.0], [2.0, bar - 12.0], theme.accent, 1.0);
                let floating = Look {
                    hovered: true,
                    floating: true,
                    ..look
                };
                draw_tab(over, engine, theme, tab, [lx, -4.0, tab_w, bar], floating);
            }
            None => draw_tab(layer, engine, theme, tab, slot, look),
        }
        let close = close_rect(slot, close_left, line_style(style, tab));
        if shows_close {
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

    // + new tab, joined to ⌄ (the profile menu, Windows Terminal's new-tab
    // dropdown): one pill, a divider between the halves
    let plus = [x + 6.0, by, size, size];
    let chevron = [plus[0] + size, by, CHEVRON_W, size];
    let pill = [plus[0], by, size + CHEVRON_W, size];
    if inside(pill, mouse) {
        layer.rect(
            [pill[0], pill[1]],
            [pill[2], pill[3]],
            lift(theme, 0.05),
            4.0,
        );
    }
    icon_button(
        layer,
        theme,
        plus,
        Icon::Plus,
        14.0,
        false,
        inside(plus, mouse),
    );
    icon_button(
        layer,
        theme,
        chevron,
        Icon::ChevronDown,
        12.0,
        false,
        inside(chevron, mouse),
    );
    layer.rect(
        [chevron[0], by + size * 0.25],
        [1.0, size * 0.5],
        lift(theme, 0.14),
        0.0,
    );
    hits.push(Hit {
        rect: plus,
        target: ClickTarget::TabNew,
    });
    hits.push(Hit {
        rect: chevron,
        target: ClickTarget::TabProfiles,
    });

    // the empty strip drags the window (macOS: the native titlebar ends at
    // the traffic lights; Linux has none)
    let end_x = pill[0] + pill[2] + 2.0;
    hits.push(Hit {
        rect: [end_x, 0.0, (w - end_x).max(0.0), bar],
        target: ClickTarget::WindowDrag,
    });
}

/// Width of the ⌄ half of the + button (the profile menu).
const CHEVRON_W: f32 = 18.0;

/// The close button box, the slots at a tab's two ends (the × or a held
/// ⌘'s number at one, the status dot or the tab's own icon at the other)
/// and their inset, and the tab width bounds.
const CLOSE_BOX: f32 = 20.0;
const SLOT: f32 = 22.0;
const SLOT_INSET: f32 = 6.0;
/// Every tab's width (Warp's fixed-width tabs); a longer title is cut short.
const TAB_W: f32 = 200.0;

/// A strip tab shows its × (`tab_close_button`): `hover` on the hovered tab
/// (the active one too only while hovered), `always` on every tab, `never`
/// on none. A tab being renamed keeps it except under `never`.
pub(crate) fn shows_close(mode: TabCloseButton, tab: &TabInfo, hovered: bool) -> bool {
    match mode {
        TabCloseButton::Never => false,
        TabCloseButton::Hover => hovered || tab.editing.is_some(),
        TabCloseButton::Always => true,
    }
}

/// The geometry a tab's lines follow: a detailed tab with nothing to say on
/// its second line (home, idle) centres its title like a one-line tab.
fn line_style(style: TabStyle, tab: &TabInfo) -> TabStyle {
    if style == TabStyle::Detailed && tab.status.is_none() && tab.idle_line.is_empty() {
        TabStyle::Dot
    } else {
        style
    }
}

/// Vertical middle of a tab's title line (and of its end slots): the tab's
/// middle, or the first line's under the detailed style, so the second line
/// runs the tab's whole width.
fn line_mid(slot: [f32; 4], style: TabStyle) -> f32 {
    let mid = slot[1] + slot[3] / 2.0;
    if style == TabStyle::Detailed {
        mid - 5.5
    } else {
        mid
    }
}

/// The × of a tab in `slot`: at its right end, or its left one
/// (`tab_close_side = "left"`, the macOS style), on the title's line.
fn close_rect(slot: [f32; 4], left: bool, style: TabStyle) -> [f32; 4] {
    [
        if left {
            slot[0] + SLOT_INSET + (SLOT - CLOSE_BOX) / 2.0
        } else {
            slot[0] + slot[2] - SLOT_INSET - SLOT + (SLOT - CLOSE_BOX) / 2.0
        },
        line_mid(slot, style) - CLOSE_BOX / 2.0,
        CLOSE_BOX,
        CLOSE_BOX,
    ]
}

/// How one tab is drawn this frame.
#[derive(Clone, Copy)]
struct Look {
    active: bool,
    hovered: bool,
    /// Lifted by a drag (drawn over its neighbours).
    floating: bool,
    shows_close: bool,
    close_left: bool,
    blink_on: bool,
    style: TabStyle,
}

/// One tab (`tab_style`): its title centred between the two end slots —
/// the × (or a held ⌘'s number) at the end `tab_close_side` names, the
/// status dot (dot style) or its own icon (Settings, synced, read-only) at
/// the other. The underline style underlines the title in the status
/// colour; the detailed style says the status in words on a second line.
fn draw_tab(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    tab: &TabInfo,
    r: [f32; 4],
    look: Look,
) {
    let [x, y, tw, th] = r;
    let mid = y + th / 2.0;
    if look.floating {
        layer.shadow([x, y], [tw, th], Color::BLACK.with_alpha(0.55), 0.0, 10.0);
        layer.bordered([x, y], [tw, th], theme.bg, lift(theme, 0.12), 0.0, 1.0);
    } else if look.active {
        // joins the pane below: covers the strip's bottom hairline
        layer.rect([x, y], [tw, th], theme.ground(theme.bg), 0.0);
    } else if look.hovered {
        layer.rect([x, y], [tw, th - 1.0], lift(theme, 0.03), 0.0);
    }
    if tab.marks.picked {
        layer.rect([x, y], [tw, th - 1.0], theme.accent.with_alpha(0.14), 0.0);
    }
    // top edge: accent on the active tab, the tab color (else its group's)
    // on any colored one
    match (tab.color.or(tab.marks.group_color), look.active) {
        (Some(c), _) => layer.rect([x, y], [tw, 2.0], c, 0.0),
        // a gradient accent (Warp gradient themes) runs across the edge
        (None, true) => match theme.accent_gradient {
            Some(g) => layer.gradient([x, y], [tw, 2.0], g.from, g.to, g.vertical, 0.0),
            None => layer.rect([x, y], [tw, 2.0], theme.accent, 0.0),
        },
        _ => {}
    }
    if !look.floating {
        // both edges: the left one lands on the previous tab's right edge,
        // and gives the first tab a left edge against the sidebar button
        layer.rect([x - 1.0, y], [1.0, th - 1.0], theme.border, 0.0);
        layer.rect([x + tw - 1.0, y], [1.0, th - 1.0], theme.border, 0.0);
    }

    let text_color = if look.active || look.hovered {
        theme.text_ui
    } else {
        theme.text_dim
    };
    // what a tab is, when it is not a shell: Settings, synced, read-only
    let icon = if tab.settings {
        Some((Icon::Settings, text_color))
    } else if tab.sync {
        Some((Icon::Sync, theme.accent))
    } else if tab.marks.read_only {
        Some((Icon::Lock, text_color))
    } else {
        None
    };
    let dot_style = look.style == TabStyle::Dot;

    if tab.marks.pinned {
        // compact: one mark centred (the title is the tooltip): a held ⌘'s
        // number, else the status, else what the tab is
        let c = [x + tw / 2.0, mid];
        if let Some(h) = &tab.hint {
            draw_hint(layer, engine, theme, h, c, look.active);
        } else if let Some(st) = tab.status {
            status_mark(layer, theme, st, tab.pulse, c);
        } else {
            let (icon, ink) = icon.unwrap_or((Icon::Terminal, text_color));
            icons::draw(layer, icon, c, 14.0, ink);
        }
        if let Some(p) = tab.progress {
            draw_progress(layer, theme, x, y + th, tw - 1.0, p);
        }
        return;
    }

    // the two end slots, on the title's line
    let line = line_mid(r, line_style(look.style, tab));
    let close = close_rect(r, look.close_left, line_style(look.style, tab));
    let close_c = [close[0] + CLOSE_BOX / 2.0, line];
    let mark_c = [
        if look.close_left {
            x + tw - SLOT_INSET - SLOT / 2.0
        } else {
            x + SLOT_INSET + SLOT / 2.0
        },
        line,
    ];
    match (tab.status.filter(|_| dot_style), icon) {
        (Some(st), _) => status_mark(layer, theme, st, tab.pulse, mark_c),
        (None, Some((icon, ink))) => icons::draw(layer, icon, mark_c, 14.0, ink),
        (None, None) => {}
    }
    if let Some(h) = &tab.hint {
        draw_hint(layer, engine, theme, h, close_c, look.active);
    } else if look.shows_close {
        icons::draw(
            layer,
            Icon::Close,
            close_c,
            10.0,
            if look.hovered && !look.floating {
                theme.text_ui
            } else {
                theme.text_dim
            },
        );
    }

    // the title, centred between the slots
    let left = x + SLOT_INSET + SLOT + SLOT_INSET;
    let right = x + tw - SLOT_INSET - SLOT - SLOT_INSET;
    let room = (right - left).max(0.0);
    let title_y = line - 8.5;
    if let Some(edit) = &tab.editing {
        // inline rename field over the title's room
        let fw = room.max(60.0);
        layer.bordered(
            [left - 4.0, line - 12.0],
            [fw + 8.0, 24.0],
            theme.bg,
            theme.accent,
            3.0,
            1.0,
        );
        // a name wider than the field shows its end, where the caret is
        let edit = &elide_start(engine, edit, 12.5, fw - 4.0);
        let ew = engine.measure(edit, 12.5, FONT_SANS, Weight::NORMAL);
        if tab.editing_selected && !edit.is_empty() {
            layer.rect(
                [left - 1.0, line - 9.0],
                [ew + 2.0, 18.0],
                theme.selection,
                2.0,
            );
        }
        if !edit.is_empty() {
            let b = make_text(engine, edit, 12.5, FONT_SANS, Weight::NORMAL);
            layer.text(b, left, line - 8.5, theme.text_bright);
        }
        if look.blink_on {
            layer.rect(
                [left + ew + 1.0, line - 8.0],
                [1.5, 16.0],
                theme.accent,
                0.0,
            );
        }
    } else {
        let title = elide(engine, &tab.title, 12.5, room);
        let title_w = engine.measure(&title, 12.5, FONT_SANS, Weight::NORMAL);
        let tx = left + (room - title_w) / 2.0;
        if !title.is_empty() {
            let b = make_text(engine, &title, 12.5, FONT_SANS, Weight::NORMAL);
            layer.text(b, tx, title_y, text_color);
        }
        match look.style {
            TabStyle::Underline => {
                if let Some(st) = tab.status
                    && title_w > 0.0
                {
                    status_underline(layer, theme, st, tx, mid + 9.0, title_w);
                }
            }
            TabStyle::Detailed => {
                // under the slots too: the whole tab's width
                let inset = x + SLOT_INSET + 4.0;
                let wide = (tw - 2.0 * (SLOT_INSET + 4.0)).max(0.0);
                draw_second_line(layer, engine, theme, tab, inset, wide, mid + 2.0);
            }
            TabStyle::Dot => {}
        }
    }
    if let Some(p) = tab.progress {
        // inside the edges drawn above
        draw_progress(layer, theme, x, y + th, tw - 1.0, p);
    }
}

/// The detailed style's second line, centred in `room` from `left` with
/// its top at `top`: the status in words after a small spinner or dot
/// (another pane's names that pane: `atlas · Needs you`, its dot a ring),
/// else the part of the path the title leaves out.
#[allow(clippy::too_many_arguments)]
fn draw_second_line(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    tab: &TabInfo,
    left: f32,
    room: f32,
    top: f32,
) {
    const SIZE: f32 = 11.0;
    const MARK: f32 = 10.0;
    let Some(st) = tab.status else {
        // a path keeps its end: the folders nearest the title tell it apart
        let line = elide_start(engine, &tab.idle_line, SIZE, room);
        if !line.is_empty() {
            let w = engine.measure(&line, SIZE, FONT_SANS, Weight::NORMAL);
            let b = make_text(engine, &line, SIZE, FONT_SANS, Weight::NORMAL);
            layer.text(b, left + (room - w) / 2.0, top, theme.text_muted);
        }
        return;
    };
    let words = if st.own || tab.status_pane.is_empty() {
        st.status.words().to_string()
    } else {
        format!("{} · {}", tab.status_pane, st.status.words())
    };
    let words = elide(engine, &words, SIZE, (room - MARK - 5.0).max(0.0));
    let ww = engine.measure(&words, SIZE, FONT_SANS, Weight::NORMAL);
    let x0 = left + (room - (MARK + 5.0 + ww)) / 2.0;
    let mark = [x0 + MARK / 2.0, top + 7.0];
    let color = status_color(theme, st.status);
    if st.status == Status::Working {
        let ink = if st.own {
            color
        } else {
            color.with_alpha(0.45)
        };
        glyphs::draw_spinner(layer, mark, MARK, ink);
    } else if st.own {
        layer.circle(mark, 6.0, color);
    } else {
        layer.bordered(
            [mark[0] - 3.5, mark[1] - 3.5],
            [7.0, 7.0],
            Color::TRANSPARENT,
            color,
            3.5,
            1.2,
        );
    }
    // the words in the status colour (new output and working stay muted:
    // the accent there is a mark, not a warning)
    let ink = match st.status {
        Status::NeedsYou | Status::Failed | Status::Done => color,
        Status::Working | Status::Activity => theme.text_muted,
    };
    let b = make_text(engine, &words, SIZE, FONT_SANS, Weight::NORMAL);
    layer.text(b, x0 + MARK + 5.0, top, ink);
}

/// A held ⌘'s number (`⌘3`) centred on `c`: muted caps on a faint pill.
fn draw_hint(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    hint: &str,
    c: [f32; 2],
    active: bool,
) {
    let w = engine.measure(hint, 11.0, FONT_SANS, Weight::MEDIUM) + 8.0;
    layer.rect(
        [c[0] - w / 2.0, c[1] - 8.0],
        [w, 16.0],
        lift(theme, 0.09),
        3.0,
    );
    let b = make_text(engine, hint, 11.0, FONT_SANS, Weight::MEDIUM);
    layer.text(
        b,
        c[0] - w / 2.0 + 4.0,
        c[1] - 7.2,
        if active {
            theme.text_ui
        } else {
            theme.text_dim
        },
    );
}

/// One strip tab for `dump`: what it shows and says.
pub fn describe(tab: &TabInfo) -> String {
    format!(
        "{:?} status={} hint={} idle_line={:?} status_pane={:?}",
        tab.title,
        tab.status.map_or("-".to_string(), |s| format!(
            "{}/{}",
            s.status.key(),
            if s.own { "own" } else { "other" }
        )),
        tab.hint.as_deref().unwrap_or("-"),
        tab.idle_line,
        tab.status_pane,
    )
}

/// The × of a tab: the Settings tab closes itself, a shell tab its panes.
fn close_target(tab: &TabInfo, i: usize) -> ClickTarget {
    if tab.settings {
        ClickTarget::SettingsClose
    } else {
        ClickTarget::TabClose(i)
    }
}

/// Width of the vertical tab rail (`tab_layout = "vertical"`; drawn by
/// `views::rail`).
pub const RAIL_W: f32 = 220.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_button_setting_picks_the_tabs_that_show_it() {
        let plain = TabInfo::plain("x");
        // hover (default): the hovered tab only, active or not
        assert!(shows_close(TabCloseButton::Hover, &plain, true));
        assert!(!shows_close(TabCloseButton::Hover, &plain, false));
        // always: every tab
        assert!(shows_close(TabCloseButton::Always, &plain, false));
        // never: not even the hovered or renamed tab
        let mut renaming = TabInfo::plain("x");
        renaming.editing = Some("x".into());
        assert!(!shows_close(TabCloseButton::Never, &plain, true));
        assert!(!shows_close(TabCloseButton::Never, &renaming, false));
        assert!(shows_close(TabCloseButton::Hover, &renaming, false));
    }

    #[test]
    fn the_detailed_style_makes_the_strip_taller() {
        // the one global the layout reads: restore it for other tests
        let was = bar_h();
        set_style(TabStyle::Detailed, true);
        assert_eq!(bar_h(), DETAILED_BAR_H);
        // the rail's top strip holds no tabs
        assert!(set_style(TabStyle::Detailed, false));
        assert_eq!(bar_h(), TAB_BAR_H);
        assert!(!set_style(TabStyle::Underline, true));
        assert_eq!(bar_h(), TAB_BAR_H);
        BAR_H.store(was.to_bits(), Ordering::Relaxed);
    }

    #[test]
    fn the_close_slot_sits_inside_the_tab_at_either_end() {
        let slot = [100.0, 0.0, 200.0, 34.0];
        let right = close_rect(slot, false, TabStyle::Dot);
        let left = close_rect(slot, true, TabStyle::Dot);
        assert!(right[0] + right[2] <= slot[0] + slot[2] - SLOT_INSET);
        assert!(left[0] >= slot[0] + SLOT_INSET);
        // centred in a one-line tab
        assert_eq!(right[1] + CLOSE_BOX / 2.0, 17.0);
        // the detailed style's: on the title's line, inside the tab, above
        // the second line (whose text starts at the middle + 2)
        let tall = close_rect([0.0, 0.0, 200.0, DETAILED_BAR_H], false, TabStyle::Detailed);
        assert!(tall[1] >= 0.0);
        assert!(tall[1] + CLOSE_BOX / 2.0 < DETAILED_BAR_H / 2.0);
    }
}
