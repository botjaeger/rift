use rift_core::{
    Bell, Color, Config, CursorStyle, InputPosition, Keymap, PromptMode, TabLayout, Theme,
};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::state::{ClickTarget, Hit};
use crate::views::icons::{self, Icon};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    /// Where Settings opens (and opens again once closed).
    #[default]
    General,
    Appearance,
    Features,
    Terminal,
    Files,
    Notifications,
    Keybindings,
    Themes,
    Remote,
    About,
}

impl Page {
    pub const ALL: [Page; 10] = [
        Page::General,
        Page::Appearance,
        Page::Features,
        Page::Terminal,
        Page::Files,
        Page::Notifications,
        Page::Keybindings,
        Page::Themes,
        Page::Remote,
        Page::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Appearance => "Appearance",
            Page::Features => "Input",
            Page::Terminal => "Terminal",
            Page::Files => "Files",
            Page::Notifications => "Notifications",
            Page::Keybindings => "Keybindings",
            Page::Themes => "Themes",
            Page::Remote => "Remote access",
            Page::About => "About",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Page::General => "Shell and session behavior.",
            Page::Appearance => "Fonts, cursor, window and theme. Changes apply as you make them.",
            Page::Features => "The command line and blocks.",
            Page::Terminal => "Mouse, clipboard and keys inside the terminal.",
            Page::Files => "Where files open, and the file explorer.",
            Page::Notifications => {
                "Toasts, desktop notifications and sounds; everything also lands in the inbox."
            }
            Page::Keybindings => "Click a shortcut to record a new one · type to filter.",
            Page::Themes => {
                static THEMES: std::sync::OnceLock<String> = std::sync::OnceLock::new();
                THEMES.get_or_init(|| {
                    let dir = shown_path(rift_core::config::themes_dir(), "themes");
                    format!("Built-in themes plus {dir} (TOML or Warp YAML).")
                })
            }
            Page::Remote => "Watch and run commands from a paired phone, at home or away.",
            Page::About => "",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Page::General => Icon::Settings,
            Page::Appearance => Icon::Appearance,
            Page::Features => Icon::Terminal,
            // Input already shows the terminal mark; this page is mostly
            // the mouse and the clipboard
            Page::Terminal => Icon::Copy,
            Page::Files => Icon::File,
            Page::Notifications => Icon::Inbox,
            Page::Keybindings => Icon::Keyboard,
            Page::Themes => Icon::Themes,
            // reaching in from outside (no phone mark in the set)
            Page::Remote => Icon::Open,
            Page::About => Icon::Info,
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

/// Settings › About's update line.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum UpdateLine {
    /// Not checked (checks off, scripted run).
    #[default]
    None,
    Checking,
    UpToDate,
    /// That version is downloading ahead of the install.
    Downloading(String),
    /// That version is downloaded (or this copy cannot download it): an
    /// "Update…" button follows.
    Available(String),
    /// That version is being installed.
    Updating(String),
    Failed,
}

/// Settings › Remote access's gateway line.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum GatewayLine {
    /// `remote_access` is off.
    #[default]
    Off,
    /// On, and no gateway answers yet.
    Starting,
    /// Running, reachable on these addresses (none: no network).
    Listening(Vec<String>),
    /// Running, with this many phones connected.
    Connected(usize),
}

impl UpdateLine {
    /// Checking, downloading or installing: no check to start.
    pub fn busy(&self) -> bool {
        matches!(
            self,
            UpdateLine::Checking | UpdateLine::Downloading(_) | UpdateLine::Updating(_)
        )
    }

    /// The state as words (`None`: never checked).
    pub fn text(&self) -> Option<String> {
        match self {
            UpdateLine::None => None,
            UpdateLine::Checking => Some("Checking for updates…".into()),
            UpdateLine::UpToDate => Some("Up to date".into()),
            UpdateLine::Downloading(v) => Some(format!("Downloading v{v}…")),
            UpdateLine::Available(v) => Some(format!("v{v} available")),
            UpdateLine::Updating(v) => Some(format!("Installing v{v}…")),
            UpdateLine::Failed => Some("Couldn't check for updates".into()),
        }
    }
}

impl GatewayLine {
    pub fn text(&self) -> String {
        match self {
            GatewayLine::Off => "Off".into(),
            GatewayLine::Starting => "Starting…".into(),
            GatewayLine::Listening(a) if a.is_empty() => "Listening, but on no network".into(),
            GatewayLine::Listening(a) => format!("Listening on {}", a.join(", ")),
            GatewayLine::Connected(1) => "1 device connected".into(),
            GatewayLine::Connected(n) => format!("{n} devices connected"),
        }
    }
}

/// Settings › Remote access's relay line (from `status.json`).
#[derive(Debug, Clone, Default, PartialEq)]
pub enum RelayLine {
    /// No relay set (or remote access is off).
    #[default]
    Off,
    /// Only the default relay is configured and the run is scripted
    /// (`RIFT_NO_DEFAULT_RELAY`).
    Scripted,
    /// Dialing or registering at this host.
    Connecting(String),
    /// Registered at this host; the last ping's round trip, when known.
    Connected {
        host: String,
        latency_ms: Option<u32>,
    },
    /// The last try failed: why (the whole text), and in how many seconds
    /// it is tried again.
    Error {
        host: String,
        reason: String,
        retry_in: Option<u64>,
    },
}

impl RelayLine {
    /// The state in a sentence (under the label).
    pub fn text(&self) -> String {
        match self {
            RelayLine::Off => "No relay: phones connect only on this network".into(),
            RelayLine::Scripted => "Not used in scripted runs".into(),
            RelayLine::Connecting(_) => "Registering this computer at the relay…".into(),
            RelayLine::Connected {
                latency_ms: Some(ms),
                ..
            } => format!("Phones away from home connect through it\u{a0}·\u{a0}{ms}\u{a0}ms"),
            RelayLine::Connected { .. } => "Phones away from home connect through it".into(),
            RelayLine::Error {
                retry_in: Some(s), ..
            } => format!(
                "Can’t reach the relay, so phones away from home can’t connect. Retrying in {s}\u{a0}s."
            ),
            RelayLine::Error { .. } => {
                "Can’t reach the relay, so phones away from home can’t connect.".into()
            }
        }
    }

    /// The relay's host (right of the label).
    pub fn host(&self) -> Option<&str> {
        match self {
            RelayLine::Connecting(host)
            | RelayLine::Connected { host, .. }
            | RelayLine::Error { host, .. } => Some(host),
            RelayLine::Off | RelayLine::Scripted => None,
        }
    }

    /// Why the last try failed.
    pub fn reason(&self) -> Option<&str> {
        match self {
            RelayLine::Error { reason, .. } => Some(reason),
            _ => None,
        }
    }
}

/// How a paired phone is connected right now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DevicePath {
    #[default]
    None,
    /// Directly, on the same network.
    Lan,
    /// Spliced by the relay.
    Relay,
}

impl DevicePath {
    pub fn text(self) -> &'static str {
        match self {
            DevicePath::None => "Not connected",
            DevicePath::Lan => "Connected on this network",
            DevicePath::Relay => "Connected through the relay",
        }
    }
}

/// A device's reported platform as people write it (`ios` is "iOS");
/// anything unknown is shown as reported.
pub fn platform_name(platform: &str) -> &str {
    match platform {
        "ios" => "iOS",
        "android" => "Android",
        "web" => "Web",
        "macos" => "macOS",
        other => other,
    }
}

/// A paired device as Settings › Remote access lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    /// `Connected through the relay · iOS · paired October 12, 2026`
    pub detail: String,
    pub path: DevicePath,
    /// It may only watch (else operate).
    pub view_only: bool,
}

/// One line of Settings › Remote access › Recent connections.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryRow {
    /// The device's name, or "Unknown device" (a failed handshake).
    pub name: String,
    /// `Relay · 10.0.0.5:51234 · Oct 12 14:03 · 5m 12s` (or `connected`,
    /// or the failure).
    pub detail: String,
}

/// A live pairing link and its QR code.
#[derive(Debug, Clone, PartialEq)]
pub struct PairingView {
    pub url: String,
    /// Modules, `true` = dark, without the quiet zone.
    pub qr: Vec<Vec<bool>>,
    pub expires_at: std::time::SystemTime,
}

/// What Settings › Remote access shows (the app re-reads it at most once
/// per second while the page is open).
#[derive(Debug, Clone, Default)]
pub struct RemoteView {
    pub gateway: GatewayLine,
    /// The addresses the gateway listens on (`IP:PORT`).
    pub addresses: Vec<String>,
    pub devices: Vec<DeviceRow>,
    /// `devices.json` could not be read.
    pub devices_error: Option<String>,
    /// `devices.json` was read at least once (before that the list is not
    /// known: "No paired devices" would be a guess).
    pub devices_loaded: bool,
    /// The next pairing link grants `view` only.
    pub view_only: bool,
    /// "Pair a phone" was pressed and waits for the gateway's addresses.
    pub pairing_wait: bool,
    pub pairing: Option<PairingView>,
    pub relay: RelayLine,
    /// The relay field is being edited: its text (⏎ saves, Esc cancels).
    pub relay_edit: Option<String>,
    /// "Relay settings…" is open (the relay address, the Cloudflare tunnel).
    pub relay_open: bool,
    /// `remote_tunnel`: the gateway's `tunnel.json` (None: not running yet).
    pub tunnel: Option<crate::tunnel::TunnelState>,
    /// Where cloudflared is installed (checked while the page shows).
    pub cloudflared: Option<std::path::PathBuf>,
    /// Homebrew can install it ("Install"; else "Download").
    pub brew: bool,
    /// Newest first, at most [`HISTORY_ROWS`]: connected devices, then
    /// `history.jsonl`.
    pub history: Vec<HistoryRow>,
}

/// Rows Recent connections keeps.
pub const HISTORY_ROWS: usize = 20;
/// Rows Recent connections shows at once; the wheel scrolls through the rest.
pub const HISTORY_SHOWN: usize = 5;

/// Modules of light quiet zone around the drawn QR code.
pub const QR_QUIET: usize = 4;
/// Logical px per QR module (at least 4, so phone cameras resolve it).
pub const QR_MODULE: f32 = 4.0;

/// `m:ss` left of `left`, rounded up (2:00 right after pairing starts).
pub fn countdown(left: std::time::Duration) -> String {
    let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
    format!("{}:{:02}", secs / 60, secs % 60)
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
    /// Height of the last rendered page content (for scroll clamping).
    pub content_h: f32,
    /// Height of the body the content scrolls in.
    pub view_h: f32,
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
    /// About's update line (set by the app from the update check).
    pub update: UpdateLine,
    /// Remote access's gateway, devices and pairing (set by the app).
    pub remote: RemoteView,
    /// Recent connections: the first row shown (whole rows, so none is cut).
    pub history_top: usize,
    /// When config.toml was last written or reloaded: "Open config.toml"
    /// shows a check for `SAVED_SHOWN` after it.
    pub saved_at: Option<std::time::Instant>,
}

/// How long the "updated" check stays beside "Open config.toml".
pub const SAVED_SHOWN: std::time::Duration = std::time::Duration::from_secs(3);

impl SettingsState {
    /// The "updated" check's end, while it shows.
    pub fn saved_until(&self) -> Option<std::time::Instant> {
        self.saved_at
            .map(|t| t + SAVED_SHOWN)
            .filter(|until| *until > std::time::Instant::now())
    }

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

    /// Remove the Settings tab. Opened again, it starts on General (only
    /// leaving it for another tab keeps its page).
    pub fn close_tab(&mut self) {
        self.open = false;
        self.tab = false;
        self.page = Page::General;
        self.scroll = 0.0;
        self.query.clear();
        self.history_top = 0;
        self.focus = None;
        self.dropdown = None;
        self.recording = None;
        self.remote.relay_edit = None;
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
    pub fn scroll_by(&mut self, lines: f32) {
        self.scroll = (self.scroll - lines * 24.0).clamp(0.0, self.max_scroll());
    }

    /// Wheel over Recent connections: `rows` > 0 scrolls up (toward the
    /// newest). False at either end, so the page scrolls on instead.
    pub fn scroll_history(&mut self, rows: isize) -> bool {
        let last = self.remote.history.len().saturating_sub(HISTORY_SHOWN);
        let top = self.history_top.min(last);
        let next = (top as isize - rows).clamp(0, last as isize) as usize;
        self.history_top = next;
        next != top
    }

    /// Furthest scroll: the content's end at the body's bottom edge.
    pub fn max_scroll(&self) -> f32 {
        (self.content_h - self.view_h).max(0.0)
    }
}

/// A control Tab can focus.
fn focusable(target: &ClickTarget) -> bool {
    matches!(
        target,
        ClickTarget::SettingsToggle(_)
            | ClickTarget::SettingsStep(..)
            | ClickTarget::SettingsSegment(..)
            | ClickTarget::SettingsCycle(_)
            | ClickTarget::SettingsSlider(_)
            | ClickTarget::SettingsTheme(_)
            | ClickTarget::SettingsKeybind(_)
            | ClickTarget::SettingsKeybindReset(_)
            | ClickTarget::SettingsReset(_)
            | ClickTarget::SettingsEditPrompt
            | ClickTarget::SettingsOpenConfig
            | ClickTarget::SettingsCopyVersion
            | ClickTarget::RemoteHistoryList
            | ClickTarget::SettingsUpdate
            | ClickTarget::SettingsCheckUpdates
            | ClickTarget::SettingsWhatsNew
            | ClickTarget::SettingsPlaySound(_)
            | ClickTarget::SettingsTestAlert(_)
            | ClickTarget::SettingsRingBell
            | ClickTarget::SettingsNotifyPermission
            | ClickTarget::RemotePair
            | ClickTarget::RemoteViewOnly
            | ClickTarget::RemoteCopyLink
            | ClickTarget::RemoteRevoke(_)
            | ClickTarget::RemoteRelay
            | ClickTarget::RemoteRelayRetry
            | ClickTarget::RemoteRelaySettings
            | ClickTarget::RemotePermission(..)
    )
}

/// The page's keyboard-focusable controls in reading order (top to bottom,
/// left to right): every body control of this frame's `hits`.
pub fn focus_order(hits: &[Hit]) -> Vec<(ClickTarget, [f32; 4])> {
    let mut order: Vec<_> = hits
        .iter()
        .filter(|h| focusable(&h.target))
        .map(|h| (h.target.clone(), h.rect))
        .collect();
    // by row (vertical centre: a row's ↺ and its control differ in height),
    // then left to right
    let key = |r: &[f32; 4]| ((r[1] + r[3] / 2.0).round(), r[0].round());
    order.sort_by(|a, b| {
        key(&a.1)
            .partial_cmp(&key(&b.1))
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

/// A config location as shown to the user: the one actually in use (XDG, or
/// an earlier app name's directory), home shortened to `~`. Resolving it
/// touches the disk, so callers cache the result.
fn shown_path(path: Option<std::path::PathBuf>, fallback: &str) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    path.map(|p| rift_core::config::tilde_path(&p, home.as_deref()))
        .unwrap_or_else(|| fallback.into())
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
    // every focusable control, also scrolled away (page coordinates)
    stops: &mut Vec<(ClickTarget, [f32; 4])>,
) -> f32 {
    names.clear();
    stops.clear();
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
        // typing searches from every page but Keybindings (its own filter
        // takes it): the caret shows there, before the placeholder if empty
        if blink_on && (searching || state.page != Page::Keybindings) {
            let tw = if searching {
                engine.measure(&shown, 12.5, FONT_SANS, Weight::NORMAL)
            } else {
                0.0
            };
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
        // config.toml just written or reloaded: a check at the row's end (a
        // dot on the file icon when the sidebar is icons only)
        let updated = state.saved_until().is_some();
        if updated && compact {
            layer.circle([icon_x + 6.0, foot[1] + 10.0], 6.0, theme.success);
        } else if updated {
            icons::draw(
                layer,
                Icon::Check,
                [foot[0] + foot[2] - 14.0, foot[1] + 16.0],
                14.0,
                theme.success,
            );
        }
        hits.push(Hit {
            rect: foot,
            target: ClickTarget::SettingsOpenConfig,
        });
        let name = if updated {
            "Open config.toml (updated)"
        } else {
            "Open config.toml"
        };
        names.push((foot, name.into()));
    }

    // ── body ──
    // the column centres in the room right of the sidebar (Warp), keeping at
    // least `pad` on each side
    let pad = if compact { 20.0 } else { 48.0 };
    let room = right - (x0 + sidebar_w);
    let card_w = (room - 2.0 * pad).clamp(120.0, BODY_MAX_W);
    let cx = (x0 + sidebar_w + ((room - card_w) / 2.0).max(pad)).round();
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
    // quads too: a row scrolled half under the tab strip must not paint its
    // dot, ↺ or control over it
    layer.set_clip(Some((
        [x0 + sidebar_w, top, aw - sidebar_w, (bottom - top).max(0.0)],
        0.0,
    )));
    // About is the logo alone, centred (Warp): no page title
    let about = !searching && state.page == Page::About;
    // the subtitle wraps (never cut) and pushes the body down
    let sub_lh = 13.0 * 1.35;
    let sub_lines = if about {
        Vec::new()
    } else {
        wrap_lines(engine, subtitle, 13.0, FONT_SANS, card_w)
    };
    let sub_extra = sub_lines.len().saturating_sub(1) as f32 * sub_lh;
    if !about {
        let title = super::palette::elide_to(engine, &title, 22.0, FONT_SANS, card_w);
        layer.text(
            sans(engine, &title, 22.0, Weight::SEMIBOLD),
            cx,
            body_top - state.scroll,
            theme.text_bright,
        );
        for (i, line) in sub_lines.iter().enumerate() {
            layer.text(
                sans(engine, line, 13.0, Weight::NORMAL),
                cx,
                body_top + 34.0 + i as f32 * sub_lh - state.scroll,
                theme.text_dim,
            );
        }
    }

    let body_y = body_top + 70.0 + sub_extra;
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
        clip_bottom: bottom,
        hits,
        names,
        row_label: String::new(),
        reserve: DEFAULT_RESERVE,
        extra_h: 0.0,
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
        pushed: false,
        stops,
        scroll: state.scroll,
    };
    if searching {
        for page in [
            Page::General,
            Page::Appearance,
            Page::Features,
            Page::Terminal,
            Page::Files,
            Page::Notifications,
            Page::Remote,
        ] {
            form.page = page.label();
            match page {
                Page::General => general_page(&mut form, config, &state.update),
                Page::Appearance => appearance_page(&mut form, config),
                Page::Features => features_page(&mut form, config),
                Page::Terminal => terminal_page(&mut form, config),
                Page::Files => files_page(&mut form, config),
                // its switch only: pairing and devices need the page
                Page::Remote => remote_switch(&mut form, config),
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
            Page::General => general_page(&mut form, config, &state.update),
            Page::Features => features_page(&mut form, config),
            Page::Terminal => terminal_page(&mut form, config),
            Page::Files => files_page(&mut form, config),
            Page::Notifications => notifications_page(&mut form, config),
            Page::Keybindings => keybindings_page(&mut form, keymap, config, state, blink_on),
            Page::Themes => themes_page(&mut form, config),
            Page::Remote => remote_page(
                &mut form,
                config,
                &state.remote,
                state.history_top,
                blink_on,
            ),
            Page::About => about_page(&mut form, top, bottom, &state.update),
        }
    }
    // from the body's top edge (the title starts 28 px below it), so
    // content_h - view_h is the furthest scroll
    let content_h = form.y - (top - state.scroll) + 24.0;

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
    layer.set_clip(None);
    for t in &mut layer.texts[text_start..] {
        t.bounds = Some(rift_renderer::glyphon::TextBounds {
            left: (x0 + sidebar_w) as i32,
            top: top as i32,
            right: right as i32,
            bottom: bottom as i32,
        });
    }
    let kept: Vec<Hit> = hits
        .drain(hit_start..)
        .filter(|hh| hh.rect[1] + hh.rect[3] > top && hh.rect[1] < bottom)
        .collect();
    hits.extend(kept);
    // the open list wins over everything under it
    for (i, h) in menu_hits.into_iter().enumerate() {
        hits.insert(i, h);
    }

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
            let t = rift_core::theme::load_by_name(value);
            layer.bordered([tx, cy - 7.0], [14.0, 14.0], t.bg, t.accent, 3.0, 2.0);
            tx += 22.0;
        }
        let ink = if row == dd.highlight {
            theme.text_bright
        } else {
            theme.text_ui
        };
        if dd.id == "font_family" {
            // each family previewed in itself (Warp's font picker)
            let mut b = Buffer::new(&mut engine.font_system, Metrics::new(13.5, 18.0));
            b.set_text(
                &mut engine.font_system,
                label,
                &Attrs::new().family(Family::Name(value)),
                Shaping::Advanced,
                None,
            );
            let bounds = TextBounds {
                left: tx as i32,
                top: ry as i32,
                right: (r[0] + r[2] - 10.0) as i32,
                bottom: (ry + MENU_ROW_H) as i32,
            };
            layer.text_clipped(b, tx, cy - 9.0, ink, bounds);
        } else {
            let label =
                super::palette::elide_to(engine, label, 13.0, FONT_SANS, r[0] + r[2] - 10.0 - tx);
            layer.text(
                sans(engine, &label, 13.0, Weight::NORMAL),
                tx,
                cy - 8.8,
                ink,
            );
        }
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
    /// Height added to the next `label_desc` row (its text is drawn by the
    /// caller below the line).
    extra_h: f32,
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
    /// The last `push_hit` added a hit (`name_last` names only that).
    pushed: bool,
    /// Every focusable control in page coordinates (see `push_hit`).
    stops: &'a mut Vec<(ClickTarget, [f32; 4])>,
    scroll: f32,
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
        let extra = std::mem::take(&mut self.extra_h);
        let f = &self.filter;
        self.hidden =
            !(f.is_empty() || text.to_lowercase().contains(f) || desc.to_lowercase().contains(f));
        if self.hidden {
            return;
        }
        self.rows += 1;
        self.y = y + ROW_H + extra;
        let desc = if self.filter.is_empty() {
            desc.to_string()
        } else {
            self.crumb.clone()
        };
        if self.visible(y, ROW_H + extra) {
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
                y + ROW_H + extra - 1.0,
                self.card_w,
                1.0,
                lift(self.theme, 0.06),
            );
        }
    }

    fn push_hit(&mut self, rect: [f32; 4], target: ClickTarget) {
        // every focusable control of the page, also scrolled away, in page
        // coordinates: Tab walks these and scrolls the one it lands on in
        if !self.hidden && focusable(&target) {
            self.stops.push((
                target.clone(),
                [rect[0], rect[1] + self.scroll, rect[2], rect[3]],
            ));
        }
        self.pushed = self.visible(rect[1], rect[3]);
        if self.pushed {
            self.hits.push(Hit { rect, target });
            self.names.push((rect, self.row_label.clone()));
        }
    }

    /// Accessible name of the control `push_hit` just added (if visible).
    /// A skipped push (search-hidden or scrolled-off row) names nothing:
    /// renaming the previous control would mislabel it.
    fn name_last(&mut self, name: String) {
        if self.pushed
            && let Some(last) = self.names.last_mut()
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
        self.switch(
            y,
            label,
            desc,
            on,
            ClickTarget::SettingsToggle(id),
            Some(id),
        );
    }

    /// A toggle row: `target` is what a click presses; `id` (a config key)
    /// adds the changed dot and ↺.
    fn switch(
        &mut self,
        y: f32,
        label: &str,
        desc: &str,
        on: bool,
        target: ClickTarget,
        id: Option<&'static str>,
    ) {
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
        if let Some(id) = id {
            self.changed(id, y, x);
        }
        self.push_hit([x - 4.0, ty - 4.0, 42.0, 28.0], target);
    }

    /// A 28 px button (DESIGN.md › Buttons) at `[x, y]`: its width.
    fn button(
        &mut self,
        x: f32,
        y: f32,
        label: &str,
        kind: ButtonKind,
        target: ClickTarget,
        name: String,
    ) -> f32 {
        let w = (self.engine.measure(label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0).round();
        let r = [x, y, w, CONTROL_H];
        if self.visible(y, CONTROL_H) {
            let hot = self.hot(r) && kind != ButtonKind::Disabled;
            let theme = self.theme;
            let (fill, edge, ink) = match kind {
                ButtonKind::Primary => (theme.accent, Color::TRANSPARENT, theme.bg),
                ButtonKind::Secondary => (
                    lift(theme, if hot { 0.10 } else { 0.06 }),
                    lift(theme, 0.12),
                    theme.text_ui,
                ),
                ButtonKind::Destructive => (
                    theme.error.mix(Color::BLACK, 0.18),
                    Color::TRANSPARENT,
                    Color::WHITE,
                ),
                ButtonKind::Disabled => (lift(theme, 0.03), lift(theme, 0.08), theme.text_muted),
            };
            self.layer
                .bordered([r[0], r[1]], [r[2], r[3]], fill, edge, 4.0, 1.0);
            self.layer.text(
                sans(self.engine, label, 12.5, Weight::MEDIUM),
                r[0] + 12.0,
                r[1] + (CONTROL_H - 12.5 * 1.35) / 2.0,
                ink,
            );
        }
        // a disabled button is named but presses nothing
        let target = if kind == ButtonKind::Disabled {
            ClickTarget::SettingsNoop
        } else {
            target
        };
        self.push_hit(r, target);
        self.name_last(name);
        w
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

    /// The Rift prompt's chips as they look (sample values), in a box
    /// that opens "Edit prompt".
    fn prompt_preview(&mut self, y: f32, chips: &[rift_core::PromptChip]) {
        self.label_desc("Prompt chips", "Click to choose and order them", y);
        if self.hidden {
            return;
        }
        let r = [self.x, self.y + 4.0, self.card_w, 40.0];
        self.y = r[1] + r[3] + 8.0;
        if self.visible(r[1], r[3]) {
            let hot = self.hot(r);
            self.layer.bordered(
                [r[0], r[1]],
                [r[2], r[3]],
                lift(self.theme, if hot { 0.06 } else { 0.03 }),
                if hot {
                    self.theme.accent
                } else {
                    lift(self.theme, 0.1)
                },
                6.0,
                1.0,
            );
            let views: Vec<_> = chips
                .iter()
                .map(|c| super::prompt_chips::sample(*c, self.theme))
                .collect();
            if views.is_empty() {
                self.layer.text(
                    sans(self.engine, "No chips", 12.5, Weight::NORMAL),
                    r[0] + 12.0,
                    r[1] + 11.0,
                    self.theme.text_muted,
                );
            }
            super::editor::draw_chips(
                self.layer,
                self.engine,
                self.theme,
                [r[0] + 12.0, r[0] + r[2] - 12.0],
                r[1] + 13.0,
                &views,
            );
        }
        self.push_hit(r, ClickTarget::SettingsEditPrompt);
        self.name_last("Edit prompt chips".into());
    }

    fn segment(&mut self, y: f32, label: &str, options: &[&str], active: usize, id: &'static str) {
        self.segment_desc(y, label, "", options, active, id);
    }

    fn segment_desc(
        &mut self,
        y: f32,
        label: &str,
        desc: &str,
        options: &[&str],
        active: usize,
        id: &'static str,
    ) {
        let total = self.segmented_width(options);
        self.reserve = total + 40.0;
        self.label_desc(label, desc, y);
        let sy = y + (ROW_H - CONTROL_H) / 2.0;
        let sx = self.right() - total;
        let targets: Vec<ClickTarget> = (0..options.len())
            .map(|i| ClickTarget::SettingsSegment(id, i as u8))
            .collect();
        self.segmented(sx, sy, label, options, active, &targets);
        self.changed(id, y, sx);
    }

    /// Width of a segmented control of `options`.
    fn segmented_width(&mut self, options: &[&str]) -> f32 {
        options
            .iter()
            .map(|o| self.engine.measure(o, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0)
            .sum()
    }

    /// A segmented control at `[sx, sy]`: option `i` presses `targets[i]`.
    fn segmented(
        &mut self,
        sx: f32,
        sy: f32,
        label: &str,
        options: &[&str],
        active: usize,
        targets: &[ClickTarget],
    ) {
        let widths: Vec<f32> = options
            .iter()
            .map(|o| self.engine.measure(o, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0)
            .collect();
        let total: f32 = widths.iter().sum();
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
            self.push_hit([bx, sy, bw, CONTROL_H], targets[i].clone());
            self.name_last(format!(
                "{label}: {opt}{}",
                if i == active { ", selected" } else { "" }
            ));
            bx += bw;
        }
    }

    /// A one-line text field `[x, y, w]` showing `value` (or `placeholder`,
    /// muted, when empty); `editing` shows its text with a caret and an
    /// accent border. A click presses `target`.
    #[allow(clippy::too_many_arguments)]
    fn text_field(
        &mut self,
        r: [f32; 4],
        value: &str,
        placeholder: &str,
        editing: Option<&str>,
        blink_on: bool,
        target: ClickTarget,
        name: String,
    ) {
        if self.visible(r[1], r[3]) {
            let theme = self.theme;
            let hot = self.hot(r) || editing.is_some();
            self.layer.bordered(
                [r[0], r[1]],
                [r[2], r[3]],
                lift(theme, if hot { 0.07 } else { 0.04 }),
                if editing.is_some() {
                    theme.accent
                } else {
                    lift(theme, 0.12)
                },
                4.0,
                1.0,
            );
            let room = r[2] - 20.0;
            let tx = r[0] + 10.0;
            let ty = r[1] + 5.2;
            match editing {
                Some(text) => {
                    // the tail stays in view (the caret is at the end)
                    let mut shown = text;
                    while !shown.is_empty()
                        && self.engine.measure(shown, 13.0, FONT_SANS, Weight::NORMAL) > room
                    {
                        let mut it = shown.chars();
                        it.next();
                        shown = it.as_str();
                    }
                    if !shown.is_empty() {
                        self.layer.text(
                            sans(self.engine, shown, 13.0, Weight::NORMAL),
                            tx,
                            ty,
                            theme.text_bright,
                        );
                    }
                    if blink_on {
                        let tw = self.engine.measure(shown, 13.0, FONT_SANS, Weight::NORMAL);
                        self.layer
                            .rect([tx + tw, r[1] + 6.0], [1.5, 16.0], theme.accent, 0.0);
                    }
                }
                None => {
                    let (text, color) = if value.is_empty() {
                        (placeholder, theme.text_muted)
                    } else {
                        (value, theme.text_bright)
                    };
                    let shown = super::palette::elide_to(self.engine, text, 13.0, FONT_SANS, room);
                    self.layer.text(
                        sans(self.engine, &shown, 13.0, Weight::NORMAL),
                        tx,
                        ty,
                        color,
                    );
                }
            }
        }
        self.push_hit(r, target);
        self.name_last(name);
    }

    /// A select: the value and a chevron; a click opens its option list.
    fn select(&mut self, y: f32, label: &str, value: &str, id: &'static str) {
        self.select_with(y, label, value, None, false, id);
    }

    /// A sound select with ▷ before it: hear the chosen sound without
    /// opening the list.
    fn sound_select(&mut self, y: f32, label: &str, value: &str, id: &'static str) {
        self.select_with(y, label, value, None, true, id);
    }

    /// A theme select: the theme's ground ringed in its accent, its name.
    fn theme_select(&mut self, y: f32, label: &str, name: &str, id: &'static str) {
        let t = rift_core::theme::load_by_name(name);
        let names = rift_core::theme::available_themes();
        let shown = names
            .iter()
            .position(|n| n == name)
            .and_then(|i| rift_core::theme::picker_labels(&names).into_iter().nth(i))
            .unwrap_or(t.name);
        self.select_with(y, label, &shown, Some((t.bg, t.accent)), false, id);
    }

    fn select_with(
        &mut self,
        y: f32,
        label: &str,
        value: &str,
        swatch: Option<(Color, Color)>,
        play: bool,
        id: &'static str,
    ) {
        let sel_w = (self.card_w * 0.45).clamp(140.0, 220.0);
        self.reserve = sel_w + 40.0 + if play { 32.0 } else { 0.0 };
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
        // ▷ sits between ↺ and the select
        let mut control_x = sx;
        if play {
            let pr = [sx - 32.0, y + (ROW_H - 24.0) / 2.0, 24.0, 24.0];
            if self.visible(pr[1], pr[3]) {
                if self.hot(pr) {
                    self.layer
                        .rect([pr[0], pr[1]], [24.0, 24.0], lift(self.theme, 0.08), 4.0);
                }
                icons::draw(
                    self.layer,
                    Icon::Play,
                    [pr[0] + 12.0, pr[1] + 12.0],
                    14.0,
                    self.theme.text_dim,
                );
            }
            self.push_hit(pr, ClickTarget::SettingsPlaySound(id));
            self.name_last(format!("Play {label}: {value}"));
            control_x = pr[0];
        }
        self.changed(id, y, control_x);
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
        // read-only, but named for assistive tech
        self.push_hit(
            [self.right() - 260.0, y, 260.0, ROW_H],
            ClickTarget::SettingsNoop,
        );
        self.name_last(format!("{label}: {value}"));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonKind {
    Primary,
    Secondary,
    Destructive,
    Disabled,
}

/// Settings › Remote access's switch (also what the search finds).
fn remote_switch(form: &mut Form, config: &Config) {
    form.group("Remote access");
    let y = form.y;
    form.toggle_desc(
        y,
        "Allow remote access",
        "Paired phones can see your tabs and run commands in them",
        config.remote_access,
        "remote_access",
    );
}

/// Settings › Remote access: the switch and the gateway's state, the relay,
/// pairing (QR code, link, expiry), the paired devices (Operate / View
/// only, Revoke) and the recent connections.
fn remote_page(
    form: &mut Form,
    config: &Config,
    view: &RemoteView,
    history_top: usize,
    blink_on: bool,
) {
    remote_switch(form, config);
    let on = config.remote_access;
    if on {
        let y = form.y;
        form.toggle_desc(
            y,
            "Phones can open tabs while Rift is closed",
            if config.durable_sessions {
                "Only phones that may run commands; you get a notification, and the tabs appear when you open Rift"
            } else {
                "Needs Keep shells running through updates (General)"
            },
            config.remote_when_closed,
            "remote_when_closed",
        );
    }

    connection_rows(form, config, view);
    relay_settings(form, config, view, blink_on);

    form.group("Pair a phone");
    let label = "Pair a phone";
    let bw = form.engine.measure(label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0;
    form.reserve = bw + 40.0;
    let y = form.y;
    form.label_desc(
        "Pairing code",
        if on {
            "Scan the code with the Rift app · one use, valid 2 minutes"
        } else {
            "Allow remote access first"
        },
        y,
    );
    if !form.hidden {
        let (kind, name) = if on {
            (ButtonKind::Primary, label.to_string())
        } else {
            (
                ButtonKind::Disabled,
                format!("{label}, unavailable while remote access is off"),
            )
        };
        let by = y + (ROW_H - CONTROL_H) / 2.0;
        let bx = form.right() - bw.round();
        form.button(bx, by, label, kind, ClickTarget::RemotePair, name);
    }
    let y = form.y;
    form.switch(
        y,
        "View only",
        "The phone can watch, but not run commands or send keys",
        view.view_only,
        ClickTarget::RemoteViewOnly,
        None,
    );
    if on && view.pairing_wait && view.pairing.is_none() {
        let y = form.y;
        let text = match &view.gateway {
            GatewayLine::Listening(a) if a.is_empty() => "No network address to pair over",
            GatewayLine::Listening(_) | GatewayLine::Connected(_)
                if config.remote_tunnel && view.cloudflared.is_none() =>
            {
                "Install cloudflared first"
            }
            GatewayLine::Listening(_) | GatewayLine::Connected(_) if config.remote_tunnel => {
                "Waiting for the tunnel…"
            }
            _ => "Waiting for the gateway to start…",
        };
        form.display(y, "Pairing", text);
    }
    if let Some(p) = view.pairing.as_ref().filter(|_| on) {
        pairing_block(form, p);
    }

    form.group("Paired devices");
    if let Some(e) = &view.devices_error {
        let y = form.y;
        form.display(y, "Paired devices", e);
    } else if view.devices.is_empty() && view.devices_loaded {
        let y = form.y;
        form.label_desc(
            "No paired devices",
            "Pair a phone above; it shows here until you revoke it",
            y,
        );
    }
    const ACCESS: [&str; 2] = ["Operate", "View only"];
    for d in &view.devices {
        let y = form.y;
        let bw = form
            .engine
            .measure("Revoke", 12.5, FONT_SANS, Weight::MEDIUM)
            + 24.0;
        let sw = form.segmented_width(&ACCESS);
        form.reserve = bw + 8.0 + sw + 40.0;
        // how it is connected must never be cut: when the line does not fit
        // beside the controls, it wraps under them
        let beside = form.card_w - form.reserve - 8.0;
        let fits = !form.filter.is_empty()
            || form
                .engine
                .measure(&d.detail, 12.0, FONT_SANS, Weight::NORMAL)
                <= beside;
        let below = if fits {
            Vec::new()
        } else {
            wrap_lines(form.engine, &d.detail, 12.0, FONT_SANS, form.card_w)
        };
        form.extra_h = if below.is_empty() {
            0.0
        } else {
            below.len() as f32 * NOTE_LH + 2.0
        };
        form.label_desc(&d.name, if fits { &d.detail } else { "" }, y);
        if !form.hidden && form.visible(y, ROW_H) {
            for (i, l) in below.iter().enumerate() {
                form.layer.text(
                    sans(form.engine, l, 12.0, Weight::NORMAL),
                    form.x,
                    y + ROW_H - 4.0 + i as f32 * NOTE_LH,
                    form.theme.text_muted,
                );
            }
        }
        if form.visible(y, ROW_H) {
            // how it is connected: green when it is, grey when not
            let color = match d.path {
                DevicePath::None => form.theme.text_muted,
                DevicePath::Lan | DevicePath::Relay => form.theme.success,
            };
            form.layer
                .circle([form.label_end + 12.0, form.label_mid], 8.0, color);
        }
        if !form.hidden {
            let by = y + (ROW_H - CONTROL_H) / 2.0;
            let bx = form.right() - bw.round();
            let targets = [
                ClickTarget::RemotePermission(d.id.clone(), false),
                ClickTarget::RemotePermission(d.id.clone(), true),
            ];
            let label = format!("{} access", d.name);
            form.segmented(
                (bx - 8.0 - sw).round(),
                by,
                &label,
                &ACCESS,
                usize::from(d.view_only),
                &targets,
            );
            form.button(
                bx,
                by,
                "Revoke",
                ButtonKind::Destructive,
                ClickTarget::RemoteRevoke(d.id.clone()),
                format!("Revoke {}", d.name),
            );
        }
    }

    form.group("Recent connections");
    if view.history.is_empty() && view.devices_loaded {
        let y = form.y;
        form.label_desc(
            "No connections yet",
            "Phones that connect, and refused attempts, show here",
            y,
        );
    }
    // at most HISTORY_SHOWN rows at once; more scroll inside the list by
    // whole rows, with a thumb at its right edge
    let n = view.history.len();
    let scrolls = n > HISTORY_SHOWN;
    let top = history_top.min(n.saturating_sub(HISTORY_SHOWN));
    let list_y = form.y;
    for h in view.history.iter().skip(top).take(HISTORY_SHOWN) {
        let y = form.y;
        form.reserve = if scrolls { 16.0 } else { 0.0 };
        form.label_desc(&h.name, &h.detail, y);
        // read-only, but named for assistive tech
        form.push_hit([form.x, y, form.card_w, ROW_H], ClickTarget::SettingsNoop);
        form.name_last(format!("{}, {}", h.name, h.detail));
    }
    if scrolls {
        let list_h = form.y - list_y;
        let track = [form.right() - 4.0, list_y + 4.0, 3.0, list_h - 8.0];
        let thumb_h = (track[3] * HISTORY_SHOWN as f32 / n as f32).max(16.0);
        let room = track[3] - thumb_h;
        let thumb_y = track[1] + room * top as f32 / (n - HISTORY_SHOWN) as f32;
        if form.visible(list_y, list_h) {
            let theme = form.theme;
            form.layer.rect(
                [track[0], track[1]],
                [track[2], track[3]],
                lift(theme, 0.05),
                1.5,
            );
            form.layer.rect(
                [track[0], thumb_y],
                [track[2], thumb_h],
                lift(theme, 0.22),
                1.5,
            );
        }
        // after the rows: a click finds a row first, the wheel finds this;
        // a Tab stop too, where ↑ ↓ scroll it
        form.push_hit(
            [form.x, list_y, form.card_w, list_h],
            ClickTarget::RemoteHistoryList,
        );
        form.name_last(format!(
            "Recent connections, {} to {} of {n}; up and down arrows scroll",
            top + 1,
            top + HISTORY_SHOWN
        ));
    }
}

/// "Use a Cloudflare tunnel instead" (`remote_tunnel`) and, while it is on, its
/// checklist: cloudflared (installed, else Install / Download) and the
/// tunnel's state.
fn tunnel_rows(form: &mut Form, config: &Config, view: &RemoteView) {
    let y = form.y;
    form.toggle_desc(
        y,
        "Use a Cloudflare tunnel instead",
        "A free tunnel from this computer replaces the relay; its address changes on every restart",
        config.remote_tunnel,
        "remote_tunnel",
    );
    if !config.remote_tunnel {
        return;
    }
    let y = form.y;
    match &view.cloudflared {
        Some(path) => {
            form.reserve = 40.0;
            form.label_desc("cloudflared", &format!("Installed · {}", path.display()), y);
            if form.visible(y, ROW_H) {
                let c = [form.right() - 8.0, y + ROW_H / 2.0];
                icons::draw(form.layer, Icon::Check, c, 16.0, form.theme.success);
            }
            form.push_hit([form.x, y, form.card_w, ROW_H], ClickTarget::SettingsNoop);
            form.name_last(format!("cloudflared: installed at {}", path.display()));
        }
        None => {
            let label = if view.brew { "Install" } else { "Download" };
            let bw = form.engine.measure(label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0;
            form.reserve = bw + 40.0;
            form.label_desc(
                "cloudflared",
                "Not installed · the tunnel needs it (free, by Cloudflare)",
                y,
            );
            if !form.hidden {
                let by = y + (ROW_H - CONTROL_H) / 2.0;
                let bx = form.right() - bw.round();
                form.button(
                    bx,
                    by,
                    label,
                    ButtonKind::Primary,
                    ClickTarget::RemoteInstallTunnel,
                    format!("{label} cloudflared"),
                );
            }
        }
    }
    let y = form.y;
    let state = tunnel_text(view);
    form.reserve = 0.0;
    form.label_desc("Tunnel", &state, y);
    form.push_hit([form.x, y, form.card_w, ROW_H], ClickTarget::SettingsNoop);
    form.name_last(format!("Tunnel: {state}"));
}

/// The Tunnel row's state.
pub fn tunnel_text(view: &RemoteView) -> String {
    use crate::tunnel::TunnelState;
    match &view.tunnel {
        _ if view.cloudflared.is_none() => "Waiting for cloudflared".into(),
        None | Some(TunnelState::Starting) => "Starting…".into(),
        Some(TunnelState::Missing) => "Waiting for cloudflared".into(),
        Some(TunnelState::Ready { url }) => format!(
            "Ready · {} · pair phones again after a restart",
            url.trim_start_matches("wss://")
        ),
        Some(TunnelState::Failed { detail }) => format!("Failed, retrying: {detail}"),
    }
}

/// Placeholder of the empty relay field.
pub const RELAY_PLACEHOLDER: &str = "wss://relay.example.com";

/// What marks a Connection row: a badge left of the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// Working (check, `success`).
    Ok,
    /// On its way (…, `accent`).
    Wait,
    /// Failed (×, `error`).
    Failed,
    /// Off (–, muted).
    Off,
}

/// One row of Settings › Remote access › Connection.
struct StatusRow<'a> {
    mark: Mark,
    label: &'a str,
    /// Under the label, wrapped; `error` colours it.
    note: &'a str,
    error: bool,
    /// Right of the label (an address, a host), monospace.
    right: &'a str,
    /// The whole error text in a block under the note, wrapped.
    detail: Option<&'a str>,
    /// A "Try again" button under `right`.
    retry: bool,
}

/// Badge width and its gap to the text.
const BADGE_W: f32 = 20.0;
const BADGE_GAP: f32 = 12.0;
/// Line heights of the note (12 px) and the error block (11.5 px mono).
const NOTE_LH: f32 = 12.0 * 1.35;
const DETAIL_LH: f32 = 11.5 * 1.35;

/// `text` broken into lines no wider than `max_w`: at spaces, and inside a
/// word wider than a line (a URL, a path).
fn wrap_lines(
    engine: &mut TextEngine,
    text: &str,
    size: f32,
    family: &'static str,
    max_w: f32,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_ascii_whitespace() {
        let next = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if engine.measure(&next, size, family, Weight::NORMAL) <= max_w {
            cur = next;
            continue;
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        // the word alone is too wide: cut it by characters
        for c in word.chars() {
            let mut grown = cur.clone();
            grown.push(c);
            if !cur.is_empty() && engine.measure(&grown, size, family, Weight::NORMAL) > max_w {
                lines.push(std::mem::replace(&mut cur, c.to_string()));
            } else {
                cur = grown;
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// A Connection row: badge, label, wrapped note (and the whole error in a
/// block), the address or host on the right and, after a failure, Try
/// again. Taller than a plain row when the text needs it.
fn status_row(form: &mut Form, s: &StatusRow) {
    let y = form.y;
    form.hidden = false;
    form.rows += 1;
    form.row_label = s.label.to_string();
    let theme = form.theme;
    let text_x = form.x + BADGE_W + BADGE_GAP;
    let right = super::palette::elide_to(form.engine, s.right, 12.5, mono_family(), 240.0);
    let right_w = form
        .engine
        .measure(&right, 12.5, mono_family(), Weight::NORMAL);
    let retry = "Try again";
    let retry_w = (form.engine.measure(retry, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0).round();
    let side = right_w.max(if s.retry { retry_w } else { 0.0 });
    let col_w = (form.right() - text_x - side - 16.0).max(120.0);
    let note = wrap_lines(form.engine, s.note, 12.0, FONT_SANS, col_w);
    let note_h = note.len() as f32 * NOTE_LH;
    let detail = s
        .detail
        .map(|d| wrap_lines(form.engine, d, 11.5, mono_family(), col_w - 20.0));
    let detail_h = detail
        .as_ref()
        .map_or(0.0, |l| l.len() as f32 * DETAIL_LH + 16.0 + 6.0);
    let mut h = (28.0 + note_h + detail_h + 8.0).round().max(ROW_H);
    if s.retry {
        // label line, the host, then the button under it
        h = h.max(38.0 + CONTROL_H + 10.0);
    }
    if form.visible(y, h) {
        let (fill, ink) = match s.mark {
            Mark::Ok => (theme.success.with_alpha(0.15), theme.success),
            Mark::Wait => (theme.accent.with_alpha(0.15), theme.accent),
            Mark::Failed => (theme.error.with_alpha(0.15), theme.error),
            Mark::Off => (lift(theme, 0.08), theme.text_dim),
        };
        let c = [form.x + BADGE_W / 2.0, y + 9.0 + 8.8];
        form.layer.circle(c, BADGE_W, fill);
        match s.mark {
            Mark::Ok => icons::draw(form.layer, Icon::Check, c, 12.0, ink),
            Mark::Failed => icons::draw(form.layer, Icon::Close, c, 10.0, ink),
            Mark::Wait | Mark::Off => {
                let glyph = if s.mark == Mark::Wait { "…" } else { "–" };
                let gw = form.engine.measure(glyph, 12.0, FONT_SANS, Weight::NORMAL);
                form.layer.text(
                    sans(form.engine, glyph, 12.0, Weight::NORMAL),
                    c[0] - gw / 2.0,
                    c[1] - 8.0,
                    ink,
                );
            }
        }
        form.layer.text(
            sans(form.engine, s.label, 13.0, Weight::NORMAL),
            text_x,
            y + 9.0,
            theme.text_ui,
        );
        let note_ink = match (s.error, s.mark) {
            (true, _) => theme.error,
            (_, Mark::Wait) => theme.accent,
            _ => theme.text_muted,
        };
        for (i, l) in note.iter().enumerate() {
            form.layer.text(
                sans(form.engine, l, 12.0, Weight::NORMAL),
                text_x,
                y + 28.0 + i as f32 * NOTE_LH,
                note_ink,
            );
        }
        if let Some(lines) = &detail {
            let top = y + 28.0 + note_h + 6.0;
            let bh = lines.len() as f32 * DETAIL_LH + 16.0;
            form.layer.rect([text_x, top], [col_w, bh], theme.bg, 4.0);
            for (i, l) in lines.iter().enumerate() {
                form.layer.text(
                    mono(form.engine, l, 11.5, Weight::NORMAL),
                    text_x + 10.0,
                    top + 8.0 + i as f32 * DETAIL_LH,
                    theme.error,
                );
            }
        }
        if !right.is_empty() {
            form.layer.text(
                mono(form.engine, &right, 12.5, Weight::NORMAL),
                form.right() - right_w,
                y + 9.0,
                theme.text_ui,
            );
        }
        form.layer
            .hline(form.x, y + h - 1.0, form.card_w, 1.0, lift(theme, 0.06));
    }
    if s.retry {
        form.button(
            form.right() - retry_w,
            y + 38.0,
            retry,
            ButtonKind::Secondary,
            ClickTarget::RemoteRelayRetry,
            "Try again: retry the relay now".into(),
        );
    }
    // read-only, but named for assistive tech: the whole text
    let mut name = s.label.to_string();
    if !s.right.is_empty() {
        name.push_str(&format!(" ({})", s.right));
    }
    name.push_str(&format!(": {}", s.note));
    if let Some(d) = s.detail {
        name.push_str(&format!(
            "{} {d}",
            if name.ends_with('.') { "" } else { "." }
        ));
    }
    form.push_hit(
        [form.x, y, form.right() - side - 8.0 - form.x, h],
        ClickTarget::SettingsNoop,
    );
    form.name_last(name);
    form.y = y + h;
}

/// Settings › Remote access › Connection: how phones reach this computer.
/// "This network" (the listening address), "Relay" (connected, connecting,
/// off or failing, with the whole error and Try again), and a line on
/// encryption.
fn connection_rows(form: &mut Form, config: &Config, view: &RemoteView) {
    let on = config.remote_access;
    form.group("Connection");
    let addresses = view.addresses.join(", ");
    let (mark, note, right) = match (&view.gateway, on) {
        (_, false) | (GatewayLine::Off, _) => (Mark::Off, "Off".to_string(), ""),
        (GatewayLine::Starting, _) => (Mark::Wait, "Starting…".to_string(), ""),
        _ if view.addresses.is_empty() => {
            (Mark::Off, "Listening, but on no network".to_string(), "")
        }
        _ => (
            Mark::Ok,
            "Phones on the same Wi-Fi connect directly".to_string(),
            addresses.as_str(),
        ),
    };
    status_row(
        form,
        &StatusRow {
            mark,
            label: "This network",
            note: &note,
            error: false,
            right,
            detail: None,
            retry: false,
        },
    );

    // the tunnel replaces the relay: its state is the relay's
    let tunnel_relay = on && config.remote_tunnel;
    let tunnel_ready = matches!(view.tunnel, Some(crate::tunnel::TunnelState::Ready { .. }));
    let line = &view.relay;
    let (mark, note, error) = if !on {
        let note = if config.relay().is_none() {
            RelayLine::Off.text()
        } else {
            "Off while remote access is off".to_string()
        };
        (Mark::Off, note, false)
    } else if tunnel_relay && !tunnel_ready {
        (Mark::Wait, "Waiting for the tunnel".to_string(), false)
    } else {
        let mark = match line {
            RelayLine::Off | RelayLine::Scripted => Mark::Off,
            RelayLine::Connecting(_) => Mark::Wait,
            RelayLine::Connected { .. } => Mark::Ok,
            RelayLine::Error { .. } => Mark::Failed,
        };
        let note = if tunnel_relay {
            format!("{} · the tunnel's relay", line.text())
        } else {
            line.text()
        };
        (mark, note, matches!(line, RelayLine::Error { .. }))
    };
    let shown = on && !(tunnel_relay && !tunnel_ready);
    let host = line.host().filter(|_| shown).unwrap_or("");
    status_row(
        form,
        &StatusRow {
            mark,
            label: "Relay",
            note: &note,
            error,
            right: match (host.is_empty(), mark) {
                (false, _) => host,
                (true, Mark::Off) => "—",
                (true, _) => "",
            },
            detail: line.reason().filter(|_| shown),
            retry: shown && error,
        },
    );

    let y = form.y + 10.0;
    let text = "End-to-end encrypted: the relay passes bytes it cannot read.";
    if form.visible(y, 20.0) {
        icons::draw(
            form.layer,
            Icon::Lock,
            [form.x + 6.0, y + 8.0],
            12.0,
            form.theme.text_muted,
        );
        form.layer.text(
            sans(form.engine, text, 12.0, Weight::NORMAL),
            form.x + 20.0,
            y,
            form.theme.text_muted,
        );
    }
    form.hidden = false;
    form.row_label = "Encryption".into();
    form.push_hit([form.x, y, form.card_w, 20.0], ClickTarget::SettingsNoop);
    form.name_last(text.into());
    form.y = y + 20.0;
}

/// "Relay settings…" (or "Hide relay settings") and, open, the relay
/// address and the Cloudflare tunnel.
fn relay_settings(form: &mut Form, config: &Config, view: &RemoteView, blink_on: bool) {
    let y = form.y + 8.0;
    let label = if view.relay_open {
        "Hide relay settings"
    } else {
        "Relay settings…"
    };
    let w = form.engine.measure(label, 13.0, FONT_SANS, Weight::NORMAL);
    // padded, so the focus ring clears the text
    let r = [form.x - 6.0, y, w.round() + 12.0, CONTROL_H];
    form.hidden = false;
    form.row_label = "Relay settings".into();
    if form.visible(y, CONTROL_H) {
        form.layer.text(
            sans(form.engine, label, 13.0, Weight::NORMAL),
            form.x,
            y + (CONTROL_H - 13.0 * 1.35) / 2.0,
            form.theme.accent,
        );
    }
    form.push_hit(r, ClickTarget::RemoteRelaySettings);
    form.name_last(format!(
        "{label}, {}",
        if view.relay_open {
            "expanded"
        } else {
            "collapsed"
        }
    ));
    form.y = y + CONTROL_H + 4.0;
    if !view.relay_open {
        return;
    }
    // the tunnel's relay has no address to edit
    if !(config.remote_access && config.remote_tunnel) {
        relay_row(form, config, view, blink_on);
    }
    if config.remote_access {
        tunnel_rows(form, config, view);
    }
}

/// The relay address field (`remote_relay`).
fn relay_row(form: &mut Form, config: &Config, view: &RemoteView, blink_on: bool) {
    let y = form.y;
    let fw = (form.card_w * 0.45).clamp(160.0, 280.0);
    form.reserve = fw + 40.0;
    let state = if config.remote_access {
        view.relay.text()
    } else if config.relay().is_none() {
        "Off".to_string()
    } else {
        "Off while remote access is off".to_string()
    };
    form.label_desc(
        "Relay address",
        "Rift's relay by default; your own with packaging/relay.Dockerfile",
        y,
    );
    if form.hidden {
        return;
    }
    let fx = form.right() - fw;
    let r = [fx, y + (ROW_H - CONTROL_H) / 2.0, fw, CONTROL_H];
    form.changed("remote_relay", y, fx);
    let value = config.remote_relay.trim();
    let name = match &view.relay_edit {
        Some(t) => format!("Relay URL, editing: {t}"),
        None if value.is_empty() => "Relay URL: none".to_string(),
        None => format!("Relay URL: {value}"),
    };
    form.text_field(
        r,
        value,
        RELAY_PLACEHOLDER,
        view.relay_edit.as_deref(),
        blink_on,
        ClickTarget::RemoteRelay,
        format!("{name}. {state}"),
    );
}

/// The live pairing: the QR code (black modules on a white plate with its
/// quiet zone, so it scans in dark themes too), the expiry countdown and
/// the link with a Copy button; once expired, "Expired — Pair again".
fn pairing_block(form: &mut Form, p: &PairingView) {
    let left = p
        .expires_at
        .duration_since(std::time::SystemTime::now())
        .unwrap_or_default();
    let theme = form.theme;
    let top = form.y + 12.0;
    if left.is_zero() {
        let r = [form.x, top, form.card_w, ROW_H];
        if form.visible(r[1], r[3]) {
            form.layer.text(
                sans(form.engine, "Expired — Pair again", 13.0, Weight::NORMAL),
                r[0],
                r[1] + 6.0,
                theme.warning,
            );
        }
        let w = form
            .engine
            .measure("Expired — Pair again", 13.0, FONT_SANS, Weight::NORMAL);
        form.button(
            (r[0] + w + 12.0).round(),
            r[1],
            "Pair again",
            ButtonKind::Secondary,
            ClickTarget::RemotePair,
            "Expired, pair again".into(),
        );
        form.y = top + CONTROL_H + 12.0;
        return;
    }
    let n = p.qr.len();
    let plate = ((n + 2 * QR_QUIET) as f32 * QR_MODULE).round();
    let (px, py) = (form.x.round(), top.round());
    if form.visible(py, plate) && n > 0 {
        form.layer.rect([px, py], [plate, plate], Color::WHITE, 4.0);
        let o = QR_QUIET as f32 * QR_MODULE;
        for (r, row) in p.qr.iter().enumerate() {
            // one quad per run of dark modules
            let mut c = 0;
            while c < row.len() {
                if !row[c] {
                    c += 1;
                    continue;
                }
                let start = c;
                while c < row.len() && row[c] {
                    c += 1;
                }
                form.layer.rect(
                    [
                        px + o + start as f32 * QR_MODULE,
                        py + o + r as f32 * QR_MODULE,
                    ],
                    [(c - start) as f32 * QR_MODULE, QR_MODULE],
                    Color::BLACK,
                    0.0,
                );
            }
        }
    }
    // read-only, but named for assistive tech
    form.push_hit([px, py, plate, plate], ClickTarget::SettingsNoop);
    form.name_last("Pairing QR code".into());
    // beside the code when there is room, else under it
    let side = form.card_w - plate - 24.0 >= 240.0;
    let (tx, mut ty, tw) = if side {
        (px + plate + 24.0, py, form.card_w - plate - 24.0)
    } else {
        (form.x, py + plate + 16.0, form.card_w)
    };
    let expires = format!("Expires in {}", countdown(left));
    if form.visible(ty, 20.0) {
        form.layer.text(
            sans(form.engine, "Scan with the Rift app", 13.0, Weight::NORMAL),
            tx,
            ty,
            theme.text_ui,
        );
        form.layer.text(
            sans(form.engine, &expires, 12.5, Weight::NORMAL),
            tx,
            ty + 22.0,
            theme.text_dim,
        );
    }
    form.push_hit([tx, ty, tw, 40.0], ClickTarget::SettingsNoop);
    form.name_last(expires);
    ty += 52.0;
    // the link, wrapped by characters (it has no spaces)
    let cw = form
        .engine
        .measure("0000000000", 11.5, mono_family(), Weight::NORMAL)
        / 10.0;
    let per = ((tw / cw).floor() as usize).max(8);
    let chars: Vec<char> = p.url.chars().collect();
    let lines: Vec<String> = chars.chunks(per).map(|c| c.iter().collect()).collect();
    let lh = 11.5 * 1.35;
    for (i, l) in lines.iter().enumerate() {
        let ly = ty + i as f32 * lh;
        if form.visible(ly, lh) {
            form.layer.text(
                mono(form.engine, l, 11.5, Weight::NORMAL),
                tx,
                ly,
                theme.text_dim,
            );
        }
    }
    let link_h = lines.len() as f32 * lh;
    form.push_hit([tx, ty, tw, link_h], ClickTarget::SettingsNoop);
    form.name_last(format!("Pairing link: {}", p.url));
    ty += link_h + 12.0;
    form.button(
        tx,
        ty,
        "Copy link",
        ButtonKind::Secondary,
        ClickTarget::RemoteCopyLink,
        "Copy pairing link".into(),
    );
    ty += CONTROL_H;
    form.y = (py + plate).max(ty) + 12.0;
}

/// `10000` → `10,000` (design 05 scrollback field).
/// Settings › About: the app icon and wordmark, the version (with a copy
/// button) and the copyright, centred in the body.
fn about_page(form: &mut Form, top: f32, bottom: f32, update: &UpdateLine) {
    const LOGO: f32 = 64.0;
    const WORD: f32 = 52.0;
    const GAP: f32 = 18.0;
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let word_w = form
        .engine
        .measure("Rift", WORD, FONT_SANS, Weight::SEMIBOLD);
    let version_w = form
        .engine
        .measure(&version, 13.0, FONT_SANS, Weight::NORMAL);
    // the update line (status and its button) takes a row under the version
    let height = LOGO + 32.0 + 26.0 + 34.0 + 20.0;
    let mid = form.x + form.card_w / 2.0;
    // a little above the middle, as Warp sits it; scrolls when too short
    let y = (top + (bottom - top - height) * 0.45).max(form.y + 28.0);
    let theme = form.theme;

    let lx = (mid - (LOGO + GAP + word_w) / 2.0).round();
    super::welcome::logo(form.layer, [lx, y], LOGO);
    form.layer.text(
        sans(form.engine, "Rift", WORD, Weight::SEMIBOLD),
        lx + LOGO + GAP,
        y + LOGO / 2.0 - WORD * 1.3 / 2.0,
        theme.text_bright,
    );

    // version, then a copy button beside it; the pair centres together
    let vy = y + LOGO + 32.0;
    let vx = (mid - (version_w + 6.0 + 24.0) / 2.0).round();
    form.layer.text(
        sans(form.engine, &version, 13.0, Weight::NORMAL),
        vx,
        vy,
        theme.text_ui,
    );
    let copy = [vx + version_w + 6.0, vy - 3.0, 24.0, 24.0];
    if form.hot(copy) {
        form.layer.rect(
            [copy[0], copy[1]],
            [copy[2], copy[3]],
            lift(theme, 0.07),
            4.0,
        );
    }
    icons::draw(
        form.layer,
        Icon::Copy,
        [copy[0] + 12.0, copy[1] + 12.0],
        14.0,
        theme.text_muted,
    );
    form.push_hit(copy, ClickTarget::SettingsCopyVersion);
    form.name_last(format!("Copy version {version}"));

    // update status ("Up to date", "vX available", why not), then "Update…"
    // for an available version or "Check now" (no button while checking)
    let uy = vy + 28.0;
    let status = update.text().unwrap_or_default();
    let available = matches!(update, UpdateLine::Available(_));
    let color = if available {
        theme.warning
    } else {
        theme.text_muted
    };
    let sw = if status.is_empty() {
        0.0
    } else {
        form.engine
            .measure(&status, 13.0, FONT_SANS, Weight::NORMAL)
    };
    // no button while something is under way
    let button = !update.busy();
    let label = if available { "Update…" } else { "Check now" };
    let bw = if button {
        form.engine.measure(label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0
    } else {
        0.0
    };
    let gap = if sw > 0.0 && button { 10.0 } else { 0.0 };
    let sx = (mid - (sw + gap + bw) / 2.0).round();
    if !status.is_empty() {
        form.layer.text(
            sans(form.engine, &status, 13.0, Weight::NORMAL),
            sx,
            uy + 4.0,
            color,
        );
        if !available {
            // read-only, but named for assistive tech
            form.push_hit([sx, uy, sw, 26.0], ClickTarget::SettingsNoop);
            form.name_last(status.clone());
        }
    }
    if button {
        // a secondary button (DESIGN.md): 6 % fill, 12 % hairline
        let r = [sx + sw + gap, uy - 1.0, bw.round(), CONTROL_H];
        let hot = form.hot(r);
        form.layer.bordered(
            [r[0], r[1]],
            [r[2], r[3]],
            lift(theme, if hot { 0.10 } else { 0.06 }),
            lift(theme, 0.12),
            4.0,
            1.0,
        );
        form.layer.text(
            sans(form.engine, label, 12.5, Weight::MEDIUM),
            r[0] + 12.0,
            r[1] + (CONTROL_H - 12.5 * 1.35) / 2.0,
            theme.text_ui,
        );
        if available {
            form.push_hit(r, ClickTarget::SettingsUpdate);
            form.name_last(format!("Update to {status}"));
        } else {
            form.push_hit(r, ClickTarget::SettingsCheckUpdates);
            form.name_last("Check for updates now".to_string());
        }
    }

    let copyright = "Copyright 2026 botjaeger · MIT License";
    let cw = form
        .engine
        .measure(copyright, 13.0, FONT_SANS, Weight::NORMAL);
    form.layer.text(
        sans(form.engine, copyright, 13.0, Weight::NORMAL),
        (mid - cw / 2.0).round(),
        vy + 26.0 + 34.0,
        theme.text_dim,
    );
    form.y = y + height;
}

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
        &format!("{:.0}", config.font_size()),
        "px",
        "font_size",
    );
    let y = form.y;
    form.slider(
        y,
        "Line height",
        (config.line_height() - 1.0) / 1.2,
        &format!("{:.2}", config.line_height()),
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

fn general_page(form: &mut Form, config: &Config, update: &UpdateLine) {
    form.group("Shell");
    // what actually runs: a configured shell that is not executable falls back
    let shell = rift_core::shell_integration::resolve_shell(config.shell.as_deref());
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
        "Restore tabs after a crash",
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
        "Keep shells running through updates",
        "Session service; updates and crashes reattach (new shells)",
        config.durable_sessions,
        "durable_sessions",
    );

    form.group("Updates");
    let y = form.y;
    form.toggle_desc(
        y,
        "Check for updates",
        "At launch and every 6 hours; the update downloads, then Update Rift appears in the tab bar",
        config.check_for_updates,
        "check_for_updates",
    );
    let y = form.y;
    let label = "Check now";
    let bw = form.engine.measure(label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0;
    form.reserve = bw + 40.0;
    let current = format!("Rift {}", env!("CARGO_PKG_VERSION"));
    let state = update.text().unwrap_or_else(|| "Not checked yet".into());
    form.label_desc(&current, &state, y);
    if !form.hidden {
        let by = y + (ROW_H - CONTROL_H) / 2.0;
        let bx = form.right() - bw.round();
        form.button(
            bx,
            by,
            label,
            if update.busy() {
                ButtonKind::Disabled
            } else {
                ButtonKind::Secondary
            },
            ClickTarget::SettingsCheckUpdates,
            "Check for updates now".into(),
        );
    }
    let y = form.y;
    let label = "Open";
    let bw = form.engine.measure(label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0;
    form.reserve = bw + 40.0;
    form.label_desc("What's New", "The changes in each version of Rift", y);
    if !form.hidden {
        let by = y + (ROW_H - CONTROL_H) / 2.0;
        let bx = form.right() - bw.round();
        form.button(
            bx,
            by,
            label,
            ButtonKind::Secondary,
            ClickTarget::SettingsWhatsNew,
            "Open What's New".into(),
        );
    }

    form.group("Automation");
    let y = form.y;
    form.segment_desc(
        y,
        "Control socket",
        "Lets scripts control Rift (rift-terminal ctl); Full also lets them type",
        &["Off", "Navigate", "Full"],
        match config.control_api.as_str() {
            "off" => 0,
            "full" => 2,
            _ => 1,
        },
        "control_api",
    );
}

fn terminal_page(form: &mut Form, config: &Config) {
    form.group("Mouse");
    let y = form.y;
    form.toggle(
        y,
        "Mouse reporting to programs (Shift selects)",
        config.mouse_reporting,
        "mouse_reporting",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Middle-click to paste",
        "Pastes the clipboard into the pane under the pointer",
        config.middle_click_paste,
        "middle_click_paste",
    );
    let y = form.y;
    form.segment_desc(
        y,
        "Clicking a link",
        "Show where it goes first, or open it at once (⌘-click always opens)",
        &["Show it", "Open it"],
        usize::from(config.link_click.trim() == "open"),
        "link_click",
    );
    let y = form.y;
    form.stepper_desc(
        y,
        "Lines per wheel notch",
        "How far one mouse-wheel step scrolls the terminal",
        &config.wheel_lines().to_string(),
        "lines",
        "wheel_scroll_lines",
    );

    form.group("Clipboard");
    let y = form.y;
    form.toggle(y, "Copy on select", config.copy_on_select, "copy_on_select");
    let y = form.y;
    form.toggle(
        y,
        "Ask before pasting lines that would run",
        config.confirm_multiline_paste,
        "confirm_multiline_paste",
    );
    let y = form.y;
    form.segment_desc(
        y,
        "Programs' clipboard access (OSC 52)",
        "Read lets any program or ssh host see your clipboard",
        &["Off", "Copy", "Copy and read"],
        match config.clipboard_access.trim() {
            "off" => 0,
            "read_write" => 2,
            _ => 1,
        },
        "clipboard_access",
    );

    if cfg!(target_os = "macos") {
        form.group("Keys");
        let y = form.y;
        form.toggle(
            y,
            "Left Option key is Meta",
            config.option_as_meta,
            "option_as_meta",
        );
        let y = form.y;
        form.toggle(
            y,
            "Right Option key is Meta",
            config.right_option_meta(),
            "right_option_as_meta",
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
        "Blocks start",
        &["Bottom", "Top"],
        match config.blocks_start {
            rift_core::BlocksStart::Bottom => 0,
            rift_core::BlocksStart::Top => 1,
        },
        "blocks_start",
    );
    let y = form.y;
    form.segment(
        y,
        "Prompt",
        &["Rift", "Shell (PS1)"],
        match config.prompt {
            PromptMode::Rift => 0,
            PromptMode::Shell => 1,
        },
        "prompt",
    );
    let y = form.y;
    form.prompt_preview(y, &config.prompt_chips);
    let y = form.y;
    form.segment(
        y,
        "Time on this tab counts",
        &["Since first command", "Only while running"],
        match config.elapsed_counts {
            rift_core::ElapsedCounts::Always => 0,
            rift_core::ElapsedCounts::Running => 1,
        },
        "elapsed_counts",
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
}

fn files_page(form: &mut Form, config: &Config) {
    use rift_core::editors;
    form.group("Editor");
    let y = form.y;
    form.select(
        y,
        "Open file links in",
        &editors::label(&config.link_editor),
        "link_editor",
    );
    let y = form.y;
    form.select(
        y,
        "Open files from the explorer and code review in",
        &editors::label(&config.file_editor),
        "file_editor",
    );

    form.group("File explorer");
    let y = form.y;
    form.toggle_desc(
        y,
        "Show hidden files",
        "Dotfiles and folders starting with . (.git and ignored files stay hidden)",
        config.explorer_hidden_files,
        "explorer_hidden_files",
    );
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

/// Settings › Notifications › Desktop notifications: whether banners can
/// show (macOS permission, with Allow… or System Settings), and what they say.
fn desktop_notifications(form: &mut Form) {
    use crate::notify::Permission;
    form.group("Desktop notifications");
    let p = crate::notify::permission();
    let (label, desc) = match p {
        Permission::NotifySend => (
            "Desktop notifications",
            "Sent with notify-send; your desktop's settings decide where they show",
        ),
        Permission::Script => (
            "macOS permission",
            "Not running as Rift.app: banners come from Script Editor",
        ),
        _ => (
            "macOS permission",
            "Banners come from Rift, with its icon; a click opens the pane",
        ),
    };
    let (status, tone) = match p {
        Permission::Allowed => ("Allowed", Some(true)),
        Permission::Denied => ("Off", Some(false)),
        Permission::NotAsked => ("Not asked yet", None),
        Permission::Checking => ("Checking…", None),
        Permission::Script | Permission::NotifySend => ("", None),
    };
    let button = match p {
        Permission::NotAsked => Some("Allow…"),
        Permission::Allowed | Permission::Denied => Some("Open System Settings"),
        _ => None,
    };
    let bw = button.map_or(0.0, |b| {
        (form.engine.measure(b, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0).round()
    });
    let pill_w = if status.is_empty() {
        0.0
    } else {
        (form
            .engine
            .measure(status, 12.0, FONT_SANS, Weight::SEMIBOLD)
            + 18.0)
            .round()
    };
    const ICON: f32 = 32.0;
    const INDENT: f32 = ICON + 12.0;
    let y = form.y;
    form.reserve = bw + pill_w + 10.0 + 40.0;
    // the label sits right of Rift's icon
    form.x += INDENT;
    form.card_w -= INDENT;
    form.label_desc(label, desc, y);
    form.x -= INDENT;
    form.card_w += INDENT;
    if form.hidden {
        return;
    }
    if form.visible(y, ROW_H) {
        super::welcome::logo(form.layer, [form.x, y + (ROW_H - ICON) / 2.0], ICON);
        form.layer
            .hline(form.x, y + ROW_H - 1.0, INDENT, 1.0, lift(form.theme, 0.06));
    }
    let mut x = form.right() - bw;
    if let Some(b) = button {
        let by = y + (ROW_H - CONTROL_H) / 2.0;
        let name = if b == "Allow…" {
            "Allow Rift's desktop notifications".to_string()
        } else {
            "Open System Settings, Notifications".to_string()
        };
        form.button(
            x,
            by,
            b,
            ButtonKind::Secondary,
            ClickTarget::SettingsNotifyPermission,
            name,
        );
        x -= 10.0;
    }
    if pill_w > 0.0 && form.visible(y, ROW_H) {
        let px = x - pill_w;
        let py = y + (ROW_H - 22.0) / 2.0;
        let (fill, ink) = match tone {
            Some(true) => (form.theme.success.with_alpha(0.16), form.theme.success),
            Some(false) => (form.theme.error.with_alpha(0.16), form.theme.error),
            None => (lift(form.theme, 0.08), form.theme.text_dim),
        };
        form.layer.rect([px, py], [pill_w, 22.0], fill, 11.0);
        form.layer.text(
            sans(form.engine, status, 12.0, Weight::SEMIBOLD),
            px + 9.0,
            py + (22.0 - 12.0 * 1.35) / 2.0,
            ink,
        );
    }
    let y = form.y;
    form.label_desc(
        "Banners show",
        "Who and what · the tab, folder and branch · the command or message",
        y,
    );
}

fn notifications_page(form: &mut Form, config: &Config) {
    let n = &config.notifications;
    form.group("Alerts");
    let y = form.y;
    form.toggle_desc(
        y,
        "Notifications",
        "Toasts while Rift is focused, desktop notifications while it is not",
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

    desktop_notifications(form);

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
    form.sound_select(y, "Sound", &sound_label(&n.sound), "notifications.sound");
    let y = form.y;
    form.sound_select(
        y,
        "Finished",
        &sound_label(&n.sound_finished),
        "notifications.sound_finished",
    );
    let y = form.y;
    form.sound_select(
        y,
        "Failed",
        &sound_label(&n.sound_failed),
        "notifications.sound_failed",
    );
    let y = form.y;
    form.sound_select(
        y,
        "Needs attention",
        &sound_label(&n.sound_attention),
        "notifications.sound_attention",
    );

    // sample alerts: the card or banner, with its sound, as the real ones
    form.group("Try it");
    use rift_core::notifications::Event;
    let kinds = [
        (Event::Finished, "Finished"),
        (Event::Failed, "Failed"),
        (Event::Attention, "Needs attention"),
    ];
    for (desktop, label, desc) in [
        (
            false,
            "In-app toast",
            "The card for a pane you are not looking at, with its sound",
        ),
        (
            true,
            "Desktop notification",
            "A real banner from Rift, even while Rift is in front, with its sound",
        ),
    ] {
        let y = form.y;
        let widths: Vec<f32> = kinds
            .iter()
            .map(|(_, k)| (form.engine.measure(k, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0).round())
            .collect();
        let total = widths.iter().sum::<f32>() + 8.0 * (kinds.len() - 1) as f32;
        form.reserve = total + 40.0;
        form.label_desc(label, desc, y);
        if !form.hidden {
            let by = y + (ROW_H - CONTROL_H) / 2.0;
            let mut bx = form.right() - total;
            for ((event, k), w) in kinds.iter().zip(&widths) {
                form.button(
                    bx,
                    by,
                    k,
                    ButtonKind::Secondary,
                    ClickTarget::SettingsTestAlert(crate::state::TestAlert {
                        event: *event,
                        desktop,
                    }),
                    format!("Try a {} {}", k.to_lowercase(), label.to_lowercase()),
                );
                bx += w + 8.0;
            }
        }
    }

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
    let y = form.y;
    let label = "Ring";
    let bw = form.engine.measure(label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0;
    form.reserve = bw + 40.0;
    form.label_desc(
        "Try the bell",
        "What a program's bell (\\a) does with this setting",
        y,
    );
    if !form.hidden {
        let by = y + (ROW_H - CONTROL_H) / 2.0;
        let bx = form.right() - bw.round();
        form.button(
            bx,
            by,
            label,
            ButtonKind::Secondary,
            ClickTarget::SettingsRingBell,
            "Ring the terminal bell".into(),
        );
    }
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

fn keybindings_page(
    form: &mut Form,
    keymap: &Keymap,
    config: &Config,
    state: &SettingsState,
    blink_on: bool,
) {
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
        // typing lands here (unless a row is recording): show the caret,
        // before the placeholder while empty
        if blink_on && state.recording.is_none() {
            let tw = if state.kb_filter.is_empty() {
                0.0
            } else {
                form.engine
                    .measure(&state.kb_filter, 13.0, FONT_SANS, Weight::NORMAL)
            };
            form.layer.rect(
                [form.x + 32.0 + tw, fy + 8.0],
                [1.5, 16.0],
                form.theme.accent,
                0.0,
            );
        }
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
    let names = rift_core::theme::available_themes();
    let labels = rift_core::theme::picker_labels(&names);
    for (i, name) in names.iter().enumerate() {
        let ry = form.y;
        form.y += ROW_H;
        if !form.visible_band(ry, ROW_H) {
            // not drawn, but Tab still reaches it (push_hit records the stop)
            form.push_hit(
                [form.x, ry, form.card_w, ROW_H],
                ClickTarget::SettingsTheme(i as u8),
            );
            continue;
        }
        let t = rift_core::theme::load_by_name(name);
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
        let shown = super::palette::elide_to(form.engine, &labels[i], 13.0, FONT_SANS, name_w);
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
            Some(tag) => format!("{}, {tag}", labels[i]),
            None => labels[i].clone(),
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
        GatewayLine, Page, SettingsState, config_table, countdown, focus_order, is_changed,
        next_focus, platform_name, reset_setting, thousands,
    };
    use crate::state::{ClickTarget, Hit};

    #[test]
    fn the_tunnel_row_says_what_it_waits_for() {
        use super::{RemoteView, tunnel_text};
        use crate::tunnel::TunnelState;
        // without cloudflared nothing else matters
        let mut v = RemoteView {
            tunnel: Some(TunnelState::Starting),
            ..Default::default()
        };
        assert_eq!(tunnel_text(&v), "Waiting for cloudflared");
        v.cloudflared = Some("/opt/homebrew/bin/cloudflared".into());
        assert_eq!(tunnel_text(&v), "Starting…");
        v.tunnel = None;
        assert_eq!(tunnel_text(&v), "Starting…");
        v.tunnel = Some(TunnelState::Ready {
            url: "wss://a-b-c.trycloudflare.com".into(),
        });
        assert_eq!(
            tunnel_text(&v),
            "Ready · a-b-c.trycloudflare.com · pair phones again after a restart"
        );
        v.tunnel = Some(TunnelState::Failed {
            detail: "boom".into(),
        });
        assert_eq!(tunnel_text(&v), "Failed, retrying: boom");
    }

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
        let mut c = rift_core::Config::default();
        let d = config_table(&c);
        c.font_size = 18.0;
        c.notifications.toasts = !c.notifications.toasts;
        let t = config_table(&c);
        assert!(is_changed(&t, &d, "font_size"));
        assert!(is_changed(&t, &d, "notifications.toasts"));
        assert!(!is_changed(&t, &d, "theme"));
        let c = reset_setting(&c, "font_size").expect("loads");
        assert_eq!(c.font_size, rift_core::Config::default().font_size);
        let c = reset_setting(&c, "notifications.toasts").expect("loads");
        assert!(!is_changed(&config_table(&c), &d, "notifications.toasts"));
    }

    #[test]
    fn platforms_are_named_as_people_write_them() {
        assert_eq!(platform_name("ios"), "iOS");
        assert_eq!(platform_name("android"), "Android");
        assert_eq!(platform_name("tizen"), "tizen");
    }

    #[test]
    fn remote_lines_and_countdown() {
        use std::time::Duration;
        assert_eq!(GatewayLine::Off.text(), "Off");
        assert_eq!(GatewayLine::Starting.text(), "Starting…");
        assert_eq!(
            GatewayLine::Listening(vec!["192.168.1.29:7878".into()]).text(),
            "Listening on 192.168.1.29:7878"
        );
        assert_eq!(
            GatewayLine::Listening(vec![]).text(),
            "Listening, but on no network"
        );
        assert_eq!(GatewayLine::Connected(1).text(), "1 device connected");
        assert_eq!(GatewayLine::Connected(3).text(), "3 devices connected");
        assert_eq!(countdown(Duration::from_secs(120)), "2:00");
        assert_eq!(countdown(Duration::from_millis(119_400)), "2:00");
        assert_eq!(countdown(Duration::from_secs(61)), "1:01");
        assert_eq!(countdown(Duration::from_millis(300)), "0:01");
        assert_eq!(countdown(Duration::ZERO), "0:00");
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
        assert_eq!(state.page, Page::About);
        state.move_page(-1);
        assert_eq!(state.page, Page::Remote);
        state.move_page(-99);
        assert_eq!(state.page, Page::General);
    }
}
