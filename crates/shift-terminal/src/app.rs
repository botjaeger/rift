use std::sync::Arc;
use std::time::{Duration, Instant};

use shift_core::git::{GitInfo, GitWorker};
use shift_core::{BlockState, Config, Keymap, Theme};
use shift_pty::{GridSize, PaneId, Session, SessionEvent, SpawnOptions};
use shift_renderer::glyphon::{TextArea, TextBounds};
use shift_renderer::{Gpu, QuadPipeline, Scene, TextEngine};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowAttributes, WindowId};

use shift_pty::alacritty_terminal::index::{
    Column as ACol, Line as ALine, Point as APoint, Side as ASide,
};
use shift_pty::alacritty_terminal::selection::{Selection, SelectionType};

use crate::input::{self, AppAction, EditorKey};
use crate::pane::{Axis, HandleRect, NodePath, PaneRect, VIEWER_PANE, focus_neighbor};
use crate::state::{ClickTarget, Hit, PaneState, TabState};
use crate::views::completion as completion_view;
use crate::views::completion::CompletionUi;
use crate::views::context_menu::{self, ContextMenuState, MenuAction, MenuItem};
use crate::views::editor::{self, EDITOR_H};
use crate::views::explorer::{ExplorerHit, ExplorerState};
use crate::views::notifications::{Kind as AttentionKind, NotificationsHits, NotificationsState};
use crate::views::palette::{self, MatchedItem, PaletteAction, PaletteState};
use crate::views::review_panel::{ReviewHit, ReviewPanelState};
use crate::views::search::{self, SearchState};
use crate::views::settings::{self, SettingsState};
use crate::views::tabbar::{self, TAB_BAR_H};
use crate::views::terminal::{self, GridLayout};
use crate::views::viewer::{ViewerHit, ViewerState};
use crate::views::whats_new::{WhatsNewHits, WhatsNewState};

/// Cross-thread wakeups.
#[derive(Debug)]
pub enum UserEvent {
    PtyWakeup(PaneId),
    PtyTitle(PaneId, String),
    PtyExit(PaneId),
    PtyBell(PaneId),
    /// OSC 52 copy from the program in the pane.
    PtyClipboard(String),
    /// OSC 52 read from the program in the pane (`clipboard_access =
    /// "read_write"` only).
    PtyClipboardLoad(PaneId, shift_pty::ClipboardReply),
    /// OSC 4/10/11/12 color query to answer from the theme.
    PtyColorRequest(PaneId, usize, shift_pty::ColorReply),
    PtyWindowOp(PaneId, Vec<Option<u16>>),
    /// The system-wide `global_hotkey` was pressed.
    GlobalHotkey,
    /// A native menu item was chosen (its id).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Menu(String),
    GitStatus(PaneId, Option<GitInfo>),
    PrStatus(PaneId, Option<shift_core::github::PrInfo>),
    DockerStatus(PaneId, Option<shift_core::docker::ComposeInfo>),
    ConfigReloaded,
    ViewerLoaded(Box<crate::views::viewer::ViewerState>),
    WindowsLoaded(
        Vec<(u32, std::path::PathBuf, Vec<crate::control::TabInfo>)>,
        bool,
    ),
    DetachedLoaded(Vec<shift_pty::ipc::SessionInfo>),
    HistoryLoaded(Vec<String>),
    ShellEnvLoaded(Arc<shift_core::shell_env::ShellEnv>),
    /// Screen reader requests (initial tree, actions).
    Access(accesskit_winit::Event),
    /// A local control API request (see control.rs).
    Control(crate::control::Pending),
    /// A program asked for a notification (OSC 9 / 777 / 99).
    PtyNotify(PaneId, String, String),
    /// A pane's connection notice (see `SessionEvent::Notice`).
    PtyNotice(String),
    /// Code review model for request `.0`; `None` outside a repository.
    ReviewLoaded(u64, Option<Result<shift_core::review::Review, String>>),
    /// A review-panel write finished (`Some` = why it failed).
    ReviewWritten(Option<String>),
    /// Completions for completion request `.0`.
    Completions(u64, Vec<shift_core::completion::Completion>),
    /// An update check finished (`true`: asked for, so its answer toasts).
    UpdateChecked(Result<shift_core::update::Manifest, String>, bool),
    /// The other windows an update would restart.
    UpdateSurvey(crate::updater::Survey),
    /// The update is downloaded and installed (or why not); the windows
    /// to restart with this one.
    UpdateInstalled(
        Result<shift_core::update::Pending, String>,
        Vec<(u32, std::path::PathBuf)>,
    ),
    /// The other windows saved their layouts and quit: relaunch (this
    /// window's restart file).
    UpdateReady(std::path::PathBuf),
    /// This process has been up long enough (`App::confirm_start`).
    StartConfirmed,
}

/// What the completion thread completes: one editor state and the
/// histories and directory it is completed against.
#[derive(Clone)]
struct CompletionRequest {
    generation: u64,
    pane: PaneId,
    buffer: String,
    cursor: usize,
    history: Vec<String>,
    shell_history: Arc<Vec<String>>,
    cwd: Option<String>,
}

impl CompletionRequest {
    fn run(&self) -> Vec<shift_core::completion::Completion> {
        // the whole word around the cursor is completed (zsh without
        // `complete_in_word`); `accept_completion` replaces that same span
        shift_core::completion::complete(
            &self.buffer,
            shift_core::completion::token_end(&self.buffer, self.cursor),
            &self.history,
            &self.shell_history,
            self.cwd.as_deref(),
        )
    }
}

/// Pause before starting a new completion thread after one failed.
const COMPLETION_RETRY: Duration = Duration::from_secs(5);

/// Queue of the completion thread, started on the first request. Providers
/// read the filesystem and Git, so they never run on the UI thread: when the
/// thread cannot start or has died, completion pauses for
/// `COMPLETION_RETRY` and the next request after that starts a new one.
struct CompletionWorker<T> {
    tx: Option<std::sync::mpsc::Sender<T>>,
    retry_at: Option<Instant>,
}

impl<T> Default for CompletionWorker<T> {
    fn default() -> Self {
        CompletionWorker {
            tx: None,
            retry_at: None,
        }
    }
}

impl<T> CompletionWorker<T> {
    /// Hand `req` to the thread, starting it with `spawn` when there is
    /// none. False while paused; the caller must not answer it itself.
    fn send(
        &mut self,
        req: T,
        now: Instant,
        spawn: impl FnOnce() -> Option<std::sync::mpsc::Sender<T>>,
    ) -> bool {
        if self.tx.is_none() {
            if self.retry_at.is_some_and(|t| now < t) {
                return false;
            }
            self.tx = spawn();
        }
        if self.tx.as_ref().is_some_and(|tx| tx.send(req).is_ok()) {
            self.retry_at = None;
            return true;
        }
        self.tx = None;
        self.retry_at = Some(now + COMPLETION_RETRY);
        false
    }
}

/// A completion key deferred until the pending request is answered.
#[derive(Clone, Copy, Debug, PartialEq)]
enum CompletionIntent {
    /// Tab
    Accept,
    /// ⌃→
    AcceptGhostWord,
}

/// Review-panel writes, applied in order: (operation, path, hunk).
type ReviewOps = Vec<(
    shift_core::review::Op,
    String,
    Option<shift_core::review::HunkId>,
)>;

/// One queued review-panel write: the repository and its operations.
type ReviewWrite = (std::path::PathBuf, ReviewOps);

/// One entry of the attention center (⇧⌘A, the notifications popover):
/// something that happened in a pane while you were looking elsewhere, or
/// that a program flagged.
#[derive(Debug, Clone)]
struct Attention {
    /// Stable id (list indices shift when the cap drops old entries).
    id: u64,
    at: std::time::SystemTime,
    pane: PaneId,
    block: Option<shift_core::BlockId>,
    kind: AttentionKind,
    /// Coding agent running in the pane (Claude Code, Codex, …).
    agent: Option<shift_core::notifications::Agent>,
    /// The pane's git branch when it happened.
    branch: Option<String>,
    /// Command or notification title.
    title: String,
    /// Status, duration or notification body.
    detail: String,
    read: bool,
}

impl Attention {
    /// A new unread entry; `push_attention` fills id, agent and branch.
    fn new(pane: PaneId, kind: AttentionKind, title: String, detail: String) -> Self {
        Attention {
            id: 0,
            at: std::time::SystemTime::now(),
            pane,
            block: None,
            kind,
            agent: None,
            branch: None,
            title,
            detail,
            read: false,
        }
    }

    /// Title and body of its desktop notification.
    fn desktop_text(&self) -> (String, String) {
        let joined = |a: &str, b: &str| {
            if b.is_empty() {
                a.to_string()
            } else {
                format!("{a} · {b}")
            }
        };
        match (self.agent, self.kind) {
            (Some(agent), _) => (agent.name().to_string(), joined(&self.title, &self.detail)),
            (None, AttentionKind::Finished) => {
                ("Command finished".into(), joined(&self.title, &self.detail))
            }
            (None, AttentionKind::Failed) => {
                ("Command failed".into(), joined(&self.title, &self.detail))
            }
            (None, AttentionKind::Request) => ("Needs your input".into(), self.title.clone()),
            (None, _) => (self.title.clone(), self.detail.clone()),
        }
    }
}

const ATTENTION_CAP: usize = 200;

impl From<accesskit_winit::Event> for UserEvent {
    fn from(e: accesskit_winit::Event) -> Self {
        UserEvent::Access(e)
    }
}

/// Spec metrics (logical px).
const SIDE_PAD: f32 = 8.0;
const TOP_PAD: f32 = 14.0;
const DEFAULT_FONT_SIZE: f32 = 13.0;
/// Half-period of blinking text (SGR 5), as xterm's default.
const TEXT_BLINK: Duration = Duration::from_millis(600);
/// Shortest gap between frames that program output or the running-command
/// spinner asks for (120 Hz). Each frame locks the terminal the reader thread
/// parses into; unpaced, the spinner redrew back to back and halved output
/// throughput.
const OUTPUT_FRAME: Duration = Duration::from_micros(8333);
const BLINK_INTERVAL: Duration = Duration::from_millis(550);
const BELL_FLASH: Duration = Duration::from_millis(120);
/// How long the boot placeholder waits for a prompt after the first output.
const BOOT_GRACE: Duration = Duration::from_millis(1500);
/// Padding inside a split pane (design 02).
const PANE_PAD: f32 = SIDE_PAD;
/// Title bar of a split pane (Warp pane header): title, ⋮ menu, × close.
const PANE_HEADER_H: f32 = 28.0;
/// Pointer travel before a press on a pane header becomes a pane drag.
const PANE_DRAG_SLOP: f32 = 4.0;

/// A split pane being dragged by its title bar.
struct PaneDrag {
    pane: PaneId,
    /// Tab the pane lives in.
    from_tab: usize,
    press: (f32, f32),
    /// Past the slop: drop zones show, release moves the pane.
    live: bool,
    /// Drop target under the pointer (None: release leaves it in place).
    target: Option<PaneDrop>,
    /// Tab under the pointer and since when: held there for
    /// [`TAB_HOVER_OPEN`] it opens, so the pane can land among its panes.
    hover_tab: Option<(usize, Instant)>,
}

/// How long a dragged pane hovers over a tab before that tab opens.
const TAB_HOVER_OPEN: Duration = Duration::from_millis(600);

/// What picking a history row does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HistoryUse {
    Run,
    Edit,
    RunInSplit,
}

/// A tab being reordered by dragging it along the strip.
#[derive(Clone, Copy)]
struct TabDrag {
    /// Current index (the tab moves as it passes neighbours).
    index: usize,
    /// Index at the press, restored by Esc.
    origin: usize,
    /// Pointer coordinate at the last reorder.
    start: f32,
    /// Pointer coordinate at the press.
    press: f32,
    /// Pointer offset inside the tab at the press (the lifted tab keeps it).
    grab: f32,
    /// Past the slop: the tab lifts and follows the pointer.
    live: bool,
}

/// Where a dragged pane lands on release.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PaneDrop {
    /// Beside a pane of the shown tab, on that edge.
    Pane(PaneId, crate::pane::Edge),
    /// In place of a pane of the same tab (the two trade places).
    Swap(PaneId),
    /// Into another tab (split right of its focused pane).
    Tab(usize),
    /// Out into a tab of its own (the empty tab strip).
    NewTab,
}

struct DragState {
    path: NodePath,
    axis: Axis,
    /// Area of the split node being resized.
    area: [f32; 4],
}

/// Armed on left-press over terminal content; becomes a live selection once
/// the pointer moves past a small threshold.
struct PendingSelection {
    pane: PaneId,
    /// Anchor row as an absolute line: a grid `Line` would drift when output
    /// scrolls the screen before the drag starts.
    abs: i64,
    col: ACol,
    side: ASide,
    origin: (f32, f32),
    started: bool,
    /// ⌥-drag: rectangular (column) selection.
    block: bool,
}

const DOUBLE_CLICK_MS: u128 = 400;
const DRAG_THRESHOLD: f32 = 3.0;

pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    quads: Option<QuadPipeline>,
    /// Theme `background_image` (Warp-style), drawn under every layer.
    bg_image: Option<shift_renderer::ImagePipeline>,
    /// Inline images (kitty graphics / iTerm2) on the GPU.
    sprites: Option<shift_renderer::SpritePipeline>,
    bg_image_opacity: f32,
    text: Option<TextEngine>,
    git: Option<GitWorker>,
    pr_worker: Option<shift_core::github::PrWorker>,
    docker_worker: Option<shift_core::docker::DockerWorker>,
    /// The prompt chips the panes' git and Docker state was last fetched for.
    chips_applied: Vec<shift_core::PromptChip>,
    tabs: Vec<TabState>,
    active: usize,
    next_pane: PaneId,
    scene: Scene,
    theme: Theme,
    cell_w: f32,
    modifiers: ModifiersState,
    /// Left and right Option (Alt) held, where the platform tells them
    /// apart (macOS); both false otherwise.
    alt_sides: (bool, bool),
    /// Why the configured theme did not load (toasted once per change).
    theme_error: Option<String>,
    hits: Vec<Hit>,
    mouse: (f32, f32),
    any_running: bool,
    editor_active: bool,
    blink_epoch: Instant,
    /// Earliest end of a pane's boot-placeholder grace (repaint then).
    boot_until: Option<Instant>,
    drag: Option<DragState>,
    palette: PaletteState,
    palette_results: Vec<MatchedItem>,
    palette_row_hits: Vec<([f32; 4], usize)>,
    /// Accessible names of the settings controls drawn last frame.
    settings_names: Vec<([f32; 4], String)>,
    /// Settings controls Tab can reach, also scrolled away (page coords).
    settings_stops: Vec<(ClickTarget, [f32; 4])>,
    completion: CompletionUi,
    editor_anchor: (f32, f32),
    /// Soft-wrap columns of the active pane's editor (last frame).
    editor_cols: usize,
    /// Find-bar toggle chips: [match case, regex] (last frame).
    search_chips: Vec<[f32; 4]>,
    config: Config,
    keymap: Keymap,
    settings: SettingsState,
    slider_drag: Option<(&'static str, [f32; 4])>,
    search: SearchState,
    clipboard: Option<arboard::Clipboard>,
    pending_sel: Option<PendingSelection>,
    last_click: Option<(Instant, (f32, f32), u8)>,
    context_menu: ContextMenuState,
    context_menu_rects: Vec<[f32; 4]>,
    /// The prompt chips' directory / branch picker, its card and rows.
    chip_menu: Option<crate::views::chip_menu::ChipMenu>,
    chip_menu_card: [f32; 4],
    chip_menu_rows: Vec<([f32; 4], usize)>,
    /// "Edit prompt" (Settings › prompt chips) and its last layout.
    prompt_editor: Option<crate::views::prompt_editor::PromptEditor>,
    prompt_editor_layout: crate::views::prompt_editor::Layout,
    /// Imported shell history (shared with the completion thread).
    shell_history: Arc<Vec<String>>,
    font_size: f32,
    line_h: f32,
    scroll_accum: f32,
    /// cwds of recently closed tabs (⇧⌘T reopens, newest last).
    closed_tabs: Vec<Option<std::path::PathBuf>>,
    _watcher: Option<shift_core::watch::ConfigWatcher>,
    /// `--script` driver (see script.rs).
    script: Option<crate::script::Script>,
    /// `--headless`: the window is never shown, focused or raised, and no
    /// global hotkey is registered (scripts only; `shot` reads offscreen).
    headless: bool,
    /// Clipboard of a headless run: the system clipboard is never touched.
    headless_clipboard: String,
    /// Next frame is also rendered offscreen and written to this PNG.
    capture: Option<std::path::PathBuf>,
    /// OS appearance is light (drives `sync_os_theme`).
    os_light: bool,
    /// Window has keyboard focus (completion notifications fire only when it doesn't).
    focused: bool,
    /// Pane that was last told it has focus (DECSET 1004), if any.
    focus_pane: Option<PaneId>,
    /// Visual bell flash start.
    bell_flash: Option<Instant>,
    /// Warp-style launch configurations (~/.config/shift/launch_configs).
    launch_configs: Vec<shift_core::launch::LaunchConfig>,
    /// Commands a launch configuration queued for a pane; sent at its first prompt.
    pending_commands: std::collections::HashMap<PaneId, Vec<String>>,
    /// Running-process close/quit warning awaiting a decision.
    confirm: Option<ConfirmClose>,
    /// Keys whose press went to a program reporting key events (kitty
    /// flag 2), so their release follows them there.
    keys_down: std::collections::HashSet<(PaneId, String)>,
    /// Hint mode labels while active (⇧⌘H).
    hints: Option<HintState>,
    /// Transcript id to replay into the next spawned pane (session restore).
    next_replay: Option<String>,
    /// Session-service shell for the next spawned pane to reattach.
    next_attach: Option<u64>,
    /// Local control API socket (removed on drop).
    control_server: Option<crate::control::Server>,
    /// Other windows' tabs, fetched when the session switcher opens.
    other_windows: Vec<(u32, std::path::PathBuf, Vec<crate::control::TabInfo>)>,
    /// Detached session-service shells, fetched when the palette opens.
    detached: Vec<shift_pty::ipc::SessionInfo>,
    windows_loading: bool,
    detached_loading: bool,
    /// Attention center entries, newest last.
    attention: Vec<Attention>,
    next_attention_id: u64,
    /// In-window notification toasts: (attention id, shown since), newest
    /// first; hovering resets the time.
    note_toasts: Vec<(u64, Instant)>,
    note_toast_hits: Vec<crate::views::notifications::ToastHit>,
    /// Newest finished block already considered per pane.
    attention_seen: std::collections::HashMap<PaneId, shift_core::BlockId>,
    /// Newest finished block written to each pane's transcript.
    transcript_saved: std::collections::HashMap<PaneId, shift_core::BlockId>,
    /// ssh profiles listed in the palette (config + ~/.ssh/config).
    ssh_profiles: Vec<shift_core::ssh::SshProfile>,
    /// Pipe modal target: (pane, block, the block's output text).
    pipe_target: Option<(PaneId, shift_core::BlockId, String)>,
    /// Screen reader bridge (AccessKit); created before the window shows.
    a11y: Option<accesskit_winit::Adapter>,
    /// Rects behind actionable accessibility nodes (click targets).
    a11y_targets: std::collections::HashMap<accesskit::NodeId, [f32; 4]>,
    /// Last pushed tree (unchanged frames push nothing); empty = resend.
    a11y_fingerprint: String,
    /// Newest finished block announced per pane.
    a11y_done: std::collections::HashMap<PaneId, shift_core::BlockId>,
    a11y_announcement: Option<String>,
    /// `SHIFT_LATENCY_LOG`: log key-to-frame latency of raw keystrokes.
    latency_log: bool,
    /// Raw key sent at .0; .1 = the program's output has arrived since.
    latency_probe: Option<(Instant, bool)>,
    /// [cancel, destructive] button rects of the confirm modal.
    confirm_rects: [[f32; 4]; 2],
    /// What's New modal (views/whats_new.rs) and the releases it lists.
    whats_new: WhatsNewState,
    whats_new_hits: WhatsNewHits,
    releases: Vec<shift_core::changelog::Release>,
    /// Notifications popover (views/notifications.rs) under the inbox button.
    notifications: NotificationsState,
    notifications_hits: NotificationsHits,
    /// Side panels (views/explorer.rs, views/review_panel.rs) and the file
    /// viewer over the pane area, with their last frame's click targets.
    explorer: ExplorerState,
    explorer_hits: Vec<([f32; 4], ExplorerHit)>,
    review: ReviewPanelState,
    review_hits: Vec<([f32; 4], ReviewHit)>,
    /// XTWINOPS state: iconified by a program, the last requested position
    /// (a hidden window reports none), the size before a one-way maximize.
    iconified: bool,
    /// The tab label's shell name for the configured shell: resolving it
    /// stats the shell binaries, too slow for every frame.
    shell_label: Option<(Option<String>, String)>,
    /// When the last frame was drawn, and the frame output asked for that
    /// waits out [`OUTPUT_FRAME`].
    last_frame: Instant,
    output_frame_due: Option<Instant>,
    fullscreen_op: bool,
    window_pos: Option<(f64, f64)>,
    restore_size: Option<winit::dpi::LogicalSize<f64>>,
    /// The hit rects under the pointer (folded indices into `hits`) and how
    /// often a change of them repainted: hover styling follows the mouse.
    hover_sig: u64,
    hover_repaints: u32,
    /// Tooltips of this frame's controls, in click priority
    /// (`views::tooltip`); a tip with no text blocks the ones under it.
    tips: Vec<crate::views::tooltip::Tip>,
    /// The control under the pointer and since when it rests there.
    tip_hover: Option<([f32; 4], Instant)>,
    /// The tooltip drawn in the last frame.
    tip_shown: Option<crate::views::tooltip::Tip>,
    /// When a shown tooltip last went away (the next one shows at once).
    tip_hidden_at: Option<Instant>,
    /// A press, key or wheel hid it until the pointer moves to another control.
    tip_suppressed: bool,
    /// The pointer is over the window.
    pointer_in: bool,
    /// Directory the review model was last requested for.
    review_cwd: Option<String>,
    viewer: ViewerState,
    viewer_hits: Vec<([f32; 4], ViewerHit)>,
    /// Pane area the viewer covers (last frame).
    viewer_rect: [f32; 4],
    /// The viewer pane (`VIEWER_PANE` leaf) has keyboard focus rather than
    /// its tab's `active_pane` shell.
    viewer_focus: bool,
    /// Inline tab rename in progress: (tab index, edited title).
    rename: Option<(usize, String)>,
    /// Rename text still fully selected (first keystroke replaces it).
    rename_fresh: bool,
    /// Search overlay edits a per-block output filter instead of find.
    filter_editing: bool,
    /// Aliases/functions/builtins probed from the user's shell (background).
    shell_env: Option<Arc<shift_core::shell_env::ShellEnv>>,
    /// Session file is read/written (off for secondary windows and scripts).
    persist_session: bool,
    /// New shells may run in the session service (`durable_sessions`, and
    /// a scripted run only with `SHIFT_PERSIST_SESSION`: tests leave no
    /// service behind).
    durable_allowed: bool,
    update: UpdateUi,
    /// A `prepare-restart` request was answered: quit, keeping durable
    /// shells.
    restart_requested: bool,
    /// Tab being dragged in the tab bar.
    tab_drag: Option<TabDrag>,
    /// ⌃R command history pane above the command line.
    history: crate::views::history::HistoryPane,
    /// Split pane being moved by its title bar.
    pane_drag: Option<PaneDrag>,
    /// Editor mouse selection in progress (pane).
    editor_drag: Option<PaneId>,
    /// Transient status message (text, shown at).
    toast: Option<(String, Instant)>,
    /// Queue of the completion thread (started on the first request).
    completion_worker: CompletionWorker<CompletionRequest>,
    /// Latest completion request not yet answered.
    completion_pending: Option<CompletionRequest>,
    /// Key that read the menu or ghost while `completion_pending` was in
    /// flight: applied once, when that request's answer arrives.
    completion_intent: Option<CompletionIntent>,
    /// Counter identifying completion requests.
    completion_gen: u64,
    /// Editor buffer the shown completions and ghost belong to.
    completion_buffer: String,
    /// Queue of the review writer thread (started on the first write).
    review_writer: Option<std::sync::mpsc::Sender<ReviewWrite>>,
    /// Why config.toml was last rejected (unreadable or invalid). While set,
    /// the live config is the last good one and Settings does not save, so
    /// the user's file is never overwritten with defaults.
    config_error: Option<String>,
    /// Mouse button held down while reporting to a mouse-tracking app:
    /// (pane, button code, last reported cell).
    mouse_report: Option<(PaneId, u8, (usize, usize))>,
    /// Last cell a button-less motion report was sent for (mode 1003).
    mouse_hover_cell: Option<(PaneId, (usize, usize))>,
    /// IME composition in progress (preedit text, shown at the cursor).
    preedit: String,
    /// Cursor position of the active pane last frame (IME candidate window).
    ime_spot: Option<(f32, f32)>,
    /// Last position handed to the OS input method.
    ime_area_sent: Option<(f32, f32)>,
    /// Pointer shape currently set on the window.
    cursor_icon: winit::window::CursorIcon,
    /// Registered `global_hotkey` (re-registered when the setting changes).
    global_hotkey: Option<crate::hotkey::GlobalHotkey>,
    /// Native menu bar and the keybindings it was built for.
    #[cfg(target_os = "macos")]
    menubar: Option<(
        crate::menubar::MenuBar,
        std::collections::BTreeMap<String, String>,
    )>,
}

/// What a hint label acts on.
#[derive(Debug, Clone, PartialEq)]
enum HintTarget {
    /// A URL or an existing path: open it (⇧: copy it).
    Open(String),
    /// A block header: select it (⇧: copy its output).
    Block(PaneId, shift_core::BlockId),
}

struct HintState {
    /// (label, on-screen rect, target)
    items: Vec<(String, [f32; 4], HintTarget)>,
    /// Label characters typed so far.
    typed: String,
}

/// Black over unfocused split panes: the panes you are not in recede.
const INACTIVE_DIM: f32 = 0.14;
/// The same on a light theme (black reads much heavier on a light ground).
const INACTIVE_DIM_LIGHT: f32 = 0.05;

fn inactive_dim(theme: &shift_core::Theme) -> shift_core::Color {
    shift_core::Color::BLACK.with_alpha(if theme.is_light() {
        INACTIVE_DIM_LIGHT
    } else {
        INACTIVE_DIM
    })
}

/// The welcome list's rows, with shortcuts from the live keymap (row `i`
/// runs `welcome_action(i)`).
fn welcome_entries(keymap: &shift_core::Keymap) -> Vec<crate::views::welcome::Entry> {
    use crate::views::icons::Icon;
    use crate::views::welcome::Entry;
    let keys = |actions: &[&str]| {
        actions
            .iter()
            .find_map(|a| keymap.preferred(a).map(|b| b.chips()))
            .unwrap_or_default()
    };
    vec![
        Entry {
            icon: Icon::SplitRight,
            title: "Split pane",
            detail: "Run two things side by side",
            keys: keys(&["split_vertical"]),
        },
        Entry {
            icon: Icon::Search,
            title: "Command palette",
            detail: "Every action, by name",
            keys: keys(&["palette"]),
        },
        Entry {
            icon: Icon::History,
            title: "Command history",
            detail: "Search everything you ran",
            keys: vec!["⌃".into(), "R".into()],
        },
        Entry {
            icon: Icon::Folder,
            title: "File explorer",
            detail: "Browse and read project files",
            keys: keys(&["toggle_explorer"]),
        },
        Entry {
            icon: Icon::Settings,
            title: "Settings",
            detail: "Theme, fonts, keys",
            keys: keys(&["open_settings"]),
        },
    ]
}

/// Native traffic lights, centred on the tab strip like Linux's drawn ones.
#[cfg(target_os = "macos")]
fn place_traffic_lights(window: &Window) {
    crate::traffic_lights::place(
        window,
        TAB_BAR_H as f64,
        tabbar::TRAFFIC_X as f64,
        tabbar::TRAFFIC_GAP as f64,
    );
}

/// Hint labels in home-row order: one letter each while they fit, two
/// letters for all of them beyond that (no label is a prefix of another).
fn hint_labels(n: usize) -> Vec<String> {
    const KEYS: &str = "asdfghjklqwertyuiopzxcvbnm";
    let keys: Vec<char> = KEYS.chars().collect();
    if n <= keys.len() {
        return keys.iter().take(n).map(|c| c.to_string()).collect();
    }
    keys.iter()
        .flat_map(|a| keys.iter().map(move |b| format!("{a}{b}")))
        .take(n)
        .collect()
}

/// Space or Enter: presses a keyboard-focused control (Space can arrive as
/// a character).
fn press_key(key: &winit::keyboard::Key) -> bool {
    use winit::keyboard::{Key, NamedKey};
    match key {
        Key::Named(NamedKey::Space | NamedKey::Enter) => true,
        Key::Character(c) => c.as_str() == " ",
        _ => false,
    }
}

/// Shortest time a confirmation dialog is on screen before Enter accepts it.
const CONFIRM_ARM: Duration = Duration::from_millis(300);

/// Whether Enter accepts a confirmation shown for `shown`: never an
/// auto-repeat, never before the dialog could have been read.
fn confirm_accepts(repeat: bool, shown: Duration) -> bool {
    !repeat && shown >= CONFIRM_ARM
}

/// How `pane`'s program wants keys encoded.
fn key_encoding(pane: &PaneState, option_as_meta: bool) -> input::KeyEncoding {
    use shift_pty::alacritty_terminal::term::TermMode;
    let mode = *pane.session.term.lock().mode();
    input::KeyEncoding {
        app_cursor: mode.contains(TermMode::APP_CURSOR),
        option_as_meta,
        kitty_flags: crate::keyboard::flags_from_mode(mode),
    }
}

/// Whether `pane`'s program enabled bracketed paste (DECSET 2004).
fn bracketed_paste_on(pane: &PaneState) -> bool {
    use shift_pty::alacritty_terminal::term::TermMode;
    pane.session
        .term
        .lock()
        .mode()
        .contains(TermMode::BRACKETED_PASTE)
}

/// The local path a link target names (`/abs/path` or `file://` URL).
fn local_path_of(target: &str) -> Option<std::path::PathBuf> {
    let rest = match target.get(..7) {
        Some(p) if p.eq_ignore_ascii_case("file://") => {
            let r = &target[7..];
            r.strip_prefix("localhost").unwrap_or(r)
        }
        _ if target.contains("://") => return None,
        _ => target,
    };
    if !rest.starts_with('/') {
        return None;
    }
    // percent-decode (file:// URLs only; plain paths keep a literal '%')
    if target.len() == rest.len() {
        return Some(rest.into());
    }
    let b = rest.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned().into())
}

/// Whether opening `path` would run code (app bundles, scripts, workflows,
/// Terminal `.command` files, anything executable) instead of showing a
/// document.
fn opens_as_program(path: &std::path::Path) -> bool {
    const PROGRAM_EXTS: &[&str] = &[
        "app", "command", "terminal", "workflow", "action", "scpt", "scptd", "tool", "sh", "pkg",
        "mpkg", "dmg", "jar", "desktop", "appimage",
    ];
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PROGRAM_EXTS.contains(&e.to_ascii_lowercase().as_str()))
    {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(path) {
            return m.is_file() && m.permissions().mode() & 0o111 != 0;
        }
    }
    false
}

/// `App::option_meta` over explicit state: `(left, right)` held, and each
/// key's Meta setting.
fn option_meta((left, right): (bool, bool), left_meta: bool, right_meta: bool) -> bool {
    if !left && !right {
        return left_meta;
    }
    (left && left_meta) || (right && right_meta)
}

/// Show `path` in the file manager without running it.
fn reveal_external(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let (opener, args): (&str, Vec<std::ffi::OsString>) =
        ("open", vec!["-R".into(), path.as_os_str().to_owned()]);
    #[cfg(not(target_os = "macos"))]
    let (opener, args): (&str, Vec<std::ffi::OsString>) = (
        "xdg-open",
        vec![
            path.parent()
                .unwrap_or(std::path::Path::new("/"))
                .as_os_str()
                .to_owned(),
        ],
    );
    log::info!("reveal {}", path.display());
    std::process::Command::new(opener)
        .args(args)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("{opener}: {e}"))
}

/// Hand a URL or path to the OS opener (documents only, never script
/// handlers; programs are revealed, not run). `Err` says why nothing opened.
fn open_external(target: &str) -> Result<(), String> {
    if let Some(path) = local_path_of(target)
        && opens_as_program(&path)
    {
        return reveal_external(&path);
    }
    if let Some((scheme, _)) = target.split_once("://")
        && !matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "file" | "ftp"
        )
    {
        return Err(format!("{scheme}: links are not opened"));
    }
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    log::info!("open {target}");
    std::process::Command::new(opener)
        .arg(target)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("{opener}: {e}"))
}

/// Write block output for "Send block output to a command" to a private
/// (0600) file under the data dir; files older than a day are swept first.
fn write_pipe_file(text: &str) -> Option<std::path::PathBuf> {
    use std::io::Write as _;
    let dir = shift_core::shell_integration::data_dir()?.join("pipe");
    std::fs::create_dir_all(&dir).ok()?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(86_400));
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let path = dir.join(format!("output-{}-{nanos}.txt", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path).ok()?;
    f.write_all(text.as_bytes()).ok()?;
    Some(path)
}

/// `YYYY-MM-DD HH:MM` (UTC) for `secs` since the epoch (no date crate).
fn utc_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// How long a toast stays up.
const TOAST_TTL: Duration = Duration::from_millis(2600);

/// What a confirmed close/quit acts on.
#[derive(Debug, Clone, Copy, PartialEq)]
enum CloseScope {
    Pane(PaneId),
    Tab(usize),
    OtherTabs(usize),
    Window,
}

/// What the confirm modal is waiting to do.
#[derive(Debug, Clone, PartialEq)]
enum ConfirmAction {
    Close(CloseScope),
    /// Multiline text headed for a raw pane that has no bracketed paste:
    /// every newline would press Enter.
    Paste {
        pane: PaneId,
        text: String,
    },
    /// Write Shift's shell hooks to an ssh host (runs in a new tab).
    InstallRemote(shift_core::ssh::SshProfile),
    /// Discard unstaged changes to a file or one hunk (review panel).
    Revert {
        root: std::path::PathBuf,
        path: String,
        hunk: Option<shift_core::review::HunkId>,
    },
    /// Install a release and restart (Update / Later).
    Update(Box<UpdatePlan>),
}

/// What the update dialog offers (computed when it opens).
#[derive(Debug, Clone, PartialEq)]
struct UpdatePlan {
    manifest: shift_core::update::Manifest,
    target: shift_core::update::InstallTarget,
    /// Other windows that restart with this one (pid, control socket).
    windows: Vec<(u32, std::path::PathBuf)>,
    description: String,
    note: String,
    /// This install cannot update itself: the button opens the release page.
    open_page: bool,
}

/// This window's view of updates (`shift_core::update`, `updater.rs`).
#[derive(Default)]
struct UpdateUi {
    status: shift_core::update::Status,
    /// "Later" was chosen for this version: no chip.
    dismissed: Option<String>,
    /// Downloading, installing or restarting.
    busy: bool,
    /// The periodic check runs while this is set.
    auto: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Held from the download until this process exits or the install
    /// stops: one window installs at a time.
    lock: Option<shift_core::update::InstallLock>,
    /// The window was closed while busy: quit once the update is done.
    quit_after: bool,
    /// Set at start: `confirm_start` runs this long after the first frame.
    confirm_after: Option<Duration>,
    /// `confirm_start` ran (it runs once).
    start_confirmed: bool,
}

/// The release page when a manifest names none.
const RELEASES_PAGE: &str = "https://github.com/botjaeger/shift-releases/releases";

struct ConfirmClose {
    /// When the dialog appeared: Enter typed ahead of it does not accept.
    opened: Instant,
    action: ConfirmAction,
    /// Running commands (close) or the first pasted lines (paste).
    commands: Vec<String>,
}

impl App {
    pub fn new(
        proxy: EventLoopProxy<UserEvent>,
        script: Option<crate::script::Script>,
        headless: bool,
    ) -> Self {
        // scripted runs and secondary windows never touch the user's session
        // file (a script run used to overwrite it)
        let persist_session = (script.is_none()
            || std::env::var_os("SHIFT_PERSIST_SESSION").is_some())
            && !crate::updater::launch().secondary;
        let durable_allowed =
            script.is_none() || std::env::var_os("SHIFT_PERSIST_SESSION").is_some();
        App {
            proxy,
            window: None,
            gpu: None,
            quads: None,
            bg_image: None,
            sprites: None,
            bg_image_opacity: 1.0,
            text: None,
            git: None,
            pr_worker: None,
            docker_worker: None,
            chips_applied: Vec::new(),
            tabs: Vec::new(),
            active: 0,
            next_pane: 1,
            scene: Scene::default(),
            theme: Theme::default(),
            cell_w: DEFAULT_FONT_SIZE * 0.6,
            modifiers: ModifiersState::empty(),
            alt_sides: (false, false),
            theme_error: None,
            hits: Vec::new(),
            mouse: (0.0, 0.0),
            any_running: false,
            editor_active: false,
            blink_epoch: Instant::now(),
            boot_until: None,
            drag: None,
            palette: PaletteState::default(),
            palette_results: Vec::new(),
            palette_row_hits: Vec::new(),
            settings_names: Vec::new(),
            settings_stops: Vec::new(),
            completion: CompletionUi::default(),
            editor_anchor: (0.0, 0.0),
            editor_cols: 80,
            search_chips: Vec::new(),
            config: Config::default(),
            keymap: Keymap::from_overrides(&Default::default()),
            settings: SettingsState::default(),
            slider_drag: None,
            search: SearchState::default(),
            clipboard: None,
            pending_sel: None,
            last_click: None,
            context_menu: ContextMenuState::default(),
            context_menu_rects: Vec::new(),
            chip_menu: None,
            chip_menu_card: [0.0; 4],
            chip_menu_rows: Vec::new(),
            prompt_editor: None,
            prompt_editor_layout: Default::default(),
            shell_history: Arc::default(),
            font_size: DEFAULT_FONT_SIZE,
            line_h: DEFAULT_FONT_SIZE * 1.2,
            scroll_accum: 0.0,
            closed_tabs: Vec::new(),
            _watcher: None,
            script,
            headless,
            headless_clipboard: String::new(),
            capture: None,
            os_light: false,
            focused: true,
            focus_pane: None,
            bell_flash: None,
            launch_configs: Vec::new(),
            pending_commands: std::collections::HashMap::new(),
            confirm: None,
            keys_down: Default::default(),
            pipe_target: None,
            ssh_profiles: Vec::new(),
            hints: None,
            next_replay: None,
            next_attach: None,
            control_server: None,
            windows_loading: false,
            detached_loading: false,
            other_windows: Vec::new(),
            detached: Vec::new(),
            attention: Vec::new(),
            next_attention_id: 0,
            note_toasts: Vec::new(),
            note_toast_hits: Vec::new(),
            attention_seen: Default::default(),
            transcript_saved: Default::default(),
            a11y: None,
            a11y_targets: Default::default(),
            a11y_fingerprint: String::new(),
            a11y_done: Default::default(),
            a11y_announcement: None,
            latency_log: std::env::var_os("SHIFT_LATENCY_LOG").is_some(),
            latency_probe: None,
            confirm_rects: [[0.0; 4]; 2],
            whats_new: WhatsNewState::on_launch(),
            whats_new_hits: WhatsNewHits::default(),
            releases: shift_core::changelog::releases(),
            notifications: NotificationsState::default(),
            notifications_hits: NotificationsHits::default(),
            explorer: ExplorerState::default(),
            explorer_hits: Vec::new(),
            review: ReviewPanelState::default(),
            review_hits: Vec::new(),
            iconified: false,
            shell_label: None,
            last_frame: Instant::now(),
            output_frame_due: None,
            fullscreen_op: false,
            window_pos: None,
            restore_size: None,
            hover_sig: 0,
            hover_repaints: 0,
            tips: Vec::new(),
            tip_hover: None,
            tip_shown: None,
            tip_hidden_at: None,
            tip_suppressed: false,
            pointer_in: true,
            review_cwd: None,
            viewer: ViewerState::default(),
            viewer_hits: Vec::new(),
            viewer_rect: [0.0; 4],
            viewer_focus: false,
            rename: None,
            rename_fresh: false,
            filter_editing: false,
            shell_env: None,
            persist_session,
            durable_allowed,
            update: UpdateUi::default(),
            restart_requested: false,
            tab_drag: None,
            history: Default::default(),
            pane_drag: None,
            editor_drag: None,
            toast: None,
            config_error: None,
            review_writer: None,
            completion_worker: CompletionWorker::default(),
            completion_pending: None,
            completion_intent: None,
            completion_gen: 0,
            completion_buffer: String::new(),
            mouse_report: None,
            mouse_hover_cell: None,
            preedit: String::new(),
            ime_spot: None,
            ime_area_sent: None,
            cursor_icon: winit::window::CursorIcon::Default,
            global_hotkey: None,
            #[cfg(target_os = "macos")]
            menubar: None,
        }
    }

    /// Re-apply everything derived from Config (theme, metrics, keymap).
    fn apply_config(&mut self) {
        self.sync_control_server();
        if self.window.is_some() {
            self.sync_update_checks();
        }
        self.theme = self.config.resolve_theme_for(self.os_light);
        // a theme that does not load falls back to Dark Default: say so once
        // per broken name instead of only logging it
        let wanted = if self.config.sync_os_theme && self.os_light {
            &self.config.theme_light
        } else {
            &self.config.theme
        };
        let error = shift_core::theme::try_load_by_name(wanted)
            .err()
            .map(|e| format!("Theme {wanted:?}: {e}; using Dark Default"));
        if error.is_some() && error != self.theme_error {
            self.toast(error.clone().unwrap_or_default());
        }
        self.theme_error = error;
        // Warp "window opacity": only the window ground goes translucent
        // (Theme::window_ground / Theme::ground); text, overlays, modals and
        // non-default cell colours stay opaque
        self.theme.ground_alpha = self.config.opacity();
        #[cfg(target_os = "macos")]
        if let Some(w) = &self.window {
            crate::translucency::apply(w, self.config.blur());
            place_traffic_lights(w);
        }
        self.font_size = self.config.font_size();
        self.line_h = self.font_size * self.config.line_height();
        self.keymap = Keymap::from_overrides(&self.config.keybindings);
        self.apply_global_hotkey();
        // menu shortcut labels follow the keymap
        #[cfg(target_os = "macos")]
        if self.window.is_some()
            && self
                .menubar
                .as_ref()
                .is_none_or(|(_, keys)| *keys != self.config.keybindings)
        {
            self.menubar =
                crate::menubar::install(&self.keymap).map(|m| (m, self.config.keybindings.clone()));
        }
        if let Some(engine) = &mut self.text {
            let used = engine.set_mono_family(&self.config.font_family);
            if !used.eq_ignore_ascii_case(self.config.font_family.trim()) {
                log::warn!(
                    "font_family {:?} not installed, using {used}",
                    self.config.font_family
                );
            }
            self.cell_w = engine.mono_cell_width(self.font_size);
        }
        self.apply_background_image();
        let vim = self.config.vim_mode;
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                pane.editor.set_vim_enabled(vim);
            }
        }
        self.refresh_chip_sources();
        self.explorer
            .set_show_hidden(self.config.explorer_hidden_files);
        // save_transcripts off means gone, not just no longer updated —
        // however it was turned off (toggle, ↺, an edit of config.toml)
        if !self.config.save_transcripts {
            shift_core::transcript::remove_except(&[]);
            self.transcript_saved.clear();
        }
        let clipboard = shift_pty::ClipboardAccess::from_config(&self.config.clipboard_access);
        for tab in &self.tabs {
            for pane in &tab.panes {
                pane.session.set_clipboard_access(clipboard);
            }
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// (Re)register `global_hotkey` when it changed. Scripted runs and
    /// secondary windows never grab a system-wide key.
    fn apply_global_hotkey(&mut self) {
        let spec = self.config.global_hotkey.trim().to_string();
        if self.global_hotkey.as_ref().map(|h| h.spec()) == Some(spec.as_str())
            || (spec.is_empty() && self.global_hotkey.is_none())
        {
            return;
        }
        self.global_hotkey = None; // drop unregisters the old key
        if spec.is_empty() || !self.persist_session || self.window.is_none() || self.headless {
            return;
        }
        let proxy = self.proxy.clone();
        match crate::hotkey::register(&spec, move || {
            let _ = proxy.send_event(UserEvent::GlobalHotkey);
        }) {
            Ok(h) => {
                log::info!("global hotkey {spec} registered");
                self.global_hotkey = Some(h);
            }
            Err(e) => {
                log::warn!("{e}");
                self.toast(e);
            }
        }
    }

    /// The window, unless headless: minimize, maximize and fullscreen would
    /// put a hidden window on screen.
    fn shown_window(&self) -> Option<&Arc<Window>> {
        self.window.as_ref().filter(|_| !self.headless)
    }

    /// Global hotkey: bring Shift forward, or hide it when it is already
    /// the focused app (Warp hotkey window behavior).
    fn toggle_visibility(&mut self) {
        let Some(window) = self.window.clone().filter(|_| !self.headless) else {
            return;
        };
        #[cfg(target_os = "macos")]
        {
            if crate::hotkey::app_active() && self.focused {
                crate::hotkey::set_app_shown(false);
            } else {
                crate::hotkey::set_app_shown(true);
                window.set_minimized(false);
                window.focus_window();
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if self.focused && window.is_visible() != Some(false) {
                window.set_minimized(true);
            } else {
                window.set_minimized(false);
                window.set_visible(true);
                window.focus_window();
            }
        }
    }

    /// Load (or drop) the theme's background image into the GPU pipeline.
    fn apply_background_image(&mut self) {
        let (Some(gpu), Some(pipe)) = (&self.gpu, &mut self.bg_image) else {
            return;
        };
        match &self.theme.background_image {
            Some(img) => {
                let mut path = std::path::PathBuf::from(&img.path);
                if path.is_relative()
                    && let Some(dir) = shift_core::config::themes_dir()
                {
                    path = dir.join(path);
                }
                self.bg_image_opacity = img.opacity.clamp(0.0, 1.0);
                // the error already names background_image and the path
                if let Err(e) = pipe.load(&gpu.device, &gpu.queue, &path) {
                    log::warn!("{e}");
                }
            }
            None => pipe.clear(),
        }
    }

    fn save_config(&mut self) {
        if let Some(e) = &self.config_error {
            let msg = format!("Settings not saved: fix config.toml first — {e}");
            self.toast(msg);
            return;
        }
        if let Err(e) = self.config.save() {
            log::warn!("config save failed: {e}");
            self.toast(format!("Could not save settings: {e}"));
        }
    }

    /// Read config.toml. A file that cannot be read or parsed keeps the
    /// current config (at startup: defaults with automation off) and says
    /// why, instead of silently resetting every setting.
    fn load_config(&mut self, startup: bool) {
        match Config::try_load() {
            Ok(c) => {
                self.config = c.unwrap_or_default();
                self.config_error = None;
            }
            Err(e) => {
                log::warn!("config.toml rejected: {e}");
                if startup {
                    self.config = Config::fallback();
                }
                self.toast(format!("config.toml not applied: {e}"));
                self.config_error = Some(e);
            }
        }
    }

    /// Map a mouse position to a grid Point + cell Side using the pane's
    /// frame row cache. Header rows clamp to the nearest grid row.
    fn grid_point_at(pane: &PaneState, mx: f32, my: f32) -> Option<(APoint, ASide)> {
        use crate::views::terminal::RowKind;
        let grid = pane.last_grid?;
        if pane.rows.is_empty() {
            return None;
        }
        // row under cursor, clamped to the visible slice
        let mut idx = (((my - grid.y) / grid.line_h).floor() as i64)
            .clamp(0, pane.rows.len() as i64 - 1) as usize;
        // clamp header rows to the nearest grid row
        if matches!(pane.rows[idx].kind, RowKind::Header(_)) {
            let down = pane.rows[idx..]
                .iter()
                .position(|r| matches!(r.kind, RowKind::Grid { .. }))
                .map(|o| idx + o);
            let up = pane.rows[..idx]
                .iter()
                .rposition(|r| matches!(r.kind, RowKind::Grid { .. }));
            idx = match (up, down) {
                (Some(u), Some(d)) => {
                    if my - pane.rows[u].y < pane.rows[d].y - my {
                        u
                    } else {
                        d
                    }
                }
                (Some(u), None) => u,
                (None, Some(d)) => d,
                (None, None) => return None,
            };
        }
        let RowKind::Grid { abs, content_x } = pane.rows[idx].kind else {
            return None;
        };
        let scrolled = pane
            .session
            .scrolled
            .load(std::sync::atomic::Ordering::Acquire);
        let line = ALine((abs as i64 - scrolled as i64) as i32);
        // DECDWL/DECDHL lines draw every cell two columns wide
        let (wide_line, cols) = {
            use shift_pty::alacritty_terminal::grid::Dimensions;
            let term = pane.session.term.lock();
            let in_grid = line.0 < term.screen_lines() as i32
                && line.0 >= -(term.grid().history_size() as i32);
            let wide = in_grid && term.grid()[line].attr != 0;
            (wide, if wide { grid.cols / 2 } else { grid.cols })
        };
        let cell_w = if wide_line {
            grid.cell_w * 2.0
        } else {
            grid.cell_w
        };
        let colf = ((mx - content_x) / cell_w).max(0.0);
        let col = (colf.floor() as usize).min(cols.saturating_sub(1));
        // past the last cell the pointer is right of its text: take the
        // cell's right half so the last char is included
        let side = if colf >= cols as f32 || colf.fract() >= 0.5 {
            ASide::Right
        } else {
            ASide::Left
        };
        Some((APoint::new(line, ACol(col)), side))
    }

    /// 1, 2, or 3 for single/double/triple click.
    fn click_count(&mut self) -> u8 {
        let now = Instant::now();
        let (mx, my) = self.mouse;
        let count = match self.last_click {
            Some((t, (px, py), c))
                if now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                    && (mx - px).abs() <= DRAG_THRESHOLD
                    && (my - py).abs() <= DRAG_THRESHOLD =>
            {
                if c >= 3 {
                    1
                } else {
                    c + 1
                }
            }
            _ => 1,
        };
        self.last_click = Some((now, (mx, my), count));
        count
    }

    /// Begin selection interaction on `pane` at the current mouse position.
    fn press_selection(&mut self, pane_id: PaneId, clicks: u8) {
        let shift = self.modifiers.shift_key();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        // one selection at a time across panes
        for p in &mut tab.panes {
            if p.session.id != pane_id {
                p.clear_selection();
            }
        }
        let Some(pane) = tab.pane_mut(pane_id) else {
            return;
        };
        // reserve the gesture for mouse-reporting apps unless shift is held
        use shift_pty::alacritty_terminal::term::TermMode;
        if !shift
            && pane
                .session
                .term
                .lock()
                .mode()
                .intersects(TermMode::MOUSE_MODE)
        {
            pane.clear_selection();
            return;
        }
        let (mx, my) = self.mouse;
        let Some((point, side)) = Self::grid_point_at(pane, mx, my) else {
            pane.clear_selection();
            return;
        };
        let scrolled = pane
            .session
            .scrolled
            .load(std::sync::atomic::Ordering::Acquire);
        match clicks {
            2 | 3 => {
                let ty = if clicks == 2 {
                    SelectionType::Semantic
                } else {
                    SelectionType::Lines
                };
                pane.session.term.lock().selection = Some(Selection::new(ty, point, side));
                pane.has_selection = true;
                self.pending_sel = Some(PendingSelection {
                    pane: pane_id,
                    abs: point.line.0 as i64 + scrolled as i64,
                    col: point.column,
                    side,
                    origin: (mx, my),
                    started: true,
                    block: false,
                });
            }
            _ => {
                // click clears; drag past threshold starts a Simple selection
                pane.clear_selection();
                self.pending_sel = Some(PendingSelection {
                    pane: pane_id,
                    abs: point.line.0 as i64 + scrolled as i64,
                    col: point.column,
                    side,
                    origin: (mx, my),
                    started: false,
                    block: self.modifiers.alt_key(),
                });
            }
        }
    }

    /// Select the block whose output holds absolute line `abs` of `pane_id`
    /// (nothing in a full-screen program or on prompt rows).
    fn select_block_at(&mut self, pane_id: PaneId, abs: i64) {
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return;
        };
        if pane.alt_screen || abs < 0 {
            return;
        }
        let block = pane
            .session
            .blocks
            .lock()
            .unwrap()
            .block_at(abs as u64)
            .map(|b| b.id);
        if let Some(block) = block {
            pane.selected_block = Some(block);
        }
    }

    /// Extend the pending/live selection to the current mouse position.
    fn drag_selection(&mut self) {
        let Some(pending) = &mut self.pending_sel else {
            return;
        };
        let (mx, my) = self.mouse;
        if !pending.started {
            let (ox, oy) = pending.origin;
            if (mx - ox).abs() < DRAG_THRESHOLD && (my - oy).abs() < DRAG_THRESHOLD {
                return;
            }
            pending.started = true;
        }
        let pane_id = pending.pane;
        let (anchor_abs, anchor_col, anchor_side, block) =
            (pending.abs, pending.col, pending.side, pending.block);
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return;
        };
        let Some((point, side)) = Self::grid_point_at(pane, mx, my) else {
            return;
        };
        let scrolled = pane
            .session
            .scrolled
            .load(std::sync::atomic::Ordering::Acquire);
        let anchor = APoint::new(ALine((anchor_abs - scrolled as i64) as i32), anchor_col);
        let mut term = pane.session.term.lock();
        if term.selection.is_none() {
            let ty = if block {
                SelectionType::Block
            } else {
                SelectionType::Simple
            };
            term.selection = Some(Selection::new(ty, anchor, anchor_side));
        }
        if let Some(sel) = &mut term.selection {
            sel.update(point, side);
        }
        drop(term);
        pane.has_selection = true;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Display-list index of a block's header (current collapse state).
    fn header_display_index(
        pane: &PaneState,
        block: shift_core::BlockId,
        hide_prompt: bool,
    ) -> Option<(usize, usize)> {
        use crate::views::terminal::{DisplayLine, build_display_list_filtered, filter_hidden};
        use shift_pty::alacritty_terminal::grid::Dimensions;
        let (blocks, clear_pad) = {
            let store = pane.session.blocks.lock().unwrap();
            (store.snapshot(), store.clear_pad())
        };
        let scrolled = pane
            .session
            .scrolled
            .load(std::sync::atomic::Ordering::Acquire);
        let (screen_lines, history, live_end) = {
            let term = pane.session.term.lock();
            let g = term.grid();
            // the list as drawn: bottom-anchored ones leave the blank tail out
            let live_end = pane
                .last_grid
                .filter(|g| g.bottom_up)
                .map(|_| crate::views::terminal::live_end(&term, scrolled));
            (g.screen_lines(), g.history_size() as u64, live_end)
        };
        let bi = blocks.iter().position(|b| b.id == block)?;
        let hidden = filter_hidden(&pane.session, pane.filter.as_ref());
        let display = build_display_list_filtered(
            &blocks,
            scrolled,
            screen_lines,
            history,
            false,
            hide_prompt,
            clear_pad,
            pane.last_grid.map_or(screen_lines, |g| g.rows),
            live_end,
            &hidden,
        );
        let idx = display.iter().position(|d| *d == DisplayLine::Header(bi))?;
        Some((idx, display.len()))
    }

    /// Scroll so `block`'s header sits at the top of the viewport.
    fn jump_to_block(&mut self, pane_id: PaneId, block: shift_core::BlockId) {
        let editor_visible = self.editor_active;
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return;
        };
        let rows = pane.last_grid.map(|g| g.rows).unwrap_or(24);
        if let Some((idx, len)) = Self::header_display_index(pane, block, editor_visible) {
            let max_off = len.saturating_sub(rows);
            // view_offset counts from the bottom: header at top ⇒
            // offset = len - rows - idx (clamped)
            pane.view_offset =
                (len as i64 - rows as i64 - idx as i64).clamp(0, max_off as i64) as usize;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Display-list index of a block's last visible row (collapse-aware).
    fn block_end_display_index(
        pane: &PaneState,
        block: shift_core::BlockId,
        hide_prompt: bool,
    ) -> Option<(usize, usize)> {
        use crate::views::terminal::{DisplayLine, build_display_list_filtered, filter_hidden};
        use shift_pty::alacritty_terminal::grid::Dimensions;
        let (blocks, clear_pad) = {
            let store = pane.session.blocks.lock().unwrap();
            (store.snapshot(), store.clear_pad())
        };
        let scrolled = pane
            .session
            .scrolled
            .load(std::sync::atomic::Ordering::Acquire);
        let (screen_lines, history, live_end) = {
            let term = pane.session.term.lock();
            let g = term.grid();
            // the list as drawn: bottom-anchored ones leave the blank tail out
            let live_end = pane
                .last_grid
                .filter(|g| g.bottom_up)
                .map(|_| crate::views::terminal::live_end(&term, scrolled));
            (g.screen_lines(), g.history_size() as u64, live_end)
        };
        let bi = blocks.iter().position(|b| b.id == block)?;
        let (start, end) = (blocks[bi].prompt_start, blocks[bi].end.unwrap_or(u64::MAX));
        let hidden = filter_hidden(&pane.session, pane.filter.as_ref());
        let display = build_display_list_filtered(
            &blocks,
            scrolled,
            screen_lines,
            history,
            false,
            hide_prompt,
            clear_pad,
            pane.last_grid.map_or(screen_lines, |g| g.rows),
            live_end,
            &hidden,
        );
        let idx = display.iter().rposition(|d| match d {
            DisplayLine::Header(i) => *i == bi,
            DisplayLine::Grid(abs) => *abs >= start && *abs < end,
            DisplayLine::Pad => false,
        })?;
        Some((idx, display.len()))
    }

    /// Scroll so `block`'s last row sits at the bottom of the viewport.
    fn jump_to_block_end(&mut self, pane_id: PaneId, block: shift_core::BlockId) {
        let editor_visible = self.editor_active;
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return;
        };
        if let Some((idx, len)) = Self::block_end_display_index(pane, block, editor_visible) {
            let rows = pane.last_grid.map(|g| g.rows).unwrap_or(24);
            let max_off = len.saturating_sub(rows);
            // bottom visible row = len - 1 - view_offset  ⇒  put idx there
            pane.view_offset = (len as i64 - 1 - idx as i64).clamp(0, max_off as i64) as usize;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Jump to the next/prev bookmarked block relative to the viewport top.
    fn jump_bookmark(&mut self, dir: i64) {
        let editor_visible = self.editor_active;
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            return;
        };
        let marks = pane.session.blocks.lock().unwrap().bookmarks();
        if marks.is_empty() {
            return;
        }
        // current top-of-viewport display index
        let rows = pane.last_grid.map(|g| g.rows).unwrap_or(24);
        let len = pane.last_display_len;
        let top = len.saturating_sub(rows).saturating_sub(pane.view_offset);
        // pick first bookmark whose header index is beyond `top` in `dir`
        let mut best: Option<(usize, shift_core::BlockId)> = None;
        for id in marks {
            if let Some((idx, _)) = Self::header_display_index(pane, id, editor_visible) {
                let cmp = idx as i64 - top as i64;
                if (dir > 0 && cmp > 0) || (dir < 0 && cmp < 0) {
                    let d = cmp.unsigned_abs() as usize;
                    if best.is_none() || d < best.unwrap().0 {
                        best = Some((d, id));
                    }
                }
            }
        }
        if let Some((_, id)) = best {
            let pane_id = pane.session.id;
            self.jump_to_block(pane_id, id);
        }
    }

    /// Open the block context menu at `pos` (mouse position for right-click,
    /// header anchor for ⌃M).
    fn open_context_menu(&mut self, pane_id: PaneId, block: shift_core::BlockId, pos: (f32, f32)) {
        let Some(pane) = self.tabs.get(self.active).and_then(|t| t.pane(pane_id)) else {
            return;
        };
        let (collapsed, bookmarked, has_output, has_cwd, has_branch) = {
            let blocks = pane.session.blocks.lock().unwrap();
            let Some(b) = blocks.iter().find(|b| b.id == block) else {
                return;
            };
            (
                b.collapsed,
                b.bookmarked,
                b.output_range().is_some(),
                b.cwd.is_some(),
                b.branch.is_some(),
            )
        };
        let filtered = pane.filter.as_ref().is_some_and(|(b, _)| *b == block);
        let rerun_ok = self.editor_active;
        self.context_menu = ContextMenuState {
            open: true,
            pane: pane_id,
            block,
            pos,
            items: vec![
                MenuItem::new("Copy command", MenuAction::CopyCommand),
                MenuItem::new("Copy output", MenuAction::CopyOutput).enabled(has_output),
                MenuItem::new("Copy both", MenuAction::CopyBoth).enabled(has_output),
                MenuItem::new("Copy working directory", MenuAction::CopyCwd).enabled(has_cwd),
                MenuItem::new("Copy git branch", MenuAction::CopyBranch).enabled(has_branch),
                MenuItem::new("Re-run command", MenuAction::Rerun)
                    .enabled(rerun_ok)
                    .sep(),
                MenuItem::new(
                    if filtered {
                        "Clear output filter"
                    } else {
                        "Filter output…"
                    },
                    MenuAction::FilterBlock,
                )
                .enabled(has_output),
                MenuItem::new("Send output to command…", MenuAction::PipeOutput)
                    .enabled(has_output && rerun_ok),
                MenuItem::new(
                    if bookmarked {
                        "Remove bookmark"
                    } else {
                        "Bookmark"
                    },
                    MenuAction::ToggleBookmark,
                ),
                MenuItem::new(
                    if collapsed { "Expand" } else { "Collapse" },
                    MenuAction::ToggleCollapse,
                )
                .enabled(has_output),
                MenuItem::new("Scroll to top of block", MenuAction::ScrollTopOfBlock).sep(),
                MenuItem::new("Scroll to bottom of block", MenuAction::ScrollBottomOfBlock),
            ],
            selected: None,
        };
    }

    /// Right-click outside any block: clipboard, splits, clear, close.
    fn open_terminal_menu(&mut self, pane_id: PaneId, pos: (f32, f32)) {
        let has_sel = self
            .tabs
            .get(self.active)
            .and_then(|t| t.pane(pane_id))
            .is_some_and(|p| p.has_selection);
        let split = self.tabs.get(self.active).is_some_and(|t| t.is_split());
        let mut items = vec![
            MenuItem::new("Copy", MenuAction::Copy).enabled(has_sel),
            MenuItem::new("Paste", MenuAction::Paste),
            MenuItem::new("Select all", MenuAction::SelectAll),
            MenuItem::new("Split pane right", MenuAction::SplitRight).sep(),
            MenuItem::new("Split pane down", MenuAction::SplitDown),
        ];
        if split {
            items.push(MenuItem::new("Maximize pane", MenuAction::Maximize));
            items.push(MenuItem::new("Move to new tab", MenuAction::MoveToNewTab));
        }
        items.push(MenuItem::new("Clear blocks", MenuAction::ClearBlocks).sep());
        items.push(MenuItem::new("Close pane", MenuAction::ClosePane));
        self.context_menu = ContextMenuState {
            open: true,
            pane: pane_id,
            block: 0,
            pos,
            items,
            selected: None,
        };
    }

    /// The file viewer's ⋮ menu.
    fn open_viewer_menu(&mut self, pos: (f32, f32)) {
        let markdown = !self.viewer.markdown.is_empty();
        let mut items = vec![
            MenuItem::new("Insert path", MenuAction::ViewerInsertPath),
            MenuItem::new("Copy path", MenuAction::ViewerCopyPath),
            MenuItem::new("Find in file", MenuAction::ViewerFind).sep(),
        ];
        if markdown {
            items.push(if self.viewer.rendered {
                MenuItem::new("Show raw", MenuAction::ViewerRendered(false))
            } else {
                MenuItem::new("Show rendered", MenuAction::ViewerRendered(true))
            });
        }
        items.push(MenuItem::new("Close", MenuAction::ViewerClose).sep());
        self.context_menu = ContextMenuState {
            open: true,
            pane: VIEWER_PANE,
            block: 0,
            pos,
            items,
            selected: None,
        };
    }

    /// Right-click a tab: rename, duplicate, color, sync, close variants.
    fn open_tab_menu(&mut self, tab: usize, pos: (f32, f32)) {
        let Some(t) = self.tabs.get(tab) else { return };
        let n = self.tabs.len();
        let split = t.is_split();
        let sync = t.sync_inputs;
        let mut items = vec![
            MenuItem::new("Rename tab", MenuAction::RenameTab(tab)),
            MenuItem::new("Duplicate tab", MenuAction::DuplicateTab(tab)),
            MenuItem::new(
                if sync {
                    "Stop synchronizing inputs"
                } else {
                    "Synchronize inputs in panes"
                },
                MenuAction::ToggleSync(tab),
            )
            .enabled(split || sync),
        ];
        for (i, name) in ["Red", "Green", "Yellow", "Blue", "Magenta", "Cyan"]
            .into_iter()
            .enumerate()
        {
            let item = MenuItem::new(name, MenuAction::SetTabColor(tab, Some(i)));
            items.push(if i == 0 { item.sep() } else { item });
        }
        items.push(
            MenuItem::new("No color", MenuAction::SetTabColor(tab, None))
                .enabled(t.color.is_some()),
        );
        items.push(MenuItem::new("Close tab", MenuAction::CloseTab(tab)).sep());
        items.push(
            MenuItem::new("Close other tabs", MenuAction::CloseOtherTabs(tab)).enabled(n > 1),
        );
        items.push(
            MenuItem::new("Close tabs to the right", MenuAction::CloseTabsRight(tab))
                .enabled(tab + 1 < n),
        );
        self.context_menu = ContextMenuState {
            open: true,
            pane: 0,
            block: 0,
            pos,
            items,
            selected: None,
        };
    }

    /// Keyboard in an open context menu: ↑/↓ move, Enter runs, Esc closes.
    /// Returns false when the key should close the menu.
    fn handle_menu_key(&mut self, event: &input::KeyPress, event_loop: &ActiveEventLoop) -> bool {
        use winit::keyboard::{Key, NamedKey};
        match event.logical_key {
            Key::Named(NamedKey::ArrowDown) | Key::Named(NamedKey::Tab) => {
                self.context_menu.step(1);
                true
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.context_menu.step(-1);
                true
            }
            Key::Named(NamedKey::Enter) => {
                if let Some(i) = self.context_menu.selected
                    && let Some(item) = self.context_menu.items.get(i)
                    && item.enabled
                {
                    let action = item.action;
                    self.run_menu_action(action, event_loop);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn run_menu_action(&mut self, action: MenuAction, event_loop: &ActiveEventLoop) {
        let pane_id = self.context_menu.pane;
        let block = self.context_menu.block;
        self.context_menu.close();
        let editor_active = self.editor_active;

        // actions that are not about one block
        match action {
            MenuAction::Copy => return self.handle_action(AppAction::Copy, event_loop),
            MenuAction::Paste => return self.handle_action(AppAction::Paste, event_loop),
            MenuAction::SelectAll => return self.handle_action(AppAction::SelectAll, event_loop),
            MenuAction::SplitRight => return self.handle_action(AppAction::SplitRight, event_loop),
            MenuAction::SplitDown => return self.handle_action(AppAction::SplitDown, event_loop),
            MenuAction::ClearBlocks => {
                return self.handle_action(AppAction::ClearBlocks, event_loop);
            }
            MenuAction::Maximize => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.active_pane = pane_id;
                }
                return self.handle_action(AppAction::MaximizePane, event_loop);
            }
            MenuAction::MoveToNewTab => {
                let from = self.active;
                return self.drop_pane(pane_id, from, PaneDrop::NewTab);
            }
            MenuAction::ClosePane => {
                return self.request_close(CloseScope::Pane(pane_id), event_loop);
            }
            MenuAction::ViewerInsertPath => {
                if let Some(p) = self.viewer.path.clone() {
                    self.viewer.close();
                    self.drop_file(&p);
                }
                return;
            }
            MenuAction::ViewerCopyPath => {
                if let Some(p) = self.viewer.path.clone() {
                    self.copy_to_clipboard(p.to_string_lossy().into_owned());
                }
                return;
            }
            MenuAction::ViewerFind => {
                self.viewer.open_find();
                self.viewer_focus = true;
                return;
            }
            MenuAction::ViewerRendered(on) => {
                self.viewer.set_rendered(on);
                return;
            }
            MenuAction::ViewerClose => {
                self.viewer.close();
                return;
            }
            MenuAction::RenameTab(i) => {
                self.start_rename(i);
                return;
            }
            MenuAction::DuplicateTab(i) => {
                let cwd = self
                    .tabs
                    .get(i)
                    .and_then(|t| t.active())
                    .and_then(|p| p.session.cwd.lock().unwrap().clone())
                    .map(std::path::PathBuf::from);
                self.spawn_tab_at(cwd.filter(|p| p.is_dir()));
                // keep the copy next to its source
                let new = self.tabs.len() - 1;
                if i + 1 < new {
                    let tab = self.tabs.remove(new);
                    self.tabs.insert(i + 1, tab);
                    self.active = i + 1;
                }
                self.save_session();
                return;
            }
            MenuAction::ToggleSync(i) => {
                if let Some(t) = self.tabs.get_mut(i) {
                    t.sync_inputs = !t.sync_inputs;
                }
                return;
            }
            MenuAction::SetTabColor(i, c) => {
                if let Some(t) = self.tabs.get_mut(i) {
                    t.color = c;
                }
                self.save_session();
                return;
            }
            MenuAction::CloseTab(i) => return self.request_close(CloseScope::Tab(i), event_loop),
            MenuAction::CloseOtherTabs(i) => {
                return self.request_close(CloseScope::OtherTabs(i), event_loop);
            }
            MenuAction::CloseTabsRight(i) => {
                // close from the end so indices stay valid
                for t in (i + 1..self.tabs.len()).rev() {
                    self.request_close(CloseScope::Tab(t), event_loop);
                    if self.confirm.is_some() {
                        break;
                    }
                }
                return;
            }
            MenuAction::FilterBlock => {
                self.toggle_block_filter(pane_id, block);
                return;
            }
            MenuAction::PipeOutput => {
                if let Some(p) = self
                    .tabs
                    .get_mut(self.active)
                    .and_then(|t| t.pane_mut(pane_id))
                {
                    p.selected_block = Some(block);
                }
                self.open_pipe();
                return;
            }
            _ => {}
        }

        // gather everything under a short borrow, then act on self
        let mut to_copy: Option<String> = None;
        let mut jump_top = false;
        let mut jump_bottom = false;
        let mut rerun: Option<String> = None;
        {
            let Some(pane) = self
                .tabs
                .get_mut(self.active)
                .and_then(|t| t.pane_mut(pane_id))
            else {
                return;
            };
            let (command, cwd, branch) = pane
                .session
                .blocks
                .lock()
                .unwrap()
                .iter()
                .find(|b| b.id == block)
                .map(|b| (b.command.clone(), b.cwd.clone(), b.branch.clone()))
                .unwrap_or_default();
            match action {
                MenuAction::CopyCommand => to_copy = Some(command),
                MenuAction::CopyCwd => to_copy = cwd,
                MenuAction::CopyBranch => to_copy = branch,
                MenuAction::ScrollTopOfBlock => jump_top = true,
                MenuAction::ScrollBottomOfBlock => jump_bottom = true,
                MenuAction::CopyOutput => {
                    to_copy = terminal::block_output_text(&pane.session, block);
                }
                MenuAction::CopyBoth => {
                    let output =
                        terminal::block_output_text(&pane.session, block).unwrap_or_default();
                    to_copy = Some(format!("{command}\n{output}"));
                }
                MenuAction::Rerun => {
                    if !command.is_empty() && editor_active {
                        rerun = Some(command);
                    }
                }
                MenuAction::ToggleBookmark => {
                    pane.session.blocks.lock().unwrap().toggle_bookmark(block);
                }
                MenuAction::ToggleCollapse => {
                    pane.session.blocks.lock().unwrap().toggle_collapsed(block);
                }
                _ => {}
            }
        }
        if let Some(cmd) = rerun {
            self.submit_to(pane_id, &cmd);
        }
        if let Some(text) = to_copy {
            self.copy_to_clipboard(text);
        }
        if jump_top {
            self.jump_to_block(pane_id, block);
        }
        if jump_bottom {
            self.jump_to_block_end(pane_id, block);
        }
    }

    /// Block filter (Warp): open the filter bar for `block`, or clear the
    /// filter when that block is already filtered.
    fn toggle_block_filter(&mut self, pane_id: PaneId, block: shift_core::BlockId) {
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return;
        };
        if pane.filter.as_ref().is_some_and(|(b, _)| *b == block) {
            pane.filter = None;
            self.filter_editing = false;
            self.search.close();
            return;
        }
        pane.filter = Some((block, String::new()));
        pane.selected_block = Some(block);
        self.search.close();
        self.search.open = true;
        self.filter_editing = true;
        self.blink_epoch = Instant::now();
        self.jump_to_block(pane_id, block);
    }

    /// Command text of `block` (None when missing or empty).
    fn block_command(&self, pane_id: PaneId, block: shift_core::BlockId) -> Option<String> {
        let pane = self.tabs.get(self.active)?.pane(pane_id)?;
        let cmd = pane
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .find(|b| b.id == block)
            .map(|b| b.command.clone())?;
        (!cmd.is_empty()).then_some(cmd)
    }

    /// Target for block-scoped shortcuts: the keyboard-selected block when
    /// still present, else the most recent finished block of the active pane.
    fn selected_or_last_block(&self) -> Option<(PaneId, shift_core::BlockId)> {
        let pane = self.tabs.get(self.active)?.active()?;
        let blocks = pane.session.blocks.lock().unwrap();
        if let Some(sel) = pane.selected_block
            && blocks.iter().any(|b| b.id == sel)
        {
            return Some((pane.session.id, sel));
        }
        blocks
            .iter()
            .rev()
            .find(|b| b.suppressed_range().is_some())
            .map(|b| (pane.session.id, b.id))
    }

    /// ⌘↑ / ⌘↓: move the keyboard block selection; past the newest block the
    /// selection clears and the view snaps back to the input.
    fn select_block(&mut self, dir: i64) {
        let editor_visible = self.editor_active;
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            return;
        };
        let eligible: Vec<shift_core::BlockId> = pane
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .filter(|b| b.suppressed_range().is_some())
            .map(|b| b.id)
            .collect();
        if eligible.is_empty() {
            return;
        }
        let cur = pane.selected_block.filter(|id| eligible.contains(id));
        let next = match (cur, dir) {
            (None, d) if d < 0 => Some(*eligible.last().unwrap()),
            (None, _) => None,
            (Some(id), d) => {
                let i = eligible.iter().position(|b| *b == id).unwrap() as i64 + d;
                if i < 0 {
                    Some(eligible[0]) // clamp at the oldest
                } else if (i as usize) < eligible.len() {
                    Some(eligible[i as usize])
                } else {
                    None // past the newest → back to the input
                }
            }
        };
        pane.selected_block = next;
        match next {
            Some(id) => {
                let pane_id = pane.session.id;
                let rows = pane.last_grid.map(|g| g.rows).unwrap_or(24);
                let len = pane.last_display_len;
                let top = len.saturating_sub(rows).saturating_sub(pane.view_offset);
                let offscreen = Self::header_display_index(pane, id, editor_visible)
                    .is_some_and(|(idx, _)| idx < top || idx >= top + rows);
                if offscreen {
                    self.jump_to_block(pane_id, id);
                }
            }
            None => pane.view_offset = 0,
        }
    }

    /// cmd+click on a detected URL opens it. Returns true when handled.
    fn try_open_url(&mut self, pane_id: PaneId) -> bool {
        if !self.modifiers.super_key() {
            return false;
        }
        let (mx, my) = self.mouse;
        let url = self
            .tabs
            .get(self.active)
            .and_then(|t| t.pane(pane_id))
            .and_then(|p| {
                p.url_hits
                    .iter()
                    .find(|(r, _)| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3])
                    .map(|(_, u)| u.clone())
            });
        let target = match url {
            Some(url) => {
                let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
                // only schemes that open documents, never script handlers
                if !matches!(
                    scheme.as_deref(),
                    Some("http" | "https" | "file" | "mailto" | "ftp")
                ) {
                    return false;
                }
                url
            }
            None => match self.path_under_mouse(pane_id) {
                Some(path) => path,
                None => return false,
            },
        };
        self.open_target(&target);
        true
    }

    /// A pane for a new shell, with the editor settings applied (vim mode
    /// was only set on panes that existed when the config was applied).
    fn new_pane(&self, session: Session) -> PaneState {
        let mut pane = PaneState::new(session);
        pane.editor.set_vim_enabled(self.config.vim_mode);
        pane
    }

    /// Option sends Meta for the Option key held now: the left key follows
    /// `option_as_meta`, the right one `right_option_as_meta`. Without side
    /// information (Linux, synthesized input) `option_as_meta` decides.
    fn option_meta(&self) -> bool {
        option_meta(
            self.alt_sides,
            self.config.option_as_meta,
            self.config.right_option_meta(),
        )
    }

    /// Open a link or path: an existing local file or folder where
    /// `link_editor` says, anything else with the OS opener.
    fn open_target(&mut self, target: &str) {
        let choice = self.config.link_editor.trim().to_string();
        match local_path_of(target).filter(|p| p.exists()) {
            Some(path) => self.open_file_in(&path, &choice),
            None => self.open_target_external(target),
        }
    }

    /// Open a link or path with the OS opener; a headless run only reports
    /// it (an opened browser or editor would take the user's focus).
    fn open_target_external(&mut self, target: &str) {
        if self.headless {
            log::info!("headless: not opening {target}");
            self.toast(format!("Headless: not opening {target}"));
        } else if let Some(path) = local_path_of(target).filter(|p| opens_as_program(p)) {
            // a program: show it in the file manager instead of running it
            match reveal_external(&path) {
                Ok(()) => self.toast(format!("Revealed {} (not run)", path.display())),
                Err(e) => self.toast(format!("Could not reveal {}: {e}", path.display())),
            }
        } else if let Err(e) = open_external(target) {
            self.toast(format!("Could not open {target}: {e}"));
        }
    }

    /// Open a file or folder with editor setting `choice`
    /// (`shift_core::editors`): the viewer pane, the OS opener, or an
    /// installed editor. A folder never goes to the viewer, and a missing
    /// editor falls back to the viewer (files) or the OS opener (folders)
    /// with a toast saying so.
    fn open_file_in(&mut self, path: &std::path::Path, choice: &str) {
        use shift_core::editors;
        let viewer_ok = path.is_file();
        match choice.trim() {
            editors::SHIFT if viewer_ok => self.open_viewer(path),
            "" | editors::SYSTEM | editors::SHIFT => {
                self.open_target_external(&path.to_string_lossy())
            }
            id => match editors::by_id(id).filter(|e| e.installed()) {
                Some(_) if self.headless => {
                    log::info!("headless: not opening {} in {id}", path.display());
                    self.toast(format!(
                        "Headless: not opening {} in {}",
                        path.display(),
                        editors::label(id)
                    ));
                }
                Some(editor) => {
                    let (program, args) = editor.command(path);
                    log::info!("open {} in {}", path.display(), editor.label);
                    let spawned = std::process::Command::new(&program)
                        .args(&args)
                        .stdin(std::process::Stdio::null())
                        .spawn();
                    if let Err(e) = spawned {
                        self.toast(format!("Could not open {}: {program}: {e}", editor.label));
                    }
                }
                None => {
                    self.toast(format!(
                        "{} is not installed; opening here instead",
                        editors::label(id)
                    ));
                    if viewer_ok {
                        self.open_viewer(path);
                    } else {
                        self.open_target_external(&path.to_string_lossy());
                    }
                }
            },
        }
    }

    /// Resolve a path token on output line `abs` of `pane` to an existing
    /// file (absolute, `~/…`, or relative to the directory the command that
    /// printed it ran in; the pane's current one for unattributed rows).
    fn resolve_path(pane: &PaneState, abs: u64, path: &str) -> Option<String> {
        let home = std::env::var("HOME").unwrap_or_default();
        let resolved = if let Some(rest) = path.strip_prefix("~/") {
            std::path::Path::new(&home).join(rest)
        } else if path.starts_with('/') {
            std::path::PathBuf::from(path)
        } else {
            let block_cwd = pane
                .session
                .blocks
                .lock()
                .unwrap()
                .cwd_at(abs)
                .map(str::to_string);
            let cwd = match block_cwd {
                Some(cwd) => cwd,
                None => pane.session.cwd.lock().unwrap().clone()?,
            };
            std::path::Path::new(&cwd).join(path)
        };
        resolved
            .exists()
            .then(|| resolved.to_string_lossy().into_owned())
    }

    /// Hint mode: label every visible link, existing path and block header
    /// of the active tab; typing a label opens it (links, paths) or selects
    /// it (blocks); ⇧ + label copies instead.
    fn start_hints(&mut self) {
        use crate::views::terminal::RowKind;
        let mut targets: Vec<([f32; 4], HintTarget)> = Vec::new();
        if let Some(tab) = self.tabs.get(self.active) {
            for pane in &tab.panes {
                let Some(g) = pane.last_grid else { continue };
                for (rect, url) in &pane.url_hits {
                    targets.push((*rect, HintTarget::Open(url.clone())));
                }
                // the sticky header stands in for a block scrolled off the top
                if let Some((rect, block)) = pane.sticky_hit {
                    targets.push((rect, HintTarget::Block(pane.session.id, block)));
                }
                for row in &pane.rows {
                    let RowKind::Grid { abs, content_x } = row.kind else {
                        continue;
                    };
                    let Some(text) = terminal::session_line_text(&pane.session, abs) else {
                        continue;
                    };
                    for (col, len, token) in terminal::path_tokens(&text) {
                        if let Some(path) = Self::resolve_path(pane, abs, &token) {
                            let rect = [
                                content_x + col as f32 * g.cell_w,
                                row.y,
                                len as f32 * g.cell_w,
                                g.line_h,
                            ];
                            targets.push((rect, HintTarget::Open(path)));
                        }
                    }
                }
            }
        }
        for h in &self.hits {
            if let ClickTarget::Block(pane, block) = h.target {
                targets.push((h.rect, HintTarget::Block(pane, block)));
            }
        }
        if targets.is_empty() {
            self.toast("No links, paths or blocks on screen".into());
            return;
        }
        let labels = hint_labels(targets.len());
        self.hints = Some(HintState {
            items: labels
                .into_iter()
                .zip(targets)
                .map(|(l, (r, t))| (l, r, t))
                .collect(),
            typed: String::new(),
        });
    }

    /// Hint-mode keys. Returns true when consumed.
    fn handle_hint_key(&mut self, event: &input::KeyPress) {
        use winit::keyboard::{Key, NamedKey};
        let Some(state) = self.hints.as_mut() else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.hints = None;
                return;
            }
            Key::Named(NamedKey::Backspace) => {
                state.typed.pop();
                return;
            }
            _ => {}
        }
        let Some(c) = event
            .text
            .as_deref()
            .and_then(|t| t.chars().next())
            .filter(char::is_ascii_alphabetic)
        else {
            return;
        };
        let copy = c.is_ascii_uppercase() || self.modifiers.shift_key();
        state.typed.push(c.to_ascii_lowercase());
        let typed = state.typed.clone();
        if let Some((_, _, target)) = state.items.iter().find(|(l, _, _)| *l == typed) {
            let target = target.clone();
            self.hints = None;
            match target {
                HintTarget::Open(t) if copy => {
                    if self.copy_to_clipboard(t) {
                        self.toast("Copied".into());
                    }
                }
                HintTarget::Open(t) => self.open_target(&t),
                HintTarget::Block(pane, block) => {
                    let mut sticky = false;
                    if let Some(tab) = self.tabs.get_mut(self.active) {
                        tab.active_pane = pane;
                        if let Some(p) = tab.pane_mut(pane) {
                            p.selected_block = Some(block);
                            sticky = p.sticky_hit.is_some_and(|(_, b)| b == block);
                        }
                    }
                    // its header is off screen: bring it into view
                    if sticky {
                        self.jump_to_block(pane, block);
                    }
                    if copy {
                        self.handle_action_copy_block(pane, block);
                    }
                }
            }
        } else if !state.items.iter().any(|(l, _, _)| l.starts_with(&typed)) {
            self.hints = None;
        }
    }

    /// ⇧ + block hint: copy that block's output.
    fn handle_action_copy_block(&mut self, pane: PaneId, block: shift_core::BlockId) {
        let text = self
            .tabs
            .get(self.active)
            .and_then(|t| t.pane(pane))
            .and_then(|p| terminal::block_output_text(&p.session, block));
        if let Some(text) = text
            && self.copy_to_clipboard(text)
        {
            self.toast("Copied block output".into());
        }
    }

    /// An existing file or directory named by the text under the pointer
    /// (resolved like `resolve_path`; `file:line:col` suffixes are dropped).
    fn path_under_mouse(&self, pane_id: PaneId) -> Option<String> {
        use crate::views::terminal::RowKind;
        let pane = self.tabs.get(self.active)?.pane(pane_id)?;
        let g = pane.last_grid?;
        let (mx, my) = self.mouse;
        let row = pane
            .rows
            .iter()
            .find(|r| my >= r.y && my < r.y + g.line_h)?;
        let RowKind::Grid { abs, content_x } = row.kind else {
            return None;
        };
        let col = ((mx - content_x) / g.cell_w).floor();
        if col < 0.0 {
            return None;
        }
        let text = terminal::session_line_text(&pane.session, abs)?;
        let (path, _line) = terminal::path_token_at(&text, col as usize)?;
        Self::resolve_path(pane, abs, &path)
    }

    /// Pointer shape: a hand over anything clickable (buttons, tabs, menu
    /// and palette rows, ⌘-clickable links), resize arrows over split
    /// handles, the I-beam over text. Checks surfaces in `on_left` order.
    fn update_cursor_icon(&mut self, window: &Window) {
        use winit::window::CursorIcon;
        let (mx, my) = self.mouse;
        let inside =
            |r: &[f32; 4]| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3];
        let hand = |on: bool| {
            if on {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            }
        };
        let overlay = if self.pane_drag.as_ref().is_some_and(|d| d.live) {
            Some(CursorIcon::Grabbing)
        } else if self.confirm.is_some() {
            Some(hand(self.confirm_rects.iter().any(inside)))
        } else if self.whats_new.open {
            let h = &self.whats_new_hits;
            Some(hand(inside(&h.close) || inside(&h.done)))
        } else if self.notifications.open {
            let h = &self.notifications_hits;
            Some(hand(
                inside(&h.close)
                    || inside(&h.mark_read)
                    || h.chips.iter().any(|(r, _)| inside(r))
                    || h.rows.iter().any(|(r, _)| inside(r))
                    || !inside(&h.card)
                        && self.hits.iter().any(|x| {
                            matches!(x.target, ClickTarget::ToggleNotifications)
                                && x.contains(mx, my)
                        }),
            ))
        } else if self.note_toast_hits.iter().any(|t| inside(&t.rect)) {
            Some(CursorIcon::Pointer)
        } else if let Some(ed) = &self.prompt_editor {
            Some(if ed.drag.as_ref().is_some_and(|d| d.live) {
                CursorIcon::Grabbing
            } else {
                hand(
                    self.prompt_editor_layout
                        .hits
                        .iter()
                        .any(|(r, _)| inside(r)),
                )
            })
        } else if self.chip_menu.is_some() {
            Some(hand(self.chip_menu_rows.iter().any(|(r, _)| inside(r))))
        } else if self.context_menu.open {
            Some(hand(
                self.context_menu_rects
                    .iter()
                    .zip(&self.context_menu.items)
                    .any(|(r, item)| item.enabled && inside(r)),
            ))
        } else if self.palette.open {
            Some(hand(
                self.palette_row_hits.iter().any(|(r, _)| inside(r))
                    || self.hits.iter().any(|h| {
                        matches!(h.target, ClickTarget::PaletteMode(_)) && inside(&h.rect)
                    }),
            ))
        } else if self.search.open && self.search_chips.iter().any(inside) {
            Some(CursorIcon::Pointer)
        } else if !self.settings.open
            && let Some(icon) = self.panel_cursor(&inside)
        {
            Some(icon)
        } else {
            let hit = self.hits.iter().find(|h| h.contains(mx, my));
            match hit.map(|h| &h.target) {
                // a handle is the gutter between panes: tall = side by side
                Some(ClickTarget::HandleDrag(_)) => {
                    let r = hit.map(|h| h.rect).unwrap_or_default();
                    Some(if r[2] < r[3] {
                        CursorIcon::ColResize
                    } else {
                        CursorIcon::RowResize
                    })
                }
                Some(
                    ClickTarget::BlockChevron(..)
                    | ClickTarget::BlockCopy(..)
                    | ClickTarget::TrafficLight(_)
                    | ClickTarget::TabSelect(_)
                    | ClickTarget::TabClose(_)
                    | ClickTarget::TabNew
                    | ClickTarget::Chip(..)
                    | ClickTarget::SettingsEditPrompt
                    | ClickTarget::PaneMenu(_)
                    | ClickTarget::PaneSplit(_)
                    | ClickTarget::Welcome(_)
                    | ClickTarget::CompletionSource(_)
                    | ClickTarget::CompletionRow(_)
                    | ClickTarget::HistoryScope(_)
                    | ClickTarget::HistoryRow(_)
                    | ClickTarget::HistoryClose
                    | ClickTarget::SettingsOption(_)
                    | ClickTarget::SettingsReset(_)
                    | ClickTarget::SettingsOpenConfig
                    | ClickTarget::SettingsCopyVersion
                    | ClickTarget::SettingsUpdate
                    | ClickTarget::UpdateChip
                    | ClickTarget::PaneClose(_)
                    | ClickTarget::ToggleExplorer
                    | ClickTarget::ToggleReview
                    | ClickTarget::ToggleNotifications
                    | ClickTarget::JumpToBlock(..)
                    | ClickTarget::SettingsNav(_)
                    | ClickTarget::SettingsClose
                    | ClickTarget::SettingsToggle(_)
                    | ClickTarget::SettingsStep(..)
                    | ClickTarget::SettingsSegment(..)
                    | ClickTarget::SettingsCycle(_)
                    | ClickTarget::SettingsSlider(_)
                    | ClickTarget::SettingsTheme(_)
                    | ClickTarget::SettingsKeybind(_)
                    | ClickTarget::SettingsKeybindReset(_),
                ) => Some(CursorIcon::Pointer),
                _ => None,
            }
        };
        let over_link = self.modifiers.super_key()
            && self.tabs.get(self.active).is_some_and(|t| {
                t.panes.iter().any(|p| {
                    p.url_hits.iter().any(|(r, _)| {
                        mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
                    })
                })
            });
        let over_path = overlay.is_none()
            && !over_link
            && self.modifiers.super_key()
            && self
                .pane_under_mouse()
                .is_some_and(|id| self.path_under_mouse(id).is_some());
        let icon = if let Some(icon) = overlay {
            icon
        } else if over_link || over_path {
            CursorIcon::Pointer
        } else if my > TAB_BAR_H
            && self.mouse.0 > self.rail_w()
            && !self.settings.open
            && !self.palette.open
        {
            CursorIcon::Text
        } else {
            CursorIcon::Default
        };
        if self.cursor_icon != icon {
            self.cursor_icon = icon;
            window.set_cursor(icon);
        }
    }

    /// Selected text of the active pane, if any.
    fn selection_text(&self) -> Option<String> {
        let pane = self.tabs.get(self.active)?.active()?;
        log::debug!("selection_text: has_selection={}", pane.has_selection);
        if !pane.has_selection {
            return None;
        }
        let text = pane.session.term.lock().selection_to_string();
        log::debug!("selection_text: got={:?}", text);
        text.filter(|t| !t.is_empty())
    }

    /// The system clipboard, opened on first use; a backend that cannot be
    /// opened is reported once per attempt.
    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(cb) => self.clipboard = Some(cb),
                Err(e) => {
                    log::warn!("clipboard unavailable: {e}");
                    self.toast(format!("Clipboard unavailable: {e}"));
                }
            }
        }
        self.clipboard.as_mut()
    }

    /// Copy `text`; `false` (after telling the user) when it failed.
    fn copy_to_clipboard(&mut self, text: String) -> bool {
        if self.headless {
            self.headless_clipboard = text;
            return true;
        }
        let Some(cb) = self.clipboard() else {
            return false;
        };
        match cb.set_text(text) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("clipboard set failed: {e}");
                self.toast(format!("Could not copy: {e}"));
                false
            }
        }
    }

    fn paste_from_clipboard(&mut self) {
        if self.headless {
            let text = self.headless_clipboard.clone();
            self.paste_text(&text);
            return;
        }
        let Some(cb) = self.clipboard() else {
            return;
        };
        let text = match cb.get_text() {
            Ok(text) => text,
            // empty or non-text clipboard: nothing to paste, not an error
            Err(arboard::Error::ContentNotAvailable) => return,
            Err(e) => {
                log::warn!("clipboard read failed: {e}");
                self.toast(format!("Could not paste: {e}"));
                return;
            }
        };
        self.paste_text(&text);
    }

    /// Paste `text` into the active pane: the owned editor when it is live,
    /// else the PTY (bracketed when the app asked for it; mirrored to
    /// synchronized panes).
    fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let editor_active = self.editor_active;
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            return;
        };
        if editor_active {
            let clean = input::sanitize_editor_paste(text);
            pane.editor.insert(&clean);
            pane.selected_block = None;
            self.update_completions();
            return;
        }
        let id = pane.session.id;
        // unbracketed: every newline presses Enter, so ask before sending
        // more than one line to any recipient without bracketed paste
        let unbracketed = self.raw_targets(id).into_iter().any(|t| {
            self.tabs
                .iter()
                .find_map(|tab| tab.pane(t))
                .is_some_and(|p| !bracketed_paste_on(p))
        });
        if unbracketed && self.config.confirm_multiline_paste && input::paste_submits_lines(text) {
            let commands = text.lines().map(str::to_string).collect();
            self.confirm = Some(ConfirmClose {
                opened: Instant::now(),
                action: ConfirmAction::Paste {
                    pane: id,
                    text: text.to_string(),
                },
                commands,
            });
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            return;
        }
        self.paste_raw(id, text);
    }

    /// Paste into `pane`'s program and its raw synchronized siblings, each
    /// framed for its own bracketed-paste mode.
    fn paste_raw(&mut self, pane: PaneId, text: &str) {
        for target in self.raw_targets(pane) {
            if let Some((_, tab)) = self.find_tab_of_pane(target)
                && let Some(p) = tab.pane_mut(target)
            {
                let bytes = input::paste_bytes(text, bracketed_paste_on(p));
                p.view_offset = 0;
                p.session.write(&bytes);
            }
        }
    }

    /// Files dropped on the window: their shell-quoted paths are pasted.
    fn drop_file(&mut self, path: &std::path::Path) {
        let mut text = input::shell_quote(&path.to_string_lossy());
        text.push(' ');
        self.paste_text(&text);
    }

    /// Re-run the search over the active pane and jump near the latest match.
    fn refresh_search(&mut self, jump: bool) {
        let Some(pane) = self.tabs.get(self.active).and_then(|t| t.active()) else {
            self.search.set_matches(terminal::Found::default());
            return;
        };
        let found = match self.search.matcher() {
            Some(m) => terminal::collect_matches(&pane.session, &m, search::MATCH_CAP),
            None => terminal::Found::default(),
        };
        self.search.set_matches(found);
        if jump && !self.search.matches.is_empty() {
            self.jump_to_match();
        }
    }

    fn jump_to_match(&mut self) {
        let Some((abs, _)) = self.search.current_anchor() else {
            return;
        };
        if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
            use shift_pty::alacritty_terminal::grid::Dimensions;
            use std::sync::atomic::Ordering;
            let scrolled = pane.session.scrolled.load(Ordering::Acquire);
            let screen_lines = pane.session.term.lock().grid().screen_lines() as u64;
            let newest = scrolled + screen_lines;
            pane.view_offset = (newest.saturating_sub(abs) as usize).saturating_sub(12);
        }
    }

    /// Search-mode keyboard handling.
    fn handle_search_key(&mut self, event: &input::KeyPress, mods: ModifiersState) {
        use winit::keyboard::{Key, NamedKey};
        let shift = mods.shift_key();
        // ⌥C match case, ⌥R regex (physical key: Option composes on macOS)
        if mods.alt_key() && !mods.super_key() {
            let key = match (&event.unmodified, &event.logical_key) {
                (Some(Key::Character(s)), _) | (None, Key::Character(s)) => s.to_lowercase(),
                _ => String::new(),
            };
            match key.as_str() {
                "c" => self.search.case_sensitive = !self.search.case_sensitive,
                "r" => self.search.regex = !self.search.regex,
                _ => return,
            }
            self.refresh_search(true);
            return;
        }
        // cmd chords: ⌘G / ⇧⌘G cycle matches; everything else is swallowed
        // (never appended to the query as text)
        if mods.super_key() {
            if let Key::Character(s) = &event.logical_key
                && s.to_ascii_lowercase() == "g"
            {
                if shift {
                    self.search.prev();
                } else {
                    self.search.next();
                }
                self.jump_to_match();
                self.blink_epoch = Instant::now();
            }
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.search.close();
                if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                    pane.view_offset = 0;
                }
            }
            Key::Named(NamedKey::Enter) => {
                if shift {
                    self.search.prev();
                } else {
                    self.search.next();
                }
                self.jump_to_match();
            }
            Key::Named(NamedKey::Backspace) => {
                self.search.query.pop();
                self.refresh_search(true);
            }
            Key::Named(NamedKey::Space) => {
                self.search.query.push(' ');
                self.refresh_search(true);
            }
            _ => {
                if let Some(text) = &event.text
                    && !text.is_empty()
                    && !text.chars().any(|c| c.is_control())
                {
                    self.search.query.push_str(text);
                    self.refresh_search(true);
                }
            }
        }
        self.blink_epoch = Instant::now();
    }

    /// Block-filter input: type to narrow, Enter keeps the filter, Esc (or
    /// the filter shortcut) clears it.
    fn handle_filter_key(&mut self, event: &input::KeyPress) {
        use winit::keyboard::{Key, NamedKey};
        let action = input::app_action(event, self.modifiers, &self.keymap);
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            self.filter_editing = false;
            self.search.close();
            return;
        };
        let Some((block, query)) = pane.filter.as_mut() else {
            self.filter_editing = false;
            self.search.close();
            return;
        };
        let block = *block;
        match &event.logical_key {
            _ if action == Some(AppAction::FilterBlock) => {
                pane.filter = None;
                self.filter_editing = false;
                self.search.close();
            }
            Key::Named(NamedKey::Escape) => {
                pane.filter = None;
                self.filter_editing = false;
                self.search.close();
            }
            Key::Named(NamedKey::Enter) => {
                if query.is_empty() {
                    pane.filter = None;
                }
                self.filter_editing = false;
                self.search.close();
            }
            Key::Named(NamedKey::Backspace) => {
                query.pop();
            }
            Key::Named(NamedKey::Space) => query.push(' '),
            _ => {
                if !self.modifiers.super_key()
                    && !self.modifiers.control_key()
                    && let Some(text) = &event.text
                    && !text.chars().any(|c| c.is_control())
                {
                    query.push_str(text);
                }
            }
        }
        let pane_id = pane.session.id;
        if self.filter_editing {
            // keep the filtered block's header in view while narrowing
            self.jump_to_block(pane_id, block);
        }
        self.blink_epoch = Instant::now();
    }

    /// Same list (and order) `SwitchTheme(i)` indexes: builtins, then user
    /// themes — the palette used to list builtins only.
    fn theme_names() -> Vec<String> {
        shift_core::theme::available_themes()
    }

    fn refresh_palette(&mut self) {
        use crate::views::palette::PaletteMode;
        let pane_history = self
            .tabs
            .get(self.active)
            .and_then(|t| t.active())
            .map(|p| p.editor.history.clone())
            .unwrap_or_default();
        let (items, recent) = match self.palette.mode {
            PaletteMode::Commands => {
                let launches: Vec<String> =
                    self.launch_configs.iter().map(|l| l.name.clone()).collect();
                let mut items = palette::base_items(&Self::theme_names(), &launches, &self.keymap);
                items.extend(palette::ssh_items(&self.ssh_profiles));
                items.extend(self.detached_items());
                (items, pane_history)
            }
            PaletteMode::History => {
                let mut all = (*self.shell_history).clone();
                all.extend(pane_history);
                (palette::history_items(&all), Vec::new())
            }
            PaletteMode::Sessions => (self.session_items(), Vec::new()),
            PaletteMode::PipeOutput => {
                let output = self
                    .pipe_target
                    .as_ref()
                    .map(|t| t.2.as_str())
                    .unwrap_or_default();
                self.palette_results = palette::pipe_items(&self.palette.query, output);
                self.palette.selected = 0;
                return;
            }
        };
        self.palette_results = palette::rank(&items, &recent, &self.palette.query);
        if self.palette.selected >= self.palette_results.len() {
            self.palette.selected = self.palette_results.len().saturating_sub(1);
        }
    }

    /// Session switcher rows: every tab, every pane, and the newest
    /// finished blocks of this window (titles carry what fuzzy search
    /// should match: names, directories, running jobs, commands).
    fn session_items(&self) -> Vec<palette::Item> {
        use palette::{Item, PaletteAction, Section};
        let mut items = Vec::new();
        let row = |title: String, id: String, section: Section, action: PaletteAction| Item {
            title,
            id,
            shortcut: Vec::new(),
            section,
            action,
            mono_title: false,
        };
        for (ti, tab) in self.tabs.iter().enumerate() {
            let panes = if tab.is_split() { tab.panes.len() } else { 1 };
            items.push(row(
                format!("{} — tab {}", tab.display_title(), ti + 1),
                if ti == self.active {
                    "current".into()
                } else if panes > 1 {
                    format!("{panes} panes")
                } else {
                    String::new()
                },
                Section::Tabs,
                PaletteAction::FocusTab(ti),
            ));
        }
        for (ti, tab) in self.tabs.iter().enumerate() {
            if !tab.is_split() {
                continue;
            }
            for p in &tab.panes {
                let cwd = p
                    .session
                    .cwd
                    .lock()
                    .unwrap()
                    .as_deref()
                    .map(editor::shorten_home)
                    .unwrap_or_default();
                let job = p
                    .session
                    .has_foreground_job()
                    .then(|| p.session.foreground_job_name())
                    .flatten();
                let title = match job {
                    Some(j) => format!("{cwd} — {j}"),
                    None => cwd,
                };
                items.push(row(
                    title,
                    format!("tab {}", ti + 1),
                    Section::Panes,
                    PaletteAction::FocusPane(ti, p.session.id),
                ));
            }
        }
        let mut blocks = Vec::new();
        for (ti, tab) in self.tabs.iter().enumerate() {
            for p in &tab.panes {
                for b in p.session.blocks.lock().unwrap().snapshot() {
                    if let shift_core::BlockState::Done { exit, .. } = b.state
                        && !b.command.trim().is_empty()
                    {
                        blocks.push((b.id, ti, p.session.id, b.command, exit, b.cwd));
                    }
                }
            }
        }
        // tabs of other Shift windows (their control sockets)
        for (pid, _, tabs) in &self.other_windows {
            for t in tabs {
                let pane = t.panes.iter().find(|p| p.active).or(t.panes.first());
                let cwd = pane
                    .and_then(|p| p.cwd.as_deref())
                    .map(editor::shorten_home)
                    .unwrap_or_default();
                items.push(row(
                    format!("{} — other window", t.title),
                    cwd,
                    Section::Tabs,
                    PaletteAction::FocusWindow(*pid, t.index, pane.map(|p| p.id).unwrap_or(0)),
                ));
            }
        }
        items.extend(self.detached_items());
        // newest first across panes (ids are per pane; interleave by id)
        blocks.sort_by_key(|a| std::cmp::Reverse(a.0));
        for (id, ti, pane, command, exit, cwd) in blocks.into_iter().take(40) {
            let status = match exit {
                Some(0) | None => String::new(),
                Some(c) => format!("exit {c} · "),
            };
            let dir = cwd.as_deref().map(editor::shorten_home).unwrap_or_default();
            items.push(row(
                command,
                format!("{status}{dir} · tab {}", ti + 1),
                Section::Blocks,
                PaletteAction::ShowBlock(ti, pane, id),
            ));
        }
        items
    }

    /// The pane is on screen and the window has focus.
    fn pane_in_view(&self, pane: PaneId) -> bool {
        self.focused
            && self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.pane(pane).is_some() && t.zoomed.is_none_or(|z| z == pane))
    }

    /// Coding agent running in a pane: the running block's command, else the
    /// foreground job (durable sessions, shells without integration).
    fn pane_agent(p: &PaneState) -> Option<shift_core::notifications::Agent> {
        use shift_core::notifications::Agent;
        let running = p
            .session
            .blocks
            .lock()
            .unwrap()
            .last()
            .filter(|b| matches!(b.state, BlockState::Running { .. }))
            .map(|b| b.command.clone());
        running
            .as_deref()
            .and_then(Agent::from_command)
            .or_else(|| {
                p.session
                    .foreground_job_name()
                    .as_deref()
                    .and_then(Agent::from_command)
            })
    }

    /// Record an inbox entry (id, agent and branch filled here); returns its id.
    fn push_attention(&mut self, mut a: Attention) -> u64 {
        let pane = self.tabs.iter().find_map(|t| t.pane(a.pane));
        a.branch = pane
            .and_then(|p| p.git.as_ref())
            .map(|g| g.branch.clone())
            .filter(|b| !b.is_empty());
        if a.agent.is_none() {
            a.agent = pane.and_then(Self::pane_agent);
        }
        self.next_attention_id += 1;
        a.id = self.next_attention_id;
        let id = a.id;
        self.attention.push(a);
        if self.attention.len() > ATTENTION_CAP {
            let extra = self.attention.len() - ATTENTION_CAP;
            self.attention.drain(..extra);
        }
        id
    }

    /// Inbox entry plus, for an `event` whose pane is out of view, the
    /// alerts `[notifications]` allows: a toast while Shift is focused, a
    /// desktop notification otherwise, and the event's sound.
    fn raise(&mut self, mut a: Attention, event: Option<shift_core::notifications::Event>) {
        let in_view = self.pane_in_view(a.pane);
        a.read = a.read || in_view;
        let id = self.push_attention(a);
        let Some(event) = event else {
            return;
        };
        let cfg = self.config.notifications.clone();
        if in_view || !cfg.enabled {
            return;
        }
        let Some(a) = self.attention.iter().find(|a| a.id == id) else {
            return;
        };
        let (title, body) = a.desktop_text();
        log::info!("alert {event:?}: {title} — {body}");
        if self.focused {
            if cfg.toasts {
                self.note_toasts.insert(0, (id, Instant::now()));
                self.note_toasts
                    .truncate(crate::views::notifications::MAX_TOASTS);
            }
        } else {
            crate::notify::send(&title, &body);
        }
        if cfg.play_sound
            && let Some(path) = shift_core::notifications::resolve_sound(
                cfg.sound_setting(event),
                event,
                &shift_core::notifications::sound_dirs(),
            )
        {
            crate::notify::play(&path);
        }
        self.a11y_announcement = Some(format!("{title}: {body}"));
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Settings → Notifications sound pickers: step to the next sound and
    /// play it (the preview).
    /// A Settings select's options as (config value, label), and the
    /// current value.
    fn select_options(&self, id: &str) -> (Vec<(String, String)>, String) {
        use shift_core::notifications::{available_sounds, sound_dirs};
        let n = &self.config.notifications;
        match id {
            "theme" | "theme_light" => {
                let current = if id == "theme" {
                    &self.config.theme
                } else {
                    &self.config.theme_light
                };
                let names = shift_core::theme::available_themes();
                let labels = shift_core::theme::picker_labels(&names);
                let options = names.into_iter().zip(labels).collect();
                (options, current.clone())
            }
            "font_family" => (
                self.text
                    .as_ref()
                    .map(|t| t.monospace_families())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|f| (f.clone(), f))
                    .collect(),
                self.config.font_family.clone(),
            ),
            "link_editor" | "file_editor" => {
                use shift_core::editors;
                let current = if id == "link_editor" {
                    &self.config.link_editor
                } else {
                    &self.config.file_editor
                };
                let mut options: Vec<(String, String)> = [editors::SYSTEM, editors::SHIFT]
                    .into_iter()
                    .chain(editors::installed().into_iter().map(|e| e.id))
                    .map(|id| (id.to_string(), editors::label(id)))
                    .collect();
                // an editor set in config.toml but not installed stays listed
                let current = current.trim().to_ascii_lowercase();
                if !current.is_empty() && !options.iter().any(|(v, _)| *v == current) {
                    options.push((current.clone(), editors::label(&current)));
                }
                (options, current)
            }
            _ if id.starts_with("notifications.sound") => {
                let mut names = available_sounds(&sound_dirs());
                if id != "notifications.sound" {
                    // per-event pickers start with "Same as Sound"
                    names.insert(0, String::new());
                }
                let current = match id {
                    "notifications.sound_finished" => &n.sound_finished,
                    "notifications.sound_failed" => &n.sound_failed,
                    "notifications.sound_attention" => &n.sound_attention,
                    _ => &n.sound,
                };
                (
                    names
                        .into_iter()
                        .map(|s| {
                            let label = crate::views::settings::sound_label(&s);
                            (s, label)
                        })
                        .collect(),
                    current.trim().to_string(),
                )
            }
            _ => (Vec::new(), String::new()),
        }
    }

    /// Open select `id`'s list with the current value highlighted.
    fn open_select(&mut self, id: &'static str) {
        let (options, current) = self.select_options(id);
        if options.is_empty() {
            return;
        }
        let current = options
            .iter()
            .position(|(v, _)| v.eq_ignore_ascii_case(&current));
        self.settings.dropdown = Some(crate::views::settings::Dropdown {
            id,
            options,
            highlight: current.unwrap_or(0),
            current,
        });
    }

    /// Set select `id` to `value`; a sound plays so it can be heard.
    fn set_select(&mut self, id: &str, value: String) {
        use shift_core::notifications::{Event, resolve_sound, sound_dirs};
        let n = &mut self.config.notifications;
        let event = match id {
            "theme" => {
                self.config.theme = value;
                None
            }
            "theme_light" => {
                self.config.theme_light = value;
                None
            }
            "font_family" => {
                self.config.font_family = value;
                None
            }
            "link_editor" => {
                self.config.link_editor = value;
                None
            }
            "file_editor" => {
                self.config.file_editor = value;
                None
            }
            "notifications.sound_finished" => {
                n.sound_finished = value;
                Some(Event::Finished)
            }
            "notifications.sound_failed" => {
                n.sound_failed = value;
                Some(Event::Failed)
            }
            "notifications.sound_attention" => {
                n.sound_attention = value;
                Some(Event::Attention)
            }
            "notifications.sound" => {
                n.sound = value;
                Some(Event::Finished)
            }
            _ => return,
        };
        if let Some(event) = event {
            let setting = self.config.notifications.sound_setting(event).to_string();
            if let Some(path) = resolve_sound(&setting, event, &sound_dirs()) {
                crate::notify::play(&path);
            }
        }
        self.save_config();
        self.apply_config();
    }

    /// Drop expired notification toasts; the one under the mouse restarts
    /// its time. True when one went.
    fn prune_note_toasts(&mut self) -> bool {
        let ttl = self.config.notifications.toast_duration();
        let now = Instant::now();
        let (mx, my) = self.mouse;
        let hits = &self.note_toast_hits;
        for (id, since) in &mut self.note_toasts {
            let hovered = hits.iter().any(|h| {
                h.id == *id
                    && mx >= h.rect[0]
                    && mx < h.rect[0] + h.rect[2]
                    && my >= h.rect[1]
                    && my < h.rect[1] + h.rect[3]
            });
            if hovered {
                *since = now;
            }
        }
        let before = self.note_toasts.len();
        self.note_toasts
            .retain(|(_, since)| now.duration_since(*since) < ttl);
        before != self.note_toasts.len()
    }

    /// Entries whose pane you are looking at count as read; their toasts go.
    fn mark_visible_read(&mut self) {
        if !self.focused {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let visible =
            |pane: PaneId| tab.pane(pane).is_some() && tab.zoomed.is_none_or(|z| z == pane);
        let mut changed = false;
        for a in self.attention.iter_mut().filter(|a| !a.read) {
            if visible(a.pane) {
                a.read = true;
                changed = true;
            }
        }
        if changed || !self.note_toasts.is_empty() {
            let attention = &self.attention;
            self.note_toasts
                .retain(|(id, _)| attention.iter().any(|a| a.id == *id && !a.read));
        }
    }

    /// A running command in an out-of-view pane shows a password prompt
    /// (once per command).
    fn check_password_prompt(&mut self, pane: PaneId) {
        if self.pane_in_view(pane) {
            return;
        }
        let Some((_, tab)) = self.find_tab_of_pane(pane) else {
            return;
        };
        let Some(p) = tab.pane_mut(pane) else { return };
        let running = p
            .session
            .blocks
            .lock()
            .unwrap()
            .last()
            .filter(|b| matches!(b.state, BlockState::Running { .. }))
            .map(|b| b.id);
        let Some(block) = running else { return };
        if p.password_block == Some(block) {
            return;
        }
        let line = p.session.cursor_line_text();
        if !shift_core::notifications::looks_like_password_prompt(&line) {
            return;
        }
        p.password_block = Some(block);
        let mut a = Attention::new(
            pane,
            AttentionKind::Request,
            line.trim().to_string(),
            "Waiting for a password.".into(),
        );
        a.block = Some(block);
        let event = self
            .config
            .notifications
            .password_prompt
            .then_some(shift_core::notifications::Event::Attention);
        self.raise(a, event);
    }

    /// Commands that finished since last time: out of view ones (and every
    /// failure) go to the attention center.
    fn record_completions(&mut self, pane: PaneId) {
        let in_view = self.pane_in_view(pane);
        let Some(p) = self
            .tabs
            .iter()
            .flat_map(|t| &t.panes)
            .find(|p| p.session.id == pane)
        else {
            return;
        };
        let seen = self.attention_seen.get(&pane).copied();
        let fresh: Vec<(shift_core::BlockId, String, Option<i32>, Duration, bool)> = p
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .filter(|b| seen.is_none_or(|s| b.id > s))
            .filter_map(|b| match b.state {
                BlockState::Done { exit, duration } => {
                    Some((b.id, b.command.clone(), exit, duration, b.restored))
                }
                _ => None,
            })
            .collect();
        let Some(newest) = fresh.last().map(|f| f.0) else {
            return;
        };
        self.attention_seen.insert(pane, newest);
        // a finished command may have changed files: reload the panels
        self.request_review();
        use shift_core::notifications::Event;
        let long = Duration::from_secs(self.config.notify_threshold_secs);
        let long_on = self.config.notify_on_completion;
        let failed_on = self.config.notifications.command_failed;
        for (id, command, exit, duration, restored) in fresh {
            let failed = exit.is_some_and(|e| e != 0);
            if restored || command.trim().is_empty() || (in_view && !failed) {
                continue;
            }
            let is_long = long_on && duration >= long;
            let (kind, event) = if failed {
                (
                    AttentionKind::Failed,
                    (failed_on || is_long).then_some(Event::Failed),
                )
            } else {
                (AttentionKind::Finished, is_long.then_some(Event::Finished))
            };
            let detail = match exit {
                Some(0) | None => format!("Completed in {}.", terminal::format_duration(duration)),
                Some(c) => format!(
                    "Failed with exit {c} after {}.",
                    terminal::format_duration(duration)
                ),
            };
            let mut a = Attention::new(pane, kind, command, detail);
            a.block = Some(id);
            // the command itself is the agent that just exited, if any
            a.agent = shift_core::notifications::Agent::from_command(&a.title);
            self.raise(a, event);
        }
    }

    /// Notifications popover rows, newest first.
    fn notification_entries(&self) -> Vec<crate::views::notifications::Entry> {
        let now = std::time::SystemTime::now();
        self.attention
            .iter()
            .rev()
            .map(|a| Self::entry_of(a, now))
            .collect()
    }

    fn entry_of(a: &Attention, now: std::time::SystemTime) -> crate::views::notifications::Entry {
        crate::views::notifications::Entry {
            id: a.id,
            kind: a.kind,
            agent: a.agent,
            branch: a.branch.clone(),
            title: a.title.clone(),
            detail: a.detail.clone(),
            age_secs: now.duration_since(a.at).map(|d| d.as_secs()).unwrap_or(0),
            unread: !a.read,
        }
    }

    /// The popover's rows under its current filter.
    fn filtered_notifications(&self) -> Vec<crate::views::notifications::Entry> {
        let filter = self.notifications.filter;
        self.notification_entries()
            .into_iter()
            .filter(|e| filter.matches(e))
            .collect()
    }

    fn open_notifications(&mut self) {
        self.notifications.open();
        self.palette.open = false;
        self.context_menu.close();
    }

    fn close_notifications(&mut self) {
        self.notifications.open = false;
    }

    fn mark_all_read(&mut self) {
        for a in &mut self.attention {
            a.read = true;
        }
        self.note_toasts.clear();
    }

    /// Open an entry: mark it read, drop its toast, go to its pane (and block).
    fn show_attention(&mut self, id: u64) {
        let Some(a) = self.attention.iter_mut().find(|a| a.id == id) else {
            return;
        };
        a.read = true;
        self.settings.open = false;
        let a = a.clone();
        self.note_toasts.retain(|(t, _)| *t != id);
        let Some(ti) = self.tabs.iter().position(|t| t.pane(a.pane).is_some()) else {
            self.toast("That pane is closed".into());
            return;
        };
        self.active = ti;
        let tab = &mut self.tabs[ti];
        tab.activity = false;
        tab.active_pane = a.pane;
        if tab.zoomed.is_some() {
            tab.zoomed = Some(a.pane);
        }
        if let Some(block) = a.block {
            if let Some(p) = tab.pane_mut(a.pane) {
                p.selected_block = Some(block);
            }
            self.jump_to_block(a.pane, block);
        }
    }

    fn unread_attention(&self) -> usize {
        self.attention.iter().filter(|a| !a.read).count()
    }

    /// Palette rows for detached session-service shells.
    fn detached_items(&self) -> Vec<palette::Item> {
        self.detached
            .iter()
            .map(|s| palette::Item {
                title: format!(
                    "Attach detached session: {}{}",
                    s.cwd
                        .as_deref()
                        .map(editor::shorten_home)
                        .unwrap_or_default(),
                    s.job
                        .as_deref()
                        .map(|j| format!(" — {j}"))
                        .unwrap_or_default()
                ),
                id: format!("pid {}", s.pid),
                shortcut: Vec::new(),
                section: palette::Section::Panes,
                action: palette::PaletteAction::AttachService(s.id),
                mono_title: false,
            })
            .collect()
    }

    /// Show the block output → command modal for the selected block (or
    /// the newest finished one).
    fn open_pipe(&mut self) {
        let Some(pane) = self.tabs.get(self.active).and_then(|t| t.active()) else {
            return;
        };
        let block = pane.selected_block.or_else(|| {
            pane.session
                .blocks
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|b| b.output_range().is_some())
                .map(|b| b.id)
        });
        let Some(block) = block else {
            self.toast("No block output to send".into());
            return;
        };
        let output = terminal::block_output_text(&pane.session, block).unwrap_or_default();
        self.pipe_target = Some((pane.session.id, block, output));
        self.open_palette(crate::views::palette::PaletteMode::PipeOutput);
    }

    /// Run the pipe modal's command with the block output as its stdin.
    fn run_pipe(&mut self) {
        let command = self.palette.query.trim().to_string();
        let Some((pane, _, output)) = self.pipe_target.take() else {
            return;
        };
        if command.is_empty() {
            return;
        }
        let Some(file) = write_pipe_file(&output) else {
            self.toast("Could not write the output to a temporary file".into());
            return;
        };
        let shell = shift_core::shell_integration::shell_name(self.config.shell.as_deref());
        let line = palette::pipe_command(&command, &file.to_string_lossy(), &shell);
        self.submit_to(pane, &line);
    }

    /// Keys go to the viewer: it is open in this tab and focused.
    fn viewer_focused(&self) -> bool {
        self.viewer_focus
            && self.viewer.is_open()
            && self
                .tabs
                .get(self.active)
                .is_some_and(|t| t.root.contains(VIEWER_PANE))
    }

    /// Keep the viewer state and its pane leaf in step: a closed viewer
    /// leaves the split tree, a leaf whose tab went away closes the viewer.
    fn sync_viewer_pane(&mut self) {
        if self.viewer.is_open() {
            if !self.tabs.iter().any(|t| t.root.contains(VIEWER_PANE)) {
                self.viewer.close();
            }
        } else {
            for tab in &mut self.tabs {
                if tab.root.contains(VIEWER_PANE) {
                    tab.root.remove(VIEWER_PANE);
                }
            }
        }
        if !self.viewer.is_open() {
            self.viewer_focus = false;
        }
    }

    /// Open `path` in the viewer pane: the tab's leftmost column, next to the
    /// explorer, reused when this tab already has one.
    fn open_viewer(&mut self, path: &std::path::Path) {
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if i != self.active && tab.root.contains(VIEWER_PANE) {
                tab.root.remove(VIEWER_PANE);
            }
        }
        if let Some(tab) = self.tabs.get_mut(self.active)
            && !tab.root.contains(VIEWER_PANE)
        {
            // the leftmost column, next to the explorer, taking an equal
            // share with the shell panes side by side
            tab.zoomed = None;
            let columns = tab.root.leaves().len() as f32;
            tab.root = crate::pane::PaneNode::Split {
                axis: Axis::Horizontal,
                ratio: (1.0 / (columns + 1.0)).clamp(0.25, 0.5),
                first: Box::new(crate::pane::PaneNode::Leaf(VIEWER_PANE)),
                second: Box::new(tab.root.clone()),
            };
        }
        self.viewer_focus = true;
        let proxy = self.proxy.clone();
        self.viewer.open(path, move |loaded| {
            let _ = proxy.send_event(UserEvent::ViewerLoaded(Box::new(loaded)));
        });
    }

    /// ⌃R: the history pane above the command line (again: next scope).
    /// Rows: this window's finished commands (newest first, with status,
    /// directory and age), then the imported shell history.
    fn open_history(&mut self) {
        use crate::views::history::{Entry, dedupe};
        if self.history.open {
            let scope = self.history.scope.next();
            self.history.set_scope(scope);
            return;
        }
        let active = self.active_pane_id();
        let mut entries: Vec<Entry> = Vec::new();
        for tab in &self.tabs {
            for p in &tab.panes {
                let here = Some(p.session.id) == active;
                let blocks = p.session.blocks.lock().unwrap();
                for b in blocks.iter() {
                    if matches!(b.state, shift_core::BlockState::Done { .. }) {
                        entries.push(Entry {
                            command: b.command.clone(),
                            cwd: b.cwd.as_deref().map(editor::shorten_home),
                            exit: b.exit_code(),
                            finished: b.finished_at,
                            here,
                        });
                    }
                }
            }
        }
        // newest first: finish time, unknown (replayed) last
        entries.sort_by_key(|e| std::cmp::Reverse(e.finished));
        // the pane's own typed history, then the shell's, newest first
        if let Some(p) = self.tabs.get(self.active).and_then(|t| t.active()) {
            entries.extend(p.editor.history.iter().rev().map(|c| Entry {
                command: c.clone(),
                cwd: None,
                exit: None,
                finished: None,
                here: true,
            }));
        }
        entries.extend(self.shell_history.iter().rev().map(|c| Entry {
            command: c.clone(),
            cwd: None,
            exit: None,
            finished: None,
            here: false,
        }));
        let query = self
            .tabs
            .get(self.active)
            .and_then(|t| t.active())
            .map(|p| p.editor.buffer.clone())
            .unwrap_or_default();
        // no completion answer in flight may reopen the panel under it
        self.completion.close();
        self.completion_pending = None;
        self.completion_intent = None;
        self.history.open(dedupe(entries), &query);
        self.history.pane = active;
    }

    /// Keys for the open history pane; `false` lets the key edit the command
    /// line (the query).
    fn handle_history_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let mods = self.modifiers;
        // nothing matches: ⏎ runs the typed line as usual; ⇥ only closes the
        // pane, so completions show before a second ⇥ accepts one
        if self.history.current().is_none() {
            match event.logical_key {
                Key::Named(NamedKey::Enter) => {
                    self.history.close();
                    return false;
                }
                Key::Named(NamedKey::Tab) => {
                    self.history.close();
                    self.update_completions();
                    return true;
                }
                _ => {}
            }
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.history.close(),
            Key::Named(NamedKey::ArrowUp) => self.history.move_by(-1),
            Key::Named(NamedKey::ArrowDown) => self.history.move_by(1),
            Key::Named(NamedKey::PageUp) => self.history.move_by(-8),
            Key::Named(NamedKey::PageDown) => self.history.move_by(8),
            Key::Named(NamedKey::Tab) => self.take_history(HistoryUse::Edit),
            Key::Named(NamedKey::Enter)
                if mods.super_key()
                    && !mods.shift_key()
                    && !mods.control_key()
                    && !mods.alt_key() =>
            {
                self.take_history(HistoryUse::RunInSplit)
            }
            Key::Named(NamedKey::Enter)
                if !mods.shift_key()
                    && !mods.alt_key()
                    && !mods.control_key()
                    && !mods.super_key() =>
            {
                self.take_history(HistoryUse::Run)
            }
            Key::Character(c) if mods.control_key() && c.eq_ignore_ascii_case("r") => {
                self.open_history()
            }
            _ => return false,
        }
        true
    }

    /// Use the selected history row: run it, put it on the command line, or
    /// run it in a new split beside the pane.
    fn take_history(&mut self, how: HistoryUse) {
        let Some(command) = self.history.current().map(|e| e.command.clone()) else {
            return;
        };
        self.history.close();
        match how {
            HistoryUse::RunInSplit => {
                let Some(target) = self.active_pane_id() else {
                    return;
                };
                // the query was only a search: this pane's line empties
                if let Some(p) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                    let len = p.editor.buffer.len();
                    p.editor.replace_range(0..len, "");
                }
                self.completion.close();
                let cwd = self
                    .tabs
                    .get(self.active)
                    .and_then(|t| t.active())
                    .and_then(|p| p.session.cwd.lock().unwrap().clone())
                    .map(std::path::PathBuf::from);
                let new = self.split_pane(target, Axis::Horizontal, cwd);
                self.pending_commands.insert(new, vec![command]);
            }
            HistoryUse::Edit | HistoryUse::Run => {
                if let Some(p) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                    let len = p.editor.buffer.len();
                    p.editor.replace_range(0..len, &command);
                }
                self.completion.close();
                if how == HistoryUse::Run {
                    self.handle_editor_key(EditorKey::Submit);
                } else {
                    self.update_completions();
                }
                self.blink_epoch = Instant::now();
            }
        }
    }

    /// Follow the command line: it is the history pane's query. The pane
    /// closes when the line is no longer the app's (a program took over).
    fn sync_history(&mut self) {
        if !self.history.open {
            return;
        }
        let Some(p) = self.tabs.get(self.active).and_then(|t| t.active()) else {
            self.history.close();
            return;
        };
        if !Self::editor_visible(p)
            || self.history.pane != Some(p.session.id)
            || self.viewer_focused()
            || self.settings.open
            || self.palette.open
            || self.search.open
        {
            self.history.close();
        } else if p.editor.buffer != self.history.query {
            let q = p.editor.buffer.clone();
            self.history.filter(&q);
        }
    }

    fn open_palette(&mut self, mode: crate::views::palette::PaletteMode) {
        if mode == crate::views::palette::PaletteMode::Commands {
            // pick up files added while running
            self.launch_configs = shift_core::launch::load_all();
            self.ssh_profiles = shift_core::ssh::all_profiles(&self.config.ssh_profiles);
        }
        if mode == crate::views::palette::PaletteMode::Sessions && !self.windows_loading {
            self.windows_loading = true;
            self.other_windows.clear();
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                crate::control::visit_other_windows(|window| {
                    let _ = proxy.send_event(UserEvent::WindowsLoaded(vec![window], false));
                });
                let _ = proxy.send_event(UserEvent::WindowsLoaded(Vec::new(), true));
            });
        }
        if matches!(
            mode,
            crate::views::palette::PaletteMode::Sessions
                | crate::views::palette::PaletteMode::Commands
        ) && !self.detached_loading
        {
            // shells the service keeps that no window shows
            self.detached_loading = true;
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                let detached = shift_pty::service::list()
                    .into_iter()
                    .filter(|s| s.clients.is_empty())
                    .collect();
                let _ = proxy.send_event(UserEvent::DetachedLoaded(detached));
            });
        }
        self.palette.open = true;
        self.palette.mode = mode;
        self.palette.query.clear();
        self.palette.selected = 0;
        self.refresh_palette();
        self.blink_epoch = Instant::now();
    }

    fn run_palette_action(&mut self, action: PaletteAction, event_loop: &ActiveEventLoop) {
        self.palette.open = false;
        match action {
            PaletteAction::App(a) => self.handle_action(a, event_loop),
            PaletteAction::OpenSettings => {
                self.settings.open_page(self.settings.page);
            }
            PaletteAction::SwitchTheme(i) => {
                let names = shift_core::theme::available_themes();
                if let Some(name) = names.into_iter().nth(i) {
                    self.config.theme = name;
                    self.save_config();
                    self.apply_config();
                }
            }
            PaletteAction::Search => {
                self.search.open = true;
                self.refresh_search(true);
            }
            PaletteAction::OpenHistory => {
                self.open_palette(crate::views::palette::PaletteMode::History);
            }
            PaletteAction::ToggleVim => {
                self.config.vim_mode = !self.config.vim_mode;
                self.save_config();
                self.apply_config();
            }
            PaletteAction::Launch(i) => {
                if let Some(cfg) = self.launch_configs.get(i).cloned() {
                    self.apply_launch_config(&cfg);
                }
            }
            PaletteAction::InsertCommand(cmd) => {
                if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                    pane.editor.clear();
                    pane.editor.insert(&cmd);
                }
                self.update_completions();
            }
            PaletteAction::FocusTab(i) => {
                self.settings.open = false;
                if i < self.tabs.len() {
                    self.active = i;
                    self.tabs[i].activity = false;
                }
            }
            PaletteAction::FocusPane(ti, pane) => {
                self.settings.open = false;
                if let Some(tab) = self.tabs.get_mut(ti) {
                    self.active = ti;
                    tab.activity = false;
                    if tab.pane(pane).is_some() {
                        tab.active_pane = pane;
                        if tab.zoomed.is_some() {
                            tab.zoomed = Some(pane);
                        }
                    }
                }
            }
            PaletteAction::ShowBlock(ti, pane, block) => {
                self.settings.open = false;
                if let Some(tab) = self.tabs.get_mut(ti)
                    && tab.pane(pane).is_some()
                {
                    self.active = ti;
                    tab.activity = false;
                    tab.active_pane = pane;
                    if let Some(p) = tab.pane_mut(pane) {
                        p.selected_block = Some(block);
                    }
                    self.jump_to_block(pane, block);
                }
            }
            PaletteAction::RunPipe => self.run_pipe(),
            PaletteAction::AttachService(id) => {
                self.next_attach = Some(id);
                self.spawn_tab_at(None);
            }
            PaletteAction::FocusWindow(pid, tab, pane) => {
                let sock = crate::control::dir().map(|d| d.join(format!("{pid}.sock")));
                let r = sock.map(|s| {
                    crate::control::request(
                        &s,
                        &crate::control::Request::Focus {
                            tab: Some(tab),
                            pane: Some(pane),
                        },
                    )
                });
                if !matches!(r, Some(Ok(ref resp)) if resp.ok) {
                    self.toast("That window is no longer running".into());
                }
            }
            PaletteAction::Ssh(i) => {
                if let Some(p) = self.ssh_profiles.get(i).cloned() {
                    self.open_tab_running(&p.name, p.command());
                }
            }
            PaletteAction::SshInstall(i) => {
                if let Some(p) = self.ssh_profiles.get(i).cloned() {
                    self.confirm = Some(ConfirmClose {
                        opened: Instant::now(),
                        commands: vec![
                            "~/.local/share/shift/shift.{bash,zsh,fish}".into(),
                            format!(
                                "~/.bashrc, ~/.zshrc: one line ending {}",
                                shift_core::ssh::RC_MARKER
                            ),
                            "~/.config/fish/conf.d/shift.fish".into(),
                        ],
                        action: ConfirmAction::InstallRemote(p),
                    });
                }
            }
            PaletteAction::RunHistory(cmd) => {
                // same path as Enter: prompt-time context, synced panes
                if let Some(id) = self.tabs.get(self.active).map(|t| t.active_pane) {
                    for target in self.input_targets(id) {
                        self.submit_to(target, &cmd);
                    }
                }
            }
        }
    }

    /// Palette-mode keyboard handling. Returns true when consumed.
    fn handle_palette_key(
        &mut self,
        event: &input::KeyPress,
        event_loop: &ActiveEventLoop,
    ) -> bool {
        use crate::views::palette::PaletteMode;
        use winit::keyboard::{Key, NamedKey};
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.palette.open = false;
            }
            Key::Named(NamedKey::Enter) => {
                if let Some(m) = self.palette_results.get(self.palette.selected) {
                    let action = m.item.action.clone();
                    self.run_palette_action(action, event_loop);
                }
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.palette.selected = self.palette.selected.saturating_sub(1);
            }
            Key::Named(NamedKey::ArrowDown) => {
                if self.palette.selected + 1 < self.palette_results.len() {
                    self.palette.selected += 1;
                }
            }
            Key::Named(NamedKey::Tab) if self.palette.mode != PaletteMode::PipeOutput => {
                let next = crate::views::palette::next_mode(self.palette.mode);
                self.open_palette(next);
            }
            Key::Named(NamedKey::Backspace) => {
                self.palette.query.pop();
                self.palette.selected = 0;
                self.refresh_palette();
            }
            Key::Named(NamedKey::Space) => {
                self.palette.query.push(' ');
                self.palette.selected = 0;
                self.refresh_palette();
            }
            _ => {
                if let Some(text) = &event.text
                    && !text.is_empty()
                    && !text.chars().any(|c| c.is_control())
                {
                    // a prefix typed into an empty query switches mode
                    let prefix = text
                        .chars()
                        .next()
                        .filter(|_| {
                            self.palette.query.is_empty()
                                && text.chars().count() == 1
                                && self.palette.mode != PaletteMode::PipeOutput
                        })
                        .and_then(crate::views::palette::mode_for_prefix)
                        .filter(|m| *m != self.palette.mode);
                    if let Some(mode) = prefix {
                        self.open_palette(mode);
                    } else {
                        self.palette.query.push_str(text);
                        self.palette.selected = 0;
                        self.refresh_palette();
                    }
                }
            }
        }
        self.blink_epoch = Instant::now();
        true
    }

    /// Recompute completions + ghost for the active editor. Providers read
    /// directories and repositories, so they run on the completion thread;
    /// the result arrives as `UserEvent::Completions`. Until then the ghost
    /// only shrinks by what was typed into it.
    fn update_completions(&mut self) {
        // a deferred Tab / ⌃→ belonged to the editor state being replaced
        self.completion_intent = None;
        let Some(pane) = self.tabs.get(self.active).and_then(|t| t.active()) else {
            self.completion_pending = None;
            self.completion.close();
            return;
        };
        let buffer = pane.editor.buffer.clone();
        let cursor = pane.editor.cursor;
        if buffer.trim().is_empty() {
            self.completion_pending = None;
            self.completion.close();
            return;
        }
        // the lookup uses at most the newest 200 pane entries
        let history = pane
            .editor
            .history
            .iter()
            .rev()
            .take(200)
            .rev()
            .cloned()
            .collect();
        let cwd = pane.session.cwd.lock().unwrap().clone();
        let pane_id = pane.session.id;
        let typed = buffer
            .strip_prefix(self.completion_buffer.as_str())
            .filter(|_| cursor == buffer.len());
        match typed {
            Some(t) if self.completion.ghost.starts_with(t) => {
                self.completion.ghost.drain(..t.len());
            }
            _ => self.completion.ghost.clear(),
        }
        self.completion_gen += 1;
        let request = CompletionRequest {
            generation: self.completion_gen,
            pane: pane_id,
            buffer,
            cursor,
            history,
            shell_history: self.shell_history.clone(),
            cwd,
        };
        let proxy = self.proxy.clone();
        let spawn = move || {
            let (tx, rx) = std::sync::mpsc::channel::<CompletionRequest>();
            let spawned = std::thread::Builder::new()
                .name("completion".into())
                .spawn(move || {
                    while let Ok(mut req) = rx.recv() {
                        // only the newest request matters
                        while let Ok(newer) = rx.try_recv() {
                            req = newer;
                        }
                        let items = req.run();
                        let _ = proxy.send_event(UserEvent::Completions(req.generation, items));
                    }
                });
            match spawned {
                Ok(_) => Some(tx),
                Err(e) => {
                    log::warn!("completion thread: {e}");
                    None
                }
            }
        };
        if self
            .completion_worker
            .send(request.clone(), Instant::now(), spawn)
        {
            self.completion_pending = Some(request);
        } else {
            // completion is paused; editing is not
            self.completion_pending = None;
            self.completion.close();
        }
    }

    /// Keys that read the menu or ghost (Tab, ⌃→) must see the answer to
    /// the current editor state. Providers never run on this thread, so
    /// while that answer is in flight the key is remembered and applied
    /// when it arrives; true when deferred.
    fn defer_completion_key(&mut self, intent: CompletionIntent) -> bool {
        if self.completion_pending.is_none() {
            return false;
        }
        self.completion_intent = Some(intent);
        true
    }

    /// Whether the active editor still holds exactly what `request` asked about.
    fn editing(&self, request: &CompletionRequest) -> bool {
        self.tabs
            .get(self.active)
            .and_then(|t| t.active())
            .is_some_and(|p| {
                p.session.id == request.pane
                    && p.editor.cursor == request.cursor
                    && p.editor.buffer == request.buffer
            })
    }

    /// A completion answer from the worker: applied only when it answers
    /// the latest request and the editor has not changed since.
    fn on_completions(&mut self, generation: u64, items: Vec<shift_core::completion::Completion>) {
        if self
            .completion_pending
            .as_ref()
            .is_none_or(|r| r.generation != generation)
        {
            return; // superseded by a newer request
        }
        let Some(request) = self.completion_pending.take() else {
            return;
        };
        let intent = self.completion_intent.take();
        if !self.editing(&request) {
            return;
        }
        self.apply_completions(&request.buffer, request.cursor, items);
        match intent {
            Some(CompletionIntent::Accept) => self.accept_completion(),
            Some(CompletionIntent::AcceptGhostWord) => self.accept_ghost_word(),
            None => {}
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn apply_completions(
        &mut self,
        buffer: &str,
        cursor: usize,
        items: Vec<shift_core::completion::Completion>,
    ) {
        self.completion_buffer = buffer.to_string();
        // the history pane owns the line while it is open
        if self.history.open {
            self.completion.close();
            return;
        }
        // ghost: best history match remainder, else the single completion
        self.completion.ghost = items
            .iter()
            .find(|c| c.source == "history")
            .and_then(|c| c.insert.strip_prefix(buffer).map(str::to_string))
            .unwrap_or_default();

        // the panel lists the token candidates, then the history lines (the
        // best history line also drives the ghost text)
        let (mut dropdown, history): (Vec<_>, Vec<_>) =
            items.into_iter().partition(|c| c.source != "history");
        if self.completion.ghost.is_empty()
            && let Some(first) = dropdown.first()
        {
            let (_, token) = shift_core::completion::current_token(buffer, cursor);
            if cursor == buffer.len() {
                self.completion.ghost = first
                    .insert
                    .strip_prefix(token)
                    .unwrap_or_default()
                    .to_string();
            }
        }
        if !self.config.autosuggestions {
            self.completion.ghost.clear();
        }
        // history lines join a panel the word candidates open; alone they
        // stay the ghost text (no panel on every keystroke)
        let show = self.config.completions_menu && !dropdown.is_empty();
        dropdown.extend(history);
        self.completion
            .set(if show { dropdown } else { Vec::new() });
    }

    /// ⌃→: accept one word of the ghost suggestion.
    fn accept_ghost_word(&mut self) {
        if self.defer_completion_key(CompletionIntent::AcceptGhostWord) {
            return;
        }
        if self.completion.ghost.is_empty() {
            return;
        }
        let ghost = self.completion.ghost.clone();
        // word = leading whitespace + next non-space run
        let trimmed = ghost.trim_start();
        let lead = ghost.len() - trimmed.len();
        let word_end = trimmed
            .find(char::is_whitespace)
            .map(|i| lead + i)
            .unwrap_or(ghost.len());
        let word = &ghost[..word_end];
        if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
            pane.editor.end();
            pane.editor.insert(word);
        }
        self.update_completions();
    }

    /// Accept the selected completion (Tab).
    fn accept_completion(&mut self) {
        if self.defer_completion_key(CompletionIntent::Accept) {
            return;
        }
        let (replacement, ghost) = if self.completion.open {
            (
                self.completion
                    .items
                    .get(self.completion.selected)
                    .map(|c| (c.insert.clone(), c.source == "history")),
                None,
            )
        } else if !self.completion.ghost.is_empty() {
            (None, Some(self.completion.ghost.clone()))
        } else {
            return;
        };
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            return;
        };
        if let Some(ghost) = ghost {
            pane.editor.end();
            pane.editor.insert(&ghost);
        } else if let Some((line, true)) = replacement {
            // a history line replaces the whole command line
            let len = pane.editor.buffer.len();
            pane.editor.replace_range(0..len, &line);
        } else if let Some((insert, false)) = replacement {
            let buffer = pane.editor.buffer.clone();
            let cursor = pane.editor.cursor;
            let (start, _) = shift_core::completion::current_token(&buffer, cursor);
            // the whole word: a closing quote or the suffix right of a
            // mid-word cursor would otherwise survive next to `insert`
            let end = shift_core::completion::token_end(&buffer, cursor);
            // through the single mutation point: makes the completion
            // undoable
            pane.editor.replace_range(start..end, &insert);
        }
        self.completion.close();
        self.update_completions();
    }

    fn logical_size(&self) -> (f32, f32) {
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        let gpu = self.gpu.as_ref().unwrap();
        (
            gpu.config.width as f32 / scale,
            gpu.config.height as f32 / scale,
        )
    }

    /// Editor is shown when shell integration is live (last block is a
    /// prompt) and no full-screen app is running.
    fn editor_visible(pane: &PaneState) -> bool {
        use shift_pty::alacritty_terminal::term::TermMode;
        let alt = pane
            .session
            .term
            .lock()
            .mode()
            .contains(TermMode::ALT_SCREEN);
        if alt {
            return false;
        }
        matches!(
            pane.session
                .blocks
                .lock()
                .unwrap()
                .last()
                .map(|b| b.state.clone()),
            Some(BlockState::Prompt)
        )
    }

    fn spawn_session(
        &mut self,
        cols: usize,
        rows: usize,
        cwd: Option<std::path::PathBuf>,
    ) -> Session {
        let id = self.next_pane;
        self.next_pane += 1;

        let proxy = self.proxy.clone();
        let observer: shift_pty::Observer = Arc::new(move |pane, event| {
            let _ = match event {
                SessionEvent::Wakeup => proxy.send_event(UserEvent::PtyWakeup(pane)),
                SessionEvent::Title(t) => proxy.send_event(UserEvent::PtyTitle(pane, t)),
                SessionEvent::ResetTitle => {
                    proxy.send_event(UserEvent::PtyTitle(pane, String::new()))
                }
                SessionEvent::Clipboard(text) => proxy.send_event(UserEvent::PtyClipboard(text)),
                SessionEvent::ClipboardLoad(reply) => {
                    proxy.send_event(UserEvent::PtyClipboardLoad(pane, reply))
                }
                SessionEvent::ColorRequest(index, reply) => {
                    proxy.send_event(UserEvent::PtyColorRequest(pane, index, reply))
                }
                SessionEvent::WindowOp(params) => {
                    proxy.send_event(UserEvent::PtyWindowOp(pane, params))
                }
                SessionEvent::Exit => proxy.send_event(UserEvent::PtyExit(pane)),
                SessionEvent::Bell => proxy.send_event(UserEvent::PtyBell(pane)),
                SessionEvent::Notify { title, body } => {
                    proxy.send_event(UserEvent::PtyNotify(pane, title, body))
                }
                SessionEvent::Notice(message) => proxy.send_event(UserEvent::PtyNotice(message)),
            };
        });
        // durable sessions: reattach a restored shell, else start one in
        // the session service; fall back to a local shell if it fails
        let attach = self.next_attach.take();
        let durable = self.config.durable_sessions && self.durable_allowed;
        let service = durable.then_some(match attach {
            Some(id) => shift_pty::ServiceTarget::Attach(id),
            None => shift_pty::ServiceTarget::Spawn,
        });
        // a reattached shell replays its own output; no transcript on top
        let replay = self
            .next_replay
            .take()
            .filter(|_| self.config.save_transcripts && attach.is_none())
            .and_then(|id| shift_core::transcript::load(&id))
            .map(|t| (t.replay_bytes(), t.meta()));
        let (cell_size, shell, scrollback) = (
            self.cell_px(),
            self.config.shell.clone(),
            self.config.scrollback_lines(),
        );
        let clipboard = shift_pty::ClipboardAccess::from_config(&self.config.clipboard_access);
        let opts = |service, replay| SpawnOptions {
            id,
            size: GridSize { cols, rows },
            cell_size,
            shell: shell.clone(),
            working_directory: cwd.clone(),
            env: vec![],
            scrollback,
            observer: observer.clone(),
            replay,
            service,
            clipboard,
        };
        match Session::spawn(opts(service, replay.clone())) {
            Ok(s) => s,
            Err(e) if service.is_some() => {
                log::warn!("session service: {e}");
                // a shell that ended while detached: start a fresh one there
                let retry = if matches!(service, Some(shift_pty::ServiceTarget::Attach(_))) {
                    Session::spawn(opts(Some(shift_pty::ServiceTarget::Spawn), replay.clone())).ok()
                } else {
                    None
                };
                match retry {
                    Some(s) => s,
                    None => {
                        let s = Session::spawn(opts(None, replay)).expect("spawn shell");
                        self.toast(format!(
                            "Session service unavailable ({e}); this shell is local"
                        ));
                        s
                    }
                }
            }
            Err(e) => panic!("spawn shell: {e}"),
        }
    }

    /// Transcript of `pane`'s finished blocks (restored ones included, so a
    /// history survives several relaunches, within the bounds).
    fn pane_transcript(pane: &PaneState) -> shift_core::transcript::Transcript {
        use shift_core::transcript::{SavedBlock, Transcript};
        // snapshot first: block_output_text takes the blocks lock itself
        let snapshot = pane.session.blocks.lock().unwrap().snapshot();
        let blocks: Vec<SavedBlock> = snapshot
            .into_iter()
            .filter_map(|b| match b.state {
                shift_core::BlockState::Done { exit, duration } => Some(SavedBlock {
                    output: terminal::block_output_text(&pane.session, b.id).unwrap_or_default(),
                    command: b.command,
                    exit,
                    duration_ms: duration.as_millis() as u64,
                    cwd: b.cwd,
                }),
                _ => None,
            })
            .collect();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        Transcript::bounded(blocks, now)
    }

    /// Write `pane`'s transcript when a block finished since the last write.
    fn save_transcript_if_changed(&mut self, pane_id: PaneId) {
        if !self.config.save_transcripts || !self.persist_session {
            return;
        }
        let Some(pane) = self
            .tabs
            .iter()
            .flat_map(|t| &t.panes)
            .find(|p| p.session.id == pane_id)
        else {
            return;
        };
        let newest = pane
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|b| matches!(b.state, shift_core::BlockState::Done { .. }))
            .map(|b| b.id);
        let Some(newest) = newest else { return };
        if self.transcript_saved.get(&pane_id) == Some(&newest) {
            return;
        }
        let id = pane.transcript_id.clone();
        let t = Self::pane_transcript(pane);
        shift_core::transcript::save(&id, &t);
        self.transcript_saved.insert(pane_id, newest);
    }

    /// Palette "Clear saved block transcripts": delete every file.
    fn clear_transcripts(&mut self) {
        let n = shift_core::transcript::remove_except(&[]);
        self.transcript_saved.clear();
        self.toast(format!(
            "Deleted {n} saved transcript{}",
            if n == 1 { "" } else { "s" }
        ));
    }

    fn spawn_tab(&mut self) {
        let dir = self.start_dir();
        self.spawn_tab_at(dir);
    }

    /// Probe the user's shell for aliases/functions off the UI thread.
    fn load_shell_env(&self) {
        let proxy = self.proxy.clone();
        let shell = shift_core::shell_integration::resolve_shell(self.config.shell.as_deref());
        std::thread::Builder::new()
            .name("shell-env".into())
            .spawn(move || {
                let env = shift_core::shell_env::ShellEnv::load(&shell);
                let _ = proxy.send_event(UserEvent::ShellEnvLoaded(Arc::new(env)));
            })
            .ok();
    }

    fn spawn_tab_at(&mut self, cwd: Option<std::path::PathBuf>) {
        self.settings.open = false;
        let (lw, lh) = self.logical_size();
        let (_, lw) = self.content_span(lw);
        let cols = ((lw - self.config.padding() * 2.0 - terminal::BLOCK_INDENT) / self.cell_w)
            .max(2.0) as usize;
        let rows = ((lh - TAB_BAR_H - TOP_PAD - EDITOR_H) / self.line_h).max(2.0) as usize;
        let session = self.spawn_session(cols, rows, cwd);
        self.tabs.push(TabState::new(self.new_pane(session)));
        self.active = self.tabs.len() - 1;
        self.save_session();
    }

    /// Persist tabs: split layouts, cwds, custom titles, colors (atomic;
    /// called on every structural change).
    fn save_session(&self) {
        if !self.persist_session {
            return;
        }
        shift_core::session_state::save(&self.saved_session());
        if self.config.save_transcripts {
            let mut keep = Vec::new();
            for p in self.tabs.iter().flat_map(|t| &t.panes) {
                shift_core::transcript::save(&p.transcript_id, &Self::pane_transcript(p));
                keep.push(p.transcript_id.clone());
            }
            // closed panes' transcripts go with them
            shift_core::transcript::remove_except(&keep);
        }
    }

    /// This window's tabs, layouts and frame, as the session file stores
    /// them.
    fn saved_session(&self) -> shift_core::session_state::SavedSession {
        use shift_core::session_state::{SavedAxis, SavedLayout, SavedSession, SavedTab};
        fn pane_cwd(p: &PaneState) -> Option<String> {
            p.session
                .cwd
                .lock()
                .unwrap()
                .clone()
                .or_else(|| p.last_git_cwd.clone())
        }
        // transcript ids are only recorded (and files only written) while
        // `save_transcripts` is on
        let transcripts = self.config.save_transcripts;
        fn layout(tab: &TabState, node: &crate::pane::PaneNode, transcripts: bool) -> SavedLayout {
            match node {
                crate::pane::PaneNode::Leaf(id) => SavedLayout::Pane {
                    cwd: tab.pane(*id).and_then(pane_cwd),
                    focused: *id == tab.active_pane,
                    transcript: tab
                        .pane(*id)
                        .filter(|_| transcripts)
                        .map(|p| p.transcript_id.clone()),
                    service: tab.pane(*id).and_then(|p| p.session.service_id),
                },
                crate::pane::PaneNode::Split {
                    axis,
                    ratio,
                    first,
                    second,
                } => SavedLayout::split(
                    match axis {
                        Axis::Horizontal => SavedAxis::Horizontal,
                        Axis::Vertical => SavedAxis::Vertical,
                    },
                    *ratio,
                    layout(tab, first, transcripts),
                    layout(tab, second, transcripts),
                ),
            }
        }
        let tabs = self
            .tabs
            .iter()
            .map(|t| SavedTab {
                cwd: t.active().and_then(pane_cwd),
                title: t.custom_title.clone(),
                color: t
                    .color
                    .and_then(|c| crate::state::TAB_COLORS.get(c))
                    .map(|s| s.to_string()),
                layout: t
                    .is_split()
                    .then(|| layout(t, &t.root.without_viewer(), transcripts)),
                transcript: (transcripts && !t.is_split())
                    .then(|| t.active().map(|p| p.transcript_id.clone()))
                    .flatten(),
                service: (!t.is_split())
                    .then(|| t.active().and_then(|p| p.session.service_id))
                    .flatten(),
            })
            .collect();
        SavedSession {
            tabs,
            active: self.active,
            window: self.window_frame(),
            ..Default::default()
        }
    }

    /// Current window frame in logical px (for session restore).
    fn window_frame(&self) -> Option<shift_core::session_state::SavedWindow> {
        let w = self.window.as_ref()?;
        let scale = w.scale_factor();
        let pos = w.outer_position().ok()?.to_logical::<f64>(scale);
        let size = w.inner_size().to_logical::<f64>(scale);
        Some(shift_core::session_state::SavedWindow {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            maximized: w.is_maximized(),
            fullscreen: w.fullscreen().is_some(),
        })
    }

    /// Reopen a saved tab: its split tree, pane cwds, title and color.
    fn restore_tab(&mut self, saved: &shift_core::session_state::SavedTab) {
        use shift_core::session_state::SavedLayout;
        fn first_cwd(l: &SavedLayout) -> Option<std::path::PathBuf> {
            match l {
                SavedLayout::Pane { cwd, .. } => cwd.clone().map(std::path::PathBuf::from),
                SavedLayout::Split { first, .. } => first_cwd(first),
            }
            .filter(|p| p.is_dir())
        }
        let top_cwd = saved.layout.as_ref().and_then(first_cwd).or_else(|| {
            saved
                .cwd
                .clone()
                .map(std::path::PathBuf::from)
                .filter(|p| p.is_dir())
        });
        let transcript = match &saved.layout {
            Some(l) => l.first_transcript().map(str::to_string),
            None => saved.transcript.clone(),
        };
        self.next_replay = transcript.clone();
        self.next_attach = match &saved.layout {
            Some(l) => l.first_service(),
            None => saved.service,
        };
        self.spawn_tab_at(top_cwd);
        let root = self.tabs[self.active].active_pane;
        if let Some(id) = transcript
            && let Some(p) = self.tabs[self.active].pane_mut(root)
        {
            p.transcript_id = id;
        }
        if let Some(layout) = &saved.layout {
            self.build_saved(layout, root);
            let tab = &mut self.tabs[self.active];
            apply_ratios(&mut tab.root, layout);
            if !tab.panes.iter().any(|p| p.session.id == tab.active_pane) {
                tab.active_pane = root;
            }
        }
        let tab = &mut self.tabs[self.active];
        tab.custom_title = saved.title.clone().filter(|t| !t.is_empty());
        tab.color = saved
            .color
            .as_deref()
            .and_then(|c| crate::state::TAB_COLORS.iter().position(|n| *n == c));

        fn apply_ratios(node: &mut crate::pane::PaneNode, saved: &SavedLayout) {
            if let (
                crate::pane::PaneNode::Split {
                    ratio,
                    first,
                    second,
                    ..
                },
                SavedLayout::Split {
                    ratio: r,
                    first: f,
                    second: s,
                    ..
                },
            ) = (node, saved)
            {
                *ratio = r.clamp(0.1, 0.9);
                apply_ratios(first, f);
                apply_ratios(second, s);
            }
        }
    }

    /// "Save current layout as launch configuration" (Warp Tab Configs): every
    /// tab's split tree + pane cwds as a Warp-compatible YAML file.
    fn save_current_layout(&mut self) {
        use shift_core::launch::{LaunchConfig, LaunchLayout, LaunchTab};
        fn layout(tab: &TabState, node: &crate::pane::PaneNode) -> LaunchLayout {
            match node {
                crate::pane::PaneNode::Leaf(id) => LaunchLayout::Pane {
                    cwd: tab
                        .pane(*id)
                        .and_then(|p| p.session.cwd.lock().unwrap().clone())
                        .map(std::path::PathBuf::from),
                    commands: Vec::new(),
                    focused: *id == tab.active_pane && tab.is_split(),
                },
                crate::pane::PaneNode::Split {
                    axis,
                    first,
                    second,
                    ..
                } => {
                    let vertical = *axis == Axis::Horizontal;
                    // flatten same-direction chains the way Warp nests them
                    let mut panes = Vec::new();
                    for child in [first, second] {
                        match layout(tab, child) {
                            LaunchLayout::Split {
                                vertical: v,
                                panes: inner,
                            } if v == vertical => panes.extend(inner),
                            other => panes.push(other),
                        }
                    }
                    LaunchLayout::split(vertical, panes)
                }
            }
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let cfg = LaunchConfig {
            name: format!("Layout {}", utc_stamp(secs)),
            tabs: self
                .tabs
                .iter()
                .map(|t| LaunchTab {
                    title: t.custom_title.clone(),
                    layout: layout(t, &t.root.without_viewer()),
                })
                .collect(),
            active_tab: self.active,
        };
        match shift_core::launch::save(&cfg) {
            Ok(path) => {
                self.launch_configs = shift_core::launch::load_all();
                let shown = editor::shorten_home(&path.to_string_lossy());
                self.toast(format!("Saved launch configuration to {shown}"));
            }
            Err(e) => self.toast(format!("Could not save launch configuration: {e}")),
        }
    }

    /// Transient status message at the bottom of the window.
    fn toast(&mut self, msg: String) {
        log::info!("toast: {msg}");
        self.toast = Some((msg, Instant::now()));
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Realize a saved pane tree inside the region held by `pane`.
    fn build_saved(&mut self, layout: &shift_core::session_state::SavedLayout, pane: PaneId) {
        use shift_core::session_state::{SavedAxis, SavedLayout};
        match layout {
            SavedLayout::Pane { focused, .. } => {
                if *focused && let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.active_pane = pane;
                }
            }
            SavedLayout::Split {
                axis,
                first,
                second,
                ..
            } => {
                fn leaf_cwd(l: &SavedLayout) -> Option<std::path::PathBuf> {
                    match l {
                        SavedLayout::Pane { cwd, .. } => cwd.clone().map(std::path::PathBuf::from),
                        SavedLayout::Split { first, .. } => leaf_cwd(first),
                    }
                    .filter(|p| p.is_dir())
                }
                let axis = match axis {
                    SavedAxis::Horizontal => Axis::Horizontal,
                    SavedAxis::Vertical => Axis::Vertical,
                };
                let transcript = second.first_transcript().map(str::to_string);
                self.next_replay = transcript.clone();
                self.next_attach = second.first_service();
                let new_id = self.split_pane(pane, axis, leaf_cwd(second));
                if let Some(id) = transcript
                    && let Some(p) = self
                        .tabs
                        .get_mut(self.active)
                        .and_then(|t| t.pane_mut(new_id))
                {
                    p.transcript_id = id;
                }
                self.build_saved(first, pane);
                self.build_saved(second, new_id);
            }
        }
    }

    fn split_active(&mut self, axis: Axis) {
        if self.tabs.is_empty() {
            return;
        }
        let dir = self.start_dir();
        let session = self.spawn_session(60, 20, dir); // resized on next layout
        let new_id = session.id;
        let pane = self.new_pane(session);
        let tab = &mut self.tabs[self.active];
        tab.zoomed = None;
        let target = tab.active_pane;
        if tab.root.split(target, new_id, axis) {
            tab.panes.push(pane);
            tab.active_pane = new_id;
        }
        self.save_session();
    }

    fn close_pane(&mut self, pane: PaneId, event_loop: &ActiveEventLoop) {
        let Some(tab_idx) = self
            .tabs
            .iter()
            .position(|t| t.panes.iter().any(|p| p.session.id == pane))
        else {
            return;
        };
        // closing a pane ends its shell even when the session service owns it
        if let Some(p) = self.tabs[tab_idx].pane(pane) {
            p.session.terminate();
        }
        let tab = &mut self.tabs[tab_idx];
        if tab.panes.len() <= 1 {
            // remember the cwd for ⇧⌘T (same source as save_session)
            let cwd = tab.active().and_then(|p| {
                p.session
                    .cwd
                    .lock()
                    .unwrap()
                    .clone()
                    .or_else(|| p.last_git_cwd.clone())
            });
            self.closed_tabs.push(cwd.map(std::path::PathBuf::from));
            if self.closed_tabs.len() > 10 {
                self.closed_tabs.remove(0);
            }
            self.tabs.remove(tab_idx);
            if self.tabs.is_empty() {
                self.save_session();
                event_loop.exit();
                return;
            }
            if self.active > tab_idx {
                self.active -= 1;
            } else if self.active >= self.tabs.len() {
                self.active = self.tabs.len() - 1;
            }
            self.save_session();
            return;
        }
        tab.zoomed = None;
        tab.root.remove(pane);
        tab.panes.retain(|p| p.session.id != pane);
        if tab.active_pane == pane {
            // the first remaining shell (the viewer leaf is not one)
            tab.active_pane = tab
                .root
                .leaves()
                .into_iter()
                .find(|id| *id != VIEWER_PANE)
                .unwrap_or(tab.panes[0].session.id);
        }
        self.save_session();
    }

    /// Ask for the active tab's panes' git status (and Docker Compose
    /// state, when that chip is on) after a `cd` or a finished command.
    fn request_git(&mut self) {
        let mut changed = false;
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let docker = self
            .config
            .prompt_chips
            .contains(&shift_core::PromptChip::Docker);
        for pane in &mut tab.panes {
            let cwd = pane.session.cwd.lock().unwrap().clone();
            let blocks = pane
                .session
                .blocks
                .lock()
                .unwrap()
                .last()
                .map_or(0, |b| b.id);
            let Some(cwd) = cwd else {
                continue;
            };
            let moved = pane.last_git_cwd.as_deref() != Some(cwd.as_str());
            if moved || pane.last_git_block != blocks {
                pane.last_git_block = blocks;
                if let Some(worker) = &self.git {
                    worker.request(pane.session.id, cwd.clone().into());
                }
                if docker && let Some(worker) = &self.docker_worker {
                    worker.request(pane.session.id, cwd.clone().into());
                }
            }
            if moved {
                pane.last_git_cwd = Some(cwd);
                changed = true;
            }
        }
        if changed {
            self.save_session();
        }
    }

    /// Blinking text's visible phase (SGR 5; its own beat, slower than the
    /// cursor's).
    fn text_blink_on(&self) -> bool {
        (self.blink_epoch.elapsed().as_millis() / TEXT_BLINK.as_millis()).is_multiple_of(2)
    }

    fn blink_on(&self) -> bool {
        (self.blink_epoch.elapsed().as_millis() / BLINK_INTERVAL.as_millis()).is_multiple_of(2)
    }

    /// Width taken by the vertical tab rail (0 with tabs on top).
    /// Cell size in physical pixels (text-area size replies and inline
    /// image sizing: programs compute image cell spans from these).
    fn cell_px(&self) -> (u16, u16) {
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        (
            (self.cell_w * scale).round() as u16,
            (self.line_h * scale).round() as u16,
        )
    }

    fn rail_w(&self) -> f32 {
        match self.config.tab_layout {
            shift_core::TabLayout::Vertical => tabbar::RAIL_W,
            shift_core::TabLayout::Horizontal => 0.0,
        }
    }

    /// Widths of the open side panels (explorer left, review right) in a
    /// window `w` wide. A panel that would squeeze the panes under
    /// `MIN_CONTENT_W` is hidden, review first.
    fn panel_widths(&self, w: f32) -> (f32, f32) {
        const MIN_CONTENT_W: f32 = 360.0;
        let room = w - self.rail_w();
        let mut ex = if self.explorer.open {
            crate::views::explorer::EXPLORER_W
        } else {
            0.0
        };
        let mut rv = if self.review.open {
            crate::views::review_panel::REVIEW_W
        } else {
            0.0
        };
        if room - ex - rv < MIN_CONTENT_W {
            rv = 0.0;
        }
        if room - ex - rv < MIN_CONTENT_W {
            ex = 0.0;
        }
        (ex, rv)
    }

    /// Left edge and width of the pane area (between rail/explorer and
    /// the review panel).
    fn content_span(&self, w: f32) -> (f32, f32) {
        let (ex, rv) = self.panel_widths(w);
        let x0 = self.rail_w() + ex;
        (x0, w - x0 - rv)
    }

    /// Reload the review model on a background thread: the top bar's
    /// `+added −removed` button, the review panel and the explorer's git
    /// colors all read it. The newest request wins.
    fn request_review(&mut self) {
        let Some(cwd) = self
            .tabs
            .get(self.active)
            .and_then(|t| t.active())
            .and_then(|p| p.session.cwd.lock().unwrap().clone())
        else {
            return;
        };
        self.review.generation += 1;
        if self.review.model().is_none() {
            self.review.load = crate::views::review_panel::Load::Loading;
        }
        let generation = self.review.generation;
        let proxy = self.proxy.clone();
        std::thread::Builder::new()
            .name("review".into())
            .spawn(move || {
                let result = shift_core::review::load(std::path::Path::new(&cwd));
                let _ = proxy.send_event(UserEvent::ReviewLoaded(generation, result));
            })
            .ok();
    }

    /// Reload the review model when the active pane's directory changes
    /// (a `cd`, another tab or pane).
    fn follow_review(&mut self) {
        let cwd = self
            .tabs
            .get(self.active)
            .and_then(|t| t.active())
            .and_then(|p| p.session.cwd.lock().unwrap().clone());
        if cwd.is_some() && cwd != self.review_cwd {
            self.review_cwd = cwd;
            self.request_review();
        }
    }

    /// Point the explorer at the active pane's project (on cwd change).
    fn follow_explorer(&mut self) {
        if !self.explorer.open {
            return;
        }
        let cwd = self
            .tabs
            .get(self.active)
            .and_then(|t| t.active())
            .and_then(|p| p.session.cwd.lock().unwrap().clone());
        if let Some(cwd) = cwd
            && self.explorer.follow(std::path::Path::new(&cwd))
        {
            self.request_review();
        }
    }

    /// XTWINOPS from `pane`'s program (xterm's window operations; DECCOLM
    /// and DECSLPP arrive as a character resize): move, resize, iconify,
    /// maximize and fullscreen the window, or report its state. Sizes in
    /// characters and pixels are the pane's text area: the window grows or
    /// shrinks by the difference. Pixels are device pixels, the unit
    /// `CSI 14 t` reports in.
    fn window_op(&mut self, pane: PaneId, params: &[Option<u16>]) {
        use winit::dpi::PhysicalSize;
        let Some(window) = self.window.clone() else {
            return;
        };
        let Some((_, tab)) = self.find_tab_of_pane(pane) else {
            return;
        };
        let Some(p) = tab.pane(pane) else {
            return;
        };
        // the pane as laid out (the grid the window currently gives it)
        let Some(grid) = p.last_grid else {
            return;
        };
        let text_area = p.text_area;
        let layout_window = p.layout_window;
        let (term_rows, term_cols) = {
            use shift_pty::alacritty_terminal::grid::Dimensions;
            let term = p.session.term.lock();
            (term.screen_lines(), term.columns())
        };
        let scale = window.scale_factor();
        let cell = (self.cell_w as f64 * scale, self.line_h as f64 * scale);
        let inner = window.inner_size();
        let outer = window.outer_size();
        let screen = window.current_monitor().map(|m| m.size()).unwrap_or(outer);
        let (inner_w, inner_h) = (inner.width as f64, inner.height as f64);
        // the window's chrome around the space the pane's grid gets, as of
        // the last layout (the window may have changed since)
        let chrome = (
            (layout_window.0 - text_area.0) as f64 * scale,
            (layout_window.1 - text_area.1) as f64 * scale,
        );
        let max_text = (
            screen.width as f64 - chrome.0,
            screen.height as f64 - chrome.1,
        );
        let _ = (inner_w, inner_h);
        let arg = |i: usize| params.get(i).copied().flatten();
        let mut out: Option<String> = None;
        // half a cell more than the exact size: the grid takes whole cells,
        // so rounding either way still lands on the requested count
        let resize_text = |w: f64, h: f64| {
            let size = PhysicalSize::new(
                (w + chrome.0 + cell.0 / 2.0).round(),
                (h + chrome.1 + cell.1 / 2.0).round(),
            );
            let _ = window.request_inner_size(size);
        };
        match arg(0).unwrap_or(0) {
            1 => {
                window.set_minimized(false);
                self.iconified = false;
            }
            2 => {
                window.set_minimized(true);
                self.iconified = true;
            }
            // positions are in points: they round-trip exactly on any scale
            3 => {
                let x = arg(1).unwrap_or(0) as f64;
                let y = arg(2).unwrap_or(0) as f64;
                window.set_outer_position(winit::dpi::LogicalPosition::new(x, y));
                self.window_pos = Some((x, y));
            }
            // the window in pixels (what `CSI 14;2 t` reports): omitted keeps,
            // 0 = the screen
            4 => {
                let h = match arg(1) {
                    None => inner.height,
                    Some(0) => screen.height,
                    Some(h) => h as u32,
                };
                let w = match arg(2) {
                    None => inner.width,
                    Some(0) => screen.width,
                    Some(w) => w as u32,
                };
                let _ = window.request_inner_size(PhysicalSize::new(w, h));
            }
            5 => window.focus_window(),
            7 => window.request_redraw(),
            // text area in characters: omitted keeps, 0 = the screen. The
            // term already took the size inline; the laid-out grid can still
            // be a resize behind, so "keep" reads the term.
            8 => {
                let rows = match arg(1) {
                    None => term_rows as f64,
                    Some(0) => (max_text.1 / cell.1).floor(),
                    Some(r) => r as f64,
                };
                let cols = match arg(2) {
                    None => term_cols as f64,
                    Some(0) => (max_text.0 / cell.0).floor(),
                    Some(c) => c as f64,
                };
                resize_text(cols * cell.0, rows * cell.1);
            }
            // maximize: 0 restore, 1 both ways, 2 vertically, 3 horizontally
            9 => match arg(1).unwrap_or(0) {
                0 => {
                    window.set_maximized(false);
                    if let Some(size) = self.restore_size.take() {
                        let _ = window.request_inner_size(size);
                    }
                }
                n => {
                    self.restore_size
                        .get_or_insert(window.inner_size().to_logical(scale));
                    let frame = (outer.width - inner.width, outer.height - inner.height);
                    let size = match n {
                        2 => PhysicalSize::new(inner.width, screen.height - frame.1),
                        3 => PhysicalSize::new(screen.width - frame.0, inner.height),
                        _ => PhysicalSize::new(screen.width - frame.0, screen.height - frame.1),
                    };
                    let _ = window.request_inner_size(size);
                }
            },
            // fullscreen: 0 leave, 1 enter, 2 toggle
            10 => {
                let on = match arg(1).unwrap_or(0) {
                    0 => false,
                    1 => true,
                    _ => !self.fullscreen_op,
                };
                self.fullscreen_op = on;
                if self.headless {
                    // a hidden window has no fullscreen space: cover the screen
                    if on {
                        self.restore_size
                            .get_or_insert(window.inner_size().to_logical(scale));
                        let _ = window.request_inner_size(screen);
                    } else if let Some(size) = self.restore_size.take() {
                        let _ = window.request_inner_size(size);
                    }
                } else {
                    window
                        .set_fullscreen(on.then_some(winit::window::Fullscreen::Borderless(None)));
                }
            }
            11 => {
                let iconified = window.is_minimized().unwrap_or(self.iconified);
                out = Some(format!("\x1b[{}t", if iconified { 2 } else { 1 }));
            }
            13 => {
                let (x, y) = window
                    .outer_position()
                    .ok()
                    .map(|p| {
                        let p: winit::dpi::LogicalPosition<f64> = p.to_logical(scale);
                        (p.x, p.y)
                    })
                    .or(self.window_pos)
                    .unwrap_or((0.0, 0.0));
                let (x, y) = if arg(1) == Some(2) {
                    (x + grid.x as f64, y + grid.y as f64)
                } else {
                    (x, y)
                };
                out = Some(format!("\x1b[3;{};{}t", x.round() as i64, y.round() as i64));
            }
            // 14;2: the whole window
            14 => out = Some(format!("\x1b[4;{};{}t", outer.height, outer.width)),
            15 => out = Some(format!("\x1b[5;{};{}t", screen.height, screen.width)),
            19 => {
                out = Some(format!(
                    "\x1b[9;{};{}t",
                    (max_text.1 / cell.1).floor() as i64,
                    (max_text.0 / cell.0).floor() as i64
                ))
            }
            _ => {}
        }
        if let Some(text) = out
            && let Some((_, tab)) = self.find_tab_of_pane(pane)
            && let Some(p) = tab.pane(pane)
        {
            p.session.write(text.as_bytes());
        }
    }

    /// Run a review-panel write, then reload. Revert asks first.
    fn review_action(&mut self, hit: ReviewHit) {
        use shift_core::review::Op;
        if hit == ReviewHit::Maximize {
            self.review.maximized = !self.review.maximized;
            return;
        }
        let Some(model) = self.review.model().cloned() else {
            return;
        };
        let root = model.root.clone();
        let ops: ReviewOps = match hit {
            ReviewHit::Refresh => {
                self.review.notice = None;
                self.explorer.refresh();
                self.request_review();
                return;
            }
            ReviewHit::ToggleSection(staged) => {
                if !self.review.collapsed_sections.remove(&staged) {
                    self.review.collapsed_sections.insert(staged);
                }
                return;
            }
            ReviewHit::ToggleFile(staged, path) => {
                let key = (staged, path);
                if !self.review.expanded.remove(&key) {
                    self.review.expanded.insert(key);
                }
                return;
            }
            ReviewHit::Reveal(staged, path, gap) => {
                self.review.reveal(staged, &path, gap);
                return;
            }
            ReviewHit::Open(path) => {
                let choice = self.config.file_editor.clone();
                // the viewer opens among the panes the maximized panel covers
                if choice.trim() == shift_core::editors::SHIFT {
                    self.review.maximized = false;
                }
                self.open_file_in(&root.join(&path), &choice);
                return;
            }
            ReviewHit::Jump(staged, path) => {
                self.review.jump(staged, &path);
                return;
            }
            ReviewHit::Copy(staged, path, id) => {
                let files = if staged {
                    &model.staged
                } else {
                    &model.unstaged
                };
                if let Some(patch) = files
                    .iter()
                    .find(|f| f.path == path)
                    .and_then(|f| crate::views::review_panel::hunk_patch(f, &id))
                    && self.copy_to_clipboard(patch)
                {
                    self.toast("Hunk copied".into());
                }
                return;
            }
            ReviewHit::Maximize => return,
            ReviewHit::Stage(path, hunk) => vec![(Op::Stage, path, hunk)],
            ReviewHit::Unstage(path, hunk) => vec![(Op::Unstage, path, hunk)],
            ReviewHit::StageAll => model
                .unstaged
                .iter()
                .map(|f| (Op::Stage, f.path.clone(), None))
                .collect(),
            ReviewHit::UnstageAll => model
                .staged
                .iter()
                .map(|f| (Op::Unstage, f.path.clone(), None))
                .collect(),
            ReviewHit::Revert(path, hunk) => {
                let what = match hunk {
                    Some(id) => format!(
                        "{path} · lines {}–{}",
                        id.new_start,
                        id.new_start + id.new_lines.saturating_sub(1)
                    ),
                    None => path.clone(),
                };
                self.confirm = Some(ConfirmClose {
                    opened: Instant::now(),
                    action: ConfirmAction::Revert { root, path, hunk },
                    commands: vec![what],
                });
                return;
            }
        };
        self.queue_review_write(root, ops);
    }

    /// Run review-panel writes (stage, unstage, revert) on the one review
    /// writer thread: repository writes stay in order and never block input
    /// or rendering. The result arrives as `UserEvent::ReviewWritten`.
    fn queue_review_write(&mut self, root: std::path::PathBuf, ops: ReviewOps) {
        if self.review_writer.is_none() {
            let (tx, rx) = std::sync::mpsc::channel::<ReviewWrite>();
            let proxy = self.proxy.clone();
            let spawned = std::thread::Builder::new()
                .name("review-write".into())
                .spawn(move || {
                    for (root, ops) in rx {
                        let result = ops.into_iter().try_for_each(|(op, path, hunk)| {
                            shift_core::review::apply(&root, op, &path, hunk)
                        });
                        let _ = proxy.send_event(UserEvent::ReviewWritten(result.err()));
                    }
                });
            if let Err(e) = spawned {
                self.review.notice = Some(format!("could not start the review writer: {e}"));
                return;
            }
            self.review_writer = Some(tx);
        }
        if let Some(tx) = &self.review_writer
            && tx.send((root, ops)).is_err()
        {
            self.review_writer = None;
            self.review.notice = Some("the review writer stopped; try again".into());
        }
    }

    /// Rects of the visible explorer and review panels (zero width = hidden).
    fn panel_rects(&self) -> ([f32; 4], [f32; 4]) {
        let (w, h) = self.logical_size();
        let (ex, rv) = self.panel_widths(w);
        let rail = self.rail_w();
        let review = if self.review_maximized() {
            [rail, TAB_BAR_H, w - rail, h - TAB_BAR_H]
        } else {
            [w - rv, TAB_BAR_H, rv, h - TAB_BAR_H]
        };
        ([rail, TAB_BAR_H, ex, h - TAB_BAR_H], review)
    }

    /// The review panel covers the panes (its ⤢ button). The pane layout
    /// behind it does not change, so no shell is resized.
    fn review_maximized(&self) -> bool {
        self.review.open && self.review.maximized && !self.settings.open
    }

    /// The pointer is over the maximized review panel.
    fn over_maximized_review(&self) -> bool {
        let (mx, my) = self.mouse;
        let r = self.panel_rects().1;
        self.review_maximized() && mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
    }

    /// Pointer over the viewer and side panels: a hand on buttons, review
    /// rows and tree rows, the arrow elsewhere. `None` outside them.
    fn panel_cursor(
        &self,
        inside: &impl Fn(&[f32; 4]) -> bool,
    ) -> Option<winit::window::CursorIcon> {
        use winit::window::CursorIcon;
        let hand = |on: bool| {
            if on {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            }
        };
        if self.viewer.is_open() && inside(&self.viewer_rect) {
            return Some(hand(self.viewer_hits.iter().any(|(r, _)| inside(r))));
        }
        let (ex_rect, rv_rect) = self.panel_rects();
        if ex_rect[2] > 0.0 && inside(&ex_rect) {
            return Some(hand(self.explorer_hits.iter().any(|(r, _)| inside(r))));
        }
        if rv_rect[2] > 0.0 && inside(&rv_rect) {
            return Some(hand(self.review_hits.iter().any(|(r, _)| inside(r))));
        }
        None
    }

    /// A left click on the viewer or a side panel; true when it was theirs.
    fn on_panel_click(&mut self, inside: impl Fn(&[f32; 4]) -> bool) -> bool {
        if self.viewer.is_open() {
            if let Some(hit) = self
                .viewer_hits
                .iter()
                .find(|(r, _)| inside(r))
                .map(|(_, h)| h.clone())
            {
                self.viewer_focus = true;
                match hit {
                    ViewerHit::Close => self.viewer.close(),
                    ViewerHit::Mode(rendered) => self.viewer.set_rendered(rendered),
                    ViewerHit::Menu => {
                        // drops down from the ⋮ button
                        let at = self
                            .viewer_hits
                            .iter()
                            .find(|(_, h)| *h == ViewerHit::Menu)
                            .map_or(self.mouse, |(r, _)| (r[0], r[1] + r[3] + 4.0));
                        self.open_viewer_menu(at);
                    }
                }
                return true;
            }
            if inside(&self.viewer_rect) {
                self.viewer_focus = true;
                // its header bar is the pane's title bar: drag to move
                let [x, y, w, _] = self.viewer_rect;
                if inside(&[x, y, w, crate::views::viewer::HEADER_H]) {
                    self.pane_drag = Some(PaneDrag {
                        pane: VIEWER_PANE,
                        from_tab: self.active,
                        press: self.mouse,
                        live: false,
                        target: None,
                        hover_tab: None,
                    });
                }
                return true;
            }
        }
        let (ex_rect, rv_rect) = self.panel_rects();
        if ex_rect[2] > 0.0 && inside(&ex_rect) {
            self.explorer.focused = true;
            let hit = self
                .explorer_hits
                .iter()
                .find(|(r, _)| inside(r))
                .map(|(_, h)| h.clone());
            match hit {
                Some(ExplorerHit::Row(i)) => {
                    if let Some(row) = self.explorer.rows.get(i).cloned() {
                        self.explorer.selected = Some(row.path.clone());
                        if self.modifiers.alt_key() {
                            self.drop_file(&row.path);
                        } else if row.is_dir {
                            self.explorer.toggle(&row.path);
                        } else {
                            let choice = self.config.file_editor.clone();
                            self.open_file_in(&row.path, &choice);
                        }
                    }
                }
                Some(ExplorerHit::Refresh) => {
                    self.explorer.refresh();
                    self.request_review();
                }
                Some(ExplorerHit::Collapse) => self.explorer.collapse_all(),
                None => {}
            }
            return true;
        }
        if rv_rect[2] > 0.0 && inside(&rv_rect) {
            if let Some(hit) = self
                .review_hits
                .iter()
                .find(|(r, _)| inside(r))
                .map(|(_, h)| h.clone())
            {
                self.review_action(hit);
            }
            return true;
        }
        false
    }

    /// Layout the active tab's panes within the content area.
    fn pane_rects(&self, w: f32, h: f32) -> (Vec<PaneRect>, Vec<HandleRect>) {
        let mut panes = Vec::new();
        let mut handles = Vec::new();
        let Some(tab) = self.tabs.get(self.active) else {
            return (panes, handles);
        };
        let (x0, w) = self.content_span(w);
        // ⇧⌘⏎ zoom: the zoomed pane takes the whole content rect (no handles
        // → no drag targets); hidden panes keep streaming, just not laid out
        if let Some(z) = tab.zoomed
            && tab.is_split()
            && tab.panes.iter().any(|p| p.session.id == z)
        {
            panes.push(PaneRect {
                id: z,
                rect: [x0, TAB_BAR_H, w, h - TAB_BAR_H],
            });
            return (panes, handles);
        }
        if tab.root.is_split() {
            // flush panes (Warp): the whole content rect, 1px dividers; the
            // viewer pane is one of the leaves
            let area = [x0, TAB_BAR_H, w, h - TAB_BAR_H];
            tab.root.layout(area, &mut panes, &mut handles, vec![]);
        } else {
            panes.push(PaneRect {
                id: tab.active_pane,
                rect: [x0, TAB_BAR_H, w, h - TAB_BAR_H],
            });
        }
        (panes, handles)
    }

    fn build_scene(&mut self, w: f32, h: f32) {
        self.sync_viewer_pane();
        self.sync_history();
        let theme = self.theme.clone();
        let blink = self.blink_on() || !self.config.cursor_blink;
        let text_blink_on = self.text_blink_on();
        // re-armed by this frame: blinking text keeps the repaint beat going
        crate::views::terminal::BLINK_DRAWN.store(false, std::sync::atomic::Ordering::Relaxed);
        let cell_w = self.cell_w;
        let cell_px = self.cell_px();
        let font_size = self.font_size;
        let line_h = self.line_h;
        let ligatures = self.config.ligatures;
        let cursor_style = self.config.cursor_style;
        let mouse = self.mouse;
        let prompt_mode = self.config.prompt;
        let input_top = self.config.input_position == shift_core::InputPosition::Top;
        let autosuggest = self.config.autosuggestions && !self.history.open;
        let side_pad = self.config.padding();
        let bell_flash = self.bell_flash.is_some_and(|t| t.elapsed() < BELL_FLASH);
        let settings_open = self.settings.open;
        let active_tab_idx = self.active;
        self.follow_explorer();
        self.follow_review();
        self.mark_visible_read();
        self.prune_note_toasts();
        let panel_buttons = tabbar::PanelButtons {
            explorer_open: self.explorer.open,
            // lit only while the panel is actually on screen
            review_open: self.review_maximized()
                || self.review.open && self.panel_widths(self.logical_size().0).1 > 0.0,
            diff: self.review.model().map(|m| {
                m.staged
                    .iter()
                    .chain(&m.unstaged)
                    .fold((0, 0), |(a, d), f| (a + f.added, d + f.removed))
            }),
            notifications_open: self.notifications.open,
            unread: self.unread_attention(),
            update: self.update_chip().is_some(),
        };
        let notification_entries = if self.notifications.open {
            self.notification_entries()
        } else {
            Vec::new()
        };
        // toasts hide under the open popover and the settings overlay
        let toast_entries: Vec<crate::views::notifications::Entry> =
            if self.notifications.open || self.settings.open {
                Vec::new()
            } else {
                let now = std::time::SystemTime::now();
                self.note_toasts
                    .iter()
                    .filter_map(|(id, _)| self.attention.iter().find(|a| a.id == *id))
                    .map(|a| Self::entry_of(a, now))
                    .collect()
            };
        let (pane_rects, handles) = self.pane_rects(w, h);
        let (explorer_w, review_w) = self.panel_widths(w);
        let viewer_label = self.viewer.path.as_ref().map(|p| {
            self.explorer
                .root
                .as_ref()
                .and_then(|r| p.strip_prefix(r).ok())
                .or_else(|| {
                    self.review
                        .model()
                        .and_then(|m| p.strip_prefix(&m.root).ok())
                })
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_else(|| editor::shorten_home(&p.to_string_lossy()))
        });
        // the viewer's own pane in this tab's split, and whether it (rather
        // than the tab's active shell) has focus
        let viewer_pane = pane_rects
            .iter()
            .find(|p| p.id == VIEWER_PANE)
            .map(|p| p.rect)
            .filter(|_| viewer_label.is_some());
        let viewer_has_focus = self.viewer_focused();

        let configured = self.config.shell.clone();
        let shell_label = match &self.shell_label {
            Some((for_config, name)) if *for_config == configured => name.clone(),
            _ => {
                let name = shift_core::shell_integration::shell_name(configured.as_deref());
                self.shell_label = Some((configured, name.clone()));
                name
            }
        };
        let tab_infos: Vec<tabbar::TabInfo> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| tabbar::TabInfo {
                title: t.display_title(),
                panes: t.root.leaves().len(),
                activity: t.activity,
                alert: self
                    .attention
                    .iter()
                    .rev()
                    .find(|a| !a.read && t.pane(a.pane).is_some())
                    .map(|a| a.kind),
                color: t.color.map(|c| context_menu::tab_color(&theme, c)),
                sync: t.sync_inputs && t.is_split(),
                editing: self
                    .rename
                    .as_ref()
                    .filter(|(ri, _)| *ri == i)
                    .map(|(_, s)| s.clone()),
                editing_selected: self.rename_fresh,
                subtitle: t
                    .active()
                    .map(|p| {
                        let cwd = p
                            .session
                            .cwd
                            .lock()
                            .unwrap()
                            .as_deref()
                            .map(editor::shorten_home)
                            .unwrap_or_default();
                        match &p.git {
                            Some(g) => format!("{cwd} · {}", g.branch),
                            None => cwd,
                        }
                    })
                    .unwrap_or_default(),
                settings: false,
                running: t.panes.iter().any(PaneState::running),
            })
            .chain(self.settings.tab.then(|| tabbar::TabInfo {
                title: "Settings".into(),
                panes: 0,
                activity: false,
                alert: None,
                color: None,
                sync: false,
                editing: None,
                editing_selected: false,
                subtitle: self.settings.page.label().into(),
                settings: true,
                running: false,
            }))
            .collect();
        // the strip's lit tab: the Settings tab (last) while it shows
        let strip_active = if self.settings.open && self.settings.tab {
            self.tabs.len()
        } else {
            self.active
        };
        let rail_w = self.rail_w();
        // chrome hover follows the pointer unless a modal surface covers it
        let chrome_mouse = (!self.palette.open && self.confirm.is_none() && !self.whats_new.open)
            .then_some(self.mouse);
        // a tab dragged past the slop floats under the pointer
        let lifted_tab =
            self.tab_drag
                .filter(|d| d.live && rail_w == 0.0)
                .map(|d| tabbar::Lifted {
                    index: d.index,
                    x: self.mouse.0 - d.grab,
                });
        let confirm = self
            .confirm
            .as_ref()
            .map(|c| (c.action.clone(), c.commands.clone()));
        let toast = self
            .toast
            .as_ref()
            .filter(|(_, t)| t.elapsed() < TOAST_TTL)
            .map(|(m, _)| m.clone());
        let filter_editing = self.filter_editing;
        let shell_env = self.shell_env.clone();
        let error_underline = self.config.error_underline;
        // Warp with a background image and translucency: the image at the
        // window opacity, then bg at (1 − image opacity) × window opacity
        let image_veil = (theme.ground_alpha < 1.0
            && self.bg_image.as_ref().is_some_and(|p| p.has_image()))
        .then(|| {
            theme
                .bg
                .with_alpha((1.0 - self.bg_image_opacity) * theme.ground_alpha)
        });

        let Self {
            scene,
            text,
            tabs,
            completion,
            palette,
            palette_results,
            palette_row_hits,
            settings_names,
            settings_stops,
            settings: settings_state,
            config: config_ref,
            keymap: keymap_ref,
            search: search_state,
            ..
        } = self;
        let Some(engine) = text.as_mut() else { return };
        let ed_char_w = engine.measure(
            "0",
            editor::FONT_SIZE,
            shift_renderer::mono_family(),
            shift_renderer::glyphon::Weight::NORMAL,
        );
        let mut active_ed_cols = 80usize;

        scene.clear();
        if let Some(veil) = image_veil {
            scene.layer(0).rect([0.0, 0.0], [w, h], veil, 0.0);
        }
        let mut hits = Vec::new();
        let mut focus_hits = Vec::new(); // lowest priority, appended last
        let mut any_running = false;
        let mut editor_active = false;
        let mut boot_until: Option<Instant> = None;
        let mut editor_anchor = (0.0f32, 0.0f32);
        let mut editor_zone = [0.0f32; 4];
        let mut editor_pane = [0.0f32; 4];
        let mut ime_spot: Option<(f32, f32)> = None;
        let completion_ghost = completion.ghost.clone();
        // the panel docked to the active command line (drawn on layer 2
        // below): the output moves out from under it (Warp)
        let docked_h = if palette.open {
            None
        } else if self.history.open {
            Some(crate::views::history::panel_height(&self.history))
        } else if completion.open && !completion.items.is_empty() {
            Some(completion_view::panel_height(completion))
        } else {
            None
        };

        // ── layer 0: chrome ──
        let mut over = shift_renderer::Layer::default();
        if rail_w > 0.0 {
            tabbar::render(
                scene.layer(0),
                &mut over,
                engine,
                &theme,
                &[],
                active_tab_idx,
                w,
                blink,
                panel_buttons,
                chrome_mouse,
                None,
                &mut hits,
            );
            tabbar::render_rail(
                scene.layer(0),
                engine,
                &theme,
                &tab_infos,
                strip_active,
                h,
                blink,
                &mut hits,
            );
        } else {
            tabbar::render(
                scene.layer(0),
                &mut over,
                engine,
                &theme,
                &tab_infos,
                strip_active,
                w,
                blink,
                panel_buttons,
                chrome_mouse,
                lifted_tab,
                &mut hits,
            );
        }
        // the lifted tab floats over everything below the menus
        let l5 = scene.layer(5);
        l5.quads.extend(over.quads);
        l5.texts.extend(over.texts);

        let Some(tab) = tabs.get_mut(active_tab_idx) else {
            self.hits = hits;
            return;
        };
        let split = tab.effective_split();

        if split {
            // gutter fill
            scene.layer(0).rect(
                [rail_w, TAB_BAR_H],
                [w - rail_w, h - TAB_BAR_H],
                theme.ground(theme.bg_gutter),
                0.0,
            );
        }

        // dividers: the 1px gap between flush panes, grabbable a few px
        // either side (these hits precede the panes' own)
        for handle in handles.iter().filter(|_| !settings_open) {
            let [hx, hy, hw, hh] = handle.rect;
            scene
                .layer(0)
                .rect([hx, hy], [hw, hh], theme.pane_border_inactive, 0.0);
            let g = crate::pane::HANDLE_GRAB;
            let grab = match handle.axis {
                Axis::Horizontal => [hx - g, hy, hw + 2.0 * g, hh],
                Axis::Vertical => [hx, hy - g, hw, hh + 2.0 * g],
            };
            hits.push(Hit {
                rect: grab,
                target: ClickTarget::HandleDrag(handle.path.clone()),
            });
        }

        // the Settings tab covers the shell tab: none of its panes draw (a
        // translucent window would show them through the page)
        for pr in pane_rects.iter().filter(|_| !settings_open) {
            let Some(pane) = tab.panes.iter_mut().find(|p| p.session.id == pr.id) else {
                continue;
            };
            let is_active_pane = pr.id == tab.active_pane && !viewer_has_focus;
            let editor_vis = Self::editor_visible(pane);
            if is_active_pane {
                editor_active = editor_vis;
            }
            // soft-wrap width of this pane's command editor
            let editor_area_w = if split {
                pr.rect[2] - PANE_PAD * 2.0
            } else {
                pr.rect[2] - side_pad * 2.0
            };
            let ed_cols =
                editor::input_cols(editor_area_w, cell_w, ed_char_w, pane.editor.vim_enabled());
            if is_active_pane {
                active_ed_cols = ed_cols;
            }
            // Warp-style boot placeholder until the first prompt (not over a
            // replayed transcript). The shell's first bytes (title, OSC 7)
            // arrive before its prompt marker; hiding the placeholder then
            // flashed the grid cursor at the top. Shells without integration
            // never mark a prompt: their raw output shows after BOOT_GRACE.
            let booting = pane.session.blocks.lock().unwrap().last().is_none()
                && if pane
                    .session
                    .saw_output
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    let first = *pane.first_output.get_or_insert_with(Instant::now);
                    let until = first + BOOT_GRACE;
                    let waiting = Instant::now() < until;
                    if waiting {
                        boot_until = Some(boot_until.map_or(until, |u: Instant| u.min(until)));
                    }
                    waiting
                } else {
                    true
                };

            // a split pane's content sits below its title bar
            let body = if split {
                [
                    pr.rect[0],
                    pr.rect[1] + PANE_HEADER_H,
                    pr.rect[2],
                    pr.rect[3] - PANE_HEADER_H,
                ]
            } else {
                pr.rect
            };
            let text_area: (f32, f32);
            let grid = {
                // inline grid_for (self is destructured)
                let rect = body;
                let (x, y, pw, ph) = if split {
                    (
                        rect[0] + PANE_PAD,
                        rect[1] + PANE_PAD,
                        rect[2] - PANE_PAD * 2.0,
                        rect[3] - PANE_PAD * 2.0,
                    )
                } else {
                    (
                        rect[0] + side_pad,
                        rect[1] + TOP_PAD,
                        rect[2] - side_pad * 2.0,
                        rect[3] - TOP_PAD,
                    )
                };
                let editor_h = if !editor_vis {
                    if split { PANE_PAD } else { SIDE_PAD }
                } else {
                    editor::editor_height(&pane.editor.buffer, prompt_mode, ed_cols)
                };
                let usable_w = (pw - terminal::BLOCK_INDENT).max(cell_w);
                let usable_h = (ph - editor_h).max(line_h);
                text_area = (usable_w, usable_h);
                // input at the top: the grid starts below the editor zone
                let bottom_up =
                    !input_top && config_ref.blocks_start == shift_core::BlocksStart::Bottom;
                let rows = (usable_h / line_h).floor().max(2.0) as usize;
                let y = if editor_vis && input_top {
                    y + editor_h - (if split { PANE_PAD } else { TOP_PAD - 4.0 })
                } else if !editor_vis && bottom_up {
                    // a program owns the screen (Claude Code, vim): its last
                    // row sits on the bottom padding and the part of a row
                    // left over goes above the grid, not under it (Warp)
                    y + (usable_h - rows as f32 * line_h).max(0.0)
                } else {
                    y
                };
                GridLayout {
                    x,
                    y,
                    right: x + pw,
                    cell_w,
                    line_h,
                    font_size,
                    cols: (usable_w / cell_w).floor().max(2.0) as usize,
                    rows,
                    ligatures,
                    cursor_style,
                    // the boot placeholder sits in the input zone; no stray
                    // grid cursor at the top while it shows
                    editor_owns_cursor: editor_vis || booting,
                    text_blink_on,
                    // with the input at the top, blocks start under it
                    bottom_up,
                    mouse: Some(mouse),
                }
            };

            pane.session.resize(
                GridSize {
                    cols: grid.cols,
                    rows: grid.rows,
                },
                cell_px,
            );
            pane.text_area = text_area;
            pane.layout_window = (w, h);
            let view = match docked_h.filter(|_| is_active_pane && editor_vis) {
                Some(panel_h) => {
                    let eh = editor::editor_height(&pane.editor.buffer, prompt_mode, ed_cols);
                    let (top, bottom) = if input_top {
                        let zone_top = pr.rect[1]
                            + if split {
                                PANE_HEADER_H + PANE_PAD - 4.0
                            } else {
                                4.0
                            };
                        (zone_top + eh, zone_top + eh + panel_h)
                    } else {
                        let zone_top = pr.rect[1] + pr.rect[3] - eh;
                        (zone_top - panel_h, zone_top)
                    };
                    terminal::beside_panel(grid, top, bottom)
                }
                None => grid,
            };

            // pane chrome (design 02)
            if split {
                // flush, square panes; focus shows as the corner mark and the
                // wash over the others (Warp's style)
                scene.layer(0).rect(
                    [pr.rect[0], pr.rect[1]],
                    [pr.rect[2], pr.rect[3]],
                    theme.ground(theme.bg),
                    0.0,
                );
                // title bar: the program's title, else the directory
                let title = if pane.title.is_empty() {
                    pane.session
                        .cwd
                        .lock()
                        .unwrap()
                        .as_deref()
                        .map(editor::shorten_home)
                        .unwrap_or_else(|| shell_label.clone())
                } else {
                    pane.title.clone()
                };
                crate::views::pane_header::render(
                    scene.layer(1),
                    engine,
                    &theme,
                    [pr.rect[0], pr.rect[1], pr.rect[2], PANE_HEADER_H],
                    &title,
                    is_active_pane,
                    pane.running(),
                    chrome_mouse.filter(|_| self.pane_drag.is_none()),
                    pr.id,
                    &mut hits,
                );
                focus_hits.push(Hit {
                    rect: pr.rect,
                    target: ClickTarget::PaneFocus(pr.id),
                });
            }

            // content
            let layer = scene.layer(1);
            let text_start = layer.texts.len();
            // the program's own default colors (OSC 4/10/11/12) and reverse
            // screen (DECSCNM: vttest, `tput flash`) apply to this pane only
            let pane_theme = terminal::pane_theme(&theme, &pane.session).inspect(|t| {
                layer.rect(
                    [pr.rect[0], pr.rect[1]],
                    [pr.rect[2], pr.rect[3]],
                    t.bg,
                    0.0,
                );
            });
            let search_ctx = if search_state.open && is_active_pane && !filter_editing {
                search_state.matcher().map(|matcher| terminal::SearchCtx {
                    matcher,
                    current: search_state.current_anchor(),
                })
            } else {
                None
            };
            let out = terminal::render(
                layer,
                engine,
                pane_theme.as_ref().unwrap_or(&theme),
                &pane.session,
                &view,
                &mut pane.view_offset,
                // shell-prompt mode keeps the shell's PS1 rows on screen
                editor_vis && prompt_mode != shift_core::PromptMode::Shell,
                search_ctx.as_ref(),
                if is_active_pane {
                    pane.selected_block
                } else {
                    None
                },
                pane.filter.as_ref(),
            );
            if pane.view_offset > 0 && out.display_len > pane.last_display_len {
                pane.view_offset += out.display_len - pane.last_display_len;
            }
            pane.last_display_len = out.display_len;
            pane.rows = out.rows;
            pane.last_grid = Some(view);
            pane.alt_screen = out.alt_screen;
            layer.sprites.extend(terminal::image_sprites(
                &pane.session,
                &pane.rows,
                &view,
                out.alt_screen,
            ));
            pane.has_selection = {
                // cheap mirror: render computed to_range under the same lock
                pane.session.term.lock().selection.is_some()
            };
            any_running |= out.any_running;
            for hb in out.hits {
                hits.push(Hit {
                    rect: hb.rect,
                    target: ClickTarget::Block(pr.id, hb.block),
                });
            }
            for hb in out.chevron_hits {
                hits.push(Hit {
                    rect: hb.rect,
                    target: ClickTarget::BlockChevron(pr.id, hb.block),
                });
            }
            for hb in out.copy_hits {
                hits.push(Hit {
                    rect: hb.rect,
                    target: ClickTarget::BlockCopy(pr.id, hb.block),
                });
            }
            for hb in out.duration_hits {
                hits.push(Hit {
                    rect: hb.rect,
                    target: ClickTarget::BlockDuration(pr.id, hb.block),
                });
            }
            for (rect, id) in &out.bookmark_hits {
                hits.push(Hit {
                    rect: *rect,
                    target: ClickTarget::JumpToBlock(pr.id, *id),
                });
            }
            pane.url_hits = out.url_hits;
            pane.sticky_hit = None;
            let sticky_block = out.sticky_block;
            let sticky_open = out.sticky_open;

            if booting {
                let shell = shift_core::shell_integration::shell_name(config_ref.shell.as_deref());
                let label = format!("Starting {shell}\u{2026}");
                let buf = {
                    use shift_renderer::glyphon::{
                        Attrs, Buffer, Family, Metrics, Shaping, Weight,
                    };
                    let mut b = Buffer::new(
                        &mut engine.font_system,
                        Metrics::new(editor::FONT_SIZE, editor::LINE_H),
                    );
                    b.set_text(
                        &mut engine.font_system,
                        &label,
                        &Attrs::new()
                            .family(Family::Name(shift_renderer::mono_family()))
                            .weight(Weight::MEDIUM),
                        Shaping::Advanced,
                        None,
                    );
                    b
                };
                // where the input line will appear
                let (lx, zone_y) = if split {
                    (body[0] + PANE_PAD, body[1] + PANE_PAD - 4.0)
                } else {
                    (pr.rect[0] + side_pad, pr.rect[1] + 4.0)
                };
                let top = if input_top {
                    zone_y
                } else if split {
                    pr.rect[1] + pr.rect[3] - editor::editor_height("", prompt_mode, 1)
                } else {
                    h - editor::editor_height("", prompt_mode, 1)
                };
                // the editor's divider, already where it will be (Warp)
                let zone_w = pr.rect[2] - 2.0 * if split { PANE_PAD } else { side_pad };
                layer.hline(lx - 8.0, top, zone_w + 16.0, 1.0, theme.border);
                // on the chip row, where the prompt context appears first (Warp)
                let ly = top + editor::first_row_offset(prompt_mode);
                layer.text(buf, lx + editor::PAD_X, ly, theme.syntax.string);
            }

            pane.last_exit = pane
                .session
                .blocks
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find_map(|b| b.exit_code());

            if editor_vis {
                let eh = editor::editor_height(&pane.editor.buffer, prompt_mode, ed_cols);
                let area = match (split, input_top) {
                    (true, false) => [
                        pr.rect[0] + PANE_PAD,
                        pr.rect[1] + pr.rect[3] - eh,
                        pr.rect[2] - PANE_PAD * 2.0,
                        eh,
                    ],
                    (true, true) => [
                        body[0] + PANE_PAD,
                        body[1] + PANE_PAD - 4.0,
                        pr.rect[2] - PANE_PAD * 2.0,
                        eh,
                    ],
                    (false, false) => [
                        pr.rect[0] + side_pad,
                        h - eh,
                        pr.rect[2] - side_pad * 2.0,
                        eh,
                    ],
                    (false, true) => [
                        pr.rect[0] + side_pad,
                        pr.rect[1] + 4.0,
                        pr.rect[2] - side_pad * 2.0,
                        eh,
                    ],
                };
                let anchor = editor::render(
                    layer,
                    engine,
                    &theme,
                    config_ref,
                    pane,
                    area,
                    cell_w,
                    blink && is_active_pane,
                    is_active_pane,
                    if is_active_pane && autosuggest {
                        &completion_ghost
                    } else {
                        ""
                    },
                    shell_env.as_deref().filter(|_| error_underline),
                    if is_active_pane {
                        pane.correction.as_deref()
                    } else {
                        None
                    },
                    ed_cols,
                );
                if is_active_pane {
                    editor_anchor = anchor.0;
                    editor_zone = area;
                    editor_pane = body;
                    ime_spot = Some(anchor.1);
                }
                // the chips that do something when clicked
                for (chip, rect) in &anchor.2 {
                    use shift_core::PromptChip as C;
                    if matches!(
                        chip,
                        C::Cwd
                            | C::GitBranch
                            | C::GitDiff
                            | C::GithubPr
                            | C::Python
                            | C::Php
                            | C::Docker
                    ) {
                        hits.push(Hit {
                            rect: *rect,
                            target: ClickTarget::Chip(pr.id, *chip),
                        });
                    }
                }
                hits.push(Hit {
                    rect: area,
                    target: ClickTarget::Editor(pr.id),
                });
                // a fresh session: the welcome list above the command line,
                // until its first command
                let fresh = !booting
                    && pane
                        .session
                        .blocks
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|b| b.command.trim().is_empty());
                let above = if input_top {
                    [
                        pr.rect[0],
                        area[1] + area[3],
                        pr.rect[2],
                        pr.rect[1] + pr.rect[3] - area[1] - area[3],
                    ]
                } else {
                    [pr.rect[0], pr.rect[1], pr.rect[2], area[1] - pr.rect[1]]
                };
                let entries = welcome_entries(keymap_ref);
                if fresh
                    && !split
                    && above[3] >= crate::views::welcome::height(entries.len()) + 40.0
                    && above[2] >= 420.0
                {
                    let cwd = pane
                        .session
                        .cwd
                        .lock()
                        .unwrap()
                        .as_deref()
                        .map(editor::shorten_home)
                        .unwrap_or_default();
                    let mut subtitle = vec![
                        shift_core::shell_integration::shell_name(config_ref.shell.as_deref()),
                        cwd,
                    ];
                    if let Some(g) = &pane.git {
                        subtitle.push(g.branch.clone());
                    }
                    let rects = crate::views::welcome::render(
                        layer,
                        engine,
                        &theme,
                        above,
                        &subtitle.join(" · "),
                        &entries,
                        chrome_mouse,
                    );
                    for (i, rect) in rects.into_iter().enumerate() {
                        hits.push(Hit {
                            rect,
                            target: ClickTarget::Welcome(i as u8),
                        });
                    }
                }
            } else if is_active_pane {
                // raw mode: the program's cursor cell
                let (line, col) = {
                    let term = pane.session.term.lock();
                    let p = term.grid().cursor.point;
                    (p.line.0, p.column.0)
                };
                let scrolled = pane
                    .session
                    .scrolled
                    .load(std::sync::atomic::Ordering::Acquire);
                let abs = scrolled as i64 + line as i64;
                ime_spot = pane.rows.iter().find_map(|r| match r.kind {
                    terminal::RowKind::Grid { abs: a, content_x } if a as i64 == abs => {
                        Some((content_x + col as f32 * cell_w, r.y))
                    }
                    _ => None,
                });
            }

            // clip pane text to its rect
            if split {
                for t in &mut layer.texts[text_start..] {
                    t.bounds = Some(TextBounds {
                        left: (pr.rect[0] + 2.0) as i32,
                        top: body[1] as i32,
                        right: (pr.rect[0] + pr.rect[2] - 2.0) as i32,
                        bottom: (pr.rect[1] + pr.rect[3] - 2.0) as i32,
                    });
                }
            } else if explorer_w > 0.0 || review_w > 0.0 {
                // side panels: keep long lines (editor context, output) off
                // them, intersecting any clip the text already has
                let (left, right) = (pr.rect[0] as i32, (pr.rect[0] + pr.rect[2]) as i32);
                for t in &mut layer.texts[text_start..] {
                    let b = t.bounds.unwrap_or(TextBounds {
                        left,
                        top: 0,
                        right,
                        bottom: h as i32,
                    });
                    t.bounds = Some(TextBounds {
                        left: b.left.max(left),
                        right: b.right.min(right),
                        ..b
                    });
                }
            }

            // sticky command header (active pane only; layer 2 covers row text).
            // Not over a running command followed live: its program owns the
            // whole PTY height, so the header would hide its top row (vttest,
            // progress UIs redraw there); scrolled up, it shows again.
            // (from this frame's blocks snapshot, not a second lock of them)
            let followed_live = pane.view_offset == 0 && sticky_open;
            if is_active_pane
                && let Some(sticky) = sticky_block.filter(|_| !followed_live)
                && let Some(rect) = terminal::render_sticky_header(
                    scene.layer(2),
                    engine,
                    &theme,
                    &view,
                    &pane.session,
                    sticky,
                    pane.selected_block == Some(sticky),
                )
            {
                pane.sticky_hit = Some((rect, sticky));
                hits.push(Hit {
                    rect,
                    target: ClickTarget::JumpToBlock(pr.id, sticky),
                });
            }

            // inactive panes recede under a dim; the focused one's title bar
            // carries the accent edge
            if split && !is_active_pane {
                scene.layer(2).rect(
                    [pr.rect[0], pr.rect[1]],
                    [pr.rect[2], pr.rect[3]],
                    inactive_dim(&theme),
                    0.0,
                );
            }
        }

        // ── layer 1: side panels beside the panes ──
        self.explorer_hits = if explorer_w > 0.0 && !settings_open {
            crate::views::explorer::render(
                scene.layer(1),
                engine,
                &theme,
                &mut self.explorer,
                [rail_w, TAB_BAR_H, explorer_w, h - TAB_BAR_H],
            )
        } else {
            Vec::new()
        };
        self.review_hits = if self.review.open && self.review.maximized && !settings_open {
            // over the panes and their overlays (layer 2), under the palette
            crate::views::review_panel::render(
                scene.layer(3),
                engine,
                &theme,
                &mut self.review,
                [rail_w, TAB_BAR_H, w - rail_w, h - TAB_BAR_H],
            )
        } else if review_w > 0.0 && !settings_open {
            crate::views::review_panel::render(
                scene.layer(1),
                engine,
                &theme,
                &mut self.review,
                [w - review_w, TAB_BAR_H, review_w, h - TAB_BAR_H],
            )
        } else {
            Vec::new()
        };
        // ── layer 2: the file viewer in its own pane of the split ──
        self.viewer_hits.clear();
        self.viewer_rect = [0.0; 4];
        if let (Some(label), Some(rect), false) = (&viewer_label, viewer_pane, settings_open) {
            self.viewer_rect = rect;
            self.viewer_hits = crate::views::viewer::render(
                scene.layer(2),
                engine,
                &theme,
                &mut self.viewer,
                label,
                rect,
                viewer_has_focus,
            );
            // focus marks as for a shell pane; the dim goes one layer up so
            // it also covers the viewer's own text (layer 2 text draws above
            // layer 2 quads)
            if viewer_has_focus {
                scene
                    .layer(2)
                    .rect([rect[0], rect[1]], [rect[2], 2.0], theme.accent, 0.0);
            } else {
                scene.layer(3).rect(
                    [rect[0], rect[1]],
                    [rect[2], rect[3]],
                    inactive_dim(&theme),
                    0.0,
                );
            }
        }

        // visual bell: brief full-window flash
        if bell_flash {
            scene
                .layer(2)
                .rect([0.0, 0.0], [w, h], theme.fg.with_alpha(0.10), 0.0);
        }

        // ── layer 2: history pane, docked like the completion panel ──
        if editor_active && self.history.open && !palette.open {
            // first in the hit list: the panel covers rows (the welcome
            // list) whose hits would otherwise take its clicks
            let mut panel_hits = Vec::new();
            crate::views::history::render(
                scene.layer(2),
                engine,
                &theme,
                &self.history,
                editor_zone,
                editor_pane,
                chrome_mouse,
                &mut panel_hits,
            );
            hits.splice(0..0, panel_hits);
        }
        // ── layer 2: completion popup (above editor) ──
        if editor_active && completion.open && !palette.open && !self.history.open {
            let (prefix_len, line_len) = tabs
                .get(active_tab_idx)
                .and_then(|t| t.active())
                .map(|p| {
                    // the whole word is the completed prefix (see `CompletionRequest::run`)
                    let (b, c) = (&p.editor.buffer, p.editor.cursor);
                    let (start, _) = shift_core::completion::current_token(b, c);
                    (shift_core::completion::token_end(b, c) - start, b.len())
                })
                .unwrap_or((0, 0));
            let mut panel_hits = Vec::new();
            completion_view::render(
                scene.layer(2),
                engine,
                &theme,
                completion,
                editor_zone,
                editor_pane,
                prefix_len,
                line_len,
                chrome_mouse,
                &mut panel_hits,
            );
            hits.splice(0..0, panel_hits);
        }

        // ── layer 2: IME composition (preedit) at the cursor ──
        if !self.preedit.is_empty()
            && let Some((px, py)) = ime_spot
        {
            let size = if editor_active {
                editor::FONT_SIZE
            } else {
                font_size
            };
            let tw = engine.measure(
                &self.preedit,
                size,
                shift_renderer::mono_family(),
                shift_renderer::glyphon::Weight::NORMAL,
            );
            let layer2 = scene.layer(2);
            layer2.rect(
                [px - 1.0, py - 1.0],
                [tw + 2.0, line_h.max(20.0)],
                theme.bg,
                2.0,
            );
            layer2.rect([px, py + size + 3.0], [tw, 1.5], theme.accent, 0.0);
            let buf = {
                use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping};
                let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.4));
                b.set_text(
                    &mut engine.font_system,
                    &self.preedit,
                    &Attrs::new().family(Family::Name(shift_renderer::mono_family())),
                    Shaping::Advanced,
                    None,
                );
                b
            };
            layer2.text(buf, px, py, theme.fg);
        }
        self.ime_spot = ime_spot;

        // ── layer 2: search bar ──
        if search_state.open && !settings_open {
            let filter = tabs
                .get(active_tab_idx)
                .and_then(|t| t.active())
                .and_then(|p| p.filter.clone());
            let chips = match (filter_editing, filter) {
                (true, Some((_, q))) => search::render(
                    scene.layer(2),
                    engine,
                    &theme,
                    &q,
                    "Enter keep · Esc clear",
                    Some("filter"),
                    w,
                    blink,
                    None,
                ),
                _ => search::render(
                    scene.layer(2),
                    engine,
                    &theme,
                    &search_state.query,
                    &search::counter(search_state),
                    None,
                    w,
                    blink,
                    Some((search_state.case_sensitive, search_state.regex)),
                ),
            };
            self.search_chips = chips;
        } else {
            self.search_chips.clear();
        }

        // ── layer 3: the Settings tab's page ──
        if settings_open {
            // it covers the content area: only the tab strip (and rail)
            // stays clickable beside its controls
            let area = [rail_w, TAB_BAR_H, w - rail_w, h - TAB_BAR_H];
            hits.retain(|h| h.rect[1] + h.rect[3] <= TAB_BAR_H || h.rect[0] + h.rect[2] <= rail_w);
            focus_hits.clear();
            let mut list = shift_renderer::Layer::default();
            settings_state.view_h = area[3] - 34.0;
            settings_state.content_h = settings::render(
                scene.layer(3),
                &mut list,
                engine,
                &theme,
                settings_state,
                config_ref,
                keymap_ref,
                area,
                chrome_mouse,
                blink,
                &mut hits,
                settings_names,
                settings_stops,
            );
            let l4 = scene.layer(4);
            l4.quads.extend(list.quads);
            l4.texts.extend(list.texts);
            // keyboard focus ring around the focused control
            if let Some(focus) = &settings_state.focus
                && let Some(hit) = hits.iter().find(|h| h.target == *focus)
            {
                let r = hit.rect;
                scene.layer(3).bordered(
                    [r[0] - 2.0, r[1] - 2.0],
                    [r[2] + 4.0, r[3] + 4.0],
                    theme.accent.with_alpha(0.0),
                    theme.accent,
                    6.0,
                    2.0,
                );
            }
        }

        // ── layer 4: command palette ──
        palette_row_hits.clear();
        if palette.open {
            palette::render(
                scene.layer(4),
                engine,
                &theme,
                palette,
                palette_results,
                w,
                h,
                blink,
                &mut hits,
                palette_row_hits,
            );
        }

        // ── layer 5: context menu ──
        if self.context_menu.open {
            self.context_menu_rects = context_menu::render(
                scene.layer(5),
                engine,
                &theme,
                &self.context_menu,
                keymap_ref,
                w,
                h,
            );
        } else {
            self.context_menu_rects.clear();
        }
        (self.chip_menu_card, self.chip_menu_rows) = match &self.chip_menu {
            Some(menu) => crate::views::chip_menu::render(
                scene.layer(5),
                engine,
                &theme,
                menu,
                [w, h],
                TAB_BAR_H + 4.0,
                self.mouse,
                blink,
            ),
            None => ([0.0; 4], Vec::new()),
        };
        // notifications popover, dropped from the inbox button; toasts stack
        // in the same corner while it is closed
        let anchor = hits
            .iter()
            .find(|h| matches!(h.target, ClickTarget::ToggleNotifications))
            .map(|h| h.rect)
            .unwrap_or([w - 80.0, 7.0, 28.0, 28.0]);
        // badges rasterize brand marks at device resolution
        let scale = self
            .window
            .as_ref()
            .map_or(1.0, |w| w.scale_factor() as f32);
        self.note_toast_hits = crate::views::notifications::render_toasts(
            scene.layer(5),
            engine,
            &theme,
            &toast_entries,
            anchor[1] + anchor[3] + 10.0,
            w,
            scale,
        );
        if self.notifications.open {
            self.notifications_hits = crate::views::notifications::render(
                scene.layer(5),
                engine,
                &theme,
                &mut self.notifications,
                &notification_entries,
                anchor[0] + anchor[2] + 4.0,
                anchor[1] + anchor[3] + 6.0,
                w,
                h,
                scale,
            );
        } else {
            self.notifications_hits = Default::default();
        }
        if let Some(hs) = &self.hints {
            let items: Vec<(&str, [f32; 4], bool)> = hs
                .items
                .iter()
                .map(|(l, r, t)| (l.as_str(), *r, matches!(t, HintTarget::Open(_))))
                .collect();
            crate::views::hints::render(scene.layer(5), engine, &theme, &items, &hs.typed);
        }

        // ── layer 6: toast + close/quit warning (modal: eats every click) ──
        if let Some(msg) = &toast {
            crate::views::confirm::toast(scene.layer(6), engine, &theme, msg, w, h);
        }
        // What's New sits under a confirm dialog (both modal, layer 6)
        if self.whats_new.open {
            hits.clear();
            focus_hits.clear();
            self.whats_new_hits = crate::views::whats_new::render(
                scene.layer(6),
                engine,
                &theme,
                &mut self.whats_new,
                &self.releases,
                w,
                h,
            );
        }
        if let Some((action, commands)) = &confirm {
            hits.clear();
            use crate::views::confirm::Dialog;
            let update_title;
            let dialog = match action {
                ConfirmAction::Close(scope) => {
                    let (title, label) = match scope {
                        CloseScope::Pane(_) => ("Close pane?", "Close pane"),
                        CloseScope::Tab(_) => ("Close tab?", "Close tab"),
                        CloseScope::OtherTabs(_) => ("Close other tabs?", "Close tabs"),
                        CloseScope::Window => ("Quit Shift?", "Quit"),
                    };
                    let count = commands.len();
                    Dialog {
                        title,
                        description: format!(
                            "{count} running process{} will be terminated.",
                            if count == 1 { "" } else { "es" }
                        ),
                        note: "This cannot be undone. Cancel to keep the session open.",
                        lines: commands,
                        empty_line: "<running shell process>",
                        confirm_label: label,
                        cancel_label: "Cancel",
                        destructive: true,
                        processes: true,
                    }
                }
                ConfirmAction::Paste { .. } => {
                    let count = commands.len();
                    Dialog {
                        title: "Paste and run?",
                        description: format!(
                            "{count} line{} will be sent to this program as typed input.",
                            if count == 1 { "" } else { "s" }
                        ),
                        note: "The program has not enabled bracketed paste, so each newline presses Enter.",
                        lines: commands,
                        empty_line: "<blank line>",
                        confirm_label: "Paste",
                        cancel_label: "Cancel",
                        destructive: false,
                        processes: false,
                    }
                }
                ConfirmAction::InstallRemote(p) => Dialog {
                    title: "Install shell integration?",
                    description: format!("Runs an installer over ssh on {} that writes:", p.host),
                    note: "Hooks load only in Shift (LC_TERMINAL). Remove the files to undo.",
                    lines: commands,
                    empty_line: "",
                    confirm_label: "Install",
                    cancel_label: "Cancel",
                    destructive: false,
                    processes: false,
                },
                ConfirmAction::Revert { hunk, .. } => Dialog {
                    title: if hunk.is_some() {
                        "Revert this change?"
                    } else {
                        "Revert file?"
                    },
                    description: "Unstaged changes will be discarded:".into(),
                    note: "This cannot be undone. Staged changes are kept.",
                    lines: commands,
                    empty_line: "",
                    confirm_label: "Revert",
                    cancel_label: "Cancel",
                    destructive: true,
                    processes: false,
                },
                ConfirmAction::Update(plan) => {
                    update_title = format!("Update to Shift v{}?", plan.manifest.version);
                    Dialog {
                        title: &update_title,
                        description: plan.description.clone(),
                        note: &plan.note,
                        lines: commands,
                        empty_line: "",
                        confirm_label: if plan.open_page {
                            "Open download page"
                        } else {
                            "Update"
                        },
                        cancel_label: "Later",
                        destructive: false,
                        processes: false,
                    }
                }
            };
            self.confirm_rects =
                crate::views::confirm::render(scene.layer(6), engine, &theme, &dialog, w, h, scale);
        }
        self.prompt_editor_layout = match &self.prompt_editor {
            Some(ed) => crate::views::prompt_editor::render(
                scene.layer(6),
                engine,
                &theme,
                ed,
                [w, h],
                self.mouse,
            ),
            None => Default::default(),
        };

        // pane drag: the drop zone under the pointer (the half of a pane, a
        // tab, or the empty strip for a new tab)
        if let Some(d) = self.pane_drag.as_ref().filter(|d| d.live) {
            // the pane being moved fades in place
            if let Some(src) = pane_rects.iter().find(|p| p.id == d.pane) {
                scene.layer(5).rect(
                    [src.rect[0], src.rect[1]],
                    [src.rect[2], src.rect[3]],
                    theme.bg.with_alpha(0.6),
                    0.0,
                );
            }
            let zone = d.target.and_then(|target| match target {
                PaneDrop::Pane(id, edge) => pane_rects
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| (edge.zone(p.rect), edge.label())),
                PaneDrop::Swap(id) => pane_rects
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| (p.rect, "Swap")),
                PaneDrop::Tab(j) => hits
                    .iter()
                    .find(|h| h.target == ClickTarget::TabSelect(j))
                    .map(|h| (h.rect, "")),
                PaneDrop::NewTab if rail_w > 0.0 => {
                    Some(([0.0, TAB_BAR_H, rail_w, h - TAB_BAR_H], "New tab"))
                }
                // a tab-sized slot where the new tab will appear
                PaneDrop::NewTab => hits
                    .iter()
                    .find(|h| h.target == ClickTarget::TabNew)
                    .map(|h| ([h.rect[0], 0.0, 160.0, TAB_BAR_H], "")),
            });
            let layer = scene.layer(5);
            if let Some((zone, label)) = zone {
                crate::views::pane_header::drop_zone(layer, engine, &theme, zone, label);
            }
            // a chip with the pane's title follows the pointer
            let title = if d.pane == VIEWER_PANE {
                viewer_label.clone().unwrap_or_default()
            } else {
                tabs.get(d.from_tab)
                    .and_then(|t| t.pane(d.pane))
                    .map(|p| {
                        if p.title.is_empty() {
                            p.session
                                .cwd
                                .lock()
                                .unwrap()
                                .as_deref()
                                .map(editor::shorten_home)
                                .unwrap_or_default()
                        } else {
                            p.title.clone()
                        }
                    })
                    .unwrap_or_default()
            };
            crate::views::pane_header::drag_ghost(
                scene.layer(6),
                engine,
                &theme,
                &title,
                d.pane == VIEWER_PANE,
                (self.mouse.0 + 14.0, self.mouse.1 + 22.0),
                w,
                h,
            );
        }

        hits.extend(focus_hits);
        self.hits = hits;
        // a background tab's spinner turns too
        self.any_running = any_running
            || self
                .tabs
                .iter()
                .any(|t| t.panes.iter().any(PaneState::running));
        self.editor_active = editor_active;
        self.boot_until = boot_until;
        self.editor_anchor = editor_anchor;
        self.editor_cols = active_ed_cols;
    }

    /// The shortcut of `action`, as key caps (`⇧⌘G`).
    fn tip_keys(&self, action: &str) -> Option<String> {
        self.keymap.preferred(action).map(|b| b.chips().concat())
    }

    /// What a click target does, or its full label; `None` for targets that
    /// explain themselves (rows, the editor, the grid).
    fn tip_for(&self, target: &ClickTarget) -> Option<(String, Option<String>)> {
        use crate::views::history::Scope;
        let pane_of = |id: PaneId| self.tabs.iter().find_map(|t| t.pane(id));
        let s = |t: &str| Some(t.to_string());
        Some(match target {
            ClickTarget::TabSelect(i) => {
                let tab = self.tabs.get(*i)?;
                let title = tab.display_title();
                let cwd = tab
                    .active()
                    .and_then(|p| p.session.cwd.lock().unwrap().clone())
                    .map(|c| editor::shorten_home(&c));
                let keys = (*i < 8)
                    .then(|| self.tip_keys(&format!("tab_select_{}", i + 1)))
                    .flatten();
                (title, cwd.or(keys))
            }
            // ⌘W closes the active pane, and the tab with its last one: the
            // shortcut is true only for the active, unsplit tab
            ClickTarget::TabClose(i) => (
                "Close tab".into(),
                self.tabs
                    .get(*i)
                    .filter(|t| *i == self.active && !t.root.is_split())
                    .and_then(|_| self.tip_keys("close_pane")),
            ),
            ClickTarget::TabNew => ("New tab".into(), self.tip_keys("new_tab")),
            ClickTarget::SettingsEditPrompt => ("Edit prompt".into(), None),
            ClickTarget::ToggleExplorer => {
                ("File explorer".into(), self.tip_keys("toggle_explorer"))
            }
            ClickTarget::ToggleReview => ("Review changes".into(), self.tip_keys("toggle_review")),
            ClickTarget::ToggleNotifications => {
                ("Notifications".into(), self.tip_keys("attention_center"))
            }
            ClickTarget::BlockChevron(pane, block) => {
                let collapsed = pane_of(*pane)?
                    .session
                    .blocks
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|b| b.id == *block)?
                    .collapsed;
                let text = if collapsed {
                    "Expand output"
                } else {
                    "Collapse output"
                };
                (text.into(), None)
            }
            ClickTarget::BlockCopy(..) => {
                ("Copy output".into(), self.tip_keys("copy_block_output"))
            }
            ClickTarget::BlockDuration(pane, block) => {
                let blocks = pane_of(*pane)?.session.blocks.lock().unwrap();
                let b = blocks.iter().find(|b| b.id == *block)?;
                (crate::views::terminal::duration_detail(b)?, None)
            }
            ClickTarget::PaneSplit(_) => ("Split right".into(), self.tip_keys("split_vertical")),
            ClickTarget::PaneMenu(_) => ("More actions".into(), None),
            ClickTarget::PaneClose(_) => ("Close pane".into(), self.tip_keys("close_pane")),
            ClickTarget::JumpToBlock(..) => ("Scroll to this command".into(), None),
            ClickTarget::HistoryScope(scope) => (
                match scope {
                    Scope::Pane => "Commands run in this pane",
                    Scope::All => "Commands from every shell",
                    Scope::Failed => "Commands that failed",
                }
                .into(),
                s("⌃R"),
            ),
            ClickTarget::HistoryClose => ("Close".into(), s("esc")),
            ClickTarget::SettingsReset(_) | ClickTarget::SettingsKeybindReset(_) => {
                ("Reset to default".into(), None)
            }
            ClickTarget::SettingsCopyVersion => ("Copy version".into(), None),
            ClickTarget::UpdateChip => (format!("v{} available", self.update_chip()?), None),
            ClickTarget::SettingsOpenConfig => (
                "Open in the default editor".into(),
                shift_core::config::config_path()
                    .map(|p| editor::shorten_home(&p.to_string_lossy())),
            ),
            ClickTarget::TrafficLight(i) => (
                match i {
                    0 => "Close window",
                    1 => "Minimize",
                    _ => "Zoom",
                }
                .into(),
                None,
            ),
            _ => return None,
        })
    }

    /// This frame's tooltips, in click priority: the side panels and the
    /// viewer first (they own their regions), then `self.hits` in order. A
    /// hit without a tooltip still blocks the ones under it.
    fn collect_tips(&self) -> Vec<crate::views::tooltip::Tip> {
        use crate::views::explorer::ExplorerHit;
        use crate::views::review_panel::ReviewHit;
        use crate::views::tooltip::Tip;
        use crate::views::viewer::ViewerHit;
        let mut tips = Vec::new();
        // the prompt editor is modal: only its chips explain themselves
        if let Some(ed) = &self.prompt_editor {
            use crate::views::prompt_editor::Target;
            for (rect, target) in &self.prompt_editor_layout.hits {
                let chip = match target {
                    Target::Palette(chip) => Some(*chip),
                    Target::Chip(i) => ed.draft.get(*i).copied(),
                    _ => None,
                };
                tips.push(match chip {
                    Some(c) => Tip::card(*rect, c.label(), c.about()),
                    None => Tip::new(*rect, "", None),
                });
            }
            tips.push(Tip::new(self.prompt_editor_layout.card, "", None));
            return tips;
        }
        for (rect, hit) in &self.viewer_hits {
            let (text, keys) = match hit {
                ViewerHit::Close => ("Close file", self.tip_keys("close_pane")),
                ViewerHit::Menu => ("More actions", None),
                _ => ("", None),
            };
            tips.push(Tip::new(*rect, text, keys));
        }
        for (rect, hit) in &self.explorer_hits {
            tips.push(match hit {
                ExplorerHit::Refresh => Tip::new(*rect, "Refresh", None),
                ExplorerHit::Collapse => Tip::new(*rect, "Collapse all", None),
                ExplorerHit::Row(i) => match self.explorer.rows.get(*i) {
                    // the full path of a name the panel may cut short
                    Some(row) => Tip::new(
                        *rect,
                        editor::shorten_home(&row.path.to_string_lossy()),
                        None,
                    ),
                    None => Tip::new(*rect, "", None),
                },
            });
        }
        for (rect, hit) in &self.review_hits {
            let text = match hit {
                ReviewHit::Refresh => "Refresh",
                ReviewHit::Stage(_, None) => "Stage file",
                ReviewHit::Stage(_, Some(_)) => "Stage hunk",
                ReviewHit::Unstage(_, None) => "Unstage file",
                ReviewHit::Unstage(_, Some(_)) => "Unstage hunk",
                ReviewHit::Revert(_, None) => "Discard changes to the file",
                ReviewHit::Revert(_, Some(_)) => "Discard the hunk",
                ReviewHit::StageAll => "Stage all",
                ReviewHit::UnstageAll => "Unstage all",
                ReviewHit::Open(_) => "Open file",
                ReviewHit::Copy(..) => "Copy hunk",
                ReviewHit::Maximize if self.review.maximized => "Restore",
                ReviewHit::Maximize => "Maximize",
                ReviewHit::Jump(_, path) => path,
                _ => "",
            };
            tips.push(Tip::new(*rect, text, None));
        }
        if self.review_maximized() {
            // the panes' controls under the panel show no tooltip
            tips.push(Tip::new(self.panel_rects().1, "", None));
        }
        for hit in &self.hits {
            // a prompt chip's card: what it shows and what a click does
            if let ClickTarget::Chip(_, chip) = hit.target {
                tips.push(Tip::card(hit.rect, chip.label(), chip.about()));
                continue;
            }
            let (text, detail) = self.tip_for(&hit.target).unwrap_or_default();
            tips.push(Tip::new(hit.rect, text, detail));
        }
        tips
    }

    /// No tooltip while a modal, a menu or a drag owns the pointer.
    fn tooltip_blocked(&self) -> bool {
        !self.pointer_in
            || self.palette.open
            || self.confirm.is_some()
            || self.context_menu.open
            || self.chip_menu.is_some()
            // the prompt editor's chips have cards, until one is dragged
            || self
                .prompt_editor
                .as_ref()
                .is_some_and(|e| e.drag.as_ref().is_some_and(|d| d.live))
            || self.whats_new.open
            || self.rename.is_some()
            || self.drag.is_some()
            || self.pending_sel.is_some()
            || self.editor_drag.is_some()
            || self.tab_drag.is_some()
            || self.pane_drag.is_some()
            || self.slider_drag.is_some()
    }

    /// Track the control under the pointer and draw its tooltip once it has
    /// rested there for `tooltip::DELAY` (at once right after another one).
    fn tooltip_pass(&mut self, w: f32, h: f32) {
        use crate::views::tooltip::{DELAY, WARM};
        let tips = self.collect_tips();
        let (mx, my) = self.mouse;
        let hovered = if self.tooltip_blocked() {
            None
        } else {
            tips.iter().find(|t| t.contains(mx, my)).cloned()
        };
        let now = Instant::now();
        let rect = hovered.as_ref().map(|t| t.rect);
        if self.tip_hover.map(|(r, _)| r) != rect {
            if self.tip_shown.is_some() {
                self.tip_hidden_at = Some(now);
            }
            let warm = self.tip_hidden_at.is_some_and(|t| now < t + WARM);
            self.tip_hover = rect.map(|r| {
                (
                    r,
                    if warm {
                        now.checked_sub(DELAY).unwrap_or(now)
                    } else {
                        now
                    },
                )
            });
            self.tip_suppressed = false;
        }
        let shown = match (hovered, self.tip_hover) {
            (Some(tip), Some((_, since)))
                if !tip.text.is_empty() && !self.tip_suppressed && now >= since + DELAY =>
            {
                Some(tip)
            }
            _ => None,
        };
        if self.tip_shown.is_some() && shown.is_none() && rect.is_some() {
            self.tip_hidden_at = Some(now);
        }
        if let (Some(tip), Some(engine)) = (&shown, self.text.as_mut()) {
            crate::views::tooltip::render(self.scene.layer(7), engine, &self.theme, tip, w, h);
        }
        self.tip_shown = shown;
        self.tips = tips;
    }

    /// When the tooltip of the control under the pointer is due (not yet drawn).
    fn tooltip_due(&self) -> Option<Instant> {
        let (_, since) = self.tip_hover?;
        (self.tip_shown.is_none() && !self.tip_suppressed)
            .then_some(since + crate::views::tooltip::DELAY)
    }

    /// A press, key or wheel: the tooltip goes until the pointer moves on.
    fn hide_tooltip(&mut self, window: &Window) {
        self.tip_suppressed = true;
        if self.tip_shown.is_some() {
            window.request_redraw();
        }
    }

    /// A frame for new program output: at once, or [`OUTPUT_FRAME`] after
    /// the last one.
    fn request_output_frame(&mut self) {
        let due = self.last_frame + OUTPUT_FRAME;
        if Instant::now() >= due {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        } else {
            self.output_frame_due.get_or_insert(due);
        }
    }

    fn redraw(&mut self, window: &Window) {
        self.last_frame = Instant::now();
        self.output_frame_due = None;
        let scale = window.scale_factor() as f32;
        if self.gpu.is_none() {
            return;
        }
        self.request_git();

        let (pw, ph) = {
            let gpu = self.gpu.as_ref().unwrap();
            (gpu.config.width, gpu.config.height)
        };
        let (lw, lh) = (pw as f32 / scale, ph as f32 / scale);

        self.build_scene(lw, lh);
        self.tooltip_pass(lw, lh);
        if let Some(spot) = self.ime_spot
            && self.ime_area_sent != Some(spot)
        {
            self.ime_area_sent = Some(spot);
            window.set_ime_cursor_area(
                winit::dpi::LogicalPosition::new(spot.0, spot.1),
                LogicalSize::new(self.cell_w, self.line_h),
            );
        }

        let mut all_quads = Vec::new();
        let mut ranges = Vec::new();
        for layer in &self.scene.layers {
            let start = all_quads.len() as u32;
            all_quads.extend_from_slice(&layer.quads);
            ranges.push(start..all_quads.len() as u32);
        }

        let (Some(gpu), Some(quads), Some(text), Some(sprites)) = (
            &mut self.gpu,
            &mut self.quads,
            &mut self.text,
            &mut self.sprites,
        ) else {
            return;
        };
        let sprite_layers: Vec<&[shift_renderer::Sprite]> = self
            .scene
            .layers
            .iter()
            .map(|l| l.sprites.as_slice())
            .collect();
        sprites.prepare(&gpu.device, &gpu.queue, [lw, lh], &sprite_layers);
        // prepared once; drawn first in every pass (main + capture)
        if let Some(img) = self.bg_image.as_mut()
            && img.has_image()
        {
            // translucent (Warp): the image at the window opacity, veiled by
            // bg in `build_scene`; opaque: the image over the bg clear
            let alpha = if self.theme.ground_alpha < 1.0 {
                self.theme.ground_alpha
            } else {
                self.bg_image_opacity
            };
            img.prepare(&gpu.queue, [pw as f32, ph as f32], alpha);
        }
        let bg_image = self.bg_image.as_ref().filter(|p| p.has_image());

        quads.prepare(&gpu.device, &gpu.queue, [lw, lh], &all_quads);

        let text_layers: Vec<Vec<TextArea<'_>>> = self
            .scene
            .layers
            .iter()
            .map(|layer| {
                layer
                    .texts
                    .iter()
                    .map(|t| TextArea {
                        buffer: t.buffer.as_ref(),
                        left: t.left * scale,
                        top: t.top * scale,
                        scale,
                        bounds: t
                            .bounds
                            .map(|b| TextBounds {
                                left: (b.left as f32 * scale) as i32,
                                top: (b.top as f32 * scale) as i32,
                                right: (b.right as f32 * scale) as i32,
                                bottom: (b.bottom as f32 * scale) as i32,
                            })
                            .unwrap_or(TextBounds {
                                left: 0,
                                top: 0,
                                right: pw as i32,
                                bottom: ph as i32,
                            }),
                        default_color: t.color,
                        custom_glyphs: &[],
                    })
                    .collect()
            })
            .collect();
        text.prepare_layers(&gpu.device, &gpu.queue, (pw, ph), text_layers);
        // the window ground; with a translucent background image the image
        // and its veil are the ground (Warp draws no surface_2 under them)
        let ground = if bg_image.is_some() && self.theme.ground_alpha < 1.0 {
            self.theme.bg.with_alpha(0.0)
        } else {
            self.theme.window_ground()
        };

        // script `shot`: offscreen, so it works even when the window
        // surface is unavailable (occluded window, display asleep)
        if let Some(path) = self.capture.take() {
            crate::capture::capture_frame(
                gpu, quads, text, sprites, bg_image, &ranges, ground, &path,
            );
        }
        let Some(frame) = gpu.acquire() else {
            text.trim();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(gpu.clear_color(ground)),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(img) = bg_image {
                img.render(&mut pass);
            }
            for (i, range) in ranges.iter().cloned().enumerate() {
                quads.render_range(&mut pass, range);
                sprites.render_layer(i, true, &mut pass);
                text.render_layer(i, &mut pass);
                sprites.render_layer(i, false, &mut pass);
            }
        }
        gpu.queue.submit([encoder.finish()]);
        window.pre_present_notify();
        frame.present();
        text.trim();
        if self.update.confirm_after.is_some() {
            self.on_first_frame();
        }
        // SHIFT_LATENCY_LOG: key → program output → presented frame
        if let Some((t, true)) = self.latency_probe {
            log::info!(
                "latency: key-to-frame {:.2}ms",
                t.elapsed().as_secs_f64() * 1e3
            );
            self.latency_probe = None;
        }
    }

    fn handle_action(&mut self, action: AppAction, event_loop: &ActiveEventLoop) {
        match action {
            AppAction::NewTab => self.spawn_tab(),
            AppAction::ClosePane => {
                // ⌘W on the viewer pane closes the viewer
                if self.viewer_focused() {
                    self.viewer.close();
                } else if let Some(tab) = self.tabs.get(self.active) {
                    self.request_close(CloseScope::Pane(tab.active_pane), event_loop);
                }
            }
            AppAction::CloseWindow | AppAction::Quit => {
                self.request_close(CloseScope::Window, event_loop);
            }
            AppAction::CloseOtherTabs => {
                self.request_close(CloseScope::OtherTabs(self.active), event_loop);
            }
            AppAction::NewWindow => self.open_new_window(),
            AppAction::Minimize => {
                if let Some(w) = self.shown_window() {
                    w.set_minimized(true);
                }
            }
            AppAction::HideApp => {
                // a headless run has nothing on screen to hide
                #[cfg(target_os = "macos")]
                if !self.headless {
                    use winit::platform::macos::ActiveEventLoopExtMacOS;
                    event_loop.hide_application();
                }
                #[cfg(not(target_os = "macos"))]
                if let Some(w) = self.shown_window() {
                    w.set_minimized(true);
                }
            }
            AppAction::ToggleFullscreen => {
                if let Some(w) = self.shown_window() {
                    let full = w.fullscreen().is_some();
                    w.set_fullscreen(
                        (!full).then_some(winit::window::Fullscreen::Borderless(None)),
                    );
                }
            }
            AppAction::RenameTab => {
                self.start_rename(self.active);
            }
            AppAction::ToggleSyncInputs => {
                if let Some(t) = self.tabs.get_mut(self.active) {
                    t.sync_inputs = !t.sync_inputs;
                }
            }
            AppAction::FilterBlock => {
                if let Some((pane_id, block)) = self.selected_or_last_block() {
                    self.toggle_block_filter(pane_id, block);
                }
            }
            AppAction::SelectAll => {
                let editor_active = self.editor_active;
                if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                    if editor_active && !pane.editor.buffer.is_empty() {
                        pane.editor.select_all();
                    } else {
                        // select the whole scrollback + screen
                        let mut term = pane.session.term.lock();
                        use shift_pty::alacritty_terminal::grid::Dimensions;
                        let top = ALine(-(term.grid().history_size() as i32));
                        let bottom = ALine(term.grid().screen_lines() as i32 - 1);
                        let right = ACol(term.grid().columns().saturating_sub(1));
                        let mut sel = Selection::new(
                            SelectionType::Simple,
                            APoint::new(top, ACol(0)),
                            ASide::Left,
                        );
                        sel.update(APoint::new(bottom, right), ASide::Right);
                        term.selection = Some(sel);
                        drop(term);
                        pane.has_selection = true;
                    }
                }
            }
            AppAction::SaveLaunchConfig => self.save_current_layout(),
            AppAction::SessionSwitcher => {
                self.open_palette(crate::views::palette::PaletteMode::Sessions)
            }
            AppAction::PipeBlockOutput => self.open_pipe(),
            AppAction::ToggleTabLayout => {
                self.config.tab_layout = match self.config.tab_layout {
                    shift_core::TabLayout::Horizontal => shift_core::TabLayout::Vertical,
                    shift_core::TabLayout::Vertical => shift_core::TabLayout::Horizontal,
                };
                self.save_config();
                self.apply_config();
            }
            AppAction::HintMode => self.start_hints(),
            AppAction::AttentionCenter => {
                if self.notifications.open {
                    self.close_notifications();
                } else {
                    self.open_notifications();
                }
            }
            AppAction::ClearTranscripts => self.clear_transcripts(),
            AppAction::TabNext => {
                if !self.tabs.is_empty() {
                    self.switch_tab((self.active + 1) % self.tabs.len());
                }
            }
            AppAction::TabPrev => {
                if !self.tabs.is_empty() {
                    self.switch_tab((self.active + self.tabs.len() - 1) % self.tabs.len());
                }
            }
            AppAction::TabSelect(n) => {
                if !self.tabs.is_empty() {
                    // Warp: ⌘9 = last tab; out-of-range numbers are no-ops
                    let idx = if n == 9 {
                        self.tabs.len() - 1
                    } else {
                        n as usize - 1
                    };
                    if idx < self.tabs.len() && idx != self.active {
                        self.switch_tab(idx);
                    }
                }
            }
            AppAction::MoveTabLeft | AppAction::MoveTabRight => {
                let n = self.tabs.len();
                let target = if action == AppAction::MoveTabLeft {
                    self.active.checked_sub(1)
                } else {
                    (self.active + 1 < n).then_some(self.active + 1)
                };
                if let Some(t) = target {
                    self.tabs.swap(self.active, t);
                    self.active = t;
                    self.save_session();
                }
            }
            AppAction::ReopenClosedTab => {
                if let Some(cwd) = self.closed_tabs.pop() {
                    self.spawn_tab_at(cwd);
                }
            }
            AppAction::ScrollUp | AppAction::ScrollDown => {
                let delta = if action == AppAction::ScrollUp {
                    10i64
                } else {
                    -10
                };
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && let Some(pane) = tab.active_mut()
                {
                    pane.view_offset = (pane.view_offset as i64 + delta).max(0) as usize;
                }
            }
            AppAction::SplitRight => self.split_active(Axis::Horizontal),
            AppAction::SplitDown => self.split_active(Axis::Vertical),
            AppAction::TogglePalette => {
                if self.palette.open {
                    self.palette.open = false;
                } else {
                    self.open_palette(crate::views::palette::PaletteMode::Commands);
                }
            }
            AppAction::OpenSettings => {
                if self.settings.open {
                    self.settings.open = false;
                } else {
                    self.settings.open_page(self.settings.page);
                }
            }
            AppAction::BookmarkToggle => {
                // selected block wins; else the most recent finished block
                if let Some((pane_id, id)) = self.selected_or_last_block()
                    && let Some(pane) = self.tabs.get(self.active).and_then(|t| t.pane(pane_id))
                {
                    pane.session.blocks.lock().unwrap().toggle_bookmark(id);
                }
            }
            AppAction::BookmarkPrev => self.jump_bookmark(-1),
            AppAction::BookmarkNext => self.jump_bookmark(1),
            AppAction::Copy => {
                let editor_sel = self
                    .tabs
                    .get(self.active)
                    .and_then(|t| t.active())
                    .filter(|_| self.editor_active)
                    .and_then(|p| p.editor.selected_text().map(str::to_string));
                if let Some(text) = editor_sel {
                    self.copy_to_clipboard(text);
                    return;
                }
                if let Some(text) = self.selection_text() {
                    self.copy_to_clipboard(text);
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                // no selection: copy the last finished block's output
                let text = self
                    .tabs
                    .get(self.active)
                    .and_then(|t| t.active())
                    .and_then(|p| {
                        let id = p
                            .session
                            .blocks
                            .lock()
                            .unwrap()
                            .iter()
                            .rev()
                            .find(|b| b.output_range().is_some())
                            .map(|b| b.id)?;
                        terminal::block_output_text(&p.session, id)
                    });
                if let Some(text) = text {
                    self.copy_to_clipboard(text);
                }
            }
            AppAction::Paste => self.paste_from_clipboard(),
            // with a file open, Find searches the file, not the terminal
            AppAction::Search if self.viewer_focused() => {
                if self.viewer.find.is_some() {
                    self.viewer.find = None;
                } else {
                    self.viewer.open_find();
                }
                self.blink_epoch = Instant::now();
            }
            AppAction::Search => {
                if self.search.open {
                    self.search.close();
                } else {
                    self.search.open = true;
                    self.refresh_search(true);
                    self.blink_epoch = Instant::now();
                }
            }
            AppAction::FocusLeft
            | AppAction::FocusRight
            | AppAction::FocusUp
            | AppAction::FocusDown => {
                let dir = match action {
                    AppAction::FocusLeft => (-1.0, 0.0),
                    AppAction::FocusRight => (1.0, 0.0),
                    AppAction::FocusUp => (0.0, -1.0),
                    _ => (0.0, 1.0),
                };
                // navigating unzooms first, then moves within the real layout
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.zoomed = None;
                }
                let (lw, lh) = self.logical_size();
                let (rects, _) = self.pane_rects(lw, lh);
                let from_viewer = self.viewer_focused();
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    let from = if from_viewer {
                        VIEWER_PANE
                    } else {
                        tab.active_pane
                    };
                    match focus_neighbor(&rects, from, dir) {
                        // the viewer pane takes focus without being a shell
                        Some(VIEWER_PANE) => self.viewer_focus = true,
                        Some(next) => {
                            if let Some(prev) = tab.active_mut() {
                                prev.selected_block = None;
                            }
                            tab.active_pane = next;
                            self.viewer_focus = false;
                        }
                        None => {}
                    }
                }
            }
            AppAction::ClearBlocks => {
                if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut())
                    && !pane.alt_screen
                {
                    pane.session.clear_screen_and_blocks();
                    pane.view_offset = 0;
                    pane.last_display_len = 0;
                    pane.selected_block = None;
                    self.search.set_matches(terminal::Found::default());
                }
            }
            AppAction::FontIncrease | AppAction::FontDecrease | AppAction::FontReset => {
                self.config.font_size = match action {
                    AppAction::FontIncrease => (self.config.font_size() + 1.0).clamp(8.0, 28.0),
                    AppAction::FontDecrease => (self.config.font_size() - 1.0).clamp(8.0, 28.0),
                    _ => DEFAULT_FONT_SIZE,
                };
                self.save_config();
                self.apply_config();
            }
            AppAction::FocusInput => {
                if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                    pane.view_offset = 0;
                    pane.selected_block = None;
                }
                self.blink_epoch = Instant::now();
            }
            AppAction::ScrollPageUp
            | AppAction::ScrollPageDown
            | AppAction::ScrollTop
            | AppAction::ScrollBottom => {
                if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                    let rows = pane.last_grid.map(|g| g.rows).unwrap_or(24);
                    let max_off = pane.last_display_len.saturating_sub(rows);
                    pane.view_offset = match action {
                        AppAction::ScrollPageUp => (pane.view_offset + rows).min(max_off),
                        AppAction::ScrollPageDown => pane.view_offset.saturating_sub(rows),
                        AppAction::ScrollTop => max_off,
                        _ => 0,
                    };
                }
            }
            AppAction::OpenKeybindings => {
                self.settings
                    .open_page(crate::views::settings::Page::Keybindings);
            }
            AppAction::OpenAbout => {
                self.palette.open = false;
                self.context_menu.close();
                self.settings.open_page(crate::views::settings::Page::About);
            }
            AppAction::CheckForUpdates => {
                self.palette.open = false;
                self.check_for_updates_now();
            }
            AppAction::UpdateShift => {
                self.palette.open = false;
                self.context_menu.close();
                self.update_shift();
            }
            AppAction::ToggleExplorer => {
                self.explorer.open = !self.explorer.open;
                self.explorer.focused = self.explorer.open;
                if self.explorer.open {
                    self.follow_explorer();
                    self.request_review();
                }
            }
            AppAction::ToggleReview => {
                let too_narrow = |app: &Self| {
                    app.review.open
                        && !app.review.maximized
                        && app.panel_widths(app.logical_size().0).1 == 0.0
                };
                // open but auto-hidden: closing it would change nothing
                // visible, so say why it is not showing instead
                if too_narrow(self) {
                    self.toast("Window too narrow for the review panel: widen it".into());
                } else {
                    self.review.open = !self.review.open;
                    self.review.maximized = false;
                    if self.review.open {
                        self.review.notice = None;
                        self.request_review();
                        if too_narrow(self) {
                            self.toast("Window too narrow for the review panel: widen it".into());
                        }
                    }
                }
            }
            AppAction::WhatsNew => {
                self.palette.open = false;
                self.settings.open = false;
                self.context_menu.close();
                self.whats_new.open();
                shift_core::changelog::mark_seen();
            }
            AppAction::MaximizePane => {
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && tab.is_split()
                {
                    tab.zoomed = if tab.zoomed.is_some() {
                        None
                    } else {
                        Some(tab.active_pane)
                    };
                }
            }
            AppAction::ResizePaneLeft
            | AppAction::ResizePaneRight
            | AppAction::ResizePaneUp
            | AppAction::ResizePaneDown => {
                let (axis, delta) = match action {
                    AppAction::ResizePaneLeft => (Axis::Horizontal, -0.05),
                    AppAction::ResizePaneRight => (Axis::Horizontal, 0.05),
                    AppAction::ResizePaneUp => (Axis::Vertical, -0.05),
                    _ => (Axis::Vertical, 0.05),
                };
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && tab.zoomed.is_none()
                    && let Some(ancestors) = tab.root.ancestors_of(tab.active_pane)
                    && let Some((path, _, _)) =
                        ancestors.into_iter().rev().find(|(_, a, _)| *a == axis)
                {
                    tab.root.adjust_ratio(&path, delta);
                }
            }
            AppAction::SelectBlockPrev => self.select_block(-1),
            AppAction::SelectBlockNext => self.select_block(1),
            AppAction::CopyBlockCommand => {
                if let Some((pane_id, block)) = self.selected_or_last_block()
                    && let Some(cmd) = self.block_command(pane_id, block)
                {
                    self.copy_to_clipboard(cmd);
                }
            }
            AppAction::CopyBlockOutput => {
                if let Some((pane_id, block)) = self.selected_or_last_block() {
                    let text = self
                        .tabs
                        .get(self.active)
                        .and_then(|t| t.pane(pane_id))
                        .and_then(|p| terminal::block_output_text(&p.session, block));
                    if let Some(text) = text {
                        self.copy_to_clipboard(text);
                    }
                }
            }
            AppAction::Reinput => {
                if self.editor_active
                    && let Some((pane_id, block)) = self.selected_or_last_block()
                    && let Some(cmd) = self.block_command(pane_id, block)
                    && let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut())
                {
                    pane.editor.clear();
                    pane.editor.insert(&cmd);
                    pane.selected_block = None;
                    pane.view_offset = 0;
                    self.update_completions();
                    self.blink_epoch = Instant::now();
                }
            }
            AppAction::BlockMenu => {
                if let Some((pane_id, block)) = self.selected_or_last_block() {
                    let anchor = self
                        .tabs
                        .get(self.active)
                        .and_then(|t| t.pane(pane_id))
                        .and_then(|p| {
                            let y = p.rows.iter().find_map(|r| match r.kind {
                                crate::views::terminal::RowKind::Header(id) if id == block => {
                                    Some(r.y)
                                }
                                _ => None,
                            })?;
                            let g = p.last_grid?;
                            Some((g.x + 24.0, y + g.line_h))
                        });
                    match anchor {
                        Some(pos) => self.open_context_menu(pane_id, block, pos),
                        None => {
                            // header offscreen: bring it into view, anchor at
                            // the viewport top (rows cache refreshes next frame)
                            self.jump_to_block(pane_id, block);
                            let pos = self
                                .tabs
                                .get(self.active)
                                .and_then(|t| t.pane(pane_id))
                                .and_then(|p| p.last_grid)
                                .map(|g| (g.x + 24.0, g.y + g.line_h))
                                .unwrap_or(self.mouse);
                            self.open_context_menu(pane_id, block, pos);
                        }
                    }
                }
            }
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Switch to tab `idx`, clearing transient state on the pane being left.
    fn switch_tab(&mut self, idx: usize) {
        self.settings.open = false;
        if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
            pane.selected_block = None;
        }
        self.active = idx;
        self.tabs[idx].activity = false;
        self.save_session();
    }

    fn handle_settings_key(
        &mut self,
        event: &input::KeyPress,
        event_loop: &ActiveEventLoop,
        window: &Window,
    ) -> bool {
        use crate::views::settings;
        use winit::keyboard::{Key, NamedKey};

        // an open select list: ↑ ↓ move, ⏎ picks, Esc closes
        if let Some(dd) = self.settings.dropdown.as_mut() {
            let n = dd.options.len();
            match &event.logical_key {
                Key::Named(NamedKey::ArrowUp) => dd.highlight = dd.highlight.saturating_sub(1),
                Key::Named(NamedKey::ArrowDown) => {
                    dd.highlight = (dd.highlight + 1).min(n.saturating_sub(1))
                }
                Key::Named(NamedKey::Enter | NamedKey::Space) => {
                    let dd = self.settings.dropdown.take().unwrap_or_default();
                    if let Some((value, _)) = dd.options.get(dd.highlight) {
                        self.set_select(dd.id, value.clone());
                    }
                }
                _ => self.settings.dropdown = None,
            }
            return true;
        }
        let recording = self.settings.recording.is_some();
        if !recording && self.handle_settings_focus_key(event, event_loop, window) {
            return true;
        }
        if self.settings.page == settings::Page::Keybindings && self.handle_keybindings_key(event) {
            return true;
        }
        let mods = self.modifiers;
        let action = input::app_action(event, mods, &self.keymap);
        // Esc clears the search first, then closes the Settings tab
        if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
            if self.settings.query.is_empty() {
                self.settings.close_tab();
            } else {
                self.settings.query.clear();
                self.settings.scroll = 0.0;
            }
            return true;
        }
        match action {
            Some(AppAction::OpenSettings) => {
                self.settings.open = false;
                self.settings.focus = None;
                return true;
            }
            // ⌘W in the Settings tab closes it
            Some(AppAction::ClosePane) => {
                self.settings.close_tab();
                return true;
            }
            // the terminal's scroll keys (plain Home / End / PageUp /
            // PageDown) move between and scroll the pages here, below
            Some(
                AppAction::ScrollPageUp
                | AppAction::ScrollPageDown
                | AppAction::ScrollTop
                | AppAction::ScrollBottom,
            ) => {}
            // tabs, palette and the rest act as from any tab (leaving this one)
            Some(a) => {
                self.settings.open = false;
                self.settings.focus = None;
                self.handle_action(a, event_loop);
                return true;
            }
            None => {}
        }
        // typing searches every page (Keybindings filters its own list)
        let plain = !mods.control_key() && !mods.super_key() && !mods.alt_key();
        if plain && self.settings.page != settings::Page::Keybindings {
            match &event.logical_key {
                Key::Named(NamedKey::Backspace) if !self.settings.query.is_empty() => {
                    self.settings.query.pop();
                    self.settings.scroll = 0.0;
                    return true;
                }
                Key::Named(NamedKey::Space) if !self.settings.query.is_empty() => {
                    self.settings.query.push(' ');
                    return true;
                }
                _ => {
                    if let Some(text) = &event.text
                        && !text.is_empty()
                        && !text.chars().any(|c| c.is_control() || c == ' ')
                    {
                        self.settings.query.push_str(text);
                        self.settings.scroll = 0.0;
                        self.settings.focus = None;
                        return true;
                    }
                }
            }
        }
        match event.logical_key {
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowLeft) => {
                self.settings.move_page(-1);
                true
            }
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowRight) => {
                self.settings.move_page(1);
                true
            }
            Key::Named(NamedKey::Home) => {
                self.settings
                    .open_page(crate::views::settings::Page::General);
                true
            }
            Key::Named(NamedKey::End) => {
                self.settings
                    .open_page(crate::views::settings::Page::Themes);
                true
            }
            Key::Named(NamedKey::PageUp | NamedKey::PageDown) => {
                let direction = if matches!(event.logical_key, Key::Named(NamedKey::PageUp)) {
                    10.0
                } else {
                    -10.0
                };
                self.settings.scroll_by(direction);
                true
            }
            _ => false,
        }
    }

    /// Keyboard operation of the page controls: Tab / ⇧Tab move the focus,
    /// Space / Enter press the focused control, ← / → adjust a focused
    /// slider, Esc drops the focus (the next Esc closes Settings).
    fn handle_settings_focus_key(
        &mut self,
        event: &input::KeyPress,
        event_loop: &ActiveEventLoop,
        window: &Window,
    ) -> bool {
        use crate::views::settings;
        use winit::keyboard::{Key, NamedKey};
        let mods = self.modifiers;
        if mods.super_key() || mods.control_key() || mods.alt_key() {
            return false;
        }
        let order = settings::focus_order(&self.hits);
        // every control of the page, also scrolled away (page coordinates)
        let mut stops = settings::focus_order(
            &self
                .settings_stops
                .iter()
                .map(|(target, rect)| Hit {
                    rect: *rect,
                    target: target.clone(),
                })
                .collect::<Vec<_>>(),
        );
        // fixed controls outside the scrolled body (Open config.toml) last
        let fixed: Vec<_> = order
            .iter()
            .filter(|(t, _)| !stops.iter().any(|(s, _)| s == t))
            .cloned()
            .collect();
        let scrolled = stops.len();
        stops.extend(fixed);
        // a control that left the page (re-laid out) loses the focus; one
        // merely scrolled away keeps it
        if let Some(f) = &self.settings.focus
            && !stops.iter().any(|(t, _)| t == f)
        {
            self.settings.focus = None;
        }
        // on screen: what Space / Enter press
        let focused = self
            .settings
            .focus
            .clone()
            .and_then(|f| order.iter().find(|(t, _)| *t == f).cloned());
        match (&event.logical_key, focused) {
            (Key::Named(NamedKey::Tab), _) => {
                let delta = if mods.shift_key() { -1 } else { 1 };
                let next = settings::next_focus(&stops, self.settings.focus.as_ref(), delta);
                if let Some(rect) = next
                    .as_ref()
                    .and_then(|n| stops[..scrolled].iter().find(|(t, _)| t == n))
                    .map(|(_, r)| *r)
                {
                    self.scroll_settings_into_view(rect);
                }
                self.settings.focus = next;
            }
            (Key::Named(NamedKey::Escape), _) if self.settings.focus.is_some() => {
                self.settings.focus = None
            }
            (
                Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight),
                Some((ClickTarget::SettingsSlider(id), _)),
            ) => {
                let step = if matches!(event.logical_key, Key::Named(NamedKey::ArrowLeft)) {
                    -0.05
                } else {
                    0.05
                };
                let t = match id {
                    "line_height" => (self.config.line_height() - 1.0) / 1.2,
                    "window_opacity" => settings::opacity_slider(self.config.opacity()),
                    "window_blur" => settings::blur_slider(self.config.blur()),
                    _ => return true,
                };
                self.set_slider(id, (t + step).clamp(0.0, 1.0));
                self.save_config();
            }
            (key, Some((target, rect)))
                if press_key(key) && !matches!(target, ClickTarget::SettingsSlider(_)) =>
            {
                // press it through the normal pointer routing
                let saved = self.mouse;
                self.mouse = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
                self.on_left(ElementState::Pressed, event_loop, window);
                self.on_left(ElementState::Released, event_loop, window);
                self.mouse = saved;
                // a pressed ↺ disappears: the focus moves on to its control
                if matches!(target, ClickTarget::SettingsReset(_)) {
                    self.settings.focus = settings::next_focus(&stops, Some(&target), 1);
                }
            }
            // focused but scrolled away: the first key brings it back
            (key, None) if press_key(key) && self.settings.focus.is_some() => {
                if let Some(rect) = self
                    .settings
                    .focus
                    .as_ref()
                    .and_then(|f| stops[..scrolled].iter().find(|(t, _)| t == f))
                    .map(|(_, r)| *r)
                {
                    self.scroll_settings_into_view(rect);
                }
            }
            _ => return false,
        }
        window.request_redraw();
        true
    }

    /// Scroll the Settings page so the control at `rect` (page coordinates:
    /// screen y at scroll 0) sits inside the body, clamped like the wheel.
    fn scroll_settings_into_view(&mut self, rect: [f32; 4]) {
        let (_, lh) = self.logical_size();
        let (view_top, view_bottom) = (TAB_BAR_H + 12.0, lh - 34.0 - 12.0);
        let s = &mut self.settings;
        let y = rect[1] - s.scroll;
        if y < view_top {
            s.scroll -= view_top - y;
        } else if y + rect[3] > view_bottom {
            s.scroll += y + rect[3] - view_bottom;
        }
        s.scroll = s.scroll.clamp(0.0, s.max_scroll());
    }

    /// Keybindings page keys: while recording, the next chord becomes the
    /// action's shortcut (Esc cancels, ⌫ unbinds); otherwise typing filters.
    /// Returns true when consumed.
    fn handle_keybindings_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let mods = self.modifiers;
        if let Some(action) = self.settings.recording.clone() {
            let plain = !mods.control_key() && !mods.alt_key() && !mods.super_key();
            match &event.logical_key {
                Key::Named(NamedKey::Escape) if plain => {
                    self.settings.recording = None;
                    return true;
                }
                Key::Named(NamedKey::Backspace) if plain && !mods.shift_key() => {
                    self.settings.recording = None;
                    self.config
                        .keybindings
                        .insert(action.clone(), "none".into());
                    self.save_config();
                    self.apply_config();
                    self.toast(format!("{action} unbound"));
                    return true;
                }
                _ => {}
            }
            let key = match (&event.unmodified, &event.logical_key) {
                (Some(Key::Character(s)), Key::Character(_)) if mods.alt_key() => {
                    Some(s.to_lowercase())
                }
                _ => input::key_string(event),
            };
            let Some(key) = key else {
                return true; // a bare modifier: keep waiting
            };
            let function_key = key.len() > 1 && !matches!(key.as_str(), "space" | "enter" | "tab");
            if plain && !function_key {
                self.toast("Shortcuts need ⌘, ⌃ or ⌥ (or a function/navigation key)".into());
                return true;
            }
            let binding = shift_core::keybindings::Binding {
                ctrl: mods.control_key(),
                shift: mods.shift_key(),
                alt: mods.alt_key(),
                cmd: mods.super_key(),
                key,
            };
            self.settings.recording = None;
            let chord = binding.chord();
            // the chord moves: whoever had it keeps its other shortcuts
            let mut note = String::new();
            if let Some(other) = self.keymap.conflict(&binding, &action).map(str::to_string) {
                let rest = self
                    .keymap
                    .bindings_for(&other)
                    .into_iter()
                    .find(|b| **b != binding)
                    .map(|b| b.chord());
                self.config
                    .keybindings
                    .insert(other.clone(), rest.unwrap_or_else(|| "none".into()));
                note = format!(" (moved from {other})");
            }
            self.config.keybindings.insert(action.clone(), chord);
            self.save_config();
            self.apply_config();
            let shown = binding.chips().concat();
            self.toast(format!("{action} → {shown}{note}"));
            return true;
        }
        // type-to-filter
        if mods.super_key() || mods.control_key() {
            return false;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Backspace) if !self.settings.kb_filter.is_empty() => {
                self.settings.kb_filter.pop();
                self.settings.scroll = 0.0;
                true
            }
            Key::Named(NamedKey::Escape) if !self.settings.kb_filter.is_empty() => {
                self.settings.kb_filter.clear();
                true
            }
            Key::Named(NamedKey::Space) => {
                self.settings.kb_filter.push(' ');
                true
            }
            Key::Character(_) => match &event.text {
                Some(t) if !t.chars().any(char::is_control) => {
                    self.settings.kb_filter.push_str(t);
                    self.settings.scroll = 0.0;
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn handle_editor_key(&mut self, key: EditorKey) {
        // an offered correction is accepted with → / Tab / End in an empty editor
        if matches!(key, EditorKey::Right | EditorKey::Complete | EditorKey::End)
            && let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut())
            && pane.editor.buffer.is_empty()
            && let Some(fix) = pane.correction.take()
        {
            pane.editor.insert(&fix);
            self.update_completions();
            self.blink_epoch = Instant::now();
            return;
        }
        // alias expansion (Warp): Space right after an alias in command
        // position replaces it with its definition
        if key == EditorKey::Insert(" ".into())
            && self.config.alias_expansion
            && let Some(env) = self.shell_env.clone()
            && let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut())
            && pane.editor.selection().is_none()
            && let Some((range, expansion)) =
                env.expand_alias_at(&pane.editor.buffer, pane.editor.cursor)
        {
            pane.editor.replace_range(range, &expansion);
        }
        // Escape cancels a request in flight: its late answer is dropped
        // and can neither reopen the menu nor run a deferred Tab
        if key == EditorKey::Dismiss {
            self.completion_pending = None;
            self.completion_intent = None;
        }
        // completion popup intercepts navigation keys
        if self.completion.open {
            match key {
                EditorKey::HistoryUp => {
                    self.completion.selected = self.completion.selected.saturating_sub(1);
                    self.blink_epoch = Instant::now();
                    return;
                }
                EditorKey::HistoryDown => {
                    if self.completion.selected + 1 < self.completion.items.len() {
                        self.completion.selected += 1;
                    }
                    self.blink_epoch = Instant::now();
                    return;
                }
                EditorKey::Dismiss => {
                    self.completion.close();
                    return;
                }
                _ => {}
            }
        }
        if key == EditorKey::Complete {
            self.accept_completion();
            self.blink_epoch = Instant::now();
            return;
        }
        if key == EditorKey::HistorySearch {
            self.open_history();
            return;
        }
        if key == EditorKey::NextCompletionSource {
            if self.completion.open {
                self.completion.cycle_source();
            }
            return;
        }
        if key == EditorKey::AcceptGhostWord {
            self.accept_ghost_word();
            self.blink_epoch = Instant::now();
            return;
        }
        // → at the end of the line takes the whole ghost (Warp; the → chip)
        if key == EditorKey::Right
            && !self.completion.ghost.is_empty()
            && self.completion_pending.is_none()
            && self
                .tabs
                .get(self.active)
                .and_then(|t| t.active())
                .is_some_and(|p| {
                    p.editor.cursor == p.editor.buffer.len() && p.editor.selection().is_none()
                })
        {
            let ghost = self.completion.ghost.clone();
            if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                pane.editor.insert(&ghost);
            }
            self.completion.close();
            self.update_completions();
            self.blink_epoch = Instant::now();
            return;
        }

        let autocomplete_pairs = self.config.autocomplete_pairs;
        let mut submitted: Option<(PaneId, String)> = None;
        let mut signal: Option<(PaneId, &'static [u8])> = None;
        let mut cut: Option<String> = None;
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            return;
        };
        let mut edited = true;
        match key {
            EditorKey::Cut => cut = pane.editor.cut_selection(),
            EditorKey::SelectLeft => pane.editor.extend(crate::state::EditorState::mv_left),
            EditorKey::SelectRight => pane.editor.extend(crate::state::EditorState::mv_right),
            EditorKey::SelectWordLeft => {
                pane.editor.extend(crate::state::EditorState::mv_word_left)
            }
            EditorKey::SelectWordRight => {
                pane.editor.extend(crate::state::EditorState::mv_word_right)
            }
            EditorKey::SelectHome => pane.editor.extend(crate::state::EditorState::mv_home),
            EditorKey::SelectEnd => pane.editor.extend(crate::state::EditorState::mv_end),
            EditorKey::Insert(s) if autocomplete_pairs => pane.editor.insert_with_pairs(&s),
            EditorKey::Insert(s) => pane.editor.insert(&s),
            EditorKey::Left => pane.editor.left(),
            EditorKey::Right => pane.editor.right(),
            EditorKey::Home => pane.editor.home(),
            EditorKey::End => pane.editor.end(),
            EditorKey::WordLeft => pane.editor.word_left(),
            EditorKey::WordRight => pane.editor.word_right(),
            EditorKey::Backspace if autocomplete_pairs => pane.editor.backspace_with_pairs(),
            EditorKey::Backspace => pane.editor.backspace(),
            EditorKey::Delete => pane.editor.delete(),
            EditorKey::DeleteWordBack => pane.editor.delete_word_back(),
            EditorKey::DeleteWordForward => pane.editor.delete_word_forward(),
            EditorKey::KillToEnd => pane.editor.kill_to_end(),
            EditorKey::KillToStart => pane.editor.kill_to_start(),
            EditorKey::Undo => pane.editor.undo(),
            EditorKey::Redo => pane.editor.redo(),
            EditorKey::ClearLine => pane.editor.clear(),
            EditorKey::ClearScreen => {
                // shell-driven repaint: blocks and scrollback survive (⌘K is
                // the full app-side clear)
                pane.session.write(b"\x0c");
                pane.view_offset = 0;
                edited = false;
            }
            EditorKey::HistoryUp => pane.editor.history_up(),
            EditorKey::HistoryDown => pane.editor.history_down(),
            EditorKey::Dismiss
            | EditorKey::Complete
            | EditorKey::NextCompletionSource
            | EditorKey::HistorySearch
            | EditorKey::AcceptGhostWord => edited = false,
            EditorKey::Submit => {
                pane.correction = None;
                let cmd = pane.editor.submit();
                pane.view_offset = 0;
                pane.selected_block = None;
                // stamp prompt-time context onto the block for its header
                let cwd = pane.session.cwd.lock().unwrap().clone();
                let branch = pane.git.as_ref().map(|g| g.branch.clone());
                pane.session
                    .blocks
                    .lock()
                    .unwrap()
                    .stamp_submit(cwd, branch, &cmd);
                pane.session.write(cmd.as_bytes());
                pane.session.write(b"\n");
                submitted = Some((pane.session.id, cmd));
                self.completion.close();
                edited = false;
            }
            EditorKey::Interrupt => {
                pane.editor.clear();
                pane.session.write(b"\x03");
                signal = Some((pane.session.id, b"\x03"));
                self.completion.close();
                edited = false;
            }
            EditorKey::Eof => {
                if pane.editor.buffer.is_empty() {
                    pane.session.write(b"\x04");
                    signal = Some((pane.session.id, b"\x04"));
                }
                edited = false;
            }
        }
        if let Some(text) = cut {
            self.copy_to_clipboard(text);
        }
        if let Some((id, bytes)) = signal {
            self.write_synced(id, bytes, true);
        }
        // synchronized inputs: the same command runs in every pane of the tab
        // only to siblings at a prompt: a pane running vim or another
        // program would receive the command as keystrokes
        if let Some((id, cmd)) = submitted {
            let ready = self.synced_siblings(id, true);
            let busy = self.input_targets(id).len() - 1 - ready.len();
            for other in ready {
                self.submit_to(other, &cmd);
            }
            if busy > 0 {
                self.toast(format!(
                    "Not sent to {busy} synchronized pane{} running a program",
                    if busy == 1 { "" } else { "s" }
                ));
            }
        }
        if edited {
            if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                pane.clear_selection();
                pane.selected_block = None; // typing returns focus to the input
                pane.correction = None;
            }
            self.update_completions();
        }
        self.blink_epoch = Instant::now(); // cursor solid while editing
    }

    fn find_tab_of_pane(&mut self, pane: PaneId) -> Option<(usize, &mut TabState)> {
        self.tabs
            .iter_mut()
            .enumerate()
            .find(|(_, t)| t.panes.iter().any(|p| p.session.id == pane))
    }

    /// Commands still running inside `scope` (the close warning lists them).
    fn running_in(&self, scope: CloseScope) -> Vec<String> {
        let panes: Vec<&PaneState> = match scope {
            CloseScope::Pane(id) => self.tabs.iter().filter_map(|t| t.pane(id)).collect(),
            CloseScope::Tab(i) => self
                .tabs
                .get(i)
                .map(|t| t.panes.iter().collect())
                .unwrap_or_default(),
            CloseScope::OtherTabs(keep) => self
                .tabs
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != keep)
                .flat_map(|(_, t)| t.panes.iter())
                .collect(),
            CloseScope::Window => self.tabs.iter().flat_map(|t| t.panes.iter()).collect(),
        };
        panes
            .into_iter()
            .filter(|p| p.session.has_foreground_job())
            .map(|p| {
                let running = p
                    .session
                    .blocks
                    .lock()
                    .unwrap()
                    .last()
                    .filter(|b| matches!(b.state, BlockState::Running { .. }))
                    .map(|b| b.command.clone())
                    .filter(|c| !c.is_empty());
                running
                    .or_else(|| p.session.foreground_job_name())
                    .unwrap_or_else(|| "process".into())
            })
            .collect()
    }

    /// Close/quit entry point: warns first when something is still running.
    fn request_close(&mut self, scope: CloseScope, event_loop: &ActiveEventLoop) {
        if matches!(scope, CloseScope::Window) && self.update.busy {
            // quitting mid-install could leave a half-finished update
            self.update.quit_after = true;
            self.toast("Shift is updating… it quits when the update is done".into());
            return;
        }
        if self.config.confirm_close_running {
            let commands = self.running_in(scope);
            if !commands.is_empty() {
                self.confirm = Some(ConfirmClose {
                    opened: Instant::now(),
                    action: ConfirmAction::Close(scope),
                    commands,
                });
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                return;
            }
        }
        self.perform_close(scope, event_loop);
    }

    fn perform_close(&mut self, scope: CloseScope, event_loop: &ActiveEventLoop) {
        match scope {
            CloseScope::Pane(id) => self.close_pane(id, event_loop),
            CloseScope::Tab(i) => {
                let ids: Vec<PaneId> = self
                    .tabs
                    .get(i)
                    .map(|t| t.panes.iter().map(|p| p.session.id).collect())
                    .unwrap_or_default();
                for id in ids {
                    self.close_pane(id, event_loop);
                }
            }
            CloseScope::OtherTabs(keep) => {
                let ids: Vec<PaneId> = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != keep)
                    .flat_map(|(_, t)| t.panes.iter().map(|p| p.session.id))
                    .collect();
                for id in ids {
                    self.close_pane(id, event_loop);
                }
                self.active = 0;
                self.save_session();
            }
            CloseScope::Window => {
                self.save_session();
                // durable shells outlive the window only when a relaunch
                // will reattach them; otherwise they would be orphaned
                if !self.persist_session || !self.config.restore_session {
                    for p in self.tabs.iter().flat_map(|t| &t.panes) {
                        p.session.terminate();
                    }
                }
                event_loop.exit();
            }
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Confirm-modal keys: Esc cancels, Enter confirms; everything else is
    /// swallowed while the modal is up.
    fn handle_confirm_key(&mut self, event: &input::KeyPress, event_loop: &ActiveEventLoop) {
        use winit::keyboard::{Key, NamedKey};
        match event.logical_key {
            Key::Named(NamedKey::Escape) => self.cancel_confirm(),
            Key::Named(NamedKey::Enter) => {
                // a held or typed-ahead Enter (e.g. right after ⌘W or a
                // paste) must not accept a dialog the user has not seen
                if self
                    .confirm
                    .as_ref()
                    .is_some_and(|c| confirm_accepts(event.repeat, c.opened.elapsed()))
                    && let Some(c) = self.confirm.take()
                {
                    self.perform_confirmed(c.action, event_loop);
                }
            }
            _ => {}
        }
    }

    /// Viewer keys: Esc closes; arrows, Page Up/Down, Space, Home/End
    /// scroll. False for anything else.
    fn handle_viewer_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let page = self.viewer.page();
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.viewer.close(),
            Key::Named(NamedKey::ArrowDown) => self.viewer.scroll_by(20.0),
            Key::Named(NamedKey::ArrowUp) => self.viewer.scroll_by(-20.0),
            // ← → scroll the source view sideways (rendered: the code block
            // under the pointer)
            Key::Named(NamedKey::ArrowRight) => self.viewer.scroll_x_by(40.0, self.mouse.1),
            Key::Named(NamedKey::ArrowLeft) => self.viewer.scroll_x_by(-40.0, self.mouse.1),
            Key::Named(NamedKey::PageDown | NamedKey::Space) => self.viewer.scroll_by(page),
            Key::Named(NamedKey::PageUp) => self.viewer.scroll_by(-page),
            Key::Named(NamedKey::Home) => self.viewer.scroll_by(f32::NEG_INFINITY),
            Key::Named(NamedKey::End) => self.viewer.scroll_by(f32::INFINITY),
            _ => return false,
        }
        true
    }

    /// A press in "Edit prompt": buttons act at once; a chip arms a drag
    /// (a release without travel is a click).
    fn prompt_editor_press(&mut self) {
        use crate::views::prompt_editor::{Drag, Target};
        let (mx, my) = self.mouse;
        let hit = self
            .prompt_editor_layout
            .hits
            .iter()
            .find(|(r, _)| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3])
            .map(|(_, t)| *t);
        let Some(ed) = &mut self.prompt_editor else {
            return;
        };
        match hit {
            Some(Target::Palette(chip)) => {
                ed.drag = Some(Drag {
                    chip,
                    from: None,
                    press: self.mouse,
                    live: false,
                })
            }
            Some(Target::Chip(i)) => {
                if let Some(&chip) = ed.draft.get(i) {
                    ed.drag = Some(Drag {
                        chip,
                        from: Some(i),
                        press: self.mouse,
                        live: false,
                    });
                }
            }
            Some(Target::Remove(i)) => ed.remove(i),
            Some(Target::Restore) => ed.restore_default(),
            Some(Target::Cancel) => self.prompt_editor = None,
            Some(Target::Save) => self.save_prompt_editor(),
            None => {}
        }
    }

    /// A release in "Edit prompt": a drag drops into the box (or, from the
    /// box, out of it to remove the chip); a palette chip clicked is added.
    fn prompt_editor_release(&mut self) {
        let layout = &self.prompt_editor_layout;
        let (mx, my) = self.mouse;
        let b = layout.drop_box;
        let in_box = mx >= b[0] && mx < b[0] + b[2] && my >= b[1] && my < b[1] + b[3];
        let Some(ed) = &mut self.prompt_editor else {
            return;
        };
        let Some(d) = ed.drag.take() else {
            return;
        };
        if !d.live {
            if d.from.is_none() {
                ed.add(d.chip);
            }
        } else if in_box {
            let at = crate::views::prompt_editor::drop_index(&layout.slots, d.from, self.mouse);
            ed.place(d.chip, at);
        } else if let Some(i) = d.from {
            ed.remove(i);
        }
    }

    /// Keep "Edit prompt"'s chips and close it.
    fn save_prompt_editor(&mut self) {
        let Some(ed) = self.prompt_editor.take() else {
            return;
        };
        if ed.changed() {
            self.config.prompt_chips = ed.draft;
            self.save_config();
            self.refresh_chip_sources();
        }
    }

    /// After the prompt chips change (here, or config.toml from anywhere),
    /// look up what the newly shown chips read — git (and through it the
    /// PR), Docker — for every pane of every tab, so each shows them at
    /// once instead of after its next command.
    fn refresh_chip_sources(&mut self) {
        if self.chips_applied == self.config.prompt_chips {
            return;
        }
        self.chips_applied = self.config.prompt_chips.clone();
        let docker = self
            .config
            .prompt_chips
            .contains(&shift_core::PromptChip::Docker);
        for tab in &self.tabs {
            for pane in &tab.panes {
                let Some(cwd) = pane.session.cwd.lock().unwrap().clone() else {
                    continue;
                };
                if let Some(worker) = &self.git {
                    worker.request(pane.session.id, cwd.clone().into());
                }
                if docker && let Some(worker) = &self.docker_worker {
                    worker.request(pane.session.id, cwd.into());
                }
            }
        }
    }

    /// Open a chip's menu for `pane` (focusing it).
    fn open_chip_menu(&mut self, pane: PaneId, kind: crate::views::chip_menu::ChipKind) {
        use crate::views::chip_menu::{ChipKind, ChipMenu};
        use shift_core::runtimes;
        let target = ClickTarget::Chip(pane, kind.chip());
        let Some(anchor) = self
            .hits
            .iter()
            .find(|h| h.target == target)
            .map(|h| h.rect)
        else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        tab.active_pane = pane;
        let Some(p) = tab.pane(pane) else {
            return;
        };
        let cwd = p.session.cwd.lock().unwrap().clone();
        let Some(cwd) = cwd.map(std::path::PathBuf::from) else {
            return;
        };
        self.chip_menu = Some(match kind {
            ChipKind::Dir => ChipMenu::dirs(pane, anchor, &cwd),
            ChipKind::Branch => {
                let current = p.git.as_ref().map(|g| g.branch.clone()).unwrap_or_default();
                ChipMenu::branches(
                    pane,
                    anchor,
                    shift_core::git::local_branches(&cwd),
                    &current,
                )
            }
            ChipKind::Php | ChipKind::Python => {
                let vars = p.session.user_vars.lock().unwrap().clone();
                let bin = |name: &str| {
                    vars.get(name)
                        .filter(|v| !v.is_empty())
                        .map(std::path::PathBuf::from)
                };
                let home = runtimes::home();
                let prefixes = runtimes::brew_prefixes();
                if kind == ChipKind::Php {
                    let active = bin("ShiftPhpBin").and_then(|b| runtimes::php_version(&b));
                    let list = runtimes::php_installs(&home, &prefixes);
                    ChipMenu::runtimes(kind, pane, anchor, list, active.as_deref())
                } else {
                    let active = vars
                        .get("ShiftPython")
                        .filter(|v| !v.is_empty())
                        .cloned()
                        .or_else(|| runtimes::python_version(&bin("ShiftPythonBin")?, &cwd));
                    // the shell's pyenv (its shims' root), else the default one
                    let pyenv = bin("ShiftPythonBin")
                        .filter(|b| b.parent().is_some_and(|d| d.ends_with("shims")))
                        .and_then(|b| Some(b.parent()?.parent()?.to_path_buf()))
                        .or_else(runtimes::pyenv_root);
                    let list = runtimes::python_installs(&home, &prefixes, pyenv.as_deref());
                    ChipMenu::runtimes(kind, pane, anchor, list, active.as_deref())
                }
            }
            ChipKind::Docker => {
                // show what is known now; a fresh `ps` replaces it
                if let Some(worker) = &self.docker_worker {
                    worker.request(pane, cwd.clone());
                }
                ChipMenu::containers(pane, anchor, p.docker.as_ref())
            }
        });
    }

    /// Run the picker's result `i` (`cd …` / `git checkout …`) as a command
    /// in its pane, which must be at its prompt.
    fn apply_chip_menu(&mut self, i: usize) {
        let Some(menu) = self.chip_menu.take() else {
            return;
        };
        let Some(item) = menu.results().into_iter().nth(i) else {
            return;
        };
        let at_prompt = self
            .find_tab_of_pane(menu.pane)
            .and_then(|(_, tab)| tab.pane(menu.pane))
            .is_some_and(|p| {
                matches!(
                    p.session.blocks.lock().unwrap().last().map(|b| &b.state),
                    Some(BlockState::Prompt)
                )
            });
        if at_prompt {
            self.submit_to(menu.pane, &crate::views::chip_menu::command(&item.action));
        } else {
            self.toast("The pane is running a command".into());
        }
    }

    /// Picker keys: typing filters, ↑↓ select, Enter runs, Esc closes.
    fn handle_chip_menu_key(&mut self, event: &input::KeyPress) {
        use winit::keyboard::{Key, NamedKey};
        let Some(menu) = &mut self.chip_menu else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.chip_menu = None,
            Key::Named(NamedKey::Enter) => {
                let i = menu.selected;
                self.apply_chip_menu(i);
            }
            Key::Named(NamedKey::ArrowUp) => menu.move_by(-1),
            Key::Named(NamedKey::ArrowDown) => menu.move_by(1),
            Key::Named(NamedKey::PageUp) => menu.move_by(-8),
            Key::Named(NamedKey::PageDown) => menu.move_by(8),
            Key::Named(NamedKey::Backspace) => {
                let mut q = menu.query.clone();
                q.pop();
                menu.set_query(q);
            }
            Key::Named(NamedKey::Space) => {
                let q = format!("{} ", menu.query);
                menu.set_query(q);
            }
            _ => {
                if !self.modifiers.super_key()
                    && !self.modifiers.control_key()
                    && let Some(text) = &event.text
                    && !text.is_empty()
                    && !text.chars().any(|c| c.is_control())
                {
                    let q = format!("{}{text}", menu.query);
                    menu.set_query(q);
                }
            }
        }
    }

    /// Maximized review keys: Esc restores the panel beside the panes;
    /// arrows, Page Up/Down, Space, Home/End scroll (← → sideways, the file
    /// under the pointer). False for anything else.
    fn handle_review_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let page = (self.review.view_h - 40.0).max(40.0);
        let y = self.mouse.1;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.review.maximized = false,
            Key::Named(NamedKey::ArrowDown) => self.review.scroll_by(40.0),
            Key::Named(NamedKey::ArrowUp) => self.review.scroll_by(-40.0),
            Key::Named(NamedKey::ArrowRight) => self.review.scroll_x_by(40.0, y),
            Key::Named(NamedKey::ArrowLeft) => self.review.scroll_x_by(-40.0, y),
            Key::Named(NamedKey::PageDown | NamedKey::Space) => self.review.scroll_by(page),
            Key::Named(NamedKey::PageUp) => self.review.scroll_by(-page),
            Key::Named(NamedKey::Home) => self.review.scroll_by(f32::NEG_INFINITY),
            Key::Named(NamedKey::End) => self.review.scroll_by(f32::INFINITY),
            _ => return false,
        }
        true
    }

    /// Tree keys while the explorer has focus: ↑↓ move, ← collapse or go
    /// to the parent, → expand, Enter opens (⌥Enter inserts the path), Esc
    /// hands focus back to the terminal. False for anything else.
    fn handle_explorer_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown) => self.explorer.move_selection(1),
            Key::Named(NamedKey::ArrowUp) => self.explorer.move_selection(-1),
            Key::Named(NamedKey::ArrowLeft) => self.explorer.left(),
            Key::Named(NamedKey::ArrowRight) => self.explorer.right(),
            Key::Named(NamedKey::Escape) => self.explorer.focused = false,
            Key::Named(NamedKey::Enter) => {
                let Some(row) = self.explorer.selected_row().cloned() else {
                    return true;
                };
                if self.modifiers.alt_key() {
                    self.explorer.focused = false;
                    self.drop_file(&row.path);
                } else if row.is_dir {
                    self.explorer.toggle(&row.path);
                } else {
                    let choice = self.config.file_editor.clone();
                    self.open_file_in(&row.path, &choice);
                }
            }
            _ => return false,
        }
        true
    }

    /// Popover keys: Esc closes, ↑/↓ highlight, ⏎ opens the highlighted
    /// row, Tab / ⇧Tab switch the filter chip. Any other plain key closes it
    /// and goes on to the usual routing (returns false).
    fn handle_notifications_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let chord =
            self.modifiers.super_key() || self.modifiers.control_key() || self.modifiers.alt_key();
        let len = self.filtered_notifications().len();
        match &event.logical_key {
            _ if event.released => return false,
            // a lone modifier is the start of a chord
            Key::Named(
                NamedKey::Shift
                | NamedKey::Control
                | NamedKey::Alt
                | NamedKey::Super
                | NamedKey::Meta
                | NamedKey::CapsLock,
            ) => {}
            Key::Named(NamedKey::Escape) => self.close_notifications(),
            Key::Named(NamedKey::ArrowDown) => self.notifications.move_selection(1, len),
            Key::Named(NamedKey::ArrowUp) => self.notifications.move_selection(-1, len),
            Key::Named(NamedKey::Tab) => {
                let delta = if self.modifiers.shift_key() { -1 } else { 1 };
                let next = self.notifications.filter.cycle(delta);
                self.notifications.set_filter(next);
            }
            Key::Named(NamedKey::Enter) => {
                let target = self
                    .notifications
                    .selected
                    .and_then(|s| self.filtered_notifications().get(s).map(|e| e.id));
                self.close_notifications();
                if let Some(id) = target {
                    self.show_attention(id);
                }
            }
            _ if !chord => {
                self.close_notifications();
                return false;
            }
            // chords (⇧⌘A toggles, ⌘P …) run as usual; their action decides
            _ => return false,
        }
        true
    }

    /// What's New keys: Esc/Enter close, arrows, Page Up/Down, Space and
    /// Home/End scroll; everything else is swallowed while it is up.
    fn handle_whats_new_key(&mut self, event: &input::KeyPress) {
        use winit::keyboard::{Key, NamedKey};
        let page = self.whats_new.page();
        match &event.logical_key {
            Key::Named(NamedKey::Escape | NamedKey::Enter) => self.whats_new.open = false,
            Key::Named(NamedKey::ArrowDown) => self.whats_new.scroll_by(40.0),
            Key::Named(NamedKey::ArrowUp) => self.whats_new.scroll_by(-40.0),
            Key::Named(NamedKey::PageDown | NamedKey::Space) => self.whats_new.scroll_by(page),
            Key::Named(NamedKey::PageUp) => self.whats_new.scroll_by(-page),
            Key::Named(NamedKey::Home) => self.whats_new.scroll_by(f32::NEG_INFINITY),
            Key::Named(NamedKey::End) => self.whats_new.scroll_by(f32::INFINITY),
            _ => {}
        }
    }

    fn perform_confirmed(&mut self, action: ConfirmAction, event_loop: &ActiveEventLoop) {
        match action {
            ConfirmAction::Close(scope) => self.perform_close(scope, event_loop),
            ConfirmAction::Paste { pane, text } => {
                self.paste_raw(pane, &text);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            ConfirmAction::InstallRemote(profile) => {
                let script = shift_core::shell_integration::data_dir()
                    .map(|d| d.join("remote-install.sh"))
                    .filter(|p| std::fs::write(p, shift_core::ssh::installer_script()).is_ok());
                match script {
                    Some(path) => {
                        let title = format!("install on {}", profile.name);
                        self.open_tab_running(&title, profile.install_command(&path));
                    }
                    None => self.toast("Could not write the installer script".into()),
                }
            }
            ConfirmAction::Revert { root, path, hunk } => {
                self.queue_review_write(root, vec![(shift_core::review::Op::Revert, path, hunk)]);
            }
            ConfirmAction::Update(plan) => self.install_update(*plan),
        }
    }

    /// New tab titled `title` that runs `command` at its first prompt (a
    /// normal block: ssh prompts and output stay visible and interactive).
    fn open_tab_running(&mut self, title: &str, command: String) {
        self.spawn_tab_at(None);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.custom_title = Some(title.to_string());
            let pane = tab.active_pane;
            self.pending_commands.insert(pane, vec![command]);
        }
        self.save_session();
    }

    /// Open another Shift window (a separate process that neither restores
    /// nor overwrites this window's saved session).
    fn open_new_window(&mut self) {
        if self.headless {
            // a second, ordinary window would show and take focus
            log::info!("headless: not opening a new window");
            self.toast("Headless: no new windows".into());
            return;
        }
        let cwd = self.start_dir();
        match std::env::current_exe() {
            Ok(exe) => {
                let mut cmd = std::process::Command::new(exe);
                cmd.env(crate::updater::SECONDARY_ENV, "1")
                    .stdin(std::process::Stdio::null());
                if let Some(dir) = cwd {
                    cmd.current_dir(dir);
                }
                if let Err(e) = cmd.spawn() {
                    log::warn!("new window: {e}");
                }
            }
            Err(e) => log::warn!("new window: {e}"),
        }
    }

    /// Directory for a new tab/pane per `working_directory`.
    fn start_dir(&self) -> Option<std::path::PathBuf> {
        let previous = self
            .tabs
            .get(self.active)
            .and_then(|t| t.active())
            .and_then(|p| p.session.cwd.lock().unwrap().clone());
        self.config.start_dir(previous.as_deref())
    }

    /// Panes that receive input typed into `pane` (itself, plus every other
    /// pane of its tab while synchronized inputs are on).
    fn input_targets(&self, pane: PaneId) -> Vec<PaneId> {
        match self.tabs.iter().find(|t| t.pane(pane).is_some()) {
            Some(t) if t.sync_inputs => {
                let mut ids = vec![pane];
                ids.extend(
                    t.panes
                        .iter()
                        .map(|p| p.session.id)
                        .filter(|id| *id != pane),
                );
                ids
            }
            _ => vec![pane],
        }
    }

    /// Synchronized siblings of `pane` in one input mode: `at_prompt` picks
    /// panes whose owned editor is live (they take submitted commands),
    /// else panes running a raw program (they take raw keys and pastes).
    /// A pane never receives input meant for the other mode.
    fn synced_siblings(&self, pane: PaneId, at_prompt: bool) -> Vec<PaneId> {
        let Some(tab) = self.tabs.iter().find(|t| t.pane(pane).is_some()) else {
            return Vec::new();
        };
        self.input_targets(pane)
            .into_iter()
            .skip(1)
            .filter(|id| {
                tab.pane(*id)
                    .is_some_and(|p| Self::editor_visible(p) == at_prompt)
            })
            .collect()
    }

    /// Recipients of raw input typed into `pane`: itself, then its raw
    /// synchronized siblings.
    fn raw_targets(&self, pane: PaneId) -> Vec<PaneId> {
        let mut targets = vec![pane];
        targets.extend(self.synced_siblings(pane, false));
        targets
    }

    /// Byte offset in the active editor under the mouse (mono font: the
    /// column is found by measuring the row's shaped prefix widths).
    fn editor_offset_at_mouse(&mut self, pane_id: PaneId) -> Option<usize> {
        let (text_x, input_y) = self.editor_anchor;
        let (mx, my) = self.mouse;
        let buffer = self
            .tabs
            .get(self.active)?
            .pane(pane_id)?
            .editor
            .buffer
            .clone();
        let rows = editor::wrap_rows(&buffer, self.editor_cols);
        let row = (((my - input_y) / editor::LINE_H).floor().max(0.0) as usize).min(rows.len() - 1);
        let row_start = rows[row].start;
        let line = &buffer[rows[row].clone()];
        let engine = self.text.as_mut()?;
        let mut col = line.chars().count();
        let mut prev = 0.0f32;
        for (i, (bi, ch)) in line.char_indices().enumerate() {
            let w = engine.measure(
                &line[..bi + ch.len_utf8()],
                editor::FONT_SIZE,
                shift_renderer::mono_family(),
                shift_renderer::glyphon::Weight::NORMAL,
            );
            if mx - text_x < (prev + w) / 2.0 {
                col = i;
                break;
            }
            prev = w;
        }
        let within = line
            .char_indices()
            .nth(col)
            .map(|(i, _)| i)
            .unwrap_or(line.len());
        Some(row_start + within)
    }

    /// Mouse press in a pane's editor: focus it, then place the cursor
    /// (double-click selects a word, triple-click the whole command, Shift
    /// extends the selection).
    fn click_editor(&mut self, pane_id: PaneId, clicks: u8) {
        if let Some(tab) = self.tabs.get_mut(self.active)
            && tab.active_pane != pane_id
        {
            if let Some(prev) = tab.active_mut() {
                prev.selected_block = None;
            }
            tab.active_pane = pane_id;
            return; // first click only focuses; geometry is the old pane's
        }
        let shift = self.modifiers.shift_key();
        let Some(offset) = self.editor_offset_at_mouse(pane_id) else {
            return;
        };
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return;
        };
        pane.clear_selection();
        pane.selected_block = None;
        let e = &mut pane.editor;
        match clicks {
            2 => {
                let is_word = |c: char| !c.is_whitespace();
                let start = e.buffer[..offset]
                    .char_indices()
                    .rev()
                    .take_while(|(_, c)| is_word(*c))
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(offset);
                let end = e.buffer[offset..]
                    .char_indices()
                    .find(|(_, c)| !is_word(*c))
                    .map(|(i, _)| offset + i)
                    .unwrap_or(e.buffer.len());
                e.click_to(start, false);
                e.click_to(end, true);
            }
            3 => e.select_all(),
            _ => e.click_to(offset, shift),
        }
        self.completion.close();
        self.blink_epoch = Instant::now();
    }

    fn drag_editor(&mut self, pane_id: PaneId) {
        let Some(offset) = self.editor_offset_at_mouse(pane_id) else {
            return;
        };
        if let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        {
            pane.editor.click_to(offset, true);
        }
    }

    /// Begin an inline rename: the current title is pre-filled and fully
    /// selected, so the first keystroke replaces it (Backspace clears it).
    fn start_rename(&mut self, i: usize) {
        if let Some(t) = self.tabs.get(i) {
            self.rename = Some((i, t.display_title()));
            self.rename_fresh = true;
        }
    }

    /// Inline tab rename keys: type to edit, Enter commits, Esc cancels.
    fn handle_rename_key(&mut self, event: &input::KeyPress) {
        use winit::keyboard::{Key, NamedKey};
        let Some((i, buf)) = self.rename.as_mut() else {
            return;
        };
        let i = *i;
        let fresh = std::mem::take(&mut self.rename_fresh);
        if fresh
            && matches!(
                &event.logical_key,
                Key::Named(NamedKey::Backspace | NamedKey::Space) | Key::Character(_)
            )
        {
            buf.clear();
            if matches!(event.logical_key, Key::Named(NamedKey::Backspace)) {
                return;
            }
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.rename = None,
            Key::Named(NamedKey::Enter) => self.commit_rename(i),
            Key::Named(NamedKey::Backspace) => {
                if self.modifiers.super_key() || self.modifiers.alt_key() {
                    buf.clear();
                } else {
                    buf.pop();
                }
            }
            Key::Named(NamedKey::Space) => buf.push(' '),
            _ => {
                if !self.modifiers.super_key()
                    && !self.modifiers.control_key()
                    && let Some(text) = &event.text
                    && !text.chars().any(|c| c.is_control())
                    && buf.chars().count() < 60
                {
                    buf.push_str(text);
                }
            }
        }
        self.blink_epoch = Instant::now();
    }

    /// Apply the inline rename; an empty name restores the automatic title.
    fn commit_rename(&mut self, i: usize) {
        if let Some((_, name)) = self.rename.take()
            && let Some(tab) = self.tabs.get_mut(i)
        {
            let name = name.trim().to_string();
            tab.custom_title = (!name.is_empty()).then_some(name);
            self.save_session();
        }
    }

    /// Mirror raw bytes typed into `pane` to its synchronized siblings
    /// (`skip_self`: the caller already wrote to `pane`).
    fn write_synced(&mut self, pane: PaneId, bytes: &[u8], skip_self: bool) {
        let targets = self.input_targets(pane);
        for id in targets.into_iter().skip(usize::from(skip_self)) {
            if let Some((_, tab)) = self.find_tab_of_pane(id)
                && let Some(p) = tab.pane_mut(id)
            {
                if skip_self && bytes == b"\x03" {
                    p.editor.clear();
                }
                p.view_offset = 0;
                p.session.write(bytes);
            }
        }
    }

    /// Run `cmd` in pane `id` at its prompt: history, prompt-time context,
    /// then the bytes (used by submit, re-run, palette and synced inputs).
    fn submit_to(&mut self, id: PaneId, cmd: &str) {
        let Some((_, tab)) = self.find_tab_of_pane(id) else {
            return;
        };
        let Some(pane) = tab.pane_mut(id) else { return };
        if !cmd.trim().is_empty() && pane.editor.history.last().map(String::as_str) != Some(cmd) {
            pane.editor.history.push(cmd.to_string());
        }
        pane.view_offset = 0;
        pane.selected_block = None;
        let cwd = pane.session.cwd.lock().unwrap().clone();
        let branch = pane.git.as_ref().map(|g| g.branch.clone());
        pane.session
            .blocks
            .lock()
            .unwrap()
            .stamp_submit(cwd, branch, cmd);
        pane.session.write(cmd.as_bytes());
        pane.session.write(b"\n");
    }
}

impl App {
    /// One key press (real or scripted). Routes overlays, app actions,
    /// the owned editor and raw PTY passthrough.
    pub(crate) fn on_key(
        &mut self,
        event: &input::KeyPress,
        event_loop: &ActiveEventLoop,
        window: &Window,
    ) {
        if !event.released {
            self.hide_tooltip(window);
        }
        // Esc while dragging: a tab springs back to where it was pressed, a
        // pane stays where it is
        if matches!(
            event.logical_key,
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        ) && (self.tab_drag.is_some_and(|d| d.live)
            || self.pane_drag.as_ref().is_some_and(|d| d.live))
        {
            if let Some(d) = self.tab_drag.take() {
                let tab = self.tabs.remove(d.index);
                let to = d.origin.min(self.tabs.len());
                self.tabs.insert(to, tab);
                self.active = to;
                self.save_session();
            }
            // a pane drag that opened another tab goes back to the pane's own
            if let Some(d) = self.pane_drag.take()
                && d.from_tab != self.active
                && d.from_tab < self.tabs.len()
            {
                self.switch_tab(d.from_tab);
            }
            self.update_cursor_icon(window);
            window.request_redraw();
            return;
        }
        if self.confirm.is_some() {
            self.handle_confirm_key(event, event_loop);
            window.request_redraw();
            return;
        }
        if self.prompt_editor.is_some() {
            // Esc cancels, Enter saves; the dialog owns the keyboard
            match event.logical_key {
                winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                    self.prompt_editor = None;
                }
                winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => {
                    self.save_prompt_editor();
                }
                _ => {}
            }
            window.request_redraw();
            return;
        }
        if self.whats_new.open {
            self.handle_whats_new_key(event);
            window.request_redraw();
            return;
        }
        if self.notifications.open && self.handle_notifications_key(event) {
            window.request_redraw();
            return;
        }
        if self.hints.is_some() {
            self.handle_hint_key(event);
            window.request_redraw();
            return;
        }
        if self.rename.is_some() {
            self.handle_rename_key(event);
            window.request_redraw();
            return;
        }
        if self.chip_menu.is_some() {
            self.handle_chip_menu_key(event);
            window.request_redraw();
            return;
        }
        if self.context_menu.open {
            if !self.handle_menu_key(event, event_loop) {
                self.context_menu.close();
            }
            window.request_redraw();
            return;
        }
        if self.palette.open {
            if input::app_action(event, self.modifiers, &self.keymap)
                == Some(AppAction::TogglePalette)
            {
                self.palette.open = false;
            } else {
                self.handle_palette_key(event, event_loop);
            }
            window.request_redraw();
            return;
        }
        if self.settings.open {
            if self.handle_settings_key(event, event_loop, window) {
                window.request_redraw();
            }
            return;
        }
        if self.review_maximized() {
            // the panel covers the command line: it takes the keys it uses,
            // app shortcuts still work, typing is swallowed
            if !self.handle_review_key(event)
                && let Some(action) = input::app_action(event, self.modifiers, &self.keymap)
                && !action.needs_editor()
                && (!event.repeat || action.repeatable())
            {
                self.handle_action(action, event_loop);
            }
            window.request_redraw();
            return;
        }
        // (closes first if its pane no longer owns the keys: raw mode, another
        // pane, the viewer, an overlay)
        self.sync_history();
        if self.history.open && self.handle_history_key(event) {
            window.request_redraw();
            return;
        }
        if self.search.open && self.filter_editing {
            self.handle_filter_key(event);
            window.request_redraw();
            return;
        }
        if self.search.open {
            let action = input::app_action(event, self.modifiers, &self.keymap);
            if action == Some(AppAction::Search) {
                self.search.close();
            } else if action == Some(AppAction::Copy) {
                self.handle_action(AppAction::Copy, event_loop);
            } else {
                self.handle_search_key(event, self.modifiers);
            }
            window.request_redraw();
            return;
        }
        if self.viewer_focused() {
            // the file's find bar takes typing; chords stay app shortcuts
            let mods = self.modifiers;
            if self.viewer.find.is_some()
                && !mods.super_key()
                && !mods.control_key()
                && input::app_action(event, mods, &self.keymap).is_none()
                && self.viewer.handle_find_key(event, mods.shift_key())
            {
                window.request_redraw();
                return;
            }
            if !self.handle_viewer_key(event) {
                // app shortcuts still work; typing is swallowed (the command
                // line is under the viewer)
                if let Some(action) = input::app_action(event, self.modifiers, &self.keymap)
                    && !action.needs_editor()
                    && (!event.repeat || action.repeatable())
                {
                    self.handle_action(action, event_loop);
                }
            }
            window.request_redraw();
            return;
        }
        if self.explorer.focused && self.explorer.open {
            if self.handle_explorer_key(event) {
                window.request_redraw();
                return;
            }
            self.explorer.focused = false;
        }
        // Escape clears the keyboard block selection before anything
        // else; in raw mode it still falls through so vim gets it.
        if matches!(
            event.logical_key,
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        ) && let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut())
            && pane.selected_block.is_some()
        {
            pane.selected_block = None;
            window.request_redraw();
            if self.editor_active {
                return;
            }
        }
        if let Some(action) = input::app_action(event, self.modifiers, &self.keymap) {
            // plain terminal keys (PageUp, Home, ⌃M…) bound to app
            // actions still belong to vim/less when the editor isn't
            // active — fall through to raw byte passthrough
            if !action.needs_editor() || self.editor_active {
                if event.repeat && !action.repeatable() {
                    return;
                }
                log::debug!("key action: {action:?} (mods {:?})", self.modifiers);
                self.handle_action(action, event_loop);
                return;
            }
        }
        log::debug!(
            "key fallthrough: {:?} mods={:?} editor={}",
            event.logical_key,
            self.modifiers,
            self.editor_active
        );
        if self.editor_active {
            if self.vim_intercept(event) {
                window.request_redraw();
                return;
            }
            if let Some(key) = input::editor_key(event, self.modifiers) {
                self.handle_editor_key(key);
                window.request_redraw();
            }
            return;
        }
        let Some(id) = self.active_pane_id() else {
            return;
        };
        // each recipient gets the bytes its own program asked for (cursor
        // mode, kitty flags), never the active pane's encoding
        let (mods, option_as_meta) = (self.modifiers, self.option_meta());
        let identity = input::key_identity(event);
        for target in self.raw_targets(id) {
            let Some((_, tab)) = self.find_tab_of_pane(target) else {
                continue;
            };
            let Some(p) = tab.pane_mut(target) else {
                continue;
            };
            let enc = key_encoding(p, option_as_meta);
            let Some(bytes) = input::key_to_bytes(event, mods, enc) else {
                continue;
            };
            // typing into a program scrolls back to it and drops the selection
            p.view_offset = 0;
            if target == id {
                p.clear_selection();
            }
            p.session.write(&bytes);
            if enc.kitty_flags & crate::keyboard::EVENT_TYPES != 0 {
                self.keys_down.insert((target, identity.clone()));
            }
            if target == id && self.latency_log {
                self.latency_probe = Some((Instant::now(), false));
            }
        }
    }

    /// Pointer coordinate along the tab strip's axis (x for the top bar,
    /// y for the vertical rail).
    fn tab_drag_coord(&self) -> f32 {
        if self.config.tab_layout == shift_core::TabLayout::Vertical {
            self.mouse.1
        } else {
            self.mouse.0
        }
    }

    /// Pointer moved during a pane-header press: go live past the slop and
    /// track the drop target under the pointer.
    fn track_pane_drag(&mut self) {
        let (mx, my) = self.mouse;
        let Some(d) = self.pane_drag.as_ref() else {
            return;
        };
        let (pane, from) = (d.pane, d.from_tab);
        let live = d.live || (mx - d.press.0).hypot(my - d.press.1) > PANE_DRAG_SLOP;
        let target = if live {
            self.pane_drop_at(pane, from, mx, my)
        } else {
            None
        };
        // held over another tab: open it, so the pane can land among its panes
        let hover_tab = match (target, d.hover_tab) {
            (Some(PaneDrop::Tab(j)), Some((k, since))) if j == k => Some((j, since)),
            (Some(PaneDrop::Tab(j)), _) => Some((j, Instant::now())),
            _ => None,
        };
        if let Some(d) = self.pane_drag.as_mut() {
            d.live = live;
            d.target = target;
            d.hover_tab = hover_tab;
        }
        if let Some((j, since)) = hover_tab
            && since.elapsed() >= TAB_HOVER_OPEN
            && j != self.active
        {
            self.switch_tab(j);
            if let Some(d) = self.pane_drag.as_mut() {
                d.hover_tab = None;
            }
        }
    }

    /// Where `pane` (of tab `from`) would land if released at the point.
    fn pane_drop_at(&self, pane: PaneId, from: usize, mx: f32, my: f32) -> Option<PaneDrop> {
        let hit = self
            .hits
            .iter()
            .find(|h| h.contains(mx, my))
            .map(|h| h.target.clone());
        // the Settings tab takes no panes
        if let Some(ClickTarget::SettingsClose) = hit {
            return None;
        }
        if let Some(ClickTarget::TabSelect(j) | ClickTarget::TabClose(j)) = hit {
            return (j < self.tabs.len() && (j != from || j != self.active))
                .then_some(PaneDrop::Tab(j));
        }
        let rail = self.rail_w();
        let in_strip = if rail > 0.0 {
            mx < rail && my > TAB_BAR_H
        } else {
            my < TAB_BAR_H
        };
        if in_strip {
            // a tab of its own needs a shell left behind; the viewer never
            // stands alone
            let alone = self.tabs.get(from).is_none_or(|t| t.panes.len() <= 1);
            return (pane != VIEWER_PANE && !alone).then_some(PaneDrop::NewTab);
        }
        let (lw, lh) = self.logical_size();
        let (rects, _) = self.pane_rects(lw, lh);
        let r = rects.iter().find(|r| {
            mx >= r.rect[0]
                && mx < r.rect[0] + r.rect[2]
                && my >= r.rect[1]
                && my < r.rect[1] + r.rect[3]
        })?;
        if r.id == pane {
            return None;
        }
        // the viewer leaf has one id in every tab: no landing beside another
        // tab's viewer, nor the viewer into a tab that already shows one
        if from != self.active
            && (r.id == VIEWER_PANE
                || (pane == VIEWER_PANE
                    && self
                        .tabs
                        .get(self.active)
                        .is_some_and(|t| t.root.contains(VIEWER_PANE))))
        {
            return None;
        }
        // the middle third swaps (same tab only); elsewhere the nearest half
        Some(
            if from == self.active && crate::pane::Edge::in_centre(r.rect, mx, my) {
                PaneDrop::Swap(r.id)
            } else {
                PaneDrop::Pane(r.id, crate::pane::Edge::nearest(r.rect, mx, my))
            },
        )
    }

    /// Move `pane` out of tab `from` to `target` (pane header drag and drop).
    /// A shell keeps its session; a tab left without shells goes away.
    fn drop_pane(&mut self, pane: PaneId, from: usize, target: PaneDrop) {
        if !self.tabs.get(from).is_some_and(|t| t.root.contains(pane)) {
            return;
        }
        let viewer = pane == VIEWER_PANE;
        // a pane of another tab (that tab was opened by hovering it): land
        // beside it there
        let mut beside = None;
        let to = match target {
            PaneDrop::Swap(onto) => {
                let tab = &mut self.tabs[from];
                if onto == pane || !tab.root.contains(onto) {
                    return;
                }
                tab.zoomed = None;
                tab.root.swap(pane, onto);
                self.save_session();
                return;
            }
            PaneDrop::Pane(onto, edge) if !self.tabs[from].root.contains(onto) => {
                let Some(to) = self.tabs.iter().position(|t| t.root.contains(onto)) else {
                    return;
                };
                beside = Some((onto, edge));
                Some(to)
            }
            PaneDrop::Pane(onto, edge) => {
                let tab = &mut self.tabs[from];
                if onto == pane {
                    return;
                }
                tab.zoomed = None;
                tab.root.remove(pane);
                tab.root.insert(onto, pane, edge);
                if viewer {
                    self.viewer_focus = true;
                } else {
                    tab.active_pane = pane;
                    self.viewer_focus = false;
                }
                self.save_session();
                return;
            }
            PaneDrop::Tab(to) if to == from || to >= self.tabs.len() => return,
            PaneDrop::Tab(to) => Some(to),
            PaneDrop::NewTab => None,
        };
        // out of the source tab
        let src = &mut self.tabs[from];
        let empties = !viewer && src.panes.len() <= 1;
        if to.is_none() && (viewer || empties) {
            return;
        }
        let state = src
            .panes
            .iter()
            .position(|p| p.session.id == pane)
            .map(|i| src.panes.remove(i));
        if !empties {
            src.zoomed = None;
            src.root.remove(pane);
            if src.active_pane == pane {
                src.active_pane = src
                    .root
                    .leaves()
                    .into_iter()
                    .find(|id| *id != VIEWER_PANE)
                    .unwrap_or(src.panes[0].session.id);
            }
        }
        // into the destination
        let dest = match to {
            Some(to) => {
                let dst = &mut self.tabs[to];
                dst.zoomed = None;
                let anchor = dst.active_pane;
                let placed = match beside {
                    Some((onto, edge)) => dst.root.insert(onto, pane, edge),
                    None => dst.root.split(anchor, pane, Axis::Horizontal),
                };
                if !placed {
                    dst.root = crate::pane::PaneNode::Split {
                        axis: Axis::Horizontal,
                        ratio: 0.5,
                        first: Box::new(dst.root.clone()),
                        second: Box::new(crate::pane::PaneNode::Leaf(pane)),
                    };
                }
                if let Some(state) = state {
                    dst.panes.push(state);
                    dst.active_pane = pane;
                }
                to
            }
            None => {
                let Some(state) = state else { return };
                self.tabs.insert(from + 1, TabState::new(state));
                from + 1
            }
        };
        let dest = if empties {
            self.tabs.remove(from);
            if dest > from { dest - 1 } else { dest }
        } else {
            dest
        };
        self.rename = None;
        self.viewer_focus = viewer;
        self.switch_tab(dest);
    }

    fn active_pane_id(&self) -> Option<PaneId> {
        Some(self.tabs.get(self.active)?.active()?.session.id)
    }

    /// A key release: reported only to programs that asked for event types
    /// (kitty flag 2), and only to the panes that received the press.
    pub(crate) fn on_key_release(&mut self, event: &input::KeyPress) {
        let identity = input::key_identity(event);
        let pressed: Vec<_> = self
            .keys_down
            .iter()
            .filter(|(_, k)| *k == identity)
            .cloned()
            .collect();
        let active = self.active_pane_id();
        let (mods, option_as_meta) = (self.modifiers, self.option_meta());
        for entry in pressed {
            self.keys_down.remove(&entry);
            let target = entry.0;
            if Some(target) == active && self.editor_active {
                continue;
            }
            if let Some((_, tab)) = self.find_tab_of_pane(target)
                && let Some(p) = tab.pane_mut(target)
                && let Some(bytes) =
                    input::key_to_bytes(event, mods, key_encoding(p, option_as_meta))
            {
                p.session.write(&bytes);
            }
        }
    }

    pub(crate) fn on_cursor_moved(&mut self, pos: (f32, f32), window: &Window) {
        self.mouse = pos;
        self.update_cursor_icon(window);
        // tabs, welcome rows, pane controls and settings rows draw their
        // hover from the mouse at render time: repaint as soon as the pointer
        // enters or leaves one, not on the next unrelated frame
        let sig = self
            .hits
            .iter()
            .enumerate()
            .filter(|(_, h)| h.contains(pos.0, pos.1))
            .fold(0u64, |s, (i, _)| {
                s.wrapping_mul(31).wrapping_add(i as u64 + 1)
            });
        if sig != self.hover_sig {
            self.hover_sig = sig;
            self.hover_repaints += 1;
            window.request_redraw();
        }
        self.pointer_in = true;
        if let Some(ed) = &mut self.prompt_editor {
            // a press that travels starts a drag; hover marks repaint
            if let Some(d) = &mut ed.drag
                && !d.live
                && ((pos.0 - d.press.0).abs() > crate::views::prompt_editor::DRAG_SLOP
                    || (pos.1 - d.press.1).abs() > crate::views::prompt_editor::DRAG_SLOP)
            {
                d.live = true;
            }
            window.request_redraw();
        }
        // a hovered diff line shows its hunk's toolbar; a file-list row lights
        let lit = self.review.hover_index();
        self.review.pointer = Some(pos);
        if self.review.hover_index() != lit {
            window.request_redraw();
        }
        let tip = self
            .tips
            .iter()
            .find(|t| t.contains(pos.0, pos.1))
            .map(|t| t.rect);
        if tip != self.tip_hover.map(|(r, _)| r) || (tip.is_none() && self.tip_shown.is_some()) {
            window.request_redraw();
        }
        if self.mouse_report.is_some() {
            self.report_motion();
            return;
        }
        let idle = !self.palette.open
            && !self.settings.open
            && !self.context_menu.open
            && self.chip_menu.is_none()
            && self.prompt_editor.is_none()
            && self.confirm.is_none()
            && !self.whats_new.open
            && self.drag.is_none()
            && self.pending_sel.is_none()
            && self.editor_drag.is_none()
            && self.tab_drag.is_none()
            && self.pane_drag.is_none()
            && self.slider_drag.is_none();
        if idle && self.report_motion() {
            return;
        }
        if let Some(drag) = &self.drag {
            let [ax, ay, aw, ah] = drag.area;
            let ratio = match drag.axis {
                Axis::Horizontal => (self.mouse.0 - ax) / (aw - crate::pane::HANDLE),
                Axis::Vertical => (self.mouse.1 - ay) / (ah - crate::pane::HANDLE),
            };
            let path = drag.path.clone();
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.root.set_ratio(&path, ratio);
            }
            window.request_redraw();
        }
        // the chip picker's highlight follows the pointer at once (its rows
        // are not frame hits, so nothing else repaints them)
        if let Some(menu) = &mut self.chip_menu
            && let Some(&(_, i)) = self.chip_menu_rows.iter().find(|(r, _)| {
                pos.0 >= r[0] && pos.0 < r[0] + r[2] && pos.1 >= r[1] && pos.1 < r[1] + r[3]
            })
            && i != menu.selected
        {
            menu.selected = i;
            window.request_redraw();
        }
        if self.context_menu.open {
            let (mx2, my2) = self.mouse;
            let hover = self
                .context_menu_rects
                .iter()
                .position(|r| mx2 >= r[0] && mx2 < r[0] + r[2] && my2 >= r[1] && my2 < r[1] + r[3]);
            if hover != self.context_menu.selected {
                self.context_menu.selected = hover;
                window.request_redraw();
            }
        }
        if self.pending_sel.is_some() {
            self.drag_selection();
        }
        if let Some(pane) = self.editor_drag {
            self.drag_editor(pane);
            window.request_redraw();
        }
        if self.pane_drag.is_some() {
            self.track_pane_drag();
            self.update_cursor_icon(window);
            window.request_redraw();
        }
        if let Some(TabDrag {
            index: i,
            start,
            press,
            ..
        }) = self.tab_drag
        {
            // drag a tab past a neighbour's midpoint to swap (Warp reorder);
            // the vertical rail reorders along y
            let vertical = self.config.tab_layout == shift_core::TabLayout::Vertical;
            let pos = self.tab_drag_coord();
            if let Some(d) = self.tab_drag.as_mut()
                && !d.live
                && (pos - press).abs() > PANE_DRAG_SLOP
            {
                d.live = true;
            }
            if self.tab_drag.is_some_and(|d| d.live) {
                window.request_redraw();
            }
            let delta = pos - start;
            // shell tabs only: the Settings tab after them never swaps
            let n_tabs = self.tabs.len();
            let neighbour = self.hits.iter().find_map(|h| match h.target {
                ClickTarget::TabSelect(j)
                    if j < n_tabs
                        && ((delta > 0.0 && j == i + 1) || (delta < 0.0 && j + 1 == i)) =>
                {
                    Some((j, h.rect))
                }
                _ => None,
            });
            if let Some((j, r)) = neighbour {
                let mid = if vertical {
                    r[1] + r[3] / 2.0
                } else {
                    r[0] + r[2] / 2.0
                };
                if (delta > 0.0 && pos > mid) || (delta < 0.0 && pos < mid) {
                    self.tabs.swap(i, j);
                    self.active = j;
                    if let Some(d) = self.tab_drag.as_mut() {
                        d.index = j;
                        d.start = pos;
                    }
                    self.save_session();
                    window.request_redraw();
                }
            }
        }
        if let Some((id, rect)) = self.slider_drag {
            let t = ((self.mouse.0 - rect[0] - 8.0) / (rect[2] - 16.0)).clamp(0.0, 1.0);
            self.set_slider(id, t);
            window.request_redraw();
        }
    }

    pub(crate) fn on_right_press(&mut self, window: &Window) {
        self.hide_tooltip(window);
        if self.chip_menu.take().is_some() {
            window.request_redraw();
            return;
        }
        if self.prompt_editor.is_some() {
            return;
        }
        if self.palette.open || self.confirm.is_some() || self.whats_new.open {
            return;
        }
        if self.notifications.open {
            self.close_notifications();
            window.request_redraw();
            return;
        }
        if !self.context_menu.open
            && !self.settings.open
            && self.report_button(crate::mouse::RIGHT, true)
        {
            window.request_redraw();
            return;
        }
        if self.over_maximized_review() {
            return;
        }
        let (mx, my) = self.mouse;
        // tab bar: tab menu
        let tab_hit = self
            .hits
            .iter()
            .find(|h| h.contains(mx, my))
            .and_then(|h| match h.target {
                ClickTarget::TabSelect(i) | ClickTarget::TabClose(i) => Some(i),
                _ => None,
            });
        if let Some(i) = tab_hit {
            self.open_tab_menu(i, self.mouse);
            window.request_redraw();
            return;
        }
        // the Settings tab's page has no context menus
        if self.settings.open {
            return;
        }
        // resolve the block under the cursor via the frame row cache
        let target = self.tabs.get(self.active).and_then(|tab| {
            tab.panes.iter().find_map(|p| {
                let grid = p.last_grid?;
                if my < grid.y || p.rows.is_empty() {
                    return None;
                }
                let idx = (((my - grid.y) / grid.line_h).floor() as i64)
                    .clamp(0, p.rows.len() as i64 - 1) as usize;
                if mx < grid.x || mx > grid.right {
                    return None;
                }
                use crate::views::terminal::RowKind;
                let blocks = p.session.blocks.lock().unwrap();
                match p.rows[idx].kind {
                    RowKind::Pad => None,
                    RowKind::Header(id) => Some((p.session.id, id)),
                    RowKind::Grid { abs, .. } => blocks
                        .iter()
                        .find(|b| {
                            b.suppressed_range().is_some()
                                && abs >= b.prompt_start
                                && abs < b.end.unwrap_or(u64::MAX)
                        })
                        .map(|b| (p.session.id, b.id)),
                }
            })
        });
        if let Some((pane_id, block)) = target {
            self.open_context_menu(pane_id, block, self.mouse);
            window.request_redraw();
            return;
        }
        // anywhere else in a pane: terminal menu for the pane under the mouse
        let (lw, lh) = self.logical_size();
        let (rects, _) = self.pane_rects(lw, lh);
        if let Some(pr) = rects.iter().find(|r| {
            mx >= r.rect[0]
                && mx < r.rect[0] + r.rect[2]
                && my >= r.rect[1]
                && my < r.rect[1] + r.rect[3]
        }) {
            let id = pr.id;
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.active_pane = id;
            }
            self.open_terminal_menu(id, self.mouse);
            window.request_redraw();
        }
    }

    pub(crate) fn on_left(
        &mut self,
        state: ElementState,
        event_loop: &ActiveEventLoop,
        window: &Window,
    ) {
        if state == ElementState::Pressed {
            self.hide_tooltip(window);
        }
        if state == ElementState::Released && self.prompt_editor.is_some() {
            self.prompt_editor_release();
            window.request_redraw();
            return;
        }
        if state == ElementState::Released {
            if self.report_button(crate::mouse::LEFT, false) {
                window.request_redraw();
                return;
            }
            if self.drag.take().is_some() {
                self.save_session(); // divider position
            }
            self.tab_drag = None;
            if let Some(d) = self.pane_drag.take() {
                if d.live
                    && let Some(target) = d.target
                {
                    self.drop_pane(d.pane, d.from_tab, target);
                } else if d.live && d.from_tab != self.active && d.from_tab < self.tabs.len() {
                    // released nowhere after opening another tab: back home
                    self.switch_tab(d.from_tab);
                }
                self.update_cursor_icon(window);
                window.request_redraw();
                return;
            }
            if self.editor_drag.take().is_some() {
                return;
            }
            let pending = self.pending_sel.take();
            let selecting = pending.as_ref().is_some_and(|p| p.started);
            // Warp: a click (no drag) on a command's output selects its block
            if let Some(p) = pending.filter(|p| !p.started) {
                self.select_block_at(p.pane, p.abs);
                window.request_redraw();
            }
            if selecting
                && self.config.copy_on_select
                && let Some(text) = self.selection_text()
            {
                self.copy_to_clipboard(text);
            }
            if self.slider_drag.take().is_some() {
                self.save_config();
            }
            return;
        }
        let (mx, my) = self.mouse;
        let inside =
            |r: &[f32; 4]| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3];
        if self.confirm.is_some() {
            if inside(&self.confirm_rects[1]) {
                if let Some(c) = self.confirm.take() {
                    self.perform_confirmed(c.action, event_loop);
                }
            } else if inside(&self.confirm_rects[0]) {
                self.cancel_confirm();
            }
            window.request_redraw();
            return;
        }
        if self.prompt_editor.is_some() {
            self.prompt_editor_press();
            window.request_redraw();
            return;
        }
        if self.whats_new.open {
            let h = self.whats_new_hits;
            if inside(&h.close) || inside(&h.done) || !inside(&h.card) {
                self.whats_new.open = false;
            }
            window.request_redraw();
            return;
        }
        if self.notifications.open {
            let h = self.notifications_hits.clone();
            if inside(&h.close) {
                self.close_notifications();
            } else if inside(&h.mark_read) {
                self.mark_all_read();
            } else if let Some((_, f)) = h.chips.iter().find(|(r, _)| inside(r)) {
                self.notifications.set_filter(*f);
            } else if let Some((_, id)) = h.rows.iter().find(|(r, _)| inside(r)) {
                self.close_notifications();
                self.show_attention(*id);
            } else if !inside(&h.card) {
                // outside closes; the inbox button itself must not reopen it
                self.close_notifications();
            }
            window.request_redraw();
            return;
        }
        // notification toasts float over everything below the modals
        if let Some(t) = self
            .note_toast_hits
            .iter()
            .find(|t| inside(&t.rect))
            .copied()
        {
            if inside(&t.close) {
                self.note_toasts.retain(|(id, _)| *id != t.id);
            } else {
                self.show_attention(t.id);
            }
            window.request_redraw();
            return;
        }
        if let Some((i, _)) = &self.rename {
            // clicking anywhere commits the rename
            let i = *i;
            self.commit_rename(i);
        }
        if self.chip_menu.is_some() {
            match self.chip_menu_rows.iter().find(|(r, _)| inside(r)) {
                Some(&(_, i)) => self.apply_chip_menu(i),
                None if inside(&self.chip_menu_card) => {}
                None => self.chip_menu = None,
            }
            window.request_redraw();
            return;
        }
        if self.context_menu.open {
            let row = self
                .context_menu_rects
                .iter()
                .position(|r| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]);
            match row {
                Some(i) => {
                    let item = &self.context_menu.items[i];
                    if item.enabled {
                        let action = item.action;
                        self.run_menu_action(action, event_loop);
                    }
                }
                None => self.context_menu.close(),
            }
            window.request_redraw();
            return;
        }
        if self.palette.open {
            let chip = self.hits.iter().find_map(|h| match h.target {
                ClickTarget::PaletteMode(i) if h.contains(mx, my) => Some(i as usize),
                _ => None,
            });
            if let Some(mode) = chip.and_then(|i| crate::views::palette::MODES.get(i)) {
                self.open_palette(mode.0);
                window.request_redraw();
                return;
            }
            let row = self
                .palette_row_hits
                .iter()
                .find(|(r, _)| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3])
                .map(|(_, i)| *i);
            match row {
                Some(i) => {
                    if let Some(m) = self.palette_results.get(i) {
                        let action = m.item.action.clone();
                        self.run_palette_action(action, event_loop);
                    }
                }
                None => self.palette.open = false,
            }
            window.request_redraw();
            return;
        }
        // find-bar toggles
        if self.search.open
            && !self.filter_editing
            && !self.over_maximized_review()
            && let Some(i) = self.search_chips.iter().position(inside)
        {
            if i == 0 {
                self.search.case_sensitive = !self.search.case_sensitive;
            } else {
                self.search.regex = !self.search.regex;
            }
            self.refresh_search(true);
            window.request_redraw();
            return;
        }
        if !self.settings.open && self.on_panel_click(inside) {
            self.last_click = None;
            window.request_redraw();
            return;
        }
        // a click anywhere else leaves the tree's keyboard focus, and one in
        // a shell pane takes focus from the viewer pane
        self.explorer.focused = false;
        if !self.settings.open && self.mouse.1 > TAB_BAR_H {
            self.viewer_focus = false;
        }
        // a program tracking the mouse (vim, tmux, htop) gets the click
        if !self.settings.open && self.report_button(crate::mouse::LEFT, true) {
            self.last_click = None;
            window.request_redraw();
            return;
        }
        let target = self
            .hits
            .iter()
            .find(|h| h.contains(mx, my))
            .map(|h| h.target.clone());
        // any click but another row ends a shortcut recording
        if !matches!(target, Some(ClickTarget::SettingsKeybind(_))) {
            self.settings.recording = None;
        }
        // a click outside the history pane (and its command line) closes it
        if self.history.open
            && !matches!(
                target,
                Some(
                    ClickTarget::HistoryRow(_)
                        | ClickTarget::HistoryScope(_)
                        | ClickTarget::HistoryClose
                        | ClickTarget::Editor(_)
                )
            )
        {
            self.history.close();
        }
        // an open select list: a click outside its options only closes it
        if self.settings.dropdown.is_some()
            && !matches!(target, Some(ClickTarget::SettingsOption(_)))
        {
            self.settings.dropdown = None;
            window.request_redraw();
            return;
        }
        // the tab strip leaves the Settings tab for a shell tab (its own
        // entry, after the shells, shows it again)
        if let Some(ClickTarget::TabSelect(i)) = target {
            if i >= self.tabs.len() {
                if self.settings.tab {
                    self.settings.open = true;
                }
                window.request_redraw();
                return;
            }
            self.settings.open = false;
        }
        match &target {
            Some(ClickTarget::Block(pane, id) | ClickTarget::BlockDuration(pane, id)) => {
                // Warp: clicking a block selects it (again deselects)
                let (pane, id) = (*pane, *id);
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    if tab.active_pane != pane
                        && let Some(prev) = tab.active_mut()
                    {
                        prev.selected_block = None;
                    }
                    tab.active_pane = pane;
                    if let Some(p) = tab.pane_mut(pane) {
                        p.selected_block = if p.selected_block == Some(id) {
                            None
                        } else {
                            Some(id)
                        };
                    }
                }
            }
            Some(ClickTarget::BlockChevron(pane, id)) => {
                if let Some((_, tab)) = self.find_tab_of_pane(*pane)
                    && let Some(p) = tab.pane(*pane)
                {
                    p.session.blocks.lock().unwrap().toggle_collapsed(*id);
                }
            }
            Some(ClickTarget::TabSelect(i)) => {
                let i = *i;
                if i < self.tabs.len() {
                    // double-click renames (Warp); a press arms drag-reorder
                    if self.click_count() == 2 {
                        self.start_rename(i);
                    }
                    if i != self.active {
                        self.switch_tab(i);
                    }
                    let pos = self.tab_drag_coord();
                    let vertical = self.config.tab_layout == shift_core::TabLayout::Vertical;
                    let grab = self
                        .hits
                        .iter()
                        .find(|h| h.target == ClickTarget::TabSelect(i))
                        .map(|h| pos - if vertical { h.rect[1] } else { h.rect[0] })
                        .unwrap_or(0.0);
                    self.tab_drag = Some(TabDrag {
                        index: i,
                        origin: i,
                        start: pos,
                        press: pos,
                        grab,
                        live: false,
                    });
                }
            }
            Some(ClickTarget::TabClose(i)) => {
                // closing a tab closes all its panes (warns if busy)
                self.request_close(CloseScope::Tab(*i), event_loop);
            }
            Some(ClickTarget::TabNew) => self.spawn_tab(),
            Some(ClickTarget::ToggleExplorer) => {
                self.handle_action(AppAction::ToggleExplorer, event_loop)
            }
            Some(ClickTarget::ToggleReview) => {
                self.handle_action(AppAction::ToggleReview, event_loop)
            }
            Some(ClickTarget::ToggleNotifications) => {
                self.handle_action(AppAction::AttentionCenter, event_loop)
            }
            Some(ClickTarget::UpdateChip) => self.handle_action(AppAction::UpdateShift, event_loop),
            Some(ClickTarget::Chip(pane, shift_core::PromptChip::GitDiff)) => {
                // opens the review of that pane's repository (focused first);
                // a second click closes it, like the tab bar's button
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.active_pane = *pane;
                }
                self.handle_action(AppAction::ToggleReview, event_loop)
            }
            Some(ClickTarget::Chip(pane, shift_core::PromptChip::GithubPr)) => {
                let url = self
                    .tabs
                    .get(self.active)
                    .and_then(|t| t.pane(*pane))
                    .and_then(|p| p.pr.as_ref())
                    .map(|pr| pr.url.clone());
                if let Some(url) = url {
                    self.open_target(&url);
                }
            }
            Some(ClickTarget::Chip(pane, chip)) => {
                if let Some(kind) = crate::views::chip_menu::ChipKind::of(*chip) {
                    self.open_chip_menu(*pane, kind);
                }
            }
            Some(ClickTarget::Editor(pane)) => {
                let pane = *pane;
                let clicks = self.click_count();
                self.click_editor(pane, clicks);
                self.editor_drag = Some(pane);
            }
            Some(ClickTarget::PaneFocus(id)) => {
                let id = *id;
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    if tab.active_pane != id
                        && let Some(prev) = tab.active_mut()
                    {
                        prev.selected_block = None;
                    }
                    tab.active_pane = id;
                }
                let clicks = self.click_count();
                if self.try_open_url(id) {
                    self.last_click = None;
                } else {
                    self.press_selection(id, clicks);
                }
            }
            Some(ClickTarget::PaneHeader(id)) => {
                // focus the pane; dragging the bar past the slop moves it
                let id = *id;
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    if tab.active_pane != id
                        && let Some(prev) = tab.active_mut()
                    {
                        prev.selected_block = None;
                    }
                    tab.active_pane = id;
                }
                self.pane_drag = Some(PaneDrag {
                    pane: id,
                    from_tab: self.active,
                    press: self.mouse,
                    live: false,
                    target: None,
                    hover_tab: None,
                });
            }
            Some(ClickTarget::PaneMenu(id)) => {
                let id = *id;
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.active_pane = id;
                }
                // drops down from the ⋮ button
                let at = self
                    .hits
                    .iter()
                    .find(|h| h.target == ClickTarget::PaneMenu(id))
                    .map_or(self.mouse, |h| (h.rect[0], h.rect[1] + h.rect[3] + 4.0));
                self.open_terminal_menu(id, at);
            }
            Some(ClickTarget::PaneClose(id)) => {
                self.request_close(CloseScope::Pane(*id), event_loop);
            }
            Some(ClickTarget::PaletteMode(i)) => {
                if let Some(m) = crate::views::palette::MODES.get(*i as usize) {
                    self.open_palette(m.0);
                }
            }
            Some(ClickTarget::HistoryScope(scope)) => self.history.set_scope(*scope),
            Some(ClickTarget::HistoryClose) => self.history.close(),
            Some(ClickTarget::HistoryRow(i)) => {
                // a click picks the row onto the command line; a second runs it
                if self.history.selected == *i && self.click_count() >= 2 {
                    self.take_history(HistoryUse::Run);
                } else {
                    self.history.selected = *i;
                    self.take_history(HistoryUse::Edit);
                }
            }
            Some(ClickTarget::CompletionSource(i)) => {
                self.completion.pick_source(*i as usize);
            }
            Some(ClickTarget::CompletionRow(i)) => {
                self.completion.selected = *i;
                self.accept_completion();
                self.blink_epoch = Instant::now();
            }
            Some(ClickTarget::Welcome(i)) => match i {
                0 => self.handle_action(AppAction::SplitRight, event_loop),
                1 => self.handle_action(AppAction::TogglePalette, event_loop),
                2 => self.open_history(),
                3 => self.handle_action(AppAction::ToggleExplorer, event_loop),
                _ => self.handle_action(AppAction::OpenSettings, event_loop),
            },
            Some(ClickTarget::PaneSplit(id)) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.active_pane = *id;
                }
                self.viewer_focus = false;
                self.split_active(Axis::Horizontal);
            }
            Some(ClickTarget::HandleDrag(path)) => {
                // find the split node's area from the current layout
                let (lw, lh) = self.logical_size();
                let (_, handles) = self.pane_rects(lw, lh);
                if let Some(hr) = handles.iter().find(|h| &h.path == path) {
                    self.drag = Some(DragState {
                        path: path.clone(),
                        axis: hr.axis,
                        area: hr.area,
                    });
                }
            }
            Some(ClickTarget::BlockCopy(pane, id)) => {
                let text = self
                    .find_tab_of_pane(*pane)
                    .and_then(|(_, tab)| tab.pane(*pane))
                    .and_then(|p| terminal::block_output_text(&p.session, *id));
                if let Some(text) = text {
                    self.copy_to_clipboard(text);
                }
            }
            Some(ClickTarget::TrafficLight(i)) => match i {
                0 => self.request_close(CloseScope::Window, event_loop),
                _ if self.headless => {}
                1 => window.set_minimized(true),
                _ => window.set_maximized(!window.is_maximized()),
            },
            // the empty strip: a double-click zooms the window (the titlebar's
            // own gesture), a press drags it
            Some(ClickTarget::WindowDrag) if !self.headless => {
                if self.click_count() == 2 {
                    window.set_maximized(!window.is_maximized());
                } else {
                    let _ = window.drag_window();
                }
            }
            Some(ClickTarget::WindowDrag) => {}
            Some(ClickTarget::JumpToBlock(pane, id)) => {
                self.jump_to_block(*pane, *id);
            }
            Some(ClickTarget::SettingsNoop) => {}
            Some(ClickTarget::SettingsNav(i)) => {
                use crate::views::settings::Page;
                let page = Page::ALL.get(*i as usize).copied().unwrap_or(Page::General);
                self.settings.open_page(page);
            }
            Some(ClickTarget::SettingsClose) => self.settings.close_tab(),
            Some(ClickTarget::SettingsOption(i)) => {
                if let Some(dd) = self.settings.dropdown.take()
                    && let Some((value, _)) = dd.options.get(*i)
                {
                    self.set_select(dd.id, value.clone());
                }
            }
            Some(ClickTarget::SettingsReset(id)) => {
                if let Some(config) = crate::views::settings::reset_setting(&self.config, id) {
                    self.config = config;
                    self.save_config();
                    self.apply_config();
                }
            }
            Some(ClickTarget::SettingsUpdate) => self.update_shift(),
            Some(ClickTarget::SettingsCopyVersion) => {
                if self.copy_to_clipboard(format!("v{}", env!("CARGO_PKG_VERSION"))) {
                    self.toast("Copied version".into());
                }
            }
            Some(ClickTarget::SettingsOpenConfig) => {
                if let Some(path) = shift_core::config::config_path() {
                    self.settings.open = false;
                    self.open_viewer(&path);
                }
            }
            Some(ClickTarget::SettingsToggle(id)) => {
                let c = &mut self.config;
                match *id {
                    "cursor_blink" => c.cursor_blink = !c.cursor_blink,
                    "ligatures" => c.ligatures = !c.ligatures,
                    "sync_os_theme" => c.sync_os_theme = !c.sync_os_theme,
                    "restore_session" => c.restore_session = !c.restore_session,
                    "vim_mode" => c.vim_mode = !c.vim_mode,
                    "autosuggestions" => c.autosuggestions = !c.autosuggestions,
                    "completions_menu" => c.completions_menu = !c.completions_menu,
                    "autocomplete_pairs" => c.autocomplete_pairs = !c.autocomplete_pairs,
                    "copy_on_select" => c.copy_on_select = !c.copy_on_select,
                    "notify_on_completion" => c.notify_on_completion = !c.notify_on_completion,
                    "alias_expansion" => c.alias_expansion = !c.alias_expansion,
                    "error_underline" => c.error_underline = !c.error_underline,
                    "command_corrections" => c.command_corrections = !c.command_corrections,
                    "confirm_close_running" => c.confirm_close_running = !c.confirm_close_running,
                    "confirm_multiline_paste" => {
                        c.confirm_multiline_paste = !c.confirm_multiline_paste
                    }
                    "durable_sessions" => c.durable_sessions = !c.durable_sessions,
                    "check_for_updates" => c.check_for_updates = !c.check_for_updates,
                    // off deletes the saved output (apply_config)
                    "save_transcripts" => c.save_transcripts = !c.save_transcripts,
                    "mouse_reporting" => c.mouse_reporting = !c.mouse_reporting,
                    "option_as_meta" => {
                        // the right key keeps what it did: it followed this
                        // setting while unset
                        c.right_option_as_meta = Some(c.right_option_meta());
                        c.option_as_meta = !c.option_as_meta;
                    }
                    "right_option_as_meta" => c.right_option_as_meta = Some(!c.right_option_meta()),
                    "middle_click_paste" => c.middle_click_paste = !c.middle_click_paste,
                    "explorer_hidden_files" => c.explorer_hidden_files = !c.explorer_hidden_files,
                    "notifications.enabled" => c.notifications.enabled = !c.notifications.enabled,
                    "notifications.toasts" => c.notifications.toasts = !c.notifications.toasts,
                    "notifications.command_failed" => {
                        c.notifications.command_failed = !c.notifications.command_failed
                    }
                    "notifications.agent_task_completed" => {
                        c.notifications.agent_task_completed = !c.notifications.agent_task_completed
                    }
                    "notifications.needs_attention" => {
                        c.notifications.needs_attention = !c.notifications.needs_attention
                    }
                    "notifications.password_prompt" => {
                        c.notifications.password_prompt = !c.notifications.password_prompt
                    }
                    "notifications.play_sound" => {
                        c.notifications.play_sound = !c.notifications.play_sound
                    }
                    _ => {}
                }
                self.save_config();
                self.apply_config();
            }
            Some(ClickTarget::SettingsStep(id, dir)) => {
                let c = &mut self.config;
                let d = *dir as i64;
                match *id {
                    "font_size" => {
                        // from the applied size: a NaN or out-of-range value still steps
                        c.font_size = (c.font_size() + d as f32).clamp(8.0, 28.0);
                    }
                    "scrollback" => {
                        c.scrollback =
                            (c.scrollback as i64 + d * 1000).clamp(1000, 1_000_000) as usize;
                    }
                    "notify_threshold_secs" => {
                        c.notify_threshold_secs =
                            (c.notify_threshold_secs as i64 + d * 5).clamp(0, 3600) as u64;
                    }
                    "window_padding" => {
                        c.window_padding = (c.padding() + d as f32 * 2.0).clamp(0.0, 40.0);
                    }
                    "wheel_scroll_lines" => {
                        c.wheel_scroll_lines = (c.wheel_lines() as i64 + d).clamp(1, 20) as u8;
                    }
                    "notifications.toast_secs" => {
                        let secs = c.notifications.toast_duration().as_secs() as i64;
                        c.notifications.toast_secs = (secs + d).clamp(2, 60) as u64;
                    }
                    _ => {}
                }
                self.save_config();
                self.apply_config();
            }
            Some(ClickTarget::SettingsEditPrompt) => {
                self.prompt_editor = Some(crate::views::prompt_editor::PromptEditor::new(
                    &self.config.prompt_chips,
                ));
            }
            Some(ClickTarget::SettingsSegment(id, v)) => {
                use shift_core::{Bell, CursorStyle, InputPosition, PromptMode};
                let c = &mut self.config;
                match *id {
                    "cursor_style" => {
                        c.cursor_style = match v {
                            0 => CursorStyle::Block,
                            1 => CursorStyle::Bar,
                            _ => CursorStyle::Underline,
                        }
                    }
                    "input_position" => {
                        c.input_position = if *v == 1 {
                            InputPosition::Top
                        } else {
                            InputPosition::Bottom
                        }
                    }
                    "blocks_start" => {
                        c.blocks_start = if *v == 1 {
                            shift_core::BlocksStart::Top
                        } else {
                            shift_core::BlocksStart::Bottom
                        }
                    }
                    "control_api" => {
                        c.control_api = ["off", "navigate", "full"]
                            .get(*v as usize)
                            .unwrap_or(&"navigate")
                            .to_string();
                    }
                    "clipboard_access" => {
                        c.clipboard_access = ["off", "write", "read_write"]
                            .get(*v as usize)
                            .unwrap_or(&"write")
                            .to_string();
                    }
                    "tab_layout" => {
                        c.tab_layout = if *v == 1 {
                            shift_core::TabLayout::Vertical
                        } else {
                            shift_core::TabLayout::Horizontal
                        }
                    }
                    "prompt" => {
                        c.prompt = if *v == 1 {
                            PromptMode::Shell
                        } else {
                            PromptMode::Shift
                        }
                    }
                    "working_directory" => match v {
                        0 => c.working_directory = "previous".into(),
                        1 => c.working_directory = "home".into(),
                        // a custom path is typed in config.toml; keep one already set
                        _ => {
                            if matches!(c.working_directory.trim(), "" | "previous" | "home") {
                                self.toast = Some((
                                    "Set working_directory = \"/path\" in config.toml".into(),
                                    Instant::now(),
                                ));
                            }
                        }
                    },
                    "bell" => {
                        c.bell = match v {
                            0 => Bell::Visual,
                            1 => Bell::Audible,
                            2 => Bell::Both,
                            _ => Bell::None,
                        }
                    }
                    _ => {}
                }
                self.save_config();
                self.apply_config();
            }
            Some(ClickTarget::SettingsCycle(id)) => self.open_select(id),
            Some(ClickTarget::SettingsSlider(id)) => {
                let rect = self
                    .hits
                    .iter()
                    .find(|h| matches!(&h.target, ClickTarget::SettingsSlider(s) if s == id))
                    .map(|h| h.rect);
                if let Some(rect) = rect {
                    self.slider_drag = Some((id, rect));
                    let t = ((mx - rect[0] - 8.0) / (rect[2] - 16.0)).clamp(0.0, 1.0);
                    self.set_slider(id, t);
                }
            }
            Some(ClickTarget::SettingsKeybind(action)) => {
                self.settings.recording = Some(action.clone());
            }
            Some(ClickTarget::SettingsKeybindReset(action)) => {
                self.settings.recording = None;
                self.config.keybindings.remove(action);
                self.save_config();
                self.apply_config();
                self.toast(format!("{action} restored to its default shortcut"));
            }
            Some(ClickTarget::SettingsTheme(i)) => {
                let names = shift_core::theme::available_themes();
                if let Some(name) = names.into_iter().nth(*i as usize) {
                    self.config.theme = name;
                    self.save_config();
                    self.apply_config();
                }
            }
            None => {
                let active_pane = self.tabs.get(self.active).map(|t| t.active_pane);
                if let Some(id) = active_pane {
                    let clicks = self.click_count();
                    if self.try_open_url(id) {
                        self.last_click = None;
                    } else {
                        self.press_selection(id, clicks);
                    }
                    window.request_redraw();
                }
            }
        }
        if target.is_some() {
            window.request_redraw();
        }
    }

    pub(crate) fn on_wheel(&mut self, delta: MouseScrollDelta, window: &Window) {
        self.hide_tooltip(window);
        // the modal holds still; nothing scrolls under it
        if self.prompt_editor.is_some() {
            return;
        }
        use shift_pty::alacritty_terminal::term::TermMode;
        // the What's New body scrolls by pixels: trackpad deltas stay smooth
        // instead of snapping to whole lines
        if self.whats_new.open && self.confirm.is_none() {
            let px = match delta {
                MouseScrollDelta::LineDelta(_, y) => y * 3.0 * 24.0,
                MouseScrollDelta::PixelDelta(p) => p.y as f32,
            };
            // positive delta = wheel up = toward the top
            self.whats_new.scroll_by(-px);
            window.request_redraw();
            return;
        }
        if let Some(menu) = &mut self.chip_menu {
            // by pixels: a trackpad's many small deltas move it smoothly
            // (rounding each to whole rows dropped most of them)
            let px = match delta {
                MouseScrollDelta::LineDelta(_, y) => y * crate::views::chip_menu::ChipMenu::NOTCH,
                MouseScrollDelta::PixelDelta(p) => p.y as f32,
            };
            let before = menu.scroll;
            menu.scroll_px(-px);
            if menu.scroll != before {
                window.request_redraw();
            }
            return;
        }
        if self.notifications.open && self.confirm.is_none() {
            let (mx, my) = self.mouse;
            let c = self.notifications_hits.card;
            if mx >= c[0] && mx < c[0] + c[2] && my >= c[1] && my < c[1] + c[3] {
                let px = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 3.0 * 20.0,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                self.notifications.scroll_by(-px);
                window.request_redraw();
                return;
            }
        }
        // the viewer and side panels scroll by pixels too
        if !self.palette.open && !self.settings.open && self.confirm.is_none() {
            let (mx, my) = self.mouse;
            let inside = |r: &[f32; 4]| {
                r[2] > 0.0 && mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
            };
            let px = -match delta {
                MouseScrollDelta::LineDelta(_, y) => y * 3.0 * 20.0,
                MouseScrollDelta::PixelDelta(p) => p.y as f32,
            };
            let (ex_rect, rv_rect) = self.panel_rects();
            let handled = if self.viewer.is_open() && inside(&self.viewer_rect) {
                // sideways: a trackpad's x, or ⇧ + a mouse wheel
                let dx = -match delta {
                    MouseScrollDelta::LineDelta(x, _) => x * 3.0 * 20.0,
                    MouseScrollDelta::PixelDelta(p) => p.x as f32,
                };
                let (px, dx) = if self.modifiers.shift_key() && dx == 0.0 {
                    (0.0, px)
                } else {
                    (px, dx)
                };
                self.viewer.scroll_by(px);
                if dx != 0.0 {
                    self.viewer.scroll_x_by(dx, my);
                }
                true
            } else if inside(&ex_rect) {
                self.explorer.scroll_by(px);
                true
            } else if inside(&self.review.side_rect) {
                self.review.side_scroll_by(px);
                true
            } else if inside(&rv_rect) {
                // sideways: a trackpad's x, or ⇧ + a mouse wheel
                let dx = -match delta {
                    MouseScrollDelta::LineDelta(x, _) => x * 3.0 * 20.0,
                    MouseScrollDelta::PixelDelta(p) => p.x as f32,
                };
                let (px, dx) = if self.modifiers.shift_key() && dx == 0.0 {
                    (0.0, px)
                } else {
                    (px, dx)
                };
                self.review.scroll_by(px);
                if dx != 0.0 {
                    self.review.scroll_x_by(dx, my);
                }
                true
            } else {
                false
            };
            if handled {
                window.request_redraw();
                return;
            }
        }
        // accumulate fractional deltas: trackpads emit many sub-line
        // pixel events that would each truncate to zero lines
        self.scroll_accum += match delta {
            MouseScrollDelta::LineDelta(_, y) => y * self.config.wheel_lines(),
            MouseScrollDelta::PixelDelta(p) => p.y as f32 / self.line_h,
        };
        let lines = self.scroll_accum as i32;
        self.scroll_accum -= lines as f32;
        if lines == 0 {
            return;
        }
        if self.settings.open {
            self.settings.scroll_by(lines as f32);
            window.request_redraw();
            return;
        }
        if self.palette.open || self.confirm.is_some() || self.context_menu.open {
            return;
        }
        // the wheel scrolls the pane under the pointer (Warp), not the focused one
        let Some(target) = self
            .pane_under_mouse()
            .or_else(|| self.tabs.get(self.active).map(|t| t.active_pane))
        else {
            return;
        };
        // mouse-tracking app: wheel reports at the pointer
        if let Some(mode) = self.mouse_tracking(target)
            && let Some(cell) = self.mouse_cell(target)
        {
            let button = if lines > 0 {
                crate::mouse::WHEEL_UP
            } else {
                crate::mouse::WHEEL_DOWN
            };
            for _ in 0..lines.unsigned_abs().min(20) {
                self.send_mouse(target, mode, button, true, false, cell);
            }
            return;
        }
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(target))
        else {
            return;
        };
        let mode = *pane.session.term.lock().mode();
        if mode.contains(TermMode::ALT_SCREEN) {
            // alternate scroll (DECSET 1007): the wheel becomes arrow keys
            // for full-screen apps without mouse tracking (less, man)
            if mode.contains(TermMode::ALTERNATE_SCROLL) {
                let app = mode.contains(TermMode::APP_CURSOR);
                let seq: &[u8] = match (lines > 0, app) {
                    (true, false) => b"\x1b[A",
                    (true, true) => b"\x1bOA",
                    (false, false) => b"\x1b[B",
                    (false, true) => b"\x1bOB",
                };
                for _ in 0..lines.unsigned_abs().min(20) {
                    pane.session.write(seq);
                }
            }
        } else {
            pane.view_offset = if lines > 0 {
                pane.view_offset.saturating_add(lines as usize)
            } else {
                pane.view_offset
                    .saturating_sub(lines.unsigned_abs() as usize)
            };
            window.request_redraw();
        }
    }

    /// Pane whose rect (in the active tab) contains the pointer.
    fn pane_under_mouse(&self) -> Option<PaneId> {
        if self.over_maximized_review() {
            return None;
        }
        let (lw, lh) = self.logical_size();
        let (rects, _) = self.pane_rects(lw, lh);
        let (mx, my) = self.mouse;
        rects
            .iter()
            .find(|r| {
                mx >= r.rect[0]
                    && mx < r.rect[0] + r.rect[2]
                    && my >= r.rect[1]
                    && my < r.rect[1] + r.rect[3]
            })
            .map(|r| r.id)
    }

    /// Terminal mode of `pane` when its program tracks the mouse and
    /// reporting applies (Shift forces local selection, xterm-style).
    fn mouse_tracking(
        &self,
        pane: PaneId,
    ) -> Option<shift_pty::alacritty_terminal::term::TermMode> {
        use shift_pty::alacritty_terminal::term::TermMode;
        if !self.config.mouse_reporting || self.modifiers.shift_key() {
            return None;
        }
        let p = self.tabs.get(self.active)?.pane(pane)?;
        let mode = *p.session.term.lock().mode();
        mode.intersects(TermMode::MOUSE_MODE).then_some(mode)
    }

    /// 0-based screen cell under the pointer inside `pane`'s grid (None over
    /// history rows, block headers' gutters, or outside the grid).
    fn mouse_cell(&self, pane: PaneId) -> Option<(usize, usize)> {
        let p = self.tabs.get(self.active)?.pane(pane)?;
        let g = p.last_grid?;
        let (mx, my) = self.mouse;
        if mx < g.x || mx > g.right || my < g.y || my >= g.y + g.rows as f32 * g.line_h {
            return None;
        }
        let (point, _) = Self::grid_point_at(p, mx, my)?;
        (point.line.0 >= 0).then_some((point.column.0, point.line.0 as usize))
    }

    /// Write one mouse report to `pane`.
    fn send_mouse(
        &self,
        pane: PaneId,
        mode: shift_pty::alacritty_terminal::term::TermMode,
        button: u8,
        pressed: bool,
        motion: bool,
        cell: (usize, usize),
    ) {
        use shift_pty::alacritty_terminal::term::TermMode;
        let enc = crate::mouse::Encoding {
            sgr: mode.contains(TermMode::SGR_MOUSE),
            utf8: mode.contains(TermMode::UTF8_MOUSE),
        };
        if let Some(bytes) =
            crate::mouse::report(button, pressed, motion, self.modifiers, cell.0, cell.1, enc)
            && let Some(p) = self.tabs.get(self.active).and_then(|t| t.pane(pane))
        {
            p.session.write(&bytes);
        }
    }

    /// Button press/release over a mouse-tracking program. Returns true when
    /// the event was reported (it then drives no local selection or menu).
    fn report_button(&mut self, button: u8, pressed: bool) -> bool {
        use shift_pty::alacritty_terminal::term::TermMode;
        if !pressed {
            let Some((pane, held, cell)) = self.mouse_report else {
                return false;
            };
            if held != button {
                return false;
            }
            self.mouse_report = None;
            let mode = self
                .tabs
                .get(self.active)
                .and_then(|t| t.pane(pane))
                .map(|p| *p.session.term.lock().mode())
                .unwrap_or(TermMode::empty());
            if mode.intersects(TermMode::MOUSE_MODE) {
                let cell = self.mouse_cell(pane).unwrap_or(cell);
                self.send_mouse(pane, mode, button, false, false, cell);
            }
            return true;
        }
        let Some(pane) = self.pane_under_mouse() else {
            return false;
        };
        let Some(mode) = self.mouse_tracking(pane) else {
            return false;
        };
        let Some(cell) = self.mouse_cell(pane) else {
            return false;
        };
        // the click still focuses the pane it lands in
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if tab.active_pane != pane
                && let Some(prev) = tab.active_mut()
            {
                prev.selected_block = None;
            }
            tab.active_pane = pane;
            if let Some(p) = tab.pane_mut(pane) {
                p.clear_selection();
            }
        }
        self.send_mouse(pane, mode, button, true, false, cell);
        self.mouse_report = Some((pane, button, cell));
        true
    }

    /// Pointer motion over a mouse-tracking program: drag reports (modes
    /// 1002/1003) while a button is held, hover reports (1003) otherwise.
    /// Returns true when the motion belongs to the program.
    fn report_motion(&mut self) -> bool {
        use shift_pty::alacritty_terminal::term::TermMode;
        if let Some((pane, button, last)) = self.mouse_report {
            let mode = self
                .tabs
                .get(self.active)
                .and_then(|t| t.pane(pane))
                .map(|p| *p.session.term.lock().mode())
                .unwrap_or(TermMode::empty());
            if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
                && let Some(cell) = self.mouse_cell(pane)
                && cell != last
            {
                self.send_mouse(pane, mode, button, true, true, cell);
                self.mouse_report = Some((pane, button, cell));
            }
            return true;
        }
        let Some(pane) = self.pane_under_mouse() else {
            return false;
        };
        let Some(mode) = self.mouse_tracking(pane) else {
            return false;
        };
        if !mode.contains(TermMode::MOUSE_MOTION) {
            return false;
        }
        let Some(cell) = self.mouse_cell(pane) else {
            return false;
        };
        if self.mouse_hover_cell != Some((pane, cell)) {
            self.mouse_hover_cell = Some((pane, cell));
            self.send_mouse(pane, mode, crate::mouse::NONE, true, true, cell);
        }
        true
    }

    /// Right/middle button release (press routing lives in on_right_press).
    pub(crate) fn on_other_button(&mut self, button: u8, pressed: bool, window: &Window) {
        let overlay = self.palette.open
            || self.settings.open
            || self.confirm.is_some()
            || self.whats_new.open
            || self.context_menu.open;
        if pressed && overlay {
            return;
        }
        if self.report_button(button, pressed) {
            window.request_redraw();
            return;
        }
        // middle-click pastes into the shell pane under the pointer, which
        // takes focus first (Warp)
        if button == crate::mouse::MIDDLE
            && pressed
            && self.config.middle_click_paste
            && let Some(id) = self.pane_under_mouse().filter(|id| *id != VIEWER_PANE)
        {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                if tab.active_pane != id
                    && let Some(prev) = tab.active_mut()
                {
                    prev.selected_block = None;
                }
                tab.active_pane = id;
            }
            self.viewer_focus = false;
            self.paste_from_clipboard();
            window.request_redraw();
        }
    }

    /// Text committed by the input method (or any composed text): routed
    /// exactly like typed characters.
    pub(crate) fn commit_text(
        &mut self,
        text: &str,
        event_loop: &ActiveEventLoop,
        window: &Window,
    ) {
        self.preedit.clear();
        if text.is_empty() {
            return;
        }
        let press = input::KeyPress::new(
            winit::keyboard::Key::Character(text.into()),
            Some(text.to_string()),
            None,
        );
        let saved = self.modifiers;
        self.modifiers = ModifiersState::empty();
        self.on_key(&press, event_loop, window);
        self.modifiers = saved;
        window.request_redraw();
    }

    /// A menu bar item: replay its chord through `on_key` (identical to the
    /// shortcut); an action with no binding runs directly.
    #[cfg(target_os = "macos")]
    fn run_menu_command(&mut self, id: &str, event_loop: &ActiveEventLoop) {
        use crate::menubar::{MenuCommand, chord_for};
        let Some(cmd) = MenuCommand::from_id(id) else {
            return;
        };
        let Some(window) = self.window.clone() else {
            return;
        };
        let chord = match &cmd {
            MenuCommand::Action(a) => chord_for(&self.keymap, a),
            MenuCommand::Chord(c) => Some(c.clone()),
        };
        if let Some((press, mods)) = chord.as_deref().and_then(crate::script::key_press) {
            let saved = self.modifiers;
            self.modifiers = mods;
            self.on_key(&press, event_loop, &window);
            self.modifiers = saved;
        } else if let MenuCommand::Action(a) = cmd
            && let Some(action) = input::action_from_name(&a)
        {
            self.handle_action(action, event_loop);
        }
        window.request_redraw();
    }

    /// Keep programs that asked for focus events (DECSET 1004: vim autoread,
    /// tmux) told which pane has focus: focus-out to the pane that lost it
    /// (window deactivated, other pane or tab selected), focus-in to the one
    /// that gained it. Independent of the mouse-reporting setting.
    fn sync_focus(&mut self) {
        use shift_pty::alacritty_terminal::term::TermMode;
        let want = if self.focused {
            self.active_pane_id()
        } else {
            None
        };
        if want == self.focus_pane {
            return;
        }
        let notify = |tabs: &[TabState], id: PaneId, seq: &[u8]| {
            if let Some(p) = tabs.iter().find_map(|t| t.pane(id))
                && p.session
                    .term
                    .lock()
                    .mode()
                    .contains(TermMode::FOCUS_IN_OUT)
            {
                p.session.write(seq);
            }
        };
        if let Some(old) = self.focus_pane {
            notify(&self.tabs, old, b"\x1b[O");
        }
        if let Some(new) = want {
            notify(&self.tabs, new, b"\x1b[I");
        }
        self.focus_pane = want;
    }
}

impl App {
    /// Open every tab/split of a launch configuration and queue its commands.
    fn apply_launch_config(&mut self, cfg: &shift_core::launch::LaunchConfig) {
        let first_new = self.tabs.len();
        for tab in &cfg.tabs {
            let (cwd, _) = Self::first_leaf(&tab.layout);
            self.spawn_tab_at(cwd.filter(|p| p.is_dir()));
            let root_pane = self.tabs[self.active].active_pane;
            if let Some(title) = &tab.title {
                self.tabs[self.active].custom_title = Some(title.clone());
            }
            self.build_layout(&tab.layout, root_pane);
        }
        self.active = (first_new + cfg.active_tab).min(self.tabs.len().saturating_sub(1));
        self.save_session();
    }

    /// cwd + commands of the first (top-left) pane of a layout.
    fn first_leaf(
        layout: &shift_core::launch::LaunchLayout,
    ) -> (Option<std::path::PathBuf>, Vec<String>) {
        use shift_core::launch::LaunchLayout;
        match layout {
            LaunchLayout::Pane { cwd, commands, .. } => (cwd.clone(), commands.clone()),
            LaunchLayout::Split { panes, .. } => panes
                .first()
                .map(Self::first_leaf)
                .unwrap_or((None, Vec::new())),
        }
    }

    /// Realize `layout` inside the region currently held by `pane`.
    fn build_layout(&mut self, layout: &shift_core::launch::LaunchLayout, pane: PaneId) {
        use shift_core::launch::LaunchLayout;
        match layout {
            LaunchLayout::Pane {
                commands, focused, ..
            } => {
                if !commands.is_empty() {
                    self.pending_commands.insert(pane, commands.clone());
                }
                if *focused && let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.active_pane = pane;
                }
            }
            LaunchLayout::Split { vertical, panes } => {
                let axis = if *vertical {
                    Axis::Horizontal // side by side
                } else {
                    Axis::Vertical
                };
                // pane holds panes[0]; each further child splits off the previous one
                let mut ids = vec![pane];
                let mut prev = pane;
                for child in panes.iter().skip(1) {
                    let (cwd, _) = Self::first_leaf(child);
                    let new_id = self.split_pane(prev, axis, cwd.filter(|p| p.is_dir()));
                    ids.push(new_id);
                    prev = new_id;
                }
                for (child, id) in panes.iter().zip(ids) {
                    self.build_layout(child, id);
                }
            }
        }
    }

    /// Split `target` along `axis`, spawning the new pane at `cwd`.
    fn split_pane(
        &mut self,
        target: PaneId,
        axis: Axis,
        cwd: Option<std::path::PathBuf>,
    ) -> PaneId {
        let session = self.spawn_session(60, 20, cwd); // resized on next layout
        let new_id = session.id;
        let pane = self.new_pane(session);
        let tab = &mut self.tabs[self.active];
        tab.zoomed = None;
        if tab.root.split(target, new_id, axis) {
            tab.panes.push(pane);
            tab.active_pane = new_id;
        }
        new_id
    }

    /// Send a launch configuration's queued commands once the pane's shell
    /// shows its first prompt.
    fn flush_pending_commands(&mut self, pane: PaneId) {
        if !self.pending_commands.contains_key(&pane) {
            return;
        }
        let ready = self
            .find_tab_of_pane(pane)
            .and_then(|(_, tab)| tab.pane(pane))
            .is_some_and(|p| {
                p.session
                    .saw_output
                    .load(std::sync::atomic::Ordering::Acquire)
                    && matches!(
                        p.session.blocks.lock().unwrap().last().map(|b| &b.state),
                        Some(BlockState::Prompt)
                    )
            });
        if !ready {
            return;
        }
        let Some(cmds) = self.pending_commands.remove(&pane) else {
            return;
        };
        for cmd in cmds {
            self.submit_to(pane, &cmd);
        }
    }

    /// Settings slider value (0..1) → config field.
    fn set_slider(&mut self, id: &str, t: f32) {
        match id {
            "line_height" => self.config.line_height = ((1.0 + 1.2 * t) * 100.0).round() / 100.0,
            // whole percents and pixels, as Warp's sliders
            "window_opacity" => self.config.window_opacity = (1.0 + 99.0 * t).round() / 100.0,
            "window_blur" => self.config.window_blur = (1.0 + 63.0 * t).round() as u8,
            _ => return,
        }
        self.apply_config();
    }

    /// Vim modal editing: route the key through the editor's vim state
    /// first. Returns true when vim consumed it.
    fn vim_intercept(&mut self, event: &input::KeyPress) -> bool {
        use crate::vim::{VimKey, VimMode, VimOutcome};
        use winit::keyboard::{Key, NamedKey};
        let mods = self.modifiers;
        if mods.super_key() || mods.control_key() || mods.alt_key() {
            return false;
        }
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            return false;
        };
        if !pane.editor.vim_enabled() {
            return false;
        }
        let is_escape = matches!(event.logical_key, Key::Named(NamedKey::Escape));
        // Esc closes the completion menu before it changes modes
        if self.completion.open && is_escape {
            return false;
        }
        // insert mode: only Esc is vim's; everything else keeps the normal
        // editor path (history, completions)
        if pane.editor.vim_mode() == Some(VimMode::Insert) && !is_escape {
            return false;
        }
        let key = match &event.logical_key {
            Key::Named(NamedKey::Escape) => VimKey::Escape,
            Key::Named(NamedKey::Enter) if !mods.shift_key() => VimKey::Enter,
            Key::Named(NamedKey::Backspace) => VimKey::Backspace,
            Key::Named(NamedKey::ArrowLeft) => VimKey::Left,
            Key::Named(NamedKey::ArrowRight) => VimKey::Right,
            Key::Named(NamedKey::ArrowUp) => VimKey::Up,
            Key::Named(NamedKey::ArrowDown) => VimKey::Down,
            Key::Named(NamedKey::Space) => VimKey::Char(' '),
            Key::Character(s) => {
                let mut cs = s.chars();
                match (cs.next(), cs.next()) {
                    (Some(c), None) => VimKey::Char(c),
                    _ => return false,
                }
            }
            _ => return false,
        };
        match pane.editor.vim_key(key) {
            VimOutcome::PassThrough => false,
            VimOutcome::Consumed => {
                pane.clear_selection();
                pane.selected_block = None;
                self.update_completions();
                self.blink_epoch = Instant::now();
                true
            }
            VimOutcome::Submit => {
                self.handle_editor_key(EditorKey::Submit);
                true
            }
            VimOutcome::HistoryUp => {
                self.handle_editor_key(EditorKey::HistoryUp);
                true
            }
            VimOutcome::HistoryDown => {
                self.handle_editor_key(EditorKey::HistoryDown);
                true
            }
        }
    }

    /// Terminal bell (BEL): visual flash, audible sound, tab activity.
    fn on_bell(&mut self, pane: PaneId) {
        use shift_core::Bell;
        if !self.pane_in_view(pane) {
            // coalesce repeated bells from the same pane
            let recent = self.attention.last().is_some_and(|a| {
                a.pane == pane
                    && a.kind == AttentionKind::Bell
                    && a.at.elapsed().is_ok_and(|d| d < Duration::from_secs(10))
            });
            if !recent {
                let job = self
                    .find_tab_of_pane(pane)
                    .and_then(|(_, t)| t.pane(pane))
                    .and_then(|p| p.session.foreground_job_name());
                // inbox only: the bell setting decides flash and sound
                self.raise(
                    Attention::new(
                        pane,
                        AttentionKind::Bell,
                        "Bell".into(),
                        job.unwrap_or_default(),
                    ),
                    None,
                );
            }
        }
        let bell = self.config.bell;
        if matches!(bell, Bell::Visual | Bell::Both) {
            self.bell_flash = Some(Instant::now());
        }
        if matches!(bell, Bell::Audible | Bell::Both) {
            crate::notify::beep();
        }
        let active = self.active;
        if let Some((idx, tab)) = self.find_tab_of_pane(pane)
            && idx != active
        {
            tab.activity = true;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Command corrections (local, no AI): after a failed command whose
    /// command word is unknown (or a git subcommand typo), offer the closest
    /// known command in the empty editor.
    fn check_correction(&mut self, pane: PaneId) {
        let enabled = self.config.command_corrections;
        let env = self.shell_env.clone();
        let Some((_, tab)) = self.find_tab_of_pane(pane) else {
            return;
        };
        let Some(p) = tab.pane_mut(pane) else { return };
        let newest = p
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|b| matches!(b.state, BlockState::Done { .. }))
            .map(|b| (b.id, b.command.clone(), b.exit_code()));
        let Some((id, cmd, exit)) = newest else {
            return;
        };
        if p.correction_checked == Some(id) {
            return;
        }
        p.correction_checked = Some(id);
        p.correction = None;
        if !enabled || exit == Some(0) || exit.is_none() || cmd.contains('\n') {
            return;
        }
        if let Some(env) = env
            && let Some(fix) = env.suggest_correction(&cmd)
            && fix != cmd
        {
            p.correction = Some(fix);
        }
    }

    /// OSC 9 / 777 / 99 notification. Its wording says what it means
    /// (Warp's Complete / Request / Error): "needs your permission" and
    /// "approval requested" wait on you, "turn complete" from an agent is a
    /// finished task.
    fn on_program_notification(&mut self, pane: PaneId, title: String, body: String) {
        use shift_core::notifications::{Event, Meaning, classify};
        let agent = self
            .find_tab_of_pane(pane)
            .and_then(|(_, t)| t.pane(pane))
            .and_then(Self::pane_agent);
        let cfg = &self.config.notifications;
        let attention = cfg.needs_attention.then_some(Event::Attention);
        let (kind, event, phrase) = match classify(&format!("{title} {body}")) {
            Meaning::Request => (AttentionKind::Request, attention, "Waiting for your input."),
            Meaning::Error => (
                AttentionKind::Failed,
                cfg.needs_attention.then_some(Event::Failed),
                "Error.",
            ),
            Meaning::Complete if agent.is_some() => (
                AttentionKind::Finished,
                cfg.agent_task_completed.then_some(Event::Finished),
                "Task completed.",
            ),
            Meaning::Complete | Meaning::Info => (AttentionKind::Notify, attention, ""),
        };
        let title = if title.trim().is_empty() {
            "Notification".to_string()
        } else {
            title
        };
        let detail = if body.trim().is_empty() && agent.is_some() {
            phrase.to_string()
        } else {
            body
        };
        let mut a = Attention::new(pane, kind, title, detail);
        a.agent = agent;
        self.raise(a, event);
    }

    /// Run every due script step (see script.rs).
    fn drive_script(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else {
            return;
        };
        loop {
            let now = Instant::now();
            let Some(step) = self.script.as_mut().and_then(|s| s.take_due(now)) else {
                break;
            };
            if matches!(
                step,
                crate::script::Step::WaitPrompt | crate::script::Step::WaitFile(..)
            ) {
                log::debug!("script: {step:?}");
            } else {
                log::info!("script: {step:?}");
            }
            self.run_step(step, event_loop, &window);
            self.sync_focus();
            window.request_redraw();
        }
    }

    fn run_step(
        &mut self,
        step: crate::script::Step,
        event_loop: &ActiveEventLoop,
        window: &Window,
    ) {
        use crate::script::{Step, char_press, key_press};
        let press = |app: &mut App, chord: &str| {
            let Some((press, mods)) = key_press(chord) else {
                log::warn!("script: bad chord {chord:?}");
                return;
            };
            let saved = app.modifiers;
            app.modifiers = mods;
            app.on_key(&press, event_loop, window);
            app.modifiers = saved;
        };
        match step {
            Step::Wait(_) => {}
            Step::Key(chord) => press(self, &chord),
            Step::KeyUp(chord) => {
                if let Some((mut p, mods)) = key_press(&chord) {
                    p.released = true;
                    p.text = None;
                    let saved = self.modifiers;
                    self.modifiers = mods;
                    self.on_key_release(&p);
                    self.modifiers = saved;
                }
            }
            Step::Type(text) => {
                for ch in text.chars() {
                    if ch == '\n' {
                        press(self, "enter");
                        continue;
                    }
                    let (p, mods) = char_press(ch);
                    let saved = self.modifiers;
                    self.modifiers = mods;
                    self.on_key(&p, event_loop, window);
                    self.modifiers = saved;
                }
            }
            Step::Shot(path) => self.capture = Some(path),
            Step::Dump(path) => {
                let mut text = self.dump_state();
                let snapshot = self.a11y_snapshot(window.scale_factor());
                text.push_str("── accessibility ──\n");
                text.push_str(&crate::a11y::describe(
                    &crate::a11y::build(&snapshot).update,
                ));
                if let Err(e) = std::fs::write(&path, text) {
                    log::warn!("script: dump {}: {e}", path.display());
                }
            }
            Step::Resize(w, h) => {
                let _ = window.request_inner_size(LogicalSize::new(w as f32, h as f32));
            }
            Step::Move(x, y) => self.on_cursor_moved((x, y), window),
            Step::Click(x, y) => {
                self.on_cursor_moved((x, y), window);
                self.on_left(ElementState::Pressed, event_loop, window);
                self.on_left(ElementState::Released, event_loop, window);
            }
            Step::DoubleClick(x, y) => {
                self.on_cursor_moved((x, y), window);
                for _ in 0..2 {
                    self.on_left(ElementState::Pressed, event_loop, window);
                    self.on_left(ElementState::Released, event_loop, window);
                }
            }
            Step::RightClick(x, y) => {
                self.on_cursor_moved((x, y), window);
                self.on_right_press(window);
                self.on_other_button(crate::mouse::RIGHT, false, window);
            }
            Step::MiddleClick(x, y) => {
                self.on_cursor_moved((x, y), window);
                self.on_other_button(crate::mouse::MIDDLE, true, window);
                self.on_other_button(crate::mouse::MIDDLE, false, window);
            }
            Step::Drag(x1, y1, x2, y2) => {
                self.on_cursor_moved((x1, y1), window);
                self.on_left(ElementState::Pressed, event_loop, window);
                self.on_cursor_moved(((x1 + x2) / 2.0, (y1 + y2) / 2.0), window);
                self.on_cursor_moved((x2, y2), window);
                self.on_left(ElementState::Released, event_loop, window);
            }
            Step::Press(x, y) => {
                self.on_cursor_moved((x, y), window);
                self.on_left(ElementState::Pressed, event_loop, window);
            }
            Step::Release(x, y) => {
                self.on_cursor_moved((x, y), window);
                self.on_left(ElementState::Released, event_loop, window);
            }
            Step::Scroll(lines, cols) => {
                // `scroll N` moves the terminal N lines whatever
                // wheel_scroll_lines is; sideways notches stay 3 columns
                let notch = self.config.wheel_lines();
                self.on_wheel(
                    MouseScrollDelta::LineDelta(cols / 3.0, lines / notch),
                    window,
                );
            }
            Step::Sh(cmd) => {
                let status = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd)
                    .status();
                log::info!("script: sh {cmd:?} -> {status:?}");
            }
            Step::Focus(f) => {
                self.focused = f;
                self.sync_focus();
            }
            Step::Ime(text) => self.commit_text(&text, event_loop, window),
            Step::Preedit(text) => self.preedit = text,
            Step::Drop(path) => self.drop_file(&path),
            Step::Paste(text) => self.paste_text(&text),
            Step::Mods(spec) => {
                let mut m = ModifiersState::empty();
                for part in spec.split('+').map(str::trim) {
                    match part {
                        "shift" => m |= ModifiersState::SHIFT,
                        "ctrl" => m |= ModifiersState::CONTROL,
                        "alt" | "opt" => m |= ModifiersState::ALT,
                        "cmd" => m |= ModifiersState::SUPER,
                        _ => {}
                    }
                }
                self.modifiers = m;
            }
            #[cfg(target_os = "macos")]
            Step::Menu(id) => self.run_menu_command(&id, event_loop),
            #[cfg(not(target_os = "macos"))]
            Step::Menu(id) => log::warn!("script: no menu bar on this platform ({id})"),
            Step::WaitFile(path, ms) => {
                if !path.exists()
                    && let Some(s) = self.script.as_mut()
                {
                    let _ = s.rearm_wait(Duration::from_millis(100), Duration::from_millis(ms));
                }
            }
            Step::WaitPrompt => {
                // re-arm until the active pane shows its first prompt (or 15s)
                if !self.editor_active
                    && let Some(s) = self.script.as_mut()
                {
                    let _ = s.rearm_wait(Duration::from_millis(50), Duration::from_secs(15));
                }
            }
            Step::Quit => {
                self.perform_close(CloseScope::Window, event_loop);
            }
        }
    }

    /// Textual snapshot of the app state for script assertions.
    /// Tabs and panes as the control API reports them.
    fn tabs_info(&self) -> Vec<crate::control::TabInfo> {
        use crate::control::{PaneInfo, TabInfo};
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| TabInfo {
                index: i,
                title: t.display_title(),
                active: i == self.active,
                panes: t
                    .panes
                    .iter()
                    .map(|p| {
                        let last = p
                            .session
                            .blocks
                            .lock()
                            .unwrap()
                            .iter()
                            .rev()
                            .find(|b| !b.command.is_empty())
                            .map(|b| (b.command.clone(), b.exit_code()));
                        PaneInfo {
                            id: p.session.id,
                            active: p.session.id == t.active_pane,
                            cwd: p.session.cwd.lock().unwrap().clone(),
                            job: p.session.foreground_job_name(),
                            last_command: last.as_ref().map(|l| l.0.clone()),
                            last_exit: last.and_then(|l| l.1),
                            durable: p.session.service_id.is_some(),
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    /// Revoke the old listener and queued requests whenever the level changes.
    fn sync_control_server(&mut self) {
        let level = crate::control::Level::parse(&self.config.control_api);
        if self
            .control_server
            .as_ref()
            .map(|s| s.level)
            .unwrap_or(crate::control::Level::Off)
            == level
        {
            return;
        }
        self.control_server = None;
        let proxy = self.proxy.clone();
        self.control_server = crate::control::serve(level, move |p| {
            let _ = proxy.send_event(UserEvent::Control(p));
        });
    }

    fn handle_control(&mut self, req: crate::control::Request) -> crate::control::Response {
        use crate::control::{Request, Response};
        if req.level() > crate::control::Level::parse(&self.config.control_api) {
            return Response::error("not allowed: control permissions changed");
        }
        let ok = |pane: Option<u64>| Response {
            ok: true,
            pane,
            ..Default::default()
        };
        match req {
            Request::List | Request::Status => Response {
                ok: true,
                pid: Some(std::process::id()),
                version: Some(env!("CARGO_PKG_VERSION").into()),
                tabs: self.tabs_info(),
                ..Default::default()
            },
            Request::Focus { tab, pane } => {
                self.settings.open = false;
                let target = match (tab, pane) {
                    (_, Some(p)) => self.find_tab_of_pane(p).map(|(i, _)| (i, Some(p))),
                    (Some(t), None) if t < self.tabs.len() => Some((t, None)),
                    (None, None) => Some((self.active, None)),
                    _ => None,
                };
                let Some((ti, pane)) = target else {
                    return Response::error("no such tab or pane");
                };
                self.active = ti;
                self.tabs[ti].activity = false;
                if let Some(p) = pane {
                    self.tabs[ti].active_pane = p;
                }
                if let Some(w) = self.window.as_ref().filter(|_| !self.headless) {
                    w.set_minimized(false);
                    w.focus_window();
                }
                ok(Some(self.tabs[ti].active_pane))
            }
            Request::Split { direction, cwd } => {
                let axis = match direction.as_str() {
                    "right" => Axis::Horizontal,
                    "down" => Axis::Vertical,
                    other => return Response::error(format!("direction {other:?}: right or down")),
                };
                let Some(pane) = self.tabs.get(self.active).map(|t| t.active_pane) else {
                    return Response::error("no tab");
                };
                // a missing or nonexistent --cwd starts where a new pane would
                let cwd = cwd
                    .map(std::path::PathBuf::from)
                    .filter(|p| p.is_dir())
                    .or_else(|| self.start_dir());
                let id = self.split_pane(pane, axis, cwd);
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.active_pane = id;
                }
                self.save_session();
                ok(Some(id))
            }
            Request::Open { cwd, command } => {
                // a missing or nonexistent --cwd starts where a new pane would
                let cwd = cwd
                    .map(std::path::PathBuf::from)
                    .filter(|p| p.is_dir())
                    .or_else(|| self.start_dir());
                self.spawn_tab_at(cwd);
                let id = self.tabs[self.active].active_pane;
                if let Some(c) = command {
                    self.pending_commands.insert(id, vec![c]);
                }
                ok(Some(id))
            }
            Request::SendText { pane, text } => {
                match self.find_tab_of_pane(pane).and_then(|(_, t)| t.pane(pane)) {
                    Some(p) => {
                        p.session.write(text.as_bytes());
                        ok(Some(pane))
                    }
                    None => Response::error("no such pane"),
                }
            }
            Request::PrepareRestart { token } => {
                // only for an update in flight from this install: its marker
                // (readable by the user only) holds the token
                use shift_core::update;
                if self.update.busy {
                    return Response::error("this window is updating Shift");
                }
                let valid = update::pending_path().is_some_and(|m| {
                    let written = std::fs::metadata(&m).and_then(|md| md.modified());
                    match (update::read_pending(&m), written) {
                        (Some(p), Ok(at)) => {
                            update::token_valid(&p, &token, at, std::time::SystemTime::now())
                        }
                        _ => false,
                    }
                });
                if !valid {
                    log::warn!("prepare-restart refused: no update in progress with that token");
                    return Response::error("not allowed: no update in progress");
                }
                let Some(file) = crate::updater::own_restart_file(self.persist_session) else {
                    return Response::error("no data directory");
                };
                if let Err(e) = shift_core::session_state::save_to(&file, &self.saved_session()) {
                    return Response::error(format!("{}: {e}", file.display()));
                }
                self.save_session();
                self.restart_requested = true;
                ok(None)
            }
        }
    }

    /// Describe the current UI for screen readers (see `a11y`).
    fn a11y_snapshot(&mut self, scale: f64) -> crate::a11y::Snapshot {
        use crate::a11y::{BlockNode, Control, EditorNode, Overlay, PaneNode, Snapshot, TabNode};
        use accesskit::Role;
        let hit = |pred: &dyn Fn(&ClickTarget) -> bool| {
            self.hits.iter().find(|h| pred(&h.target)).map(|h| h.rect)
        };
        let settings_shown = self.settings.open && self.settings.tab;
        let n = self.tabs.len();
        let tabs = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| TabNode {
                title: t.display_title(),
                selected: i == self.active && !settings_shown,
                rect: hit(&|c| *c == ClickTarget::TabSelect(i)),
            })
            .chain(self.settings.tab.then(|| TabNode {
                title: "Settings".into(),
                selected: settings_shown,
                rect: hit(&|c| *c == ClickTarget::TabSelect(n)),
            }))
            .collect();

        let mut panes = Vec::new();
        let mut finished = Vec::new();
        // the Settings tab covers the shell tab's panes
        if let Some(tab) = self.tabs.get(self.active).filter(|_| !settings_shown) {
            for p in &tab.panes {
                let id = p.session.id;
                let visible = tab.zoomed.is_none_or(|z| z == id);
                if !visible {
                    continue;
                }
                let active = id == tab.active_pane;
                let cwd = p.session.cwd.lock().unwrap().clone().unwrap_or_default();
                let running = p.session.foreground_job_name();
                let snapshot = p.session.blocks.lock().unwrap().snapshot();
                let mut blocks = Vec::new();
                for b in &snapshot {
                    let status = match b.state {
                        shift_core::BlockState::Prompt => continue,
                        shift_core::BlockState::Running { .. } => "running".to_string(),
                        shift_core::BlockState::Done { exit, duration } => format!(
                            "{}, {:.1}s",
                            match exit {
                                Some(0) => "succeeded".to_string(),
                                Some(c) => format!("failed with exit code {c}"),
                                None => "finished".to_string(),
                            },
                            duration.as_secs_f32()
                        ),
                    };
                    if matches!(b.state, shift_core::BlockState::Done { .. }) {
                        finished.push((id, b.id, b.command.clone(), status.clone()));
                    }
                    blocks.push(BlockNode {
                        id: b.id,
                        command: b.command.clone(),
                        output: terminal::block_output_text(&p.session, b.id).unwrap_or_default(),
                        selected: p.selected_block == Some(b.id),
                        rect: hit(&|c| *c == ClickTarget::Block(id, b.id)),
                        status,
                    });
                }
                let screen = p
                    .rows
                    .iter()
                    .filter_map(|r| match r.kind {
                        terminal::RowKind::Grid { abs, .. } => {
                            terminal::session_read_text(&p.session, abs)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let editor = (active && self.editor_active).then(|| EditorNode {
                    value: p.editor.buffer.clone(),
                    rect: hit(&|c| *c == ClickTarget::Editor(id)),
                });
                let mut label = format!("Pane {cwd}");
                if let Some(job) = running.filter(|_| !self.editor_active || !active) {
                    label.push_str(&format!(", running {job}"));
                }
                // title bar buttons, and the welcome rows of a fresh pane
                let welcome = welcome_entries(&self.keymap);
                let controls = self
                    .hits
                    .iter()
                    .filter_map(|h| {
                        let label = match h.target {
                            ClickTarget::PaneSplit(p) if p == id => "Split pane right".into(),
                            ClickTarget::PaneMenu(p) if p == id => "Pane menu".into(),
                            ClickTarget::PaneClose(p) if p == id => "Close pane".into(),
                            ClickTarget::Welcome(i) if active => {
                                let e = welcome.get(i as usize)?;
                                format!("{}, {}", e.title, e.keys.concat())
                            }
                            _ => return None,
                        };
                        Some(Control {
                            label,
                            role: Role::Button,
                            toggled: None,
                            selected: false,
                            rect: Some(h.rect),
                        })
                    })
                    .collect();
                panes.push(PaneNode {
                    id,
                    label,
                    active,
                    rect: hit(&|c| *c == ClickTarget::PaneFocus(id)),
                    blocks,
                    screen,
                    editor,
                    controls,
                });
            }
        }

        // announce the newest completion per pane once
        for (pane, block, command, status) in finished {
            let prev = self.a11y_done.insert(pane, block);
            if prev.is_some_and(|b| b < block) {
                self.a11y_announcement = Some(format!("{command} {status}"));
            }
        }

        let overlay = if let Some(c) = &self.confirm {
            let (title, description) = match &c.action {
                ConfirmAction::Close(_) => (
                    "Close with running commands?".to_string(),
                    format!("Running: {}", c.commands.join(", ")),
                ),
                ConfirmAction::Paste { .. } => (
                    "Paste and run?".to_string(),
                    format!(
                        "{} lines would run: {}",
                        c.commands.len(),
                        c.commands.join("; ")
                    ),
                ),
                ConfirmAction::InstallRemote(p) => (
                    "Install shell integration?".to_string(),
                    format!("On {}, writes {}", p.host, c.commands.join("; ")),
                ),
                ConfirmAction::Revert { .. } => (
                    "Revert changes?".to_string(),
                    format!("Discards unstaged changes: {}", c.commands.join("; ")),
                ),
                ConfirmAction::Update(plan) => (
                    format!("Update to Shift v{}?", plan.manifest.version),
                    format!("{} {}", plan.description, plan.note)
                        .trim()
                        .to_string(),
                ),
            };
            let label = match &c.action {
                ConfirmAction::Close(_) => "Close",
                ConfirmAction::Paste { .. } => "Paste",
                ConfirmAction::InstallRemote(_) => "Install",
                ConfirmAction::Revert { .. } => "Revert",
                ConfirmAction::Update(plan) if plan.open_page => "Open download page",
                ConfirmAction::Update(_) => "Update",
            };
            let cancel = match &c.action {
                ConfirmAction::Update(_) => "Later",
                _ => "Cancel",
            };
            let button = |l: &str, r: [f32; 4]| Control {
                label: l.into(),
                role: Role::Button,
                toggled: None,
                selected: false,
                rect: Some(r),
            };
            Some(Overlay::Alert {
                title,
                description,
                buttons: vec![
                    button(cancel, self.confirm_rects[0]),
                    button(label, self.confirm_rects[1]),
                ],
            })
        } else if let Some(ed) = &self.prompt_editor {
            use crate::views::prompt_editor::Target;
            let controls = self
                .prompt_editor_layout
                .hits
                .iter()
                .map(|(r, t)| {
                    let name = |i: &usize| ed.draft.get(*i).map_or("", |c| c.label());
                    let label = match t {
                        Target::Palette(c) => format!("Add {}", c.label()),
                        Target::Chip(i) => format!("{}, in the prompt", name(i)),
                        Target::Remove(i) => format!("Remove {}", name(i)),
                        Target::Restore => "Restore default".into(),
                        Target::Cancel => "Cancel".into(),
                        Target::Save => "Save changes".into(),
                    };
                    Control {
                        label,
                        role: Role::Button,
                        toggled: None,
                        selected: false,
                        rect: Some(*r),
                    }
                })
                .collect();
            Some(Overlay::Dialog {
                label: "Edit prompt".into(),
                input: None,
                status: Some(format!(
                    "Prompt: {}",
                    ed.draft
                        .iter()
                        .map(|c| c.label())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                controls,
                rect: Some(self.prompt_editor_layout.card),
            })
        } else if let Some(menu) = &self.chip_menu {
            let results = menu.results();
            let controls = self
                .chip_menu_rows
                .iter()
                .filter_map(|(r, i)| {
                    Some(Control {
                        label: results.get(*i)?.label.clone(),
                        role: Role::ListItem,
                        toggled: None,
                        selected: *i == menu.selected,
                        rect: Some(*r),
                    })
                })
                .collect();
            Some(Overlay::Dialog {
                label: menu.kind.title().into(),
                input: Some(("Search".into(), menu.query.clone())),
                status: Some(format!("{} results", results.len())),
                controls,
                rect: Some(self.chip_menu_card),
            })
        } else if self.whats_new.open {
            // the notes as one readable status string; the body is not focusable
            let mut notes = Vec::new();
            for r in &self.releases {
                notes.push(format!("Version {} {}", r.version, r.date));
                if !r.summary.is_empty() {
                    notes.push(r.summary.clone());
                }
                for s in &r.sections {
                    notes.push(s.title.clone());
                    for i in &s.items {
                        notes.push(format!("{}: {}", i.title, i.text));
                    }
                }
            }
            let button = |l: &str, r: [f32; 4]| Control {
                label: l.into(),
                role: Role::Button,
                toggled: None,
                selected: false,
                rect: Some(r),
            };
            Some(Overlay::Dialog {
                label: "What's new".into(),
                input: None,
                status: Some(notes.join(". ")),
                controls: vec![
                    button("Close", self.whats_new_hits.close),
                    button("Got it", self.whats_new_hits.done),
                ],
                rect: Some(self.whats_new_hits.card),
            })
        } else if self.notifications.open {
            let h = &self.notifications_hits;
            let entries = self.notification_entries();
            let shown = self.filtered_notifications();
            let highlighted = self
                .notifications
                .selected
                .and_then(|s| shown.get(s))
                .map(|e| e.id);
            let button = |l: String, r: [f32; 4], selected: bool| Control {
                label: l,
                role: Role::Button,
                toggled: None,
                selected,
                rect: Some(r),
            };
            let mut controls = vec![button("Close".into(), h.close, false)];
            if h.mark_read[2] > 0.0 {
                controls.push(button("Mark all as read".into(), h.mark_read, false));
            }
            for (r, f) in &h.chips {
                let n = entries.iter().filter(|e| f.matches(e)).count();
                controls.push(Control {
                    toggled: Some(*f == self.notifications.filter),
                    ..button(format!("{} ({n})", f.label()), *r, false)
                });
            }
            for (r, id) in &h.rows {
                if let Some(e) = entries.iter().find(|e| e.id == *id) {
                    let mut l = e
                        .agent
                        .map(|a| format!("{}: ", a.name()))
                        .unwrap_or_default();
                    l.push_str(&e.title);
                    if !e.detail.is_empty() {
                        l.push_str(&format!(", {}", e.detail));
                    }
                    if let Some(b) = &e.branch {
                        l.push_str(&format!(", on {b}"));
                    }
                    l.push_str(&format!(
                        ", {}",
                        crate::views::notifications::relative_time(e.age_secs)
                    ));
                    if e.unread {
                        l.push_str(", unread");
                    }
                    controls.push(button(l, *r, highlighted == Some(*id)));
                }
            }
            Some(Overlay::Dialog {
                label: "Notifications".into(),
                input: None,
                status: entries.is_empty().then(|| "No notifications".to_string()),
                controls,
                rect: Some(h.card),
            })
        } else if self.context_menu.open {
            Some(Overlay::Menu {
                items: self
                    .context_menu
                    .items
                    .iter()
                    .zip(&self.context_menu_rects)
                    .enumerate()
                    .filter(|(_, (item, _))| item.enabled)
                    .map(|(i, (item, r))| Control {
                        label: item.label.to_string(),
                        role: Role::MenuItem,
                        toggled: None,
                        selected: self.context_menu.selected == Some(i),
                        rect: Some(*r),
                    })
                    .collect(),
            })
        } else if self.palette.open {
            Some(Overlay::Dialog {
                label: "Command palette".into(),
                input: Some(("Search commands".into(), self.palette.query.clone())),
                status: Some(format!("{} results", self.palette_results.len())),
                // the mode chips, then the rows
                controls: self
                    .hits
                    .iter()
                    .filter_map(|h| match h.target {
                        ClickTarget::PaletteMode(i) => {
                            let (mode, label, prefix) =
                                crate::views::palette::MODES.get(i as usize)?;
                            Some(Control {
                                label: format!("{label} mode, prefix {prefix}"),
                                role: Role::RadioButton,
                                toggled: None,
                                selected: *mode == self.palette.mode,
                                rect: Some(h.rect),
                            })
                        }
                        _ => None,
                    })
                    .chain(self.palette_row_hits.iter().filter_map(|(r, i)| {
                        let m = self.palette_results.get(*i)?;
                        let mut label = m.item.title.clone();
                        if !m.item.shortcut.is_empty() {
                            label.push_str(&format!(", {}", m.item.shortcut.join(" ")));
                        }
                        Some(Control {
                            label,
                            role: Role::ListBoxOption,
                            toggled: None,
                            selected: *i == self.palette.selected,
                            rect: Some(*r),
                        })
                    }))
                    .collect(),
                rect: None,
            })
        } else if self.settings.open {
            // switches read their state from the config; an unset right
            // Option key shows the value it follows
            let mut shown = self.config.clone();
            shown.right_option_as_meta = Some(shown.right_option_meta());
            let values = toml::Value::try_from(&shown).ok();
            let current = crate::views::settings::Page::ALL
                .iter()
                .position(|p| *p == self.settings.page);
            let controls = self
                .hits
                .iter()
                .filter_map(|h| {
                    let (role, toggled, selected) = match &h.target {
                        ClickTarget::SettingsNav(i) => {
                            (Role::Tab, None, current == Some(*i as usize))
                        }
                        ClickTarget::SettingsToggle(id) => (
                            Role::Switch,
                            // dotted ids name nested tables (notifications.toasts)
                            id.split('.')
                                .try_fold(values.as_ref(), |v, key| Some(v?.get(key)))
                                .flatten()
                                .and_then(|v| v.as_bool()),
                            false,
                        ),
                        ClickTarget::SettingsSegment(..) | ClickTarget::SettingsTheme(_) => {
                            (Role::RadioButton, None, false)
                        }
                        // read-only rows (Shell path); unnamed Noop hits
                        // (the page body) find no name below and drop out
                        ClickTarget::SettingsNoop => (Role::Label, None, false),
                        ClickTarget::SettingsOption(i) => (
                            Role::ListBoxOption,
                            None,
                            self.settings
                                .dropdown
                                .as_ref()
                                .is_some_and(|d| d.current == Some(*i)),
                        ),
                        ClickTarget::SettingsClose
                        | ClickTarget::SettingsStep(..)
                        | ClickTarget::SettingsCycle(_)
                        | ClickTarget::SettingsSlider(_)
                        | ClickTarget::SettingsReset(_)
                        | ClickTarget::SettingsOpenConfig
                        | ClickTarget::SettingsCopyVersion
                        | ClickTarget::SettingsUpdate
                        | ClickTarget::SettingsEditPrompt
                        | ClickTarget::SettingsKeybind(_)
                        | ClickTarget::SettingsKeybindReset(_) => (Role::Button, None, false),
                        _ => return None,
                    };
                    let label = self
                        .settings_names
                        .iter()
                        .find(|(r, _)| *r == h.rect)
                        .map(|(_, n)| n.clone())?;
                    Some(Control {
                        label,
                        role,
                        toggled,
                        selected,
                        rect: Some(h.rect),
                    })
                })
                .collect();
            Some(Overlay::Dialog {
                label: format!("Settings, {}", self.settings.page.label()),
                input: Some(("Search settings".into(), self.settings.query.clone())),
                status: None,
                controls,
                rect: None,
            })
        } else if self.history.open {
            let hp = &self.history;
            let controls = self
                .hits
                .iter()
                .filter_map(|h| {
                    let (label, role, selected) = match h.target {
                        ClickTarget::HistoryRow(i) => {
                            let e = &hp.entries[*hp.results.get(i)?];
                            let status = match e.exit {
                                Some(0) => "succeeded",
                                Some(_) => "failed",
                                None => "status unknown",
                            };
                            (
                                format!("{}, {status}", e.command),
                                Role::ListBoxOption,
                                i == hp.selected,
                            )
                        }
                        ClickTarget::HistoryScope(s) => {
                            (s.label().to_string(), Role::RadioButton, s == hp.scope)
                        }
                        ClickTarget::HistoryClose => {
                            ("Close history".to_string(), Role::Button, false)
                        }
                        _ => return None,
                    };
                    Some(Control {
                        label,
                        role,
                        toggled: None,
                        selected,
                        rect: Some(h.rect),
                    })
                })
                .collect();
            Some(Overlay::Dialog {
                label: format!("Command history, {}", hp.scope.label()),
                input: Some(("Search history".into(), hp.query.clone())),
                status: Some(format!("{} of {}", hp.results.len(), hp.entries.len())),
                controls,
                rect: None,
            })
        } else if self.search.open {
            let (label, input, status) = if self.filter_editing {
                let query = self
                    .tabs
                    .get(self.active)
                    .and_then(|tab| tab.active())
                    .and_then(|pane| pane.filter.as_ref())
                    .map(|(_, query)| query.clone())
                    .unwrap_or_default();
                (
                    "Filter block output",
                    ("Filter output lines".into(), query),
                    "Enter keeps filter, Escape clears".into(),
                )
            } else {
                (
                    "Find",
                    ("Find in terminal".into(), self.search.query.clone()),
                    match (&self.search.error, self.search.position()) {
                        (Some(e), _) => e.clone(),
                        (None, None) => "No matches".into(),
                        (None, Some((n, total))) => format!("Match {n} of {total}"),
                    },
                )
            };
            Some(Overlay::Dialog {
                label: label.into(),
                input: Some(input),
                status: Some(status),
                controls: Vec::new(),
                rect: None,
            })
        } else {
            None
        };

        let (w, h) = self.logical_size();
        let panels = self.a11y_panels();
        Snapshot {
            size: (w, h),
            scale,
            tabs,
            panes,
            overlay,
            announcement: self.a11y_announcement.clone(),
            panels,
        }
    }

    /// Explorer rows, review files/hunks and the viewer as a11y regions,
    /// built from the last frame's click targets.
    fn a11y_panels(&self) -> Vec<crate::a11y::PanelNode> {
        use crate::a11y::{Control, PanelNode};
        use accesskit::Role;
        let control = |label: String, role: Role, selected: bool, rect: [f32; 4]| Control {
            label,
            role,
            toggled: None,
            selected,
            rect: Some(rect),
        };
        let mut out = Vec::new();
        let (ex_rect, rv_rect) = self.panel_rects();
        if ex_rect[2] > 0.0 {
            let controls = self
                .explorer_hits
                .iter()
                .map(|(r, hit)| match hit {
                    ExplorerHit::Row(i) => {
                        let row = &self.explorer.rows[*i];
                        let kind = if !row.is_dir {
                            "file"
                        } else if self.explorer.expanded.contains(&row.path) {
                            "folder, expanded"
                        } else {
                            "folder, collapsed"
                        };
                        control(
                            format!("{}, {kind}", row.name),
                            Role::TreeItem,
                            self.explorer.selected.as_ref() == Some(&row.path),
                            *r,
                        )
                    }
                    ExplorerHit::Refresh => control("Refresh".into(), Role::Button, false, *r),
                    ExplorerHit::Collapse => {
                        control("Collapse all".into(), Role::Button, false, *r)
                    }
                })
                .collect();
            // unreadable or cut folders: their notices are read out too
            let notices: Vec<&str> = self.explorer.notices().collect();
            out.push(PanelNode {
                label: "File explorer".into(),
                rect: Some(ex_rect),
                text: (!notices.is_empty()).then(|| notices.join("\n")),
                controls,
                focused: self.explorer.focused,
            });
        }
        if rv_rect[2] > 0.0 {
            let hunk_label = |id: &Option<shift_core::review::HunkId>| {
                id.map(|h| {
                    format!(
                        " lines {}–{}",
                        h.new_start,
                        h.new_start + h.new_lines.saturating_sub(1)
                    )
                })
                .unwrap_or_default()
            };
            // hunk actions show on a hovered line; the tree lists them all
            let latent = self
                .review
                .latent
                .iter()
                .filter(|(_, l)| !self.review_hits.iter().any(|(_, h)| h == l));
            let controls = self
                .review_hits
                .iter()
                .chain(latent)
                .map(|(r, hit)| {
                    let label = match hit {
                        ReviewHit::Refresh => "Refresh".to_string(),
                        ReviewHit::ToggleSection(true) => "Staged changes".into(),
                        ReviewHit::ToggleSection(false) => "Unstaged changes".into(),
                        ReviewHit::ToggleFile(staged, p) => format!(
                            "{p}, {}, {}",
                            if *staged { "staged" } else { "unstaged" },
                            if self.review.expanded.contains(&(*staged, p.clone())) {
                                "expanded"
                            } else {
                                "collapsed"
                            }
                        ),
                        ReviewHit::Reveal(_, p, _) => format!("Show unmodified lines of {p}"),
                        ReviewHit::Open(p) => format!("Open {p}"),
                        ReviewHit::Stage(p, h) => format!("Stage {p}{}", hunk_label(h)),
                        ReviewHit::Unstage(p, h) => format!("Unstage {p}{}", hunk_label(h)),
                        ReviewHit::Revert(p, h) => format!("Revert {p}{}", hunk_label(h)),
                        ReviewHit::StageAll => "Stage all".into(),
                        ReviewHit::UnstageAll => "Unstage all".into(),
                        ReviewHit::Maximize if self.review.maximized => "Restore".into(),
                        ReviewHit::Maximize => "Maximize".into(),
                        ReviewHit::Copy(_, p, h) => format!("Copy {p}{}", hunk_label(&Some(*h))),
                        ReviewHit::Jump(_, p) => format!("Show {p}"),
                    };
                    control(label, Role::Button, false, *r)
                })
                .collect();
            out.push(PanelNode {
                label: "Code review".into(),
                rect: Some(rv_rect),
                text: None,
                controls,
                focused: false,
            });
        }
        if let Some(path) = &self.viewer.path {
            let mut text = self
                .viewer
                .error
                .clone()
                .unwrap_or_else(|| self.viewer.lines.join("\n"));
            if text.len() > 20_000 {
                let cut = (0..=20_000)
                    .rev()
                    .find(|&i| text.is_char_boundary(i))
                    .unwrap_or(0);
                text.truncate(cut);
                text.push('…');
            }
            let controls = self
                .viewer_hits
                .iter()
                .map(|(r, hit)| match hit {
                    ViewerHit::Close => control("Close viewer".into(), Role::Button, false, *r),
                    ViewerHit::Menu => control("Viewer menu".into(), Role::Button, false, *r),
                    ViewerHit::Mode(m) => control(
                        if *m { "Rendered" } else { "Raw" }.into(),
                        Role::RadioButton,
                        self.viewer.rendered == *m,
                        *r,
                    ),
                })
                .collect();
            out.push(PanelNode {
                label: format!("File viewer, {}", path.display()),
                rect: Some(self.viewer_rect),
                text: Some(text),
                controls,
                // it takes the keys while open
                focused: true,
            });
        }
        // the tab bar's "Update Shift" chip
        if let Some(version) = self.update_chip()
            && let Some(h) = self
                .hits
                .iter()
                .find(|h| h.target == ClickTarget::UpdateChip)
        {
            out.push(PanelNode {
                label: "Update available".into(),
                rect: Some(h.rect),
                text: None,
                controls: vec![control(
                    format!("Update Shift, v{version} available"),
                    Role::Button,
                    false,
                    h.rect,
                )],
                focused: false,
            });
        }
        out
    }

    /// Push the accessibility tree when a screen reader is attached and
    /// something changed.
    fn update_a11y(&mut self, window: &Window) {
        let Some(mut adapter) = self.a11y.take() else {
            return;
        };
        adapter.update_if_active(|| {
            let snapshot = self.a11y_snapshot(window.scale_factor());
            let built = crate::a11y::build(&snapshot);
            self.a11y_targets = built.targets;
            if built.fingerprint == self.a11y_fingerprint {
                // unchanged: keep focus, send no nodes
                return accesskit::TreeUpdate {
                    nodes: Vec::new(),
                    tree: None,
                    tree_id: accesskit::TreeId::ROOT,
                    focus: built.update.focus,
                };
            }
            self.a11y_fingerprint = built.fingerprint;
            built.update
        });
        self.a11y = Some(adapter);
    }

    fn on_access_event(
        &mut self,
        event: accesskit_winit::WindowEvent,
        event_loop: &ActiveEventLoop,
    ) {
        use accesskit_winit::WindowEvent as A;
        let Some(window) = self.window.clone() else {
            return;
        };
        match event {
            A::InitialTreeRequested => {
                log::info!("accessibility: screen reader attached");
                // the next update must carry the full tree
                self.a11y_fingerprint.clear();
                self.update_a11y(&window);
            }
            A::ActionRequested(req) => {
                if !crate::a11y::activates(req.action) {
                    return;
                }
                let Some(r) = self.a11y_targets.get(&req.target_node).copied() else {
                    return;
                };
                // act exactly like a pointer click on the node
                let saved = self.mouse;
                self.mouse = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
                self.on_left(ElementState::Pressed, event_loop, &window);
                self.on_left(ElementState::Released, event_loop, &window);
                self.mouse = saved;
                window.request_redraw();
            }
            A::AccessibilityDeactivated => self.a11y_fingerprint.clear(),
        }
    }

    fn dump_state(&mut self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "theme_bg={:?} control_api={}",
            self.theme.bg, self.config.control_api
        );
        let _ = writeln!(
            out,
            "tabs={} active={} editor_active={} any_running={} mouse=({:.0},{:.0}) mouse_cursor={:?}",
            self.tabs.len(),
            self.active,
            self.editor_active,
            self.any_running,
            self.mouse.0,
            self.mouse.1,
            self.cursor_icon
        );
        let _ = writeln!(
            out,
            "font_size={} line_height={} theme={:?} cursor={:?}",
            self.font_size, self.config.line_height, self.config.theme, self.config.cursor_style
        );
        let _ = writeln!(
            out,
            "palette={} mode={:?} query={:?} selected={} results={}",
            self.palette.open,
            self.palette.mode,
            self.palette.query,
            self.palette.selected,
            self.palette_results.len()
        );
        let _ = writeln!(
            out,
            "settings={} page={:?} search={} query={:?} matches={} total={} current={} menu={} completion={} ghost={:?} items={} tooltip={:?}",
            self.settings.open,
            self.settings.page,
            self.search.open,
            self.search.query,
            self.search.matches.len(),
            self.search.total,
            self.search.current,
            self.context_menu.open,
            self.completion.open,
            self.completion.ghost,
            self.completion.items.len(),
            self.tip_shown.as_ref().map(|t| format!(
                "{} {} {}",
                t.text,
                t.detail.as_deref().unwrap_or(""),
                t.body
                    .as_deref()
                    .map(|b| format!("| {b}"))
                    .unwrap_or_default()
            )
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "))
        );
        // every control with a tooltip: `text@x,y,w,h` (a script hovers one
        // by these, whatever the platform's layout)
        let tips: Vec<String> = self
            .tips
            .iter()
            .filter(|t| !t.text.is_empty())
            .map(|t| {
                let [x, y, w, h] = t.rect;
                format!("{}@{x:.0},{y:.0},{w:.0},{h:.0}", t.text)
            })
            .collect();
        let _ = writeln!(out, "tips={tips:?}");
        let _ = writeln!(
            out,
            "settings_tab={} settings_query={:?} settings_select={:?} completion_source={:?} history={} scope={:?} history_rows={} history_selected={:?}",
            self.settings.tab,
            self.settings.query,
            self.settings.dropdown.as_ref().map(|d| d.id),
            self.completion.filter,
            self.history.open,
            self.history.scope,
            self.history.results.len(),
            self.history.current().map(|e| e.command.as_str()),
        );
        // links/paths as-is; blocks as `block <label> <block id>`
        let hint_targets: Vec<String> = self
            .hints
            .iter()
            .flat_map(|h| &h.items)
            .map(|(label, _, t)| match t {
                HintTarget::Open(t) => t.clone(),
                HintTarget::Block(_, b) => format!("block {label} {b:?}"),
            })
            .collect();
        let _ = writeln!(
            out,
            "settings_focus={:?} config_error={:?} hints={hint_targets:?}",
            self.settings.focus, self.config_error
        );
        let _ = writeln!(
            out,
            "explorer={} focused={} root={:?} rows={} selected={:?} scroll={:.0}",
            self.explorer.open,
            self.explorer.focused,
            self.explorer.root,
            self.explorer.rows.len(),
            self.explorer
                .selected
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned()),
            self.explorer.scroll
        );
        let _ = writeln!(
            out,
            "review={} load={} staged={:?} unstaged={:?} expanded={} revealed={} notice={:?} scroll={:.0} shown={} hover_repaints={}",
            self.review.open,
            match &self.review.load {
                crate::views::review_panel::Load::Idle => "idle",
                crate::views::review_panel::Load::Loading => "loading",
                crate::views::review_panel::Load::NotARepo => "not-a-repo",
                crate::views::review_panel::Load::Failed(_) => "failed",
                crate::views::review_panel::Load::Ready(_) => "ready",
            },
            self.review.model().map(|m| m
                .staged
                .iter()
                .map(|f| format!("{}{}", f.status, f.path))
                .collect::<Vec<_>>()),
            self.review.model().map(|m| m
                .unstaged
                .iter()
                .map(|f| format!("{}{}", f.status, f.path))
                .collect::<Vec<_>>()),
            self.review.expanded.len(),
            self.review.revealed.values().sum::<u32>(),
            self.review.notice,
            self.review.scroll,
            self.panel_widths(self.logical_size().0).1 > 0.0,
            self.hover_repaints
        );
        // maximized, sideways scroll per file, the hunk toolbar's buttons
        // and the file list's rows (kind@x,y,w,h)
        let mut scroll_x: Vec<_> = self
            .review
            .scroll_x
            .iter()
            .filter(|(_, s)| **s > 0.0)
            .map(|((_, p), s)| format!("{p}:{s:.0}"))
            .collect();
        scroll_x.sort();
        let review_hits: Vec<String> = self
            .review_hits
            .iter()
            .filter_map(|(r, hit)| {
                let kind = match hit {
                    ReviewHit::Stage(_, Some(_)) => "stage-hunk",
                    ReviewHit::Unstage(_, Some(_)) => "unstage-hunk",
                    ReviewHit::Revert(_, Some(_)) => "revert-hunk",
                    ReviewHit::Copy(..) => "copy-hunk",
                    ReviewHit::Jump(..) => "jump",
                    ReviewHit::Maximize => "maximize",
                    _ => return None,
                };
                Some(format!(
                    "{kind}@{:.0},{:.0},{:.0},{:.0}",
                    r[0], r[1], r[2], r[3]
                ))
            })
            .collect();
        let _ = writeln!(
            out,
            "prompt_chips={:?} prompt_editor={:?} prompt_editor_hits={:?}",
            self.config.prompt_chips,
            self.prompt_editor
                .as_ref()
                .map(|e| (&e.draft, e.drag.as_ref().map(|d| (d.chip, d.live)))),
            self.prompt_editor_layout
                .hits
                .iter()
                .map(|(r, t)| format!("{t:?}@{:.0},{:.0},{:.0},{:.0}", r[0], r[1], r[2], r[3]))
                .collect::<Vec<_>>(),
        );
        // the active pane's prompt chips as drawn (their texts)
        if let Some(pane) = self.tabs.get(self.active).and_then(|t| t.active()) {
            let cwd = pane
                .session
                .cwd
                .lock()
                .unwrap()
                .as_deref()
                .map(editor::shorten_home)
                .unwrap_or_else(|| "~".into());
            let chips: Vec<String> = editor::chip_views(&self.config, &self.theme, pane, &cwd)
                .iter()
                .map(|v| {
                    let t: Vec<&str> = v.texts.iter().map(|(t, _)| t.as_str()).collect();
                    format!("{:?}:{}", v.chip, t.join(" "))
                })
                .collect();
            let _ = writeln!(out, "prompt_chips_shown={chips:?}");
        }
        // the prompt chips' picker: kind, query, results, selection
        let _ = writeln!(
            out,
            "chip_menu={:?} chip_menu_rows={:?}",
            self.chip_menu.as_ref().map(|m| (
                m.kind,
                m.query.clone(),
                m.results().into_iter().map(|i| i.label).collect::<Vec<_>>(),
                m.selected
            )),
            self.chip_menu_rows
                .iter()
                .map(|(r, i)| format!("{i}@{:.0},{:.0},{:.0},{:.0}", r[0], r[1], r[2], r[3]))
                .collect::<Vec<_>>()
        );
        if let Some(m) = &self.chip_menu {
            let _ = writeln!(out, "chip_menu_scroll={:.1}", m.scroll);
        }
        let _ = writeln!(
            out,
            "review_maximized={} scroll_x={scroll_x:?} latent={} review_hits={review_hits:?}",
            self.review_maximized(),
            self.review.latent.len(),
        );
        let _ = writeln!(
            out,
            "viewer={:?} lines={} error={:?} scroll={:.0} find={:?}",
            self.viewer.path,
            self.viewer.lines.len(),
            self.viewer.error,
            self.viewer.scroll,
            self.viewer.find.as_ref().map(|f| f.counter())
        );
        let _ = writeln!(
            out,
            "whats_new={} scroll={:.0}/{:.0}",
            self.whats_new.open,
            self.whats_new.scroll,
            (self.whats_new.content_h - self.whats_new.view_h).max(0.0)
        );
        let _ = writeln!(
            out,
            "note_toasts={:?} notifications={} filter={:?} selected={:?} rows={} unread={} scroll={:.0}/{:.0}",
            self.note_toasts
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            self.notifications.open,
            self.notifications.filter,
            self.notifications.selected,
            self.notifications_hits.rows.len(),
            self.unread_attention(),
            self.notifications.scroll,
            (self.notifications.content_h - self.notifications.view_h).max(0.0)
        );
        let _ = writeln!(
            out,
            "update={} chip={:?} dismissed={:?} busy={} version={}",
            match &self.update.status {
                shift_core::update::Status::Idle => "idle".to_string(),
                shift_core::update::Status::Checking => "checking".into(),
                shift_core::update::Status::UpToDate => "up-to-date".into(),
                shift_core::update::Status::Available(m) => format!("available:{}", m.version),
                shift_core::update::Status::Failed(e) => format!("failed:{e:?}"),
            },
            self.update_chip(),
            self.update.dismissed,
            self.update.busy,
            env!("CARGO_PKG_VERSION"),
        );
        let _ = writeln!(
            out,
            "confirm={:?} rename={:?} filter_editing={} toast={:?} shell_env={}",
            self.confirm
                .as_ref()
                .map(|c| (&c.action, c.commands.clone())),
            self.rename,
            self.filter_editing,
            self.toast.as_ref().map(|(m, _)| m.clone()),
            self.shell_env.is_some()
        );
        for a in &self.attention {
            let _ = writeln!(
                out,
                "attention id={} pane={} kind={:?} agent={:?} read={} branch={:?} title={:?} detail={:?}",
                a.id,
                a.pane,
                a.kind,
                a.agent.map(|g| g.name()),
                a.read,
                a.branch,
                a.title,
                a.detail
            );
        }
        if self.context_menu.open {
            let labels: Vec<&str> = self.context_menu.items.iter().map(|i| i.label).collect();
            let _ = writeln!(
                out,
                "menu_items={labels:?} menu_selected={:?}",
                self.context_menu.selected
            );
        }
        for (i, tab) in self.tabs.iter().enumerate() {
            let _ = writeln!(
                out,
                "tab {i} title={:?} panes={} active_pane={} zoomed={:?} activity={} color={:?} sync={} custom_title={:?} running={}",
                tab.display_title(),
                tab.panes.len(),
                tab.active_pane,
                tab.zoomed,
                tab.activity,
                tab.color,
                tab.sync_inputs,
                tab.custom_title,
                tab.panes.iter().any(PaneState::running)
            );
            let _ = writeln!(out, "  layout={:?}", tab.root);
            for p in &tab.panes {
                let cwd = p.session.cwd.lock().unwrap().clone();
                let _ = writeln!(
                    out,
                    "  pane {} cwd={:?} editor={:?} cursor={} view_offset={} selected_block={:?} alt={} selection={} last_exit={:?} vim={:?} git={:?}",
                    p.session.id,
                    cwd,
                    p.editor.buffer,
                    p.editor.cursor,
                    p.view_offset,
                    p.selected_block,
                    p.alt_screen,
                    p.has_selection,
                    p.last_exit,
                    p.editor.vim_mode(),
                    p.git.as_ref().map(|g| g.branch.clone())
                );
                let _ = writeln!(
                    out,
                    "    editor_selection={:?} filter={:?} correction={:?} sticky={:?} reverse_screen={}",
                    p.editor.selected_text(),
                    p.filter,
                    p.correction,
                    p.sticky_hit.map(|(_, b)| b),
                    p.session.term.lock().reverse_screen()
                );
                // screen geometry for scripted mouse steps: cell size and
                // the y of every grid row (`abs:y`) with the x of its text
                if let Some(g) = p.last_grid {
                    let rows: Vec<String> = p
                        .rows
                        .iter()
                        .filter_map(|r| match r.kind {
                            terminal::RowKind::Grid { abs, content_x } => {
                                Some(format!("{abs}:{:.1}@{content_x:.1}", r.y))
                            }
                            _ => None,
                        })
                        .collect();
                    let _ = writeln!(
                        out,
                        "    geometry cell_w={:.2} line_h={:.2} cols={} rows=[{}]",
                        g.cell_w,
                        g.line_h,
                        g.cols,
                        rows.join(" ")
                    );
                }
                let blocks = p.session.blocks.lock().unwrap().snapshot();
                for b in blocks {
                    let _ = writeln!(
                        out,
                        "    block {} state={} duration_ms={:?} cmd={:?} exit={:?} collapsed={} bookmarked={} prompt_start={} output_start={:?} end={:?} cwd={:?}",
                        b.id,
                        match b.state {
                            shift_core::BlockState::Prompt => "prompt",
                            shift_core::BlockState::Running { .. } => "running",
                            shift_core::BlockState::Done { .. } => "done",
                        },
                        match b.state {
                            shift_core::BlockState::Done { duration, .. } => {
                                Some(duration.as_millis())
                            }
                            _ => None,
                        },
                        b.command,
                        b.exit_code(),
                        b.collapsed,
                        b.bookmarked,
                        b.prompt_start,
                        b.output_start,
                        b.end,
                        b.cwd
                    );
                }
                // visible rows (what the user sees), for text assertions
                let text = crate::views::terminal::visible_text(&p.session);
                for line in text.lines() {
                    let _ = writeln!(out, "    | {line}");
                }
            }
        }
        let clip = if self.headless {
            self.headless_clipboard.clone()
        } else {
            self.clipboard
                .as_mut()
                .and_then(|c| c.get_text().ok())
                .unwrap_or_default()
        };
        let _ = writeln!(out, "clipboard={clip:?}");
        let visible = self.window.as_ref().and_then(|w| w.is_visible());
        let _ = writeln!(out, "headless={} window_visible={visible:?}", self.headless);
        out
    }
}

/// Updates: the check, the chip and dialog, the install and the restart.
impl App {
    /// Checks run outside scripted runs, and in one only under
    /// `SHIFT_UPDATE_URL` (tests never reach the network).
    fn update_checks_allowed(&self) -> bool {
        self.script.is_none() || shift_core::update::url_overridden()
    }

    /// The version the tab-bar chip offers: an available update that was
    /// not dismissed, while nothing is being installed.
    fn update_chip(&self) -> Option<String> {
        match &self.update.status {
            shift_core::update::Status::Available(m)
                if !self.update.busy && self.update.dismissed.as_deref() != Some(&m.version) =>
            {
                Some(m.version.clone())
            }
            _ => None,
        }
    }

    /// Settings › About's line follows the status.
    fn sync_update_view(&mut self) {
        use crate::views::settings::UpdateLine;
        use shift_core::update::Status;
        self.settings.update = match &self.update.status {
            Status::Idle => UpdateLine::None,
            Status::Checking => UpdateLine::Checking,
            Status::UpToDate => UpdateLine::UpToDate,
            Status::Available(m) => UpdateLine::Available(m.version.clone()),
            Status::Failed(_) => UpdateLine::Failed,
        };
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Start the periodic check when `check_for_updates` is on (and stop it
    /// when it is turned off). The cached answer shows at once.
    fn sync_update_checks(&mut self) {
        use shift_core::update;
        use std::sync::atomic::{AtomicBool, Ordering};
        let want = self.config.check_for_updates && self.update_checks_allowed();
        if !want {
            if let Some(flag) = self.update.auto.take() {
                // turned off: the chip and About's line go too
                flag.store(false, Ordering::Release);
                if !self.update.busy {
                    self.update.status = update::Status::Idle;
                    self.sync_update_view();
                }
            }
            return;
        }
        if self.update.auto.is_some() {
            return;
        }
        let overridden = update::url_overridden();
        let cache = update::load_cache();
        self.update.dismissed = cache.dismissed.clone();
        // a test manifest is always fetched fresh
        if !overridden
            && let Some(m) = cache.latest
            && matches!(self.update.status, update::Status::Idle)
        {
            self.update.status = update::evaluate(m, shift_core::changelog::VERSION);
        }
        let flag = Arc::new(AtomicBool::new(true));
        self.update.auto = Some(flag.clone());
        let proxy = self.proxy.clone();
        let started = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                // sleep in short steps so turning checks off ends the thread
                let nap = |secs: u64| {
                    let end = Instant::now() + Duration::from_secs(secs);
                    while flag.load(Ordering::Acquire) && Instant::now() < end {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                };
                let mut first = true;
                let mut seen = 0;
                while flag.load(Ordering::Acquire) {
                    let cache = update::load_cache();
                    let wait = if first && overridden {
                        0
                    } else {
                        cache.due_in(update::now())
                    };
                    first = false;
                    if wait > 0 {
                        // another window checked: take its answer
                        if cache.checked_at != seen
                            && let Some(m) = cache.latest.clone()
                        {
                            seen = cache.checked_at;
                            if proxy
                                .send_event(UserEvent::UpdateChecked(Ok(m), false))
                                .is_err()
                            {
                                return;
                            }
                        }
                        nap(wait);
                        continue;
                    }
                    let result = update::fetch(&update::manifest_url());
                    if let Ok(f) = &result {
                        update::update_cache(|c| {
                            c.checked_at = update::now();
                            c.store(f);
                        });
                        seen = update::load_cache().checked_at;
                    }
                    let result = result.map(|f| f.manifest);
                    let failed = result.is_err();
                    if proxy
                        .send_event(UserEvent::UpdateChecked(result, false))
                        .is_err()
                    {
                        return;
                    }
                    // a failure retries within the hour; a success waits
                    // for the cache to fall due
                    nap(if failed { 3600 } else { 60 });
                }
            });
        if started.is_err() {
            self.update.auto = None;
        }
    }

    /// "Check for updates" (palette, About): one check now; its answer
    /// toasts.
    fn check_for_updates_now(&mut self) {
        use shift_core::update;
        if !self.update_checks_allowed() {
            self.toast("Update checks are off in scripted runs".into());
            return;
        }
        if matches!(self.update.status, update::Status::Checking) {
            return;
        }
        self.update.status = update::Status::Checking;
        self.sync_update_view();
        let proxy = self.proxy.clone();
        std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let result = update::fetch(&update::manifest_url());
                if let Ok(f) = &result {
                    update::update_cache(|c| {
                        c.checked_at = update::now();
                        c.store(f);
                    });
                }
                let result = result.map(|f| f.manifest);
                let _ = proxy.send_event(UserEvent::UpdateChecked(result, true));
            })
            .ok();
    }

    fn on_update_checked(
        &mut self,
        result: Result<shift_core::update::Manifest, String>,
        manual: bool,
    ) {
        use shift_core::update::{self, Status};
        let current = shift_core::changelog::VERSION;
        match result {
            Ok(m) => {
                let status = update::evaluate(m, current);
                if manual {
                    match &status {
                        Status::Available(m) => {
                            // asked for: the chip comes back even if dismissed
                            self.update.dismissed = None;
                            update::update_cache(|c| c.dismissed = None);
                            self.toast(format!("Shift v{} is available", m.version));
                        }
                        _ => self.toast(format!("Shift v{current} is up to date")),
                    }
                }
                self.update.status = status;
            }
            Err(e) => {
                log::info!("update check: {e}");
                if manual {
                    // curl's own wording stays in the log
                    self.toast(
                        if e.starts_with("malformed manifest")
                            || e == update::SIG_MISSING
                            || e == update::SIG_INVALID
                        {
                            "Couldn't check for updates: the release information is invalid".into()
                        } else if e == "curl is not installed" {
                            "Couldn't check for updates: curl is not installed".into()
                        } else {
                            "Couldn't check for updates. Check your connection and try again."
                                .into()
                        },
                    );
                }
                // a known update stays offered through a failed recheck
                if !matches!(self.update.status, Status::Available(_)) {
                    self.update.status = Status::Failed(e);
                }
            }
        }
        self.sync_update_view();
    }

    /// Chip, About's button, `update_shift`: the dialog for an available
    /// update (after finding the other windows), else a check.
    fn update_shift(&mut self) {
        if self.update.busy {
            self.toast("Shift is already updating".into());
            return;
        }
        if !matches!(self.update.status, shift_core::update::Status::Available(_)) {
            self.check_for_updates_now();
            return;
        }
        let proxy = self.proxy.clone();
        std::thread::Builder::new()
            .name("update-survey".into())
            .spawn(move || {
                let survey = crate::updater::survey(Duration::from_millis(1500));
                let _ = proxy.send_event(UserEvent::UpdateSurvey(survey));
            })
            .ok();
    }

    /// What updating means here: which shells come back, which windows.
    fn update_plan(
        &self,
        manifest: shift_core::update::Manifest,
        survey: crate::updater::Survey,
    ) -> UpdatePlan {
        use shift_core::update::{self, InstallTarget};
        let mut target = update::install_target();
        let key = update::platform_key();
        if !matches!(target, InstallTarget::Unsupported(_))
            && update::asset_for(&manifest, &key).is_none()
        {
            target = InstallTarget::Unsupported(format!("this release has no download for {key}"));
        }
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        let panes: Vec<&PaneState> = self.tabs.iter().flat_map(|t| &t.panes).collect();
        let tabs = self.tabs.len() + survey.tabs();
        let shells = panes.len() + survey.shells();
        let durable = panes
            .iter()
            .filter(|p| p.session.service_id.is_some())
            .count()
            + survey.durable_shells();
        let compatible = update::protocol_compatible(&manifest, shift_pty::ipc::PROTOCOL_VERSION);
        let tabs_text = plural(tabs, "tab");
        let (description, mut note, open_page) = match &target {
            InstallTarget::Unsupported(why) if why == update::MOVE_TO_APPLICATIONS => (
                format!("{why}."),
                "It runs from the disk image or a temporary copy macOS made. Drag Shift into Applications, open it from there, then update.".to_string(),
                true,
            ),
            InstallTarget::Unsupported(why) => (
                format!("This copy of Shift can't update itself: {why}."),
                "Download the new version from the release page.".to_string(),
                true,
            ),
            _ if durable > 0 && !compatible => (
                format!("Shift restarts and reopens your {tabs_text}."),
                "This update can't reattach running shells; they keep running in the background but won't reopen.".to_string(),
                false,
            ),
            _ if durable > 0 && durable == shells => (
                format!(
                    "Shift restarts. Your {tabs_text} and {} keep running and come back where they were.",
                    plural(shells, "shell")
                ),
                String::new(),
                false,
            ),
            _ if durable > 0 => (
                format!(
                    "Shift restarts and reopens your {tabs_text}. {durable} of {} keep running; the others end.",
                    plural(shells, "shell")
                ),
                String::new(),
                false,
            ),
            _ => (
                format!("Shift restarts and reopens your {tabs_text}."),
                "Open shells will end when Shift restarts. Turn on “Keep shells running when Shift quits” in Settings › General to keep them.".to_string(),
                false,
            ),
        };
        if survey.unreachable > 0 && !open_page {
            let n = survey.unreachable;
            let more = if n == 1 {
                "1 window can't be reopened (its control socket is off); it stays open on the current version.".to_string()
            } else {
                format!(
                    "{n} windows can't be reopened (their control socket is off); they stay open on the current version."
                )
            };
            note = if note.is_empty() {
                more
            } else {
                format!("{note} {more}")
            };
        }
        UpdatePlan {
            manifest,
            target,
            windows: survey
                .windows
                .into_iter()
                .map(|(pid, path, _)| (pid, path))
                .collect(),
            description,
            note,
            open_page,
        }
    }

    fn open_update_dialog(&mut self, survey: crate::updater::Survey) {
        let shift_core::update::Status::Available(m) = self.update.status.clone() else {
            return;
        };
        if self.update.busy || self.confirm.is_some() {
            return;
        }
        let plan = self.update_plan(m, survey);
        self.palette.open = false;
        self.context_menu.close();
        self.confirm = Some(ConfirmClose {
            opened: Instant::now(),
            action: ConfirmAction::Update(Box::new(plan)),
            commands: Vec::new(),
        });
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Cancel / Esc on the confirm modal; "Later" on the update dialog
    /// dismisses that version (no chip until a newer one).
    fn cancel_confirm(&mut self) {
        if let Some(ConfirmClose {
            action: ConfirmAction::Update(plan),
            ..
        }) = self.confirm.take()
        {
            let version = plan.manifest.version.clone();
            self.update.dismissed = Some(version.clone());
            if self.update_checks_allowed() {
                shift_core::update::update_cache(|c| c.dismissed = Some(version));
            }
        }
    }

    /// Update: take the install lock (one window at a time), then download,
    /// verify and install off the UI thread; Open download page: the
    /// release page in the browser.
    fn install_update(&mut self, plan: UpdatePlan) {
        use shift_core::update;
        if plan.open_page {
            let url = if plan.manifest.url.starts_with("https://") {
                plan.manifest.url.clone()
            } else {
                RELEASES_PAGE.to_string()
            };
            self.open_target_external(&url);
            return;
        }
        let Some(dir) = update::updates_dir() else {
            self.toast("Update failed: no data directory".into());
            return;
        };
        match update::try_lock(&dir) {
            Ok(lock) => self.update.lock = Some(lock),
            Err(update::LockError::Held(pid)) => {
                log::info!("update: install lock held by {pid}");
                self.toast("Another window is updating Shift".into());
                return;
            }
            Err(update::LockError::Io(e)) => {
                self.toast(format!("Update failed: {e}"));
                return;
            }
        }
        self.update.busy = true;
        self.update.quit_after = false;
        let version = plan.manifest.version.clone();
        self.toast(format!("Downloading Shift v{version}…"));
        let proxy = self.proxy.clone();
        let spawned = std::thread::Builder::new()
            .name("update-install".into())
            .spawn(move || {
                let result = (|| {
                    let asset = update::asset_for(&plan.manifest, &update::platform_key())
                        .ok_or("this release has no download for this computer")?;
                    // what an interrupted install left (the lock is held)
                    update::sweep(&dir, &plan.target);
                    // a fixed name: nothing from the manifest reaches a path
                    let file = update::download_path(&dir, asset);
                    update::download(asset, &file)?;
                    let installed = update::install(
                        &plan.target,
                        &file,
                        &version,
                        shift_core::changelog::VERSION,
                    );
                    let _ = std::fs::remove_file(&file);
                    installed
                })();
                let _ = proxy.send_event(UserEvent::UpdateInstalled(result, plan.windows));
            });
        if spawned.is_err() {
            self.update.busy = false;
            self.update.lock = None;
            self.toast("Could not start the update".into());
        }
    }

    /// The update stopped (failed, or undone before the restart): release
    /// the lock, say why, and quit now if a quit waited for it.
    fn abort_update(&mut self, message: String, event_loop: &ActiveEventLoop) {
        self.update.busy = false;
        self.update.lock = None;
        self.toast(message);
        self.sync_update_view();
        if std::mem::take(&mut self.update.quit_after) {
            self.perform_close(CloseScope::Window, event_loop);
        }
    }

    /// Installed: commit to the restart in an order where any failure
    /// leaves the old app in place and every window open — 1. the rollback
    /// marker with a fresh token, 2. this window's restart file, 3. the
    /// helper that starts the new version once this process is gone — and
    /// only then 4. ask the other windows (with the token) to save their
    /// layouts and quit.
    fn on_update_installed(
        &mut self,
        result: Result<shift_core::update::Pending, String>,
        windows: Vec<(u32, std::path::PathBuf)>,
        event_loop: &ActiveEventLoop,
    ) {
        use shift_core::update;
        let mut pending = match result {
            Ok(p) => p,
            Err(e) => return self.abort_update(format!("Update failed: {e}"), event_loop),
        };
        let to = pending.to.clone();
        // nothing will start the new version: the old one goes back
        let undo = |pending: &update::Pending, why: String| match update::restore_old(pending) {
            Ok(()) => format!("Couldn't restart into Shift v{to} ({why}); nothing was changed"),
            Err(e) => {
                log::warn!("update: restoring the previous version: {e}");
                format!("Couldn't restart into Shift v{to} ({why})")
            }
        };
        // 1. the marker (the helper's rollback signal, the others' token)
        let marker = update::pending_path();
        let written = update::new_token()
            .map_err(|e| format!("random token: {e}"))
            .and_then(|token| {
                pending.token = token;
                let m = marker.as_deref().ok_or("no data directory")?;
                update::write_pending(m, &pending).map_err(|e| format!("{}: {e}", m.display()))
            });
        if let Err(e) = written {
            let message = undo(&pending, e);
            return self.abort_update(message, event_loop);
        }
        let marker = marker.unwrap_or_default();
        if self.update.quit_after {
            // quit was asked for during the install: no restart. The new
            // version finishes the update (the marker) on its next launch.
            self.update.busy = false;
            self.update.quit_after = false;
            self.perform_close(CloseScope::Window, event_loop);
            return;
        }
        // 2. this window's layout
        let own = crate::updater::own_restart_file(self.persist_session)
            .ok_or_else(|| "no data directory".to_string())
            .and_then(|f| {
                shift_core::session_state::save_to(&f, &self.saved_session())
                    .map(|()| f.clone())
                    .map_err(|e| format!("{}: {e}", f.display()))
            });
        let own = match own {
            Ok(f) => f,
            Err(e) => {
                let _ = std::fs::remove_file(&marker);
                let message = undo(&pending, e);
                return self.abort_update(message, event_loop);
            }
        };
        // 3. the helper
        if let Err(e) = self.spawn_update_helper(&pending, &own) {
            let _ = std::fs::remove_file(&own);
            let _ = std::fs::remove_file(&marker);
            let message = undo(&pending, e);
            return self.abort_update(message, event_loop);
        }
        // 4. the other windows; then this one quits (UpdateReady)
        self.toast(format!("Restarting into Shift v{to}…"));
        let proxy = self.proxy.clone();
        let token = pending.token.clone();
        let asked = std::thread::Builder::new()
            .name("update-restart".into())
            .spawn(move || {
                let n = crate::updater::restart_others(&windows, &token, Duration::from_secs(5));
                log::info!("update: {n} other window(s) saved their layouts");
                let _ = proxy.send_event(UserEvent::UpdateReady(own));
            });
        if asked.is_err() {
            // the helper is waiting for this process: quit; the other
            // windows stay open on the current version
            log::warn!("update: could not ask the other windows to restart");
            event_loop.exit();
        }
    }

    /// Start the detached helper that launches the new version (restoring
    /// this window from `own`, reopening the others) once this process is
    /// gone, and rolls back if it dies before confirming.
    fn spawn_update_helper(
        &self,
        pending: &shift_core::update::Pending,
        own: &std::path::Path,
    ) -> Result<(), String> {
        use shift_core::update;
        let exe = if pending.new_path.extension().is_some_and(|e| e == "app") {
            pending.new_path.join("Contents/MacOS/shift-terminal")
        } else {
            pending.new_path.clone()
        };
        let script = std::env::var(crate::updater::RELAUNCH_SCRIPT_ENV).ok();
        let args = update::relaunch_args(&crate::updater::launch().args, script.as_deref());
        let env = vec![
            (crate::updater::OTHERS_ENV.to_string(), "1".to_string()),
            (crate::updater::RELAUNCH_ENV.to_string(), "1".to_string()),
            (
                crate::updater::RESTORE_ENV.to_string(),
                own.to_string_lossy().into_owned(),
            ),
        ];
        let (Some(marker), Some(failed), Some(dir)) = (
            update::pending_path(),
            update::failed_path(),
            update::updates_dir(),
        ) else {
            return Err("no data directory".into());
        };
        let _ = std::fs::create_dir_all(&dir);
        // the helper keeps it under 1 MB; it holds every relaunch's output
        let log = dir.join("relaunch.log");
        update::spawn_helper(pending, &exe, &marker, &failed, &log, &args, &env, &[])
            .map_err(|e| format!("helper: {e}"))?;
        log::info!("update: relaunching {} after exit", exe.display());
        Ok(())
    }

    /// The other windows saved their layouts and quit: save this window's
    /// layout once more (the shells may have moved meanwhile; the earlier
    /// file stays if this fails) and quit with durable shells running.
    fn relaunch_for_update(&mut self, own: std::path::PathBuf, event_loop: &ActiveEventLoop) {
        if let Err(e) = shift_core::session_state::save_to(&own, &self.saved_session()) {
            log::warn!("restart file {}: {e}", own.display());
        }
        self.save_session();
        event_loop.exit();
    }

    /// At start: what waits for the first frame (`confirm_start`), and how
    /// long after it — [`shift_core::update::CONFIRM_AFTER`] when this is the
    /// version an update just installed, so a crash early on still rolls
    /// back (the other windows reopen only then: a rollback reopens them on
    /// the old version instead).
    fn plan_start_confirmation(&mut self) {
        use shift_core::update;
        let fresh_install = self.handles_update_marker()
            && update::pending_path()
                .and_then(|m| update::read_pending(&m))
                .is_some_and(|p| update::same_version(&p.to, shift_core::changelog::VERSION));
        let wait = if fresh_install {
            update::CONFIRM_AFTER
        } else {
            Duration::ZERO
        };
        self.update.confirm_after = Some(wait);
        // a window that never presents a frame (hidden on another Space;
        // headless runs present none at all) still confirms, later
        let proxy = self.proxy.clone();
        let _ = std::thread::Builder::new()
            .name("update-confirm-fallback".into())
            .spawn(move || {
                std::thread::sleep(wait + Duration::from_secs(20));
                let _ = proxy.send_event(UserEvent::StartConfirmed);
            });
        // SHIFT_TEST_NO_FIRST_FRAME (tests): a headless run that waits for
        // the fallback like a window that never draws
        if self.headless && std::env::var_os("SHIFT_TEST_NO_FIRST_FRAME").is_none() {
            self.on_first_frame();
        }
    }

    /// The update marker is this process's business: it was started by the
    /// helper, or it is a plain launch of the main window (which finishes
    /// an update that quit instead of restarting).
    fn handles_update_marker(&self) -> bool {
        let launch = crate::updater::launch();
        launch.relaunched || self.plain_main_launch()
    }

    fn plain_main_launch(&self) -> bool {
        let launch = crate::updater::launch();
        // a scripted run persists (owns session.toml) only under
        // SHIFT_PERSIST_SESSION, so it counts as a plain launch then
        self.persist_session && !launch.secondary && !launch.relaunched && launch.restore.is_none()
    }

    /// The first frame is on screen: confirm the start after the planned
    /// wait.
    fn on_first_frame(&mut self) {
        let Some(wait) = self.update.confirm_after.take() else {
            return;
        };
        let proxy = self.proxy.clone();
        let spawned = std::thread::Builder::new()
            .name("update-confirm".into())
            .spawn(move || {
                std::thread::sleep(wait);
                let _ = proxy.send_event(UserEvent::StartConfirmed);
            });
        if spawned.is_err() {
            self.confirm_start();
        }
    }

    /// This process is up: finish an update that started it (delete the
    /// old copy and the marker, or report a rollback), drop the restart file
    /// it came from, and reopen the other windows — after an update, or,
    /// on a plain launch of the main window, any a crashed update left.
    fn confirm_start(&mut self) {
        use shift_core::update;
        if std::mem::replace(&mut self.update.start_confirmed, true) {
            return;
        }
        self.update.confirm_after = None;
        let launch = crate::updater::launch();
        let current = shift_core::changelog::VERSION;
        let mut marker_left = false;
        if self.handles_update_marker()
            && let Some(marker) = update::pending_path()
            && let Some(p) = update::read_pending(&marker)
        {
            let age = std::fs::metadata(&marker)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .unwrap_or_default();
            if update::same_version(&p.to, current) {
                let target = update::install_target();
                if update::pending_matches_target(&p, &target) {
                    if p.old_path.is_dir() {
                        let _ = std::fs::remove_dir_all(&p.old_path);
                    } else {
                        let _ = std::fs::remove_file(&p.old_path);
                    }
                } else {
                    log::warn!(
                        "update marker names {} / {}, not this install: left alone",
                        p.new_path.display(),
                        p.old_path.display()
                    );
                }
                let _ = std::fs::remove_file(&marker);
                self.toast(format!("Updated to Shift v{current}"));
                // started by a background helper: come to the front
                if let Some(w) = self.shown_window() {
                    w.focus_window();
                }
            } else if launch.relaunched || age > Duration::from_secs(600) {
                // not the version it names (the helper put the old one back,
                // or a stale marker): clear it — never a reason to roll back
                log::warn!("update marker for v{} on v{current}: cleared", p.to);
                let _ = std::fs::remove_file(&marker);
                if launch.relaunched {
                    self.toast(format!(
                        "Couldn't update to Shift v{}; still on v{current}",
                        p.to
                    ));
                }
            } else {
                // another window's update is in flight
                marker_left = true;
            }
        }
        if self.handles_update_marker()
            && let Some(failed) = update::failed_path()
            && let Some(p) = update::read_pending(&failed)
        {
            let _ = std::fs::remove_file(&failed);
            self.toast(format!(
                "Couldn't update to Shift v{}; still on v{current}",
                p.to
            ));
        }
        if let Some(file) = &launch.restore {
            // validated in take_env: a restart file
            let _ = std::fs::remove_file(file);
        }
        if launch.restart_others {
            let n = crate::updater::reopen_windows(launch.restore.as_deref(), false, self.headless);
            log::info!("update: reopened {n} window(s)");
        } else if self.plain_main_launch() && !marker_left {
            // windows a crashed update saved and never reopened
            let n = crate::updater::reopen_windows(None, true, self.headless);
            if n > 0 {
                log::info!("reopened {n} window(s) left by an interrupted update");
            }
        }
        if launch.relaunched {
            quiet_relaunch_output();
        }
    }
}

/// A process the update helper started writes to `updates/relaunch.log`;
/// once it is confirmed, its later output goes nowhere instead of growing
/// that file for the rest of the session.
fn quiet_relaunch_output() {
    let Ok(null) = std::fs::OpenOptions::new().write(true).open("/dev/null") else {
        return;
    };
    use std::os::fd::AsRawFd;
    unsafe {
        libc::dup2(null.as_raw_fd(), 1);
        libc::dup2(null.as_raw_fd(), 2);
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        self.load_config(true);
        // an update's restart restores from its own file, whatever
        // restore_session says (the dialog promised the tabs come back)
        let restored = if let Some(file) = &crate::updater::launch().restore {
            shift_core::session_state::load_from(file)
        } else if self.config.restore_session && self.persist_session {
            shift_core::session_state::load()
        } else {
            None
        };
        // last window frame (only when it is still a sane size)
        let frame = restored
            .as_ref()
            .and_then(|s| s.window)
            .filter(|f| f.width >= 320.0 && f.height >= 200.0);
        let mut attrs = WindowAttributes::default()
            .with_title("Shift")
            .with_window_icon(crate::icon::window_icon())
            // always transparent so `window_opacity` can change live
            .with_transparent(true)
            .with_inner_size(match frame {
                Some(f) => LogicalSize::new(f.width, f.height),
                None => LogicalSize::new(1180.0, 760.0),
            });
        if let Some(f) = frame {
            attrs = attrs
                .with_position(winit::dpi::LogicalPosition::new(f.x, f.y))
                .with_maximized(f.maximized);
            if f.fullscreen {
                attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
            }
        }

        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs = attrs
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true)
                .with_title_hidden(true);
            if !self.headless {
                crate::icon::set_dock_icon();
            }
        }
        // X11 WM_CLASS / Wayland app_id "shift" matches shift.desktop, so
        // docks, taskbars and switchers show its hicolor icon
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use winit::platform::wayland::WindowAttributesExtWayland;
            use winit::platform::x11::WindowAttributesExtX11;
            attrs = WindowAttributesExtX11::with_name(attrs, "shift", "shift");
            attrs = WindowAttributesExtWayland::with_name(attrs, "shift", "shift");
        }

        // hidden until the accessibility adapter is attached (AccessKit
        // must see the window before it is first shown)
        let window = Arc::new(
            event_loop
                .create_window(attrs.with_visible(false))
                .expect("create window"),
        );
        self.a11y = Some(accesskit_winit::Adapter::with_event_loop_proxy(
            event_loop,
            &window,
            self.proxy.clone(),
        ));
        if !self.headless {
            window.set_visible(true);
        }
        // input methods (CJK composition, dead keys, emoji picker)
        window.set_ime_allowed(true);
        self.os_light = window.theme() == Some(winit::window::Theme::Light);
        let size = window.inner_size();
        let gpu = Gpu::new(window.clone(), size.width, size.height);
        let quads = QuadPipeline::new(&gpu.device, gpu.config.format);
        self.bg_image = Some(shift_renderer::ImagePipeline::new(
            &gpu.device,
            gpu.config.format,
        ));
        self.sprites = Some(shift_renderer::SpritePipeline::new(
            &gpu.device,
            gpu.config.format,
        ));
        let mut text = TextEngine::new(&gpu.device, &gpu.queue, gpu.config.format);
        self.cell_w = text.mono_cell_width(self.config.font_size());

        let git_proxy = self.proxy.clone();
        self.git = Some(GitWorker::new(Arc::new(move |pane, info| {
            let _ = git_proxy.send_event(UserEvent::GitStatus(pane, info));
        })));
        let pr_proxy = self.proxy.clone();
        self.pr_worker = Some(shift_core::github::PrWorker::new(Arc::new(
            move |pane, info| {
                let _ = pr_proxy.send_event(UserEvent::PrStatus(pane, info));
            },
        )));
        let docker_proxy = self.proxy.clone();
        self.docker_worker = Some(shift_core::docker::DockerWorker::new(Arc::new(
            move |pane, info| {
                let _ = docker_proxy.send_event(UserEvent::DockerStatus(pane, info));
            },
        )));

        let watch_proxy = self.proxy.clone();
        self._watcher = shift_core::watch::watch(Arc::new(move || {
            let _ = watch_proxy.send_event(UserEvent::ConfigReloaded);
        }));

        #[cfg(target_os = "macos")]
        {
            let menu_proxy = self.proxy.clone();
            muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| {
                let _ = menu_proxy.send_event(UserEvent::Menu(e.id.0));
            }));
        }

        self.sync_control_server();

        self.window = Some(window);
        self.gpu = Some(gpu);
        self.quads = Some(quads);
        self.text = Some(text);

        self.apply_config();
        self.launch_configs = shift_core::launch::load_all();

        let hist_proxy = self.proxy.clone();
        let shell = shift_core::shell_integration::resolve_shell(self.config.shell.as_deref());
        std::thread::Builder::new()
            .name("history-import".into())
            .spawn(move || {
                let h = shift_core::history::load_shell_history(&shell, 5000);
                let _ = hist_proxy.send_event(UserEvent::HistoryLoaded(h));
            })
            .ok();

        match restored {
            Some(saved) if !saved.tabs.is_empty() => {
                // restoring must not rewrite the file tab by tab
                let persist = std::mem::replace(&mut self.persist_session, false);
                for tab in &saved.tabs {
                    self.restore_tab(tab);
                }
                self.persist_session = persist;
                self.active = saved.active.min(self.tabs.len() - 1);
                self.save_session();
            }
            _ => self.spawn_tab(),
        }
        self.load_shell_env();
        self.plan_start_confirmation();
        self.sync_update_checks();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if let Some(a11y) = &mut self.a11y {
            a11y.process_event(&window, &event);
        }

        match event {
            WindowEvent::CloseRequested => {
                self.request_close(CloseScope::Window, event_loop);
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                #[cfg(target_os = "macos")]
                place_traffic_lights(&window);
                window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                window.request_redraw();
            }
            WindowEvent::ThemeChanged(t) => {
                self.os_light = t == winit::window::Theme::Light;
                if self.config.sync_os_theme {
                    self.apply_config();
                }
            }
            WindowEvent::Focused(f) => {
                self.focused = f;
                self.sync_focus();
                // AppKit may lay the titlebar out again on key-state changes
                #[cfg(target_os = "macos")]
                place_traffic_lights(&window);
                window.request_redraw();
            }
            WindowEvent::Ime(ime) => match ime {
                winit::event::Ime::Preedit(text, _) => {
                    self.preedit = text;
                    window.request_redraw();
                }
                winit::event::Ime::Commit(text) => self.commit_text(&text, event_loop, &window),
                winit::event::Ime::Enabled | winit::event::Ime::Disabled => {
                    self.preedit.clear();
                    window.request_redraw();
                }
            },
            WindowEvent::DroppedFile(path) => {
                self.drop_file(&path);
                window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                use winit::keyboard::ModifiersKeyState::Pressed;
                self.alt_sides = (m.lalt_state() == Pressed, m.ralt_state() == Pressed);
                self.update_cursor_icon(&window);
                window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let press = input::KeyPress::from(&event);
                if press.released {
                    self.on_key_release(&press);
                    return;
                }
                // the input method owns keys while it is composing
                if !self.preedit.is_empty() {
                    return;
                }
                self.on_key(&press, event_loop, &window);
            }
            WindowEvent::CursorLeft { .. } => {
                self.pointer_in = false;
                if self.review.pointer.take().is_some() && !self.review.hover_rows.is_empty() {
                    window.request_redraw();
                }
                if self.tip_shown.is_some() {
                    window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = window.scale_factor() as f32;
                let pos = (position.x as f32 / scale, position.y as f32 / scale);
                self.on_cursor_moved(pos, &window);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: winit::event::MouseButton::Right,
                ..
            } => self.on_right_press(&window),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: winit::event::MouseButton::Right,
                ..
            } => self.on_other_button(crate::mouse::RIGHT, false, &window),
            WindowEvent::MouseInput {
                state,
                button: winit::event::MouseButton::Middle,
                ..
            } => self.on_other_button(
                crate::mouse::MIDDLE,
                state == ElementState::Pressed,
                &window,
            ),
            WindowEvent::MouseInput {
                state,
                button: winit::event::MouseButton::Left,
                ..
            } => self.on_left(state, event_loop, &window),
            WindowEvent::MouseWheel { delta, .. } => self.on_wheel(delta, &window),
            WindowEvent::RedrawRequested => {
                self.redraw(&window);
                // hit rects are rebuilt per frame: a dialog or menu that just
                // opened under a still mouse gets the right shape
                self.update_cursor_icon(&window);
                self.update_a11y(&window);
                if self.any_running {
                    // the running spinner animates; paced so back-to-back
                    // frames never starve the reader thread of the terminal
                    self.request_output_frame();
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::PtyWakeup(pane) => {
                if let Some((_, seen)) = &mut self.latency_probe {
                    *seen = true;
                }
                self.record_completions(pane);
                self.save_transcript_if_changed(pane);
                self.flush_pending_commands(pane);
                self.check_password_prompt(pane);
                self.check_correction(pane);
                let active_idx = self.active;
                if let Some((idx, tab)) = self.find_tab_of_pane(pane) {
                    if idx == active_idx {
                        self.request_output_frame();
                    } else {
                        tab.activity = true;
                        if let Some(p) = tab.pane(pane) {
                            p.session
                                .dirty
                                .store(false, std::sync::atomic::Ordering::Release);
                        }
                        if let Some(w) = &self.window {
                            w.request_redraw(); // tab bar dot
                        }
                    }
                }
            }
            UserEvent::PtyTitle(pane, title) => {
                if let Some((_, tab)) = self.find_tab_of_pane(pane) {
                    if let Some(p) = tab.pane_mut(pane) {
                        p.title = title.clone();
                    }
                    tab.title = title;
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::PtyBell(pane) => {
                self.on_bell(pane);
            }
            UserEvent::GlobalHotkey => self.toggle_visibility(),
            #[cfg(target_os = "macos")]
            UserEvent::Menu(id) => self.run_menu_command(&id, event_loop),
            #[cfg(not(target_os = "macos"))]
            UserEvent::Menu(_) => {}
            UserEvent::PtyClipboard(text) => {
                // a copy queued before clipboard_access turned off is dropped
                if self.config.clipboard_access.trim() != "off" {
                    self.copy_to_clipboard(text);
                }
            }
            UserEvent::PtyClipboardLoad(pane, reply) => {
                // re-checked here: the policy may have changed since the
                // request was parsed
                if self.config.clipboard_access.trim() == "read_write" {
                    let text = if self.headless {
                        Some(self.headless_clipboard.clone())
                    } else {
                        self.clipboard().and_then(|cb| cb.get_text().ok())
                    };
                    if let Some((_, tab)) = self.find_tab_of_pane(pane)
                        && let Some(p) = tab.pane(pane)
                    {
                        p.session
                            .write((reply.0)(&text.unwrap_or_default()).as_bytes());
                    }
                }
            }
            UserEvent::PtyColorRequest(pane, index, reply) => {
                // the program's own setting (OSC 4/10/11/12) wins over the theme
                let theme = self.theme.clone();
                if let Some((_, tab)) = self.find_tab_of_pane(pane)
                    && let Some(p) = tab.pane(pane)
                    && let Some(c) = terminal::palette_color(&theme, &p.session, index)
                {
                    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                    let rgb = shift_pty::alacritty_terminal::vte::ansi::Rgb {
                        r: to8(c.r),
                        g: to8(c.g),
                        b: to8(c.b),
                    };
                    p.session.write((reply.0)(rgb).as_bytes());
                }
            }
            UserEvent::PtyWindowOp(pane, params) => self.window_op(pane, &params),
            UserEvent::PtyExit(pane) => {
                self.close_pane(pane, event_loop);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::ShellEnvLoaded(env) => {
                self.shell_env = Some(env);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::Access(e) => self.on_access_event(e.window_event, event_loop),
            UserEvent::PtyNotify(pane, title, body) => {
                self.on_program_notification(pane, title, body);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::PtyNotice(message) => self.toast(message),
            UserEvent::Completions(generation, items) => self.on_completions(generation, items),
            UserEvent::ReviewWritten(error) => {
                self.review.notice = error;
                self.explorer.refresh();
                self.request_review();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::ReviewLoaded(generation, result) => {
                use crate::views::review_panel::Load;
                if generation != self.review.generation {
                    return; // a newer request is in flight
                }
                self.review.load = match result {
                    None => Load::NotARepo,
                    Some(Err(e)) => Load::Failed(e),
                    Some(Ok(r)) => {
                        self.explorer.set_statuses(r.statuses(), r.dirty_dirs());
                        Load::Ready(Arc::new(r))
                    }
                };
                if !matches!(self.review.load, Load::Ready(_)) {
                    self.explorer
                        .set_statuses(Default::default(), Default::default());
                }
                // listings may have changed with the files
                self.explorer.refresh();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::Control(p) => {
                let response = if p.allowed(crate::control::Level::parse(&self.config.control_api))
                {
                    self.handle_control(p.request)
                } else {
                    crate::control::Response::error("not allowed: control permissions changed")
                };
                let _ = p.reply.send(response);
                if self.restart_requested {
                    // an updating window restarts this one: its layout is
                    // saved; durable shells keep running for the new version
                    log::info!("restarting for an update");
                    event_loop.exit();
                    return;
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::UpdateChecked(result, manual) => self.on_update_checked(result, manual),
            UserEvent::UpdateSurvey(survey) => self.open_update_dialog(survey),
            UserEvent::UpdateInstalled(result, windows) => {
                self.on_update_installed(result, windows, event_loop)
            }
            UserEvent::UpdateReady(own) => self.relaunch_for_update(own, event_loop),
            UserEvent::StartConfirmed => self.confirm_start(),
            UserEvent::HistoryLoaded(mut history) => {
                log::info!("shell history imported: {} entries", history.len());
                Arc::make_mut(&mut self.shell_history).append(&mut history);
            }
            UserEvent::ConfigReloaded => {
                // Theme files can change without any change to config.toml.
                self.load_config(false);
                self.apply_config();
            }
            UserEvent::ViewerLoaded(loaded) => {
                if self.viewer.accept(*loaded)
                    && let Some(w) = &self.window
                {
                    w.request_redraw();
                }
            }
            UserEvent::WindowsLoaded(windows, done) => {
                self.other_windows.extend(windows);
                self.windows_loading = !done;
                if self.palette.open {
                    self.refresh_palette();
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            UserEvent::DetachedLoaded(detached) => {
                self.detached = detached;
                self.detached_loading = false;
                if self.palette.open {
                    self.refresh_palette();
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            UserEvent::GitStatus(pane, info) => {
                let pr_chip = self
                    .config
                    .prompt_chips
                    .contains(&shift_core::PromptChip::GithubPr);
                let mut lookup = None;
                if let Some((_, tab)) = self.find_tab_of_pane(pane)
                    && let Some(p) = tab.pane_mut(pane)
                {
                    let branch = info.as_ref().map(|g| g.branch.clone());
                    // another branch's PR is not this one's
                    if p.git.as_ref().map(|g| &g.branch) != branch.as_ref() {
                        p.pr = None;
                    }
                    p.git = info;
                    let cwd = p.session.cwd.lock().unwrap().clone();
                    if pr_chip && let (Some(cwd), Some(branch)) = (cwd, branch) {
                        lookup = Some((cwd, branch));
                    }
                }
                if let (Some((cwd, branch)), Some(worker)) = (lookup, &self.pr_worker) {
                    worker.request(pane, cwd.into(), branch);
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::PrStatus(pane, info) => {
                if let Some((_, tab)) = self.find_tab_of_pane(pane)
                    && let Some(p) = tab.pane_mut(pane)
                {
                    p.pr = info;
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::DockerStatus(pane, info) => {
                if let Some((_, tab)) = self.find_tab_of_pane(pane)
                    && let Some(p) = tab.pane_mut(pane)
                {
                    p.docker = info;
                }
                // an open container list follows the new state
                if let Some(menu) = self.chip_menu.as_mut()
                    && menu.pane == pane
                    && menu.kind == crate::views::chip_menu::ChipKind::Docker
                {
                    let docker = self
                        .find_tab_of_pane(pane)
                        .and_then(|(_, t)| t.pane(pane))
                        .and_then(|p| p.docker.clone());
                    if let Some(menu) = self.chip_menu.as_mut() {
                        menu.set_containers(docker.as_ref());
                    }
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.sync_focus();
        // a paced output frame whose time has come
        let output_frame = self.output_frame_due;
        if output_frame.is_some_and(|due| Instant::now() >= due) {
            self.output_frame_due = None;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        // a tooltip whose rest time is up
        let tip_due = self.tooltip_due();
        if tip_due.is_some_and(|due| Instant::now() >= due)
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
        let tip_due = tip_due.filter(|due| Instant::now() < *due);
        let output_frame = self.output_frame_due;
        if self.script.is_some() {
            self.drive_script(event_loop);
            if let Some(due) = self.script.as_ref().and_then(|s| s.due()) {
                let due = output_frame.map_or(due, |f| f.min(due));
                let due = tip_due.map_or(due, |t| t.min(due));
                event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(due));
                return;
            }
        }
        // every timed repaint (boot grace, toasts, cursor blink) wakes at the
        // earliest of its deadlines
        let now = Instant::now();
        let mut repaint = false;
        let mut deadlines: Vec<Instant> = Vec::new();
        // boot placeholder grace ends (shell without a prompt marker)
        if let Some(until) = self.boot_until {
            if now >= until {
                self.boot_until = None;
                repaint = true;
            } else {
                deadlines.push(until);
            }
        }
        // status toast
        if let Some((_, at)) = &self.toast {
            let until = *at + TOAST_TTL;
            if now >= until {
                self.toast = None;
                repaint = true;
            } else {
                deadlines.push(until);
            }
        }
        // a dragged pane held still over a tab opens it after TAB_HOVER_OPEN
        if let Some((_, since)) = self
            .pane_drag
            .as_ref()
            .and_then(|d| d.hover_tab)
            .filter(|(j, _)| *j != self.active)
        {
            let due = since + TAB_HOVER_OPEN;
            if now >= due {
                self.track_pane_drag();
                repaint = true;
            } else {
                deadlines.push(due);
            }
        }
        // notification toasts (a hovered one stays)
        repaint |= self.prune_note_toasts();
        let ttl = self.config.notifications.toast_duration();
        deadlines.extend(self.note_toasts.iter().map(|(_, since)| *since + ttl));
        if (self.editor_active || self.palette.open || self.rename.is_some()) && !self.any_running {
            let elapsed = self.blink_epoch.elapsed().as_millis();
            let next_flip = BLINK_INTERVAL.as_millis() - (elapsed % BLINK_INTERVAL.as_millis());
            deadlines.push(now + Duration::from_millis(next_flip as u64 + 5));
            repaint |= next_flip > BLINK_INTERVAL.as_millis() - 30;
        }
        // blinking text (SGR 5) on screen flips on its own beat
        if crate::views::terminal::BLINK_DRAWN.load(std::sync::atomic::Ordering::Relaxed) {
            let period = TEXT_BLINK.as_millis();
            let elapsed = self.blink_epoch.elapsed().as_millis();
            let next_flip = period - (elapsed % period);
            deadlines.push(now + Duration::from_millis(next_flip as u64 + 5));
            repaint |= next_flip > period - 30;
        }
        if repaint && let Some(w) = &self.window {
            w.request_redraw();
        }
        deadlines.extend(output_frame);
        deadlines.extend(tip_due);
        event_loop.set_control_flow(match deadlines.into_iter().min() {
            Some(t) => winit::event_loop::ControlFlow::WaitUntil(t),
            None => winit::event_loop::ControlFlow::Wait,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_option_key_follows_its_own_meta_setting() {
        // (left held, right held), left Meta, right Meta
        assert!(option_meta((true, false), true, false));
        assert!(!option_meta((false, true), true, false));
        assert!(option_meta((false, true), false, true));
        assert!(!option_meta((true, false), false, true));
        assert!(option_meta((true, true), false, true));
        // no side information: the left (main) setting decides
        assert!(option_meta((false, false), true, false));
        assert!(!option_meta((false, false), false, true));
    }

    #[test]
    fn programs_are_revealed_not_opened() {
        use std::path::Path;
        assert_eq!(
            local_path_of("file:///tmp/My%20App.app"),
            Some("/tmp/My App.app".into())
        );
        assert_eq!(local_path_of("/tmp/100%25"), Some("/tmp/100%25".into()));
        assert_eq!(local_path_of("https://example.com/x.app"), None);
        for p in [
            "/x/Foo.app",
            "/x/run.COMMAND",
            "/x/a.workflow",
            "/x/t.terminal",
        ] {
            assert!(opens_as_program(Path::new(p)), "{p}");
        }
        assert!(!opens_as_program(Path::new("/x/notes.txt")));
        // an executable regular file is a program whatever its name
        let dir = std::env::temp_dir().join(format!("shift-exec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("tool");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        assert!(!opens_as_program(&exe));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(opens_as_program(&exe));
        }
        assert!(!opens_as_program(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn confirm_ignores_repeat_and_typed_ahead_enter() {
        assert!(!confirm_accepts(false, Duration::from_millis(5)));
        assert!(!confirm_accepts(true, Duration::from_secs(2)));
        assert!(confirm_accepts(false, CONFIRM_ARM));
    }

    #[test]
    fn completion_worker_pauses_after_failed_start() {
        let mut w = CompletionWorker::<u32>::default();
        let t0 = Instant::now();
        let mut spawns = 0;
        assert!(!w.send(1, t0, || {
            spawns += 1;
            None
        }));
        // paused: no new start, nothing queued
        assert!(!w.send(2, t0 + Duration::from_secs(1), || panic!("respawned")));
        assert!(w.tx.is_none());
        // after the back-off a working start takes the newest request only
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(w.send(3, t0 + COMPLETION_RETRY, || {
            spawns += 1;
            Some(tx)
        }));
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [3]);
        assert_eq!(spawns, 2);
        assert!(w.retry_at.is_none());
    }

    #[test]
    fn completion_worker_recovers_from_a_dead_thread() {
        let mut w = CompletionWorker::<u32>::default();
        let t0 = Instant::now();
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(w.send(1, t0, || Some(tx)));
        drop(rx); // the thread died (its receiver is gone)
        assert!(!w.send(2, t0, || panic!("a live sender needs no start")));
        assert!(!w.send(3, t0 + Duration::from_secs(1), || panic!("respawned")));
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(w.send(4, t0 + COMPLETION_RETRY, || Some(tx)));
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [4]);
    }
}
