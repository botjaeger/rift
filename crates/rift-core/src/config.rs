use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
    /// An outlined block: the glyph under it keeps its colour (Windows
    /// Terminal "emptyBox").
    #[serde(alias = "hollow_block")]
    Hollow,
    /// Two thin lines under the cell (Windows Terminal "doubleUnderscore").
    #[serde(rename = "double_underline")]
    DoubleUnderline,
}

/// Cells an East Asian Ambiguous character (Unicode EAW=A: `±`, `①`, Greek
/// and Cyrillic letters, box drawing) takes in the terminal grid. `wide`
/// is for CJK fonts and programs that expect them two cells wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AmbiguousWidth {
    #[default]
    Narrow,
    Wide,
}

/// Where the command editor sits (Warp "Input position").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum InputPosition {
    #[default]
    Bottom,
    Top,
}

/// Where blocks start while they do not fill the pane, with the input at
/// the bottom: right above the input (Warp "Pin to the bottom"), or at the
/// top of the pane, the gap left above the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BlocksStart {
    #[default]
    Bottom,
    Top,
}

/// The terminal's overlay scrollbar with marks (Windows Terminal
/// `showMarksOnScrollbar`): `auto` shows it while the pane scrolls or the
/// pointer is over its edge, `always` while there is anything to scroll,
/// `never` not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScrollbarMode {
    #[default]
    Auto,
    Always,
    Never,
}

/// Who answers ⌃R, ⌃T and ⌥C in the command editor: the shell's own
/// widgets (fzf, atuin) where its config binds them, else Rift (Warp); or
/// always Rift (its history pane for ⌃R).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ShellWidgets {
    #[default]
    Auto,
    Rift,
}

/// What the "Time on this tab" chip counts: the time since the tab's first
/// command, or only the time a command ran in it (time logging).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ElapsedCounts {
    #[default]
    Always,
    Running,
}

/// The key that sends the rich-input composer's text to the running
/// program (`rich_input_submit`); ⇧⏎ / ⌥⏎ always insert a newline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RichInputSubmit {
    /// ⏎ sends (the coding agents' own convention).
    #[default]
    Enter,
    /// ⌘⏎ sends, ⏎ inserts a newline.
    CmdEnter,
}

/// One context chip of the Rift prompt (Warp "Edit prompt"). A chip with
/// nothing to show (no repository, no virtualenv) is left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptChip {
    /// Working directory (click: change directory).
    Cwd,
    /// Branch with ahead/behind and the dirty dot (click: change branch).
    GitBranch,
    /// Changed files and lines: `3 • +10 −2`.
    GitDiff,
    User,
    Host,
    /// Python virtualenv, else the version `python3` runs (click: switch
    /// this shell's version).
    Python,
    /// The version `php` runs (click: switch this shell's version).
    Php,
    /// The checked-out branch's pull request, via `gh` (click: open it).
    GithubPr,
    /// The Docker Compose project around the cwd and its running containers
    /// (click: list them).
    Docker,
    Date,
    /// 12-hour time (`03:48 pm`).
    Time12,
    /// 24-hour time (`15:48`).
    Time24,
    /// Time since the tab's first command (`2h 05m`); hidden until then.
    Elapsed,
}

impl PromptChip {
    pub const ALL: [PromptChip; 13] = [
        PromptChip::User,
        PromptChip::Host,
        PromptChip::Cwd,
        PromptChip::GitBranch,
        PromptChip::GitDiff,
        PromptChip::GithubPr,
        PromptChip::Python,
        PromptChip::Php,
        PromptChip::Docker,
        PromptChip::Date,
        PromptChip::Time12,
        PromptChip::Time24,
        PromptChip::Elapsed,
    ];

    /// The default prompt: the design's cwd and branch chips, then the diff
    /// stats (Warp's default; hidden on a clean tree).
    pub fn defaults() -> Vec<PromptChip> {
        vec![
            PromptChip::Elapsed,
            PromptChip::Cwd,
            PromptChip::GitBranch,
            PromptChip::GitDiff,
            PromptChip::Docker,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            PromptChip::Cwd => "Working directory",
            PromptChip::GitBranch => "Git branch",
            PromptChip::GitDiff => "Git diff stats",
            PromptChip::User => "User",
            PromptChip::Host => "Host",
            PromptChip::Python => "Python environment",
            PromptChip::Php => "PHP version",
            PromptChip::GithubPr => "GitHub pull request",
            PromptChip::Docker => "Docker Compose",
            PromptChip::Date => "Date",
            PromptChip::Time12 => "Time (12-hour)",
            PromptChip::Time24 => "Time (24-hour)",
            PromptChip::Elapsed => "Time on this tab",
        }
    }

    /// What the chip shows and what a click on it does (its hover card).
    pub fn about(self) -> &'static str {
        match self {
            PromptChip::Cwd => {
                "The pane's working directory. Click to move into a subdirectory or up to a parent."
            }
            PromptChip::GitBranch => {
                "The checked-out branch, commits ahead (↑) and behind (↓) its upstream, and • \
                 when there are uncommitted changes. Click to switch to another branch or \
                 create one."
            }
            PromptChip::GitDiff => {
                "Files changed in the working tree, and the lines added and removed. Click to \
                 open the code review panel."
            }
            PromptChip::User => "The user you are logged in as.",
            PromptChip::Host => "This machine's host name.",
            PromptChip::Python => {
                "The active virtualenv, else the Python version python3 runs here. Click to \
                 switch this shell's Python (pyenv versions and installed interpreters)."
            }
            PromptChip::Php => {
                "The PHP version php runs here. Click to switch this shell's PHP between the \
                 installed versions."
            }
            PromptChip::GithubPr => {
                "The pull request of the checked-out branch, from the GitHub CLI (gh). Click \
                 to open it in your browser."
            }
            PromptChip::Docker => {
                "The Docker Compose project around this directory and how many of its \
                 containers run. Click to see each container's ports, uptime and health, \
                 and to follow a service's logs."
            }
            PromptChip::Date => "Today's date.",
            PromptChip::Time12 => "The current time, 12-hour.",
            PromptChip::Time24 => "The current time, 24-hour.",
            PromptChip::Elapsed => {
                "The time worked on this tab. Shows once a command has run. By default it \
                 counts from the first command and restarts with each new shell; with \
                 elapsed_counts = \"running\" it counts only while a command runs in the tab \
                 (overlapping panes once) and is kept across an update's restart."
            }
        }
    }
}

/// Where the tabs sit: the top strip, or a vertical rail on the left that
/// also shows each tab's directory and git branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TabLayout {
    #[default]
    Horizontal,
    Vertical,
}

/// When a horizontal tab shows its × (Warp has only a position setting):
/// on the active and the hovered tab, on every tab, or never (⌘W, the tab
/// menu and the palette still close tabs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TabCloseButton {
    #[default]
    Hover,
    Always,
    Never,
}

/// Where a new tab (⌘T, the + button) lands in the strip: after every tab,
/// or right after the active one (Warp `new_tab_placement`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NewTabPlacement {
    #[default]
    End,
    AfterCurrent,
}

/// What ⌃Tab / ⌃⇧Tab step through: the strip's order, or the tabs last
/// used first (Windows Terminal `tabSwitcherMode`), with a switcher shown
/// while ⌃ is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TabSwitchOrder {
    #[default]
    InOrder,
    Mru,
}

/// Space between blocks (Warp "Compact mode", inverted: Rift's blocks have
/// always sat flush): `normal` puts a blank row above every block header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlockSpacing {
    #[default]
    Compact,
    Normal,
}

/// What a pane does when its program exits (Windows Terminal
/// `closeOnExit`): close (`always`, Rift's behaviour before), stay with its
/// last output and a Restart bar (`never`), or close only on exit code 0
/// (`graceful`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloseOnExit {
    #[default]
    Always,
    Never,
    Graceful,
}

impl CloseOnExit {
    /// Whether a pane whose program ended with `code` (`None`: not known)
    /// closes.
    pub fn closes(self, code: Option<i32>) -> bool {
        match self {
            CloseOnExit::Always => true,
            CloseOnExit::Never => false,
            CloseOnExit::Graceful => code == Some(0),
        }
    }
}

/// Which end of a horizontal tab its × sits at (Warp
/// `tab_close_button_position`; left is the macOS style).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TabCloseSide {
    #[default]
    Right,
    Left,
}

/// When the tab strip shows (Warp "Show the tab bar"): always, only while
/// the window is not full screen, or only while the pointer is at the top
/// edge. A hidden strip gives its height to the panes and slides over them
/// when revealed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TabBarVisibility {
    #[default]
    Always,
    HideFullscreen,
    OnHover,
}

/// A button of the tab strip's toolbar (Warp's configurable toolbar):
/// `toolbar_left` and `toolbar_right` list them in order; an item in
/// neither is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolbarItem {
    /// The file explorer toggle.
    Explorer,
    /// Code review (shows the working tree's `+added −removed`).
    Review,
    /// The notifications inbox.
    Notifications,
    /// The command palette.
    Palette,
    /// Settings.
    Settings,
}

impl ToolbarItem {
    pub const ALL: [ToolbarItem; 5] = [
        ToolbarItem::Explorer,
        ToolbarItem::Review,
        ToolbarItem::Notifications,
        ToolbarItem::Palette,
        ToolbarItem::Settings,
    ];

    /// Its Settings control id (`toolbar.<name>`).
    pub fn setting_id(self) -> &'static str {
        match self {
            ToolbarItem::Explorer => "toolbar.explorer",
            ToolbarItem::Review => "toolbar.review",
            ToolbarItem::Notifications => "toolbar.notifications",
            ToolbarItem::Palette => "toolbar.palette",
            ToolbarItem::Settings => "toolbar.settings",
        }
    }

    /// From its config name (`explorer`, …).
    pub fn from_name(name: &str) -> Option<ToolbarItem> {
        Self::ALL.into_iter().find(|i| {
            toml::Value::try_from(i)
                .ok()
                .is_some_and(|v| v.as_str() == Some(name.trim()))
        })
    }

    pub fn from_setting_id(id: &str) -> Option<ToolbarItem> {
        Self::ALL.into_iter().find(|i| i.setting_id() == id)
    }

    /// Its position stepper's Settings id (`toolbar_order.<name>`).
    pub fn order_id(self) -> &'static str {
        match self {
            ToolbarItem::Explorer => "toolbar_order.explorer",
            ToolbarItem::Review => "toolbar_order.review",
            ToolbarItem::Notifications => "toolbar_order.notifications",
            ToolbarItem::Palette => "toolbar_order.palette",
            ToolbarItem::Settings => "toolbar_order.settings",
        }
    }

    pub fn from_order_id(id: &str) -> Option<ToolbarItem> {
        Self::ALL.into_iter().find(|i| i.order_id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            ToolbarItem::Explorer => "File explorer",
            ToolbarItem::Review => "Code review",
            ToolbarItem::Notifications => "Notifications",
            ToolbarItem::Palette => "Command palette",
            ToolbarItem::Settings => "Settings",
        }
    }
}

/// The app's Dock and window icon (Warp `app_icon`): the "Prompt" design in
/// one of its colourways (`packaging/gen-icon.py` `VARIANTS`, same order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppIcon {
    #[default]
    Default,
    Light,
    Mono,
    Ocean,
    Ember,
    Forest,
    Neon,
}

impl AppIcon {
    pub const ALL: [AppIcon; 7] = [
        AppIcon::Default,
        AppIcon::Light,
        AppIcon::Mono,
        AppIcon::Ocean,
        AppIcon::Ember,
        AppIcon::Forest,
        AppIcon::Neon,
    ];

    /// Its config value and image name (`assets/icon/variants/<name>.png`).
    pub fn name(self) -> &'static str {
        match self {
            AppIcon::Default => "default",
            AppIcon::Light => "light",
            AppIcon::Mono => "mono",
            AppIcon::Ocean => "ocean",
            AppIcon::Ember => "ember",
            AppIcon::Forest => "forest",
            AppIcon::Neon => "neon",
        }
    }

    /// Its tile, chevron and cursor colours (`#rrggbb`), as
    /// `packaging/gen-icon.py` draws them: Settings' picker previews it.
    pub fn colors(self) -> [&'static str; 3] {
        match self {
            AppIcon::Default => ["#232326", "#569cd6", "#d4d4d4"],
            AppIcon::Light => ["#f5f5f7", "#1f6fcc", "#2b2b2e"],
            AppIcon::Mono => ["#161618", "#f0f0f0", "#f0f0f0"],
            AppIcon::Ocean => ["#0f2a44", "#5ec4ff", "#e6f4ff"],
            AppIcon::Ember => ["#2b1610", "#ff8a3d", "#ffe2cc"],
            AppIcon::Forest => ["#12261b", "#4fd18b", "#dff7e9"],
            AppIcon::Neon => ["#1a0f2e", "#ff4fd8", "#4ff0ff"],
        }
    }

    pub fn from_name(name: &str) -> Option<AppIcon> {
        Self::ALL.into_iter().find(|i| i.name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            AppIcon::Default => "Default",
            AppIcon::Light => "Light",
            AppIcon::Mono => "Mono",
            AppIcon::Ocean => "Ocean",
            AppIcon::Ember => "Ember",
            AppIcon::Forest => "Forest",
            AppIcon::Neon => "Neon",
        }
    }
}

/// Which side of the toolbar an item is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarSide {
    Left,
    Right,
    Hidden,
}

/// What renders above the editor: Rift's context chips, or the shell's
/// own prompt (PS1 / starship / p10k) left visible (Warp "Use my shell's
/// prompt").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PromptMode {
    #[default]
    #[serde(alias = "shift", alias = "runway")]
    Rift,
    Shell,
}

/// Terminal bell handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Bell {
    #[default]
    Visual,
    Audible,
    Both,
    None,
}

/// The screen edge the drop-down hotkey window hangs from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyEdge {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

/// How a new window opens: as its saved frame says (`default`), maximized
/// or full screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LaunchMode {
    #[default]
    Default,
    Maximized,
    Fullscreen,
}

/// Window padding per side in logical px (see [`Config::paddings`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Padding {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// The top padding of a pane alone in its tab while `window_padding_top`
/// is unset.
pub const DEFAULT_PADDING_TOP: f32 = 14.0;
/// The padding under a full-screen program's last row while
/// `window_padding_bottom` is unset.
pub const DEFAULT_PADDING_BOTTOM: f32 = 8.0;

/// An f32 written as its shortest decimal (`1.6`, not the f64 widening
/// `1.600000023841858`).
fn short_f32<S: serde::Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(v.to_string().parse().unwrap_or(f64::from(*v)))
}

/// [`short_f32`] for an optional setting (skipped when unset).
fn short_opt_f32<S: serde::Serializer>(v: &Option<f32>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(v) => short_f32(v, s),
        None => s.serialize_none(),
    }
}

/// Toolbar items by name; an unknown one (a typo, or an item of a newer
/// Rift) is skipped with a warning instead of failing the whole file.
fn toolbar_items<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<ToolbarItem>, D::Error> {
    let names = Vec::<String>::deserialize(d)?;
    Ok(names
        .into_iter()
        .filter_map(|n| {
            let item = ToolbarItem::from_name(&n);
            if item.is_none() {
                log::warn!("toolbar item {n:?} is not one Rift knows (skipped)");
            }
            item
        })
        .collect())
}

/// `font_features` entries; one that is not a feature (see
/// [`font_feature`]) is dropped with a warning instead of failing the file.
/// One string is a list of one; any other value is no features.
fn font_features<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let specs = match toml::Value::deserialize(d)? {
        toml::Value::String(s) => vec![s],
        v => lenient_or(v, "font_features", Vec::new).unwrap_or_default(),
    };
    Ok(specs
        .into_iter()
        .filter(|f| {
            let ok = font_feature(f).is_some();
            if !ok {
                log::warn!("font_features entry {f:?} is not an OpenType feature (skipped)");
            }
            ok
        })
        .collect())
}

/// One `font_features` entry as an OpenType tag and value: `"ss01"` and
/// `"+ss01"` turn the feature on (1), `"-calt"` off (0), `"cv05=2"` picks
/// alternate 2. A tag is four ASCII letters or digits.
pub fn font_feature(spec: &str) -> Option<([u8; 4], u16)> {
    let spec = spec.trim();
    let (tag, value) = if let Some(t) = spec.strip_prefix('-') {
        (t, 0)
    } else if let Some((t, v)) = spec.split_once('=') {
        (t.trim(), v.trim().parse().ok()?)
    } else {
        (spec.strip_prefix('+').unwrap_or(spec), 1)
    };
    let tag: [u8; 4] = tag.as_bytes().try_into().ok()?;
    tag.iter()
        .all(u8::is_ascii_alphanumeric)
        .then_some((tag, value))
}

/// `app_icon` by name; an unknown one is the default, with a warning.
fn app_icon<'de, D: serde::Deserializer<'de>>(d: D) -> Result<AppIcon, D::Error> {
    let name = String::deserialize(d)?;
    Ok(AppIcon::from_name(&name).unwrap_or_else(|| {
        log::warn!("app_icon {name:?} is not one Rift has (default used)");
        AppIcon::Default
    }))
}

/// A u8 setting read from any integer, saturating at 0 / 255 (the getter
/// clamps further): an out-of-range number must not reject the whole file.
fn saturating_u8<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    let v = i64::deserialize(d)?;
    Ok(v.clamp(0, 255) as u8)
}

/// A setting whose value has the wrong type, is out of its type's range or
/// is not one of its names falls back to its default with a warning: one
/// bad option must not reject the whole file.
fn lenient_or<'de, D, T>(d: D, key: &str, fallback: impl FnOnce() -> T) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = toml::Value::deserialize(d)?;
    Ok(T::deserialize(value.clone()).unwrap_or_else(|e| {
        log::warn!("{key} = {value}: {e} (default used)");
        fallback()
    }))
}

/// [`lenient_or`] for each such setting, named after it, with
/// `Config::default()`'s value as the fallback.
mod lenient {
    use super::{Config, lenient_or};
    macro_rules! settings {
        ($($field:ident: $t:ty),* $(,)?) => {$(
            pub(super) fn $field<'de, D: serde::Deserializer<'de>>(d: D) -> Result<$t, D::Error> {
                lenient_or(d, stringify!($field), || Config::default().$field)
            }
        )*};
    }
    settings! {
        cursor_style: super::CursorStyle,
        cursor_hollow_unfocused: bool,
        window_padding_left: Option<f32>,
        window_padding_right: Option<f32>,
        window_padding_top: Option<f32>,
        window_padding_bottom: Option<f32>,
        alt_screen_padding: bool,
        font_weight: u16,
        bold_font_weight: u16,
        minimum_contrast: f32,
        bold_is_bright: bool,
        ambiguous_width: super::AmbiguousWidth,
        window_width: f32,
        window_height: f32,
        launch_mode: super::LaunchMode,
        always_on_top: bool,
        snap_to_cells: bool,
        resize_overlay: bool,
        wheel_zoom: bool,
        request_attention: bool,
        large_paste_warning: u32,
        trim_paste: bool,
        hotkey_window_edge: super::HotkeyEdge,
        word_delimiters: String,
    }

    /// `hotkey_window_screen`: a name, or a screen number written bare
    /// (`2` as `"2"`).
    pub(super) fn hotkey_window_screen<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<String, D::Error> {
        use serde::Deserialize;
        match toml::Value::deserialize(d)? {
            toml::Value::Integer(n) => Ok(n.to_string()),
            v => Ok(lenient_or(v, "hotkey_window_screen", || {
                Config::default().hotkey_window_screen
            })
            .unwrap_or_else(|_| Config::default().hotkey_window_screen)),
        }
    }
}

/// User configuration — `~/.config/rift/config.toml`, hot-reloaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub font_family: String,
    #[serde(serialize_with = "short_f32")]
    pub font_size: f32,
    /// Multiplier (1.2, as in Warp; the design spec drew 1.65).
    #[serde(serialize_with = "short_f32")]
    pub line_height: f32,
    #[serde(deserialize_with = "lenient::cursor_style")]
    pub cursor_style: CursorStyle,
    /// The cursor draws hollow while its window or pane does not have the
    /// keyboard focus (off: it keeps `cursor_style`).
    #[serde(deserialize_with = "lenient::cursor_hollow_unfocused")]
    pub cursor_hollow_unfocused: bool,
    pub cursor_blink: bool,
    /// Theme name: builtin (`dark-default`, `catppuccin-mocha`, `tokyo-night`)
    /// or a file stem in `~/.config/rift/themes/`.
    pub theme: String,
    pub scrollback: usize,
    /// Left and right padding (logical px). Getter: `paddings()`.
    #[serde(serialize_with = "short_f32")]
    pub window_padding: f32,
    /// Per-side padding of a pane alone in its tab; unset: `window_padding`
    /// at the left and right, 14 at the top, 8 under a full-screen
    /// program's last row (the command editor sits on the bottom edge).
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "short_opt_f32"
    )]
    #[serde(deserialize_with = "lenient::window_padding_left")]
    pub window_padding_left: Option<f32>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "short_opt_f32"
    )]
    #[serde(deserialize_with = "lenient::window_padding_right")]
    pub window_padding_right: Option<f32>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "short_opt_f32"
    )]
    #[serde(deserialize_with = "lenient::window_padding_top")]
    pub window_padding_top: Option<f32>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "short_opt_f32"
    )]
    #[serde(deserialize_with = "lenient::window_padding_bottom")]
    pub window_padding_bottom: Option<f32>,
    /// Padding around a full-screen program (alternate screen: vim, htop);
    /// off: its grid fills the pane edge to edge.
    #[serde(deserialize_with = "lenient::alt_screen_padding")]
    pub alt_screen_padding: bool,
    pub ligatures: bool,
    /// Weight of regular terminal text, 100–900 (400 normal). The font
    /// draws the nearest weight it has.
    #[serde(deserialize_with = "lenient::font_weight")]
    pub font_weight: u16,
    /// Weight of bold (SGR 1) terminal text, 100–900 (700 bold).
    #[serde(deserialize_with = "lenient::bold_font_weight")]
    pub bold_font_weight: u16,
    /// OpenType features of terminal text (`["ss01", "zero", "-calt"]`;
    /// see [`font_feature`]), applied after `ligatures`.
    #[serde(deserialize_with = "font_features")]
    pub font_features: Vec<String>,
    /// Terminal text keeps at least this contrast ratio against its cell's
    /// background, 1–21 (1 = off; WCAG AA asks 4.5).
    #[serde(serialize_with = "short_f32")]
    #[serde(deserialize_with = "lenient::minimum_contrast")]
    pub minimum_contrast: f32,
    /// Bold text in one of the eight ANSI colours draws in its bright
    /// variant (it stays bold).
    #[serde(deserialize_with = "lenient::bold_is_bright")]
    pub bold_is_bright: bool,
    /// Cells an East Asian Ambiguous character takes (`narrow` 1, `wide` 2).
    #[serde(deserialize_with = "lenient::ambiguous_width")]
    pub ambiguous_width: AmbiguousWidth,
    /// Reopen last session's tabs (with cwds) on launch.
    pub restore_session: bool,
    /// Also save finished blocks (command, status, output) and show them
    /// again on relaunch. Output may contain secrets, so this is opt-in.
    pub save_transcripts: bool,
    /// Shells run in the per-user session service, so an update's restart
    /// or a crash leaves them running and the relaunch reattaches them (on
    /// by default; `durable_sessions = false` keeps shells in the window).
    /// An intentional quit ends them either way.
    pub durable_sessions: bool,
    /// Look for a new release at launch and every 6 hours (`update.rs`).
    pub check_for_updates: bool,
    /// Local control socket: "off", "navigate" (list/status/focus, and
    /// prepare-restart, honored only with the token of an update in flight)
    /// or "full" (also split/open/send-text). See control.rs.
    pub control_api: String,
    /// Paired phones may reach this user's windows through the remote
    /// gateway (`rift-terminal remote --managed`, which the windows start
    /// while this is on; docs/remote-protocol.md). Off by default.
    pub remote_access: bool,
    /// Address the remote gateway listens on (`IP:PORT`; `0.0.0.0` = every
    /// network interface).
    pub remote_listen: String,
    /// Relay the remote gateway also registers at, for phones outside the
    /// local network (`wss://…`; `ws://` only for local relays). Defaults to
    /// the public `DEFAULT_RELAY`; empty: no relay. The relay only ever sees
    /// encrypted traffic.
    pub remote_relay: String,
    /// Reach the gateway from any network without a server: it runs a relay
    /// on this computer behind a Cloudflare quick tunnel (`cloudflared`)
    /// and registers there instead of at `remote_relay`. The tunnel's
    /// address changes on every start, so phones pair again after one.
    pub remote_tunnel: bool,
    /// Rift may keep running after its last window closes (macOS: with a
    /// Rift icon in the menu bar, so it shows): paired phones that may run
    /// commands can open tabs meanwhile: the gateway keeps running and
    /// starts them in the session service, and Rift shows them as tabs when
    /// it opens. Needs `remote_access` and `durable_sessions`. Off by
    /// default. Named `remote_when_closed` before the menu bar icon joined
    /// it (`menu_bar_icon` is no longer read).
    #[serde(alias = "remote_when_closed")]
    pub background_processes: bool,
    pub shell: Option<String>,
    /// The `[[profiles]]` entry (or discovered shell) new tabs and panes
    /// open with; unset: `shell` as before profiles.
    pub default_profile: Option<String>,
    /// The executable shells in /etc/shells appear as profiles named after
    /// them (`zsh`, `fish`); a `[[profiles]]` entry of the same name wins.
    pub discover_shells: bool,
    /// Theme used while the OS is in light mode when `sync_os_theme` is on.
    pub theme_light: String,
    /// Follow the OS light/dark appearance (`theme` = dark, `theme_light` = light).
    pub sync_os_theme: bool,
    /// Window background opacity, 0.01–1.0 (Warp: 1–100 %).
    #[serde(serialize_with = "short_f32")]
    pub window_opacity: f32,
    /// macOS background blur radius behind the window, 1–64 px (Warp:
    /// default 1, i.e. none). Visible only below full opacity.
    #[serde(deserialize_with = "saturating_u8")]
    pub window_blur: u8,
    /// Size of a window opened without a saved frame (logical px).
    /// Getter: `window_size()`.
    #[serde(serialize_with = "short_f32")]
    #[serde(deserialize_with = "lenient::window_width")]
    pub window_width: f32,
    #[serde(serialize_with = "short_f32")]
    #[serde(deserialize_with = "lenient::window_height")]
    pub window_height: f32,
    /// How new windows open: `default` (as saved), `maximized`, `fullscreen`.
    #[serde(deserialize_with = "lenient::launch_mode")]
    pub launch_mode: LaunchMode,
    /// New windows float above other apps' windows.
    #[serde(deserialize_with = "lenient::always_on_top")]
    pub always_on_top: bool,
    /// Window resizes snap to whole terminal cells.
    #[serde(deserialize_with = "lenient::snap_to_cells")]
    pub snap_to_cells: bool,
    /// Show the grid's columns × rows while the window is resized.
    #[serde(deserialize_with = "lenient::resize_overlay")]
    pub resize_overlay: bool,
    /// ⌘ + wheel (Ctrl + wheel on Linux) zooms the font; with ⇧ it changes
    /// the window opacity.
    #[serde(deserialize_with = "lenient::wheel_zoom")]
    pub wheel_zoom: bool,
    pub input_position: InputPosition,
    pub blocks_start: BlocksStart,
    pub tab_layout: TabLayout,
    /// When a tab in the top strip shows its close button.
    pub tab_close_button: TabCloseButton,
    /// Which end of a tab the close button is at.
    pub tab_close_side: TabCloseSide,
    /// When the tab strip shows.
    pub tab_bar: TabBarVisibility,
    /// Where ⌘T and the + button put the new tab.
    pub new_tab_placement: NewTabPlacement,
    /// ⌃Tab order: `in_order` (the strip) or `mru` (last used first).
    pub tab_switch_order: TabSwitchOrder,
    /// The Dock icon (macOS, also while Rift is not running) and the window
    /// icon (Linux).
    #[serde(deserialize_with = "app_icon")]
    pub app_icon: AppIcon,
    /// Toolbar buttons left of the tabs, in order. Getter: `toolbar()`.
    #[serde(deserialize_with = "toolbar_items")]
    pub toolbar_left: Vec<ToolbarItem>,
    /// Toolbar buttons at the strip's right end, in order.
    #[serde(deserialize_with = "toolbar_items")]
    pub toolbar_right: Vec<ToolbarItem>,
    /// Show the tabs (the strip's tab, the vertical rail) while only one
    /// tab is open; off leaves the bar's buttons and + alone.
    pub tab_bar_with_one_tab: bool,
    /// Pin the header of a block whose start scrolled off the top of the
    /// active pane (Warp "Show sticky command header").
    pub sticky_command_header: bool,
    /// A blank row above each block header (`normal`) or none.
    pub block_spacing: BlockSpacing,
    /// Offer to load Rift's hooks in a subshell Rift's own hooks do not
    /// reach (`docker exec … bash`, `poetry shell`; Warp "Warpify
    /// subshells").
    pub subshell_offer: bool,
    /// More commands (regular expressions) that start such a subshell
    /// (Warp `added_subshell_commands`).
    pub subshell_commands: Vec<String>,
    /// Commands never to offer it for ("Don't ask again"; Warp
    /// `subshell_commands_denylist`).
    pub subshell_denylist: Vec<String>,
    /// A rule between blocks (Warp "Show block dividers").
    pub block_dividers: bool,
    /// A button that scrolls to the end of a block running past the
    /// bottom of the pane, while the pointer is over it (Warp "Show the
    /// jump to bottom of block button").
    pub jump_to_bottom_button: bool,
    /// The overlay scrollbar on each pane's right edge, with ticks at
    /// prompts, failed commands, bookmarks and find matches.
    pub scrollbar: ScrollbarMode,
    /// Show secrets (tokens, keys, passwords; built-in patterns plus
    /// `secret_patterns`) as `••••`, and mask them in copies, transcripts
    /// and the phone mirror. See `secrets.rs`.
    pub mask_secrets: bool,
    /// Wash the unfocused split panes so the focused one stands out (Warp
    /// "Dim inactive panes").
    pub dim_inactive_panes: bool,
    /// Strength of that wash in percent, 1–40 (half as strong on a light
    /// theme). Getter: `inactive_pane_dim()`.
    #[serde(deserialize_with = "saturating_u8")]
    pub inactive_pane_dim: u8,
    /// The split pane under the pointer takes focus as the pointer moves
    /// (Warp "Focus follows mouse").
    pub focus_follows_mouse: bool,
    /// What a pane does when its program exits.
    pub close_on_exit: CloseOnExit,
    pub prompt: PromptMode,
    /// The Rift prompt's chips, in order (`prompt = "rift"`).
    pub prompt_chips: Vec<PromptChip>,
    /// What the "Time on this tab" chip counts: `always` (since the tab's
    /// first command) or `running` (only while a command runs in the tab).
    pub elapsed_counts: ElapsedCounts,
    /// Copy the selection to the clipboard as soon as the mouse is released.
    pub copy_on_select: bool,
    /// Inline ghost-text suggestion after the cursor.
    pub autosuggestions: bool,
    /// Completion dropdown (Tab still accepts the ghost suggestion).
    pub completions_menu: bool,
    /// Ask the user's own shell (zsh, bash or fish completion functions)
    /// for completions too, and the remote shell over an SSH profile's
    /// shared connection.
    pub shell_completions: bool,
    /// ⌃R, ⌃T and ⌥C run the shell's own widgets where it binds them.
    pub shell_widgets: ShellWidgets,
    /// Automatically insert and remove matching quotes, brackets and braces.
    pub autocomplete_pairs: bool,
    /// Modal (vim) editing in the command editor.
    pub vim_mode: bool,
    /// Vim mode's unnamed register: `"none"` keeps it inside Rift,
    /// `"unnamed"` makes it the system clipboard (vim's `clipboard=unnamed`).
    pub vim_clipboard: String,
    /// Alert (toast, and a desktop notification while the window is not
    /// focused) when a long command finishes.
    pub notify_on_completion: bool,
    /// Minimum command duration (seconds) for a completion notification.
    pub notify_threshold_secs: u64,
    /// Show programs' OSC 9;4 progress reports: a bar under the tab and the
    /// Dock tile badge (macOS).
    pub show_progress: bool,
    pub bell: Bell,
    /// A bell or an alert while no Rift window has focus bounces the Dock
    /// icon once (macOS) or sets the window's urgency hint (Linux).
    #[serde(deserialize_with = "lenient::request_attention")]
    pub request_attention: bool,
    /// Warn before closing a pane, tab or window (or quitting) while a
    /// command is still running in it.
    pub confirm_close_running: bool,
    /// Ask before pasting text containing a newline into a raw program that
    /// has not enabled bracketed paste (each newline would press Enter).
    pub confirm_multiline_paste: bool,
    /// Ask before a paste larger than this many KiB; 0: never.
    #[serde(deserialize_with = "lenient::large_paste_warning")]
    pub large_paste_warning: u32,
    /// Drop trailing whitespace and newlines from pasted text (a copied
    /// line no longer runs when pasted).
    #[serde(deserialize_with = "lenient::trim_paste")]
    pub trim_paste: bool,
    /// Where new tabs and panes start: `"previous"` (the active pane's
    /// directory), `"home"`, or an absolute path (`~` expands).
    pub working_directory: String,
    /// Expand a shell alias inline when Space is typed right after it.
    pub alias_expansion: bool,
    /// Rich input (the composer over a running program): the key that
    /// sends its text, `"enter"` or `"cmd_enter"`.
    pub rich_input_submit: RichInputSubmit,
    /// Rich input opens by itself when a coding agent in a pane waits for
    /// input (its own notification, or a second of quiet after output).
    pub rich_input_auto: bool,
    /// Underline an unknown command name in the editor (error highlighting).
    pub error_underline: bool,
    /// Suggest a local fix after a failed "command not found".
    pub command_corrections: bool,
    /// System-wide show/hide shortcut (e.g. `"alt+space"`); empty = off.
    pub global_hotkey: String,
    /// The global hotkey shows and hides a drop-down window at the top of
    /// the screen (Warp's dedicated hotkey window) instead of the whole app.
    pub hotkey_window: bool,
    /// Height of the drop-down window in percent of the screen, 20–100.
    /// Getter: `hotkey_window_height()`.
    #[serde(deserialize_with = "saturating_u8")]
    pub hotkey_window_height: u8,
    /// The screen edge it hangs from: `top`, `bottom`, `left`, `right`.
    /// At the left or right edge `hotkey_window_height` is its width.
    #[serde(deserialize_with = "lenient::hotkey_window_edge")]
    pub hotkey_window_edge: HotkeyEdge,
    /// Its size along that edge in percent of the screen, 20–100, centred.
    /// Getter: `hotkey_window_width()`.
    #[serde(deserialize_with = "saturating_u8")]
    pub hotkey_window_width: u8,
    /// The screen it drops onto: `pointer` (the one with the mouse
    /// pointer), `primary`, or a screen's number (`1` is the primary).
    #[serde(deserialize_with = "lenient::hotkey_window_screen")]
    pub hotkey_window_screen: String,
    /// The drop-down window hides when it loses keyboard focus.
    pub hotkey_window_hide_on_blur: bool,
    /// macOS: Rift opens when you log in (a login item).
    pub launch_at_login: bool,
    /// macOS: the Option key sends Meta (ESC prefix, for emacs/readline
    /// word motion) instead of composing characters like `∫`. Linux Alt is
    /// always Meta.
    pub option_as_meta: bool,
    /// macOS: the right Option key's Meta setting; unset = same as
    /// `option_as_meta` (which then covers only the left key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_option_as_meta: Option<bool>,
    /// Middle-click pastes the clipboard into the pane under the pointer
    /// (unless the program there takes mouse clicks).
    pub middle_click_paste: bool,
    /// Lines one mouse-wheel notch scrolls the terminal (1–20; trackpads
    /// scroll by pixels).
    pub wheel_scroll_lines: u8,
    /// OSC 52 clipboard access for programs: `"off"`, `"write"` (copy only)
    /// or `"read_write"` (also read it back: any program or remote host can
    /// then read your clipboard).
    pub clipboard_access: String,
    /// Forward mouse clicks, drags and the wheel to apps that enable mouse
    /// reporting (vim, tmux, htop). Hold Shift to select text instead.
    /// Focus in/out reports (mode 1004) do not depend on it.
    pub mouse_reporting: bool,
    /// Extra characters a double-click counts as part of a word (Warp's
    /// `word_char_allowlist`), e.g. `"-./"`. Empty: the default word
    /// boundaries.
    pub word_chars: String,
    /// Characters that also end a word, e.g. `"/.-"`: for a double-click
    /// and for the command editor's word moves and deletes (⌥← ⌥→ ⌥⌫),
    /// which otherwise stop at blanks only.
    #[serde(deserialize_with = "lenient::word_delimiters")]
    pub word_delimiters: String,
    /// Where ⌘-clicked file paths and file links open: `"system"` (the
    /// file's default app), `"rift"` (the viewer pane) or an editor id
    /// from `editors::EDITORS` (`"vscode"`, `"zed"`, …).
    pub link_editor: String,
    /// What a plain click on a link does: `"prompt"` shows the link and
    /// that ⌘-click opens it, `"open"` opens it at once. ⌘-click always
    /// opens.
    pub link_click: String,
    /// Where files from the explorer and code review (↗) open: `"rift"`
    /// (the viewer pane), `"system"` or an editor id.
    pub file_editor: String,
    /// The file explorer lists dotfiles (`.env`, `.github/`); `.git` and
    /// git-ignored entries stay hidden either way.
    pub explorer_hidden_files: bool,
    /// Toasts, desktop notifications and sounds per event (`[notifications]`;
    /// tables stay after the plain keys so the TOML writer accepts them).
    pub notifications: crate::notifications::Notifications,
    /// Named ssh hosts (`[[ssh_profiles]]`); ~/.ssh/config hosts are added
    /// to the palette automatically.
    pub ssh_profiles: Vec<crate::ssh::SshProfile>,
    /// Reusable launch settings (`[[profiles]]`): program, arguments,
    /// directory, environment, tab title and colour. See `profiles.rs`.
    pub profiles: Vec<crate::profiles::Profile>,
    /// Named step sequences (`[[actions]]`): palette rows and chords. See
    /// `actions.rs`.
    pub actions: Vec<crate::actions::UserAction>,
    /// More secret patterns for `mask_secrets` (`[[secret_patterns]]`:
    /// `name`, `regex`).
    pub secret_patterns: Vec<crate::secrets::SecretPattern>,
    /// Directory rules for tab colours (`[[tab_color_rules]]`: `dir`,
    /// `color`); a tab's own colour wins. See `tab_color.rs`.
    pub tab_color_rules: Vec<crate::tab_color::TabColorRule>,
    /// Overrides: action name → binding (e.g. `new_tab = "ctrl+shift+t"`).
    pub keybindings: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            font_family: "Hack".into(),
            font_size: 13.0,
            line_height: 1.2,
            cursor_style: CursorStyle::Block,
            cursor_hollow_unfocused: false,
            cursor_blink: true,
            theme: "dark-default".into(),
            scrollback: 10_000,
            window_padding: 8.0,
            window_padding_left: None,
            window_padding_right: None,
            window_padding_top: None,
            window_padding_bottom: None,
            alt_screen_padding: true,
            ligatures: true,
            font_weight: 400,
            bold_font_weight: 700,
            font_features: Vec::new(),
            minimum_contrast: 1.0,
            bold_is_bright: false,
            ambiguous_width: AmbiguousWidth::Narrow,
            restore_session: true,
            save_transcripts: false,
            durable_sessions: true,
            check_for_updates: true,
            control_api: "navigate".into(),
            remote_access: false,
            remote_listen: "0.0.0.0:7878".into(),
            remote_relay: DEFAULT_RELAY.into(),
            remote_tunnel: false,
            background_processes: false,
            shell: None,
            default_profile: None,
            discover_shells: true,
            theme_light: "light-default".into(),
            sync_os_theme: false,
            window_opacity: 0.5,
            window_blur: 20,
            window_width: 1180.0,
            window_height: 760.0,
            launch_mode: LaunchMode::Default,
            always_on_top: false,
            snap_to_cells: false,
            resize_overlay: false,
            wheel_zoom: false,
            input_position: InputPosition::Bottom,
            blocks_start: BlocksStart::Bottom,
            tab_layout: TabLayout::Horizontal,
            tab_close_button: TabCloseButton::Hover,
            tab_close_side: TabCloseSide::Right,
            tab_bar: TabBarVisibility::Always,
            new_tab_placement: NewTabPlacement::End,
            tab_switch_order: TabSwitchOrder::InOrder,
            app_icon: AppIcon::Default,
            toolbar_left: vec![ToolbarItem::Explorer],
            toolbar_right: vec![ToolbarItem::Notifications, ToolbarItem::Review],
            tab_bar_with_one_tab: true,
            sticky_command_header: true,
            block_spacing: BlockSpacing::Compact,
            subshell_offer: true,
            subshell_commands: Vec::new(),
            subshell_denylist: Vec::new(),
            block_dividers: false,
            jump_to_bottom_button: true,
            scrollbar: ScrollbarMode::Auto,
            mask_secrets: false,
            dim_inactive_panes: true,
            inactive_pane_dim: 10,
            focus_follows_mouse: false,
            close_on_exit: CloseOnExit::Always,
            prompt: PromptMode::Rift,
            prompt_chips: PromptChip::defaults(),
            elapsed_counts: ElapsedCounts::Always,
            copy_on_select: true,
            autosuggestions: true,
            completions_menu: true,
            shell_completions: true,
            shell_widgets: ShellWidgets::Auto,
            autocomplete_pairs: true,
            vim_mode: false,
            vim_clipboard: "none".into(),
            notify_on_completion: true,
            notify_threshold_secs: 10,
            show_progress: true,
            bell: Bell::Both,
            request_attention: false,
            confirm_close_running: true,
            confirm_multiline_paste: true,
            large_paste_warning: 0,
            trim_paste: false,
            working_directory: "previous".into(),
            alias_expansion: false,
            rich_input_submit: RichInputSubmit::Enter,
            rich_input_auto: false,
            error_underline: true,
            command_corrections: true,
            global_hotkey: String::new(),
            hotkey_window: false,
            hotkey_window_height: 40,
            hotkey_window_edge: HotkeyEdge::Top,
            hotkey_window_width: 100,
            hotkey_window_screen: "pointer".into(),
            hotkey_window_hide_on_blur: true,
            launch_at_login: false,
            option_as_meta: false,
            right_option_as_meta: None,
            middle_click_paste: true,
            wheel_scroll_lines: 3,
            clipboard_access: "write".into(),
            mouse_reporting: true,
            word_chars: String::new(),
            word_delimiters: String::new(),
            link_editor: crate::editors::RIFT.into(),
            link_click: "open".into(),
            file_editor: crate::editors::RIFT.into(),
            explorer_hidden_files: true,
            notifications: Default::default(),
            ssh_profiles: Vec::new(),
            profiles: Vec::new(),
            actions: Vec::new(),
            secret_patterns: Vec::new(),
            tab_color_rules: Vec::new(),
            keybindings: BTreeMap::new(),
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    config_dir_from(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

/// `config_dir` over explicit env values; an empty `XDG_CONFIG_HOME` counts
/// as unset (a relative path would put the config in the working directory).
fn config_dir_from(
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    xdg.filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|h| PathBuf::from(h).join(".config")))
        .map(|root| app_dir(&root))
}

/// Rift's directory under a config or data root: `rift`, or the directory of
/// an earlier name of the app (`shift`, then `runway`) while `rift` does not
/// exist.
pub(crate) fn app_dir(root: &Path) -> PathBuf {
    let current = root.join("rift");
    if current.exists() {
        return current;
    }
    ["shift", "runway"]
        .into_iter()
        .map(|name| root.join(name))
        .find(|legacy| legacy.exists())
        .unwrap_or(current)
}

/// Expand a leading `~` or `~/`; `~root` stays a literal name.
fn expand_tilde(path: &str, home: Option<&Path>) -> PathBuf {
    match home {
        Some(h) if path == "~" => h.to_path_buf(),
        Some(h) if path.starts_with("~/") => h.join(path[2..].trim_start_matches('/')),
        _ => PathBuf::from(path),
    }
}

/// The public relay `remote_relay` defaults to: phones reach a computer from
/// any network with nothing to set up. It forwards only Noise ciphertext.
pub const DEFAULT_RELAY: &str = "wss://rift-relay.playhaws.com";

/// Set in a scripted run (and inherited by the gateway it starts): the
/// default relay is not used, so tests never reach the public relay. A relay
/// the config names explicitly still is.
pub const NO_DEFAULT_RELAY_ENV: &str = "RIFT_NO_DEFAULT_RELAY";

fn relay_of(setting: &str, no_default: bool) -> Option<&str> {
    let relay = setting.trim();
    (!relay.is_empty() && !(no_default && relay == DEFAULT_RELAY)).then_some(relay)
}

/// Top-level `Option` settings: absent from a serialized default, so
/// `merge_table` cannot tell them from unknown keys without this list.
const OPTIONAL_KEYS: &[&str] = &[
    "shell",
    "default_profile",
    "right_option_as_meta",
    "window_padding_left",
    "window_padding_right",
    "window_padding_top",
    "window_padding_bottom",
];

/// Make `doc` hold `new` (see `Config::edit_toml`). `default` is the same
/// table for a default config; an empty `default` marks a map such as
/// `[keybindings]`, whose entries are exactly those of `new`.
fn merge_table(
    doc: &mut dyn toml_edit::TableLike,
    new: &toml::Table,
    default: &toml::Table,
    optional: &[&str],
) {
    let is_map = default.is_empty();
    let stale: Vec<String> = doc
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| {
            !new.contains_key(k)
                && (is_map || default.contains_key(k) || optional.contains(&k.as_str()))
        })
        .collect();
    for k in stale {
        doc.remove(&k);
    }
    let empty = toml::Table::new();
    for (k, v) in new {
        let dv = default.get(k);
        let sub_default = dv.and_then(toml::Value::as_table).unwrap_or(&empty);
        match (doc.get_mut(k), v) {
            (Some(item), toml::Value::Table(nt)) if item.is_table_like() => {
                if let Some(t) = item.as_table_like_mut() {
                    merge_table(t, nt, sub_default, &[]);
                }
            }
            (Some(item), _) => {
                if plain_value(k, item).as_ref() != Some(v)
                    && let Some(mut fresh) = edit_item(k, v)
                {
                    // keep the value's trailing comment and spacing
                    if let (Some(old), Some(fresh_v)) = (item.as_value(), fresh.as_value_mut()) {
                        *fresh_v.decor_mut() = old.decor().clone();
                    }
                    *item = fresh;
                }
            }
            (None, toml::Value::Table(nt)) => {
                let mut t = toml_edit::Table::new();
                merge_table(&mut t, nt, sub_default, &[]);
                if !t.is_empty() {
                    doc.insert(k, toml_edit::Item::Table(t));
                }
            }
            (None, _) => {
                if dv != Some(v)
                    && let Some(fresh) = edit_item(k, v)
                {
                    doc.insert(k, fresh);
                }
            }
        }
    }
}

/// `item` (stored under `key`) as a plain TOML value, for comparison.
fn plain_value(key: &str, item: &toml_edit::Item) -> Option<toml::Value> {
    let mut d = toml_edit::DocumentMut::new();
    d.insert(key, item.clone());
    toml::from_str::<toml::Table>(&d.to_string())
        .ok()?
        .remove(key)
}

/// `value` as an editable item, formatted like `toml::to_string_pretty`.
fn edit_item(key: &str, value: &toml::Value) -> Option<toml_edit::Item> {
    let mut t = toml::Table::new();
    t.insert(key.to_string(), value.clone());
    let mut d: toml_edit::DocumentMut = toml::to_string_pretty(&t).ok()?.parse().ok()?;
    d.remove(key)
}

pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

/// `path` for display, with the home directory shortened to `~`.
pub fn tilde_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

pub fn themes_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("themes"))
}

impl Config {
    /// The relay the gateway registers at: `remote_relay` trimmed, `None`
    /// when empty, and `None` for the default relay under
    /// `RIFT_NO_DEFAULT_RELAY`.
    pub fn relay(&self) -> Option<&str> {
        relay_of(
            &self.remote_relay,
            std::env::var_os(NO_DEFAULT_RELAY_ENV).is_some(),
        )
    }

    /// Read `config.toml`. `Ok(None)`: no file yet (first run, defaults
    /// apply). `Err`: the file exists but cannot be read or parsed — the
    /// caller keeps its last good config rather than resetting to defaults,
    /// which would silently re-enable `control_api`.
    pub fn try_load() -> Result<Option<Config>, String> {
        match config_path() {
            Some(path) => Self::try_load_from(&path),
            None => Ok(None),
        }
    }

    pub fn try_load_from(path: &Path) -> Result<Option<Config>, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str::<Config>(&s)
                .map_err(|e| e.to_string())
                .and_then(|c| {
                    // an invalid `[[profiles]]` entry rejects the file like
                    // a parse error: the last good config stays
                    crate::profiles::validate(&c.profiles)?;
                    // so does an invalid or cyclic `[[actions]]` entry
                    crate::actions::validate(&c.actions, &crate::keybindings::all_actions())?;
                    crate::secrets::validate(&c.secret_patterns)?;
                    Ok(Some(c.migrated()))
                })
                .map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Values written under an earlier name of the app: the viewer pane was
    /// `"shift"` before it was `editors::RIFT`.
    fn migrated(mut self) -> Config {
        for editor in [&mut self.link_editor, &mut self.file_editor] {
            if editor.trim() == "shift" {
                *editor = crate::editors::RIFT.into();
            }
        }
        self
    }

    /// Settings for a config file that exists but is invalid at startup:
    /// defaults, except automation stays off until the file is fixed.
    pub fn fallback() -> Config {
        Config {
            control_api: "off".into(),
            ..Config::default()
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = config_path() else {
            return Err(std::io::Error::other("no config dir"));
        };
        self.save_to(&path)
    }

    /// Edit the file in place (see `edit_toml`), then replace it atomically:
    /// write a sibling temp file, flush it, keep the old file's
    /// permissions, rename over the target. A failed write never truncates
    /// the existing config, and the hot-reload watcher never reads a
    /// half-written file.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        // a symlinked config.toml (dotfiles, stow) is edited through the
        // link: rename onto the link itself would replace it with a file
        let resolved;
        let path = match std::fs::symlink_metadata(path) {
            Ok(m) if m.file_type().is_symlink() => {
                resolved = std::fs::canonicalize(path)?;
                resolved.as_path()
            }
            _ => path,
        };
        let dir = path
            .parent()
            .ok_or_else(|| std::io::Error::other("config path has no parent"))?;
        std::fs::create_dir_all(dir)?;
        let existing = match std::fs::read_to_string(path) {
            Ok(s) => Some(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        let body = self
            .edit_toml(existing.as_deref())
            .map_err(std::io::Error::other)?;
        let tmp = dir.join(format!(".config.toml.tmp-{}", std::process::id()));
        let result = (|| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(body.as_bytes())?;
            f.sync_all()?;
            if let Ok(meta) = std::fs::metadata(path) {
                std::fs::set_permissions(&tmp, meta.permissions())?;
            }
            std::fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// The user's file (`existing`) changed to hold this config. A changed
    /// value is rewritten in place, keeping its comments; a setting the
    /// file never mentioned is added only when it differs from the
    /// default; comments, order and unknown keys stay. A file that does not
    /// parse is never rewritten. Should the edit not read back as exactly
    /// this config, the whole file is written instead.
    pub fn edit_toml(&self, existing: Option<&str>) -> Result<String, String> {
        let mut doc: toml_edit::DocumentMut = existing
            .unwrap_or("")
            .parse()
            .map_err(|e| format!("config.toml does not parse, not overwriting it: {e}"))?;
        // a file of only comments keeps them on top, above the new settings
        let header = if doc.as_table().is_empty() {
            let text = doc.trailing().as_str().unwrap_or_default().to_string();
            doc.set_trailing("");
            text
        } else {
            String::new()
        };
        let table = |c: &Config| match toml::Value::try_from(c) {
            Ok(toml::Value::Table(t)) => Ok(t),
            Ok(_) => Err("config is not a table".to_string()),
            Err(e) => Err(e.to_string()),
        };
        merge_table(
            doc.as_table_mut(),
            &table(self)?,
            &table(&Config::default())?,
            OPTIONAL_KEYS,
        );
        let out = format!("{header}{doc}");
        // compared as TOML text: an accepted `nan` never equals itself as
        // f32, and the lossy fallback would drop comments and unknown keys
        let same = |back: &Config| match (toml::to_string(back), toml::to_string(self)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        match toml::from_str::<Config>(&out) {
            Ok(back) if same(&back) => Ok(out),
            _ => toml::to_string_pretty(self).map_err(|e| e.to_string()),
        }
    }

    /// Font size in points: non-finite falls back to the default, then
    /// clamps to the supported range.
    pub fn font_size(&self) -> f32 {
        if self.font_size.is_finite() {
            self.font_size.clamp(8.0, 28.0)
        } else {
            13.0
        }
    }

    /// Line-height multiplier, validated like `font_size()`.
    pub fn line_height(&self) -> f32 {
        if self.line_height.is_finite() {
            self.line_height.clamp(1.0, 2.2)
        } else {
            1.2
        }
    }

    /// Resolve the configured theme: builtin name or user theme file.
    pub fn resolve_theme(&self) -> crate::Theme {
        crate::theme::load_by_name(&self.theme)
    }

    /// Theme for the current OS appearance: `theme_light` while the OS is
    /// light and `sync_os_theme` is on, else `theme`.
    pub fn resolve_theme_for(&self, os_light: bool) -> crate::Theme {
        if self.sync_os_theme && os_light {
            crate::theme::load_by_name(&self.theme_light)
        } else {
            self.resolve_theme()
        }
    }

    /// Start directory for a new tab or pane. `previous` is the directory of
    /// the pane the user was in; a configured path that does not exist falls
    /// back to home.
    pub fn start_dir(&self, previous: Option<&str>) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let setting = self.working_directory.trim();
        let dir = match setting {
            "" | "previous" => previous.map(PathBuf::from).or(home.clone()),
            "home" => home.clone(),
            path => Some(expand_tilde(path, home.as_deref())),
        };
        dir.filter(|d| d.is_dir()).or(home)
    }

    /// Horizontal window padding (logical px) clamped to 0–40.
    pub fn padding(&self) -> f32 {
        if self.window_padding.is_finite() {
            self.window_padding.clamp(0.0, 40.0)
        } else {
            8.0
        }
    }

    /// Regular and bold text weights, each clamped to 100–900.
    pub fn font_weights(&self) -> (u16, u16) {
        (
            self.font_weight.clamp(100, 900),
            self.bold_font_weight.clamp(100, 900),
        )
    }

    /// `font_features` as OpenType tags and values, in order (a later entry
    /// for the same tag wins when they are applied).
    pub fn font_feature_tags(&self) -> Vec<([u8; 4], u16)> {
        self.font_features
            .iter()
            .filter_map(|f| font_feature(f))
            .collect()
    }

    /// `minimum_contrast` clamped to 1–21 (1 when not a number).
    pub fn min_contrast(&self) -> f32 {
        if self.minimum_contrast.is_finite() {
            self.minimum_contrast.clamp(1.0, 21.0)
        } else {
            1.0
        }
    }

    /// Padding per side of a pane alone in its tab, each clamped to 0–40;
    /// `alt_screen`: the pane shows a full-screen program, which gets none
    /// with `alt_screen_padding` off.
    pub fn paddings(&self, alt_screen: bool) -> Padding {
        if alt_screen && !self.alt_screen_padding {
            return Padding {
                left: 0.0,
                right: 0.0,
                top: 0.0,
                bottom: 0.0,
            };
        }
        let side = |v: Option<f32>, default: f32| match v {
            Some(v) if v.is_finite() => v.clamp(0.0, 40.0),
            _ => default,
        };
        Padding {
            left: side(self.window_padding_left, self.padding()),
            right: side(self.window_padding_right, self.padding()),
            top: side(self.window_padding_top, DEFAULT_PADDING_TOP),
            bottom: side(self.window_padding_bottom, DEFAULT_PADDING_BOTTOM),
        }
    }

    /// `window_width` × `window_height`, each at least 320 × 200 and at
    /// most 16384 (the default for a value that is not a number).
    pub fn window_size(&self) -> (f64, f64) {
        let dim = |v: f32, min: f64, default: f64| {
            if v.is_finite() {
                f64::from(v).clamp(min, 16384.0)
            } else {
                default
            }
        };
        (
            dim(self.window_width, 320.0, 1180.0),
            dim(self.window_height, 200.0, 760.0),
        )
    }

    /// Paste size above which Rift asks first, in bytes; `None`: never.
    pub fn large_paste_bytes(&self) -> Option<usize> {
        (self.large_paste_warning > 0).then(|| self.large_paste_warning as usize * 1024)
    }

    /// Drop-down window size along its edge, clamped to 20–100 %.
    pub fn hotkey_window_width(&self) -> u8 {
        self.hotkey_window_width.clamp(20, 100)
    }

    /// The screen the drop-down window uses: `None` for the pointer's,
    /// else a 0-based screen index (0 is the primary). An unknown value is
    /// the pointer's screen.
    pub fn hotkey_screen(&self) -> Option<usize> {
        match self.hotkey_window_screen.trim() {
            "" | "pointer" => None,
            "primary" => Some(0),
            n => match n.parse::<usize>() {
                Ok(n) if n >= 1 => Some(n - 1),
                _ => {
                    log::warn!("hotkey_window_screen {n:?} is not a screen (pointer used)");
                    None
                }
            },
        }
    }

    /// Scrollback lines clamped to 0–1,000,000 (a hand-edited file can hold
    /// any `usize`, and the terminal allocates per line).
    pub fn scrollback_lines(&self) -> usize {
        self.scrollback.min(1_000_000)
    }

    /// Terminal lines per wheel notch, clamped to 1–20.
    pub fn wheel_lines(&self) -> f32 {
        self.wheel_scroll_lines.clamp(1, 20) as f32
    }

    /// Drop-down window height in percent of the screen, clamped to 20–100.
    pub fn hotkey_window_height(&self) -> u8 {
        self.hotkey_window_height.clamp(20, 100)
    }

    /// The right Option key sends Meta (macOS).
    pub fn right_option_meta(&self) -> bool {
        self.right_option_as_meta.unwrap_or(self.option_as_meta)
    }

    /// Window opacity clamped to the supported range. The floor is Warp's
    /// 1 %: at 0 Metal shows the previous frame's pixels.
    pub fn opacity(&self) -> f32 {
        if self.window_opacity.is_finite() {
            self.window_opacity.clamp(0.01, 1.0)
        } else {
            1.0
        }
    }

    /// Background blur radius clamped to Warp's 1–64 px.
    pub fn blur(&self) -> u8 {
        self.window_blur.clamp(1, 64)
    }

    /// `inactive_pane_dim` as a wash alpha for a dark theme (a light theme
    /// takes half of it); 0 while `dim_inactive_panes` is off.
    pub fn inactive_pane_dim(&self) -> f32 {
        if self.dim_inactive_panes {
            f32::from(self.inactive_pane_dim.clamp(1, 40)) / 100.0
        } else {
            0.0
        }
    }

    /// The toolbar's (left, right) buttons: an item listed twice shows
    /// once, where it is listed first (left before right).
    pub fn toolbar(&self) -> (Vec<ToolbarItem>, Vec<ToolbarItem>) {
        let mut seen = std::collections::HashSet::new();
        let mut side = |items: &[ToolbarItem]| -> Vec<ToolbarItem> {
            items.iter().copied().filter(|i| seen.insert(*i)).collect()
        };
        let left = side(&self.toolbar_left);
        (left, side(&self.toolbar_right))
    }

    /// The side `item` shows on.
    pub fn toolbar_side(&self, item: ToolbarItem) -> ToolbarSide {
        let (left, right) = self.toolbar();
        if left.contains(&item) {
            ToolbarSide::Left
        } else if right.contains(&item) {
            ToolbarSide::Right
        } else {
            ToolbarSide::Hidden
        }
    }

    /// `item`'s place on its side (1 = leftmost), `None` while hidden.
    pub fn toolbar_position(&self, item: ToolbarItem) -> Option<usize> {
        let (left, right) = self.toolbar();
        left.iter()
            .chain(right.iter())
            .position(|i| *i == item)
            .map(|p| {
                if p < left.len() {
                    p + 1
                } else {
                    p - left.len() + 1
                }
            })
    }

    /// Move `item` `by` places along its side (negative: left), within it.
    pub fn move_toolbar_item(&mut self, item: ToolbarItem, by: i64) {
        let (mut left, mut right) = self.toolbar();
        for side in [&mut left, &mut right] {
            if let Some(at) = side.iter().position(|i| *i == item) {
                let to = (at as i64 + by).clamp(0, side.len() as i64 - 1) as usize;
                let it = side.remove(at);
                side.insert(to, it);
            }
        }
        self.toolbar_left = left;
        self.toolbar_right = right;
    }

    /// Move `item` to the end of `side` (its order on the other side is
    /// kept for the rest).
    pub fn set_toolbar_side(&mut self, item: ToolbarItem, side: ToolbarSide) {
        if self.toolbar_side(item) == side {
            return;
        }
        let (mut left, mut right) = self.toolbar();
        left.retain(|i| *i != item);
        right.retain(|i| *i != item);
        match side {
            ToolbarSide::Left => left.push(item),
            ToolbarSide::Right => right.push(item),
            ToolbarSide::Hidden => {}
        }
        self.toolbar_left = left;
        self.toolbar_right = right;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsetting_an_optional_key_edits_only_that_line() {
        let existing =
            "# keep\nfuture_option = 1\nright_option_as_meta = true\nshell = \"/bin/sh\"\n";
        let out = Config::default().edit_toml(Some(existing)).unwrap();
        assert_eq!(out, "# keep\nfuture_option = 1\n");
    }

    #[test]
    fn floats_write_short_and_a_huge_blur_still_loads() {
        let c = Config {
            line_height: 1.6,
            window_opacity: 0.51,
            ..Default::default()
        };
        let text = toml::to_string(&c).unwrap();
        assert!(text.contains("line_height = 1.6\n"), "{text}");
        assert!(text.contains("window_opacity = 0.51\n"), "{text}");
        let big: Config = toml::from_str("window_blur = 300\nfont_size = 15.0\n").unwrap();
        assert_eq!(
            (big.window_blur, big.blur(), big.font_size),
            (255, 64, 15.0)
        );
        let neg: Config = toml::from_str("window_blur = -4\n").unwrap();
        assert_eq!(neg.blur(), 1);
    }

    #[test]
    fn background_processes_replaces_the_menu_bar_icon_and_phone_tabs_keys() {
        assert!(!Config::default().background_processes);
        let on: Config = toml::from_str("background_processes = true\n").unwrap();
        assert!(on.background_processes);
        // the key's old name keeps working, and the removed icon key is ignored
        let old: Config =
            toml::from_str("remote_when_closed = true\nmenu_bar_icon = true\n").unwrap();
        assert!(old.background_processes);
        let icon_only: Config = toml::from_str("menu_bar_icon = true\n").unwrap();
        assert!(!icon_only.background_processes);
    }

    #[test]
    fn hotkey_window_defaults_and_height_clamp() {
        let c = Config::default();
        assert_eq!(
            (
                c.hotkey_window,
                c.hotkey_window_height(),
                c.hotkey_window_hide_on_blur
            ),
            (false, 40, true)
        );
        let on: Config =
            toml::from_str("hotkey_window = true\nhotkey_window_height = 65\n").unwrap();
        assert!(on.hotkey_window);
        assert_eq!(on.hotkey_window_height(), 65);
        let low: Config = toml::from_str("hotkey_window_height = 5").unwrap();
        assert_eq!(low.hotkey_window_height(), 20);
        let high: Config = toml::from_str("hotkey_window_height = 900").unwrap();
        assert_eq!(high.hotkey_window_height(), 100);
        let neg: Config = toml::from_str("hotkey_window_height = -1").unwrap();
        assert_eq!(neg.hotkey_window_height(), 20);
    }

    #[test]
    fn hotkey_window_edge_width_and_screen() {
        let c = Config::default();
        assert_eq!(
            (
                c.hotkey_window_edge,
                c.hotkey_window_width(),
                c.hotkey_screen()
            ),
            (HotkeyEdge::Top, 100, None)
        );
        let set: Config = toml::from_str(
            "hotkey_window_edge = \"left\"\nhotkey_window_width = 5\nhotkey_window_screen = \"2\"\n",
        )
        .unwrap();
        assert_eq!(
            (
                set.hotkey_window_edge,
                set.hotkey_window_width(),
                set.hotkey_screen()
            ),
            (HotkeyEdge::Left, 20, Some(1))
        );
        let mut c = Config::default();
        for (v, want) in [
            ("primary", Some(0)),
            ("1", Some(0)),
            ("0", None),
            ("x", None),
        ] {
            c.hotkey_window_screen = v.into();
            assert_eq!(c.hotkey_screen(), want, "{v}");
        }
    }

    #[test]
    fn one_bad_tier3_value_falls_back_alone() {
        let d = Config::default();
        // each line alone is a bad value of its setting; the file still
        // loads, that setting is its default and the next line's is kept
        for bad in [
            "cursor_style = \"triangle\"",
            "cursor_hollow_unfocused = \"yes\"",
            "window_padding_left = \"wide\"",
            "window_padding_top = [1]",
            "alt_screen_padding = 1",
            "font_weight = -1",
            "font_weight = 70000",
            "bold_font_weight = \"bold\"",
            "minimum_contrast = \"high\"",
            "bold_is_bright = 1",
            "ambiguous_width = \"medium\"",
            "window_width = \"big\"",
            "window_height = true",
            "launch_mode = \"maximised\"",
            "always_on_top = \"on\"",
            "snap_to_cells = 0",
            "resize_overlay = \"on\"",
            "wheel_zoom = 1",
            "request_attention = \"y\"",
            "large_paste_warning = -5",
            "large_paste_warning = 1e3",
            "trim_paste = \"y\"",
            "hotkey_window_edge = \"middle\"",
            "hotkey_window_screen = true",
            "word_delimiters = 5",
            "font_features = 5",
        ] {
            let c: Config = toml::from_str(&format!("{bad}\nfont_size = 17\n"))
                .unwrap_or_else(|e| panic!("{bad}: {e}"));
            assert_eq!(c.font_size, 17.0, "{bad}");
            let key = bad.split(' ').next().unwrap();
            // (an unset optional setting is absent)
            let back = |c: &Config| toml::Value::try_from(c).unwrap().get(key).cloned();
            let mut want = d.clone();
            want.font_size = 17.0;
            assert_eq!(back(&c), back(&want), "{bad}");
        }
        // a single feature string is a list of one
        let c: Config = toml::from_str("font_features = \"zero\"").unwrap();
        assert_eq!(c.font_features, ["zero"]);
        // a bare screen number is its name
        let c: Config = toml::from_str("hotkey_window_screen = 2").unwrap();
        assert_eq!(c.hotkey_screen(), Some(1));
        // good values still read
        let c: Config = toml::from_str(
            "font_weight = 300\nlaunch_mode = \"maximized\"\nwindow_padding_top = 3",
        )
        .unwrap();
        assert_eq!(
            (c.font_weight, c.launch_mode, c.window_padding_top),
            (300, LaunchMode::Maximized, Some(3.0))
        );
    }

    #[test]
    fn paddings_default_to_the_old_layout_and_set_per_side() {
        let c = Config::default();
        let p = c.paddings(false);
        assert_eq!((p.left, p.right, p.top, p.bottom), (8.0, 8.0, 14.0, 8.0));
        // the alternate screen keeps them unless alt_screen_padding is off
        assert_eq!(c.paddings(true), p);
        let set: Config = toml::from_str(
            "window_padding = 12\nwindow_padding_right = 2.5\nwindow_padding_top = 99\n\
             window_padding_bottom = -3\nalt_screen_padding = false\n",
        )
        .unwrap();
        let p = set.paddings(false);
        assert_eq!((p.left, p.right, p.top, p.bottom), (12.0, 2.5, 40.0, 0.0));
        let left = Config {
            window_padding_left: Some(30.0),
            ..set.clone()
        };
        assert_eq!(
            (left.paddings(false).left, left.paddings(false).right),
            (30.0, 2.5)
        );
        let alt = set.paddings(true);
        assert_eq!(
            (alt.left, alt.right, alt.top, alt.bottom),
            (0.0, 0.0, 0.0, 0.0)
        );
        // unset sides stay out of config.toml; a set one is written short
        let text = toml::to_string(&set).unwrap();
        assert!(text.contains("window_padding_right = 2.5\n"), "{text}");
        assert!(!text.contains("window_padding_left"), "{text}");
        assert!(
            !toml::to_string(&Config::default())
                .unwrap()
                .contains("window_padding_top")
        );
    }

    #[test]
    fn window_size_paste_and_launch_defaults() {
        let c = Config::default();
        assert_eq!(c.window_size(), (1180.0, 760.0));
        assert_eq!(c.launch_mode, LaunchMode::Default);
        assert_eq!(c.large_paste_bytes(), None);
        assert!(!c.trim_paste && !c.always_on_top && !c.snap_to_cells);
        assert!(!c.resize_overlay && !c.wheel_zoom && !c.request_attention);
        let set: Config = toml::from_str(
            "window_width = 100\nwindow_height = 90000\nlaunch_mode = \"maximized\"\n\
             large_paste_warning = 5\n",
        )
        .unwrap();
        assert_eq!(set.window_size(), (320.0, 16384.0));
        assert_eq!(set.launch_mode, LaunchMode::Maximized);
        assert_eq!(set.large_paste_bytes(), Some(5120));
        let nan = Config {
            window_width: f32::NAN,
            ..Default::default()
        };
        assert_eq!(nan.window_size().0, 1180.0);
    }

    #[test]
    fn wheel_lines_clamp_and_right_option_follows_left_until_set() {
        let mut c = Config::default();
        assert_eq!(c.wheel_lines(), 3.0);
        c.wheel_scroll_lines = 0;
        assert_eq!(c.wheel_lines(), 1.0);
        c.wheel_scroll_lines = 200;
        assert_eq!(c.wheel_lines(), 20.0);
        c.option_as_meta = true;
        assert!(c.right_option_meta());
        c.right_option_as_meta = Some(false);
        assert!(!c.right_option_meta());
        // unset stays out of config.toml; set is written
        let unset = toml::to_string(&Config::default()).unwrap();
        assert!(!unset.contains("right_option_as_meta"), "{unset}");
        assert!(
            toml::to_string(&c)
                .unwrap()
                .contains("right_option_as_meta = false")
        );
    }

    #[test]
    fn roundtrip() {
        let mut c = Config {
            font_size: 14.0,
            ..Default::default()
        };
        c.keybindings.insert("new_tab".into(), "cmd+t".into());
        c.notifications.sound_failed = "Basso".into();
        c.notifications.toasts = false;
        let s = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn remote_access_is_off_unless_a_real_boolean_turns_it_on() {
        let c = Config::default();
        assert!(!c.remote_access);
        assert_eq!(c.remote_listen, "0.0.0.0:7878");
        let c: Config =
            toml::from_str("remote_access = true\nremote_listen = \"127.0.0.1:9\"\n").unwrap();
        assert!(c.remote_access);
        assert_eq!(c.remote_listen, "127.0.0.1:9");
        // under another table it is not the setting
        let c: Config = toml::from_str("[x]\nremote_access = true\n").unwrap();
        assert!(!c.remote_access);
        for bad in ["remote_access = \"true\"", "remote_access = 1"] {
            assert!(toml::from_str::<Config>(bad).is_err(), "{bad}");
        }
        // the invalid-file fallback keeps it off too
        assert!(!Config::fallback().remote_access);
        // Settings writes only the changed key, in place
        let on = Config {
            remote_access: true,
            ..Default::default()
        };
        let out = on.edit_toml(Some("# mine\nfont_size = 13.0\n")).unwrap();
        assert_eq!(out, "# mine\nfont_size = 13.0\nremote_access = true\n");
        let off = Config {
            remote_access: false,
            ..toml::from_str(&out).unwrap()
        };
        let out = off.edit_toml(Some(&out)).unwrap();
        assert_eq!(out, "# mine\nfont_size = 13.0\nremote_access = false\n");
    }

    #[test]
    fn remote_relay_defaults_to_the_public_relay_and_is_saved_in_place() {
        let c = Config::default();
        assert_eq!(c.remote_relay, DEFAULT_RELAY);
        assert!(DEFAULT_RELAY.starts_with("wss://"));
        // a file without the key gets the default; an empty value opts out
        let c: Config = toml::from_str("font_size = 13.0\n").unwrap();
        assert_eq!(c.remote_relay, DEFAULT_RELAY);
        let c: Config = toml::from_str("remote_relay = \"\"\n").unwrap();
        assert_eq!(c.remote_relay, "");
        let c: Config = toml::from_str("remote_relay = \"wss://relay.example.com\"\n").unwrap();
        assert_eq!(c.remote_relay, "wss://relay.example.com");
        assert!(toml::from_str::<Config>("remote_relay = true").is_err());
        // the default is not written; another one is, in place
        let out = Config::default()
            .edit_toml(Some("# mine\nfont_size = 13.0\n"))
            .unwrap();
        assert_eq!(out, "# mine\nfont_size = 13.0\n");
        let set = Config {
            remote_relay: "ws://127.0.0.1:18790".into(),
            ..Default::default()
        };
        let out = set.edit_toml(Some(&out)).unwrap();
        assert_eq!(
            out,
            "# mine\nfont_size = 13.0\nremote_relay = \"ws://127.0.0.1:18790\"\n"
        );
        // clearing it writes the empty value back over the old one
        let cleared = Config {
            remote_relay: String::new(),
            ..toml::from_str(&out).unwrap()
        };
        let out = cleared.edit_toml(Some(&out)).unwrap();
        assert_eq!(out, "# mine\nfont_size = 13.0\nremote_relay = \"\"\n");
        assert_eq!(toml::from_str::<Config>(&out).unwrap().remote_relay, "");
    }

    #[test]
    fn scripted_runs_skip_only_the_default_relay() {
        assert_eq!(relay_of(DEFAULT_RELAY, false), Some(DEFAULT_RELAY));
        assert_eq!(relay_of(DEFAULT_RELAY, true), None);
        assert_eq!(relay_of(&format!(" {DEFAULT_RELAY} "), true), None);
        assert_eq!(
            relay_of(" ws://127.0.0.1:9 ", true),
            Some("ws://127.0.0.1:9")
        );
        assert_eq!(relay_of("  ", false), None);
    }

    #[test]
    fn app_icon_defaults_and_parses() {
        let c: Config = toml::from_str("font_size = 14").unwrap();
        assert_eq!(c.app_icon, AppIcon::Default);
        for icon in AppIcon::ALL {
            let c: Config = toml::from_str(&format!("app_icon = \"{}\"", icon.name())).unwrap();
            assert_eq!(c.app_icon, icon);
        }
        // an icon this Rift does not have is the default, not a dead config
        let other: Config = toml::from_str("app_icon = \"warp\"\nfont_size = 15").unwrap();
        assert_eq!((other.app_icon, other.font_size), (AppIcon::Default, 15.0));
    }

    #[test]
    fn toolbar_keeps_the_old_layout_and_moves_items() {
        use ToolbarItem::*;
        let mut c: Config = toml::from_str("font_size = 14").unwrap();
        assert_eq!(c.toolbar(), (vec![Explorer], vec![Notifications, Review]));
        assert_eq!(c.toolbar_side(Palette), ToolbarSide::Hidden);
        c.set_toolbar_side(Palette, ToolbarSide::Left);
        c.set_toolbar_side(Review, ToolbarSide::Left);
        c.set_toolbar_side(Explorer, ToolbarSide::Hidden);
        assert_eq!(c.toolbar(), (vec![Palette, Review], vec![Notifications]));
        // reordered within a side, clamped at its ends
        assert_eq!(c.toolbar_position(Review), Some(2));
        c.move_toolbar_item(Review, -1);
        assert_eq!(c.toolbar(), (vec![Review, Palette], vec![Notifications]));
        c.move_toolbar_item(Review, -1);
        c.move_toolbar_item(Notifications, 3);
        assert_eq!(c.toolbar(), (vec![Review, Palette], vec![Notifications]));
        c.move_toolbar_item(Review, 1);
        assert_eq!(c.toolbar_position(Explorer), None);
        // a saved config reads back the same
        let back: Config = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.toolbar(), c.toolbar());
        // listed twice: shown once, on the first side
        let twice: Config = toml::from_str(
            "toolbar_left = [\"review\"]\ntoolbar_right = [\"review\", \"settings\", \"settings\"]",
        )
        .unwrap();
        assert_eq!(twice.toolbar(), (vec![Review], vec![Settings]));
        // an unknown item is skipped, the rest of the file still applies
        let typo: Config =
            toml::from_str("toolbar_left = [\"weather\", \"palette\"]\nfont_size = 15").unwrap();
        assert_eq!(
            (typo.toolbar_left.clone(), typo.font_size),
            (vec![Palette], 15.0)
        );
        let none: Config = toml::from_str("toolbar_left = []\ntoolbar_right = []").unwrap();
        assert_eq!(none.toolbar(), (vec![], vec![]));
    }

    #[test]
    fn block_and_tab_settings_default_to_the_old_behavior() {
        // an existing config without the keys keeps what Rift did before
        let c: Config = toml::from_str("font_size = 14").unwrap();
        assert_eq!(c.tab_close_button, TabCloseButton::Hover);
        assert_eq!(c.new_tab_placement, NewTabPlacement::End);
        assert!(c.tab_bar_with_one_tab);
        assert!(c.sticky_command_header);
        assert_eq!(c.block_spacing, BlockSpacing::Compact);
        assert!(
            c.subshell_offer && c.subshell_commands.is_empty() && c.subshell_denylist.is_empty()
        );
        assert!(!c.block_dividers);
        assert!(c.jump_to_bottom_button);
        assert_eq!(c.tab_close_side, TabCloseSide::Right);
        assert_eq!(c.tab_bar, TabBarVisibility::Always);
        assert!(c.dim_inactive_panes);
        assert_eq!(c.inactive_pane_dim, 10);
        assert_eq!(c.inactive_pane_dim(), 0.10);
        assert!(!c.focus_follows_mouse);
        assert_eq!(c.close_on_exit, CloseOnExit::Always);
    }

    #[test]
    fn close_on_exit_parses_and_decides() {
        let c: Config = toml::from_str("close_on_exit = \"graceful\"").unwrap();
        assert_eq!(c.close_on_exit, CloseOnExit::Graceful);
        assert!(CloseOnExit::Graceful.closes(Some(0)));
        assert!(!CloseOnExit::Graceful.closes(Some(1)));
        assert!(!CloseOnExit::Graceful.closes(None));
        assert!(CloseOnExit::Always.closes(Some(2)));
        assert!(!CloseOnExit::Never.closes(Some(0)));
        assert!(toml::from_str::<Config>("close_on_exit = \"sometimes\"").is_err());
    }

    #[test]
    fn block_and_tab_settings_parse() {
        let c: Config = toml::from_str(
            "tab_close_button = \"always\"\n\
             new_tab_placement = \"after_current\"\n\
             tab_bar_with_one_tab = false\n\
             sticky_command_header = false\n\
             inactive_pane_dim = 25\n\
             focus_follows_mouse = true",
        )
        .unwrap();
        assert_eq!(c.tab_close_button, TabCloseButton::Always);
        assert_eq!(c.new_tab_placement, NewTabPlacement::AfterCurrent);
        assert!(!c.tab_bar_with_one_tab);
        assert!(!c.sticky_command_header);
        assert_eq!(c.inactive_pane_dim(), 0.25);
        assert!(c.focus_follows_mouse);
        let more: Config = toml::from_str(
            "block_spacing = \"normal\"\n\
             block_dividers = true\n\
             jump_to_bottom_button = false\n\
             tab_close_side = \"left\"\n\
             tab_bar = \"on_hover\"",
        )
        .unwrap();
        assert_eq!(more.block_spacing, BlockSpacing::Normal);
        assert!(more.block_dividers);
        assert!(!more.jump_to_bottom_button);
        assert_eq!(more.tab_close_side, TabCloseSide::Left);
        assert_eq!(more.tab_bar, TabBarVisibility::OnHover);
        let fs: Config = toml::from_str("tab_bar = \"hide_fullscreen\"").unwrap();
        assert_eq!(fs.tab_bar, TabBarVisibility::HideFullscreen);
        assert!(toml::from_str::<Config>("tab_bar = \"never\"").is_err());
        assert!(toml::from_str::<Config>("block_spacing = \"wide\"").is_err());
        let never: Config = toml::from_str("tab_close_button = \"never\"").unwrap();
        assert_eq!(never.tab_close_button, TabCloseButton::Never);
        assert!(toml::from_str::<Config>("tab_close_button = \"left\"").is_err());
        assert!(toml::from_str::<Config>("new_tab_placement = \"first\"").is_err());
        // off draws no wash at any strength; out-of-range strengths clamp
        let off: Config =
            toml::from_str("dim_inactive_panes = false\ninactive_pane_dim = 30").unwrap();
        assert_eq!(off.inactive_pane_dim(), 0.0);
        let huge: Config = toml::from_str("inactive_pane_dim = 9000").unwrap();
        assert_eq!(huge.inactive_pane_dim(), 0.40);
        let zero: Config = toml::from_str("inactive_pane_dim = -3").unwrap();
        assert_eq!(zero.inactive_pane_dim(), 0.01);
        // and they round-trip through the writer
        let back: Config = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let c: Config = toml::from_str("font_size = 15.0").unwrap();
        assert_eq!(c.font_size, 15.0);
        assert_eq!(c.theme, "dark-default");
        assert!(c.ligatures);
    }

    #[test]
    fn text_rendering_options() {
        // defaults keep today's rendering
        let d = Config::default();
        assert_eq!(d.font_weights(), (400, 700));
        assert!(d.font_feature_tags().is_empty());
        assert_eq!(d.min_contrast(), 1.0);
        assert!(!d.bold_is_bright && !d.cursor_hollow_unfocused);
        assert_eq!(d.ambiguous_width, AmbiguousWidth::Narrow);

        let c: Config = toml::from_str(
            "font_weight = 350\nbold_font_weight = 2000\nminimum_contrast = 4.5\n\
             bold_is_bright = true\nambiguous_width = \"wide\"\n\
             cursor_style = \"double_underline\"\ncursor_hollow_unfocused = true\n\
             font_features = [\"ss01\", \"+zero\", \"-calt\", \"cv05=2\", \"bogus\", \"ss0\"]",
        )
        .unwrap();
        assert_eq!(c.font_weights(), (350, 900), "clamped to 100–900");
        assert_eq!(
            c.font_feature_tags(),
            vec![(*b"ss01", 1), (*b"zero", 1), (*b"calt", 0), (*b"cv05", 2)]
        );
        assert_eq!(c.font_features.len(), 4, "invalid entries are dropped");
        assert_eq!(c.min_contrast(), 4.5);
        assert!(c.bold_is_bright && c.cursor_hollow_unfocused);
        assert_eq!(c.ambiguous_width, AmbiguousWidth::Wide);
        assert_eq!(c.cursor_style, CursorStyle::DoubleUnderline);
        let hollow: Config = toml::from_str("cursor_style = \"hollow\"").unwrap();
        assert_eq!(hollow.cursor_style, CursorStyle::Hollow);
        let huge: Config = toml::from_str("minimum_contrast = 99.0").unwrap();
        assert_eq!(huge.min_contrast(), 21.0);
        let nan: Config = toml::from_str("minimum_contrast = nan").unwrap();
        assert_eq!(nan.min_contrast(), 1.0);
        assert_eq!(font_feature("cv05=x"), None);
        assert_eq!(font_feature("s s1"), None);
        // and they round-trip through the writer
        let back: Config = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn enum_fields_parse_lowercase() {
        let c: Config = toml::from_str(
            "input_position = \"top\"\nprompt = \"shell\"\nbell = \"none\"\nvim_mode = true",
        )
        .unwrap();
        assert_eq!(c.input_position, InputPosition::Top);
        assert_eq!(c.blocks_start, BlocksStart::Bottom);
        let top: Config = toml::from_str("blocks_start = \"top\"").unwrap();
        assert_eq!(top.blocks_start, BlocksStart::Top);
        assert_eq!(c.shell_widgets, ShellWidgets::Auto);
        let rift: Config = toml::from_str("shell_widgets = \"rift\"").unwrap();
        assert_eq!(rift.shell_widgets, ShellWidgets::Rift);
        assert_eq!(c.prompt_chips, PromptChip::defaults());
        let chips: Config =
            toml::from_str("prompt_chips = [\"time24\", \"cwd\", \"git_diff\"]").unwrap();
        assert_eq!(
            chips.prompt_chips,
            [PromptChip::Time24, PromptChip::Cwd, PromptChip::GitDiff]
        );
        assert!(toml::from_str::<Config>("prompt_chips = [\"weather\"]").is_err());
        assert_eq!(c.elapsed_counts, ElapsedCounts::Always);
        let logged: Config = toml::from_str("elapsed_counts = \"running\"").unwrap();
        assert_eq!(logged.elapsed_counts, ElapsedCounts::Running);
        assert!(toml::from_str::<Config>("elapsed_counts = \"never\"").is_err());
        assert_eq!(c.prompt, PromptMode::Shell);
        assert_eq!(c.bell, Bell::None);
        assert!(c.vim_mode);
        assert!(toml::from_str::<Config>("input_position = \"middle\"").is_err());
        let legacy: Config = toml::from_str("prompt = \"runway\"").unwrap();
        assert_eq!(legacy.prompt, PromptMode::Rift);
        let legacy: Config = toml::from_str("prompt = \"shift\"").unwrap();
        assert_eq!(legacy.prompt, PromptMode::Rift);
    }

    #[test]
    fn os_theme_sync_picks_light_only_when_enabled() {
        let c = Config {
            theme: "dark-default".into(),
            theme_light: "light-default".into(),
            sync_os_theme: false,
            ..Default::default()
        };
        assert!(!c.resolve_theme_for(true).is_light());
        let c = Config {
            sync_os_theme: true,
            ..c
        };
        assert!(c.resolve_theme_for(true).is_light());
        assert!(!c.resolve_theme_for(false).is_light());
    }

    #[test]
    fn start_dir_modes() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        let tmp = std::env::temp_dir();
        let tmp_s = tmp.to_string_lossy().into_owned();
        let mut c = Config::default();
        assert_eq!(c.start_dir(Some(&tmp_s)), Some(tmp.clone()));
        assert_eq!(c.start_dir(None), Some(home.clone()));
        // a vanished previous directory falls back to home
        assert_eq!(
            c.start_dir(Some("/definitely/not/here")),
            Some(home.clone())
        );
        c.working_directory = "home".into();
        assert_eq!(c.start_dir(Some(&tmp_s)), Some(home.clone()));
        c.working_directory = tmp_s.clone();
        assert_eq!(c.start_dir(None), Some(tmp));
        c.working_directory = "~".into();
        assert_eq!(c.start_dir(None), Some(home));
    }

    #[test]
    fn start_dir_expands_only_tilde_and_tilde_slash() {
        let h = Path::new("/home/u");
        assert_eq!(expand_tilde("~", Some(h)), PathBuf::from("/home/u"));
        assert_eq!(expand_tilde("~/", Some(h)), PathBuf::from("/home/u"));
        assert_eq!(expand_tilde("~/a/b", Some(h)), PathBuf::from("/home/u/a/b"));
        // `~root` is a literal name, never $HOME/root
        assert_eq!(expand_tilde("~root", Some(h)), PathBuf::from("~root"));
        assert_eq!(expand_tilde("/abs", Some(h)), PathBuf::from("/abs"));
        assert_eq!(expand_tilde("~/a", None), PathBuf::from("~/a"));
    }

    #[test]
    fn empty_xdg_config_home_is_unset() {
        use std::ffi::OsString;
        let home = Some(OsString::from("/home/u"));
        let want = Some(PathBuf::from("/home/u/.config/rift"));
        assert_eq!(config_dir_from(Some(OsString::new()), home.clone()), want);
        assert_eq!(config_dir_from(None, home.clone()), want);
        assert_eq!(
            config_dir_from(Some(OsString::from("/x")), home),
            Some(PathBuf::from("/x/rift"))
        );
    }

    #[test]
    fn tilde_path_shortens_only_the_home_prefix() {
        let h = Some(Path::new("/home/u"));
        assert_eq!(
            tilde_path(Path::new("/home/u/.config/rift/config.toml"), h),
            "~/.config/rift/config.toml"
        );
        assert_eq!(tilde_path(Path::new("/home/u"), h), "~");
        assert_eq!(tilde_path(Path::new("/home/user2/x"), h), "/home/user2/x");
        assert_eq!(tilde_path(Path::new("/etc/x"), None), "/etc/x");
    }

    #[test]
    fn earlier_app_names_keep_their_config_dir() {
        let root = std::env::temp_dir().join(format!("rift-legacy-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let xdg = Some(root.clone().into_os_string());
        let dir = |name: &str| Some(root.join(name));
        std::fs::create_dir_all(root.join("runway")).unwrap();
        assert_eq!(config_dir_from(xdg.clone(), None), dir("runway"));
        std::fs::create_dir_all(root.join("shift")).unwrap();
        assert_eq!(config_dir_from(xdg.clone(), None), dir("shift"));
        std::fs::create_dir_all(root.join("rift")).unwrap();
        assert_eq!(config_dir_from(xdg, None), dir("rift"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn shift_viewer_value_loads_as_rift() {
        let path =
            std::env::temp_dir().join(format!("rift-legacy-editor-{}.toml", std::process::id()));
        std::fs::write(&path, "link_editor = \"shift\"\nfile_editor = \"zed\"\n").unwrap();
        let c = Config::try_load_from(&path).unwrap().unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(c.link_editor, crate::editors::RIFT);
        assert_eq!(c.file_editor, "zed");
    }

    #[cfg(unix)]
    #[test]
    fn save_through_symlink_keeps_the_link() {
        let dir = scratch("symlink");
        let real_dir = dir.join("dotfiles");
        std::fs::create_dir_all(&real_dir).unwrap();
        let real = real_dir.join("config.toml");
        std::fs::write(&real, "# mine\nfont_size = 15.0\n").unwrap();
        let link = dir.join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let c = Config {
            font_size: 17.0,
            ..Config::default()
        };
        c.save_to(&link).unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "link replaced by a regular file"
        );
        let body = std::fs::read_to_string(&real).unwrap();
        assert!(body.contains("# mine") && body.contains("17"), "{body}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scrollback_lines_is_bounded() {
        let c = Config {
            scrollback: usize::MAX,
            ..Config::default()
        };
        assert_eq!(c.scrollback_lines(), 1_000_000);
        assert_eq!(Config::default().scrollback_lines(), 10_000);
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rift-config-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn invalid_file_is_an_error_not_defaults() {
        let dir = scratch("invalid");
        let path = dir.join("config.toml");
        assert_eq!(Config::try_load_from(&path), Ok(None));
        std::fs::write(&path, "control_api = \"off\"\nfont_size = 19.0\n").unwrap();
        let good = Config::try_load_from(&path).unwrap().unwrap();
        assert_eq!(good.control_api, "off");
        std::fs::write(&path, "font_size = [\n").unwrap();
        let err = Config::try_load_from(&path).unwrap_err();
        assert!(err.contains("config.toml"), "{err}");
        assert_eq!(Config::fallback().control_api, "off");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn actions_and_tab_order_load_save_back_and_reject_cycles() {
        let c = Config::default();
        assert!(c.actions.is_empty());
        assert_eq!(c.tab_switch_order, TabSwitchOrder::InOrder);
        let src = r#"tab_switch_order = "mru"

[[actions]]
name = "Watch"
key = "ctrl+alt+h"
steps = [{ action = "split_vertical" }, { send = "htop\n" }]
"#;
        let c: Config = toml::from_str(src).unwrap();
        assert_eq!(c.tab_switch_order, TabSwitchOrder::Mru);
        assert_eq!(c.actions[0].steps.len(), 2);
        let out = c.edit_toml(Some(src)).unwrap();
        assert_eq!(toml::from_str::<Config>(&out).unwrap(), c, "{out}");
        let out = c.edit_toml(None).unwrap();
        assert_eq!(toml::from_str::<Config>(&out).unwrap(), c, "{out}");
        assert!(toml::from_str::<Config>("tab_switch_order = \"random\"").is_err());

        let dir = scratch("actions");
        let path = dir.join("config.toml");
        std::fs::write(&path, src).unwrap();
        assert!(Config::try_load_from(&path).unwrap().is_some());
        let cyclic = "[[actions]]\nname = \"a\"\nsteps = [{ action = \"b\" }]\n\
                      [[actions]]\nname = \"b\"\nsteps = [{ action = \"a\" }]\n";
        std::fs::write(&path, cyclic).unwrap();
        let err = Config::try_load_from(&path).unwrap_err();
        assert!(
            err.contains("config.toml") && err.contains("cycle (a, b, a)"),
            "{err}"
        );
    }

    #[test]
    fn tab_color_rules_parse_and_round_trip() {
        let dir = scratch("tab-color-rules");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "theme = \"nord\"\n\n[[tab_color_rules]]\ndir = \"~/work/prod\"\ncolor = \"#ff3b30\"\n\n\
             [[tab_color_rules]]\ndir = \"/srv\"\ncolor = \"green\"\n",
        )
        .unwrap();
        let c = Config::try_load_from(&path).unwrap().unwrap();
        assert_eq!(c.tab_color_rules.len(), 2);
        assert_eq!(c.tab_color_rules[0].dir, "~/work/prod");
        assert_eq!(c.tab_color_rules[1].color, "green");
        let out = toml::to_string(&c).unwrap();
        assert_eq!(toml::from_str::<Config>(&out).unwrap(), c);
    }

    #[test]
    fn scrollbar_and_secret_masking_parse_and_bad_patterns_are_refused() {
        // absent: masking off, the scrollbar on `auto`
        let c = Config::default();
        assert!(!c.mask_secrets && c.secret_patterns.is_empty());
        assert!(
            c.tab_color_rules.is_empty(),
            "no directory rules by default"
        );
        assert_eq!(c.scrollbar, ScrollbarMode::Auto);
        let dir = scratch("secrets");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "scrollbar = \"never\"\nmask_secrets = true\n\n[[secret_patterns]]\n\
             name = \"corp\"\nregex = 'corp-[0-9]{6}'\n",
        )
        .unwrap();
        let c = Config::try_load_from(&path).unwrap().unwrap();
        assert_eq!(c.scrollbar, ScrollbarMode::Never);
        assert!(c.mask_secrets);
        assert_eq!(c.secret_patterns[0].name, "corp");
        // saved back, the table stays after the plain keys
        let out = toml::to_string(&c).unwrap();
        assert_eq!(toml::from_str::<Config>(&out).unwrap(), c);
        // a pattern that does not compile refuses the file
        std::fs::write(
            &path,
            "[[secret_patterns]]\nname = \"bad\"\nregex = '(unclosed'\n",
        )
        .unwrap();
        let err = Config::try_load_from(&path).unwrap_err();
        assert!(err.contains("secret pattern \"bad\""), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn profiles_parse_validate_and_save_back() {
        // absent: today's behaviour
        let c = Config::default();
        assert!(c.profiles.is_empty() && c.default_profile.is_none() && c.discover_shells);
        let src = r#"default_profile = "dev"
discover_shells = false

[[profiles]]
name = "dev"
command = "/bin/zsh"
args = ["-c", "echo \"a b\""]
cwd = "~/proj x"
title = "Dev"
color = "green"

[profiles.env]
X = "1"

[[profiles]]
name = "clean"
"#;
        let c: Config = toml::from_str(src).unwrap();
        assert_eq!(c.default_profile.as_deref(), Some("dev"));
        assert!(!c.discover_shells);
        assert_eq!(c.profiles.len(), 2);
        let dev = &c.profiles[0];
        assert_eq!(dev.args, ["-c", "echo \"a b\""]);
        assert_eq!(dev.env.get("X").map(String::as_str), Some("1"));
        assert_eq!(c.profiles[1].command, None);
        // saved and read back unchanged
        let out = c.edit_toml(Some(src)).unwrap();
        assert_eq!(toml::from_str::<Config>(&out).unwrap(), c, "{out}");
        let out = c.edit_toml(None).unwrap();
        assert_eq!(toml::from_str::<Config>(&out).unwrap(), c, "{out}");
        // clearing default_profile removes its line
        let cleared = Config {
            default_profile: None,
            ..c.clone()
        };
        assert!(
            !cleared
                .edit_toml(Some(src))
                .unwrap()
                .contains("default_profile")
        );

        // an invalid entry rejects the file like a parse error
        let dir = scratch("profiles");
        let path = dir.join("config.toml");
        std::fs::write(&path, src).unwrap();
        assert!(Config::try_load_from(&path).unwrap().is_some());
        let dup = format!("{src}\n[[profiles]]\nname = \"dev\"\n");
        std::fs::write(&path, dup).unwrap();
        let err = Config::try_load_from(&path).unwrap_err();
        assert!(
            err.contains("config.toml") && err.contains("used twice"),
            "{err}"
        );
        std::fs::write(&path, "[[profiles]]\ncommand = \"/bin/sh\"\n").unwrap();
        let err = Config::try_load_from(&path).unwrap_err();
        assert!(err.contains("name"), "a profile needs a name: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_is_atomic_under_concurrent_reads() {
        let dir = scratch("atomic");
        let path = dir.join("config.toml");
        let mut c = Config::default();
        for i in 0..400 {
            c.keybindings
                .insert(format!("action_{i}"), format!("cmd+{i}"));
        }
        c.save_to(&path).unwrap();
        let reader_path = path.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader_stop = stop.clone();
        let reader = std::thread::spawn(move || {
            let mut bad = 0;
            while !reader_stop.load(std::sync::atomic::Ordering::Relaxed) {
                if Config::try_load_from(&reader_path).ok().flatten().is_none() {
                    bad += 1;
                }
            }
            bad
        });
        for _ in 0..200 {
            c.save_to(&path).unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(reader.join().unwrap(), 0, "a reader saw a partial config");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name() != "config.toml")
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_save_keeps_the_original_bytes() {
        let dir = scratch("failsave");
        let path = dir.join("config.toml");
        std::fs::write(&path, "# mine\nfont_size = 19.0\n").unwrap();
        // a directory occupying the temp file's name makes the write fail
        std::fs::create_dir(dir.join(format!(".config.toml.tmp-{}", std::process::id()))).unwrap();
        assert!(Config::default().save_to(&path).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# mine\nfont_size = 19.0\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_edits_in_place_keeping_comments_and_unknown_keys() {
        let file = "# my setup\nfont_size = 19.0 # big\ntheme = \"dark-default\"\nfuture_option = 1\n\n# bindings\n[keybindings]\nnew_tab = \"cmd+t\"\ncopy = \"cmd+c\"\n";
        let mut c: Config = toml::from_str(file).unwrap();
        c.font_size = 15.0;
        c.keybindings.remove("copy");
        let out = c.edit_toml(Some(file)).unwrap();
        assert_eq!(
            out,
            "# my setup\nfont_size = 15.0 # big\ntheme = \"dark-default\"\nfuture_option = 1\n\n# bindings\n[keybindings]\nnew_tab = \"cmd+t\"\n"
        );
        assert_eq!(toml::from_str::<Config>(&out).unwrap(), c);
    }

    #[test]
    fn save_adds_only_changed_settings() {
        assert_eq!(Config::default().edit_toml(None).unwrap(), "");
        let mut c = Config {
            font_size: 14.0,
            shell: Some("/bin/zsh".into()),
            ..Default::default()
        };
        c.notifications.toasts = false;
        let out = c.edit_toml(Some("# hi\n")).unwrap();
        assert!(out.starts_with("# hi\n"), "{out}");
        assert!(out.contains("font_size = 14.0"), "{out}");
        assert!(out.contains("[notifications]\ntoasts = false\n"), "{out}");
        assert!(!out.contains("theme"), "unchanged defaults stay out: {out}");
        assert_eq!(toml::from_str::<Config>(&out).unwrap(), c);
        // clearing an optional setting removes its line
        let cleared = Config {
            shell: None,
            ..c.clone()
        };
        let out = cleared.edit_toml(Some(&out)).unwrap();
        assert!(!out.contains("shell"), "{out}");
    }

    #[test]
    fn save_refuses_to_rewrite_an_unparseable_file() {
        assert!(
            Config::default()
                .edit_toml(Some("font_size = [\n"))
                .is_err()
        );
    }

    #[test]
    fn font_metrics_reject_non_finite() {
        let c: Config = toml::from_str("font_size = nan\nline_height = inf").unwrap();
        assert_eq!(c.font_size(), 13.0);
        assert_eq!(c.line_height(), 1.2);
        let c = Config {
            font_size: 99.0,
            line_height: 0.1,
            ..Default::default()
        };
        assert_eq!(c.font_size(), 28.0);
        assert_eq!(c.line_height(), 1.0);
    }

    #[test]
    fn opacity_clamps() {
        let mut c = Config::default();
        assert_eq!(c.opacity(), 0.5);
        c.window_opacity = 0.1;
        assert_eq!(c.opacity(), 0.1);
        c.window_opacity = 0.0;
        assert_eq!(c.opacity(), 0.01);
        c.window_opacity = 7.0;
        assert_eq!(c.opacity(), 1.0);
        c.window_opacity = f32::NAN;
        assert_eq!(c.opacity(), 1.0);
        assert_eq!(c.blur(), 20);
        c.window_blur = 0;
        assert_eq!(c.blur(), 1);
        c.window_blur = 200;
        assert_eq!(c.blur(), 64);
    }

    #[test]
    fn save_keeps_comments_next_to_non_finite_floats() {
        for key in [
            "font_size",
            "line_height",
            "window_padding",
            "window_opacity",
        ] {
            for value in ["nan", "inf", "-inf", "13.0"] {
                let file = format!(
                    "# keep me\n{key} = {value}\nfuture_option = 42\ncontrol_api = \"off\"\n"
                );
                let mut c: Config = toml::from_str(&file).unwrap();
                c.notifications.toasts = !c.notifications.toasts;
                let out = c.edit_toml(Some(&file)).unwrap();
                let case = format!("{key} = {value}");
                assert!(out.starts_with("# keep me\n"), "{case}: {out}");
                assert!(out.contains("future_option = 42"), "{case}: {out}");
                assert!(out.contains(&case), "{case}: {out}");
                let back: Config = toml::from_str(&out).unwrap();
                assert_eq!(back.control_api, "off", "{case}");
                assert_eq!(back.notifications.toasts, c.notifications.toasts, "{case}");
                assert_eq!(back.font_size(), c.font_size(), "{case}");
                assert_eq!(back.line_height(), c.line_height(), "{case}");
            }
        }
    }
}
