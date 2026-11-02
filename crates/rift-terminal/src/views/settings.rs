use rift_core::{
    Bell, BlockSpacing, Color, Config, CursorStyle, InputPosition, Keymap, NewTabPlacement,
    PromptMode, TabBarVisibility, TabCloseButton, TabCloseSide, TabLayout, TabStyle, Theme,
    ToolbarItem,
};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::state::{ClickTarget, Hit};
use crate::views::icons::{self, Icon};
pub mod collections;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    /// Where Settings opens (and opens again once closed).
    #[default]
    General,
    Profiles,
    Actions,
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
    pub const ALL: [Page; 12] = [
        Page::General,
        Page::Profiles,
        Page::Actions,
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
            Page::Profiles => "Profiles",
            Page::Actions => "Actions",
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
            Page::Actions => "Named command sequences and shortcuts. Saving never runs an action.",
            Page::Profiles => {
                "The shells and programs tabs open with. Drag a row (or ⌥↑ ⌥↓) to reorder."
            }
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
            // what a tab is launched with
            Page::Profiles => Icon::Launch,
            // what an action does: runs its steps
            Page::Actions => Icon::Play,
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
    /// No relay set.
    #[default]
    Off,
    /// A relay is set, but remote access is off: nothing dials it.
    Stopped,
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
            RelayLine::Off => "No relay set: phones connect only on this Wi-Fi".into(),
            RelayLine::Stopped => "Off while remote access is off".into(),
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
            RelayLine::Off | RelayLine::Stopped | RelayLine::Scripted => None,
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
    /// `iOS · paired October 12, 2026` (how it is connected left out: the
    /// page says that itself while remote access is off).
    pub about: String,
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
    /// The page has been read since it opened (`App::redraw` reads it on
    /// its first frame, so the phones never pop in a second later).
    pub page_read: bool,
    /// The next pairing link grants `view` only.
    pub view_only: bool,
    /// "Pair a phone" was pressed and waits for the gateway's addresses.
    pub pairing_wait: bool,
    pub pairing: Option<PairingView>,
    pub relay: RelayLine,
    /// The relay field is being edited: its text (⏎ saves, Esc cancels).
    pub relay_edit: Option<String>,
    /// "Connection settings" is open (the listen address, the relay
    /// address, the Cloudflare tunnel).
    pub relay_open: bool,
    /// "Recent connections" is open.
    pub history_open: bool,
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
    pub collection: Option<collections::Draft>,
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
    /// The text field being edited (`ClickTarget::SettingsText`): its id
    /// and draft. ⏎ or a click away saves it, Esc cancels.
    pub text_edit: Option<(&'static str, String)>,
    /// Settings › Profiles: the profile whose edit form is open.
    pub profile_open: Option<usize>,
    /// Settings › Profiles: why the last change was not saved (shown in the
    /// open form until the next edit).
    pub profile_error: Option<String>,
    /// Settings › Profiles: a row pressed for a drag.
    pub profile_drag: Option<ProfileDrag>,
    /// Settings › Themes: which themes the grid shows.
    pub theme_filter: ThemeFilter,
}

/// Settings › Themes' All / Dark / Light filter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemeFilter {
    #[default]
    All,
    Dark,
    Light,
}

impl ThemeFilter {
    pub const ALL: [ThemeFilter; 3] = [ThemeFilter::All, ThemeFilter::Dark, ThemeFilter::Light];

    pub fn label(self) -> &'static str {
        match self {
            ThemeFilter::All => "All",
            ThemeFilter::Dark => "Dark",
            ThemeFilter::Light => "Light",
        }
    }

    /// A theme with a light (`Some(true)`) or dark ground shows under this
    /// filter; one whose file does not load (`None`) only under All.
    pub fn shows(self, light: Option<bool>) -> bool {
        match self {
            ThemeFilter::All => true,
            ThemeFilter::Dark => light == Some(false),
            ThemeFilter::Light => light == Some(true),
        }
    }

    /// How many of `lights` (one per theme, as [`ThemeFilter::shows`] takes
    /// it) show.
    pub fn count(self, lights: &[Option<bool>]) -> usize {
        lights.iter().filter(|l| self.shows(**l)).count()
    }
}

/// Columns of the theme grid in a body `width` wide: cards at least
/// [`THEME_CARD_MIN_W`] wide with [`THEME_GAP`] between them, at most three.
pub fn theme_columns(width: f32) -> usize {
    (((width + THEME_GAP) / (THEME_CARD_MIN_W + THEME_GAP)).floor() as usize).clamp(1, 3)
}

/// An arrow key over a grid of cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridStep {
    Left,
    Right,
    Up,
    Down,
}

/// The card an arrow key moves to from `from` among `cards` (in reading
/// order, with their rects): ← → the previous / next one; ↑ ↓ the nearest
/// in the row above / below (the closest column). `None`: nowhere to go.
pub fn grid_step(
    cards: &[(ClickTarget, [f32; 4])],
    from: Option<&ClickTarget>,
    step: GridStep,
) -> Option<ClickTarget> {
    let i = cards.iter().position(|(t, _)| Some(t) == from)?;
    let r = cards[i].1;
    match step {
        GridStep::Left => i.checked_sub(1).map(|j| cards[j].0.clone()),
        GridStep::Right => cards.get(i + 1).map(|c| c.0.clone()),
        GridStep::Up | GridStep::Down => {
            let row_y = |c: &[f32; 4]| c[1];
            let ahead = |c: &[f32; 4]| {
                if step == GridStep::Up {
                    row_y(c) < r[1] - 1.0
                } else {
                    row_y(c) > r[1] + 1.0
                }
            };
            // the nearest row in that direction, then the closest column
            let row = cards
                .iter()
                .filter(|(_, c)| ahead(c))
                .map(|(_, c)| (row_y(c) - r[1]).abs())
                .fold(f32::INFINITY, f32::min);
            cards
                .iter()
                .filter(|(_, c)| ahead(c) && ((row_y(c) - r[1]).abs() - row).abs() < 1.0)
                .min_by(|a, b| {
                    let d = |c: &[f32; 4]| (c[0] - r[0]).abs();
                    d(&a.1).total_cmp(&d(&b.1))
                })
                .map(|c| c.0.clone())
        }
    }
}

/// Narrowest theme card, and the gap between cards.
pub const THEME_CARD_MIN_W: f32 = 190.0;
pub const THEME_GAP: f32 = 14.0;

/// What Settings › Remote access › Pair a phone shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairStage {
    /// Remote access is off: no pairing at all.
    Hidden,
    /// The Pair a phone button.
    Idle,
    /// Pair a phone was pressed; the gateway (or the tunnel) has no address
    /// to put in the link yet.
    Waiting,
    /// The code, its countdown, Copy link and Cancel.
    Showing,
    /// The code ran out: "Expired" and Show a new code.
    Expired,
}

/// The stage of the pairing section at `now`.
pub fn pair_stage(
    on: bool,
    wait: bool,
    pairing: Option<&PairingView>,
    now: std::time::SystemTime,
) -> PairStage {
    match pairing {
        _ if !on => PairStage::Hidden,
        Some(p) if p.expires_at > now => PairStage::Showing,
        Some(_) => PairStage::Expired,
        None if wait => PairStage::Waiting,
        None => PairStage::Idle,
    }
}

/// The status card's title and line on Settings › Remote access.
pub fn remote_heading(on: bool, gateway: &GatewayLine) -> (String, &'static str) {
    if !on {
        return (
            "Remote access is off".into(),
            "Turn it on to watch and run commands from your phone. Until then nothing listens and no phone can reach this computer.",
        );
    }
    let line =
        "Paired phones can see your tabs and run commands in them, on this Wi-Fi or from away.";
    match gateway {
        GatewayLine::Off | GatewayLine::Starting => ("Starting remote access…".into(), line),
        GatewayLine::Listening(a) if a.is_empty() => (
            "Not on a network".into(),
            "Remote access is on, but this computer has no network address phones could reach.",
        ),
        GatewayLine::Listening(_) => ("Ready for your phone".into(), line),
        GatewayLine::Connected(1) => ("1 phone connected".into(), line),
        GatewayLine::Connected(n) => (format!("{n} phones connected"), line),
    }
}

/// A Settings › Profiles row held by the pointer: past a few pixels it moves
/// with it; released, it lands at `to`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileDrag {
    pub from: usize,
    pub press_y: f32,
    pub live: bool,
    /// Where it would land (an index into the list without it moved yet).
    pub to: usize,
}

/// Pointer travel (logical px) before a pressed profile row drags.
pub const PROFILE_DRAG_SLOP: f32 = 4.0;

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
        self.text_edit = None;
        self.profile_open = None;
        self.collection = None;
        self.profile_error = None;
        self.profile_drag = None;
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
            | ClickTarget::SettingsEditToolbar
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
            | ClickTarget::SettingsThemeFilter(_)
            | ClickTarget::SettingsOpenThemes
            | ClickTarget::RemotePair
            | ClickTarget::RemoteViewOnly
            | ClickTarget::RemotePairCancel
            | ClickTarget::RemoteHistory
            | ClickTarget::RemoteCopyLink
            | ClickTarget::RemoteRevoke(_)
            | ClickTarget::RemoteRelay
            | ClickTarget::RemoteRelayRetry
            | ClickTarget::RemoteRelaySettings
            | ClickTarget::RemotePermission(..)
            | ClickTarget::SettingsText(_)
            | ClickTarget::ProfileRow(_)
            | ClickTarget::Profile(..)
            | ClickTarget::ProfileAdd
            | ClickTarget::Collection(..)
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

/// Every config.toml key (dotted inside a table) a Settings row controls,
/// page by page. `every_config_key_has_a_settings_row` holds config.toml to
/// it: a new key needs a row here, or a reason in that test's allow-list.
/// User-facing configuration belongs in dedicated controls. Internal provenance
/// and generated runtime state do not; see docs/configuration.md, Settings UI
/// exposure policy. A config-editor fallback is a UI gap, not a hidden-setting policy.
pub const ROWS: &[&str] = &[
    // General
    "launch_at_login",
    "shell",
    "scrollback",
    "restore_session",
    "working_directory",
    "confirm_close_running",
    "save_transcripts",
    "durable_sessions",
    "global_hotkey",
    "hotkey_window",
    "hotkey_window_height",
    "hotkey_window_edge",
    "hotkey_window_width",
    "hotkey_window_screen",
    "hotkey_window_hide_on_blur",
    "check_for_updates",
    "control_api",
    // Profiles (the list itself is the page)
    "default_profile",
    "discover_shells",
    // Appearance
    "font_family",
    "font_size",
    "line_height",
    "font_weight",
    "bold_font_weight",
    "bold_is_bright",
    "minimum_contrast",
    "font_features",
    "ambiguous_width",
    "cursor_style",
    "cursor_hollow_unfocused",
    "cursor_blink",
    "theme",
    "sync_os_theme",
    "theme_light",
    "window_opacity",
    "window_blur",
    "window_padding",
    "window_padding_left",
    "window_padding_right",
    "window_padding_top",
    "window_padding_bottom",
    "alt_screen_padding",
    "app_icon",
    "ligatures",
    "launch_mode",
    "window_width",
    "window_height",
    "always_on_top",
    "snap_to_cells",
    "resize_overlay",
    "tab_layout",
    "tab_style",
    "tab_close_button",
    "tab_close_side",
    "tab_bar",
    "new_tab_placement",
    "tab_switch_order",
    "tab_bar_with_one_tab",
    // the Toolbar buttons preview opens Edit toolbar
    "toolbar_left",
    "toolbar_right",
    "dim_inactive_panes",
    "inactive_pane_dim",
    "focus_follows_mouse",
    // Input
    "vim_mode",
    "vim_clipboard",
    "shell_widgets",
    "autosuggestions",
    "tab_behavior",
    "autosuggestion_hint",
    "autosuggestion_ignore_button",
    "completions_open_while_typing",
    "shell_completions",
    "autocomplete_pairs",
    "input_position",
    "blocks_start",
    "sticky_command_header",
    "block_spacing",
    "block_dividers",
    "jump_to_bottom_button",
    "prompt",
    // the Prompt chips preview opens Edit prompt
    "prompt_chips",
    "elapsed_counts",
    "alias_expansion",
    "error_underline",
    "command_corrections",
    "subshell_offer",
    "subshell_commands",
    "subshell_denylist",
    "rich_input_submit",
    "rich_input_auto",
    // Terminal
    "mouse_reporting",
    "middle_click_paste",
    "link_click",
    "wheel_scroll_lines",
    "wheel_zoom",
    "word_chars",
    "word_delimiters",
    "copy_on_select",
    "confirm_multiline_paste",
    "clipboard_access",
    "large_paste_warning",
    "trim_paste",
    "scrollbar",
    "close_on_exit",
    "mask_secrets",
    "show_progress",
    "option_as_meta",
    "right_option_as_meta",
    // Files
    "link_editor",
    "file_editor",
    "explorer_hidden_files",
    // Notifications
    "notifications.enabled",
    "notifications.toasts",
    "notifications.toast_secs",
    "request_attention",
    "notify_on_completion",
    "notify_threshold_secs",
    "notifications.command_failed",
    "notifications.agent_task_completed",
    "notifications.needs_attention",
    "notifications.password_prompt",
    "notifications.play_sound",
    "notifications.sound",
    "notifications.sound_finished",
    "notifications.sound_failed",
    "notifications.sound_attention",
    "bell",
    // Remote access
    "remote_access",
    "remote_listen",
    "remote_relay",
    "remote_tunnel",
    "remote_clipboard",
];

/// Settings the control of `id` also stands for (the "Window padding"
/// stepper shows the left and right padding in effect).
fn covered(id: &str) -> &'static [&'static str] {
    match id {
        "window_padding" => &["window_padding_left", "window_padding_right"],
        // the Toolbar buttons preview stands for both sides
        "toolbar_left" => &["toolbar_right"],
        _ => &[],
    }
}

/// The setting `id` (or one its control stands for) differs from its
/// default.
pub fn is_changed(current: &toml::Table, defaults: &toml::Table, id: &str) -> bool {
    lookup(current, id) != lookup(defaults, id)
        || covered(id)
            .iter()
            .any(|k| lookup(current, k) != lookup(defaults, k))
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
    // (top-level settings only)
    for k in covered(id) {
        table.remove(*k);
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
        // takes it): the caret shows there, before the placeholder if empty;
        // a field being typed in has the only caret
        let typing_elsewhere = state.text_edit.is_some() || state.remote.relay_edit.is_some();
        if blink_on && !typing_elsewhere && (searching || state.page != Page::Keybindings) {
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

    // a short window closes the 2 px gaps between the page rows, then
    // shortens the rows (down to 22 px), before the foot (Open config.toml)
    // runs out of room
    let foot_room = 8.0 + 40.0;
    let fit = (bottom - foot_room - ny) / Page::ALL.len() as f32;
    let pitch = if fit >= 34.0 {
        34.0
    } else {
        fit.clamp(22.0, 32.0)
    };
    let row_h = pitch.min(32.0);
    let mid = row_h / 2.0;
    for (i, page) in Page::ALL.iter().enumerate() {
        let selected = state.page == *page && state.query.is_empty();
        let row = [x0 + 8.0, ny, sidebar_w - 16.0, row_h];
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
        icons::draw(layer, page.icon(), [icon_x, ny + mid], 16.0, color);
        if !compact {
            layer.text(
                sans(engine, page.label(), 13.0, Weight::NORMAL),
                row[0] + 36.0,
                ny + mid - 8.5,
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
        ny += pitch;
    }

    // sidebar foot: open config.toml in the reading pane
    let foot = [x0 + 8.0, bottom - 40.0, sidebar_w - 16.0, 32.0];
    if foot[1] + 0.5 >= ny + 8.0 {
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
        edit: state.text_edit.as_ref().map(|(id, t)| (*id, t.as_str())),
        blink_on,
    };
    if searching {
        for page in [
            Page::General,
            Page::Profiles,
            Page::Actions,
            Page::Appearance,
            Page::Features,
            Page::Terminal,
            Page::Files,
            Page::Notifications,
            Page::Remote,
            Page::Themes,
        ] {
            form.page = page.label();
            match page {
                // by name: a match applies that theme
                Page::Themes => themes_search(&mut form, config),
                Page::General => general_page(&mut form, config, &state.update),
                // its settings only: the list and its forms need the page
                Page::Profiles => profile_settings(&mut form, config),
                Page::Actions => {}
                Page::Appearance => appearance_page(&mut form, config),
                Page::Features => features_page(&mut form, config),
                Page::Terminal => terminal_page(&mut form, config),
                Page::Files => files_page(&mut form, config),
                // its switch only: pairing and devices need the page
                Page::Remote => remote_switch(&mut form, config),
                _ => notifications_page(&mut form, config),
            }
            collections::page(&mut form, config, state, page);
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
        collections::page(&mut form, config, state, state.page);
        match state.page {
            Page::Appearance => appearance_page(&mut form, config),
            Page::General => general_page(&mut form, config, &state.update),
            Page::Profiles => profiles_page(&mut form, config, state),
            Page::Actions => {}
            Page::Features => features_page(&mut form, config),
            Page::Terminal => terminal_page(&mut form, config),
            Page::Files => files_page(&mut form, config),
            Page::Notifications => notifications_page(&mut form, config),
            Page::Keybindings => keybindings_page(&mut form, keymap, config, state, blink_on),
            Page::Themes => themes_page(&mut form, config, state.theme_filter),
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
/// A small picture of app icon `icon` (its tile, chevron and cursor) in a
/// `size` square at `pos`.
fn icon_swatch(layer: &mut Layer, icon: rift_core::AppIcon, pos: [f32; 2], size: f32) {
    let [tile, accent, cursor] = icon.colors().map(|c| Color::hex(c).unwrap_or(Color::BLACK));
    let [x, y] = pos;
    let u = size / 16.0;
    layer.bordered(
        [x, y],
        [size, size],
        tile,
        Color::WHITE.with_alpha(0.12),
        3.5 * u,
        1.0,
    );
    layer.line(
        [x + 4.0 * u, y + 5.0 * u],
        [x + 7.0 * u, y + 8.0 * u],
        1.6 * u,
        accent,
    );
    layer.line(
        [x + 7.0 * u, y + 8.0 * u],
        [x + 4.0 * u, y + 11.0 * u],
        1.6 * u,
        accent,
    );
    layer.rect(
        [x + 9.0 * u, y + 5.0 * u],
        [2.6 * u, 6.0 * u],
        cursor,
        0.5 * u,
    );
}

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
        if dd.id == "app_icon"
            && let Some(icon) = rift_core::AppIcon::from_name(value)
        {
            icon_swatch(layer, icon, [tx, cy - 8.0], 16.0);
            tx += 24.0;
        }
        // themes: their ground ringed in their accent (a profile's "Window
        // theme" has none of its own)
        if dd.id.starts_with("theme") || (dd.id == "profile.theme" && !value.is_empty()) {
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
    /// The text field being edited and its draft.
    edit: Option<(&'static str, &'a str)>,
    blink_on: bool,
}

/// A text setting's row (`Form::text_row`).
struct TextRow<'a> {
    label: &'a str,
    desc: &'a str,
    value: &'a str,
    placeholder: &'a str,
    id: &'static str,
    /// Shown, not editable (a detected profile's command).
    read_only: bool,
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
        // a group named like its page (Input › Input) says it once
        self.crumb = if label.eq_ignore_ascii_case(self.page) {
            self.page.to_string()
        } else {
            format!("{} › {label}", self.page)
        };
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
        // a row's key is listed (a profile's fields stand for the list)
        debug_assert!(
            ROWS.contains(&id) || id.starts_with("profile.") || id.starts_with("collection."),
            "Settings row {id} is missing from settings::ROWS"
        );
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
                ButtonKind::Outline => (
                    if hot {
                        theme.error.with_alpha(0.10)
                    } else {
                        Color::TRANSPARENT
                    },
                    theme.error.with_alpha(0.55),
                    theme.error,
                ),
                ButtonKind::Disabled => (lift(theme, 0.03), lift(theme, 0.08), theme.text_muted),
                ButtonKind::Selected => (lift(theme, 0.11), theme.accent, theme.text_bright),
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
        self.changed("prompt_chips", y, self.right());
        if self.hidden {
            return;
        }
        let r = [self.x, self.y + 4.0, self.card_w, 40.0];
        self.y = r[1] + r[3] + 8.0;
        if self.visible(r[1], r[3]) {
            let edit_w = self.preview_box(r);
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
                [r[0] + 12.0, r[0] + r[2] - 12.0 - edit_w],
                r[1] + 13.0,
                &views,
            );
        }
        self.push_hit(r, ClickTarget::SettingsEditPrompt);
        self.name_last("Edit prompt chips".into());
    }

    /// The frame of a preview that opens its editor (prompt chips,
    /// toolbar): a hairline box that lifts with an accent border on hover,
    /// and "✎ Edit" at its right end, brighter on hover. Returns the width
    /// the hint takes (the preview keeps clear of it).
    fn preview_box(&mut self, r: [f32; 4]) -> f32 {
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
        let ink = if hot {
            self.theme.text_ui
        } else {
            self.theme.text_muted
        };
        let tw = self.engine.measure("Edit", 12.0, FONT_SANS, Weight::NORMAL);
        let w = 13.0 + 5.0 + tw;
        let x = r[0] + r[2] - 12.0 - w;
        let mid = r[1] + r[3] / 2.0;
        icons::draw(self.layer, Icon::Pencil, [x + 6.5, mid], 13.0, ink);
        self.layer.text(
            sans(self.engine, "Edit", 12.0, Weight::NORMAL),
            x + 18.0,
            mid - 8.0,
            ink,
        );
        w + 12.0
    }

    /// The toolbar as the strip shows it (traffic lights, the left
    /// buttons, two sample tabs, the right buttons) in a preview that opens
    /// "Edit toolbar".
    fn toolbar_preview(&mut self, y: f32, left: &[ToolbarItem], right: &[ToolbarItem]) {
        self.label_desc("Toolbar buttons", "Click to choose and arrange them", y);
        self.changed("toolbar_left", y, self.right());
        if self.hidden {
            return;
        }
        let r = [self.x, self.y + 4.0, self.card_w, 46.0];
        self.y = r[1] + r[3] + 8.0;
        if self.visible(r[1], r[3]) {
            let edit_w = self.preview_box(r);
            let theme = self.theme;
            let strip = [r[0] + 8.0, r[1] + 8.0, r[2] - 16.0 - edit_w, 30.0];
            self.layer.rect(
                [strip[0], strip[1]],
                [strip[2], strip[3]],
                theme.ground(theme.bg_chrome),
                4.0,
            );
            let mid = strip[1] + strip[3] / 2.0;
            let mut x = strip[0] + 10.0;
            for _ in 0..3 {
                self.layer.circle([x + 5.0, mid], 10.0, lift(theme, 0.2));
                x += 17.0;
            }
            x += 4.0;
            for item in left {
                super::toolbar_editor::draw_mark(
                    self.layer,
                    *item,
                    [x + 12.0, mid],
                    theme.text_dim,
                );
                x += 26.0;
            }
            // the right buttons from the strip's right end
            let mut rx = strip[0] + strip[2] - 6.0 - 26.0 * right.len() as f32;
            let tabs_end = rx - 6.0;
            for item in right {
                super::toolbar_editor::draw_mark(
                    self.layer,
                    *item,
                    [rx + 13.0, mid],
                    theme.text_dim,
                );
                rx += 26.0;
            }
            // two sample tabs between, the first active
            let tw = ((tabs_end - x - 6.0) / 2.0).clamp(0.0, 120.0);
            for (k, title) in ["project1", "~"].iter().enumerate() {
                let tx = x + 6.0 + k as f32 * tw;
                if tw < 40.0 {
                    break;
                }
                if k == 0 {
                    self.layer
                        .rect([tx, strip[1]], [tw, strip[3]], theme.ground(theme.bg), 0.0);
                    self.layer
                        .rect([tx, strip[1]], [tw, 1.5], theme.accent, 0.0);
                }
                self.layer.rect(
                    [tx + tw - 1.0, strip[1]],
                    [1.0, strip[3]],
                    theme.border,
                    0.0,
                );
                let w = self.engine.measure(title, 11.0, FONT_SANS, Weight::NORMAL);
                self.layer.text(
                    sans(self.engine, title, 11.0, Weight::NORMAL),
                    tx + (tw - w) / 2.0,
                    mid - 7.5,
                    if k == 0 {
                        theme.text_ui
                    } else {
                        theme.text_dim
                    },
                );
            }
        }
        self.push_hit(r, ClickTarget::SettingsEditToolbar);
        let names = |items: &[ToolbarItem]| {
            if items.is_empty() {
                "none".to_string()
            } else {
                items
                    .iter()
                    .map(|i| i.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };
        self.name_last(format!(
            "Edit toolbar buttons: left {}; right {}",
            names(left),
            names(right)
        ));
    }

    /// `tab_style`: one picture card per style (two small tabs drawn in it,
    /// a radio and its name, Dot tagged Default); a card picks its style
    /// (`SettingsSegment("tab_style", i)`).
    fn tab_style_cards(&mut self, y: f32, active: usize) {
        const NAMES: [&str; 3] = ["Dot", "Underline", "Detailed"];
        const ABOUT: [&str; 3] = [
            "A dot by the title",
            "The title underlined",
            "The state in words",
        ];
        const ILLUS_H: f32 = 58.0;
        const CARD_H: f32 = ILLUS_H + 46.0;
        const GAP: f32 = 12.0;
        self.label_desc("Tab style", "How a tab shows what its panes are doing", y);
        self.changed("tab_style", y, self.right());
        if self.hidden {
            return;
        }
        let top = self.y + 4.0;
        self.y = top + CARD_H + 10.0;
        let w = ((self.card_w - 2.0 * GAP) / 3.0).floor();
        for (i, name) in NAMES.iter().enumerate() {
            let r = [self.x + i as f32 * (w + GAP), top, w, CARD_H];
            let on = i == active;
            if self.visible(r[1], r[3]) {
                let hot = self.hot(r);
                let border = if on {
                    self.theme.accent
                } else if hot {
                    lift(self.theme, 0.28)
                } else {
                    lift(self.theme, 0.12)
                };
                self.layer.bordered(
                    [r[0], r[1]],
                    [r[2], r[3]],
                    lift(self.theme, 0.02),
                    border,
                    8.0,
                    if on { 1.5 } else { 1.0 },
                );
                let illus = [r[0] + 6.0, r[1] + 6.0, r[2] - 12.0, ILLUS_H - 6.0];
                self.mini_tabs(illus, i);
                // radio, name, Default
                let cy = r[1] + ILLUS_H + 16.0;
                let rx = r[0] + 12.0;
                self.layer.bordered(
                    [rx, cy - 7.0],
                    [14.0, 14.0],
                    Color::TRANSPARENT,
                    if on {
                        self.theme.accent
                    } else {
                        self.theme.text_muted
                    },
                    7.0,
                    1.5,
                );
                if on {
                    self.layer.circle([rx + 7.0, cy], 6.0, self.theme.accent);
                }
                self.layer.text(
                    sans(self.engine, name, 13.0, Weight::SEMIBOLD),
                    rx + 22.0,
                    cy - 8.5,
                    self.theme.text_bright,
                );
                if i == 0 {
                    let nw = self.engine.measure(name, 13.0, FONT_SANS, Weight::SEMIBOLD);
                    let tag = "Default";
                    let tw = self.engine.measure(tag, 10.5, FONT_SANS, Weight::MEDIUM) + 12.0;
                    let tx = rx + 22.0 + nw + 8.0;
                    self.layer
                        .rect([tx, cy - 8.0], [tw, 16.0], lift(self.theme, 0.08), 3.0);
                    self.layer.text(
                        sans(self.engine, tag, 10.5, Weight::MEDIUM),
                        tx + 6.0,
                        cy - 7.0,
                        self.theme.text_dim,
                    );
                }
                let about =
                    super::palette::elide_to(self.engine, ABOUT[i], 12.0, FONT_SANS, w - 24.0);
                self.layer.text(
                    sans(self.engine, &about, 12.0, Weight::NORMAL),
                    rx,
                    cy + 11.0,
                    self.theme.text_muted,
                );
            }
            self.push_hit(r, ClickTarget::SettingsSegment("tab_style", i as u8));
            self.name_last(format!(
                "Tab style: {name}, {}{}",
                ABOUT[i].to_lowercase(),
                if on { ", selected" } else { "" }
            ));
        }
    }

    /// The picture on tab style card `style` (0 dot, 1 underline, 2
    /// detailed): two small tabs in the strip's colours, one needing you
    /// (active) and one done.
    fn mini_tabs(&mut self, r: [f32; 4], style: usize) {
        let theme = self.theme;
        self.layer.rect(
            [r[0], r[1]],
            [r[2], r[3]],
            theme.ground(theme.bg_chrome),
            5.0,
        );
        let th = if style == 2 { 36.0 } else { 28.0 };
        let ty = r[1] + (r[3] - th) / 2.0;
        let tw = ((r[2] - 16.0) / 2.0).floor();
        let samples = [
            ("project2", theme.warning, "Needs you"),
            ("project1", theme.success, "Done"),
        ];
        for (k, (title, color, words)) in samples.iter().enumerate() {
            let x = r[0] + 8.0 + k as f32 * tw;
            let active = k == 0;
            if active {
                self.layer
                    .rect([x, ty], [tw, th], theme.ground(theme.bg), 0.0);
                self.layer.rect([x, ty], [tw, 1.5], theme.accent, 0.0);
            }
            self.layer
                .rect([x + tw - 1.0, ty], [1.0, th], theme.border, 0.0);
            let ink = if active {
                theme.text_ui
            } else {
                theme.text_dim
            };
            // a narrow card shrinks the sample text to fit its tab (the dot
            // style also fits the dot)
            let room = tw - if style == 0 { 24.0 } else { 10.0 };
            let full = self.engine.measure(title, 11.0, FONT_SANS, Weight::NORMAL);
            let size = (11.0 * (room / full).min(1.0)).max(8.0);
            let w = self.engine.measure(title, size, FONT_SANS, Weight::NORMAL);
            let mid = ty + th / 2.0;
            match style {
                0 => {
                    self.layer.circle([x + 9.0, mid], 6.0, *color);
                    let tx = x + 16.0 + (tw - 16.0 - w) / 2.0;
                    self.layer.text(
                        sans(self.engine, title, size, Weight::NORMAL),
                        tx,
                        mid - 7.5,
                        ink,
                    );
                }
                1 => {
                    let tx = x + (tw - w) / 2.0;
                    self.layer.text(
                        sans(self.engine, title, size, Weight::NORMAL),
                        tx,
                        mid - 8.5,
                        ink,
                    );
                    self.layer.rect([tx, mid + 6.0], [w, 2.0], *color, 1.0);
                }
                _ => {
                    let tx = x + (tw - w) / 2.0;
                    self.layer.text(
                        sans(self.engine, title, size, Weight::NORMAL),
                        tx,
                        mid - 14.0,
                        ink,
                    );
                    let wfull = self.engine.measure(words, 9.5, FONT_SANS, Weight::NORMAL);
                    let ws = (9.5 * ((tw - 20.0) / wfull).min(1.0)).max(7.5);
                    let ww = self.engine.measure(words, ws, FONT_SANS, Weight::NORMAL);
                    let lx = x + (tw - ww - 9.0) / 2.0;
                    self.layer.circle([lx + 2.5, mid + 6.5], 5.0, *color);
                    self.layer.text(
                        sans(self.engine, words, ws, Weight::NORMAL),
                        lx + 9.0,
                        mid + 0.5,
                        *color,
                    );
                }
            }
        }
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

    /// The app icon select: a picture of the chosen icon, its name.
    fn icon_select(&mut self, y: f32, icon: rift_core::AppIcon) {
        self.select_with(y, "App icon", icon.label(), None, false, "app_icon");
        if !self.hidden && self.visible(y, ROW_H) {
            let sel_w = (self.card_w * 0.45).clamp(140.0, 220.0);
            let x = self.right() - sel_w - 24.0;
            icon_swatch(self.layer, icon, [x, y + (ROW_H - 16.0) / 2.0], 16.0);
        }
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

    /// A check box row: the box, then the label and a muted line under it;
    /// the whole row presses `target`.
    fn checkbox(&mut self, y: f32, label: &str, desc: &str, on: bool, target: ClickTarget) {
        const BOX: f32 = 16.0;
        let f = &self.filter;
        self.row_label = label.to_string();
        self.hidden =
            !(f.is_empty() || label.to_lowercase().contains(f) || desc.to_lowercase().contains(f));
        if self.hidden {
            return;
        }
        self.rows += 1;
        self.y = y + ROW_H;
        let tx = self.x + BOX + 10.0;
        let lw = self.engine.measure(label, 13.0, FONT_SANS, Weight::NORMAL);
        let dw = self.engine.measure(desc, 12.0, FONT_SANS, Weight::NORMAL);
        let hit = [
            self.x - 4.0,
            y + 4.0,
            BOX + 10.0 + lw.max(dw) + 8.0,
            ROW_H - 8.0,
        ];
        if self.visible(y, ROW_H) {
            let theme = self.theme;
            let by = y + 9.0 + 8.8 - BOX / 2.0;
            if on {
                self.layer.rect([self.x, by], [BOX, BOX], theme.accent, 3.0);
                icons::draw(
                    self.layer,
                    Icon::Check,
                    [self.x + BOX / 2.0, by + BOX / 2.0],
                    12.0,
                    theme.bg,
                );
            } else {
                let hot = self.hot(hit);
                self.layer.bordered(
                    [self.x, by],
                    [BOX, BOX],
                    if hot {
                        lift(theme, 0.06)
                    } else {
                        Color::TRANSPARENT
                    },
                    theme.text_muted,
                    3.0,
                    1.5,
                );
            }
            self.layer.text(
                sans(self.engine, label, 13.0, Weight::NORMAL),
                tx,
                y + 9.0,
                theme.text_ui,
            );
            if !desc.is_empty() {
                self.layer.text(
                    sans(self.engine, desc, 12.0, Weight::NORMAL),
                    tx,
                    y + 28.0,
                    theme.text_muted,
                );
            }
            self.layer
                .hline(self.x, y + ROW_H - 1.0, self.card_w, 1.0, lift(theme, 0.06));
        }
        self.push_hit(hit, target);
    }

    /// A row of plain words: a label (may be empty) over a wrapped muted
    /// line; nothing to press, but named for assistive tech.
    fn note_row(&mut self, label: &str, text: &str) {
        let f = &self.filter;
        self.row_label = if label.is_empty() {
            text.to_string()
        } else {
            label.to_string()
        };
        self.hidden =
            !(f.is_empty() || label.to_lowercase().contains(f) || text.to_lowercase().contains(f));
        if self.hidden {
            return;
        }
        self.rows += 1;
        let y = self.y;
        let lines = wrap_lines(self.engine, text, 12.0, FONT_SANS, self.card_w);
        let label_h = if label.is_empty() { 0.0 } else { 19.0 };
        let h = (12.0 + label_h + lines.len() as f32 * NOTE_LH + 12.0).round();
        if self.visible(y, h) {
            let theme = self.theme;
            if !label.is_empty() {
                self.layer.text(
                    sans(self.engine, label, 13.0, Weight::NORMAL),
                    self.x,
                    y + 12.0,
                    theme.text_ui,
                );
            }
            for (i, l) in lines.iter().enumerate() {
                self.layer.text(
                    sans(self.engine, l, 12.0, Weight::NORMAL),
                    self.x,
                    y + 12.0 + label_h + i as f32 * NOTE_LH,
                    theme.text_muted,
                );
            }
            self.layer
                .hline(self.x, y + h - 1.0, self.card_w, 1.0, lift(theme, 0.06));
        }
        self.push_hit([self.x, y, self.card_w, h], ClickTarget::SettingsNoop);
        self.name_last(if label.is_empty() {
            text.to_string()
        } else {
            format!("{label}. {text}")
        });
        self.y = y + h;
    }

    /// A row that folds a section open and shut: a chevron (› shut, ⌄ open),
    /// the title and an optional count tag.
    fn disclosure(&mut self, title: &str, count: Option<String>, open: bool, target: ClickTarget) {
        self.hidden = false;
        self.rows += 1;
        self.row_label = title.to_string();
        let y = self.y;
        let r = [self.x - 6.0, y, self.card_w + 12.0, 40.0];
        if self.visible(y, 40.0) {
            let theme = self.theme;
            let hot = self.hot(r);
            if hot {
                self.layer
                    .rect([r[0], r[1]], [r[2], r[3]], lift(theme, 0.03), 4.0);
            }
            let ink = if hot { theme.text_ui } else { theme.text_dim };
            icons::draw(
                self.layer,
                if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                [self.x + 6.0, y + 20.0],
                12.0,
                ink,
            );
            self.layer.text(
                sans(self.engine, title, 13.0, Weight::MEDIUM),
                self.x + 20.0,
                y + 11.0,
                ink,
            );
            if let Some(count) = &count {
                let tw = self.engine.measure(title, 13.0, FONT_SANS, Weight::MEDIUM);
                let cw = self.engine.measure(count, 11.0, FONT_SANS, Weight::MEDIUM);
                let cx = self.x + 20.0 + tw + 8.0;
                self.layer
                    .rect([cx, y + 11.5], [cw + 12.0, 17.0], lift(theme, 0.08), 3.0);
                self.layer.text(
                    sans(self.engine, count, 11.0, Weight::MEDIUM),
                    cx + 6.0,
                    y + 13.0,
                    theme.text_dim,
                );
            }
            self.layer
                .hline(self.x, y + 40.0 - 1.0, self.card_w, 1.0, lift(theme, 0.06));
        }
        self.push_hit(r, target);
        self.name_last(format!(
            "{title}{}, {}",
            count.map(|c| format!(" ({c})")).unwrap_or_default(),
            if open { "expanded" } else { "collapsed" }
        ));
        self.y = y + 40.0;
    }

    /// A text setting: label (and description) left, a field right (the
    /// relay field's) that a click edits; ⏎ or a click away saves it.
    fn text_row(&mut self, y: f32, t: &TextRow) {
        let fw = (self.card_w * 0.45).clamp(160.0, 280.0);
        self.reserve = fw + 40.0;
        // a description too long beside the field wraps (never cut): the
        // row grows by its extra lines
        let beside = (self.card_w - self.reserve - 8.0).max(60.0);
        let lines = if self.filter.is_empty()
            && self.engine.measure(t.desc, 12.0, FONT_SANS, Weight::NORMAL) > beside
        {
            wrap_lines(self.engine, t.desc, 12.0, FONT_SANS, beside)
        } else {
            Vec::new()
        };
        let extra = lines.len().saturating_sub(1) as f32 * NOTE_LH;
        self.extra_h = extra;
        // (a blank description keeps the label on the two-line row's top line)
        self.label_desc(t.label, if lines.is_empty() { t.desc } else { " " }, y);
        if self.hidden {
            return;
        }
        if self.visible(y, ROW_H + extra) {
            for (k, l) in lines.iter().enumerate() {
                self.layer.text(
                    sans(self.engine, l, 12.0, Weight::NORMAL),
                    self.x,
                    y + 28.0 + k as f32 * NOTE_LH,
                    self.theme.text_muted,
                );
            }
        }
        let fx = self.right() - fw;
        let r = [fx, y + (ROW_H - CONTROL_H) / 2.0, fw, CONTROL_H];
        self.changed(t.id, y, fx);
        if t.read_only {
            if self.visible(r[1], r[3]) {
                // a disabled control (DESIGN.md › Buttons): 3 % fill, 8 % hairline
                self.layer.bordered(
                    [r[0], r[1]],
                    [r[2], r[3]],
                    lift(self.theme, 0.03),
                    lift(self.theme, 0.08),
                    4.0,
                    1.0,
                );
                let shown =
                    super::palette::elide_to(self.engine, t.value, 13.0, FONT_SANS, r[2] - 20.0);
                self.layer.text(
                    sans(self.engine, &shown, 13.0, Weight::NORMAL),
                    r[0] + 10.0,
                    r[1] + 5.2,
                    self.theme.text_muted,
                );
            }
            self.push_hit(r, ClickTarget::SettingsNoop);
            self.name_last(format!("{}: {}, read-only", t.label, t.value));
            return;
        }
        let editing = self.edit.filter(|(id, _)| *id == t.id).map(|(_, d)| d);
        let name = match editing {
            Some(d) => format!("{}, editing: {d}", t.label),
            None if t.value.is_empty() && t.placeholder.is_empty() => format!("{}: none", t.label),
            None if t.value.is_empty() => format!("{}: {}", t.label, t.placeholder),
            None => format!("{}: {}", t.label, t.value),
        };
        let blink = self.blink_on;
        self.text_field(
            r,
            t.value,
            t.placeholder,
            editing,
            blink,
            ClickTarget::SettingsText(t.id),
            name,
        );
    }

    /// Width of a [`Form::badge`] reading `text`.
    fn badge_width(&mut self, text: &str) -> f32 {
        (self.engine.measure(text, 11.5, FONT_SANS, Weight::MEDIUM) + 18.0).round()
    }

    /// A tag (the Themes page's "active" chip) at `x` on the row at `y`:
    /// `ink` text on `ink` at 16 % (`None`: muted on a lift).
    fn badge(&mut self, x: f32, y: f32, text: &str, ink: Option<Color>) {
        if !self.visible(y, ROW_H) {
            return;
        }
        let w = self.badge_width(text);
        let (fill, ink) = match ink {
            Some(c) => (c.with_alpha(0.16), c),
            None => (lift(self.theme, 0.08), self.theme.text_dim),
        };
        self.layer.rect([x, y + 15.0], [w, 22.0], fill, 4.0);
        self.layer.text(
            sans(self.engine, text, 11.5, Weight::MEDIUM),
            x + 9.0,
            y + 18.5,
            ink,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonKind {
    Primary,
    Secondary,
    Destructive,
    /// Destructive but quiet: an `error` outline and label, no fill (Revoke
    /// beside each paired phone).
    Outline,
    Disabled,
    /// The picked one of a row of choices: a segmented control's active
    /// fill, an accent edge.
    Selected,
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

/// Settings › Remote access: one card with the switch and, while on, how
/// phones reach this computer; then (on) pairing, the paired phones, what a
/// quit does, and the recent connections and connection settings folded
/// away. Off, only the paired phones follow, so one can still be revoked.
fn remote_page(
    form: &mut Form,
    config: &Config,
    view: &RemoteView,
    history_top: usize,
    blink_on: bool,
) {
    let on = config.remote_access;
    form.crumb = format!("{} › Remote access", form.page);
    remote_card(form, config, view);
    if !on {
        paired_phones(form, view, false);
        return;
    }

    form.group("Pair a phone");
    let stage = pair_stage(
        on,
        view.pairing_wait,
        view.pairing.as_ref(),
        std::time::SystemTime::now(),
    );
    match stage {
        PairStage::Showing => {
            if let Some(p) = &view.pairing {
                pairing_panel(form, p, view.view_only);
            }
        }
        PairStage::Expired => {
            let label = "Show a new code";
            let bw = form.engine.measure(label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0;
            form.reserve = bw + 40.0;
            let y = form.y;
            form.label_desc("The code expired", "Each code works once, for 2 minutes", y);
            if !form.hidden {
                let by = y + (ROW_H - CONTROL_H) / 2.0;
                form.button(
                    form.right() - bw.round(),
                    by,
                    label,
                    ButtonKind::Primary,
                    ClickTarget::RemotePair,
                    "The code expired: show a new code".into(),
                );
            }
        }
        PairStage::Idle | PairStage::Waiting | PairStage::Hidden => {
            let label = "Pair a phone";
            let bw = form.engine.measure(label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0;
            form.reserve = bw + 40.0;
            let y = form.y;
            form.label_desc(
                "Add a phone",
                "Show a code to scan with the Rift app · one use, valid 2 minutes",
                y,
            );
            if !form.hidden {
                let by = y + (ROW_H - CONTROL_H) / 2.0;
                form.button(
                    form.right() - bw.round(),
                    by,
                    label,
                    ButtonKind::Primary,
                    ClickTarget::RemotePair,
                    label.into(),
                );
            }
            if stage == PairStage::Waiting {
                let y = form.y;
                let text = match &view.gateway {
                    GatewayLine::Listening(a) if a.is_empty() => "No network address to pair over",
                    GatewayLine::Listening(_) | GatewayLine::Connected(_)
                        if config.remote_tunnel && view.cloudflared.is_none() =>
                    {
                        "Install cloudflared first"
                    }
                    GatewayLine::Listening(_) | GatewayLine::Connected(_)
                        if config.remote_tunnel =>
                    {
                        "Waiting for the tunnel…"
                    }
                    _ => "Waiting for the gateway to start…",
                };
                form.display(y, "Pairing", text);
            }
        }
    }
    let y = form.y;
    form.checkbox(
        y,
        "The new phone can only watch",
        "It sees your tabs but can't run commands or send keys",
        view.view_only,
        ClickTarget::RemoteViewOnly,
    );

    paired_phones(form, view, true);

    form.group("When you quit");
    let (title, line) = if !cfg!(target_os = "macos") {
        (
            "Rift quits",
            "Phones can no longer reach it until you open Rift again.",
        )
    } else if !config.durable_sessions {
        (
            "Rift quits",
            "Keep shells running through updates is off in General, so nothing stays behind.",
        )
    } else {
        (
            "Your shells end; the menu bar icon stays",
            "Phones that can run commands can still open tabs. Quit Rift completely from the menu bar icon.",
        )
    };
    form.note_row(title, line);

    form.y += 16.0;
    let n = view.history.len();
    form.disclosure(
        "Recent connections",
        (n > 0).then(|| n.to_string()),
        view.history_open,
        ClickTarget::RemoteHistory,
    );
    if view.history_open {
        recent_connections(form, view, history_top);
    }
    form.disclosure(
        "Connection settings",
        None,
        view.relay_open,
        ClickTarget::RemoteRelaySettings,
    );
    if view.relay_open {
        connection_settings(form, config, view, blink_on);
    }
}

/// Settings › Remote access's top card: a phone icon, what remote access
/// is doing (title and a line), the switch, and while on how phones reach
/// this computer (On this Wi-Fi, Away from home) with the encryption line.
fn remote_card(form: &mut Form, config: &Config, view: &RemoteView) {
    const PAD: f32 = 18.0;
    const ICON: f32 = 40.0;
    const DESC_LH: f32 = 12.5 * 1.4;
    let on = config.remote_access;
    let theme = form.theme;
    form.hidden = false;
    form.rows += 1;
    form.groups += 1;
    let (x, w) = (form.x, form.card_w);
    let top = form.y + 4.0;
    let switch_x = x + w - PAD - 34.0;
    let changed = is_changed(form.current, form.defaults, "remote_access");
    let text_x = x + PAD + ICON + 16.0;
    let text_w = (switch_x - 16.0 - if changed { 32.0 } else { 0.0 } - text_x).max(120.0);
    let (title, line) = remote_heading(on, &view.gateway);
    let desc = wrap_lines(form.engine, line, 12.5, FONT_SANS, text_w);
    let head_h = (24.0 + desc.len() as f32 * DESC_LH).max(ICON);
    let inner = [x + PAD, top + PAD + head_h + 16.0, w - 2.0 * PAD];
    let tiles = on.then(|| remote_tiles(config, view));
    let layout = tiles
        .as_ref()
        .map(|t| tile_layout(form.engine, t, inner[2]));
    let tiles_h = layout.as_ref().map_or(0.0, |l| l.height);
    // the lock line under the tiles
    let lock_h = if on { 12.0 + 18.0 } else { 0.0 };
    let h = (PAD + head_h + if on { 16.0 + tiles_h + lock_h } else { 0.0 } + PAD).round();
    if form.visible_band(top, h) {
        form.layer
            .bordered([x, top], [w, h], lift(theme, 0.025), theme.border, 8.0, 1.0);
        let (fill, ink) = if on {
            (theme.accent.with_alpha(0.14), theme.accent)
        } else {
            (lift(theme, 0.06), theme.text_muted)
        };
        let ic = [x + PAD, top + PAD];
        form.layer.rect(ic, [ICON, ICON], fill, 8.0);
        icons::draw(
            form.layer,
            Icon::Phone,
            [ic[0] + ICON / 2.0, ic[1] + ICON / 2.0],
            20.0,
            ink,
        );
        form.layer.text(
            sans(form.engine, &title, 15.0, Weight::SEMIBOLD),
            text_x,
            top + PAD,
            theme.text_bright,
        );
        for (i, l) in desc.iter().enumerate() {
            form.layer.text(
                sans(form.engine, l, 12.5, Weight::NORMAL),
                text_x,
                top + PAD + 24.0 + i as f32 * DESC_LH,
                theme.text_dim,
            );
        }
    }
    // the card's words, for assistive tech
    form.row_label = title.clone();
    form.push_hit(
        [text_x, top + PAD, text_w, head_h],
        ClickTarget::SettingsNoop,
    );
    form.name_last(format!("{title}. {line}"));
    // the switch, level with the title
    let ty = top + PAD;
    if form.visible(ty, 20.0) {
        let bg = if on { theme.accent } else { lift(theme, 0.14) };
        form.layer.rect([switch_x, ty], [34.0, 20.0], bg, 10.0);
        let knob_x = if on { switch_x + 17.0 } else { switch_x + 3.0 };
        let knob = if on || theme.is_light() {
            Color::WHITE
        } else {
            theme.text_ui
        };
        form.layer.circle([knob_x + 7.0, ty + 10.0], 14.0, knob);
    }
    form.row_label = "Allow remote access".into();
    form.label_end = text_x
        + form
            .engine
            .measure(&title, 15.0, FONT_SANS, Weight::SEMIBOLD);
    form.label_mid = top + PAD + 10.0;
    // ↺ and the changed dot use a row's geometry: centre them on the switch
    form.changed("remote_access", ty + 10.0 - ROW_H / 2.0, switch_x);
    form.row_label = "Allow remote access".into();
    form.push_hit(
        [switch_x - 4.0, ty - 4.0, 42.0, 28.0],
        ClickTarget::SettingsToggle("remote_access"),
    );
    if let (Some(tiles), Some(layout)) = (&tiles, &layout) {
        draw_tiles(form, tiles, layout, [inner[0], inner[1]]);
        let y = inner[1] + tiles_h + 12.0;
        let text = "End-to-end encrypted: the relay passes bytes it cannot read.";
        if form.visible(y, 18.0) {
            icons::draw(
                form.layer,
                Icon::Lock,
                [inner[0] + 6.0, y + 8.0],
                12.0,
                theme.text_muted,
            );
            form.layer.text(
                sans(form.engine, text, 12.0, Weight::NORMAL),
                inner[0] + 20.0,
                y,
                theme.text_muted,
            );
        }
        form.row_label = "Encryption".into();
        form.push_hit([inner[0], y, inner[2], 18.0], ClickTarget::SettingsNoop);
        form.name_last(text.into());
    }
    form.y = top + h + 4.0;
}

/// One tile of the Remote access card: how phones reach this computer one
/// way, and how that is going.
struct Tile {
    mark: Mark,
    label: &'static str,
    /// After the label (the relay's round trip).
    tag: Option<String>,
    /// An address or host, monospace (empty: none).
    value: String,
    note: String,
    error: bool,
    /// The whole error, in a block under the note.
    detail: Option<String>,
    /// A "Try again" button at the tile's foot.
    retry: bool,
}

/// The two tiles while remote access is on: On this Wi-Fi (the listening
/// addresses) and Away from home (the relay, or the tunnel's relay).
fn remote_tiles(config: &Config, view: &RemoteView) -> [Tile; 2] {
    let (mark, note, value) = match &view.gateway {
        GatewayLine::Off | GatewayLine::Starting => (Mark::Wait, "Starting…", String::new()),
        _ if view.addresses.is_empty() => {
            (Mark::Off, "Listening, but on no network", String::new())
        }
        _ => (
            Mark::Ok,
            "Phones on the same Wi-Fi connect directly",
            view.addresses.join(", "),
        ),
    };
    let lan = Tile {
        mark,
        label: "On this Wi-Fi",
        tag: None,
        value,
        note: note.into(),
        error: false,
        detail: None,
        retry: false,
    };
    // the tunnel replaces the relay: its state is the relay's
    let tunnel = config.remote_tunnel;
    let tunnel_ready = matches!(view.tunnel, Some(crate::tunnel::TunnelState::Ready { .. }));
    let line = &view.relay;
    let away = if tunnel && !tunnel_ready {
        Tile {
            mark: Mark::Wait,
            label: "Away from home",
            tag: None,
            value: String::new(),
            note: "Waiting for the tunnel".into(),
            error: false,
            detail: None,
            retry: false,
        }
    } else {
        let mark = match line {
            RelayLine::Off | RelayLine::Stopped | RelayLine::Scripted => Mark::Off,
            RelayLine::Connecting(_) => Mark::Wait,
            RelayLine::Connected { .. } => Mark::Ok,
            RelayLine::Error { .. } => Mark::Failed,
        };
        // the round trip is the tile's tag, not part of the sentence
        let (note, tag) = match line {
            RelayLine::Connected { latency_ms, .. } => (
                "Phones away from home connect through it".to_string(),
                latency_ms.map(|ms| format!("{ms}\u{a0}ms")),
            ),
            _ => (line.text(), None),
        };
        let note = if tunnel {
            format!("{note} · the tunnel's relay")
        } else {
            note
        };
        let error = matches!(line, RelayLine::Error { .. });
        Tile {
            mark,
            label: "Away from home",
            tag,
            value: line.host().map_or_else(|| "—".to_string(), str::to_string),
            note,
            error,
            detail: line.reason().map(str::to_string),
            retry: error,
        }
    };
    [lan, away]
}

/// Where each tile goes in a row `width` wide (side by side when both fit,
/// else one under the other) and what each holds, wrapped.
struct TileLayout {
    /// `[x offset, y offset, width, height]` per tile.
    rects: [[f32; 4]; 2],
    notes: [Vec<String>; 2],
    details: [Vec<String>; 2],
    height: f32,
}

const TILE_PAD: f32 = 12.0;
const TILE_GAP: f32 = 10.0;
/// Narrowest tile before the two stack.
const TILE_MIN_W: f32 = 220.0;

fn tile_height(note: usize, detail: usize, value: bool, retry: bool) -> f32 {
    let mut h = TILE_PAD + 18.0;
    if value {
        h += 18.0;
    }
    h += note as f32 * NOTE_LH;
    if detail > 0 {
        h += 6.0 + detail as f32 * DETAIL_LH + 16.0;
    }
    if retry {
        h += 8.0 + CONTROL_H;
    }
    h + TILE_PAD
}

fn tile_layout(engine: &mut TextEngine, tiles: &[Tile; 2], width: f32) -> TileLayout {
    let side = (width - TILE_GAP) / 2.0 >= TILE_MIN_W;
    let tw = if side {
        ((width - TILE_GAP) / 2.0).floor()
    } else {
        width
    };
    let text_w = tw - 2.0 * TILE_PAD - BADGE_W - BADGE_GAP;
    let notes = [
        wrap_lines(engine, &tiles[0].note, 12.0, FONT_SANS, text_w),
        wrap_lines(engine, &tiles[1].note, 12.0, FONT_SANS, text_w),
    ];
    let detail = |t: &Tile, engine: &mut TextEngine| {
        t.detail
            .as_deref()
            .map(|d| wrap_lines(engine, d, 11.5, mono_family(), text_w - 20.0))
            .unwrap_or_default()
    };
    let details = [detail(&tiles[0], engine), detail(&tiles[1], engine)];
    let hs: Vec<f32> = (0..2)
        .map(|i| {
            tile_height(
                notes[i].len(),
                details[i].len(),
                !tiles[i].value.is_empty(),
                tiles[i].retry,
            )
            .round()
        })
        .collect();
    let (rects, height) = if side {
        let h = hs[0].max(hs[1]);
        (
            [
                [0.0, 0.0, tw, h],
                [tw + TILE_GAP, 0.0, width - tw - TILE_GAP, h],
            ],
            h,
        )
    } else {
        (
            [[0.0, 0.0, tw, hs[0]], [0.0, hs[0] + TILE_GAP, tw, hs[1]]],
            hs[0] + TILE_GAP + hs[1],
        )
    };
    TileLayout {
        rects,
        notes,
        details,
        height,
    }
}

fn draw_tiles(form: &mut Form, tiles: &[Tile; 2], layout: &TileLayout, origin: [f32; 2]) {
    let theme = form.theme;
    for (i, t) in tiles.iter().enumerate() {
        let r = layout.rects[i];
        let (x, y) = (origin[0] + r[0], origin[1] + r[1]);
        let text_x = x + TILE_PAD + BADGE_W + BADGE_GAP;
        let text_w = r[2] - 2.0 * TILE_PAD - BADGE_W - BADGE_GAP;
        let mut ty = y + TILE_PAD;
        if form.visible(y, r[3]) {
            form.layer.bordered(
                [x, y],
                [r[2], r[3]],
                theme.ground(theme.bg).with_alpha(0.55),
                lift(theme, 0.08),
                6.0,
                1.0,
            );
            let (fill, ink) = match t.mark {
                Mark::Ok => (theme.success.with_alpha(0.15), theme.success),
                Mark::Wait => (theme.accent.with_alpha(0.15), theme.accent),
                Mark::Failed => (theme.error.with_alpha(0.15), theme.error),
                Mark::Off => (lift(theme, 0.08), theme.text_dim),
            };
            let c = [x + TILE_PAD + BADGE_W / 2.0, ty + 9.0];
            form.layer.circle(c, BADGE_W, fill);
            match t.mark {
                Mark::Ok => icons::draw(form.layer, Icon::Check, c, 12.0, ink),
                Mark::Failed => icons::draw(form.layer, Icon::Close, c, 10.0, ink),
                Mark::Wait | Mark::Off => {
                    let glyph = if t.mark == Mark::Wait { "…" } else { "–" };
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
                sans(form.engine, t.label, 13.0, Weight::NORMAL),
                text_x,
                ty + 1.0,
                theme.text_ui,
            );
            if let Some(tag) = &t.tag {
                let lw = form
                    .engine
                    .measure(t.label, 13.0, FONT_SANS, Weight::NORMAL);
                let tw = form.engine.measure(tag, 11.0, FONT_SANS, Weight::MEDIUM);
                let tx = text_x + lw + 8.0;
                form.layer
                    .rect([tx, ty + 1.0], [tw + 12.0, 17.0], lift(theme, 0.08), 3.0);
                form.layer.text(
                    sans(form.engine, tag, 11.0, Weight::MEDIUM),
                    tx + 6.0,
                    ty + 2.5,
                    theme.text_dim,
                );
            }
        }
        ty += 18.0;
        if !t.value.is_empty() {
            if form.visible(ty, 18.0) {
                let v =
                    super::palette::elide_to(form.engine, &t.value, 12.0, mono_family(), text_w);
                form.layer.text(
                    mono(form.engine, &v, 12.0, Weight::NORMAL),
                    text_x,
                    ty + 1.0,
                    theme.text_ui,
                );
            }
            ty += 18.0;
        }
        let note_ink = match (t.error, t.mark) {
            (true, _) => theme.error,
            (_, Mark::Wait) => theme.accent,
            _ => theme.text_muted,
        };
        for l in &layout.notes[i] {
            if form.visible(ty, NOTE_LH) {
                form.layer.text(
                    sans(form.engine, l, 12.0, Weight::NORMAL),
                    text_x,
                    ty,
                    note_ink,
                );
            }
            ty += NOTE_LH;
        }
        let lines = &layout.details[i];
        if !lines.is_empty() {
            let top = ty + 6.0;
            let bh = lines.len() as f32 * DETAIL_LH + 16.0;
            if form.visible(top, bh) {
                form.layer.rect([text_x, top], [text_w, bh], theme.bg, 4.0);
                for (j, l) in lines.iter().enumerate() {
                    form.layer.text(
                        mono(form.engine, l, 11.5, Weight::NORMAL),
                        text_x + 10.0,
                        top + 8.0 + j as f32 * DETAIL_LH,
                        theme.error,
                    );
                }
            }
            ty = top + bh;
        }
        // read-only, but named for assistive tech: the whole text
        let mut name = t.label.to_string();
        if !t.value.is_empty() && t.value != "—" {
            name.push_str(&format!(" ({})", t.value));
        }
        name.push_str(&format!(": {}", t.note));
        if let Some(tag) = &t.tag {
            name.push_str(&format!(" · {tag}"));
        }
        if let Some(d) = &t.detail {
            name.push_str(&format!(
                "{} {d}",
                if name.ends_with('.') { "" } else { "." }
            ));
        }
        form.row_label = t.label.into();
        form.push_hit([x, y, r[2], r[3]], ClickTarget::SettingsNoop);
        form.name_last(name);
        if t.retry {
            form.button(
                text_x,
                ty + 8.0,
                "Try again",
                ButtonKind::Secondary,
                ClickTarget::RemoteRelayRetry,
                "Try again: retry the relay now".into(),
            );
        }
    }
}

/// Settings › Remote access › Paired phones: each phone's dot (green while
/// connected), name and line, its access (on) and Revoke. Off, the list
/// shows only when there is one, and says why they cannot connect.
fn paired_phones(form: &mut Form, view: &RemoteView, on: bool) {
    let n = view.devices.len();
    if !on && n == 0 && view.devices_error.is_none() {
        return;
    }
    form.group(&if n > 0 {
        format!("Paired phones ({n})")
    } else {
        "Paired phones".to_string()
    });
    if !on && n > 0 {
        form.note_row(
            "",
            "They can't connect while remote access is off. Revoke one to remove it for good.",
        );
    }
    if let Some(e) = &view.devices_error {
        let y = form.y;
        form.display(y, "Paired phones", e);
    } else if n == 0 && view.devices_loaded {
        let y = form.y;
        form.label_desc(
            "No phones yet",
            "Pair one above; it shows here until you revoke it",
            y,
        );
    }
    const ACCESS: [&str; 2] = ["Can run commands", "View only"];
    for d in &view.devices {
        let y = form.y;
        let detail = if on {
            d.detail.clone()
        } else {
            format!("Can’t connect while off · {}", d.about)
        };
        let bw = form
            .engine
            .measure("Revoke", 12.5, FONT_SANS, Weight::MEDIUM)
            + 24.0;
        let sw = if on {
            form.segmented_width(&ACCESS)
        } else {
            0.0
        };
        form.reserve = bw + if on { 8.0 + sw } else { 0.0 } + 40.0;
        // how it is connected must never be cut: when the line does not fit
        // beside the controls, it wraps under them
        let beside = form.card_w - form.reserve - 8.0;
        let fits = !form.filter.is_empty()
            || form
                .engine
                .measure(&detail, 12.0, FONT_SANS, Weight::NORMAL)
                <= beside;
        let below = if fits {
            Vec::new()
        } else {
            wrap_lines(form.engine, &detail, 12.0, FONT_SANS, form.card_w)
        };
        form.extra_h = if below.is_empty() {
            0.0
        } else {
            below.len() as f32 * NOTE_LH + 2.0
        };
        form.label_desc(&d.name, if fits { &detail } else { "" }, y);
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
                DevicePath::Lan | DevicePath::Relay if on => form.theme.success,
                _ => form.theme.text_muted,
            };
            form.layer
                .circle([form.label_end + 12.0, form.label_mid], 8.0, color);
        }
        if !form.hidden {
            let by = y + (ROW_H - CONTROL_H) / 2.0;
            let bx = form.right() - bw.round();
            if on {
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
            }
            form.button(
                bx,
                by,
                "Revoke",
                ButtonKind::Outline,
                ClickTarget::RemoteRevoke(d.id.clone()),
                format!("Revoke {}", d.name),
            );
        }
    }
}

/// Settings › Remote access › Recent connections (open): at most
/// HISTORY_SHOWN rows at once; more scroll inside the list by whole rows,
/// with a thumb at its right edge.
fn recent_connections(form: &mut Form, view: &RemoteView, history_top: usize) {
    if view.history.is_empty() && view.devices_loaded {
        let y = form.y;
        form.label_desc(
            "No connections yet",
            "Phones that connect, and refused attempts, show here",
            y,
        );
    }
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

/// Settings › Remote access › Connection settings (open): the listen
/// address, the relay address (not while the tunnel replaces it) and the
/// Cloudflare tunnel.
fn connection_settings(form: &mut Form, config: &Config, view: &RemoteView, blink_on: bool) {
    let y = form.y;
    form.text_row(
        y,
        &TextRow {
            label: "Listen address",
            desc: "IP:PORT phones on this network reach; 0.0.0.0 is every network",
            value: &config.remote_listen,
            placeholder: "0.0.0.0:7878",
            id: "remote_listen",
            read_only: false,
        },
    );
    // the tunnel's relay has no address to edit
    if !config.remote_tunnel {
        relay_row(form, config, view, blink_on);
    }
    tunnel_rows(form, config, view);
    let y = form.y;
    form.toggle_desc(
        y,
        "Copy to phones too",
        "What a program copies also reaches the phones that may operate; a program could write to their clipboard unasked",
        config.remote_clipboard,
        "remote_clipboard",
    );
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

/// The live pairing code, in a panel: the QR code (black modules on a
/// white plate with its quiet zone, so it scans in dark themes too), how to
/// scan it, what the phone may do, the countdown, Copy link and Cancel, and
/// the link itself.
fn pairing_panel(form: &mut Form, p: &PairingView, view_only: bool) {
    const PAD: f32 = 16.0;
    let left = p
        .expires_at
        .duration_since(std::time::SystemTime::now())
        .unwrap_or_default();
    let theme = form.theme;
    form.hidden = false;
    form.rows += 1;
    form.row_label = "Pairing".into();
    let top = form.y + 12.0;
    let n = p.qr.len();
    let plate = ((n + 2 * QR_QUIET) as f32 * QR_MODULE).round();
    let (x, w) = (form.x, form.card_w);
    let (px, py) = ((x + PAD).round(), (top + PAD).round());
    // the words beside the code when there is room, else under it
    let side = w - 2.0 * PAD - plate - 20.0 >= 240.0;
    let (tx, ty0, tw) = if side {
        (px + plate + 20.0, py, w - 2.0 * PAD - plate - 20.0)
    } else {
        (px, py + plate + 16.0, w - 2.0 * PAD)
    };
    let how = if view_only {
        "Open Rift on your phone, add a computer, and point the camera here. It will only be able to watch."
    } else {
        "Open Rift on your phone, add a computer, and point the camera here. It will be able to run commands."
    };
    let how_lines = wrap_lines(form.engine, how, 12.5, FONT_SANS, tw);
    let how_lh = 12.5 * 1.4;
    // the link, wrapped by characters (it has no spaces)
    let cw = form
        .engine
        .measure("0000000000", 11.5, mono_family(), Weight::NORMAL)
        / 10.0;
    let per = ((tw / cw).floor() as usize).max(8);
    let chars: Vec<char> = p.url.chars().collect();
    let link: Vec<String> = chars.chunks(per).map(|c| c.iter().collect()).collect();
    let link_lh = 11.5 * 1.35;
    let words_h = 22.0
        + how_lines.len() as f32 * how_lh
        + 8.0
        + 20.0
        + 10.0
        + CONTROL_H
        + 12.0
        + link.len() as f32 * link_lh;
    let body_h = if side {
        plate.max(words_h)
    } else {
        plate + 16.0 + words_h
    };
    let h = (PAD + body_h + PAD).round();
    if form.visible_band(top, h) {
        form.layer.bordered(
            [x, top],
            [w, h],
            theme.accent.with_alpha(0.05),
            theme.accent.with_alpha(0.35),
            8.0,
            1.0,
        );
    }
    if form.visible_band(py, plate) && n > 0 {
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
    let mut ty = ty0;
    let expires = format!("Expires in {}", countdown(left));
    if form.visible_band(ty, words_h) {
        form.layer.text(
            sans(
                form.engine,
                "Scan with the Rift app",
                15.0,
                Weight::SEMIBOLD,
            ),
            tx,
            ty,
            theme.text_bright,
        );
        for (i, l) in how_lines.iter().enumerate() {
            form.layer.text(
                sans(form.engine, l, 12.5, Weight::NORMAL),
                tx,
                ty + 26.0 + i as f32 * how_lh,
                theme.text_dim,
            );
        }
    }
    form.push_hit(
        [tx, ty, tw, 22.0 + how_lines.len() as f32 * how_lh],
        ClickTarget::SettingsNoop,
    );
    form.name_last(format!("Scan with the Rift app. {how}"));
    ty += 22.0 + how_lines.len() as f32 * how_lh + 8.0;
    if form.visible_band(ty, 20.0) {
        form.layer.text(
            sans(form.engine, &expires, 13.0, Weight::NORMAL),
            tx,
            ty,
            theme.text_ui,
        );
    }
    form.push_hit([tx, ty, tw, 20.0], ClickTarget::SettingsNoop);
    form.name_last(expires);
    ty += 20.0 + 10.0;
    let copy_w = form.button(
        tx,
        ty,
        "Copy link",
        ButtonKind::Secondary,
        ClickTarget::RemoteCopyLink,
        "Copy pairing link".into(),
    );
    form.button(
        tx + copy_w + 8.0,
        ty,
        "Cancel",
        ButtonKind::Secondary,
        ClickTarget::RemotePairCancel,
        "Cancel pairing: the code stops working".into(),
    );
    ty += CONTROL_H + 12.0;
    for (i, l) in link.iter().enumerate() {
        let ly = ty + i as f32 * link_lh;
        if form.visible_band(ly, link_lh) {
            form.layer.text(
                mono(form.engine, l, 11.5, Weight::NORMAL),
                tx,
                ly,
                theme.text_muted,
            );
        }
    }
    let link_h = link.len() as f32 * link_lh;
    form.push_hit([tx, ty, tw, link_h], ClickTarget::SettingsNoop);
    form.name_last(format!("Pairing link: {}", p.url));
    form.y = top + h + 6.0;
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

/// Words as a text field shows them, each quoted when it needs it (a
/// profile's arguments and environment).
pub fn join_words<S: AsRef<str>>(words: &[S]) -> String {
    words
        .iter()
        .map(|w| rift_core::ssh::quote(w.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A text field's words: split at blanks, quotes and backslashes as in a
/// shell.
pub fn split_words(text: &str) -> Vec<String> {
    rift_core::completion_specs::tokenize(text)
}

/// A profile's environment as its field shows it: `NAME=value …`.
pub fn env_text(env: &std::collections::BTreeMap<String, String>) -> String {
    let pairs: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    join_words(&pairs)
}

/// What a profile runs, as its row describes it: the program and its
/// arguments, or "Shell path" (the global `shell`).
pub fn profile_command(p: &rift_core::profiles::Profile) -> String {
    let mut what = p
        .command
        .clone()
        .unwrap_or_else(|| "Shell path".to_string());
    if !p.args.is_empty() {
        what.push(' ');
        what.push_str(&join_words(&p.args));
    }
    what
}

/// A profile's tab colour as its select shows it (`None`: no colour).
pub fn profile_color_label(color: Option<&str>) -> String {
    match color.map(str::trim).filter(|c| !c.is_empty()) {
        None => "None".into(),
        Some(c) if c.starts_with('#') => c.to_string(),
        Some(c) => {
            let mut chars = c.chars();
            chars
                .next()
                .map(|f| f.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// The Default profile select's value for no default profile.
pub const NO_DEFAULT_PROFILE: &str = "Shell path";

/// Settings › Profiles' own settings (also what the search finds): the
/// default profile and the installed shells.
fn profile_settings(form: &mut Form, config: &Config) {
    form.group("New tabs");
    let default = config
        .default_profile
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let y = form.y;
    form.select(
        y,
        "Default profile",
        default.unwrap_or(NO_DEFAULT_PROFILE),
        "default_profile",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Add installed shells as profiles",
        "Each shell in /etc/shells, once; a deleted one stays deleted",
        config.discover_shells,
        "discover_shells",
    );
}

/// Settings › Profiles: the default profile, the installed shells, then
/// every `[[profiles]]` entry in file order (badges, Edit, drag to move),
/// the open one's form under it, and Add.
fn profiles_page(form: &mut Form, config: &Config, state: &SettingsState) {
    profile_settings(form, config);
    let default = config
        .default_profile
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    form.group("Your profiles");
    let drag = state.profile_drag.filter(|d| d.live);
    let n = config.profiles.len();
    // the dragged row's landing place: a caret at its top edge (or the
    // last row's bottom)
    let mut caret = None;
    for (i, p) in config.profiles.iter().enumerate() {
        // (just above or below itself it would not move)
        if drag.is_some_and(|d| d.to == i && d.to != d.from && d.to != d.from + 1) {
            caret = Some(form.y);
        }
        let open = state.profile_open == Some(i);
        profile_row(
            form,
            i,
            n,
            p,
            default == Some(p.name.trim()),
            open,
            drag.is_some_and(|d| d.from == i),
        );
        if open {
            profile_form(
                form,
                i,
                p,
                default == Some(p.name.trim()),
                state.profile_error.as_deref(),
            );
        }
    }
    if drag.is_some_and(|d| d.to >= n && d.from + 1 < n) {
        caret = Some(form.y);
    }
    if let Some(cy) = caret
        && form.visible_band(cy - 1.0, 2.0)
    {
        form.layer.rect(
            [form.x, cy - 1.0],
            [form.card_w, 2.0],
            form.theme.accent,
            0.0,
        );
    }
    if config.profiles.is_empty() {
        let y = form.y;
        form.label_desc(
            "No profiles",
            "New tabs run the Shell path; add a profile for another shell or program",
            y,
        );
    }
    let y = form.y;
    let label = "Add";
    let bw = form.engine.measure(label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0;
    form.reserve = bw + 40.0;
    form.label_desc(
        "New profile",
        "Runs your login shell until you change it",
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
            ClickTarget::ProfileAdd,
            "Add a profile".into(),
        );
    }
}

/// Width of the drag grip column left of a profile's name.
const GRIP_W: f32 = 24.0;

/// One profile's row: grip, name over what it runs, its badges (Default,
/// Detected, Hidden, Not found) and Edit (Done while its form is open).
#[allow(clippy::too_many_arguments)]
fn profile_row(
    form: &mut Form,
    i: usize,
    n: usize,
    p: &rift_core::profiles::Profile,
    is_default: bool,
    open: bool,
    dragged: bool,
) {
    let y = form.y;
    let theme = form.theme;
    let mut badges: Vec<(&str, Option<Color>)> = Vec::new();
    if is_default {
        badges.push(("Default", Some(theme.accent)));
    }
    if p.is_generated() {
        badges.push(("Detected", None));
    }
    if p.hidden {
        badges.push(("Hidden", None));
    }
    if p.orphaned {
        badges.push(("Not found", Some(theme.error)));
    }
    let edit = if open { "Done" } else { "Edit" };
    let bw = (form.engine.measure(edit, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0).round();
    let widths: Vec<f32> = badges.iter().map(|(t, _)| form.badge_width(t)).collect();
    let tags_w: f32 = widths.iter().map(|w| w + 8.0).sum();
    form.reserve = bw + tags_w + 40.0;
    let what = profile_command(p);
    // the name sits right of the grip
    form.x += GRIP_W;
    form.card_w -= GRIP_W;
    form.label_desc(&p.name, &what, y);
    form.x -= GRIP_W;
    form.card_w += GRIP_W;
    if form.hidden {
        return;
    }
    let row = [form.x, y, form.card_w, ROW_H];
    if form.visible(y, ROW_H) {
        if dragged {
            form.layer.rect(
                [row[0], row[1]],
                [row[2], row[3] - 1.0],
                lift(theme, 0.07),
                0.0,
            );
        } else if form.hot(row) {
            form.layer.rect(
                [row[0], row[1]],
                [row[2], row[3] - 1.0],
                lift(theme, 0.03),
                0.0,
            );
        }
        icons::draw(
            form.layer,
            Icon::Grip,
            [form.x + 8.0, y + ROW_H / 2.0],
            14.0,
            theme.text_muted,
        );
        form.layer
            .hline(form.x, y + ROW_H - 1.0, GRIP_W, 1.0, lift(theme, 0.06));
    }
    let bx = form.right() - bw;
    let mut tx = bx - tags_w;
    for ((text, ink), w) in badges.iter().zip(&widths) {
        form.badge(tx, y, text, *ink);
        tx += w + 8.0;
    }
    form.button(
        bx,
        y + (ROW_H - CONTROL_H) / 2.0,
        edit,
        ButtonKind::Secondary,
        ClickTarget::Profile(i, crate::state::ProfileOp::Edit),
        format!("{edit} {}", p.name),
    );
    // after the button, so a click finds the button first
    let tags: Vec<&str> = badges.iter().map(|(t, _)| *t).collect();
    form.push_hit(row, ClickTarget::ProfileRow(i));
    form.name_last(format!(
        "Profile {}, {} of {n}{}{}: {what}. Drag, or Option up and down arrows, to move it",
        p.name,
        i + 1,
        if tags.is_empty() { "" } else { ", " },
        tags.join(", ").to_lowercase(),
    ));
}

/// The open profile's form: its fields (a detected one's command shown, not
/// editable), why the last change was refused, and Set as default,
/// Duplicate, Hide / Show, Delete.
fn profile_form(
    form: &mut Form,
    i: usize,
    p: &rift_core::profiles::Profile,
    is_default: bool,
    error: Option<&str>,
) {
    form.x += GRIP_W;
    form.card_w -= GRIP_W;
    let detected = p.is_generated();
    let args = join_words(&p.args);
    let env = env_text(&p.env);
    let command = p.command.clone().unwrap_or_default();
    let fields = [
        TextRow {
            label: "Name",
            desc: "Shown in the + menu and the palette",
            value: &p.name,
            placeholder: "",
            id: "profile.name",
            read_only: false,
        },
        TextRow {
            label: "Command",
            desc: if detected {
                "Duplicate to change the command"
            } else {
                "A path, ~/…, or a program on PATH"
            },
            value: &command,
            placeholder: NO_DEFAULT_PROFILE,
            id: "profile.command",
            read_only: detected,
        },
        TextRow {
            label: "Arguments",
            desc: "Separated by spaces; quote one that has spaces",
            value: &args,
            placeholder: "None",
            id: "profile.args",
            read_only: false,
        },
        TextRow {
            label: "Directory",
            desc: "Where its tabs start (~ expands)",
            value: p.cwd.as_deref().unwrap_or(""),
            placeholder: "As New tabs start in",
            id: "profile.cwd",
            read_only: false,
        },
        TextRow {
            label: "Environment",
            desc: "NAME=value pairs, separated by spaces",
            value: &env,
            placeholder: "None",
            id: "profile.env",
            read_only: false,
        },
        TextRow {
            label: "Tab title",
            desc: "",
            value: p.title.as_deref().unwrap_or(""),
            placeholder: "The program's own",
            id: "profile.title",
            read_only: false,
        },
    ];
    for f in &fields {
        let y = form.y;
        form.text_row(y, f);
    }
    let y = form.y;
    form.select(
        y,
        "Tab color",
        &profile_color_label(p.color.as_deref()),
        "profile.color",
    );
    let y = form.y;
    form.select(
        y,
        "Icon",
        &profile_icon_label(p.icon.as_deref()),
        "profile.icon",
    );
    let y = form.y;
    match p.theme.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => form.theme_select(y, "Colour scheme", t, "profile.theme"),
        None => form.select(y, "Colour scheme", "Window theme", "profile.theme"),
    }
    if let Some(e) = error {
        let lines = wrap_lines(form.engine, e, 12.0, FONT_SANS, form.card_w);
        let y = form.y + 8.0;
        for (k, l) in lines.iter().enumerate() {
            let ly = y + k as f32 * NOTE_LH;
            if form.visible_band(ly, NOTE_LH) {
                form.layer.text(
                    sans(form.engine, l, 12.0, Weight::NORMAL),
                    form.x,
                    ly,
                    form.theme.error,
                );
            }
        }
        let h = lines.len() as f32 * NOTE_LH;
        form.hidden = false;
        form.row_label = "Not saved".into();
        form.push_hit([form.x, y, form.card_w, h], ClickTarget::SettingsNoop);
        form.name_last(format!("Not saved: {e}"));
        form.y = y + h;
    }
    // the actions, left to right; a row too narrow wraps them
    use crate::state::ProfileOp;
    let name = p.name.clone();
    let buttons = [
        if is_default {
            (
                "Set as default",
                ButtonKind::Disabled,
                ProfileOp::SetDefault,
                format!("Set as default, unavailable: {name} is the default profile"),
            )
        } else {
            (
                "Set as default",
                ButtonKind::Secondary,
                ProfileOp::SetDefault,
                format!("Make {name} the default profile"),
            )
        },
        (
            "Duplicate",
            ButtonKind::Secondary,
            ProfileOp::Duplicate,
            format!("Duplicate {name}"),
        ),
        (
            if p.hidden { "Show" } else { "Hide" },
            ButtonKind::Secondary,
            ProfileOp::ToggleHidden,
            if p.hidden {
                format!("Show {name} in the + menu and the palette")
            } else {
                format!("Hide {name} from the + menu and the palette")
            },
        ),
        (
            "Delete",
            ButtonKind::Destructive,
            ProfileOp::Delete,
            format!("Delete {name}"),
        ),
    ];
    let mut bx = form.x;
    let mut by = form.y + 12.0;
    for (label, kind, op, a11y) in buttons {
        let w = (form.engine.measure(label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0).round();
        if bx > form.x && bx + w > form.right() {
            bx = form.x;
            by += CONTROL_H + 8.0;
        }
        form.hidden = false;
        form.row_label = label.into();
        form.button(bx, by, label, kind, ClickTarget::Profile(i, op), a11y);
        bx += w + 8.0;
    }
    form.y = by + CONTROL_H + 16.0;
    if form.visible_band(form.y - 1.0, 1.0) {
        form.layer.hline(
            form.x - GRIP_W,
            form.y - 1.0,
            form.card_w + GRIP_W,
            1.0,
            lift(form.theme, 0.06),
        );
    }
    form.x -= GRIP_W;
    form.card_w += GRIP_W;
}

/// A profile's icon as its select shows it (`None`: the terminal).
pub fn profile_icon_label(icon: Option<&str>) -> String {
    match icon.map(str::trim).filter(|i| !i.is_empty()) {
        None => "Terminal".into(),
        Some(i) => profile_color_label(Some(i)),
    }
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
    let (regular, bold) = config.font_weights();
    let y = form.y;
    form.stepper_desc(
        y,
        "Font weight",
        "The font draws the nearest weight it has",
        &regular.to_string(),
        "",
        "font_weight",
    );
    let y = form.y;
    form.stepper(
        y,
        "Bold font weight",
        &bold.to_string(),
        "",
        "bold_font_weight",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Bold text in bright colors",
        "Bold text in one of the eight ANSI colors uses its bright variant",
        config.bold_is_bright,
        "bold_is_bright",
    );
    let y = form.y;
    let contrast = config.min_contrast();
    form.stepper_desc(
        y,
        "Minimum contrast",
        "Against its background; WCAG AA asks 4.5",
        &if contrast <= 1.0 {
            "Off".to_string()
        } else {
            format!("{contrast:.1}")
        },
        "",
        "minimum_contrast",
    );
    let y = form.y;
    let features = join_words(&config.font_features);
    form.text_row(
        y,
        &TextRow {
            label: "Font features",
            desc: "OpenType features, e.g. ss01 zero -calt",
            value: &features,
            placeholder: "None",
            id: "font_features",
            read_only: false,
        },
    );
    let y = form.y;
    form.segment_desc(
        y,
        "Ambiguous-width characters",
        "East Asian Ambiguous characters",
        &["Narrow", "Wide"],
        usize::from(config.ambiguous_width == rift_core::AmbiguousWidth::Wide),
        "ambiguous_width",
    );

    form.group("Cursor");
    let y = form.y;
    let active = match config.cursor_style {
        CursorStyle::Block => 0,
        CursorStyle::Bar => 1,
        CursorStyle::Underline => 2,
        CursorStyle::Hollow => 3,
        CursorStyle::DoubleUnderline => 4,
    };
    form.segment(
        y,
        "Cursor style",
        &["Block", "Bar", "Underline", "Hollow", "Double"],
        active,
        "cursor_style",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Hollow when unfocused",
        "Outline the cursor while its window or pane is not focused",
        config.cursor_hollow_unfocused,
        "cursor_hollow_unfocused",
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
        // left and right, as in effect (`window_padding_left` wins)
        &format!("{:.0}", config.paddings(false).left),
        "px",
        "window_padding",
    );
    let pad = config.paddings(false);
    let y = form.y;
    form.stepper(
        y,
        "Top padding",
        &format!("{:.0}", pad.top),
        "px",
        "window_padding_top",
    );
    let y = form.y;
    form.stepper(
        y,
        "Bottom padding",
        &format!("{:.0}", pad.bottom),
        "px",
        "window_padding_bottom",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Pad full-screen programs",
        "Off: vim, htop and the like fill the pane edge to edge",
        config.alt_screen_padding,
        "alt_screen_padding",
    );
    let y = form.y;
    form.icon_select(y, config.app_icon);
    let y = form.y;
    form.toggle_desc(
        y,
        "Font ligatures",
        "Render -> >= != as combined glyphs",
        config.ligatures,
        "ligatures",
    );

    form.group("New windows");
    let y = form.y;
    form.segment(
        y,
        "Open new windows",
        &["As saved", "Maximized", "Full screen"],
        match config.launch_mode {
            rift_core::LaunchMode::Default => 0,
            rift_core::LaunchMode::Maximized => 1,
            rift_core::LaunchMode::Fullscreen => 2,
        },
        "launch_mode",
    );
    let (w, h) = config.window_size();
    let y = form.y;
    form.stepper_desc(
        y,
        "Window width",
        "Without a saved size",
        &format!("{w:.0}"),
        "px",
        "window_width",
    );
    let y = form.y;
    form.stepper_desc(
        y,
        "Window height",
        "Without a saved size",
        &format!("{h:.0}"),
        "px",
        "window_height",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Keep windows on top",
        "New windows float above other apps' windows",
        config.always_on_top,
        "always_on_top",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Resize in whole cells",
        "A window's size snaps to the terminal grid",
        config.snap_to_cells,
        "snap_to_cells",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Show the size while resizing",
        "Columns × rows over the window as it resizes",
        config.resize_overlay,
        "resize_overlay",
    );

    form.group("Tabs");
    let y = form.y;
    form.segment(
        y,
        "Tab position",
        &["Top", "Side"],
        match config.tab_layout {
            TabLayout::Horizontal => 0,
            TabLayout::Vertical => 1,
        },
        "tab_layout",
    );
    let y = form.y;
    form.tab_style_cards(
        y,
        match config.tab_style {
            TabStyle::Dot => 0,
            TabStyle::Underline => 1,
            TabStyle::Detailed => 2,
        },
    );
    let y = form.y;
    form.segment_desc(
        y,
        "Close buttons",
        "On hover shows × only on the tab under the pointer",
        &["On hover", "Always", "Never"],
        match config.tab_close_button {
            TabCloseButton::Hover => 0,
            TabCloseButton::Always => 1,
            TabCloseButton::Never => 2,
        },
        "tab_close_button",
    );
    let y = form.y;
    form.segment(
        y,
        "Close button side",
        &["Right", "Left"],
        match config.tab_close_side {
            TabCloseSide::Right => 0,
            TabCloseSide::Left => 1,
        },
        "tab_close_side",
    );
    let y = form.y;
    form.segment_desc(
        y,
        "Show the tab bar",
        "Out of sight, it slides in at the window's top edge",
        &["Always", "When windowed", "Only on hover"],
        match config.tab_bar {
            TabBarVisibility::Always => 0,
            TabBarVisibility::HideFullscreen => 1,
            TabBarVisibility::OnHover => 2,
        },
        "tab_bar",
    );
    let y = form.y;
    form.segment(
        y,
        "New tabs open",
        &["At the end", "After the current tab"],
        match config.new_tab_placement {
            NewTabPlacement::End => 0,
            NewTabPlacement::AfterCurrent => 1,
        },
        "new_tab_placement",
    );
    let y = form.y;
    form.segment(
        y,
        "⌃Tab switches",
        &["In order", "Last used first"],
        usize::from(config.tab_switch_order == rift_core::TabSwitchOrder::Mru),
        "tab_switch_order",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Show tabs with only one tab open",
        "Off hides the lone tab; the bar's buttons stay",
        config.tab_bar_with_one_tab,
        "tab_bar_with_one_tab",
    );

    // Warp's configurable toolbar: a preview of the strip's buttons that
    // opens "Edit toolbar" (left of the tabs, at the right end, or off;
    // `toolbar_left` / `toolbar_right` in config.toml)
    form.group("Toolbar");
    let y = form.y;
    let (left, right) = config.toolbar();
    form.toolbar_preview(y, &left, &right);

    form.group("Panes");
    let y = form.y;
    form.toggle(
        y,
        "Dim inactive panes",
        config.dim_inactive_panes,
        "dim_inactive_panes",
    );
    if config.dim_inactive_panes {
        let y = form.y;
        form.slider(
            y,
            "Dimming strength",
            dim_slider(config.inactive_pane_dim),
            &format!("{}%", config.inactive_pane_dim.clamp(1, 40)),
            "inactive_pane_dim",
        );
    }
    let y = form.y;
    form.toggle_desc(
        y,
        "Focus follows mouse",
        "The split pane under the pointer takes focus",
        config.focus_follows_mouse,
        "focus_follows_mouse",
    );
}

fn general_page(form: &mut Form, config: &Config, update: &UpdateLine) {
    form.group("App");
    // the login item is macOS's
    if cfg!(target_os = "macos") {
        let y = form.y;
        form.toggle(
            y,
            "Launch at login",
            config.launch_at_login,
            "launch_at_login",
        );
    }

    form.group("Shell");
    // unset: what runs (a configured shell that is not executable falls
    // back); profiles are on their own page
    let resolved = rift_core::shell_integration::resolve_shell(None);
    let y = form.y;
    form.text_row(
        y,
        &TextRow {
            label: "Shell path",
            desc: "What tabs run without a profile; empty: your login shell",
            value: config.shell.as_deref().unwrap_or(""),
            placeholder: &resolved,
            id: "shell",
            read_only: false,
        },
    );
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
        "Restore saved sessions on normal launches",
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
        "An update or a crash leaves them running",
        config.durable_sessions,
        "durable_sessions",
    );

    form.group("Hotkey window");
    let y = form.y;
    form.text_row(
        y,
        &TextRow {
            label: "Global hotkey",
            desc: "Shows and hides Rift from any app, e.g. alt+space",
            value: &config.global_hotkey,
            placeholder: "Off",
            id: "global_hotkey",
            read_only: false,
        },
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Drop-down hotkey window",
        "The global hotkey slides a window down from the top of the screen",
        config.hotkey_window,
        "hotkey_window",
    );
    let y = form.y;
    form.stepper_desc(
        y,
        "Hotkey window height",
        "Share of the screen it covers",
        &config.hotkey_window_height().to_string(),
        "%",
        "hotkey_window_height",
    );
    let y = form.y;
    form.segment(
        y,
        "Screen edge",
        &["Top", "Bottom", "Left", "Right"],
        match config.hotkey_window_edge {
            rift_core::HotkeyEdge::Top => 0,
            rift_core::HotkeyEdge::Bottom => 1,
            rift_core::HotkeyEdge::Left => 2,
            rift_core::HotkeyEdge::Right => 3,
        },
        "hotkey_window_edge",
    );
    let y = form.y;
    form.stepper_desc(
        y,
        "Hotkey window width",
        "Share of the edge it covers, centred",
        &config.hotkey_window_width().to_string(),
        "%",
        "hotkey_window_width",
    );
    let y = form.y;
    form.text_row(
        y,
        &TextRow {
            label: "Screen",
            desc: "pointer (the one with the mouse), primary, or a screen's number",
            value: &config.hotkey_window_screen,
            placeholder: "pointer",
            id: "hotkey_window_screen",
            read_only: false,
        },
    );
    let y = form.y;
    form.toggle(
        y,
        "Hide it when it loses focus",
        config.hotkey_window_hide_on_blur,
        "hotkey_window_hide_on_blur",
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
    let y = form.y;
    form.toggle_desc(
        y,
        if cfg!(target_os = "macos") {
            "⌘-scroll zooms the font"
        } else {
            "Ctrl-scroll zooms the font"
        },
        "With ⇧ it changes the window opacity",
        config.wheel_zoom,
        "wheel_zoom",
    );
    let y = form.y;
    form.text_row(
        y,
        &TextRow {
            label: "Word characters",
            desc: "Extra characters a double-click keeps in a word, e.g. -./",
            value: &config.word_chars,
            placeholder: "None",
            id: "word_chars",
            read_only: false,
        },
    );
    let y = form.y;
    form.text_row(
        y,
        &TextRow {
            label: "Word delimiters",
            desc: "Characters that also end a word, for double-click and ⌥← ⌥→",
            value: &config.word_delimiters,
            placeholder: "None",
            id: "word_delimiters",
            read_only: false,
        },
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
    let y = form.y;
    form.stepper_desc(
        y,
        "Ask before pasting more than",
        "Off never asks",
        &if config.large_paste_warning == 0 {
            "Off".to_string()
        } else {
            thousands(config.large_paste_warning as usize)
        },
        if config.large_paste_warning == 0 {
            ""
        } else {
            "KiB"
        },
        "large_paste_warning",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Trim trailing blanks from pastes",
        "A copied line then no longer runs when pasted",
        config.trim_paste,
        "trim_paste",
    );

    form.group("Panes");
    let y = form.y;
    form.segment_desc(
        y,
        "Scrollbar",
        "Auto: while scrolling or hovered",
        &["Auto", "Always", "Never"],
        match config.scrollbar {
            rift_core::ScrollbarMode::Auto => 0,
            rift_core::ScrollbarMode::Always => 1,
            rift_core::ScrollbarMode::Never => 2,
        },
        "scrollbar",
    );
    let y = form.y;
    form.segment_desc(
        y,
        "When its program exits",
        "Keep: its output and Restart",
        &["Close", "Close on success", "Keep"],
        match config.close_on_exit {
            rift_core::CloseOnExit::Always => 0,
            rift_core::CloseOnExit::Graceful => 1,
            rift_core::CloseOnExit::Never => 2,
        },
        "close_on_exit",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Mask secrets",
        "Tokens and keys show as ••••",
        config.mask_secrets,
        "mask_secrets",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Show programs' progress",
        "A bar under the tab (OSC 9;4)",
        config.show_progress,
        "show_progress",
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
    form.segment_desc(
        y,
        "Vim's unnamed register",
        "Where y and p go",
        &["Rift only", "System clipboard"],
        usize::from(config.vim_clipboard.trim() == "unnamed"),
        "vim_clipboard",
    );
    let y = form.y;
    form.segment_desc(
        y,
        "⌃R, ⌃T and ⌥C run",
        "fzf or atuin, where the shell binds them",
        &["The shell's", "Rift's"],
        usize::from(config.shell_widgets == rift_core::ShellWidgets::Rift),
        "shell_widgets",
    );
    let y = form.y;
    form.toggle(
        y,
        "Autosuggestions (ghost text)",
        config.autosuggestions,
        "autosuggestions",
    );
    let y = form.y;
    form.segment_desc(
        y,
        "Tab key",
        "ghost text goes word by word; ⌃Space completes",
        &["Completions", "Ghost text"],
        usize::from(config.tab_behavior == rift_core::TabBehavior::Autosuggestion),
        "tab_behavior",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Show the accept-key hint",
        "a chip after the ghost text; click it to change the key",
        config.autosuggestion_hint,
        "autosuggestion_hint",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Show the ignore button",
        "a ✕ after the ghost text: never suggest that command again",
        config.autosuggestion_ignore_button,
        "autosuggestion_ignore_button",
    );
    let y = form.y;
    form.toggle(
        y,
        "Open completions as you type",
        config.completions_open_while_typing,
        "completions_open_while_typing",
    );
    let y = form.y;
    form.toggle(
        y,
        "Completions from your shell",
        config.shell_completions,
        "shell_completions",
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
    form.toggle_desc(
        y,
        "Sticky command header",
        "Pins a long block's command to the top while you scroll its output",
        config.sticky_command_header,
        "sticky_command_header",
    );
    let y = form.y;
    form.segment_desc(
        y,
        "Block spacing",
        "Normal leaves a blank row above each block",
        &["Compact", "Normal"],
        match config.block_spacing {
            BlockSpacing::Compact => 0,
            BlockSpacing::Normal => 1,
        },
        "block_spacing",
    );
    let y = form.y;
    form.toggle(
        y,
        "Show block dividers",
        config.block_dividers,
        "block_dividers",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Jump to bottom of block button",
        "Over a block that runs past the bottom of the pane",
        config.jump_to_bottom_button,
        "jump_to_bottom_button",
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
    let y = form.y;
    form.toggle_desc(
        y,
        "Offer Rift in subshells",
        "docker exec … bash, poetry shell, and the commands below",
        config.subshell_offer,
        "subshell_offer",
    );
    let more = join_words(&config.subshell_commands);
    let y = form.y;
    form.text_row(
        y,
        &TextRow {
            label: "More subshell commands",
            desc: "Regular expressions, separated by spaces",
            value: &more,
            placeholder: "None",
            id: "subshell_commands",
            read_only: false,
        },
    );
    let never = join_words(&config.subshell_denylist);
    let y = form.y;
    form.text_row(
        y,
        &TextRow {
            label: "Never offer it for",
            desc: "Commands, separated by spaces (Don't ask again adds one)",
            value: &never,
            placeholder: "None",
            id: "subshell_denylist",
            read_only: false,
        },
    );
    let y = form.y;
    form.segment(
        y,
        "Rich input sends with",
        &["⏎", "⌘⏎"],
        match config.rich_input_submit {
            rift_core::RichInputSubmit::Enter => 0,
            rift_core::RichInputSubmit::CmdEnter => 1,
        },
        "rich_input_submit",
    );
    let y = form.y;
    form.toggle_desc(
        y,
        "Open rich input when an agent waits",
        "Claude Code, Codex, OpenCode, Gemini CLI at their prompt",
        config.rich_input_auto,
        "rich_input_auto",
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
    let y = form.y;
    form.toggle_desc(
        y,
        if cfg!(target_os = "macos") {
            "Bounce the Dock icon"
        } else {
            "Mark the window urgent"
        },
        "For a bell or alert while Rift is in the back",
        config.request_attention,
        "request_attention",
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

/// Settings › Themes: an All / Dark / Light filter, then a grid of cards,
/// each a small Rift window in that theme's own colours (tab strip, a
/// passing and a failing block, the prompt and the cursor); the active one
/// ringed in the accent with a check. A click applies it. Last, a card for
/// adding your own (Open themes folder).
fn themes_page(form: &mut Form, config: &Config, filter: ThemeFilter) {
    let names = rift_core::theme::available_themes();
    let labels = rift_core::theme::picker_labels(&names);
    let themes: Vec<(Result<Theme, String>, &String)> = names
        .iter()
        .map(|n| (rift_core::theme::try_load_by_name(n), n))
        .collect();
    let lights: Vec<Option<bool>> = themes
        .iter()
        .map(|(t, _)| t.as_ref().ok().map(Theme::is_light))
        .collect();
    let theme = form.theme;

    // the filter, and which theme is active, on one line
    let y = form.y;
    form.hidden = false;
    form.row_label = "Show".into();
    let options: Vec<String> = ThemeFilter::ALL
        .iter()
        .map(|f| format!("{}  {}", f.label(), f.count(&lights)))
        .collect();
    let opts: Vec<&str> = options.iter().map(String::as_str).collect();
    let targets: Vec<ClickTarget> = (0..ThemeFilter::ALL.len())
        .map(|i| ClickTarget::SettingsThemeFilter(i as u8))
        .collect();
    let current = ThemeFilter::ALL
        .iter()
        .position(|f| *f == filter)
        .unwrap_or(0);
    form.segmented(form.x, y + 4.0, "Show", &opts, current, &targets);
    let active = labels
        .iter()
        .zip(&names)
        .find(|(_, n)| **n == config.theme)
        .map_or(config.theme.as_str(), |(l, _)| l.as_str());
    let text = format!("Active: {active}");
    let tw = form.engine.measure(&text, 12.5, FONT_SANS, Weight::NORMAL);
    if form.visible(y, ROW_H) {
        form.layer.text(
            sans(form.engine, &text, 12.5, Weight::NORMAL),
            form.right() - tw,
            y + 4.0 + (CONTROL_H - 12.5 * 1.35) / 2.0,
            theme.text_dim,
        );
    }
    form.y = y + CONTROL_H + 18.0;

    let cols = theme_columns(form.card_w);
    let cw = ((form.card_w - (cols - 1) as f32 * THEME_GAP) / cols as f32).floor();
    let ch = THEME_CARD_H;
    let shown: Vec<usize> = (0..names.len())
        .filter(|i| filter.shows(lights[*i]))
        .collect();
    let top = form.y;
    let cells = shown.len() + 1;
    for cell in 0..cells {
        let (row, col) = (cell / cols, cell % cols);
        let r = [
            form.x + col as f32 * (cw + THEME_GAP),
            top + row as f32 * (ch + THEME_GAP),
            cw,
            ch,
        ];
        let Some(&i) = shown.get(cell) else {
            add_theme_card(form, r);
            continue;
        };
        let (loaded, name) = &themes[i];
        let is_active = config.theme == **name;
        let light_tag = config.sync_os_theme && config.theme_light == **name && !is_active;
        let tag = if is_active {
            Some("active")
        } else if light_tag {
            Some("light")
        } else {
            None
        };
        form.row_label = match tag {
            Some(tag) => format!("{}, {tag}", labels[i]),
            None => labels[i].clone(),
        };
        if form.visible_band(r[1], r[3]) {
            // a file that does not load: its name (the card says the rest)
            let shown = if loaded.is_ok() {
                labels[i].as_str()
            } else {
                name.as_str()
            };
            theme_card(
                form,
                r,
                loaded.as_ref().ok(),
                shown,
                lights[i] == Some(true),
                is_active,
                light_tag,
            );
        }
        form.push_hit(r, ClickTarget::SettingsTheme(i as u8));
    }
    let rows = cells.div_ceil(cols);
    form.y = top + rows as f32 * (ch + THEME_GAP) - THEME_GAP;
}

/// Settings search over the themes: a row per theme whose name matches,
/// which applies it like its card.
fn themes_search(form: &mut Form, config: &Config) {
    form.group("Themes");
    let names = rift_core::theme::available_themes();
    let labels = rift_core::theme::picker_labels(&names);
    for (i, (name, label)) in names.iter().zip(&labels).enumerate() {
        let y = form.y;
        let active = config.theme == *name;
        form.reserve = if active { 90.0 } else { 0.0 };
        form.label_desc(label, "Theme", y);
        if form.hidden {
            continue;
        }
        if active && form.visible(y, ROW_H) {
            let tag = "active";
            let aw = form.engine.measure(tag, 11.5, FONT_SANS, Weight::MEDIUM);
            form.layer.rect(
                [form.right() - aw - 18.0, y + 15.0],
                [aw + 18.0, 22.0],
                form.theme.accent.with_alpha(0.16),
                4.0,
            );
            form.layer.text(
                sans(form.engine, tag, 11.5, Weight::MEDIUM),
                form.right() - aw - 9.0,
                y + 18.5,
                form.theme.accent,
            );
        }
        form.row_label = if active {
            format!("{label} theme, active")
        } else {
            format!("{label} theme")
        };
        form.push_hit(
            [form.x, y, form.card_w, ROW_H],
            ClickTarget::SettingsTheme(i as u8),
        );
    }
}

/// Height of a theme card: the small window and the name under it.
pub const THEME_CARD_H: f32 = 8.0 + THEME_MINI_H + 10.0 + 18.0 + 10.0;
/// Height of the small window in a theme card.
const THEME_MINI_H: f32 = 104.0;

/// One theme's card at `r`: the small window drawn in `t`'s tokens (an
/// invalid file draws an empty ground with the error colour), its name, a
/// Dark / Light tag, and the active ring and check.
fn theme_card(
    form: &mut Form,
    r: [f32; 4],
    t: Option<&Theme>,
    label: &str,
    light: bool,
    active: bool,
    light_tag: bool,
) {
    let ui = form.theme;
    let hot = form.hot(r);
    let edge = if active {
        ui.accent
    } else if hot {
        lift(ui, 0.28)
    } else {
        lift(ui, 0.1)
    };
    form.layer.bordered(
        [r[0], r[1]],
        [r[2], r[3]],
        lift(ui, 0.02),
        edge,
        8.0,
        if active { 2.0 } else { 1.0 },
    );
    let m = [r[0] + 8.0, r[1] + 8.0, r[2] - 16.0, THEME_MINI_H];
    match t {
        Some(t) => mini_window(form, m, t),
        None => {
            form.layer.bordered(
                [m[0], m[1]],
                [m[2], m[3]],
                ui.ground(ui.bg),
                ui.error.with_alpha(0.6),
                5.0,
                1.0,
            );
            let text = "This file does not load";
            let w = form.engine.measure(text, 12.0, FONT_SANS, Weight::NORMAL);
            form.layer.text(
                sans(form.engine, text, 12.0, Weight::NORMAL),
                m[0] + (m[2] - w) / 2.0,
                m[1] + m[3] / 2.0 - 8.0,
                ui.error,
            );
        }
    }
    // the name, its kind and (active) a check
    let ny = m[1] + m[3] + 10.0;
    let kind = if t.is_none() {
        "Invalid"
    } else if light {
        "Light"
    } else {
        "Dark"
    };
    let kw = form.engine.measure(kind, 10.5, FONT_SANS, Weight::MEDIUM) + 12.0;
    let check_w = if active { 22.0 } else { 0.0 };
    let tag_w = if light_tag {
        form.engine
            .measure("light mode", 10.5, FONT_SANS, Weight::MEDIUM)
            + 12.0
            + 6.0
    } else {
        0.0
    };
    let name_w = (r[2] - 20.0 - kw - 8.0 - check_w - tag_w).max(30.0);
    let shown = super::palette::elide_to(form.engine, label, 13.0, FONT_SANS, name_w);
    form.layer.text(
        sans(form.engine, &shown, 13.0, Weight::MEDIUM),
        r[0] + 10.0,
        ny,
        ui.text_ui,
    );
    let mut x = r[0] + r[2] - 10.0 - check_w;
    if active {
        form.layer
            .circle([x + 8.0 + 4.0, ny + 9.0], 16.0, ui.accent);
        icons::draw(form.layer, Icon::Check, [x + 12.0, ny + 9.0], 11.0, ui.bg);
    }
    x -= kw;
    form.layer
        .rect([x, ny + 1.0], [kw, 17.0], lift(ui, 0.07), 3.0);
    form.layer.text(
        sans(form.engine, kind, 10.5, Weight::MEDIUM),
        x + 6.0,
        ny + 2.5,
        ui.text_dim,
    );
    if light_tag {
        let w = tag_w - 6.0;
        x -= tag_w;
        form.layer
            .rect([x, ny + 1.0], [w, 17.0], ui.accent.with_alpha(0.16), 3.0);
        form.layer.text(
            sans(form.engine, "light mode", 10.5, Weight::MEDIUM),
            x + 6.0,
            ny + 2.5,
            ui.accent,
        );
    }
}

/// A small Rift window in `t`'s colours at `m`: the chrome strip with its
/// three dots, an active tab (accent edge) and an inactive one; a passing
/// block (success edge, accent command), a failing one, and the prompt with
/// its chevron and cursor.
fn mini_window(form: &mut Form, m: [f32; 4], t: &Theme) {
    let ground = t.bg.with_alpha(1.0);
    let chrome = t.bg_chrome.with_alpha(1.0);
    form.layer
        .bordered([m[0], m[1]], [m[2], m[3]], ground, t.border, 5.0, 1.0);
    // the strip
    let sh = 18.0;
    form.layer
        .rect([m[0] + 1.0, m[1] + 1.0], [m[2] - 2.0, sh], chrome, 4.0);
    form.layer
        .rect([m[0] + 1.0, m[1] + sh], [m[2] - 2.0, 1.0], t.border, 0.0);
    for i in 0..3 {
        form.layer.circle(
            [m[0] + 9.0 + i as f32 * 7.0, m[1] + 1.0 + sh / 2.0],
            4.5,
            t.text_muted.with_alpha(0.7),
        );
    }
    let tab_w = ((m[2] - 34.0) / 2.6).min(64.0);
    let tx = m[0] + 31.0;
    form.layer.rect([tx, m[1] + 1.0], [tab_w, sh], ground, 0.0);
    form.layer
        .rect([tx, m[1] + 1.0], [tab_w, 1.5], t.accent, 0.0);
    form.layer
        .rect([tx + tab_w, m[1] + 1.0], [1.0, sh], t.border, 0.0);
    let label = |form: &mut Form, text: &str, x: f32, w: f32, color: Color| {
        let tw = form.engine.measure(text, 7.5, FONT_SANS, Weight::NORMAL);
        form.layer.text(
            sans(form.engine, text, 7.5, Weight::NORMAL),
            x + (w - tw) / 2.0,
            m[1] + 1.0 + (sh - 7.5 * 1.35) / 2.0,
            color,
        );
    };
    label(form, "project1", tx, tab_w, t.text_ui);
    label(form, "~", tx + tab_w, tab_w, t.text_dim);
    // the blocks and the prompt, monospace
    let x = m[0] + 9.0;
    let mut y = m[1] + sh + 8.0;
    let lh = 11.0;
    let line = |form: &mut Form, text: &str, x: f32, y: f32, color: Color| {
        form.layer
            .text(mono(form.engine, text, 8.0, Weight::NORMAL), x, y, color);
    };
    form.layer
        .rect([x, y], [2.0, 3.0 * lh - 2.0], t.success, 0.0);
    line(form, "cargo test", x + 7.0, y, t.accent);
    line(form, "running 42 tests", x + 7.0, y + lh, t.fg);
    line(form, "test result: ok", x + 7.0, y + 2.0 * lh, t.success);
    y += 3.0 * lh + 5.0;
    form.layer.rect([x, y], [2.0, 2.0 * lh - 2.0], t.error, 0.0);
    line(form, "git push", x + 7.0, y, t.accent);
    line(form, "rejected: fetch first", x + 7.0, y + lh, t.error);
    y += 2.0 * lh + 5.0;
    line(form, "❯", x, y, t.accent);
    line(form, "npm run", x + 10.0, y, t.text_dim);
    let cw = form
        .engine
        .measure("npm run", 8.0, mono_family(), Weight::NORMAL);
    form.layer
        .rect([x + 10.0 + cw + 2.0, y + 1.0], [4.5, 9.0], t.cursor, 0.0);
}

/// The last card of the grid: how to add a theme of your own, and a button
/// that opens the themes folder (made when missing).
fn add_theme_card(form: &mut Form, r: [f32; 4]) {
    let ui = form.theme;
    let label = "Open themes folder";
    let bw = (form.engine.measure(label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0).round();
    let lines = wrap_lines(
        form.engine,
        "Drop a .toml or a Warp .yaml theme into the folder; it shows here at once.",
        12.0,
        FONT_SANS,
        r[2] - 28.0,
    );
    let body_h = 22.0 + 20.0 + lines.len() as f32 * NOTE_LH + 10.0 + CONTROL_H;
    let mut y = r[1] + ((r[3] - body_h) / 2.0).max(10.0);
    if form.visible_band(r[1], r[3]) {
        super::arranger::dashed(form.layer, r, lift(ui, 0.22), 1.5);
        icons::draw(
            form.layer,
            Icon::Plus,
            [r[0] + r[2] / 2.0, y + 11.0],
            18.0,
            ui.text_muted,
        );
        y += 22.0;
        let t = "Add your own";
        let tw = form.engine.measure(t, 13.0, FONT_SANS, Weight::NORMAL);
        form.layer.text(
            sans(form.engine, t, 13.0, Weight::NORMAL),
            r[0] + (r[2] - tw) / 2.0,
            y + 2.0,
            ui.text_ui,
        );
        y += 20.0;
        for l in &lines {
            let lw = form.engine.measure(l, 12.0, FONT_SANS, Weight::NORMAL);
            form.layer.text(
                sans(form.engine, l, 12.0, Weight::NORMAL),
                r[0] + (r[2] - lw) / 2.0,
                y,
                ui.text_muted,
            );
            y += NOTE_LH;
        }
        y += 10.0;
    } else {
        y += 42.0 + lines.len() as f32 * NOTE_LH + 10.0;
    }
    form.row_label = "Add your own theme".into();
    form.button(
        (r[0] + (r[2] - bw) / 2.0).round(),
        y.round(),
        label,
        ButtonKind::Secondary,
        ClickTarget::SettingsOpenThemes,
        "Open themes folder: drop a .toml or a Warp .yaml theme into it".into(),
    );
}

/// `window_opacity` (0.01–1.0) as a slider position.
pub fn opacity_slider(opacity: f32) -> f32 {
    (opacity - 0.01) / 0.99
}

/// `window_blur` (1–64 px) as a slider position.
pub fn blur_slider(blur: u8) -> f32 {
    (f32::from(blur) - 1.0) / 63.0
}

/// `inactive_pane_dim` (1–40 %) as a slider position.
pub fn dim_slider(dim: u8) -> f32 {
    (f32::from(dim.clamp(1, 40)) - 1.0) / 39.0
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
        // per-side padding shows on the "Window padding" stepper: changed,
        // and its reset clears the sides (top and bottom stay)
        let c = rift_core::Config {
            window_padding_left: Some(30.0),
            window_padding_top: Some(20.0),
            ..Default::default()
        };
        assert!(is_changed(&config_table(&c), &d, "window_padding"));
        let c = reset_setting(&c, "window_padding").expect("loads");
        assert_eq!(
            (c.window_padding_left, c.window_padding_top),
            (None, Some(20.0))
        );
        assert!(!is_changed(&config_table(&c), &d, "window_padding"));
    }

    /// config.toml keys of `config` (dotted inside a table), nested tables
    /// walked, lists of tables kept whole.
    fn config_keys(table: &toml::Table, prefix: &str, out: &mut Vec<String>) {
        for (k, v) in table {
            let id = format!("{prefix}{k}");
            match v {
                // (an empty one is a map: `keybindings`)
                toml::Value::Table(t) if !t.is_empty() => config_keys(t, &format!("{id}."), out),
                _ => out.push(id),
            }
        }
    }

    #[test]
    fn every_config_key_has_a_settings_row() {
        // Keys not represented in scalar ROWS have dedicated collection editors.
        // Internal metadata (profiles.source) and derived state (profiles.orphaned)
        // must not become editable controls; profiles.hidden is user intent, not
        // internal metadata. See docs/configuration.md, Settings UI exposure policy.
        const ELSEWHERE: &[(&str, &str)] = &[
            // the Profiles page lists, edits and orders them
            ("profiles", "Settings › Profiles"),
            // the Keybindings page records and resets them
            ("keybindings", "Settings › Keybindings"),
            // Structured lists have draft editors, rather than scalar rows.
            ("ssh_profiles", "Settings › Profiles › SSH profiles"),
            ("actions", "Settings › Actions"),
            ("secret_patterns", "Settings › Terminal › Secret patterns"),
            ("tab_color_rules", "Settings › Appearance › Tab color rules"),
        ];
        let mut keys = Vec::new();
        config_keys(&config_table(&rift_core::Config::default()), "", &mut keys);
        // unset by default, so absent from the default's table
        keys.extend(
            rift_core::config::OPTIONAL_KEYS
                .iter()
                .map(|k| k.to_string()),
        );
        let missing: Vec<&String> = keys
            .iter()
            .filter(|k| !super::ROWS.contains(&k.as_str()))
            .filter(|k| !ELSEWHERE.iter().any(|(e, _)| e == k))
            .collect();
        assert!(
            missing.is_empty(),
            "config.toml keys without a Settings row: {missing:?} (add a row and \
             list it in settings::ROWS, or give a reason in ELSEWHERE)"
        );
        // and every row names a real key (an optional one, or the default's)
        for row in super::ROWS.iter().chain(ELSEWHERE.iter().map(|(k, _)| k)) {
            assert!(keys.iter().any(|k| k == row), "{row} is not a config key");
        }
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
    fn the_remote_page_shows_what_its_state_needs() {
        use super::{PairStage, PairingView, RelayLine, pair_stage, remote_heading};
        use std::time::{Duration, SystemTime};
        let now = SystemTime::now();
        let link = |expires_at| PairingView {
            url: "rift-remote://pair?s=x".into(),
            qr: Vec::new(),
            expires_at,
        };
        let live = link(now + Duration::from_secs(90));
        let old = link(now - Duration::from_secs(1));
        // off: nothing to pair, whatever is left over
        assert_eq!(pair_stage(false, true, Some(&live), now), PairStage::Hidden);
        assert_eq!(pair_stage(true, false, None, now), PairStage::Idle);
        assert_eq!(pair_stage(true, true, None, now), PairStage::Waiting);
        assert_eq!(
            pair_stage(true, false, Some(&live), now),
            PairStage::Showing
        );
        assert_eq!(pair_stage(true, false, Some(&old), now), PairStage::Expired);
        assert_eq!(
            remote_heading(false, &GatewayLine::Off).0,
            "Remote access is off"
        );
        assert_eq!(
            remote_heading(true, &GatewayLine::Starting).0,
            "Starting remote access…"
        );
        assert_eq!(
            remote_heading(true, &GatewayLine::Listening(vec!["a".into()])).0,
            "Ready for your phone"
        );
        assert_eq!(
            remote_heading(true, &GatewayLine::Listening(vec![])).0,
            "Not on a network"
        );
        assert_eq!(
            remote_heading(true, &GatewayLine::Connected(2)).0,
            "2 phones connected"
        );
        // a relay that is set but stopped never reads "No relay"
        assert_eq!(RelayLine::Stopped.text(), "Off while remote access is off");
        assert!(RelayLine::Off.text().starts_with("No relay set"));
        assert_eq!(RelayLine::Stopped.host(), None);
    }

    #[test]
    fn the_theme_grid_filters_counts_and_walks() {
        use super::{GridStep, ThemeFilter, grid_step, theme_columns};
        // 2 dark, 1 light, 1 file that does not load
        let kinds = [Some(false), Some(true), Some(false), None];
        assert_eq!(ThemeFilter::All.count(&kinds), 4);
        assert_eq!(ThemeFilter::Dark.count(&kinds), 2);
        assert_eq!(ThemeFilter::Light.count(&kinds), 1);
        assert!(!ThemeFilter::Dark.shows(None) && ThemeFilter::All.shows(None));
        assert_eq!(theme_columns(720.0), 3);
        assert_eq!(theme_columns(420.0), 2);
        assert_eq!(theme_columns(150.0), 1);
        // three per row: 0 1 2 / 3 4
        let cards: Vec<(ClickTarget, [f32; 4])> = (0..5u8)
            .map(|i| {
                let (r, c) = (f32::from(i / 3), f32::from(i % 3));
                (
                    ClickTarget::SettingsTheme(i),
                    [c * 200.0, r * 150.0, 190.0, 140.0],
                )
            })
            .collect();
        let at = |i: u8| ClickTarget::SettingsTheme(i);
        let go = |from: u8, step| grid_step(&cards, Some(&at(from)), step);
        assert_eq!(go(1, GridStep::Down), Some(at(4)));
        assert_eq!(go(4, GridStep::Up), Some(at(1)));
        // the row below is short: the closest column
        assert_eq!(go(2, GridStep::Down), Some(at(4)));
        assert_eq!(go(2, GridStep::Right), Some(at(3)));
        assert_eq!(go(3, GridStep::Left), Some(at(2)));
        assert_eq!(go(0, GridStep::Up), None);
        assert_eq!(go(0, GridStep::Left), None);
        assert_eq!(go(4, GridStep::Right), None);
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
