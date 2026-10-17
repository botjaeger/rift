use std::sync::Arc;
use std::time::{Duration, Instant};

use rift_core::git::{GitInfo, GitWorker};
use rift_core::{BlockState, Config, Keymap, Theme};
use rift_pty::{GridSize, PaneId, Session, SessionEvent, SpawnOptions};
use rift_renderer::glyphon::{TextArea, TextBounds};
use rift_renderer::{Gpu, QuadPipeline, Scene, TextEngine};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowAttributes, WindowId};

use rift_pty::alacritty_terminal::index::{
    Column as ACol, Line as ALine, Point as APoint, Side as ASide,
};
use rift_pty::alacritty_terminal::selection::{Selection, SelectionType};

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

mod hotkey_window;
mod windows;

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
    PtyClipboardLoad(PaneId, rift_pty::ClipboardReply),
    /// OSC 4/10/11/12 color query to answer from the theme.
    PtyColorRequest(PaneId, usize, rift_pty::ColorReply),
    PtyWindowOp(PaneId, Vec<Option<u16>>),
    /// The system-wide `global_hotkey` was pressed.
    GlobalHotkey,
    /// A native menu item was chosen (its id).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Menu(String),
    /// AppKit was asked to quit (the Dock's Quit): quit as ⌘Q does.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    QuitRequested,
    GitStatus(PaneId, Option<GitInfo>),
    PrStatus(PaneId, Option<rift_core::github::PrInfo>),
    DockerStatus(PaneId, Option<rift_core::docker::ComposeInfo>),
    ConfigReloaded,
    ViewerLoaded(Box<crate::views::viewer::ViewerState>),
    /// The switcher's other-process windows, for the window that asked.
    WindowsLoaded(
        Option<WindowId>,
        Vec<(u32, std::path::PathBuf, Vec<crate::control::TabInfo>)>,
        bool,
    ),
    /// Detached service shells, for the window that asked.
    DetachedLoaded(Option<WindowId>, Vec<rift_pty::ipc::SessionInfo>),
    HistoryLoaded(Vec<String>),
    ShellEnvLoaded(Arc<rift_core::shell_env::ShellEnv>),
    /// Screen reader requests (initial tree, actions).
    Access(accesskit_winit::Event),
    /// A local control API request (see control.rs).
    Control(crate::control::Pending),
    /// A program asked for a notification (OSC 9 / 777 / 99).
    PtyNotify(PaneId, String, String),
    /// A desktop notification was clicked, or its Show / Rerun pressed.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    NotificationResponse(crate::notify::Response),
    /// The desktop-notification permission was read or answered.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    NotifyPermission,
    /// A pane's connection notice (see `SessionEvent::Notice`).
    PtyNotice(String),
    /// Code review model for request `.0`; `None` outside a repository.
    ReviewLoaded(u64, Option<Result<rift_core::review::Review, String>>),
    /// A review-panel write finished (`Some` = why it failed).
    ReviewWritten(Option<String>),
    /// Completions for completion request `.0`.
    Completions(u64, Vec<rift_core::completion::Completion>),
    /// Rift's own completions for request `.0` while the user's shell or
    /// the remote machine is still asked; `Completions` follows.
    CompletionsPartial(u64, Vec<rift_core::completion::Completion>),
    /// The command an SSH profile's tab `.0` runs, built off the UI thread
    /// (`ssh -G` reads the user's ssh config: up to 2 s).
    SshCommand(PaneId, String),
    /// An update check finished (`true`: asked for, so its answer toasts).
    UpdateChecked(Result<rift_core::update::Manifest, String>, bool),
    /// The other windows an update would restart.
    UpdateSurvey(crate::updater::Survey),
    /// The update is downloaded and installed (or why not); the windows
    /// to restart with this one.
    UpdateInstalled(
        Result<rift_core::update::Pending, String>,
        Vec<(u32, std::path::PathBuf)>,
    ),
    /// The other windows saved their layouts and quit: relaunch (this
    /// window's restart file).
    UpdateReady(std::path::PathBuf),
    /// This process has been up long enough (`App::confirm_start`).
    StartConfirmed,
    /// The update for version `.0` was downloaded ahead of the install
    /// (`Ok(true)`; `Ok(false)`: this copy has nothing to download) or why
    /// not.
    UpdateDownloaded(String, Result<bool, String>),
}

/// What the completion thread completes: one editor state and the
/// histories and directory it is completed against.
#[derive(Clone)]
struct CompletionRequest {
    generation: u64,
    pane: PaneId,
    buffer: String,
    cursor: usize,
    /// The pane's own history (shared with its editor, not copied).
    history: Arc<Vec<String>>,
    shell_history: Arc<Vec<String>>,
    cwd: Option<String>,
    /// The pane shell's `$PATH` as its hooks last reported it (`rift-use`
    /// changes it); `None`: the probed shell's.
    path: Option<String>,
    /// The user's shell program, when its own completions are asked too
    /// (`shell_completions`, a local pane).
    shell: Option<String>,
    /// A pane on another machine, when its completions are asked there
    /// (`shell_completions`).
    remote: Option<RemoteCompletion>,
}

/// Where a remote pane completes: its directory there, and the `ssh`
/// command it runs (`None`: none found, so nothing is asked remotely).
#[derive(Clone)]
struct RemoteCompletion {
    cwd: String,
    target: Option<rift_core::remote_complete::Target>,
}

/// The menu row a remote pane shows when it cannot complete remotely.
const REMOTE_HINT: &str = "Remote completions need an SSH profile";

/// The slower half of a request, run on its own thread so Rift's own
/// candidates for newer requests never wait on it: the user's shell, or the
/// remote machine of an ssh pane.
struct SlowCompletion {
    req: CompletionRequest,
    /// Rift's own candidates, to merge into.
    gathered: rift_core::completion::Gathered,
    end: usize,
    source: SlowSource,
}

enum SlowSource {
    /// The shell program and what it is asked.
    Shell(String, rift_core::shell_complete::Query),
    /// The pane's ssh destination and its directory there.
    Remote(rift_core::remote_complete::Target, String),
}

impl CompletionRequest {
    fn finish(&self, g: rift_core::completion::Gathered) -> Vec<rift_core::completion::Completion> {
        g.finish(&self.buffer, &self.history, &self.shell_history)
    }

    /// Rift's own candidates: the whole answer, or the first part while the
    /// user's shell or the remote machine is asked.
    fn run(&self) -> Answer {
        // the whole word around the cursor is completed (zsh without
        // `complete_in_word`); `accept_completion` replaces that same span
        let end = rift_core::completion::token_end(&self.buffer, self.cursor);
        // a pane on another machine: nothing from this one (its commands)
        let g = if self.remote.is_some() {
            rift_core::completion::gather_remote(&self.buffer, end)
        } else {
            rift_core::completion::gather(
                &self.buffer,
                end,
                self.cwd.as_deref(),
                self.path.as_deref().map(std::ffi::OsStr::new),
            )
        };
        let source = if let Some(remote) = &self.remote {
            match &remote.target {
                Some(t) => SlowSource::Remote(t.clone(), remote.cwd.clone()),
                // an ssh Rift cannot find: local-only, with the hint
                None => return Answer::Done(with_remote_hint(self.finish(g))),
            }
        } else {
            let query = self
                .shell
                .as_deref()
                .zip(self.cwd.as_deref())
                .and_then(|(sh, cwd)| {
                    let kind = rift_core::shell_env::ShellKind::from_program(sh);
                    rift_core::shell_complete::query(kind, &self.buffer, end, cwd)
                        .map(|q| (sh.to_string(), q))
                });
            match query {
                Some((shell, q)) => SlowSource::Shell(shell, q),
                None => return Answer::Done(self.finish(g)),
            }
        };
        Answer::Partial(
            self.finish(g.clone()),
            Box::new(SlowCompletion {
                req: self.clone(),
                gathered: g,
                end,
                source,
            }),
        )
    }
}

/// What the fast half of a request answers.
enum Answer {
    /// Rift's own candidates are the whole answer.
    Done(Vec<rift_core::completion::Completion>),
    /// Rift's own candidates now; the rest comes from the slow half.
    Partial(Vec<rift_core::completion::Completion>, Box<SlowCompletion>),
}

impl SlowCompletion {
    /// Ask the shell or the remote machine (within its budget) and merge.
    fn run(self) -> Vec<rift_core::completion::Completion> {
        let SlowCompletion {
            req,
            mut gathered,
            end,
            source,
        } = self;
        match source {
            SlowSource::Shell(shell, q) => {
                let items = rift_core::shell_complete::complete(
                    &shell,
                    &q,
                    rift_core::shell_complete::BUDGET,
                );
                gathered.add(&items, "shell");
                req.finish(gathered)
            }
            SlowSource::Remote(target, cwd) => {
                // no live shared connection: local-only, with the hint
                let Some(socket) = rift_core::remote_complete::control_socket(&target) else {
                    return with_remote_hint(req.finish(gathered));
                };
                rift_core::remote_complete::complete_into(
                    &mut gathered,
                    &socket,
                    &target,
                    &req.buffer,
                    end,
                    &cwd,
                    rift_core::remote_complete::BUDGET,
                );
                req.finish(gathered)
            }
        }
    }
}

/// Start the thread that runs the slow half of requests (newest first;
/// older ones still queued are dropped: their answer would be discarded).
fn spawn_slow(proxy: EventLoopProxy<UserEvent>) -> std::sync::mpsc::Sender<Box<SlowCompletion>> {
    let (tx, rx) = std::sync::mpsc::channel::<Box<SlowCompletion>>();
    let spawned = std::thread::Builder::new()
        .name("completion-shell".into())
        .spawn(move || {
            while let Ok(mut job) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    job = newer;
                }
                let generation = job.req.generation;
                let items = job.run();
                let _ = proxy.send_event(UserEvent::Completions(generation, items));
            }
        });
    if let Err(e) = spawned {
        // `tx` then fails to send and the request is answered without it
        log::warn!("completion-shell thread: {e}");
    }
    tx
}

/// `items` with the [`REMOTE_HINT`] row after the word candidates.
fn with_remote_hint(
    mut items: Vec<rift_core::completion::Completion>,
) -> Vec<rift_core::completion::Completion> {
    let at = items
        .iter()
        .position(|c| c.source == "history")
        .unwrap_or(items.len());
    items.insert(
        at,
        rift_core::completion::Completion {
            insert: String::new(),
            description: REMOTE_HINT.into(),
            source: "hint",
        },
    );
    items
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
    rift_core::review::Op,
    String,
    Option<rift_core::review::HunkId>,
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
    block: Option<rift_core::BlockId>,
    kind: AttentionKind,
    /// Coding agent running in the pane (Claude Code, Codex, …).
    agent: Option<rift_core::notifications::Agent>,
    /// The pane's git branch when it happened.
    branch: Option<String>,
    /// Command or notification title.
    title: String,
    /// Status, duration or notification body.
    detail: String,
    read: bool,
    /// The pane's input count when it was raised (`Session::inputs`): a
    /// request is still open while nothing was typed since.
    inputs: u64,
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
            inputs: 0,
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
/// The audible bell plays at most once per this gap.
const BELL_SOUND_GAP: Duration = Duration::from_millis(150);
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
    /// Divider index within the split at `path`.
    index: usize,
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
    /// The window being served: events, drawing and commands act on it.
    /// The others wait in `parked` (`App::enter_window` swaps one in).
    win: Box<WindowState>,
    /// Every other window (in-process, one per ⌘N or restored window).
    parked: Vec<Box<WindowState>>,
    /// Every window's id in the order they were opened (the session file
    /// and the script driver's `window <n>` count in this order).
    window_order: Vec<WindowId>,
    /// The window that last had keyboard focus: menu commands, the global
    /// hotkey and the control socket act on it.
    focused_window: Option<WindowId>,
    /// Windows by when they last took focus, newest last (closing one
    /// serves the one before it).
    focus_history: Vec<WindowId>,
    /// Saved windows to open at the next turn of the event loop (an
    /// update's restart files).
    queued_windows: Vec<rift_core::session_state::SavedWindowTabs>,
    /// Last review-panel request id, across every window.
    review_generation: u64,
    proxy: EventLoopProxy<UserEvent>,
    bg_image_opacity: f32,
    text: Option<TextEngine>,
    git: Option<GitWorker>,
    pr_worker: Option<rift_core::github::PrWorker>,
    docker_worker: Option<rift_core::docker::DockerWorker>,
    /// The prompt chips the panes' git and Docker state was last fetched for.
    chips_applied: Vec<rift_core::PromptChip>,
    next_pane: PaneId,
    theme: Theme,
    /// The theme's color scheme as reported to programs (1 dark, 2 light).
    color_scheme: Option<u8>,
    cell_w: f32,
    /// Why the configured theme did not load (toasted once per change).
    theme_error: Option<String>,
    blink_epoch: Instant,
    config: Config,
    keymap: Keymap,
    clipboard: Option<arboard::Clipboard>,
    /// Imported shell history (shared with the completion thread).
    shell_history: Arc<Vec<String>>,
    font_size: f32,
    line_h: f32,
    _watcher: Option<rift_core::watch::ConfigWatcher>,
    /// `--script` driver (see script.rs).
    script: Option<crate::script::Script>,
    /// `--headless`: the window is never shown, focused or raised, and no
    /// global hotkey is registered (scripts only; `shot` reads offscreen).
    headless: bool,
    /// Clipboard of a headless run: the system clipboard is never touched.
    headless_clipboard: String,
    /// Files on the headless clipboard (script `copyfiles`).
    headless_files: Vec<std::path::PathBuf>,
    /// OS appearance is light (drives `sync_os_theme`).
    os_light: bool,
    /// Last audible bell: a flood of BELs (`cat` of a binary) plays one sound
    /// per `BELL_SOUND_GAP`, not one process each.
    last_beep: Option<Instant>,
    /// Warp-style launch configurations (~/.config/rift/launch_configs).
    launch_configs: Vec<rift_core::launch::LaunchConfig>,
    /// Config files the palette last found unreadable: a toast names them
    /// once, not again on every palette open while the set stays the same.
    launch_skipped: Vec<rift_core::launch::Skipped>,
    /// Commands a launch configuration queued for a pane; sent at its first prompt.
    pending_commands: std::collections::HashMap<PaneId, Vec<String>>,
    /// Keys whose press went to a program reporting key events (kitty
    /// flag 2), so their release follows them there.
    keys_down: std::collections::HashSet<(PaneId, String)>,
    /// Transcript id to replay into the next spawned pane (session restore).
    next_replay: Option<String>,
    /// Session-service shell for the next spawned pane to reattach.
    next_attach: Option<u64>,
    /// Local control API socket (removed on drop).
    control_server: Option<crate::control::Server>,
    /// Attention center entries, newest last.
    attention: Vec<Attention>,
    next_attention_id: u64,
    /// Attention entries raised by Settings › Notifications › Try it: their
    /// sample command is never rerun.
    test_alerts: std::collections::HashSet<u64>,
    /// Newest finished block already considered per pane.
    attention_seen: std::collections::HashMap<PaneId, rift_core::BlockId>,
    /// Newest finished block written to each pane's transcript.
    transcript_saved: std::collections::HashMap<PaneId, rift_core::BlockId>,
    /// ssh profiles listed in the palette (config + ~/.ssh/config).
    ssh_profiles: Vec<rift_core::ssh::SshProfile>,
    /// `RIFT_LATENCY_LOG`: log key-to-frame latency of raw keystrokes.
    latency_log: bool,
    /// Raw key sent at .0; .1 = the program's output has arrived since.
    latency_probe: Option<(Instant, bool)>,
    /// The releases What's New lists.
    releases: Vec<rift_core::changelog::Release>,
    /// The tab label's shell name for the configured shell: resolving it
    /// stats the shell binaries, too slow for every frame.
    shell_label: Option<(Option<String>, String)>,
    /// When the session file last took the counted time while a tab was
    /// counting (None while none is): a crash loses at most a minute of it.
    clock_saved: Option<Instant>,
    /// How many tabs were counting at the last wake (a drop saves the file).
    clock_counting: usize,
    /// Every tab's focused pane at the last wake: a change saves the
    /// session file (None until the first wake).
    saved_focus: Option<Vec<PaneId>>,
    /// Aliases/functions/builtins probed from the user's shell (background).
    shell_env: Option<Arc<rift_core::shell_env::ShellEnv>>,
    /// Session file is read/written (off for scripts).
    persist_session: bool,
    /// New shells may run in the session service (`durable_sessions`, and
    /// a scripted run only with `RIFT_PERSIST_SESSION`: tests leave no
    /// service behind).
    durable_allowed: bool,
    update: UpdateUi,
    /// Remote access: the gateway's status, its start, pairing.
    remote: RemoteUi,
    /// A `prepare-restart` request was answered: quit, keeping durable
    /// shells.
    restart_requested: bool,
    next_toast_id: u64,
    /// Queue of the completion thread (started on the first request).
    completion_worker: CompletionWorker<CompletionRequest>,
    /// Latest completion request not yet answered.
    completion_pending: Option<CompletionRequest>,
    /// Key that read the menu or ghost while `completion_pending` was in
    /// flight: applied once, when that request's answer arrives.
    completion_intent: Option<CompletionIntent>,
    /// Counter identifying completion requests.
    completion_gen: u64,
    /// The pending request whose partial answer (Rift's own candidates) is
    /// shown while the shell or remote machine is asked.
    completion_partial: Option<u64>,
    /// The pane the shown completions belong to.
    completion_pane: Option<PaneId>,
    /// Queue of the review writer thread (started on the first write).
    review_writer: Option<std::sync::mpsc::Sender<ReviewWrite>>,
    /// Why config.toml was last rejected (unreadable or invalid). While set,
    /// the live config is the last good one and Settings does not save, so
    /// the user's file is never overwritten with defaults.
    config_error: Option<String>,
    /// Registered `global_hotkey` (re-registered when the setting changes).
    global_hotkey: Option<crate::hotkey::GlobalHotkey>,
    /// Native menu bar and the keybindings it was built for.
    #[cfg(target_os = "macos")]
    menubar: Option<(
        crate::menubar::MenuBar,
        std::collections::BTreeMap<String, String>,
    )>,
}

/// Everything one window owns: its winit window and GPU surface, tabs and
/// panes, overlays, menus, panels and per-frame hit rects. Shared state
/// (config, fonts, workers, the control socket) stays on [`App`].
struct WindowState {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    quads: Option<QuadPipeline>,
    /// Theme `background_image` (Warp-style), drawn under every layer.
    bg_image: Option<rift_renderer::ImagePipeline>,
    /// Inline images (kitty graphics / iTerm2) on the GPU.
    sprites: Option<rift_renderer::SpritePipeline>,
    tabs: Vec<TabState>,
    active: usize,
    scene: Scene,
    modifiers: ModifiersState,
    /// Left and right Option (Alt) held, where the platform tells them
    /// apart (macOS); both false otherwise.
    alt_sides: (bool, bool),
    hits: Vec<Hit>,
    mouse: (f32, f32),
    any_running: bool,
    editor_active: bool,
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
    settings: SettingsState,
    slider_drag: Option<(&'static str, [f32; 4])>,
    search: SearchState,
    pending_sel: Option<PendingSelection>,
    /// When a selection dragged past its pane's edge last scrolled it.
    select_scrolled: Option<Instant>,
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
    scroll_accum: f32,
    /// cwds of recently closed tabs (⇧⌘T reopens, newest last).
    closed_tabs: Vec<Option<std::path::PathBuf>>,
    /// A block header's copy button just copied (pane, block, when): its
    /// icon shows a check for [`COPIED_SHOWN`].
    copied: Option<(PaneId, rift_core::BlockId, Instant)>,
    /// Next frame is also rendered offscreen and written to this PNG.
    capture: Option<std::path::PathBuf>,
    /// Window has keyboard focus (completion notifications fire only when it doesn't).
    focused: bool,
    /// Pane that was last told it has focus (DECSET 1004), if any.
    focus_pane: Option<PaneId>,
    /// Visual bell flash start.
    bell_flash: Option<Instant>,
    /// Running-process close/quit warning awaiting a decision.
    confirm: Option<ConfirmClose>,
    /// Hint mode labels while active (⇧⌘H).
    hints: Option<HintState>,
    /// Other windows' tabs, fetched when the session switcher opens.
    other_windows: Vec<(u32, std::path::PathBuf, Vec<crate::control::TabInfo>)>,
    /// Detached session-service shells, fetched when the palette opens.
    detached: Vec<rift_pty::ipc::SessionInfo>,
    windows_loading: bool,
    detached_loading: bool,
    /// In-window notification toasts: (attention id, shown since), newest
    /// first; hovering resets the time.
    note_toasts: Vec<(u64, Instant)>,
    note_toast_hits: Vec<crate::views::notifications::ToastHit>,
    /// Rift's own notices (updates) among `note_toasts`: no pane, no inbox
    /// row; they go when their toast goes.
    app_notes: Vec<AppNote>,
    /// Pipe modal target: (pane, block, the block's output text).
    pipe_target: Option<(PaneId, rift_core::BlockId, String)>,
    /// Screen reader bridge (AccessKit); created before the window shows.
    a11y: Option<accesskit_winit::Adapter>,
    /// Rects behind actionable accessibility nodes (click targets).
    a11y_targets: std::collections::HashMap<accesskit::NodeId, [f32; 4]>,
    /// Last pushed tree (unchanged frames push nothing); empty = resend.
    a11y_fingerprint: String,
    /// Newest finished block announced per pane.
    a11y_done: std::collections::HashMap<PaneId, rift_core::BlockId>,
    a11y_announcement: Option<String>,
    /// [cancel, destructive] button rects of the confirm modal.
    confirm_rects: [[f32; 4]; 2],
    /// What's New modal (views/whats_new.rs) and the releases it lists.
    whats_new: WhatsNewState,
    whats_new_hits: WhatsNewHits,
    /// Notifications popover (views/notifications.rs) under the inbox button.
    notifications: NotificationsState,
    notifications_hits: NotificationsHits,
    /// Side panels (views/explorer.rs, views/review_panel.rs) and the file
    /// viewer over the pane area, with their last frame's click targets.
    explorer: ExplorerState,
    explorer_hits: Vec<([f32; 4], ExplorerHit)>,
    review: ReviewPanelState,
    review_hits: Vec<([f32; 4], ReviewHit)>,
    /// This frame's prompt chips that do nothing when clicked but explain
    /// themselves on hover (Elapsed: when its clock started).
    info_chips: Vec<([f32; 4], PaneId, rift_core::PromptChip)>,
    /// XTWINOPS state: iconified by a program, the last requested position
    /// (a hidden window reports none), the size before a one-way maximize.
    iconified: bool,
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
    /// Unfocused panes the last frame washed (`dim_inactive_panes`; dump).
    dimmed_panes: usize,
    /// Tooltips of this frame's controls, in click priority
    /// (`views::tooltip`); a tip with no text blocks the ones under it.
    tips: Vec<crate::views::tooltip::Tip>,
    /// The control under the pointer and since when it rests there.
    tip_hover: Option<([f32; 4], Instant)>,
    /// The tooltip drawn in the last frame.
    tip_shown: Option<crate::views::tooltip::Tip>,
    /// Whole minutes the Elapsed chip showed last (a change repaints it).
    elapsed_minutes: Option<u64>,
    /// Where that tooltip's card and its button were drawn.
    tip_drawn: Option<crate::views::tooltip::Drawn>,
    /// A plain press on a link, acted on at its release (`click_link`).
    link_press: Option<(PaneId, String)>,
    /// When a shown tooltip last went away (the next one shows at once).
    tip_hidden_at: Option<Instant>,
    /// A press, key or wheel hid it until the pointer moves to another control.
    tip_suppressed: bool,
    /// The pointer is over the window.
    pointer_in: bool,
    /// Directory the review model was last requested for.
    review_cwd: Option<String>,
    /// Last git status poll of the active tab's panes (changes made while a
    /// pane is busy, by an agent or an editor, show without a command
    /// ending), and of its Docker Compose state.
    git_polled: Option<Instant>,
    docker_polled: Option<Instant>,
    /// The pane the last status request saw focused: focusing another
    /// refreshes it at once.
    status_pane: Option<PaneId>,
    viewer: ViewerState,
    viewer_hits: Vec<([f32; 4], ViewerHit)>,
    /// Pane area the viewer covers (last frame).
    viewer_rect: [f32; 4],
    /// The viewer shows `config.toml` beside the Settings page, in the
    /// Settings tab, instead of in a terminal tab's split.
    config_viewer: bool,
    /// The viewer pane (`VIEWER_PANE` leaf) has keyboard focus rather than
    /// its tab's `active_pane` shell.
    viewer_focus: bool,
    /// Inline tab rename in progress: (tab index, edited title).
    rename: Option<(usize, String)>,
    /// Rename text still fully selected (first keystroke replaces it).
    rename_fresh: bool,
    /// Search overlay edits a per-block output filter instead of find.
    filter_editing: bool,
    /// Tab being dragged in the tab bar.
    tab_drag: Option<TabDrag>,
    /// ⌃R command history pane above the command line.
    history: crate::views::history::HistoryPane,
    /// Split pane being moved by its title bar.
    pane_drag: Option<PaneDrag>,
    /// Editor mouse selection in progress (pane).
    editor_drag: Option<PaneId>,
    /// Transient status message (text, shown at).
    /// Status toasts, newest first (`App::toast_as`).
    toasts: Vec<Toast>,
    toast_hits: Vec<crate::views::confirm::ToastHit>,
    /// Editor buffer the shown completions and ghost belong to.
    completion_buffer: String,
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
    /// The drop-down hotkey window's state; `None` for an ordinary window.
    dropdown: Option<hotkey_window::Dropdown>,
}

/// What a hint label acts on.
#[derive(Debug, Clone, PartialEq)]
enum HintTarget {
    /// A URL or an existing path: open it (⇧: copy it).
    Open(String),
    /// A block header: select it (⇧: copy its output).
    Block(PaneId, rift_core::BlockId),
}

struct HintState {
    /// (label, on-screen rect, target)
    items: Vec<(String, [f32; 4], HintTarget)>,
    /// Label characters typed so far.
    typed: String,
}

/// White over unfocused split panes on a dark theme (Warp): the pane you are
/// in stays dark and the others wash lighter, so the focused one stands out
/// at a glance (Warp's inactive pane measures #0e0e0e → #252525: ~10 %, the
/// `inactive_pane_dim` default). The pane dividers keep this strength
/// whatever the setting.
const INACTIVE_LIFT: f32 = 0.10;

/// The wash at `lift` (a dark theme's alpha): black at half of it on a light
/// theme (white would vanish on a light ground, and black reads much heavier
/// there).
fn inactive_dim(theme: &rift_core::Theme, lift: f32) -> rift_core::Color {
    if theme.is_light() {
        rift_core::Color::BLACK.with_alpha(lift / 2.0)
    } else {
        rift_core::Color::WHITE.with_alpha(lift)
    }
}

/// The welcome list's rows, with shortcuts from the live keymap (row `i`
/// runs `welcome_action(i)`).
fn welcome_entries(keymap: &rift_core::Keymap) -> Vec<crate::views::welcome::Entry> {
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

/// The close/quit warning's description: the connected phones (a quit
/// only), then the running processes.
fn close_description(running: usize, phones: &[String]) -> String {
    let processes = match running {
        0 => String::new(),
        1 => "1 running process will be terminated.".to_string(),
        n => format!("{n} running processes will be terminated."),
    };
    let phone = match phones {
        [] => return processes,
        [one] => format!("{one} is connected and loses its tabs."),
        [first, rest @ ..] => format!(
            "{first} and {} other phone{} are connected and lose their tabs.",
            rest.len(),
            if rest.len() == 1 { "" } else { "s" }
        ),
    };
    if processes.is_empty() {
        phone
    } else {
        format!("{phone} {processes}")
    }
}

/// Whether Enter accepts a confirmation shown for `shown`: never an
/// auto-repeat, never before the dialog could have been read.
fn confirm_accepts(repeat: bool, shown: Duration) -> bool {
    !repeat && shown >= CONFIRM_ARM
}

/// How `pane`'s program wants keys encoded.
fn key_encoding(pane: &PaneState, option_as_meta: bool) -> input::KeyEncoding {
    use rift_pty::alacritty_terminal::term::TermMode;
    let mode = *pane.session.term.lock().mode();
    input::KeyEncoding {
        app_cursor: mode.contains(TermMode::APP_CURSOR),
        option_as_meta,
        kitty_flags: crate::keyboard::flags_from_mode(mode),
    }
}

/// Whether `pane`'s program enabled bracketed paste (DECSET 2004).
fn bracketed_paste_on(pane: &PaneState) -> bool {
    use rift_pty::alacritty_terminal::term::TermMode;
    pane.session
        .term
        .lock()
        .mode()
        .contains(TermMode::BRACKETED_PASTE)
}

/// A link target's URL scheme (RFC 3986: a letter, then letters, digits,
/// `+`, `-` or `.`, before the first `:`), lowercased. `None` for a path.
fn scheme_of(target: &str) -> Option<String> {
    let (scheme, _) = target.split_once(':')?;
    let mut chars = scheme.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then(|| scheme.to_ascii_lowercase())
}

/// The local path a link target names: `/abs/path`, or a `file:` URL of
/// this machine (`file:/p`, `file:///p`, `file://localhost/p`).
fn local_path_of(target: &str) -> Option<std::path::PathBuf> {
    let rest = match scheme_of(target).as_deref() {
        Some("file") => {
            let r = &target["file:".len()..];
            match r.strip_prefix("//") {
                // an authority: only this machine's
                Some(auth) => auth
                    .get(..9)
                    .filter(|h| h.eq_ignore_ascii_case("localhost"))
                    .map_or(auth, |_| &auth[9..]),
                None => r,
            }
        }
        Some(_) => return None,
        None => target,
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
/// Terminal `.command` files, link files that open another target, system
/// add-ons, anything executable) instead of showing a document. A symlink is
/// judged by what it points at too: the opener follows it.
fn opens_as_program(path: &std::path::Path) -> bool {
    const PROGRAM_EXTS: &[&str] = &[
        "app",
        "command",
        "terminal",
        "workflow",
        "action",
        "scpt",
        "scptd",
        "tool",
        "sh",
        "pkg",
        "mpkg",
        "dmg",
        "jar",
        "desktop",
        "appimage",
        "fileloc",
        "webloc",
        "inetloc",
        "url",
        "prefpane",
        "saver",
        "mobileconfig",
        "bundle",
        "plugin",
        "kext",
        "osax",
        "service",
    ];
    let program_ext = |p: &std::path::Path| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| PROGRAM_EXTS.contains(&e.to_ascii_lowercase().as_str()))
    };
    if program_ext(path) || std::fs::canonicalize(path).is_ok_and(|real| program_ext(&real)) {
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
    match local_path_of(target) {
        Some(path) if opens_as_program(&path) => return reveal_external(&path),
        Some(_) => {}
        // the opener takes anything with a scheme (`file:/x.app` with one
        // slash, `x-apple-…:` handlers) and reads a leading `-` as an
        // option: only web and mail links get past here
        None => match scheme_of(target) {
            Some(s) if matches!(s.as_str(), "http" | "https" | "ftp" | "mailto") => {}
            Some(s) => return Err(format!("{s}: links are not opened")),
            None => return Err("not a link or an absolute path".into()),
        },
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
    let dir = rift_core::shell_integration::data_dir()?.join("pipe");
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
/// How long a toast that worked stays (an error stays until dismissed).
const TOAST_TTL: Duration = Duration::from_millis(2600);
/// How long other toasts stay (information, a phone's action).
const TOAST_INFO_TTL: Duration = Duration::from_secs(4);
/// Toasts kept at once (an error stays until dismissed, so they can pile).
const TOASTS_MAX: usize = 6;

/// A status toast: a chip in the pane it is about (DESIGN.md "Toasts").
struct Toast {
    id: u64,
    kind: crate::views::confirm::ToastKind,
    text: String,
    /// The pane it is about; `None` or a pane out of view: the window's
    /// bottom-right corner.
    pane: Option<PaneId>,
    action: Option<ToastAction>,
    at: Instant,
}

impl Toast {
    /// Gone after this; `None`: an error, until dismissed.
    fn ttl(&self) -> Option<Duration> {
        use crate::views::confirm::ToastKind;
        match self.kind {
            ToastKind::Error => None,
            ToastKind::Done => Some(TOAST_TTL),
            ToastKind::Info | ToastKind::Phone => Some(TOAST_INFO_TTL),
        }
    }
}

/// A toast's button.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ToastAction {
    OpenConfig,
}

impl ToastAction {
    fn label(self) -> &'static str {
        match self {
            ToastAction::OpenConfig => "Open config.toml",
        }
    }
}

/// How long a header's copy button shows its check after a copy.
const COPIED_SHOWN: Duration = Duration::from_millis(1500);
/// How often the session file takes the counted time while a tab counts.
const CLOCK_SAVE_EVERY: Duration = Duration::from_secs(60);
/// How often the focused window's active tab is asked for its git status
/// (the branch and diff chips, the review panel) between commands.
const GIT_POLL: Duration = Duration::from_secs(3);
/// The same for its Docker Compose state (a `docker compose` call, dearer).
const DOCKER_POLL: Duration = Duration::from_secs(10);
/// A selection dragged past its pane's edge scrolls on this beat...
const SELECT_SCROLL_EVERY: Duration = Duration::from_millis(50);
/// ...by up to this many lines a step (one per line height out).
const SELECT_SCROLL_MAX: usize = 8;

/// What a confirmed close/quit acts on.
#[derive(Debug, Clone, Copy, PartialEq)]
enum CloseScope {
    Pane(PaneId),
    Tab(usize),
    OtherTabs(usize),
    /// The window being served; the last one quits, as `App` does.
    Window,
    /// Every window: quit.
    App,
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
    /// Write Rift's shell hooks to an ssh host (runs in a new tab).
    InstallRemote(rift_core::ssh::SshProfile),
    /// Discard unstaged changes to a file or one hunk (review panel).
    Revert {
        root: std::path::PathBuf,
        path: String,
        hunk: Option<rift_core::review::HunkId>,
    },
    /// Install a release and restart (Update / Later).
    Update(Box<UpdatePlan>),
    /// Unpair a phone (Settings › Remote access).
    RevokeDevice {
        id: String,
        name: String,
    },
    /// Let a view-only phone run commands and send keys again.
    AllowOperate {
        id: String,
        name: String,
    },
}

/// The Operate confirmation's description.
const ALLOW_OPERATE: &str =
    "It can run commands and send keys in your tabs. Its open connections reconnect.";

/// What the update dialog offers (computed when it opens).
#[derive(Debug, Clone, PartialEq)]
struct UpdatePlan {
    manifest: rift_core::update::Manifest,
    target: rift_core::update::InstallTarget,
    /// Other windows that restart with this one (pid, control socket).
    windows: Vec<(u32, std::path::PathBuf)>,
    description: String,
    note: String,
    /// This install cannot update itself: the button opens the release page.
    open_page: bool,
}

/// This window's view of updates (`rift_core::update`, `updater.rs`).
#[derive(Default)]
struct UpdateUi {
    status: rift_core::update::Status,
    /// The version that can be offered: downloaded ahead of time, or one
    /// this copy cannot download (it opens the release page instead), or
    /// whose download failed (the install then downloads it).
    ready: Option<String>,
    /// The version being downloaded ahead of time.
    fetching: Option<String>,
    /// The version whose "ready" notice was shown (once per version).
    notified: Option<String>,
    /// The ready notice's detail for `ready` (downloaded, nothing to
    /// download, or to be downloaded by the install).
    ready_detail: &'static str,
    /// "Later" was chosen for this version: no chip.
    dismissed: Option<String>,
    /// Downloading, installing or restarting.
    busy: bool,
    /// The periodic check runs while this is set.
    auto: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Held from the download until this process exits or the install
    /// stops: one window installs at a time.
    lock: Option<rift_core::update::InstallLock>,
    /// The window was closed while busy: quit once the update is done.
    quit_after: bool,
    /// Set at start: `confirm_start` runs this long after the first frame.
    confirm_after: Option<Duration>,
    /// `confirm_start` ran (it runs once).
    start_confirmed: bool,
}

/// One of Rift's own notices (`App::app_note`), shown as a notification
/// toast.
struct AppNote {
    id: u64,
    title: String,
    detail: String,
    /// What a click on it does.
    action: AppNoteAction,
    /// Stays until clicked or dismissed (the toast duration does not end it).
    sticky: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum AppNoteAction {
    None,
    /// The update dialog.
    Update,
    WhatsNew,
}

/// This window's view of the remote gateway (`remote.rs`,
/// `rift_remote::registry`): read from `<data dir>/remote/` at most once a
/// second, never per frame.
#[derive(Default)]
struct RemoteUi {
    /// The running gateway's `status.json` (`None`: no gateway holds the
    /// lock).
    status: Option<rift_remote::registry::Status>,
    /// `status.json`'s mtime when `status` was read (re-read on change).
    status_mtime: Option<std::time::SystemTime>,
    /// Last poll; the next is due a second later.
    polled: Option<Instant>,
    /// Last time this window started a gateway (at most every 5 s).
    spawned: Option<Instant>,
    /// `remote-status` notices received from the gateway.
    notices: u64,
    /// When the tab-bar chip last appeared, changed or went away.
    chip_changed: Option<Instant>,
    /// "Try again" was pressed: the relay shows as connecting until then,
    /// though a failure that is instant (DNS) already wrote its error.
    retry_shown_until: Option<Instant>,
}

/// How often the remote gateway's files are read.
const REMOTE_POLL: Duration = Duration::from_secs(1);
/// A window starts a gateway at most this often.
const REMOTE_SPAWN_EVERY: Duration = Duration::from_secs(5);
/// Longest relay URL the Settings field takes, in bytes.
const MAX_RELAY_URL: usize = 512;

/// The release page when a manifest names none.
const RELEASES_PAGE: &str = "https://github.com/botjaeger/rift-releases/releases";

struct ConfirmClose {
    /// When the dialog appeared: Enter typed ahead of it does not accept.
    opened: Instant,
    action: ConfirmAction,
    /// Running commands (close) or the first pasted lines (paste).
    commands: Vec<String>,
}

impl WindowState {
    /// A window not yet created (`App::create_window` fills it in);
    /// `first`: the launch window, which shows What's New after an update.
    fn new(first: bool) -> Self {
        WindowState {
            window: None,
            gpu: None,
            quads: None,
            bg_image: None,
            sprites: None,
            tabs: Vec::new(),
            active: 0,
            scene: Scene::default(),
            modifiers: ModifiersState::empty(),
            alt_sides: (false, false),
            hits: Vec::new(),
            mouse: (0.0, 0.0),
            any_running: false,
            editor_active: false,
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
            settings: SettingsState::default(),
            slider_drag: None,
            search: SearchState::default(),
            pending_sel: None,
            select_scrolled: None,
            last_click: None,
            context_menu: ContextMenuState::default(),
            context_menu_rects: Vec::new(),
            chip_menu: None,
            chip_menu_card: [0.0; 4],
            chip_menu_rows: Vec::new(),
            prompt_editor: None,
            prompt_editor_layout: Default::default(),
            scroll_accum: 0.0,
            closed_tabs: Vec::new(),
            copied: None,
            capture: None,
            focused: true,
            focus_pane: None,
            bell_flash: None,
            confirm: None,
            pipe_target: None,
            hints: None,
            windows_loading: false,
            detached_loading: false,
            other_windows: Vec::new(),
            detached: Vec::new(),
            note_toasts: Vec::new(),
            app_notes: Vec::new(),
            note_toast_hits: Vec::new(),
            a11y: None,
            a11y_targets: Default::default(),
            a11y_fingerprint: String::new(),
            a11y_done: Default::default(),
            a11y_announcement: None,
            confirm_rects: [[0.0; 4]; 2],
            whats_new: if first {
                WhatsNewState::on_launch()
            } else {
                WhatsNewState::default()
            },
            whats_new_hits: WhatsNewHits::default(),
            notifications: NotificationsState::default(),
            notifications_hits: NotificationsHits::default(),
            explorer: ExplorerState::default(),
            explorer_hits: Vec::new(),
            review: ReviewPanelState::default(),
            review_hits: Vec::new(),
            info_chips: Vec::new(),
            iconified: false,
            last_frame: Instant::now(),
            output_frame_due: None,
            fullscreen_op: false,
            window_pos: None,
            restore_size: None,
            hover_sig: 0,
            hover_repaints: 0,
            dimmed_panes: 0,
            tips: Vec::new(),
            tip_hover: None,
            tip_shown: None,
            tip_drawn: None,
            elapsed_minutes: None,
            link_press: None,
            tip_hidden_at: None,
            tip_suppressed: false,
            pointer_in: true,
            review_cwd: None,
            git_polled: None,
            docker_polled: None,
            status_pane: None,
            viewer: ViewerState::default(),
            viewer_hits: Vec::new(),
            viewer_rect: [0.0; 4],
            config_viewer: false,
            viewer_focus: false,
            rename: None,
            rename_fresh: false,
            filter_editing: false,
            tab_drag: None,
            history: Default::default(),
            pane_drag: None,
            editor_drag: None,
            toasts: Vec::new(),
            toast_hits: Vec::new(),
            completion_buffer: String::new(),
            mouse_report: None,
            mouse_hover_cell: None,
            preedit: String::new(),
            ime_spot: None,
            ime_area_sent: None,
            cursor_icon: winit::window::CursorIcon::Default,
            dropdown: None,
        }
    }
}

impl App {
    pub fn new(
        proxy: EventLoopProxy<UserEvent>,
        script: Option<crate::script::Script>,
        headless: bool,
    ) -> Self {
        // scripted runs never touch the user's session file (a script run
        // used to overwrite it)
        let persist_session =
            script.is_none() || std::env::var_os("RIFT_PERSIST_SESSION").is_some();
        let durable_allowed =
            script.is_none() || std::env::var_os("RIFT_PERSIST_SESSION").is_some();
        App {
            win: Box::new(WindowState::new(true)),
            parked: Vec::new(),
            window_order: Vec::new(),
            focused_window: None,
            focus_history: Vec::new(),
            queued_windows: Vec::new(),
            review_generation: 0,
            proxy,
            bg_image_opacity: 1.0,
            text: None,
            git: None,
            pr_worker: None,
            docker_worker: None,
            chips_applied: Vec::new(),
            next_pane: 1,
            theme: Theme::default(),
            color_scheme: None,
            cell_w: DEFAULT_FONT_SIZE * 0.6,
            theme_error: None,
            blink_epoch: Instant::now(),
            config: Config::default(),
            keymap: Keymap::from_overrides(&Default::default()),
            clipboard: None,
            shell_history: Arc::default(),
            font_size: DEFAULT_FONT_SIZE,
            line_h: DEFAULT_FONT_SIZE * 1.2,
            _watcher: None,
            script,
            headless,
            headless_clipboard: String::new(),
            headless_files: Vec::new(),
            os_light: false,
            last_beep: None,
            launch_configs: Vec::new(),
            launch_skipped: Vec::new(),
            pending_commands: std::collections::HashMap::new(),
            keys_down: Default::default(),
            ssh_profiles: Vec::new(),
            next_replay: None,
            next_attach: None,
            control_server: None,
            attention: Vec::new(),
            next_attention_id: 0,
            test_alerts: Default::default(),
            attention_seen: Default::default(),
            transcript_saved: Default::default(),
            latency_log: std::env::var_os("RIFT_LATENCY_LOG").is_some(),
            latency_probe: None,
            releases: rift_core::changelog::releases(),
            shell_label: None,
            clock_saved: None,
            clock_counting: 0,
            saved_focus: None,
            shell_env: None,
            persist_session,
            durable_allowed,
            update: UpdateUi::default(),
            remote: RemoteUi::default(),
            restart_requested: false,
            next_toast_id: 0,
            config_error: None,
            review_writer: None,
            completion_worker: CompletionWorker::default(),
            completion_pending: None,
            completion_intent: None,
            completion_gen: 0,
            completion_partial: None,
            completion_pane: None,
            global_hotkey: None,
            #[cfg(target_os = "macos")]
            menubar: None,
        }
    }

    /// Re-apply everything derived from Config (theme, metrics, keymap).
    fn apply_config(&mut self) {
        self.sync_control_server();
        // remote access turned on starts a gateway now; off clears pairing
        self.poll_remote(true);
        if self.win.window.is_some() {
            self.sync_update_checks();
        }
        self.theme = self.config.resolve_theme_for(self.os_light);
        let colors = terminal::theme_rgbs(&self.theme);
        // programs that asked (?2031, Claude Code) hear a dark/light switch
        let scheme = rift_pty::alacritty_terminal::term::color_scheme(colors[257]);
        if self
            .color_scheme
            .replace(scheme)
            .is_some_and(|was| was != scheme)
        {
            let report = format!("\x1b[?997;{scheme}n");
            for p in self.all_panes() {
                if p.session.term.lock().color_scheme_updates() {
                    p.session.write_report(report.as_bytes());
                }
            }
        }
        rift_pty::set_default_colors(colors);
        // the remote screens' colours come from the theme: each is new
        for p in self.all_panes() {
            p.session
                .generation
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }
        // a theme that does not load falls back to Dark Default: say so once
        // per broken name instead of only logging it
        let wanted = if self.config.sync_os_theme && self.os_light {
            &self.config.theme_light
        } else {
            &self.config.theme
        };
        let error = rift_core::theme::try_load_by_name(wanted)
            .err()
            .map(|e| format!("Theme {wanted:?}: {e}; using Dark Default"));
        if error.is_some() && error != self.theme_error {
            self.toast_error(error.clone().unwrap_or_default());
        }
        self.theme_error = error;
        // Warp "window opacity": only the window ground goes translucent
        // (Theme::window_ground / Theme::ground); text, overlays, modals and
        // non-default cell colours stay opaque
        self.theme.ground_alpha = self.config.opacity();
        self.font_size = self.config.font_size();
        self.line_h = self.font_size * self.config.line_height();
        self.keymap = Keymap::from_overrides(&self.config.keybindings);
        self.apply_global_hotkey();
        // menu shortcut labels follow the keymap
        #[cfg(target_os = "macos")]
        if self.win.window.is_some()
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
        self.refresh_chip_sources();
        // save_transcripts off means gone, not just no longer updated —
        // however it was turned off (toggle, ↺, an edit of config.toml)
        if !self.config.save_transcripts {
            rift_core::transcript::remove_except(&[]);
            self.transcript_saved.clear();
        }
        self.each_window(|app| app.apply_window_config());
        self.retire_hotkey_window();
    }

    /// The part of `apply_config` each window takes on its own: its
    /// translucency, background image, its panes' editor and clipboard
    /// settings, the explorer; then a repaint.
    fn apply_window_config(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(w) = &self.win.window {
            crate::translucency::apply(w, self.config.blur());
            place_traffic_lights(w);
        }
        self.place_hotkey_window();
        self.apply_background_image();
        let vim = self.config.vim_mode;
        for tab in &mut self.win.tabs {
            for pane in &mut tab.panes {
                pane.editor.set_vim_enabled(vim);
            }
        }
        self.win
            .explorer
            .set_show_hidden(self.config.explorer_hidden_files);
        let clipboard = rift_pty::ClipboardAccess::from_config(&self.config.clipboard_access);
        for tab in &self.win.tabs {
            for pane in &tab.panes {
                pane.session.set_clipboard_access(clipboard);
            }
        }
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// (Re)register `global_hotkey` when it changed. Scripted runs never
    /// grab a system-wide key.
    fn apply_global_hotkey(&mut self) {
        let spec = self.config.global_hotkey.trim().to_string();
        if self.global_hotkey.as_ref().map(|h| h.spec()) == Some(spec.as_str())
            || (spec.is_empty() && self.global_hotkey.is_none())
        {
            return;
        }
        self.global_hotkey = None; // drop unregisters the old key
        if spec.is_empty() || !self.persist_session || self.win.window.is_none() || self.headless {
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
                self.toast_error(e);
            }
        }
    }

    /// The window, unless headless: minimize, maximize and fullscreen would
    /// put a hidden window on screen.
    fn shown_window(&self) -> Option<&Arc<Window>> {
        self.win.window.as_ref().filter(|_| !self.headless)
    }

    /// Global hotkey: bring Rift forward, or hide it when it is already
    /// the focused app (Warp hotkey window behavior).
    fn toggle_visibility(&mut self) {
        let Some(window) = self.win.window.clone().filter(|_| !self.headless) else {
            return;
        };
        #[cfg(target_os = "macos")]
        {
            if crate::hotkey::app_active() && self.win.focused {
                crate::hotkey::set_app_shown(false);
            } else {
                crate::hotkey::set_app_shown(true);
                window.set_minimized(false);
                window.focus_window();
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if self.win.focused && window.is_visible() != Some(false) {
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
        let (Some(gpu), Some(pipe)) = (&self.win.gpu, &mut self.win.bg_image) else {
            return;
        };
        match &self.theme.background_image {
            Some(img) => {
                let mut path = std::path::PathBuf::from(&img.path);
                if path.is_relative()
                    && let Some(dir) = rift_core::config::themes_dir()
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
            self.toast_as(
                crate::views::confirm::ToastKind::Error,
                msg,
                None,
                Some(ToastAction::OpenConfig),
            );
            return;
        }
        match self.config.save() {
            Ok(()) => self.win.settings.saved_at = Some(Instant::now()),
            Err(e) => {
                log::warn!("config save failed: {e}");
                self.toast_error(format!("Could not save settings: {e}"));
            }
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
                // fixed: its "not applied" / "not saved" errors are moot
                self.win
                    .toasts
                    .retain(|t| t.action != Some(ToastAction::OpenConfig));
                if !startup {
                    self.win.settings.saved_at = Some(Instant::now());
                }
            }
            Err(e) => {
                log::warn!("config.toml rejected: {e}");
                if startup {
                    self.config = Config::fallback();
                }
                self.toast_as(
                    crate::views::confirm::ToastKind::Error,
                    format!("config.toml not applied: {e}"),
                    None,
                    Some(ToastAction::OpenConfig),
                );
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
            use rift_pty::alacritty_terminal::grid::Dimensions;
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
        let (mx, my) = self.win.mouse;
        let count = match self.win.last_click {
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
        self.win.last_click = Some((now, (mx, my), count));
        count
    }

    /// Begin selection interaction on `pane` at the current mouse position.
    fn press_selection(&mut self, pane_id: PaneId, clicks: u8) {
        let shift = self.win.modifiers.shift_key();
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
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
        use rift_pty::alacritty_terminal::term::TermMode;
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
        let (mx, my) = self.win.mouse;
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
                self.win.pending_sel = Some(PendingSelection {
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
                self.win.pending_sel = Some(PendingSelection {
                    pane: pane_id,
                    abs: point.line.0 as i64 + scrolled as i64,
                    col: point.column,
                    side,
                    origin: (mx, my),
                    started: false,
                    block: self.win.modifiers.alt_key(),
                });
            }
        }
    }

    /// Select the block whose output holds absolute line `abs` of `pane_id`
    /// (nothing in a full-screen program or on prompt rows); clicking the
    /// selected block again clears the selection.
    fn select_block_at(&mut self, pane_id: PaneId, abs: i64) {
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
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
            pane.selected_block = (pane.selected_block != Some(block)).then_some(block);
        }
    }

    /// Extend the pending/live selection to the current mouse position.
    fn drag_selection(&mut self) {
        let Some(pending) = &mut self.win.pending_sel else {
            return;
        };
        let (mx, my) = self.win.mouse;
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
            .win
            .tabs
            .get_mut(self.win.active)
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
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// A live selection dragged above or below its pane's rows scrolls the
    /// pane toward the pointer, faster the farther out it is; the next frame's
    /// rows then carry the selection on (`drag_selection` on each tick).
    /// Returns whether the view moved.
    fn selection_edge_scroll(&mut self) -> bool {
        let Some(pending) = self.win.pending_sel.as_ref().filter(|p| p.started) else {
            return false;
        };
        let pane_id = pending.pane;
        let my = self.win.mouse.1;
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return false;
        };
        let (Some(grid), Some(first), Some(last)) =
            (pane.last_grid, pane.rows.first(), pane.rows.last())
        else {
            return false;
        };
        if pane.alt_screen {
            return false;
        }
        let (top, bottom) = (first.y, last.y + grid.line_h);
        let lines = |d: f32| (1 + (d / grid.line_h) as usize).min(SELECT_SCROLL_MAX);
        let max_off = pane.last_display_len.saturating_sub(grid.rows);
        let before = pane.view_offset;
        if my < top {
            pane.view_offset = (pane.view_offset + lines(top - my)).min(max_off);
        } else if my > bottom {
            pane.view_offset = pane.view_offset.saturating_sub(lines(my - bottom));
        }
        pane.view_offset != before
    }

    /// A selection held past its pane's top or bottom keeps scrolling, every
    /// `SELECT_SCROLL_EVERY` while it moves the view; returns when to wake next.
    fn tick_select_scroll(&mut self) -> Option<Instant> {
        if !self.win.pending_sel.as_ref().is_some_and(|p| p.started) {
            self.win.select_scrolled = None;
            return None;
        }
        let now = Instant::now();
        let due = self
            .win
            .select_scrolled
            .map_or(now, |t| t + SELECT_SCROLL_EVERY);
        if now < due {
            return Some(due);
        }
        self.drag_selection();
        if !self.selection_edge_scroll() {
            self.win.select_scrolled = None;
            return None;
        }
        self.win.select_scrolled = Some(now);
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
        Some(now + SELECT_SCROLL_EVERY)
    }

    /// The pane's display-line list as drawn (`build_display_list_filtered`
    /// with its block filter, soft-clear padding and bottom anchoring), and
    /// the blocks snapshot its headers index into.
    fn display_list(
        pane: &PaneState,
        hide_prompt: bool,
    ) -> (
        Vec<crate::views::terminal::DisplayLine>,
        Vec<rift_core::Block>,
    ) {
        use crate::views::terminal::{build_display_list_filtered, filter_hidden};
        use rift_pty::alacritty_terminal::grid::Dimensions;
        use rift_pty::alacritty_terminal::term::TermMode;
        let (blocks, clear_pad) = {
            let store = pane.session.blocks.lock().unwrap();
            (store.snapshot(), store.clear_pad())
        };
        let scrolled = pane
            .session
            .scrolled
            .load(std::sync::atomic::Ordering::Acquire);
        let (screen_lines, history, live_end, alt) = {
            let term = pane.session.term.lock();
            let g = term.grid();
            // the list as drawn: bottom-anchored ones leave the blank tail out
            let live_end = pane
                .last_grid
                .filter(|g| g.bottom_up)
                .map(|_| crate::views::terminal::live_end(&term, scrolled));
            let alt = term.mode().contains(TermMode::ALT_SCREEN);
            (g.screen_lines(), g.history_size() as u64, live_end, alt)
        };
        let hidden = filter_hidden(&pane.session, pane.filter.as_ref());
        let display = build_display_list_filtered(
            &blocks,
            scrolled,
            screen_lines,
            history,
            alt,
            hide_prompt,
            clear_pad,
            pane.last_grid.map_or(screen_lines, |g| g.rows),
            live_end,
            &hidden,
        );
        (display, blocks)
    }

    /// Display-list index of a block's header (current collapse state).
    fn header_display_index(
        pane: &PaneState,
        block: rift_core::BlockId,
        hide_prompt: bool,
    ) -> Option<(usize, usize)> {
        use crate::views::terminal::DisplayLine;
        let (display, blocks) = Self::display_list(pane, hide_prompt);
        let bi = blocks.iter().position(|b| b.id == block)?;
        let idx = display.iter().position(|d| *d == DisplayLine::Header(bi))?;
        Some((idx, display.len()))
    }

    /// Scroll so `block`'s header sits at the top of the viewport.
    fn jump_to_block(&mut self, pane_id: PaneId, block: rift_core::BlockId) {
        let editor_visible = self.win.editor_active;
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
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
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Display-list index of a block's last visible row (collapse-aware).
    fn block_end_display_index(
        pane: &PaneState,
        block: rift_core::BlockId,
        hide_prompt: bool,
    ) -> Option<(usize, usize)> {
        use crate::views::terminal::DisplayLine;
        let (display, blocks) = Self::display_list(pane, hide_prompt);
        let bi = blocks.iter().position(|b| b.id == block)?;
        let (start, end) = (blocks[bi].prompt_start, blocks[bi].end.unwrap_or(u64::MAX));
        let idx = display.iter().rposition(|d| match d {
            DisplayLine::Header(i) => *i == bi,
            DisplayLine::Grid(abs) => *abs >= start && *abs < end,
            DisplayLine::Pad => false,
        })?;
        Some((idx, display.len()))
    }

    /// Scroll so `block`'s last row sits at the bottom of the viewport.
    fn jump_to_block_end(&mut self, pane_id: PaneId, block: rift_core::BlockId) {
        let editor_visible = self.win.editor_active;
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
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
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Jump to the next/prev bookmarked block relative to the viewport top.
    fn jump_bookmark(&mut self, dir: i64) {
        let editor_visible = self.win.editor_active;
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        else {
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
        let mut best: Option<(usize, rift_core::BlockId)> = None;
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
    fn open_context_menu(&mut self, pane_id: PaneId, block: rift_core::BlockId, pos: (f32, f32)) {
        let Some(pane) = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.pane(pane_id))
        else {
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
        let rerun_ok = self.win.editor_active;
        self.win.context_menu = ContextMenuState {
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
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.pane(pane_id))
            .is_some_and(|p| p.has_selection);
        let split = self
            .win
            .tabs
            .get(self.win.active)
            .is_some_and(|t| t.is_split());
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
        self.win.context_menu = ContextMenuState {
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
        let markdown = !self.win.viewer.markdown.is_empty();
        let mut items = vec![
            MenuItem::new("Insert path", MenuAction::ViewerInsertPath),
            MenuItem::new("Copy path", MenuAction::ViewerCopyPath),
            MenuItem::new("Find in file", MenuAction::ViewerFind).sep(),
        ];
        if markdown {
            items.push(if self.win.viewer.rendered {
                MenuItem::new("Show raw", MenuAction::ViewerRendered(false))
            } else {
                MenuItem::new("Show rendered", MenuAction::ViewerRendered(true))
            });
        }
        items.push(MenuItem::new("Close", MenuAction::ViewerClose).sep());
        self.win.context_menu = ContextMenuState {
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
        let Some(t) = self.win.tabs.get(tab) else {
            return;
        };
        let n = self.win.tabs.len();
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
        self.win.context_menu = ContextMenuState {
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
                self.win.context_menu.step(1);
                true
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.win.context_menu.step(-1);
                true
            }
            Key::Named(NamedKey::Enter) => {
                if let Some(i) = self.win.context_menu.selected
                    && let Some(item) = self.win.context_menu.items.get(i)
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
        let pane_id = self.win.context_menu.pane;
        let block = self.win.context_menu.block;
        self.win.context_menu.close();
        let editor_active = self.win.editor_active;

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
                if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
                    tab.active_pane = pane_id;
                }
                return self.handle_action(AppAction::MaximizePane, event_loop);
            }
            MenuAction::MoveToNewTab => {
                let from = self.win.active;
                return self.drop_pane(pane_id, from, PaneDrop::NewTab);
            }
            MenuAction::ClosePane => {
                return self.request_close(CloseScope::Pane(pane_id), event_loop);
            }
            MenuAction::ViewerInsertPath => {
                if let Some(p) = self.win.viewer.path.clone() {
                    self.win.viewer.close();
                    self.drop_file(&p);
                }
                return;
            }
            MenuAction::ViewerCopyPath => {
                if let Some(p) = self.win.viewer.path.clone() {
                    self.copy_to_clipboard(p.to_string_lossy().into_owned());
                }
                return;
            }
            MenuAction::ViewerFind => {
                self.win.viewer.open_find();
                self.win.viewer_focus = true;
                return;
            }
            MenuAction::ViewerRendered(on) => {
                self.win.viewer.set_rendered(on);
                return;
            }
            MenuAction::ViewerClose => {
                self.win.viewer.close();
                return;
            }
            MenuAction::RenameTab(i) => {
                self.start_rename(i);
                return;
            }
            MenuAction::DuplicateTab(i) => {
                let cwd = self
                    .win
                    .tabs
                    .get(i)
                    .and_then(|t| t.active())
                    .and_then(|p| p.session.local_cwd())
                    .map(std::path::PathBuf::from);
                self.spawn_tab_at(cwd.filter(|p| p.is_dir()));
                // keep the copy next to its source
                let new = self.win.tabs.len() - 1;
                if i + 1 < new {
                    let tab = self.win.tabs.remove(new);
                    self.win.tabs.insert(i + 1, tab);
                    self.win.active = i + 1;
                }
                self.save_session();
                return;
            }
            MenuAction::ToggleSync(i) => {
                if let Some(t) = self.win.tabs.get_mut(i) {
                    t.sync_inputs = !t.sync_inputs;
                }
                return;
            }
            MenuAction::SetTabColor(i, c) => {
                if let Some(t) = self.win.tabs.get_mut(i) {
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
                for t in (i + 1..self.win.tabs.len()).rev() {
                    self.request_close(CloseScope::Tab(t), event_loop);
                    if self.win.confirm.is_some() {
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
                    .win
                    .tabs
                    .get_mut(self.win.active)
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
                .win
                .tabs
                .get_mut(self.win.active)
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
    fn toggle_block_filter(&mut self, pane_id: PaneId, block: rift_core::BlockId) {
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return;
        };
        if pane.filter.as_ref().is_some_and(|(b, _)| *b == block) {
            pane.filter = None;
            self.win.filter_editing = false;
            self.win.search.close();
            return;
        }
        pane.filter = Some((block, String::new()));
        pane.selected_block = Some(block);
        self.win.search.close();
        self.win.search.open = true;
        self.win.filter_editing = true;
        self.blink_epoch = Instant::now();
        self.jump_to_block(pane_id, block);
    }

    /// Command text of `block` (None when missing or empty).
    fn block_command(&self, pane_id: PaneId, block: rift_core::BlockId) -> Option<String> {
        let pane = self.win.tabs.get(self.win.active)?.pane(pane_id)?;
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
    fn selected_or_last_block(&self) -> Option<(PaneId, rift_core::BlockId)> {
        let pane = self.win.tabs.get(self.win.active)?.active()?;
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
        let editor_visible = self.win.editor_active;
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        else {
            return;
        };
        let eligible: Vec<rift_core::BlockId> = pane
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

    /// The detected URL or hyperlink under the pointer in `pane_id`.
    fn url_under_mouse(&self, pane_id: PaneId) -> Option<String> {
        let (mx, my) = self.win.mouse;
        self.win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.pane(pane_id))
            .and_then(|p| {
                p.url_hits
                    .iter()
                    .find(|(r, _)| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3])
                    .map(|(_, u)| u.clone())
            })
    }

    /// Only schemes that open documents, never script handlers.
    fn openable_url(url: &str) -> bool {
        let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
        matches!(
            scheme.as_deref(),
            Some("http" | "https" | "file" | "mailto" | "ftp")
        )
    }

    /// A plain click (no drag) on a link: open it (`link_click = "open"`),
    /// else show its tooltip at once, naming ⌘-click.
    fn click_link(&mut self, pane_id: PaneId, url: &str) {
        if self.url_under_mouse(pane_id).as_deref() != Some(url) {
            return;
        }
        if self.config.link_click.trim() == "open" {
            if Self::openable_url(url) {
                self.open_target(url);
            }
            return;
        }
        let now = Instant::now();
        self.win.tip_suppressed = false;
        if let Some((_, since)) = self.win.tip_hover.as_mut() {
            *since = now.checked_sub(crate::views::tooltip::DELAY).unwrap_or(now);
        }
    }

    /// A single plain press on a link: remembered until the release, which
    /// acts on it unless the press became a drag (a selection).
    fn press_link(&mut self, pane_id: PaneId, clicks: u8) {
        self.win.link_press = (clicks == 1 && !self.win.modifiers.shift_key())
            .then(|| self.url_under_mouse(pane_id).map(|u| (pane_id, u)))
            .flatten();
    }

    /// cmd+click on a detected URL opens it. Returns true when handled.
    fn try_open_url(&mut self, pane_id: PaneId) -> bool {
        if !self.win.modifiers.super_key() {
            return false;
        }
        let target = match self.url_under_mouse(pane_id) {
            Some(url) => {
                if !Self::openable_url(&url) {
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
            self.win.alt_sides,
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
                Ok(()) => self.toast_done(format!("Revealed {} (not run)", path.display())),
                Err(e) => self.toast_error(format!("Could not reveal {}: {e}", path.display())),
            }
        } else if let Err(e) = open_external(target) {
            self.toast_error(format!("Could not open {target}: {e}"));
        }
    }

    /// Open a file or folder with editor setting `choice`
    /// (`rift_core::editors`): the viewer pane, the OS opener, or an
    /// installed editor. A folder never goes to the viewer, and a missing
    /// editor falls back to the viewer (files) or the OS opener (folders)
    /// with a toast saying so.
    fn open_file_in(&mut self, path: &std::path::Path, choice: &str) {
        use rift_core::editors;
        let viewer_ok = path.is_file();
        match choice.trim() {
            editors::RIFT if viewer_ok => self.open_viewer(path),
            "" | editors::SYSTEM | editors::RIFT => {
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
                        self.toast_error(format!(
                            "Could not open {}: {program}: {e}",
                            editor.label
                        ));
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
                None => pane.session.local_cwd()?,
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
        if let Some(tab) = self.win.tabs.get(self.win.active) {
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
        for h in &self.win.hits {
            if let ClickTarget::Block(pane, block) = h.target {
                targets.push((h.rect, HintTarget::Block(pane, block)));
            }
        }
        if targets.is_empty() {
            self.toast("No links, paths or blocks on screen".into());
            return;
        }
        let labels = hint_labels(targets.len());
        self.win.hints = Some(HintState {
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
        let Some(state) = self.win.hints.as_mut() else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.win.hints = None;
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
        let copy = c.is_ascii_uppercase() || self.win.modifiers.shift_key();
        state.typed.push(c.to_ascii_lowercase());
        let typed = state.typed.clone();
        if let Some((_, _, target)) = state.items.iter().find(|(l, _, _)| *l == typed) {
            let target = target.clone();
            self.win.hints = None;
            match target {
                HintTarget::Open(t) if copy => {
                    if self.copy_to_clipboard(t) {
                        self.toast_done("Copied".into());
                    }
                }
                HintTarget::Open(t) => self.open_target(&t),
                HintTarget::Block(pane, block) => {
                    let mut sticky = false;
                    if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
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
            self.win.hints = None;
        }
    }

    /// ⇧ + block hint: copy that block's output.
    fn handle_action_copy_block(&mut self, pane: PaneId, block: rift_core::BlockId) {
        let text = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.pane(pane))
            .and_then(|p| terminal::block_output_text(&p.session, block));
        if let Some(text) = text
            && self.copy_to_clipboard(text)
        {
            self.toast_done("Copied block output".into());
        }
    }

    /// An existing file or directory named by the text under the pointer
    /// (resolved like `resolve_path`; `file:line:col` suffixes are dropped).
    fn path_under_mouse(&self, pane_id: PaneId) -> Option<String> {
        use crate::views::terminal::RowKind;
        let pane = self.win.tabs.get(self.win.active)?.pane(pane_id)?;
        let g = pane.last_grid?;
        let (mx, my) = self.win.mouse;
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
        let (mx, my) = self.win.mouse;
        let inside =
            |r: &[f32; 4]| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3];
        let hand = |on: bool| {
            if on {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            }
        };
        let overlay = if self.win.pane_drag.as_ref().is_some_and(|d| d.live) {
            Some(CursorIcon::Grabbing)
        } else if self.win.confirm.is_some() {
            Some(hand(self.win.confirm_rects.iter().any(inside)))
        } else if self.win.whats_new.open {
            let h = &self.win.whats_new_hits;
            Some(hand(inside(&h.close) || inside(&h.done)))
        } else if self.win.notifications.open {
            let h = &self.win.notifications_hits;
            Some(hand(
                inside(&h.close)
                    || inside(&h.mark_read)
                    || h.chips.iter().any(|(r, _)| inside(r))
                    || h.rows.iter().any(|(r, _)| inside(r))
                    || !inside(&h.card)
                        && self.win.hits.iter().any(|x| {
                            matches!(x.target, ClickTarget::ToggleNotifications)
                                && x.contains(mx, my)
                        }),
            ))
        } else if self.win.note_toast_hits.iter().any(|t| inside(&t.rect)) {
            Some(CursorIcon::Pointer)
        } else if let Some(t) = self.win.toast_hits.iter().find(|t| inside(&t.rect)) {
            // a hand over its button and ✕, an arrow over the message
            Some(hand(
                t.action.is_some_and(|r| inside(&r)) || t.close.is_some_and(|r| inside(&r)),
            ))
        } else if let Some(d) = self.win.tip_drawn.as_ref().filter(|d| {
            inside(&d.rect)
                && self
                    .win
                    .tip_shown
                    .as_ref()
                    .is_some_and(|t| t.action.is_some())
        }) {
            // an interactive hover card: a hand over its button
            Some(hand(d.action.as_ref().is_some_and(inside)))
        } else if let Some(ed) = &self.win.prompt_editor {
            Some(if ed.drag.as_ref().is_some_and(|d| d.live) {
                CursorIcon::Grabbing
            } else {
                hand(
                    self.win
                        .prompt_editor_layout
                        .hits
                        .iter()
                        .any(|(r, _)| inside(r)),
                )
            })
        } else if self.win.chip_menu.is_some() {
            Some(hand(self.win.chip_menu_rows.iter().any(|(r, _)| inside(r))))
        } else if self.win.context_menu.open {
            Some(hand(
                self.win
                    .context_menu_rects
                    .iter()
                    .zip(&self.win.context_menu.items)
                    .any(|(r, item)| item.enabled && inside(r)),
            ))
        } else if self.win.palette.open {
            Some(hand(
                self.win.palette_row_hits.iter().any(|(r, _)| inside(r))
                    || self.win.hits.iter().any(|h| {
                        matches!(h.target, ClickTarget::PaletteMode(_)) && inside(&h.rect)
                    }),
            ))
        } else if self.win.search.open && self.win.search_chips.iter().any(inside) {
            Some(CursorIcon::Pointer)
        } else if (!self.win.settings.open
            || (self.win.config_viewer && inside(&self.win.viewer_rect)))
            && let Some(icon) = self.panel_cursor(&inside)
        {
            Some(icon)
        } else {
            let hit = self.win.hits.iter().find(|h| h.contains(mx, my));
            match hit.map(|h| &h.target) {
                // a handle is the gutter between panes: tall = side by side
                Some(ClickTarget::HandleDrag(..)) => {
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
                    | ClickTarget::BlockMenu(..)
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
                    | ClickTarget::SettingsCheckUpdates
                    | ClickTarget::SettingsWhatsNew
                    | ClickTarget::SettingsPlaySound(_)
                    | ClickTarget::SettingsTestAlert(_)
                    | ClickTarget::SettingsRingBell
                    | ClickTarget::SettingsNotifyPermission
                    | ClickTarget::UpdateChip
                    | ClickTarget::RemoteChip
                    | ClickTarget::RemotePair
                    | ClickTarget::RemoteViewOnly
                    | ClickTarget::RemoteCopyLink
                    | ClickTarget::RemoteInstallTunnel
                    | ClickTarget::RemoteRelayRetry
                    | ClickTarget::RemoteRelaySettings
                    | ClickTarget::RemoteRevoke(_)
                    | ClickTarget::RemotePermission(..)
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
                Some(ClickTarget::RemoteRelay) => Some(CursorIcon::Text),
                _ => None,
            }
        };
        // links show the hand without ⌘ (a click shows or opens them);
        // file paths only with ⌘ held
        let over_link = overlay.is_none()
            && self.win.tabs.get(self.win.active).is_some_and(|t| {
                t.panes.iter().any(|p| {
                    p.url_hits.iter().any(|(r, _)| {
                        mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
                    })
                })
            });
        let over_path = overlay.is_none()
            && !over_link
            && self.win.modifiers.super_key()
            && self
                .pane_under_mouse()
                .is_some_and(|id| self.path_under_mouse(id).is_some());
        let icon = if let Some(icon) = overlay {
            icon
        } else if over_link || over_path {
            CursorIcon::Pointer
        } else if my > TAB_BAR_H
            && self.win.mouse.0 > self.rail_w()
            && !self.win.settings.open
            && !self.win.palette.open
        {
            CursorIcon::Text
        } else {
            CursorIcon::Default
        };
        if self.win.cursor_icon != icon {
            self.win.cursor_icon = icon;
            window.set_cursor(icon);
        }
    }

    /// Selected text of the active pane, if any.
    fn selection_text(&self) -> Option<String> {
        let pane = self.win.tabs.get(self.win.active)?.active()?;
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
                    self.toast_error(format!("Clipboard unavailable: {e}"));
                }
            }
        }
        self.clipboard.as_mut()
    }

    /// Copy `text`; `false` (after telling the user) when it failed.
    fn copy_to_clipboard(&mut self, text: String) -> bool {
        if self.headless {
            self.headless_clipboard = text;
            self.headless_files.clear();
            return true;
        }
        let Some(cb) = self.clipboard() else {
            return false;
        };
        match cb.set_text(text) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("clipboard set failed: {e}");
                self.toast_error(format!("Could not copy: {e}"));
                false
            }
        }
    }

    fn paste_from_clipboard(&mut self) {
        if self.headless {
            let text = input::pasted_files_text(&self.headless_files)
                .unwrap_or_else(|| self.headless_clipboard.clone());
            self.paste_text(&text);
            return;
        }
        let Some(cb) = self.clipboard() else {
            return;
        };
        // files copied in the file manager: their paths (the clipboard's
        // text is only the file name, its image only the file's icon)
        if let Some(text) = cb
            .get()
            .file_list()
            .ok()
            .and_then(|paths| input::pasted_files_text(&paths))
        {
            self.paste_text(&text);
            return;
        }
        let text = match cb.get_text() {
            Ok(text) => text,
            // empty or non-text clipboard (an image): an empty paste, which
            // programs like Claude Code answer by reading the image
            Err(arboard::Error::ContentNotAvailable) => String::new(),
            Err(e) => {
                log::warn!("clipboard read failed: {e}");
                self.toast_error(format!("Could not paste: {e}"));
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
            // only a program reading bracketed pastes sees an empty one
            let relay_edit =
                self.win.settings.open && self.win.settings.remote.relay_edit.is_some();
            if !relay_edit
                && !self.win.editor_active
                && let Some(id) = self
                    .win
                    .tabs
                    .get(self.win.active)
                    .and_then(|t| t.active())
                    .map(|p| p.session.id)
            {
                self.paste_raw(id, text);
            }
            return;
        }
        // the relay field while it is edited (one line, no controls)
        if self.win.settings.open
            && let Some(draft) = self.win.settings.remote.relay_edit.as_mut()
        {
            let clean: String = text.trim().chars().filter(|c| !c.is_control()).collect();
            if draft.len() + clean.len() <= MAX_RELAY_URL {
                draft.push_str(&clean);
            }
            if let Some(w) = &self.win.window {
                w.request_redraw();
            }
            return;
        }
        let editor_active = self.win.editor_active;
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        else {
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
            self.win
                .tabs
                .iter()
                .find_map(|tab| tab.pane(t))
                .is_some_and(|p| !bracketed_paste_on(p))
        });
        if unbracketed && self.config.confirm_multiline_paste && input::paste_submits_lines(text) {
            let commands = text.lines().map(str::to_string).collect();
            self.win.confirm = Some(ConfirmClose {
                opened: Instant::now(),
                action: ConfirmAction::Paste {
                    pane: id,
                    text: text.to_string(),
                },
                commands,
            });
            if let Some(w) = &self.win.window {
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
                let bracketed = bracketed_paste_on(p);
                if text.is_empty() && !bracketed {
                    continue;
                }
                let bytes = input::paste_bytes(text, bracketed);
                p.view_offset = 0;
                p.session.write(&bytes);
            }
        }
    }

    /// Files dropped on the window: their shell-quoted paths are pasted.
    fn drop_file(&mut self, path: &std::path::Path) {
        let text = input::paths_text(&[path.to_path_buf()]);
        self.paste_text(&text);
    }

    /// Re-run the search over the active pane and jump near the latest match.
    fn refresh_search(&mut self, jump: bool) {
        let Some(pane) = self.win.tabs.get(self.win.active).and_then(|t| t.active()) else {
            self.win.search.set_matches(terminal::Found::default());
            return;
        };
        // only the rows the pane draws: a prompt echo (shown as the block
        // header), collapsed output and filtered-out lines would count
        // matches nobody can see
        let found = match self.win.search.matcher() {
            Some(m) => {
                let shown = Self::shown_rows(pane, self.win.editor_active);
                terminal::collect_matches(&pane.session, &shown, &m, search::MATCH_CAP)
            }
            None => terminal::Found::default(),
        };
        self.win.search.set_matches(found);
        if jump && !self.win.search.matches.is_empty() {
            self.jump_to_match();
        }
    }

    fn jump_to_match(&mut self) {
        let Some((abs, _)) = self.win.search.current_anchor() else {
            return;
        };
        let editor_visible = self.win.editor_active;
        if let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        {
            use crate::views::terminal::DisplayLine;
            // view_offset counts display lines, not grid rows: headers,
            // collapsed output and the block filter make them differ
            let (display, _) = Self::display_list(pane, editor_visible);
            let Some(idx) = display.iter().position(|d| *d == DisplayLine::Grid(abs)) else {
                return;
            };
            let rows = pane.last_grid.map_or(24, |g| g.rows);
            let max_off = display.len().saturating_sub(rows);
            // the match a dozen rows above the bottom (or as low as it goes)
            let below = display.len() - 1 - idx;
            pane.view_offset = below.saturating_sub(12).min(max_off);
        }
    }

    /// Grid rows the pane draws, oldest first (see [`Self::display_list`]).
    fn shown_rows(pane: &PaneState, hide_prompt: bool) -> Vec<u64> {
        use crate::views::terminal::DisplayLine;
        Self::display_list(pane, hide_prompt)
            .0
            .into_iter()
            .filter_map(|d| match d {
                DisplayLine::Grid(abs) => Some(abs),
                _ => None,
            })
            .collect()
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
                "c" => self.win.search.case_sensitive = !self.win.search.case_sensitive,
                "r" => self.win.search.regex = !self.win.search.regex,
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
                    self.win.search.prev();
                } else {
                    self.win.search.next();
                }
                self.jump_to_match();
                self.blink_epoch = Instant::now();
            }
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.win.search.close();
                if let Some(pane) = self
                    .win
                    .tabs
                    .get_mut(self.win.active)
                    .and_then(|t| t.active_mut())
                {
                    pane.view_offset = 0;
                }
            }
            Key::Named(NamedKey::Enter) => {
                if shift {
                    self.win.search.prev();
                } else {
                    self.win.search.next();
                }
                self.jump_to_match();
            }
            Key::Named(NamedKey::Backspace) => {
                self.win.search.query.pop();
                self.refresh_search(true);
            }
            Key::Named(NamedKey::Space) => {
                self.win.search.query.push(' ');
                self.refresh_search(true);
            }
            _ => {
                if let Some(text) = &event.text
                    && !text.is_empty()
                    && !text.chars().any(|c| c.is_control())
                {
                    self.win.search.query.push_str(text);
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
        let action = input::app_action(event, self.win.modifiers, &self.keymap);
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        else {
            self.win.filter_editing = false;
            self.win.search.close();
            return;
        };
        let Some((block, query)) = pane.filter.as_mut() else {
            self.win.filter_editing = false;
            self.win.search.close();
            return;
        };
        let block = *block;
        match &event.logical_key {
            _ if action == Some(AppAction::FilterBlock) => {
                pane.filter = None;
                self.win.filter_editing = false;
                self.win.search.close();
            }
            Key::Named(NamedKey::Escape) => {
                pane.filter = None;
                self.win.filter_editing = false;
                self.win.search.close();
            }
            Key::Named(NamedKey::Enter) => {
                if query.is_empty() {
                    pane.filter = None;
                }
                self.win.filter_editing = false;
                self.win.search.close();
            }
            Key::Named(NamedKey::Backspace) => {
                query.pop();
            }
            Key::Named(NamedKey::Space) => query.push(' '),
            _ => {
                if !self.win.modifiers.super_key()
                    && !self.win.modifiers.control_key()
                    && let Some(text) = &event.text
                    && !text.chars().any(|c| c.is_control())
                {
                    query.push_str(text);
                }
            }
        }
        let pane_id = pane.session.id;
        if self.win.filter_editing {
            // keep the filtered block's header in view while narrowing
            self.jump_to_block(pane_id, block);
        }
        self.blink_epoch = Instant::now();
    }

    /// Same list (and order) `SwitchTheme(i)` indexes: builtins, then user
    /// themes — the palette used to list builtins only.
    fn theme_names() -> Vec<String> {
        rift_core::theme::available_themes()
    }

    fn refresh_palette(&mut self) {
        use crate::views::palette::PaletteMode;
        let pane_history = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .map(|p| (*p.editor.history).clone())
            .unwrap_or_default();
        let (items, recent) = match self.win.palette.mode {
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
                    .win
                    .pipe_target
                    .as_ref()
                    .map(|t| t.2.as_str())
                    .unwrap_or_default();
                self.win.palette_results = palette::pipe_items(&self.win.palette.query, output);
                self.win.palette.selected = 0;
                return;
            }
        };
        self.win.palette_results = palette::rank(&items, &recent, &self.win.palette.query);
        if self.win.palette.selected >= self.win.palette_results.len() {
            self.win.palette.selected = self.win.palette_results.len().saturating_sub(1);
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
        for (ti, tab) in self.win.tabs.iter().enumerate() {
            let panes = if tab.is_split() { tab.panes.len() } else { 1 };
            items.push(row(
                format!("{} — tab {}", tab.display_title(), ti + 1),
                if ti == self.win.active {
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
        for (ti, tab) in self.win.tabs.iter().enumerate() {
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
        for (ti, tab) in self.win.tabs.iter().enumerate() {
            for p in &tab.panes {
                for b in p.session.blocks.lock().unwrap().snapshot() {
                    if let rift_core::BlockState::Done { exit, .. } = b.state
                        && !b.command.trim().is_empty()
                    {
                        blocks.push((b.id, ti, p.session.id, b.command, exit, b.cwd));
                    }
                }
            }
        }
        // tabs of this process's other windows (tab indices count across
        // windows, as the control API's do)
        let here = self.window_index();
        let mut first = 0;
        for (wi, w) in self.windows_in_order().into_iter().enumerate() {
            if wi != here {
                for (ti, t) in w.tabs.iter().enumerate() {
                    let cwd = t
                        .active()
                        .and_then(|p| p.session.cwd.lock().unwrap().clone())
                        .map(|c| editor::shorten_home(&c))
                        .unwrap_or_default();
                    let window = if w.dropdown.is_some() {
                        "hotkey window".to_string()
                    } else {
                        format!("window {}", wi + 1)
                    };
                    items.push(row(
                        format!("{} — {window}", t.display_title()),
                        cwd,
                        Section::Tabs,
                        PaletteAction::FocusWindow(std::process::id(), first + ti, t.active_pane),
                    ));
                }
            }
            first += w.tabs.len();
        }
        // tabs of other Rift processes (their control sockets)
        for (pid, _, tabs) in &self.win.other_windows {
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
        self.win.focused
            && self
                .win
                .tabs
                .get(self.win.active)
                .is_some_and(|t| t.pane(pane).is_some() && t.zoomed.is_none_or(|z| z == pane))
    }

    /// Coding agent running in a pane: the running block's command, else the
    /// foreground job (durable sessions, shells without integration).
    fn pane_agent(p: &PaneState) -> Option<rift_core::notifications::Agent> {
        use rift_core::notifications::Agent;
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
        let pane = self.win.tabs.iter().find_map(|t| t.pane(a.pane));
        a.branch = pane
            .and_then(|p| p.git.as_ref())
            .map(|g| g.branch.clone())
            .filter(|b| !b.is_empty());
        if a.agent.is_none() {
            a.agent = pane.and_then(Self::pane_agent);
        }
        a.inputs = pane.map_or(0, |p| p.session.inputs());
        self.next_attention_id += 1;
        a.id = self.next_attention_id;
        let id = a.id;
        self.attention.push(a);
        if self.attention.len() > ATTENTION_CAP {
            let extra = self.attention.len() - ATTENTION_CAP;
            self.attention.drain(..extra);
        }
        // every window's inbox button counts it
        self.each_window(|app| {
            if let Some(w) = &app.win.window {
                w.request_redraw();
            }
        });
        id
    }

    /// What block `block` of `pane` did, for its notification: command,
    /// exit, duration and last output line.
    fn command_facts(
        &self,
        pane: PaneId,
        block: rift_core::BlockId,
    ) -> Option<crate::notify::CommandFacts> {
        let p = self.win.tabs.iter().find_map(|t| t.pane(pane))?;
        let (command, exit, duration) = {
            let blocks = p.session.blocks.lock().unwrap();
            let b = blocks.iter().find(|b| b.id == block)?;
            let (exit, duration) = match b.state {
                BlockState::Done { exit, duration } => (exit, Some(duration)),
                _ => (None, None),
            };
            (b.command.clone(), exit, duration)
        };
        // a failure's reason is usually its last line
        let last_line = exit.filter(|e| *e != 0).and_then(|_| {
            let (text, _) = terminal::block_output_tail(&p.session, block)?;
            let line = text.lines().rev().map(str::trim).find(|l| !l.is_empty())?;
            Some(line.chars().take(160).collect::<String>())
        });
        Some(crate::notify::CommandFacts {
            command,
            exit,
            duration,
            last_line,
        })
    }

    /// The desktop notification for entry `a`: its wording, and the tab,
    /// folder and branch of its pane.
    fn desktop_alert(
        &self,
        a: &Attention,
        command: Option<&crate::notify::CommandFacts>,
    ) -> crate::notify::Alert {
        let tab = self.win.tabs.iter().find(|t| t.pane(a.pane).is_some());
        let tab_title = tab.map(|t| t.display_title()).unwrap_or_default();
        let cwd = tab
            .and_then(|t| t.pane(a.pane))
            .and_then(|p| p.session.cwd.lock().unwrap().clone())
            .map(|c| editor::shorten_home(&c));
        let mut alert = crate::notify::compose(&crate::notify::Facts {
            kind: a.kind,
            agent: a.agent,
            title: &a.title,
            detail: &a.detail,
            command,
            tab: &tab_title,
            cwd: cwd.as_deref(),
            branch: a.branch.as_deref(),
        });
        alert.pane = a.pane;
        alert.attention = a.id;
        alert
    }

    /// Read the desktop-notification permission again (Settings shows it;
    /// it changes in System Settings while Rift runs).
    fn refresh_notify_permission(&self) {
        let proxy = std::sync::Mutex::new(self.proxy.clone());
        crate::notify::refresh_permission(move || {
            if let Ok(p) = proxy.lock() {
                let _ = p.send_event(UserEvent::NotifyPermission);
            }
        });
    }

    /// A click on a desktop notification (or its Show / Rerun): Rift comes
    /// forward on the pane; Rerun runs the failed command again there.
    fn on_notification_response(&mut self, r: crate::notify::Response) {
        // the pane's window: a hidden drop-down window drops down first
        self.reveal_hotkey_window();
        if let Some(w) = &self.win.window {
            w.set_minimized(false);
            w.focus_window();
        }
        let command = self
            .attention
            .iter()
            .find(|a| a.id == r.attention)
            .map(|a| a.title.clone());
        if r.attention != 0 {
            self.show_attention(r.attention);
        }
        if r.rerun
            && let Some(cmd) = command.filter(|c| !c.trim().is_empty())
        {
            // only at the shell's prompt: typed into a running program it
            // would be that program's input
            let at_prompt = self
                .find_tab_of_pane(r.pane)
                .and_then(|(_, t)| t.pane(r.pane))
                .is_some_and(|p| {
                    !p.session
                        .term
                        .lock()
                        .mode()
                        .contains(rift_pty::alacritty_terminal::term::TermMode::ALT_SCREEN)
                        && matches!(
                            p.session.blocks.lock().unwrap().last().map(|b| &b.state),
                            Some(BlockState::Prompt)
                        )
                });
            if self.test_alerts.contains(&r.attention) {
                // a Try it sample: its command is made up
                log::info!("notify: rerun of a test alert (not run): {cmd}");
                self.toast(format!(
                    "Rerun works: a real failed command would run again here ({cmd})"
                ));
            } else if at_prompt {
                log::info!("notify: rerun in pane {}: {cmd}", r.pane);
                self.submit_to(r.pane, &cmd);
            } else {
                log::info!("notify: rerun skipped, pane {} is busy", r.pane);
                self.toast("Not rerun: a command is still running in that pane".into());
            }
        }
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Inbox entry plus, for an `event` whose pane is out of view, the
    /// alerts `[notifications]` allows: a toast in the focused window while
    /// any Rift window is focused (the pane's own window may be in the
    /// background), a desktop notification otherwise, and the event's sound.
    fn raise(&mut self, mut a: Attention, event: Option<rift_core::notifications::Event>) {
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
        let focused = if self.win.focused {
            Some(&mut *self.win)
        } else {
            self.parked.iter_mut().find(|w| w.focused).map(|w| &mut **w)
        };
        if let Some(fw) = focused {
            if cfg.toasts {
                fw.note_toasts.insert(0, (id, Instant::now()));
                fw.note_toasts
                    .truncate(crate::views::notifications::MAX_TOASTS);
                if let Some(w) = &fw.window {
                    w.request_redraw();
                }
            }
        } else {
            let a = a.clone();
            let facts = a.block.and_then(|b| self.command_facts(a.pane, b));
            crate::notify::send_alert(&self.desktop_alert(&a, facts.as_ref()));
        }
        if cfg.play_sound
            && let Some(path) = rift_core::notifications::resolve_sound(
                cfg.sound_setting(event),
                event,
                &rift_core::notifications::sound_dirs(),
            )
        {
            crate::notify::play(&path);
        }
        self.win.a11y_announcement = Some(format!("{title}: {body}"));
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// ▷ beside a sound select, or a pick in one: play what it now plays (a
    /// preview: it ignores *Play notification sounds*). Says so when there is
    /// nothing to hear.
    fn play_sound_setting(&mut self, id: &str) {
        use rift_core::notifications::{Event, resolve_sound, sound_dirs};
        let n = &self.config.notifications;
        // the main Sound previews as Finished (the alert it covers most)
        let (event, setting) = match id {
            "notifications.sound" => (Event::Finished, n.sound.clone()),
            "notifications.sound_finished" => {
                (Event::Finished, n.sound_setting(Event::Finished).into())
            }
            "notifications.sound_failed" => (Event::Failed, n.sound_setting(Event::Failed).into()),
            "notifications.sound_attention" => {
                (Event::Attention, n.sound_setting(Event::Attention).into())
            }
            _ => return,
        };
        match resolve_sound(&setting, event, &sound_dirs()) {
            Some(path) => crate::notify::play(&path),
            None if setting.trim().eq_ignore_ascii_case("none") => {
                self.toast("No sound: this one is set to None".into())
            }
            None => self.toast_error(format!(
                "Sound not found: {}",
                crate::views::settings::sound_label(&setting)
            )),
        }
    }

    /// Settings › Notifications › Try it: a sample alert of `t`'s kind as a
    /// real one shows (a toast, or the desktop banner), with its sound while
    /// sounds are on. It goes in the inbox already read.
    fn test_alert(&mut self, t: crate::state::TestAlert) {
        use rift_core::notifications::{Event, resolve_sound, sound_dirs};
        let pane = self
            .win
            .tabs
            .get(self.win.active)
            .map_or(0, |tab| tab.active_pane);
        let (kind, title, detail) = match t.event {
            Event::Finished => (
                AttentionKind::Finished,
                "cargo build --release",
                "Test · completed in 1m 12s",
            ),
            Event::Failed => (AttentionKind::Failed, "npm test", "Test · exit 1 after 8s"),
            Event::Attention => (
                AttentionKind::Request,
                "Test · approve running npm test?",
                "",
            ),
        };
        let mut a = Attention::new(pane, kind, title.into(), detail.into());
        a.read = true;
        let id = self.push_attention(a);
        self.test_alerts.insert(id);
        let (title, body) = self
            .attention
            .iter()
            .find(|a| a.id == id)
            .map(Attention::desktop_text)
            .unwrap_or_default();
        log::info!(
            "test alert {:?} ({}): {title} — {body}",
            t.event,
            if t.desktop { "desktop" } else { "toast" }
        );
        if t.desktop {
            // a sample command for the command kinds, as a real one reads
            let facts = match t.event {
                Event::Finished => Some(crate::notify::CommandFacts {
                    command: "cargo build --release".into(),
                    exit: Some(0),
                    duration: Some(Duration::from_secs(72)),
                    last_line: None,
                }),
                Event::Failed => Some(crate::notify::CommandFacts {
                    command: "npm test".into(),
                    exit: Some(1),
                    duration: Some(Duration::from_secs(8)),
                    last_line: Some("Test · 1 failing: checkout › applies the coupon".into()),
                }),
                Event::Attention => None,
            };
            if let Some(a) = self.attention.iter().find(|a| a.id == id).cloned() {
                crate::notify::send_alert(&self.desktop_alert(&a, facts.as_ref()));
            }
        } else {
            self.win.note_toasts.insert(0, (id, Instant::now()));
            self.win
                .note_toasts
                .truncate(crate::views::notifications::MAX_TOASTS);
        }
        let n = &self.config.notifications;
        if n.play_sound
            && let Some(path) = resolve_sound(n.sound_setting(t.event), t.event, &sound_dirs())
        {
            crate::notify::play(&path);
        }
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// A Settings select's options as (config value, label), and the
    /// current value.
    fn select_options(&self, id: &str) -> (Vec<(String, String)>, String) {
        use rift_core::notifications::{available_sounds, sound_dirs};
        let n = &self.config.notifications;
        match id {
            "theme" | "theme_light" => {
                let current = if id == "theme" {
                    &self.config.theme
                } else {
                    &self.config.theme_light
                };
                let names = rift_core::theme::available_themes();
                let labels = rift_core::theme::picker_labels(&names);
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
                use rift_core::editors;
                let current = if id == "link_editor" {
                    &self.config.link_editor
                } else {
                    &self.config.file_editor
                };
                let mut options: Vec<(String, String)> = [editors::SYSTEM, editors::RIFT]
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
        self.win.settings.dropdown = Some(crate::views::settings::Dropdown {
            id,
            options,
            highlight: current.unwrap_or(0),
            current,
        });
    }

    /// Set select `id` to `value`; a sound plays so it can be heard.
    fn set_select(&mut self, id: &str, value: String) {
        let n = &mut self.config.notifications;
        match id {
            "theme" => self.config.theme = value,
            "theme_light" => self.config.theme_light = value,
            "font_family" => self.config.font_family = value,
            "link_editor" => self.config.link_editor = value,
            "file_editor" => self.config.file_editor = value,
            "notifications.sound_finished" => n.sound_finished = value,
            "notifications.sound_failed" => n.sound_failed = value,
            "notifications.sound_attention" => n.sound_attention = value,
            "notifications.sound" => n.sound = value,
            _ => return,
        }
        if id.starts_with("notifications.sound") {
            self.play_sound_setting(id);
        }
        self.save_config();
        self.apply_config();
    }

    /// Drop expired notification toasts; the one under the mouse restarts
    /// its time. True when one went.
    fn prune_note_toasts(&mut self) -> bool {
        let ttl = self.config.notifications.toast_duration();
        let now = Instant::now();
        let (mx, my) = self.win.mouse;
        let hits = &self.win.note_toast_hits;
        for (id, since) in &mut self.win.note_toasts {
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
        let before = self.win.note_toasts.len();
        let notes = &self.win.app_notes;
        self.win.note_toasts.retain(|(id, since)| {
            notes.iter().any(|n| n.id == *id && n.sticky) || now.duration_since(*since) < ttl
        });
        let toasts = &self.win.note_toasts;
        self.win
            .app_notes
            .retain(|n| toasts.iter().any(|(id, _)| *id == n.id));
        before != self.win.note_toasts.len()
    }

    /// Show one of Rift's own notices as a notification toast (a desktop
    /// notification instead while another app is in front); returns its id.
    fn app_note(
        &mut self,
        title: String,
        detail: String,
        action: AppNoteAction,
        sticky: bool,
    ) -> u64 {
        log::info!("notice: {title} — {detail}");
        self.next_attention_id += 1;
        let id = self.next_attention_id;
        if !self.win.focused && sticky && self.config.notifications.enabled {
            crate::notify::send(&title, &detail);
        }
        self.win.a11y_announcement = Some(if detail.is_empty() {
            title.clone()
        } else {
            format!("{title}: {detail}")
        });
        // in-window cards off (Settings › Notifications): an update that is
        // ready still shows in the tab bar's chip; the rest is a status toast
        if !self.config.notifications.toasts {
            let text = if detail.is_empty() {
                title
            } else {
                format!("{title}: {detail}")
            };
            match (sticky, action) {
                (true, AppNoteAction::Update) => {}
                // a failed update stays until dismissed
                (true, _) => self.toast_error(text),
                _ => self.toast(text),
            }
            return id;
        }
        self.win.app_notes.push(AppNote {
            id,
            title,
            detail,
            action,
            sticky,
        });
        self.win.note_toasts.insert(0, (id, Instant::now()));
        // a sticky notice is never pushed out by the ones after it
        let notes = &self.win.app_notes;
        let max = crate::views::notifications::MAX_TOASTS;
        while self.win.note_toasts.len() > max {
            let drop = self
                .win
                .note_toasts
                .iter()
                .rposition(|(id, _)| !notes.iter().any(|n| n.id == *id && n.sticky))
                .unwrap_or(self.win.note_toasts.len() - 1);
            self.win.note_toasts.remove(drop);
        }
        let toasts = &self.win.note_toasts;
        self.win
            .app_notes
            .retain(|n| toasts.iter().any(|(t, _)| *t == n.id));
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
        id
    }

    /// Drop Rift's own notices that `action` names (an update that was
    /// installed or replaced).
    fn drop_app_notes(&mut self, action: AppNoteAction) {
        let gone: Vec<u64> = self
            .win
            .app_notes
            .iter()
            .filter(|n| n.action == action)
            .map(|n| n.id)
            .collect();
        self.win.app_notes.retain(|n| n.action != action);
        self.win.note_toasts.retain(|(id, _)| !gone.contains(id));
    }

    /// Entries whose pane you are looking at count as read; their toasts go.
    fn mark_visible_read(&mut self) {
        if !self.win.focused {
            return;
        }
        let Some(tab) = self.win.tabs.get(self.win.active) else {
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
        if changed || !self.win.note_toasts.is_empty() {
            let attention = &self.attention;
            let notes = &self.win.app_notes;
            // a Try it sample is read from the start, yet its toast is the point
            let tests = &self.test_alerts;
            self.win.note_toasts.retain(|(id, _)| {
                notes.iter().any(|n| n.id == *id)
                    || tests.contains(id)
                    || attention.iter().any(|a| a.id == *id && !a.read)
            });
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
        if !rift_core::notifications::looks_like_password_prompt(&line) {
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
            .then_some(rift_core::notifications::Event::Attention);
        self.raise(a, event);
    }

    /// Commands that finished since last time: out of view ones (and every
    /// failure) go to the attention center.
    fn record_completions(&mut self, pane: PaneId) {
        let in_view = self.pane_in_view(pane);
        let Some(p) = self
            .win
            .tabs
            .iter()
            .flat_map(|t| &t.panes)
            .find(|p| p.session.id == pane)
        else {
            return;
        };
        let seen = self.attention_seen.get(&pane).copied();
        let fresh: Vec<(rift_core::BlockId, String, Option<i32>, Duration, bool)> = p
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .filter(|b| seen.is_none_or(|s| b.id > s))
            .filter_map(|b| match b.state {
                BlockState::Done { exit, duration } => Some((
                    b.id,
                    b.command.clone(),
                    exit,
                    duration,
                    b.restored || b.replayed,
                )),
                _ => None,
            })
            .collect();
        let Some(newest) = fresh.last().map(|f| f.0) else {
            return;
        };
        self.attention_seen.insert(pane, newest);
        // a finished command may have changed files: reload the panels
        self.request_review();
        use rift_core::notifications::Event;
        let long = Duration::from_secs(self.config.notify_threshold_secs);
        let long_on = self.config.notify_on_completion;
        let failed_on = self.config.notifications.command_failed;
        // restored or replayed: it finished before this window, nothing to say
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
            a.agent = rift_core::notifications::Agent::from_command(&a.title);
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
        let filter = self.win.notifications.filter;
        self.notification_entries()
            .into_iter()
            .filter(|e| filter.matches(e))
            .collect()
    }

    fn open_notifications(&mut self) {
        self.win.notifications.open();
        self.win.palette.open = false;
        self.win.context_menu.close();
    }

    fn close_notifications(&mut self) {
        self.win.notifications.open = false;
    }

    fn mark_all_read(&mut self) {
        for a in &mut self.attention {
            a.read = true;
        }
        self.win.note_toasts.clear();
    }

    /// Open an entry: mark it read, drop its toast, go to its pane (and block).
    fn show_attention(&mut self, id: u64) {
        let Some(a) = self.attention.iter_mut().find(|a| a.id == id) else {
            return;
        };
        a.read = true;
        let a = a.clone();
        self.win.note_toasts.retain(|(t, _)| *t != id);
        // the pane may be in another window: that one comes forward
        if self.win.tabs.iter().all(|t| t.pane(a.pane).is_none()) && self.enter_pane_window(a.pane)
        {
            self.focus_pane_here(a.pane);
        }
        // never a pane of a hidden drop-down window: typing would land unseen
        self.reveal_hotkey_window();
        self.win.settings.open = false;
        let Some(ti) = self.win.tabs.iter().position(|t| t.pane(a.pane).is_some()) else {
            self.toast("That pane is closed".into());
            return;
        };
        self.win.active = ti;
        let tab = &mut self.win.tabs[ti];
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
        self.win
            .detached
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
        let Some(pane) = self.win.tabs.get(self.win.active).and_then(|t| t.active()) else {
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
        self.win.pipe_target = Some((pane.session.id, block, output));
        self.open_palette(crate::views::palette::PaletteMode::PipeOutput);
    }

    /// Run the pipe modal's command with the block output as its stdin.
    fn run_pipe(&mut self) {
        let command = self.win.palette.query.trim().to_string();
        let Some((pane, _, output)) = self.win.pipe_target.take() else {
            return;
        };
        if command.is_empty() {
            return;
        }
        let Some(file) = write_pipe_file(&output) else {
            self.toast_error("Could not write the output to a temporary file".into());
            return;
        };
        let shell = rift_core::shell_integration::shell_name(self.config.shell.as_deref());
        let line = palette::pipe_command(&command, &file.to_string_lossy(), &shell);
        self.submit_to(pane, &line);
    }

    /// Keys go to the viewer: it is open in this tab and focused.
    fn viewer_focused(&self) -> bool {
        self.win.viewer_focus
            && self.win.viewer.is_open()
            && if self.win.config_viewer {
                self.win.settings.open
            } else {
                // a shell tab's file viewer sits under the Settings page
                !self.win.settings.open
                    && self
                        .win
                        .tabs
                        .get(self.win.active)
                        .is_some_and(|t| t.root.contains(VIEWER_PANE))
            }
    }

    /// Keep the viewer state and its pane leaf in step: a closed viewer
    /// leaves the split tree, a leaf whose tab went away closes the viewer.
    fn sync_viewer_pane(&mut self) {
        // config.toml beside Settings: open while the Settings tab is
        if self.win.config_viewer {
            if !self.win.settings.tab {
                self.win.viewer.close();
            }
            if !self.win.viewer.is_open() {
                self.win.config_viewer = false;
            }
        }
        if self.win.viewer.is_open() {
            if !self.win.config_viewer
                && !self.win.tabs.iter().any(|t| t.root.contains(VIEWER_PANE))
            {
                self.win.viewer.close();
            }
        } else {
            for tab in &mut self.win.tabs {
                if tab.root.contains(VIEWER_PANE) {
                    tab.root.remove(VIEWER_PANE);
                }
            }
        }
        if !self.win.viewer.is_open() {
            self.win.viewer_focus = false;
        }
    }

    /// Open `path` in the viewer pane: the tab's leftmost column, next to the
    /// explorer, reused when this tab already has one.
    /// Settings › Open config.toml: the file beside the Settings page, in
    /// the Settings tab (no terminal tab's split changes).
    fn open_config_viewer(&mut self, path: &std::path::Path) {
        for tab in &mut self.win.tabs {
            if tab.root.contains(VIEWER_PANE) {
                tab.root.remove(VIEWER_PANE);
            }
        }
        self.win.config_viewer = true;
        self.win.viewer_focus = true;
        let proxy = self.proxy.clone();
        self.win.viewer.open(path, move |loaded| {
            let _ = proxy.send_event(UserEvent::ViewerLoaded(Box::new(loaded)));
        });
    }

    fn open_viewer(&mut self, path: &std::path::Path) {
        self.win.config_viewer = false;
        for (i, tab) in self.win.tabs.iter_mut().enumerate() {
            if i != self.win.active && tab.root.contains(VIEWER_PANE) {
                tab.root.remove(VIEWER_PANE);
            }
        }
        if let Some(tab) = self.win.tabs.get_mut(self.win.active)
            && !tab.root.contains(VIEWER_PANE)
        {
            // the leftmost column, next to the explorer, taking an equal
            // share with the shell panes side by side
            tab.zoomed = None;
            let columns = tab.root.leaves().len() as f32;
            tab.root.insert_root(
                VIEWER_PANE,
                crate::pane::Edge::Left,
                (1.0 / (columns + 1.0)).clamp(0.25, 0.5),
            );
        }
        self.win.viewer_focus = true;
        let proxy = self.proxy.clone();
        self.win.viewer.open(path, move |loaded| {
            let _ = proxy.send_event(UserEvent::ViewerLoaded(Box::new(loaded)));
        });
    }

    /// ⌃R: the history pane above the command line (again: next scope).
    /// Rows: this window's finished commands (newest first, with status,
    /// directory and age), then the imported shell history.
    fn open_history(&mut self) {
        use crate::views::history::{Entry, dedupe};
        if self.win.history.open {
            let scope = self.win.history.scope.next();
            self.win.history.set_scope(scope);
            return;
        }
        let active = self.active_pane_id();
        let mut entries: Vec<Entry> = Vec::new();
        for tab in &self.win.tabs {
            for p in &tab.panes {
                let here = Some(p.session.id) == active;
                let blocks = p.session.blocks.lock().unwrap();
                for b in blocks.iter() {
                    if matches!(b.state, rift_core::BlockState::Done { .. }) {
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
        if let Some(p) = self.win.tabs.get(self.win.active).and_then(|t| t.active()) {
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
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .map(|p| p.editor.buffer.clone())
            .unwrap_or_default();
        // no completion answer in flight may reopen the panel under it
        self.win.completion.close();
        self.completion_pending = None;
        self.completion_intent = None;
        self.win.history.open(dedupe(entries), &query);
        self.win.history.pane = active;
    }

    /// Keys for the open history pane; `false` lets the key edit the command
    /// line (the query).
    fn handle_history_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let mods = self.win.modifiers;
        // nothing matches: ⏎ runs the typed line as usual; ⇥ only closes the
        // pane, so completions show before a second ⇥ accepts one
        if self.win.history.current().is_none() {
            match event.logical_key {
                Key::Named(NamedKey::Enter) => {
                    self.win.history.close();
                    return false;
                }
                Key::Named(NamedKey::Tab) => {
                    self.win.history.close();
                    self.update_completions();
                    return true;
                }
                _ => {}
            }
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.win.history.close(),
            Key::Named(NamedKey::ArrowUp) => self.win.history.move_by(-1),
            Key::Named(NamedKey::ArrowDown) => self.win.history.move_by(1),
            Key::Named(NamedKey::PageUp) => self.win.history.move_by(-8),
            Key::Named(NamedKey::PageDown) => self.win.history.move_by(8),
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
        let Some(command) = self.win.history.current().map(|e| e.command.clone()) else {
            return;
        };
        self.win.history.close();
        match how {
            HistoryUse::RunInSplit => {
                let Some(target) = self.active_pane_id() else {
                    return;
                };
                // the query was only a search: this pane's line empties
                if let Some(p) = self
                    .win
                    .tabs
                    .get_mut(self.win.active)
                    .and_then(|t| t.active_mut())
                {
                    let len = p.editor.buffer.len();
                    p.editor.replace_range(0..len, "");
                }
                self.win.completion.close();
                let cwd = self
                    .win
                    .tabs
                    .get(self.win.active)
                    .and_then(|t| t.active())
                    .and_then(|p| p.session.local_cwd())
                    .map(std::path::PathBuf::from);
                let new = self.split_pane(target, Axis::Horizontal, cwd);
                self.pending_commands.insert(new, vec![command]);
            }
            HistoryUse::Edit | HistoryUse::Run => {
                if let Some(p) = self
                    .win
                    .tabs
                    .get_mut(self.win.active)
                    .and_then(|t| t.active_mut())
                {
                    let len = p.editor.buffer.len();
                    p.editor.replace_range(0..len, &command);
                }
                self.win.completion.close();
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
        if !self.win.history.open {
            return;
        }
        let Some(p) = self.win.tabs.get(self.win.active).and_then(|t| t.active()) else {
            self.win.history.close();
            return;
        };
        if !Self::editor_visible(p)
            || self.win.history.pane != Some(p.session.id)
            || self.viewer_focused()
            || self.win.settings.open
            || self.win.palette.open
            || self.win.search.open
        {
            self.win.history.close();
        } else if p.editor.buffer != self.win.history.query {
            let q = p.editor.buffer.clone();
            self.win.history.filter(&q);
        }
    }

    fn open_palette(&mut self, mode: crate::views::palette::PaletteMode) {
        if mode == crate::views::palette::PaletteMode::Commands {
            // pick up files added while running
            let (configs, skipped) = rift_core::launch::load_all_reporting();
            self.launch_configs = configs;
            if !skipped.is_empty() && skipped != self.launch_skipped {
                let list = skipped
                    .iter()
                    .map(|s| format!("{} ({})", s.file, s.reason))
                    .collect::<Vec<_>>()
                    .join("; ");
                let what = match skipped.len() {
                    1 => "Tab config skipped".to_string(),
                    n => format!("{n} tab configs skipped"),
                };
                self.toast(format!("{what}: {list}"));
            }
            self.launch_skipped = skipped;
            self.ssh_profiles = rift_core::ssh::all_profiles(&self.config.ssh_profiles);
        }
        if mode == crate::views::palette::PaletteMode::Sessions && !self.win.windows_loading {
            self.win.windows_loading = true;
            self.win.other_windows.clear();
            let proxy = self.proxy.clone();
            let asker = self.win_id();
            std::thread::spawn(move || {
                crate::control::visit_other_windows(|window| {
                    let _ = proxy.send_event(UserEvent::WindowsLoaded(asker, vec![window], false));
                });
                let _ = proxy.send_event(UserEvent::WindowsLoaded(asker, Vec::new(), true));
            });
        }
        if matches!(
            mode,
            crate::views::palette::PaletteMode::Sessions
                | crate::views::palette::PaletteMode::Commands
        ) && !self.win.detached_loading
        {
            // shells the service keeps that no window shows
            self.win.detached_loading = true;
            let proxy = self.proxy.clone();
            let asker = self.win_id();
            std::thread::spawn(move || {
                let detached = rift_pty::service::list()
                    .into_iter()
                    .filter(|s| s.clients.is_empty())
                    .collect();
                let _ = proxy.send_event(UserEvent::DetachedLoaded(asker, detached));
            });
        }
        self.win.palette.open = true;
        self.win.palette.mode = mode;
        self.win.palette.query.clear();
        self.win.palette.selected = 0;
        self.refresh_palette();
        self.blink_epoch = Instant::now();
    }

    fn run_palette_action(&mut self, action: PaletteAction, event_loop: &ActiveEventLoop) {
        self.win.palette.open = false;
        match action {
            PaletteAction::App(a) => self.handle_action(a, event_loop),
            PaletteAction::OpenSettings => {
                self.win.settings.open_page(self.win.settings.page);
            }
            PaletteAction::SwitchTheme(i) => {
                let names = rift_core::theme::available_themes();
                if let Some(name) = names.into_iter().nth(i) {
                    self.config.theme = name;
                    self.save_config();
                    self.apply_config();
                }
            }
            PaletteAction::Search => {
                self.win.search.open = true;
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
                if let Some(pane) = self
                    .win
                    .tabs
                    .get_mut(self.win.active)
                    .and_then(|t| t.active_mut())
                {
                    pane.editor.clear();
                    pane.editor.insert(&cmd);
                }
                self.update_completions();
            }
            PaletteAction::FocusTab(i) => {
                self.win.settings.open = false;
                if i < self.win.tabs.len() {
                    self.win.active = i;
                    self.win.tabs[i].activity = false;
                }
            }
            PaletteAction::FocusPane(ti, pane) => {
                self.win.settings.open = false;
                if let Some(tab) = self.win.tabs.get_mut(ti) {
                    self.win.active = ti;
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
                self.win.settings.open = false;
                if let Some(tab) = self.win.tabs.get_mut(ti)
                    && tab.pane(pane).is_some()
                {
                    self.win.active = ti;
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
            // another window of this process
            PaletteAction::FocusWindow(pid, _, pane) if pid == std::process::id() => {
                if self.enter_pane_window(pane) {
                    self.focus_pane_here(pane);
                } else {
                    self.toast("That window is closed".into());
                }
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
                    // the tab opens now; its command follows from a thread
                    // (`UserEvent::SshCommand`) and runs at the first prompt
                    self.spawn_tab_at(None);
                    let pane = self.win.tabs.get_mut(self.win.active).map(|tab| {
                        tab.custom_title = Some(p.name.clone());
                        tab.active_pane
                    });
                    self.save_session();
                    let proxy = self.proxy.clone();
                    let spawned = std::thread::Builder::new()
                        .name("ssh-profile".into())
                        .spawn(move || {
                            // shared connection (remote completions run over
                            // it), unless the user's own ssh config or args
                            // set one up
                            let control = rift_core::ssh::control_dir()
                                .filter(|_| !p.shares_connections_itself())
                                .and_then(|d| rift_core::ssh::private_control_dir(&d));
                            let command = p.command_shared(control.as_deref());
                            if let Some(pane) = pane {
                                let _ = proxy.send_event(UserEvent::SshCommand(pane, command));
                            }
                        });
                    if let Err(e) = spawned {
                        log::warn!("ssh profile thread: {e}");
                    }
                }
            }
            PaletteAction::SshInstall(i) => {
                if let Some(p) = self.ssh_profiles.get(i).cloned() {
                    self.win.confirm = Some(ConfirmClose {
                        opened: Instant::now(),
                        commands: vec![
                            "~/.local/share/rift/rift.{bash,zsh,fish}".into(),
                            "~/.local/share/rift/complete.{bash,zsh,fish}".into(),
                            format!(
                                "~/.bashrc, ~/.zshrc: one line ending {}",
                                rift_core::ssh::RC_MARKER
                            ),
                            "~/.config/fish/conf.d/rift.fish".into(),
                        ],
                        action: ConfirmAction::InstallRemote(p),
                    });
                }
            }
            PaletteAction::RunHistory(cmd) => {
                // same path as Enter: prompt-time context, synced panes
                if let Some(id) = self.win.tabs.get(self.win.active).map(|t| t.active_pane) {
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
                self.win.palette.open = false;
            }
            Key::Named(NamedKey::Enter) => {
                if let Some(m) = self.win.palette_results.get(self.win.palette.selected) {
                    let action = m.item.action.clone();
                    self.run_palette_action(action, event_loop);
                }
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.win.palette.selected = self.win.palette.selected.saturating_sub(1);
            }
            Key::Named(NamedKey::ArrowDown) => {
                if self.win.palette.selected + 1 < self.win.palette_results.len() {
                    self.win.palette.selected += 1;
                }
            }
            Key::Named(NamedKey::Tab) if self.win.palette.mode != PaletteMode::PipeOutput => {
                let next = crate::views::palette::next_mode(self.win.palette.mode);
                self.open_palette(next);
            }
            Key::Named(NamedKey::Backspace) => {
                self.win.palette.query.pop();
                self.win.palette.selected = 0;
                self.refresh_palette();
            }
            Key::Named(NamedKey::Space) => {
                self.win.palette.query.push(' ');
                self.win.palette.selected = 0;
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
                            self.win.palette.query.is_empty()
                                && text.chars().count() == 1
                                && self.win.palette.mode != PaletteMode::PipeOutput
                        })
                        .and_then(crate::views::palette::mode_for_prefix)
                        .filter(|m| *m != self.win.palette.mode);
                    if let Some(mode) = prefix {
                        self.open_palette(mode);
                    } else {
                        self.win.palette.query.push_str(text);
                        self.win.palette.selected = 0;
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
        let Some(pane) = self.win.tabs.get(self.win.active).and_then(|t| t.active()) else {
            self.completion_pending = None;
            self.win.completion.close();
            return;
        };
        let buffer = pane.editor.buffer.clone();
        let cursor = pane.editor.cursor;
        if buffer.trim().is_empty() {
            self.completion_pending = None;
            self.win.completion.close();
            return;
        }
        let history = pane.editor.history.clone();
        let path = pane
            .session
            .user_vars
            .lock()
            .unwrap()
            .get(rift_core::osc::USER_VAR_PATH)
            .filter(|p| !p.is_empty())
            .cloned();
        let cwd = pane.session.local_cwd();
        let pane_id = pane.session.id;
        let asks_shell = self.config.shell_completions;
        let shell = (asks_shell && cwd.is_some())
            .then(|| rift_core::shell_integration::resolve_shell(self.config.shell.as_deref()));
        // a pane on another machine completes there over the shared
        // connection of the `ssh` it runs (the newest ssh block submitted
        // from the editor; cached until the pane's blocks change)
        let remote = asks_shell
            .then(|| pane.session.remote_cwd())
            .flatten()
            .map(|cwd| {
                let target = pane.session.blocks.lock().unwrap().ssh_target();
                RemoteCompletion { cwd, target }
            });
        // another pane's menu never stays up while this one's is computed
        if self.completion_pane != Some(pane_id) {
            self.win.completion.close();
            self.win.completion_buffer.clear();
        }
        let typed = buffer
            .strip_prefix(self.win.completion_buffer.as_str())
            .filter(|_| cursor == buffer.len());
        match typed {
            Some(t) if self.win.completion.ghost.starts_with(t) => {
                self.win.completion.ghost.drain(..t.len());
            }
            _ => self.win.completion.ghost.clear(),
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
            path,
            shell,
            remote,
        };
        self.completion_partial = None;
        let proxy = self.proxy.clone();
        let spawn = move || {
            let (tx, rx) = std::sync::mpsc::channel::<CompletionRequest>();
            let spawned = std::thread::Builder::new()
                .name("completion".into())
                .spawn(move || {
                    // the shell / remote half, started on first use
                    let mut slow: Option<std::sync::mpsc::Sender<Box<SlowCompletion>>> = None;
                    while let Ok(mut req) = rx.recv() {
                        // only the newest request matters
                        while let Ok(newer) = rx.try_recv() {
                            req = newer;
                        }
                        let generation = req.generation;
                        match req.run() {
                            Answer::Done(items) => {
                                let _ = proxy.send_event(UserEvent::Completions(generation, items));
                            }
                            Answer::Partial(first, rest) => {
                                let _ = proxy
                                    .send_event(UserEvent::CompletionsPartial(generation, first));
                                let tx = slow.get_or_insert_with(|| spawn_slow(proxy.clone()));
                                if let Err(e) = tx.send(rest) {
                                    // that thread died: answer with Rift's own
                                    slow = None;
                                    let items = e.0.req.finish(e.0.gathered);
                                    let _ =
                                        proxy.send_event(UserEvent::Completions(generation, items));
                                }
                            }
                        }
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
            self.win.completion.close();
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
        self.win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .is_some_and(|p| {
                p.session.id == request.pane
                    && p.editor.cursor == request.cursor
                    && p.editor.buffer == request.buffer
            })
    }

    /// A completion answer from the worker: applied only when it answers
    /// the latest request and the editor has not changed since.
    fn on_completions(&mut self, generation: u64, items: Vec<rift_core::completion::Completion>) {
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
        let partial_shown = self.completion_partial == Some(generation);
        self.completion_partial = None;
        if !self.editing(&request) {
            return;
        }
        // the row picked in Rift's own list stays picked
        let picked = (partial_shown && self.win.completion.selected > 0)
            .then(|| {
                self.win
                    .completion
                    .items
                    .get(self.win.completion.selected)
                    .cloned()
            })
            .flatten();
        self.completion_pane = Some(request.pane);
        self.apply_completions(&request.buffer, request.cursor, items);
        if let Some(i) = picked.and_then(|p| self.win.completion.items.iter().position(|c| *c == p))
        {
            self.win.completion.selected = i;
        }
        match intent {
            Some(CompletionIntent::Accept) => self.accept_completion(),
            Some(CompletionIntent::AcceptGhostWord) => self.accept_ghost_word(),
            None => {}
        }
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Rift's own answer while the shell or remote machine is still asked:
    /// shown at once; a deferred Tab waits for the full answer.
    fn on_partial_completions(
        &mut self,
        generation: u64,
        items: Vec<rift_core::completion::Completion>,
    ) {
        let Some(request) = self
            .completion_pending
            .as_ref()
            .filter(|r| r.generation == generation)
            .cloned()
        else {
            return;
        };
        if !self.editing(&request) {
            return;
        }
        self.completion_partial = Some(generation);
        self.completion_pane = Some(request.pane);
        self.apply_completions(&request.buffer, request.cursor, items);
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    fn apply_completions(
        &mut self,
        buffer: &str,
        cursor: usize,
        items: Vec<rift_core::completion::Completion>,
    ) {
        self.win.completion_buffer = buffer.to_string();
        // the history pane owns the line while it is open
        if self.win.history.open {
            self.win.completion.close();
            return;
        }
        // ghost: best history match remainder, else the single completion
        self.win.completion.ghost = items
            .iter()
            .find(|c| c.source == "history")
            .and_then(|c| c.insert.strip_prefix(buffer).map(str::to_string))
            .unwrap_or_default();

        // the panel lists the token candidates, then the history lines (the
        // best history line also drives the ghost text)
        let (mut dropdown, history): (Vec<_>, Vec<_>) =
            items.into_iter().partition(|c| c.source != "history");
        // the whole word around the cursor is what a candidate replaces
        let (start, _) = rift_core::completion::current_token(buffer, cursor);
        let end = rift_core::completion::token_end(buffer, cursor);
        let word = buffer.get(start..end).unwrap_or_default();
        self.win.completion.token = word.to_string();
        let candidates =
            |rows: &[rift_core::completion::Completion]| rows.iter().any(|c| c.source != "hint");
        // the remote hint is a note that rides along real candidates: alone
        // it never opens the menu
        if !candidates(&dropdown) {
            dropdown.clear();
        }
        let prefixed = dropdown
            .iter()
            .any(|c| c.source != "hint" && c.insert.starts_with(word));
        if self.win.completion.ghost.is_empty()
            && let Some(first) = dropdown.first()
        {
            let (_, token) = rift_core::completion::current_token(buffer, cursor);
            if cursor == buffer.len() {
                self.win.completion.ghost = first
                    .insert
                    .strip_prefix(token)
                    .unwrap_or_default()
                    .to_string();
            }
        }
        if !self.config.autosuggestions {
            self.win.completion.ghost.clear();
        }
        // history lines join a panel the word candidates open; alone they
        // stay the ghost text (no panel on every keystroke). Fuzzy rows
        // alone (none starts with the word) give way to a history ghost:
        // they neither open the panel nor take Tab from it
        let fuzzy_only = !prefixed && !self.win.completion.ghost.is_empty();
        let show = self.config.completions_menu && !dropdown.is_empty() && !fuzzy_only;
        dropdown.extend(history);
        self.win
            .completion
            .set(if show { dropdown } else { Vec::new() });
    }

    /// ⌃→: accept one word of the ghost suggestion.
    fn accept_ghost_word(&mut self) {
        if self.defer_completion_key(CompletionIntent::AcceptGhostWord) {
            return;
        }
        if self.win.completion.ghost.is_empty() {
            return;
        }
        let ghost = self.win.completion.ghost.clone();
        // word = leading whitespace + next non-space run
        let trimmed = ghost.trim_start();
        let lead = ghost.len() - trimmed.len();
        let word_end = trimmed
            .find(char::is_whitespace)
            .map(|i| lead + i)
            .unwrap_or(ghost.len());
        let word = &ghost[..word_end];
        if let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        {
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
        let on_hint = self.win.completion.open
            && self
                .win
                .completion
                .items
                .get(self.win.completion.selected)
                .is_some_and(|c| c.source == "hint");
        if on_hint && self.win.completion.ghost.is_empty() {
            // a note, not a candidate: nothing to insert
            self.win.completion.close();
            return;
        }
        // on the hint, Tab takes the ghost text
        let (replacement, ghost) = if self.win.completion.open && !on_hint {
            (
                self.win
                    .completion
                    .items
                    .get(self.win.completion.selected)
                    .map(|c| (c.insert.clone(), c.source == "history")),
                None,
            )
        } else if !self.win.completion.ghost.is_empty() {
            (None, Some(self.win.completion.ghost.clone()))
        } else {
            return;
        };
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        else {
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
            let (start, _) = rift_core::completion::current_token(&buffer, cursor);
            // the whole word: a closing quote or the suffix right of a
            // mid-word cursor would otherwise survive next to `insert`
            let end = rift_core::completion::token_end(&buffer, cursor);
            // through the single mutation point: makes the completion
            // undoable
            pane.editor.replace_range(start..end, &insert);
        }
        self.win.completion.close();
        self.update_completions();
    }

    fn logical_size(&self) -> (f32, f32) {
        let scale = self
            .win
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        let gpu = self.win.gpu.as_ref().unwrap();
        (
            gpu.config.width as f32 / scale,
            gpu.config.height as f32 / scale,
        )
    }

    /// Editor is shown when shell integration is live (last block is a
    /// prompt) and no full-screen app is running.
    fn editor_visible(pane: &PaneState) -> bool {
        use rift_pty::alacritty_terminal::term::TermMode;
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
        // no usable directory (a launch-config pane without one, a remote
        // tab reopened, a restored pane without cwd, a removed folder): the
        // configured start directory, never the app process's own cwd
        let cwd = cwd
            .filter(|d| d.is_dir())
            .or_else(|| self.config.start_dir(None));

        let proxy = self.proxy.clone();
        let observer: rift_pty::Observer = Arc::new(move |pane, event| {
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
            Some(id) => rift_pty::ServiceTarget::Attach(id),
            None => rift_pty::ServiceTarget::Spawn,
        });
        // a reattached shell replays its own output; no transcript on top
        let replay = self
            .next_replay
            .take()
            .filter(|_| self.config.save_transcripts && attach.is_none())
            .and_then(|id| rift_core::transcript::load(&id))
            .map(|t| (t.replay_bytes(), t.meta()));
        let (cell_size, shell, scrollback) = (
            self.cell_px(),
            self.config.shell.clone(),
            self.config.scrollback_lines(),
        );
        let clipboard = rift_pty::ClipboardAccess::from_config(&self.config.clipboard_access);
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
                // a service that accepts but stalls: every later pane would
                // wait out the same deadline, so this run stays local
                let stalled = e.kind() == std::io::ErrorKind::TimedOut;
                if stalled {
                    self.durable_allowed = false;
                }
                // a shell that ended while detached: start a fresh one there
                let retry = if !stalled
                    && matches!(service, Some(rift_pty::ServiceTarget::Attach(_)))
                {
                    Session::spawn(opts(Some(rift_pty::ServiceTarget::Spawn), replay.clone())).ok()
                } else {
                    None
                };
                match retry {
                    Some(s) => s,
                    None => {
                        let s = Session::spawn(opts(None, replay)).expect("spawn shell");
                        self.toast_error(format!(
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
    fn pane_transcript(pane: &PaneState) -> rift_core::transcript::Transcript {
        use rift_core::transcript::{SavedBlock, Transcript};
        // snapshot first: block_output_text takes the blocks lock itself
        let snapshot = pane.session.blocks.lock().unwrap().snapshot();
        let blocks: Vec<SavedBlock> = snapshot
            .into_iter()
            .filter_map(|b| match b.state {
                rift_core::BlockState::Done { exit, duration } => Some(SavedBlock {
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
            .win
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
            .find(|b| matches!(b.state, rift_core::BlockState::Done { .. }))
            .map(|b| b.id);
        let Some(newest) = newest else { return };
        if self.transcript_saved.get(&pane_id) == Some(&newest) {
            return;
        }
        let id = pane.transcript_id.clone();
        let t = Self::pane_transcript(pane);
        rift_core::transcript::save(&id, &t);
        self.transcript_saved.insert(pane_id, newest);
    }

    /// Palette "Clear saved block transcripts": delete every file.
    fn clear_transcripts(&mut self) {
        let n = rift_core::transcript::remove_except(&[]);
        self.transcript_saved.clear();
        self.toast_done(format!(
            "Deleted {n} saved transcript{}",
            if n == 1 { "" } else { "s" }
        ));
    }

    fn spawn_tab(&mut self) {
        let dir = self.start_dir();
        self.spawn_tab_at(dir);
    }

    /// ⌘T and the + button: a tab where `new_tab_placement` puts it (other
    /// paths — restores, launch configs, ctl — always append).
    fn spawn_new_tab(&mut self) {
        // the tab the user is on: the Settings tab sits after every shell tab
        let from = if self.win.settings.open && self.win.settings.tab {
            self.win.tabs.len().saturating_sub(1)
        } else {
            self.win.active
        };
        self.spawn_tab();
        let last = self.win.tabs.len() - 1;
        let to = from + 1;
        if self.config.new_tab_placement == rift_core::NewTabPlacement::AfterCurrent && to < last {
            let tab = self.win.tabs.remove(last);
            self.win.tabs.insert(to, tab);
            self.win.active = to;
            self.save_session();
        }
    }

    /// Probe the user's shell for aliases/functions off the UI thread.
    fn load_shell_env(&self) {
        let proxy = self.proxy.clone();
        let shell = rift_core::shell_integration::resolve_shell(self.config.shell.as_deref());
        std::thread::Builder::new()
            .name("shell-env".into())
            .spawn(move || {
                let env = rift_core::shell_env::ShellEnv::load(&shell);
                let _ = proxy.send_event(UserEvent::ShellEnvLoaded(Arc::new(env)));
            })
            .ok();
    }

    fn spawn_tab_at(&mut self, cwd: Option<std::path::PathBuf>) {
        self.win.settings.open = false;
        let (lw, lh) = self.logical_size();
        let (_, lw) = self.content_span(lw);
        let cols = ((lw - self.config.padding() * 2.0 - terminal::BLOCK_INDENT) / self.cell_w)
            .max(2.0) as usize;
        let rows = ((lh - TAB_BAR_H - TOP_PAD - EDITOR_H) / self.line_h).max(2.0) as usize;
        let session = self.spawn_session(cols, rows, cwd);
        self.win.tabs.push(TabState::new(self.new_pane(session)));
        self.win.active = self.win.tabs.len() - 1;
        self.save_session();
    }

    /// Persist tabs: split layouts, cwds, custom titles, colors (atomic;
    /// called on every structural change).
    fn save_session(&self) {
        if !self.persist_session {
            return;
        }
        self.save_session_file();
        if self.config.save_transcripts {
            let mut keep = Vec::new();
            for p in self.all_panes() {
                rift_core::transcript::save(&p.transcript_id, &Self::pane_transcript(p));
                keep.push(p.transcript_id.clone());
            }
            // closed panes' transcripts go with them
            rift_core::transcript::remove_except(&keep);
        }
    }

    /// The session file alone (layout, counted time), without transcripts.
    fn save_session_file(&self) {
        if self.persist_session {
            rift_core::session_state::save(&self.saved_session());
        }
    }

    /// After a quit: the session file keeps only the first window's frame,
    /// so the next launch opens one window with one new tab where it was;
    /// saved transcripts go with the tabs.
    fn save_fresh_session(&self) {
        if !self.persist_session {
            return;
        }
        let mut fresh = self.saved_session();
        fresh.tabs.clear();
        fresh.active = 0;
        fresh.other_windows.clear();
        rift_core::session_state::save(&fresh);
        rift_core::transcript::remove_except(&[]);
    }

    /// Every window's tabs, layouts and frame, as the session file stores
    /// them.
    fn saved_session(&self) -> rift_core::session_state::SavedSession {
        rift_core::session_state::SavedSession::of_windows(self.saved_windows())
    }

    /// One window's tabs, layouts and frame, as the session file stores
    /// them.
    fn saved_window(&self, win: &WindowState) -> rift_core::session_state::SavedWindowTabs {
        use rift_core::session_state::{SavedLayout, SavedTab, SavedWindowTabs};
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
            node.to_saved(&mut |id| SavedLayout::Pane {
                cwd: tab.pane(id).and_then(pane_cwd),
                focused: id == tab.active_pane,
                transcript: tab
                    .pane(id)
                    .filter(|_| transcripts)
                    .map(|p| p.transcript_id.clone()),
                service: tab.pane(id).and_then(|p| p.session.service_id),
            })
        }
        let tabs = win
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
                started: t.started.and_then(|s| {
                    s.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|d| d.as_secs())
                }),
                counted: Some(t.clock.total(Instant::now()).as_secs()).filter(|s| *s > 0),
            })
            .collect();
        SavedWindowTabs {
            tabs,
            active: win.active,
            window: win.window.as_deref().and_then(Self::window_frame),
        }
    }

    /// A window's frame in logical px (for session restore).
    fn window_frame(w: &Window) -> Option<rift_core::session_state::SavedWindow> {
        let scale = w.scale_factor();
        let pos = w.outer_position().ok()?.to_logical::<f64>(scale);
        let size = w.inner_size().to_logical::<f64>(scale);
        Some(rift_core::session_state::SavedWindow {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            maximized: w.is_maximized(),
            fullscreen: w.fullscreen().is_some(),
        })
    }

    /// Reopen a saved tab: its split tree, pane cwds, title and color.
    fn restore_tab(&mut self, saved: &rift_core::session_state::SavedTab) {
        use rift_core::session_state::SavedLayout;
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
        let service = match &saved.layout {
            Some(l) => l.first_service(),
            None => saved.service,
        };
        self.next_attach = service;
        self.spawn_tab_at(top_cwd);
        let root = self.win.tabs[self.win.active].active_pane;
        // the Elapsed clock belongs to the shell: it survives only a
        // reattach to the same live shell, and restarts with a new one
        let reattached = service.is_some()
            && self.win.tabs[self.win.active]
                .pane(root)
                .is_some_and(|p| p.session.service_id == service);
        if let Some(id) = transcript
            && let Some(p) = self.win.tabs[self.win.active].pane_mut(root)
        {
            p.transcript_id = id;
        }
        if let Some(layout) = &saved.layout {
            let mut focus = None;
            self.build_saved(layout, root, &mut focus);
            let tab = &mut self.win.tabs[self.win.active];
            // splitting makes each new pane active: the saved focus is
            // applied once the whole tree exists
            if let Some(id) = focus {
                tab.active_pane = id;
            }
            // the weights; layouts saved as a binary tree flatten here
            tab.root.apply_saved(layout);
            if !tab.panes.iter().any(|p| p.session.id == tab.active_pane) {
                tab.active_pane = root;
            }
        }
        let tab = &mut self.win.tabs[self.win.active];
        tab.custom_title = saved.title.clone().filter(|t| !t.is_empty());
        tab.started = saved
            .started
            .filter(|_| reattached)
            .map(|s| std::time::UNIX_EPOCH + Duration::from_secs(s));
        // counted time is kept across restarts: the closed time is not
        // counted, and a command still running counts again from reattach
        tab.clock = rift_core::tab_clock::TabClock::from_secs(saved.counted.unwrap_or(0));
        tab.color = saved
            .color
            .as_deref()
            .and_then(|c| crate::state::TAB_COLORS.iter().position(|n| *n == c));
    }

    /// "Save current layout as launch configuration" (Warp Tab Configs): every
    /// tab's split tree + pane cwds as a Warp Tab Config TOML file (one per tab).
    fn save_current_layout(&mut self) {
        use rift_core::launch::{LaunchConfig, LaunchLayout, LaunchTab};
        fn layout(tab: &TabState, node: &crate::pane::PaneNode) -> LaunchLayout {
            match node {
                crate::pane::PaneNode::Leaf(id) => LaunchLayout::Pane {
                    cwd: tab
                        .pane(*id)
                        .and_then(|p| p.session.local_cwd())
                        .map(std::path::PathBuf::from),
                    commands: Vec::new(),
                    focused: *id == tab.active_pane && tab.is_split(),
                },
                crate::pane::PaneNode::Split { axis, children } => {
                    let vertical = *axis == Axis::Horizontal;
                    // flatten same-direction chains the way Warp nests them
                    let mut panes = Vec::new();
                    for (_, child) in children {
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
                .win
                .tabs
                .iter()
                .map(|t| LaunchTab {
                    title: t.custom_title.clone(),
                    color: t
                        .color
                        .and_then(|i| crate::state::TAB_COLORS.get(i))
                        .map(|c| c.to_string()),
                    layout: layout(t, &t.root.without_viewer()),
                })
                .collect(),
            active_tab: self.win.active,
        };
        match rift_core::launch::save(&cfg) {
            Ok(paths) => {
                self.launch_configs = rift_core::launch::load_all();
                let shown = paths
                    .first()
                    .map(|p| editor::shorten_home(&p.to_string_lossy()))
                    .unwrap_or_default();
                let more = if paths.len() > 1 {
                    format!(" (+{} more)", paths.len() - 1)
                } else {
                    String::new()
                };
                self.toast_done(format!("Saved launch configuration to {shown}{more}"));
            }
            Err(e) => self.toast_error(format!("Could not save launch configuration: {e}")),
        }
    }

    /// A status toast about what just happened, in the pane you are in.
    fn toast(&mut self, msg: String) {
        self.toast_as(crate::views::confirm::ToastKind::Info, msg, None, None);
    }

    /// It worked (copied, saved): a check, gone after [`TOAST_TTL`].
    fn toast_done(&mut self, msg: String) {
        self.toast_as(crate::views::confirm::ToastKind::Done, msg, None, None);
    }

    /// It failed: stays until dismissed.
    fn toast_error(&mut self, msg: String) {
        self.toast_as(crate::views::confirm::ToastKind::Error, msg, None, None);
    }

    /// A paired phone acted on `pane`.
    fn toast_phone(&mut self, pane: Option<PaneId>, msg: String) {
        self.toast_as(crate::views::confirm::ToastKind::Phone, msg, pane, None);
    }

    /// Show a toast in `pane` (default: the pane you are in; none while the
    /// Settings tab shows): the same text there replaces itself, at most
    /// [`TOASTS_MAX`] stay, an error until dismissed.
    fn toast_as(
        &mut self,
        kind: crate::views::confirm::ToastKind,
        msg: String,
        pane: Option<PaneId>,
        action: Option<ToastAction>,
    ) {
        log::info!("toast ({kind:?}): {msg}");
        let pane = pane.or_else(|| {
            (!self.win.settings.open)
                .then(|| self.win.tabs.get(self.win.active).map(|t| t.active_pane))
                .flatten()
        });
        self.win
            .toasts
            .retain(|t| !(t.text == msg && t.pane == pane));
        self.next_toast_id += 1;
        self.win.toasts.insert(
            0,
            Toast {
                id: self.next_toast_id,
                kind,
                text: msg.clone(),
                pane,
                action,
                at: Instant::now(),
            },
        );
        self.win.toasts.truncate(TOASTS_MAX);
        self.win.a11y_announcement = Some(msg);
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Realize a saved pane tree inside the region held by `pane`.
    fn build_saved(
        &mut self,
        layout: &rift_core::session_state::SavedLayout,
        pane: PaneId,
        focus: &mut Option<PaneId>,
    ) {
        use rift_core::session_state::{SavedAxis, SavedLayout};
        match layout {
            SavedLayout::Pane { focused, .. } => {
                if *focused && focus.is_none() {
                    *focus = Some(pane);
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
                        .win
                        .tabs
                        .get_mut(self.win.active)
                        .and_then(|t| t.pane_mut(new_id))
                {
                    p.transcript_id = id;
                }
                self.build_saved(first, pane, focus);
                self.build_saved(second, new_id, focus);
            }
        }
    }

    fn split_active(&mut self, axis: Axis) {
        if self.win.tabs.is_empty() {
            return;
        }
        let dir = self.start_dir();
        let session = self.spawn_session(60, 20, dir); // resized on next layout
        let new_id = session.id;
        let pane = self.new_pane(session);
        let tab = &mut self.win.tabs[self.win.active];
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
            .win
            .tabs
            .iter()
            .position(|t| t.panes.iter().any(|p| p.session.id == pane))
        else {
            return;
        };
        // closing a pane ends its shell even when the session service owns it
        if let Some(p) = self.win.tabs[tab_idx].pane(pane) {
            p.session.terminate();
        }
        let tab = &mut self.win.tabs[tab_idx];
        if tab.panes.len() <= 1 {
            // remember the cwd for ⇧⌘T: a local one only (a remote tab's
            // path does not exist here)
            let cwd = tab
                .active()
                .and_then(|p| p.session.local_cwd().or_else(|| p.last_git_cwd.clone()));
            self.win.closed_tabs.push(cwd.map(std::path::PathBuf::from));
            if self.win.closed_tabs.len() > 10 {
                self.win.closed_tabs.remove(0);
            }
            self.win.tabs.remove(tab_idx);
            if self.win.tabs.is_empty() {
                // another window stays open: only this one closes
                if self.close_window() {
                    return;
                }
                // the drop-down window's shells end with the app (it is not
                // saved, so nothing would reattach them)
                for p in self.all_panes() {
                    p.session.terminate();
                }
                self.save_session();
                crate::quit::exit(event_loop);
                return;
            }
            if self.win.active > tab_idx {
                self.win.active -= 1;
            } else if self.win.active >= self.win.tabs.len() {
                self.win.active = self.win.tabs.len() - 1;
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
        let now = Instant::now();
        let due = |at: Option<Instant>, every: Duration| at.is_none_or(|t| now - t >= every);
        // while the window is in use, the active tab is asked again on a
        // beat: an agent or an editor changes files with no command ending
        let git_due = self.win.focused && due(self.win.git_polled, GIT_POLL);
        let docker_due = self.win.focused && due(self.win.docker_polled, DOCKER_POLL);
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        let active = tab.active_pane;
        let refocused = self.win.status_pane != Some(active);
        let docker = self
            .config
            .prompt_chips
            .contains(&rift_core::PromptChip::Docker);
        for pane in &mut tab.panes {
            let cwd = pane.session.local_cwd();
            let blocks = pane
                .session
                .blocks
                .lock()
                .unwrap()
                .last()
                .map_or(0, |b| b.id);
            let Some(cwd) = cwd else {
                // a directory on another host (ssh): the local repo's
                // branch and compose project no longer describe it
                if pane
                    .session
                    .cwd_remote
                    .load(std::sync::atomic::Ordering::Relaxed)
                    && (pane.last_git_cwd.is_some() || pane.git.is_some())
                {
                    pane.last_git_cwd = None;
                    pane.git = None;
                    pane.pr = None;
                    pane.docker = None;
                    changed = true;
                }
                continue;
            };
            let moved = pane.last_git_cwd.as_deref() != Some(cwd.as_str());
            let focused_now = refocused && pane.session.id == active;
            let event = moved || pane.last_git_block != blocks || focused_now;
            if event || git_due {
                pane.last_git_block = blocks;
                if let Some(worker) = &self.git {
                    worker.request(pane.session.id, cwd.clone().into());
                }
            }
            if docker
                && (event || docker_due)
                && let Some(worker) = &self.docker_worker
            {
                worker.request(pane.session.id, cwd.clone().into());
            }
            if moved {
                pane.last_git_cwd = Some(cwd);
                changed = true;
            }
        }
        self.win.status_pane = Some(active);
        if git_due {
            self.win.git_polled = Some(now);
        }
        if docker_due {
            self.win.docker_polled = Some(now);
        }
        if changed {
            self.save_session();
        }
    }

    /// When the focused window's next status poll is due (none while no
    /// window of Rift is in use).
    fn status_poll_due(&self) -> Option<Instant> {
        let next = |at: Option<Instant>, every: Duration| at.map_or(Instant::now(), |t| t + every);
        self.win.focused.then(|| {
            next(self.win.git_polled, GIT_POLL).min(next(self.win.docker_polled, DOCKER_POLL))
        })
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
            .win
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
            rift_core::TabLayout::Vertical if self.tabs_shown() => tabbar::RAIL_W,
            _ => 0.0,
        }
    }

    /// The tabs show (strip tabs or the vertical rail): more than one tab,
    /// counting the Settings tab, or `tab_bar_with_one_tab`.
    fn tabs_shown(&self) -> bool {
        self.config.tab_bar_with_one_tab
            || self.win.tabs.len() + usize::from(self.win.settings.tab) > 1
    }

    /// Widths of the open side panels (explorer left, review right) in a
    /// window `w` wide. A panel that would squeeze the panes under
    /// `MIN_CONTENT_W` is hidden, review first.
    fn panel_widths(&self, w: f32) -> (f32, f32) {
        const MIN_CONTENT_W: f32 = 360.0;
        let room = w - self.rail_w();
        let mut ex = if self.win.explorer.open {
            crate::views::explorer::EXPLORER_W
        } else {
            0.0
        };
        let mut rv = if self.win.review.open {
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
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .and_then(|p| p.session.local_cwd())
        else {
            return;
        };
        // unique across windows: the answer finds its window by it
        self.review_generation += 1;
        self.win.review.generation = self.review_generation;
        // another repository's changes are never shown for this one while
        // its own load
        if self
            .win
            .review
            .model()
            .is_none_or(|m| !std::path::Path::new(&cwd).starts_with(&m.root))
        {
            self.win.review.load = crate::views::review_panel::Load::Loading;
        }
        let generation = self.win.review.generation;
        let proxy = self.proxy.clone();
        std::thread::Builder::new()
            .name("review".into())
            .spawn(move || {
                let result = rift_core::review::load(std::path::Path::new(&cwd));
                let _ = proxy.send_event(UserEvent::ReviewLoaded(generation, result));
            })
            .ok();
    }

    /// Reload the review model when the active pane's directory changes
    /// (a `cd`, another tab or pane).
    fn follow_review(&mut self) {
        let cwd = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .and_then(|p| p.session.local_cwd());
        if cwd.is_some() && cwd != self.win.review_cwd {
            self.win.review_cwd = cwd;
            self.request_review();
        }
    }

    /// Point the explorer at the active pane's project (on cwd change).
    fn follow_explorer(&mut self) {
        if !self.win.explorer.open {
            return;
        }
        let cwd = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .and_then(|p| p.session.local_cwd());
        if let Some(cwd) = cwd
            && self.win.explorer.follow(std::path::Path::new(&cwd))
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
        let Some(window) = self.win.window.clone() else {
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
            use rift_pty::alacritty_terminal::grid::Dimensions;
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
                self.win.iconified = false;
            }
            2 => {
                window.set_minimized(true);
                self.win.iconified = true;
            }
            // positions are in points: they round-trip exactly on any scale
            3 => {
                let x = arg(1).unwrap_or(0) as f64;
                let y = arg(2).unwrap_or(0) as f64;
                window.set_outer_position(winit::dpi::LogicalPosition::new(x, y));
                self.win.window_pos = Some((x, y));
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
                    if let Some(size) = self.win.restore_size.take() {
                        let _ = window.request_inner_size(size);
                    }
                }
                n => {
                    self.win
                        .restore_size
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
                    _ => !self.win.fullscreen_op,
                };
                self.win.fullscreen_op = on;
                if self.headless {
                    // a hidden window has no fullscreen space: cover the screen
                    if on {
                        self.win
                            .restore_size
                            .get_or_insert(window.inner_size().to_logical(scale));
                        let _ = window.request_inner_size(screen);
                    } else if let Some(size) = self.win.restore_size.take() {
                        let _ = window.request_inner_size(size);
                    }
                } else {
                    window
                        .set_fullscreen(on.then_some(winit::window::Fullscreen::Borderless(None)));
                }
            }
            11 => {
                let iconified = window.is_minimized().unwrap_or(self.win.iconified);
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
                    .or(self.win.window_pos)
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
            p.session.write_report(text.as_bytes());
        }
    }

    /// Run a review-panel write, then reload. Revert asks first.
    fn review_action(&mut self, hit: ReviewHit) {
        use rift_core::review::Op;
        if hit == ReviewHit::Maximize {
            self.win.review.maximized = !self.win.review.maximized;
            return;
        }
        let Some(model) = self.win.review.model().cloned() else {
            return;
        };
        let root = model.root.clone();
        let ops: ReviewOps = match hit {
            ReviewHit::Refresh => {
                self.win.review.notice = None;
                self.win.explorer.refresh();
                self.request_review();
                return;
            }
            ReviewHit::ToggleSection(staged) => {
                if !self.win.review.collapsed_sections.remove(&staged) {
                    self.win.review.collapsed_sections.insert(staged);
                }
                return;
            }
            ReviewHit::ToggleFile(staged, path) => {
                let key = (staged, path);
                if !self.win.review.expanded.remove(&key) {
                    self.win.review.expanded.insert(key);
                }
                return;
            }
            ReviewHit::Reveal(staged, path, gap) => {
                self.win.review.reveal(staged, &path, gap);
                return;
            }
            ReviewHit::Open(path) => {
                let choice = self.config.file_editor.clone();
                // the viewer opens among the panes the maximized panel covers
                if choice.trim() == rift_core::editors::RIFT {
                    self.win.review.maximized = false;
                }
                self.open_file_in(&root.join(&path), &choice);
                return;
            }
            ReviewHit::Jump(staged, path) => {
                self.win.review.jump(staged, &path);
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
                    self.toast_done("Hunk copied".into());
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
                self.win.confirm = Some(ConfirmClose {
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
                            rift_core::review::apply(&root, op, &path, hunk)
                        });
                        let _ = proxy.send_event(UserEvent::ReviewWritten(result.err()));
                    }
                });
            if let Err(e) = spawned {
                self.win.review.notice = Some(format!("could not start the review writer: {e}"));
                return;
            }
            self.review_writer = Some(tx);
        }
        if let Some(tx) = &self.review_writer
            && tx.send((root, ops)).is_err()
        {
            self.review_writer = None;
            self.win.review.notice = Some("the review writer stopped; try again".into());
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
        self.win.review.open && self.win.review.maximized && !self.win.settings.open
    }

    /// The pointer is over the maximized review panel.
    fn over_maximized_review(&self) -> bool {
        let (mx, my) = self.win.mouse;
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
        if self.win.viewer.is_open() && inside(&self.win.viewer_rect) {
            return Some(hand(self.win.viewer_hits.iter().any(|(r, _)| inside(r))));
        }
        let (ex_rect, rv_rect) = self.panel_rects();
        if ex_rect[2] > 0.0 && inside(&ex_rect) {
            return Some(hand(self.win.explorer_hits.iter().any(|(r, _)| inside(r))));
        }
        if rv_rect[2] > 0.0 && inside(&rv_rect) {
            return Some(hand(self.win.review_hits.iter().any(|(r, _)| inside(r))));
        }
        None
    }

    /// A left click on the viewer or a side panel; true when it was theirs.
    fn on_panel_click(&mut self, inside: impl Fn(&[f32; 4]) -> bool) -> bool {
        if self.win.viewer.is_open() {
            if let Some(hit) = self
                .win
                .viewer_hits
                .iter()
                .find(|(r, _)| inside(r))
                .map(|(_, h)| h.clone())
            {
                self.win.viewer_focus = true;
                match hit {
                    ViewerHit::Close => self.win.viewer.close(),
                    ViewerHit::Mode(rendered) => self.win.viewer.set_rendered(rendered),
                    ViewerHit::Menu => {
                        // drops down from the ⋮ button
                        let at = self
                            .win
                            .viewer_hits
                            .iter()
                            .find(|(_, h)| *h == ViewerHit::Menu)
                            .map_or(self.win.mouse, |(r, _)| (r[0], r[1] + r[3] + 4.0));
                        self.open_viewer_menu(at);
                    }
                }
                return true;
            }
            if inside(&self.win.viewer_rect) {
                self.win.viewer_focus = true;
                // its header bar is the pane's title bar: drag to move
                let [x, y, w, _] = self.win.viewer_rect;
                // (beside Settings it is no pane: nothing to move)
                if !self.win.config_viewer && inside(&[x, y, w, crate::views::viewer::HEADER_H]) {
                    self.win.pane_drag = Some(PaneDrag {
                        pane: VIEWER_PANE,
                        from_tab: self.win.active,
                        press: self.win.mouse,
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
            self.win.explorer.focused = true;
            let hit = self
                .win
                .explorer_hits
                .iter()
                .find(|(r, _)| inside(r))
                .map(|(_, h)| h.clone());
            match hit {
                Some(ExplorerHit::Row(i)) => {
                    if let Some(row) = self.win.explorer.rows.get(i).cloned() {
                        self.win.explorer.selected = Some(row.path.clone());
                        if self.win.modifiers.alt_key() {
                            self.drop_file(&row.path);
                        } else if row.is_dir {
                            self.win.explorer.toggle(&row.path);
                        } else {
                            let choice = self.config.file_editor.clone();
                            self.open_file_in(&row.path, &choice);
                        }
                    }
                }
                Some(ExplorerHit::Refresh) => {
                    self.win.explorer.refresh();
                    self.request_review();
                }
                Some(ExplorerHit::Collapse) => self.win.explorer.collapse_all(),
                None => {}
            }
            return true;
        }
        if rv_rect[2] > 0.0 && inside(&rv_rect) {
            if let Some(hit) = self
                .win
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
        let Some(tab) = self.win.tabs.get(self.win.active) else {
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
        // device pixels per logical one: cell backgrounds snap to them
        let scale = self
            .win
            .window
            .as_ref()
            .map_or(1.0, |w| w.scale_factor() as f32);
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
        let mouse = self.win.mouse;
        let prompt_mode = self.config.prompt;
        let input_top = self.config.input_position == rift_core::InputPosition::Top;
        let autosuggest = self.config.autosuggestions && !self.win.history.open;
        let side_pad = self.config.padding();
        let bell_flash = self
            .win
            .bell_flash
            .is_some_and(|t| t.elapsed() < BELL_FLASH);
        let settings_open = self.win.settings.open;
        let active_tab_idx = self.win.active;
        self.follow_explorer();
        self.follow_review();
        self.mark_visible_read();
        self.prune_note_toasts();
        let panel_buttons = tabbar::PanelButtons {
            explorer_open: self.win.explorer.open,
            // lit only while the panel is actually on screen
            review_open: self.review_maximized()
                || self.win.review.open && self.panel_widths(self.logical_size().0).1 > 0.0,
            diff: self.win.review.model().map(|m| {
                m.staged
                    .iter()
                    .chain(&m.unstaged)
                    .fold((0, 0), |(a, d), f| (a + f.added, d + f.removed))
            }),
            notifications_open: self.win.notifications.open,
            unread: self.unread_attention(),
            update: self.update_chip().is_some(),
            remote: self.remote_chip(),
            no_window_buttons: self.win.dropdown.is_some(),
        };
        let notification_entries = if self.win.notifications.open {
            self.notification_entries()
        } else {
            Vec::new()
        };
        // toasts hide under the open popover and the settings overlay
        let toast_entries: Vec<crate::views::notifications::Entry> =
            // over Settings too: an alert raised there (or Try it) would
            // otherwise run out its time unseen
            if self.win.notifications.open {
                Vec::new()
            } else {
                let now = std::time::SystemTime::now();
                self.win.note_toasts
                    .iter()
                    .filter_map(|(id, _)| {
                        if let Some(n) = self.win.app_notes.iter().find(|n| n.id == *id) {
                            return Some(crate::views::notifications::Entry {
                                id: n.id,
                                kind: crate::views::notifications::Kind::App,
                                agent: None,
                                branch: None,
                                title: n.title.clone(),
                                detail: n.detail.clone(),
                                age_secs: 0,
                                unread: true,
                            });
                        }
                        let a = self.attention.iter().find(|a| a.id == *id)?;
                        Some(Self::entry_of(a, now))
                    })
                    .collect()
            };
        let (pane_rects, handles) = self.pane_rects(w, h);
        // where each pane's toasts sit (its grid's bottom, last frame)
        let toast_active = self.win.tabs.get(self.win.active).map(|t| t.active_pane);
        let toast_panes: Vec<(PaneId, [f32; 4], Option<f32>)> = if settings_open {
            Vec::new()
        } else {
            pane_rects
                .iter()
                .filter_map(|pr| {
                    let tab = self.win.tabs.get(self.win.active)?;
                    if tab.zoomed.is_some_and(|z| z != pr.id) {
                        return None;
                    }
                    let grid_bottom = tab
                        .pane(pr.id)?
                        .last_grid
                        .map(|g| g.y + g.rows as f32 * g.line_h);
                    Some((pr.id, pr.rect, grid_bottom))
                })
                .collect()
        };
        let (explorer_w, review_w) = self.panel_widths(w);
        let viewer_label = self.win.viewer.path.as_ref().map(|p| {
            self.win
                .explorer
                .root
                .as_ref()
                .and_then(|r| p.strip_prefix(r).ok())
                .or_else(|| {
                    self.win
                        .review
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
                let name = rift_core::shell_integration::shell_name(configured.as_deref());
                self.shell_label = Some((configured, name.clone()));
                name
            }
        };
        let tab_infos: Vec<tabbar::TabInfo> = self
            .win
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
                    .win
                    .rename
                    .as_ref()
                    .filter(|(ri, _)| *ri == i)
                    .map(|(_, s)| s.clone()),
                editing_selected: self.win.rename_fresh,
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
            .chain(self.win.settings.tab.then(|| tabbar::TabInfo {
                title: "Settings".into(),
                panes: 0,
                activity: false,
                alert: None,
                color: None,
                sync: false,
                editing: None,
                editing_selected: false,
                subtitle: self.win.settings.page.label().into(),
                settings: true,
                running: false,
            }))
            .collect();
        // the strip's lit tab: the Settings tab (last) while it shows
        let strip_active = if self.win.settings.open && self.win.settings.tab {
            self.win.tabs.len()
        } else {
            self.win.active
        };
        let rail_w = self.rail_w();
        // one tab and tab_bar_with_one_tab off: no tab in the strip (its
        // buttons and + stay; the rail is gone, rail_w() is 0)
        let strip_tabs: &[tabbar::TabInfo] = if self.tabs_shown() { &tab_infos } else { &[] };
        let close_button = self.config.tab_close_button;
        let pane_dim = self.config.inactive_pane_dim();
        let sticky_headers = self.config.sticky_command_header;
        self.win.dimmed_panes = 0;
        // chrome hover follows the pointer unless a modal surface covers it
        let chrome_mouse =
            (!self.win.palette.open && self.win.confirm.is_none() && !self.win.whats_new.open)
                .then_some(self.win.mouse);
        // a tab dragged past the slop floats under the pointer
        let lifted_tab = self
            .win
            .tab_drag
            .filter(|d| d.live && rail_w == 0.0)
            .map(|d| tabbar::Lifted {
                index: d.index,
                x: self.win.mouse.0 - d.grab,
            });
        let confirm = self
            .win
            .confirm
            .as_ref()
            .map(|c| (c.action.clone(), c.commands.clone()));
        let phones = self.connected_phones();
        // the confirmation is about a quit (it empties the last ordinary window)
        let quits = confirm
            .as_ref()
            .is_some_and(|(a, _)| matches!(a, ConfirmAction::Close(s) if self.quits(*s)));
        let filter_editing = self.win.filter_editing;
        let shell_env = self.shell_env.clone();
        let error_underline = self.config.error_underline;
        // Warp with a background image and translucency: the image at the
        // window opacity, then bg at (1 − image opacity) × window opacity
        let image_veil = (theme.ground_alpha < 1.0
            && self.win.bg_image.as_ref().is_some_and(|p| p.has_image()))
        .then(|| {
            theme
                .bg
                .with_alpha((1.0 - self.bg_image_opacity) * theme.ground_alpha)
        });

        let Self {
            text,
            config: config_ref,
            keymap: keymap_ref,
            ..
        } = self;
        let scene = &mut self.win.scene;
        let tabs = &mut self.win.tabs;
        let completion = &mut self.win.completion;
        let palette = &mut self.win.palette;
        let palette_results = &mut self.win.palette_results;
        let palette_row_hits = &mut self.win.palette_row_hits;
        let settings_names = &mut self.win.settings_names;
        let settings_stops = &mut self.win.settings_stops;
        let settings_state = &mut self.win.settings;
        let search_state = &mut self.win.search;
        let Some(engine) = text.as_mut() else { return };
        let ed_char_w = engine.measure(
            "0",
            editor::FONT_SIZE,
            rift_renderer::mono_family(),
            rift_renderer::glyphon::Weight::NORMAL,
        );
        let mut active_ed_cols = 80usize;

        scene.clear();
        if let Some(veil) = image_veil {
            scene.layer(0).rect([0.0, 0.0], [w, h], veil, 0.0);
        }
        let mut hits = Vec::new();
        let mut focus_hits = Vec::new(); // lowest priority, appended last
        // chips that only explain themselves on hover (Elapsed)
        let mut info_chips = Vec::new();
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
        } else if self.win.history.open {
            Some(crate::views::history::panel_height(&self.win.history))
        } else if completion.open && !completion.items.is_empty() {
            Some(completion_view::panel_height(completion))
        } else {
            None
        };

        // ── layer 0: chrome ──
        let mut over = rift_renderer::Layer::default();
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
                close_button,
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
                close_button,
                &mut hits,
            );
        } else {
            tabbar::render(
                scene.layer(0),
                &mut over,
                engine,
                &theme,
                strip_tabs,
                strip_active,
                w,
                blink,
                panel_buttons,
                chrome_mouse,
                lifted_tab,
                close_button,
                &mut hits,
            );
        }
        // the lifted tab floats over everything below the menus
        let l5 = scene.layer(5);
        l5.quads.extend(over.quads);
        l5.texts.extend(over.texts);

        let Some(tab) = tabs.get_mut(active_tab_idx) else {
            self.win.hits = hits;
            self.win.info_chips = info_chips;
            return;
        };
        let split = tab.effective_split();
        let clock_now = Instant::now();
        tab.sync_clock(clock_now);
        let elapsed = tab.elapsed(config_ref.elapsed_counts, clock_now);

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
        // either side (these hits precede the panes' own). Like Warp's, a
        // translucent line over the panes' own wash: it stays the same step
        // brighter whatever shows through a translucent window.
        for handle in handles.iter().filter(|_| !settings_open) {
            let [hx, hy, hw, hh] = handle.rect;
            scene
                .layer(0)
                .rect([hx, hy], [hw, hh], inactive_dim(&theme, INACTIVE_LIFT), 0.0);
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
                target: ClickTarget::HandleDrag(handle.path.clone(), handle.index),
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
            // prompt chips that do not fit one row wrap onto more (Warp)
            let chip_rows = if prompt_mode == rift_core::PromptMode::Shell {
                1
            } else {
                let views = editor::pane_chip_views(config_ref, &theme, pane, elapsed);
                editor::chip_rows(engine, &views, editor_area_w)
            };
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
            // the window's own grid, before a phone's `fit`
            let desktop_grid: (usize, usize);
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
                    editor::editor_height(&pane.editor.buffer, prompt_mode, ed_cols, chip_rows)
                };
                let usable_w = (pw - terminal::BLOCK_INDENT).max(cell_w);
                let usable_h = (ph - editor_h).max(line_h);
                text_area = (usable_w, usable_h);
                // input at the top: the grid starts below the editor zone
                let bottom_up =
                    !input_top && config_ref.blocks_start == rift_core::BlocksStart::Bottom;
                let cols = (usable_w / cell_w).floor().max(2.0) as usize;
                let rows = (usable_h / line_h).floor().max(2.0) as usize;
                desktop_grid = (cols, rows);
                let (cols, rows) = pane.fitted(cols, rows);
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
                    cols,
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
                    copied: self
                        .win
                        .copied
                        .filter(|(p, _, at)| *p == pr.id && at.elapsed() < COPIED_SHOWN)
                        .map(|(_, b, _)| b),
                    scale,
                }
            };

            pane.session.resize(
                GridSize {
                    cols: grid.cols,
                    rows: grid.rows,
                },
                cell_px,
            );
            pane.base_grid = Some((desktop_grid.0, desktop_grid.1, cell_px));
            pane.text_area = text_area;
            pane.layout_window = (w, h);
            let view = match docked_h.filter(|_| is_active_pane && editor_vis) {
                Some(panel_h) => {
                    let eh =
                        editor::editor_height(&pane.editor.buffer, prompt_mode, ed_cols, chip_rows);
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
                    chrome_mouse.filter(|_| self.win.pane_drag.is_none()),
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
                editor_vis && prompt_mode != rift_core::PromptMode::Shell,
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
            for hb in out.menu_hits {
                hits.push(Hit {
                    rect: hb.rect,
                    target: ClickTarget::BlockMenu(pr.id, hb.block),
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
                let shell = rift_core::shell_integration::shell_name(config_ref.shell.as_deref());
                let label = format!("Starting {shell}\u{2026}");
                let buf = {
                    use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
                    let mut b = Buffer::new(
                        &mut engine.font_system,
                        Metrics::new(editor::FONT_SIZE, editor::LINE_H),
                    );
                    b.set_text(
                        &mut engine.font_system,
                        &label,
                        &Attrs::new()
                            .family(Family::Name(rift_renderer::mono_family()))
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
                    pr.rect[1] + pr.rect[3] - editor::editor_height("", prompt_mode, 1, 1)
                } else {
                    h - editor::editor_height("", prompt_mode, 1, 1)
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
                let eh =
                    editor::editor_height(&pane.editor.buffer, prompt_mode, ed_cols, chip_rows);
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
                    elapsed,
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
                    use rift_core::PromptChip as C;
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
                    } else if *chip == C::Elapsed {
                        info_chips.push((*rect, pr.id, *chip));
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
                        rift_core::shell_integration::shell_name(config_ref.shell.as_deref()),
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
                && sticky_headers
                && let Some(sticky) = sticky_block.filter(|_| !followed_live)
                && let Some(sh) = terminal::render_sticky_header(
                    scene.layer(2),
                    engine,
                    &theme,
                    &view,
                    &pane.session,
                    sticky,
                    pane.selected_block == Some(sticky),
                )
            {
                pane.sticky_hit = Some((sh.rect, sticky));
                // its buttons act on the block as on its own header; only the
                // rest of it scrolls to the block (the first matching hit wins)
                let id = pr.id;
                let front = sh
                    .chevron
                    .iter()
                    .map(|h| (h.rect, ClickTarget::BlockChevron(id, h.block)))
                    .chain(
                        sh.copy
                            .iter()
                            .map(|h| (h.rect, ClickTarget::BlockCopy(id, h.block))),
                    )
                    .chain(
                        sh.menu
                            .iter()
                            .map(|h| (h.rect, ClickTarget::BlockMenu(id, h.block))),
                    )
                    .chain(std::iter::once((
                        sh.rect,
                        ClickTarget::JumpToBlock(id, sticky),
                    )))
                    .map(|(rect, target)| Hit { rect, target });
                hits.extend(front);
            }

            // inactive panes wash lighter (darker on a light theme); the
            // focused one's title bar carries the accent edge
            if split && !is_active_pane && pane_dim > 0.0 {
                scene.layer(2).rect(
                    [pr.rect[0], pr.rect[1]],
                    [pr.rect[2], pr.rect[3]],
                    inactive_dim(&theme, pane_dim),
                    0.0,
                );
                self.win.dimmed_panes += 1;
            }
        }

        // ── layer 1: side panels beside the panes ──
        self.win.explorer_hits = if explorer_w > 0.0 && !settings_open {
            crate::views::explorer::render(
                scene.layer(1),
                engine,
                &theme,
                &mut self.win.explorer,
                [rail_w, TAB_BAR_H, explorer_w, h - TAB_BAR_H],
            )
        } else {
            Vec::new()
        };
        self.win.review_hits =
            if self.win.review.open && self.win.review.maximized && !settings_open {
                // over the panes and their overlays (layer 2), under the palette
                crate::views::review_panel::render(
                    scene.layer(3),
                    engine,
                    &theme,
                    &mut self.win.review,
                    [rail_w, TAB_BAR_H, w - rail_w, h - TAB_BAR_H],
                )
            } else if review_w > 0.0 && !settings_open {
                crate::views::review_panel::render(
                    scene.layer(1),
                    engine,
                    &theme,
                    &mut self.win.review,
                    [w - review_w, TAB_BAR_H, review_w, h - TAB_BAR_H],
                )
            } else {
                Vec::new()
            };
        // ── layer 2: the file viewer in its own pane of the split ──
        self.win.viewer_hits.clear();
        self.win.viewer_rect = [0.0; 4];
        if let (Some(label), Some(rect), false) = (&viewer_label, viewer_pane, settings_open) {
            self.win.viewer_rect = rect;
            self.win.viewer_hits = crate::views::viewer::render(
                scene.layer(2),
                engine,
                &theme,
                &mut self.win.viewer,
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
            } else if pane_dim > 0.0 {
                scene.layer(3).rect(
                    [rect[0], rect[1]],
                    [rect[2], rect[3]],
                    inactive_dim(&theme, pane_dim),
                    0.0,
                );
                self.win.dimmed_panes += 1;
            }
        }

        // visual bell: brief full-window flash, over Settings and dialogs
        // too (on layer 2 the Settings page hid all but the tab bar)
        if bell_flash {
            scene
                .layer(6)
                .rect([0.0, 0.0], [w, h], theme.fg.with_alpha(0.10), 0.0);
        }

        // ── layer 2: history pane, docked like the completion panel ──
        if editor_active && self.win.history.open && !palette.open {
            // first in the hit list: the panel covers rows (the welcome
            // list) whose hits would otherwise take its clicks
            let mut panel_hits = Vec::new();
            crate::views::history::render(
                scene.layer(2),
                engine,
                &theme,
                &self.win.history,
                editor_zone,
                editor_pane,
                chrome_mouse,
                &mut panel_hits,
            );
            hits.splice(0..0, panel_hits);
        }
        // ── layer 2: completion popup (above editor) ──
        if editor_active && completion.open && !palette.open && !self.win.history.open {
            let (prefix_len, line_len) = tabs
                .get(active_tab_idx)
                .and_then(|t| t.active())
                .map(|p| {
                    // the whole word is the completed prefix (see `CompletionRequest::run`)
                    let (b, c) = (&p.editor.buffer, p.editor.cursor);
                    let (start, _) = rift_core::completion::current_token(b, c);
                    (rift_core::completion::token_end(b, c) - start, b.len())
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
        if !self.win.preedit.is_empty()
            && let Some((px, py)) = ime_spot
        {
            let size = if editor_active {
                editor::FONT_SIZE
            } else {
                font_size
            };
            let tw = engine.measure(
                &self.win.preedit,
                size,
                rift_renderer::mono_family(),
                rift_renderer::glyphon::Weight::NORMAL,
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
                use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping};
                let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.4));
                b.set_text(
                    &mut engine.font_system,
                    &self.win.preedit,
                    &Attrs::new().family(Family::Name(rift_renderer::mono_family())),
                    Shaping::Advanced,
                    None,
                );
                b
            };
            layer2.text(buf, px, py, theme.fg);
        }
        self.win.ime_spot = ime_spot;

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
            self.win.search_chips = chips;
        } else {
            self.win.search_chips.clear();
        }

        // ── layer 3: the Settings tab's page ──
        if settings_open {
            // it covers the content area: only the tab strip (and rail)
            // stays clickable beside its controls
            let mut area = [rail_w, TAB_BAR_H, w - rail_w, h - TAB_BAR_H];
            hits.retain(|h| h.rect[1] + h.rect[3] <= TAB_BAR_H || h.rect[0] + h.rect[2] <= rail_w);
            // config.toml opened from Settings: beside the page, in this tab
            if self.win.config_viewer
                && let Some(label) = &viewer_label
            {
                let vw = (area[2] * 0.45).clamp(320.0, (area[2] - 360.0).max(320.0));
                area[2] -= vw;
                let rect = [area[0] + area[2], area[1], vw, area[3]];
                self.win.viewer_rect = rect;
                self.win.viewer_hits = crate::views::viewer::render(
                    scene.layer(3),
                    engine,
                    &theme,
                    &mut self.win.viewer,
                    label,
                    rect,
                    viewer_has_focus,
                );
                scene
                    .layer(3)
                    .rect([rect[0], rect[1]], [1.0, rect[3]], theme.border, 0.0);
            }
            focus_hits.clear();
            let mut list = rift_renderer::Layer::default();
            settings_state.view_h = area[3];
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
        if self.win.context_menu.open {
            self.win.context_menu_rects = context_menu::render(
                scene.layer(5),
                engine,
                &theme,
                &self.win.context_menu,
                keymap_ref,
                w,
                h,
            );
        } else {
            self.win.context_menu_rects.clear();
        }
        (self.win.chip_menu_card, self.win.chip_menu_rows) = match &self.win.chip_menu {
            Some(menu) => crate::views::chip_menu::render(
                scene.layer(5),
                engine,
                &theme,
                menu,
                [w, h],
                TAB_BAR_H + 4.0,
                self.win.mouse,
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
            .win
            .window
            .as_ref()
            .map_or(1.0, |w| w.scale_factor() as f32);
        self.win.note_toast_hits = crate::views::notifications::render_toasts(
            scene.layer(5),
            engine,
            &theme,
            &toast_entries,
            anchor[1] + anchor[3] + 10.0,
            w,
            scale,
        );
        if self.win.notifications.open {
            self.win.notifications_hits = crate::views::notifications::render(
                scene.layer(5),
                engine,
                &theme,
                &mut self.win.notifications,
                &notification_entries,
                anchor[0] + anchor[2] + 4.0,
                anchor[1] + anchor[3] + 6.0,
                w,
                h,
                scale,
            );
        } else {
            self.win.notifications_hits = Default::default();
        }
        if let Some(hs) = &self.win.hints {
            let items: Vec<(&str, [f32; 4], bool)> = hs
                .items
                .iter()
                .map(|(l, r, t)| (l.as_str(), *r, matches!(t, HintTarget::Open(_))))
                .collect();
            crate::views::hints::render(scene.layer(5), engine, &theme, &items, &hs.typed);
        }

        // ── layer 6: toasts + close/quit warning (modal: eats every click) ──
        // each in its pane, right above the command line; a pane out of
        // view (another tab, the Settings tab) sends it to the corner
        self.win.toast_hits.clear();
        {
            use crate::views::confirm::{ToastView, toasts};
            let shown_panes = &toast_panes;
            let mut groups: Vec<(Option<PaneId>, Vec<&Toast>)> = Vec::new();
            for t in &self.win.toasts {
                // its pane out of view (or none): the pane you are in; the
                // corner only while no pane shows (the Settings tab)
                let shown = |p: &PaneId| shown_panes.iter().any(|(id, ..)| id == p);
                let at = t.pane.filter(shown).or(toast_active.filter(shown));
                match groups.iter_mut().find(|(a, _)| *a == at) {
                    Some((_, list)) => list.push(t),
                    None => groups.push((at, vec![t])),
                }
            }
            for (at, list) in groups {
                let (right, bottom, max_w) =
                    match at.and_then(|p| shown_panes.iter().find(|(id, ..)| *id == p)) {
                        // in the command line's chip row (right of the
                        // folder chip), clear of the output above it; no
                        // room under the grid (a full-screen program, raw
                        // input): the pane's bottom corner
                        Some((_, r, Some(g))) if !input_top && *g + 50.0 <= r[1] + r[3] => {
                            (r[0] + r[2] - 14.0, g + 47.0, r[2] - 124.0)
                        }
                        Some((_, r, _)) => (r[0] + r[2] - 14.0, r[1] + r[3] - 12.0, r[2] - 28.0),
                        None => (w - 16.0, h - 16.0, (w - 32.0).min(560.0)),
                    };
                // one line: a multi-line message (a parse error) runs on
                let lines: Vec<String> = list
                    .iter()
                    .map(|t| t.text.split_whitespace().collect::<Vec<_>>().join(" "))
                    .collect();
                let views: Vec<ToastView> = list
                    .iter()
                    .zip(&lines)
                    .map(|(t, line)| ToastView {
                        id: t.id,
                        kind: t.kind,
                        text: line,
                        action: t.action.map(ToastAction::label),
                        sticky: t.ttl().is_none(),
                    })
                    .collect();
                let hits = toasts(scene.layer(6), engine, &theme, &views, right, bottom, max_w);
                self.win.toast_hits.extend(hits);
            }
        }
        // What's New sits under a confirm dialog (both modal, layer 6)
        if self.win.whats_new.open {
            hits.clear();
            focus_hits.clear();
            self.win.whats_new_hits = crate::views::whats_new::render(
                scene.layer(6),
                engine,
                &theme,
                &mut self.win.whats_new,
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
                    let quit = quits;
                    let (title, label) = match scope {
                        _ if quit => ("Quit Rift?", "Quit"),
                        CloseScope::Pane(_) => ("Close pane?", "Close pane"),
                        CloseScope::Tab(_) => ("Close tab?", "Close tab"),
                        CloseScope::OtherTabs(_) => ("Close other tabs?", "Close tabs"),
                        CloseScope::Window | CloseScope::App => ("Close window?", "Close window"),
                    };
                    Dialog {
                        title,
                        description: close_description(
                            commands.len(),
                            if quit { &phones } else { &[] },
                        ),
                        note: if quit {
                            "Every tab and shell ends; Rift starts fresh next time. This cannot be undone."
                        } else {
                            "This cannot be undone. Cancel to keep the session open."
                        },
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
                    note: "Hooks load only in Rift (LC_TERMINAL). Remove the files to undo.",
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
                ConfirmAction::RevokeDevice { name, .. } => {
                    update_title = format!("Revoke {name}?");
                    Dialog {
                        title: &update_title,
                        description:
                            "It disconnects now and cannot reconnect without pairing again.".into(),
                        note: "",
                        lines: commands,
                        empty_line: "",
                        confirm_label: "Revoke",
                        cancel_label: "Cancel",
                        destructive: true,
                        processes: false,
                    }
                }
                ConfirmAction::AllowOperate { name, .. } => {
                    update_title = format!("Let {name} run commands?");
                    Dialog {
                        title: &update_title,
                        description: ALLOW_OPERATE.into(),
                        note: "",
                        lines: commands,
                        empty_line: "",
                        confirm_label: "Allow",
                        cancel_label: "Cancel",
                        destructive: false,
                        processes: false,
                    }
                }
                ConfirmAction::Update(plan) => {
                    update_title = format!("Update to Rift v{}?", plan.manifest.version);
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
            self.win.confirm_rects =
                crate::views::confirm::render(scene.layer(6), engine, &theme, &dialog, w, h, scale);
        }
        self.win.prompt_editor_layout = match &self.win.prompt_editor {
            Some(ed) => crate::views::prompt_editor::render(
                scene.layer(6),
                engine,
                &theme,
                ed,
                [w, h],
                self.win.mouse,
            ),
            None => Default::default(),
        };

        // pane drag: the drop zone under the pointer (the half of a pane, a
        // tab, or the empty strip for a new tab)
        if let Some(d) = self.win.pane_drag.as_ref().filter(|d| d.live) {
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
                (self.win.mouse.0 + 14.0, self.win.mouse.1 + 22.0),
                w,
                h,
            );
        }

        hits.extend(focus_hits);
        self.win.hits = hits;
        self.win.info_chips = info_chips;
        // a background tab's spinner turns too
        self.win.any_running = any_running
            || self
                .win
                .tabs
                .iter()
                .any(|t| t.panes.iter().any(PaneState::running));
        self.win.editor_active = editor_active;
        self.win.boot_until = boot_until;
        self.win.editor_anchor = editor_anchor;
        self.win.editor_cols = active_ed_cols;
    }

    /// The shortcut of `action`, as key caps (`⇧⌘G`).
    fn tip_keys(&self, action: &str) -> Option<String> {
        self.keymap.preferred(action).map(|b| b.chips().concat())
    }

    /// What a click target does, or its full label; `None` for targets that
    /// explain themselves (rows, the editor, the grid).
    fn tip_for(&self, target: &ClickTarget) -> Option<(String, Option<String>)> {
        use crate::views::history::Scope;
        let pane_of = |id: PaneId| self.win.tabs.iter().find_map(|t| t.pane(id));
        let s = |t: &str| Some(t.to_string());
        Some(match target {
            ClickTarget::TabSelect(i) => {
                let tab = self.win.tabs.get(*i)?;
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
                self.win
                    .tabs
                    .get(*i)
                    .filter(|t| *i == self.win.active && !t.root.is_split())
                    .and_then(|_| self.tip_keys("close_pane")),
            ),
            ClickTarget::TabNew => ("New tab".into(), self.tip_keys("new_tab")),
            ClickTarget::SettingsEditPrompt => ("Edit prompt".into(), None),
            ClickTarget::SettingsPlaySound(_) => ("Play this sound".into(), None),
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
            ClickTarget::BlockMenu(..) => ("More actions".into(), None),
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
            ClickTarget::RemoteChip => {
                self.remote.status.as_ref()?;
                (format!("Connected: {}", self.remote_clients_text()), None)
            }
            ClickTarget::SettingsOpenConfig => (
                "Open in the default editor".into(),
                rift_core::config::config_path()
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
    /// viewer first (they own their regions), then `self.win.hits` in order. A
    /// hit without a tooltip still blocks the ones under it.
    fn collect_tips(&self) -> Vec<crate::views::tooltip::Tip> {
        use crate::views::explorer::ExplorerHit;
        use crate::views::review_panel::ReviewHit;
        use crate::views::tooltip::Tip;
        use crate::views::viewer::ViewerHit;
        let mut tips = Vec::new();
        // the prompt editor is modal: only its chips explain themselves
        if let Some(ed) = &self.win.prompt_editor {
            use crate::views::prompt_editor::Target;
            for (rect, target) in &self.win.prompt_editor_layout.hits {
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
            tips.push(Tip::new(self.win.prompt_editor_layout.card, "", None));
            return tips;
        }
        for (rect, hit) in &self.win.viewer_hits {
            let (text, keys) = match hit {
                ViewerHit::Close => ("Close file", self.tip_keys("close_pane")),
                ViewerHit::Menu => ("More actions", None),
                _ => ("", None),
            };
            tips.push(Tip::new(*rect, text, keys));
        }
        for (rect, hit) in &self.win.explorer_hits {
            tips.push(match hit {
                ExplorerHit::Refresh => Tip::new(*rect, "Refresh", None),
                ExplorerHit::Collapse => Tip::new(*rect, "Collapse all", None),
                ExplorerHit::Row(i) => match self.win.explorer.rows.get(*i) {
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
        for (rect, hit) in &self.win.review_hits {
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
                ReviewHit::Maximize if self.win.review.maximized => "Restore",
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
        // a link names its target and what opens it (as in Warp)
        let open_with = if self.config.link_click.trim() == "open" {
            "Click"
        } else {
            "⌘ Click"
        };
        for pane in self
            .win
            .tabs
            .get(self.win.active)
            .into_iter()
            .flat_map(|t| &t.panes)
        {
            for (rect, url) in &pane.url_hits {
                tips.push(Tip::new(*rect, url.clone(), Some(open_with.into())));
            }
        }
        // Elapsed: when its clock started, with the date after a day; over
        // the command line's own hit, so first
        let mode = self.config.elapsed_counts;
        let now = Instant::now();
        for (rect, pane, chip) in &self.win.info_chips {
            let tab = self.win.tabs.iter().find(|t| t.pane(*pane).is_some());
            let shown = tab.and_then(|t| t.elapsed(mode, now));
            let line =
                crate::views::prompt_chips::elapsed_line(mode, tab.and_then(|t| t.started), shown);
            let body = match line {
                Some(line) => format!("{line}\n\n{}", chip.about()),
                None => chip.about().to_string(),
            };
            let tip = Tip::card(*rect, chip.label(), body);
            tips.push(if shown.is_some() {
                tip.with_action("Reset")
            } else {
                tip
            });
        }
        for hit in &self.win.hits {
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
        !self.win.pointer_in
            || self.win.palette.open
            || self.win.confirm.is_some()
            || self.win.context_menu.open
            || self.win.chip_menu.is_some()
            // the prompt editor's chips have cards, until one is dragged
            || self
                .win.prompt_editor
                .as_ref()
                .is_some_and(|e| e.drag.as_ref().is_some_and(|d| d.live))
            || self.win.whats_new.open
            || self.win.rename.is_some()
            || self.win.drag.is_some()
            || self.win.pending_sel.is_some()
            || self.win.editor_drag.is_some()
            || self.win.tab_drag.is_some()
            || self.win.pane_drag.is_some()
            || self.win.slider_drag.is_some()
    }

    /// Track the control under the pointer and draw its tooltip once it has
    /// rested there for `tooltip::DELAY` (at once right after another one).
    fn tooltip_pass(&mut self, w: f32, h: f32) {
        use crate::views::tooltip::{DELAY, WARM};
        let tips = self.collect_tips();
        let (mx, my) = self.win.mouse;
        // an interactive card stays while the pointer moves from its control
        // into it
        let sticky = self
            .win
            .tip_shown
            .as_ref()
            .zip(self.win.tip_drawn.as_ref())
            .filter(|(t, d)| {
                t.action.is_some() && crate::views::tooltip::bridged(t.rect, d.rect, mx, my)
            })
            .and_then(|(t, _)| tips.iter().find(|n| n.rect == t.rect).cloned());
        let hovered = if self.tooltip_blocked() {
            None
        } else {
            sticky.or_else(|| tips.iter().find(|t| t.contains(mx, my)).cloned())
        };
        let now = Instant::now();
        let rect = hovered.as_ref().map(|t| t.rect);
        if self.win.tip_hover.map(|(r, _)| r) != rect {
            if self.win.tip_shown.is_some() {
                self.win.tip_hidden_at = Some(now);
            }
            let warm = self.win.tip_hidden_at.is_some_and(|t| now < t + WARM);
            self.win.tip_hover = rect.map(|r| {
                (
                    r,
                    if warm {
                        now.checked_sub(DELAY).unwrap_or(now)
                    } else {
                        now
                    },
                )
            });
            self.win.tip_suppressed = false;
        }
        let shown = match (hovered, self.win.tip_hover) {
            (Some(tip), Some((_, since)))
                if !tip.text.is_empty() && !self.win.tip_suppressed && now >= since + DELAY =>
            {
                Some(tip)
            }
            _ => None,
        };
        if self.win.tip_shown.is_some() && shown.is_none() && rect.is_some() {
            self.win.tip_hidden_at = Some(now);
        }
        self.win.tip_drawn = None;
        if let (Some(tip), Some(engine)) = (&shown, self.text.as_mut()) {
            self.win.tip_drawn = Some(crate::views::tooltip::render(
                self.win.scene.layer(7),
                engine,
                &self.theme,
                tip,
                (w, h),
                self.win.mouse,
            ));
        }
        self.win.tip_shown = shown;
        self.win.tips = tips;
    }

    /// When the tooltip of the control under the pointer is due (not yet drawn).
    fn tooltip_due(&self) -> Option<Instant> {
        let (_, since) = self.win.tip_hover?;
        (self.win.tip_shown.is_none() && !self.win.tip_suppressed)
            .then_some(since + crate::views::tooltip::DELAY)
    }

    /// A press on the shown hover card: its button acts, and the press never
    /// reaches what the card covers. Returns true when it landed on the card.
    fn press_tip_card(&mut self, window: &Window) -> bool {
        let (mx, my) = self.win.mouse;
        let inside =
            |r: &[f32; 4]| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3];
        let (Some(tip), Some(drawn)) = (&self.win.tip_shown, &self.win.tip_drawn) else {
            return false;
        };
        if tip.action.is_none() || !inside(&drawn.rect) {
            return false;
        }
        if drawn.action.as_ref().is_some_and(inside) {
            let chip = tip.rect;
            let pane = self
                .win
                .info_chips
                .iter()
                .find(|(r, _, _)| *r == chip)
                .map(|(_, p, _)| *p);
            if let Some(pane) = pane {
                self.reset_tab_timer_of(pane);
            }
        }
        window.request_redraw();
        true
    }

    /// Zero the "Time on this tab" of the tab holding `pane`.
    fn reset_tab_timer_of(&mut self, pane: PaneId) {
        let mode = self.config.elapsed_counts;
        if let Some((_, tab)) = self.find_tab_of_pane(pane) {
            tab.reset_clock(mode, Instant::now());
            self.save_session();
        }
    }

    /// A press, key or wheel: the tooltip goes until the pointer moves on.
    fn hide_tooltip(&mut self, window: &Window) {
        self.win.tip_suppressed = true;
        if self.win.tip_shown.is_some() {
            window.request_redraw();
        }
    }

    /// A frame for new program output: at once, or [`OUTPUT_FRAME`] after
    /// the last one.
    fn request_output_frame(&mut self) {
        let due = self.win.last_frame + OUTPUT_FRAME;
        if Instant::now() >= due {
            if let Some(w) = &self.win.window {
                w.request_redraw();
            }
        } else {
            self.win.output_frame_due.get_or_insert(due);
        }
    }

    fn redraw(&mut self, window: &Window) {
        self.win.last_frame = Instant::now();
        self.win.output_frame_due = None;
        let scale = window.scale_factor() as f32;
        if self.win.gpu.is_none() {
            return;
        }
        self.request_git();

        let (pw, ph) = {
            let gpu = self.win.gpu.as_ref().unwrap();
            (gpu.config.width, gpu.config.height)
        };
        let (lw, lh) = (pw as f32 / scale, ph as f32 / scale);

        self.build_scene(lw, lh);
        self.tooltip_pass(lw, lh);
        if let Some(spot) = self.win.ime_spot
            && self.win.ime_area_sent != Some(spot)
        {
            self.win.ime_area_sent = Some(spot);
            window.set_ime_cursor_area(
                winit::dpi::LogicalPosition::new(spot.0, spot.1),
                LogicalSize::new(self.cell_w, self.line_h),
            );
        }

        let mut all_quads = Vec::new();
        let mut ranges = Vec::new();
        for layer in &self.win.scene.layers {
            let start = all_quads.len() as u32;
            all_quads.extend_from_slice(&layer.quads);
            ranges.push(start..all_quads.len() as u32);
        }

        let (Some(gpu), Some(quads), Some(text), Some(sprites)) = (
            &mut self.win.gpu,
            &mut self.win.quads,
            &mut self.text,
            &mut self.win.sprites,
        ) else {
            return;
        };
        let sprite_layers: Vec<&[rift_renderer::Sprite]> = self
            .win
            .scene
            .layers
            .iter()
            .map(|l| l.sprites.as_slice())
            .collect();
        sprites.prepare(&gpu.device, &gpu.queue, [lw, lh], &sprite_layers);
        // prepared once; drawn first in every pass (main + capture)
        if let Some(img) = self.win.bg_image.as_mut()
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
        let bg_image = self.win.bg_image.as_ref().filter(|p| p.has_image());

        quads.prepare(&gpu.device, &gpu.queue, [lw, lh], &all_quads);

        let text_layers: Vec<Vec<TextArea<'_>>> = self
            .win
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
        if let Some(path) = self.win.capture.take() {
            crate::capture::capture_frame(
                gpu, quads, text, sprites, bg_image, &ranges, ground, &path,
            );
        }
        let Some(frame) = gpu.acquire() else {
            // no frame (display asleep, window occluded): the uploads queued
            // above live in wgpu's pending writes until a submit, so flush
            // them or every skipped frame leaks its staging memory
            gpu.queue.submit([]);
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
        // RIFT_LATENCY_LOG: key → program output → presented frame
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
            AppAction::NewTab => self.spawn_new_tab(),
            AppAction::ClosePane => {
                // ⌘W on the viewer pane closes the viewer
                if self.viewer_focused() {
                    self.win.viewer.close();
                } else if let Some(tab) = self.win.tabs.get(self.win.active) {
                    self.request_close(CloseScope::Pane(tab.active_pane), event_loop);
                }
            }
            AppAction::CloseWindow => self.request_close(CloseScope::Window, event_loop),
            AppAction::Quit => self.request_close(CloseScope::App, event_loop),
            AppAction::CloseOtherTabs => {
                self.request_close(CloseScope::OtherTabs(self.win.active), event_loop);
            }
            AppAction::NewWindow => self.open_new_window(event_loop),
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
                self.start_rename(self.win.active);
            }
            AppAction::ToggleSyncInputs => {
                if let Some(t) = self.win.tabs.get_mut(self.win.active) {
                    t.sync_inputs = !t.sync_inputs;
                }
            }
            AppAction::ResetTabTimer => {
                let mode = self.config.elapsed_counts;
                if let Some(t) = self.win.tabs.get_mut(self.win.active) {
                    t.reset_clock(mode, Instant::now());
                    self.save_session();
                }
            }
            AppAction::FilterBlock => {
                if let Some((pane_id, block)) = self.selected_or_last_block() {
                    self.toggle_block_filter(pane_id, block);
                }
            }
            AppAction::SelectAll => {
                let editor_active = self.win.editor_active;
                if let Some(pane) = self
                    .win
                    .tabs
                    .get_mut(self.win.active)
                    .and_then(|t| t.active_mut())
                {
                    if editor_active && !pane.editor.buffer.is_empty() {
                        pane.editor.select_all();
                    } else {
                        // select the whole scrollback + screen
                        let mut term = pane.session.term.lock();
                        use rift_pty::alacritty_terminal::grid::Dimensions;
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
                    rift_core::TabLayout::Horizontal => rift_core::TabLayout::Vertical,
                    rift_core::TabLayout::Vertical => rift_core::TabLayout::Horizontal,
                };
                self.save_config();
                self.apply_config();
            }
            AppAction::HintMode => self.start_hints(),
            AppAction::AttentionCenter => {
                if self.win.notifications.open {
                    self.close_notifications();
                } else {
                    self.open_notifications();
                }
            }
            AppAction::ClearTranscripts => self.clear_transcripts(),
            AppAction::TabNext => {
                if !self.win.tabs.is_empty() {
                    self.switch_tab((self.win.active + 1) % self.win.tabs.len());
                }
            }
            AppAction::TabPrev => {
                if !self.win.tabs.is_empty() {
                    self.switch_tab(
                        (self.win.active + self.win.tabs.len() - 1) % self.win.tabs.len(),
                    );
                }
            }
            AppAction::TabSelect(n) => {
                if !self.win.tabs.is_empty() {
                    // Warp: ⌘9 = last tab; out-of-range numbers are no-ops
                    let idx = if n == 9 {
                        self.win.tabs.len() - 1
                    } else {
                        n as usize - 1
                    };
                    if idx < self.win.tabs.len() && idx != self.win.active {
                        self.switch_tab(idx);
                    }
                }
            }
            AppAction::MoveTabLeft | AppAction::MoveTabRight => {
                let n = self.win.tabs.len();
                let target = if action == AppAction::MoveTabLeft {
                    self.win.active.checked_sub(1)
                } else {
                    (self.win.active + 1 < n).then_some(self.win.active + 1)
                };
                if let Some(t) = target {
                    self.win.tabs.swap(self.win.active, t);
                    self.win.active = t;
                    self.save_session();
                }
            }
            AppAction::ReopenClosedTab => {
                if let Some(cwd) = self.win.closed_tabs.pop() {
                    self.spawn_tab_at(cwd);
                }
            }
            AppAction::ScrollUp | AppAction::ScrollDown => {
                let delta = if action == AppAction::ScrollUp {
                    10i64
                } else {
                    -10
                };
                if let Some(tab) = self.win.tabs.get_mut(self.win.active)
                    && let Some(pane) = tab.active_mut()
                {
                    pane.view_offset = (pane.view_offset as i64 + delta).max(0) as usize;
                }
            }
            AppAction::SplitRight => self.split_active(Axis::Horizontal),
            AppAction::SplitDown => self.split_active(Axis::Vertical),
            AppAction::TogglePalette => {
                if self.win.palette.open {
                    self.win.palette.open = false;
                } else {
                    self.open_palette(crate::views::palette::PaletteMode::Commands);
                }
            }
            AppAction::OpenSettings => {
                if self.win.settings.open {
                    self.win.settings.open = false;
                } else {
                    // back to the Settings tab as it was; a closed one
                    // starts again on General (`close_tab`)
                    self.win.settings.open_page(self.win.settings.page);
                }
            }
            AppAction::BookmarkToggle => {
                // selected block wins; else the most recent finished block
                if let Some((pane_id, id)) = self.selected_or_last_block()
                    && let Some(pane) = self
                        .win
                        .tabs
                        .get(self.win.active)
                        .and_then(|t| t.pane(pane_id))
                {
                    pane.session.blocks.lock().unwrap().toggle_bookmark(id);
                }
            }
            AppAction::BookmarkPrev | AppAction::BookmarkClosestPrev => self.jump_bookmark(-1),
            AppAction::BookmarkNext | AppAction::BookmarkClosestNext => self.jump_bookmark(1),
            AppAction::Copy => {
                let editor_sel = self
                    .win
                    .tabs
                    .get(self.win.active)
                    .and_then(|t| t.active())
                    .filter(|_| self.win.editor_active)
                    .and_then(|p| p.editor.selected_text().map(str::to_string));
                if let Some(text) = editor_sel {
                    self.copy_to_clipboard(text);
                    return;
                }
                if let Some(text) = self.selection_text() {
                    self.copy_to_clipboard(text);
                    if let Some(w) = &self.win.window {
                        w.request_redraw();
                    }
                    return;
                }
                // no selection: copy the last finished block's output
                let text = self
                    .win
                    .tabs
                    .get(self.win.active)
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
                if self.win.viewer.find.is_some() {
                    self.win.viewer.find = None;
                } else {
                    self.win.viewer.open_find();
                }
                self.blink_epoch = Instant::now();
            }
            AppAction::Search => {
                if self.win.search.open {
                    self.win.search.close();
                } else {
                    self.win.search.open = true;
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
                if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
                    tab.zoomed = None;
                }
                let (lw, lh) = self.logical_size();
                let (rects, _) = self.pane_rects(lw, lh);
                let from_viewer = self.viewer_focused();
                if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
                    let from = if from_viewer {
                        VIEWER_PANE
                    } else {
                        tab.active_pane
                    };
                    match focus_neighbor(&rects, from, dir) {
                        // the viewer pane takes focus without being a shell
                        Some(VIEWER_PANE) => self.win.viewer_focus = true,
                        Some(next) => {
                            if let Some(prev) = tab.active_mut() {
                                prev.selected_block = None;
                            }
                            tab.active_pane = next;
                            self.win.viewer_focus = false;
                        }
                        None => {}
                    }
                }
            }
            AppAction::ClearBlocks => {
                if let Some(pane) = self
                    .win
                    .tabs
                    .get_mut(self.win.active)
                    .and_then(|t| t.active_mut())
                    && !pane.alt_screen
                {
                    pane.session.clear_screen_and_blocks();
                    pane.view_offset = 0;
                    pane.last_display_len = 0;
                    pane.selected_block = None;
                    self.win.search.set_matches(terminal::Found::default());
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
                if let Some(pane) = self
                    .win
                    .tabs
                    .get_mut(self.win.active)
                    .and_then(|t| t.active_mut())
                {
                    pane.view_offset = 0;
                    pane.selected_block = None;
                }
                self.blink_epoch = Instant::now();
            }
            AppAction::ScrollPageUp
            | AppAction::ScrollPageDown
            | AppAction::ScrollTop
            | AppAction::ScrollBottom => {
                if let Some(pane) = self
                    .win
                    .tabs
                    .get_mut(self.win.active)
                    .and_then(|t| t.active_mut())
                {
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
                self.win
                    .settings
                    .open_page(crate::views::settings::Page::Keybindings);
            }
            AppAction::OpenAbout => {
                self.win.palette.open = false;
                self.win.context_menu.close();
                self.win
                    .settings
                    .open_page(crate::views::settings::Page::About);
            }
            AppAction::CheckForUpdates => {
                self.win.palette.open = false;
                self.check_for_updates_now();
            }
            AppAction::UpdateRift => {
                self.win.palette.open = false;
                self.win.context_menu.close();
                self.update_rift();
            }
            AppAction::ToggleExplorer => {
                self.win.explorer.open = !self.win.explorer.open;
                self.win.explorer.focused = self.win.explorer.open;
                if self.win.explorer.open {
                    self.follow_explorer();
                    self.request_review();
                }
            }
            AppAction::ToggleReview => {
                let too_narrow = |app: &Self| {
                    app.win.review.open
                        && !app.win.review.maximized
                        && app.panel_widths(app.logical_size().0).1 == 0.0
                };
                // open but auto-hidden: closing it would change nothing
                // visible, so say why it is not showing instead
                if too_narrow(self) {
                    self.toast("Window too narrow for the review panel: widen it".into());
                } else {
                    self.win.review.open = !self.win.review.open;
                    self.win.review.maximized = false;
                    if self.win.review.open {
                        self.win.review.notice = None;
                        self.request_review();
                        if too_narrow(self) {
                            self.toast("Window too narrow for the review panel: widen it".into());
                        }
                    }
                }
            }
            AppAction::WhatsNew => {
                // a modal over the tab on screen: from Settings it opens over
                // the Settings tab, not over the terminal tab behind it
                self.win.palette.open = false;
                self.win.context_menu.close();
                self.win.whats_new.open();
                rift_core::changelog::mark_seen();
            }
            AppAction::MaximizePane => {
                if let Some(tab) = self.win.tabs.get_mut(self.win.active)
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
                // Warp: the divider beside the pane moves 10 px that way
                let step = crate::pane::RESIZE_STEP;
                let (axis, delta) = match action {
                    AppAction::ResizePaneLeft => (Axis::Horizontal, -step),
                    AppAction::ResizePaneRight => (Axis::Horizontal, step),
                    AppAction::ResizePaneUp => (Axis::Vertical, -step),
                    _ => (Axis::Vertical, step),
                };
                let (lw, lh) = self.logical_size();
                let (x0, w) = self.content_span(lw);
                let area = [x0, TAB_BAR_H, w, lh - TAB_BAR_H];
                if let Some(tab) = self.win.tabs.get_mut(self.win.active)
                    && tab.zoomed.is_none()
                    && tab.root.resize_pane(area, tab.active_pane, axis, delta)
                {
                    self.save_session_file();
                }
            }
            AppAction::EqualizePanes => {
                if let Some(tab) = self.win.tabs.get_mut(self.win.active)
                    && tab.root.equalize_all()
                {
                    self.save_session_file();
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
                        .win
                        .tabs
                        .get(self.win.active)
                        .and_then(|t| t.pane(pane_id))
                        .and_then(|p| terminal::block_output_text(&p.session, block));
                    if let Some(text) = text {
                        self.copy_to_clipboard(text);
                    }
                }
            }
            AppAction::Reinput => {
                if self.win.editor_active
                    && let Some((pane_id, block)) = self.selected_or_last_block()
                    && let Some(cmd) = self.block_command(pane_id, block)
                    && let Some(pane) = self
                        .win
                        .tabs
                        .get_mut(self.win.active)
                        .and_then(|t| t.active_mut())
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
                        .win
                        .tabs
                        .get(self.win.active)
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
                                .win
                                .tabs
                                .get(self.win.active)
                                .and_then(|t| t.pane(pane_id))
                                .and_then(|p| p.last_grid)
                                .map(|g| (g.x + 24.0, g.y + g.line_h))
                                .unwrap_or(self.win.mouse);
                            self.open_context_menu(pane_id, block, pos);
                        }
                    }
                }
            }
        }
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Switch to tab `idx`, clearing transient state on the pane being left.
    fn switch_tab(&mut self, idx: usize) {
        self.win.settings.open = false;
        if let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        {
            pane.selected_block = None;
        }
        self.win.active = idx;
        self.win.tabs[idx].activity = false;
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
        if let Some(dd) = self.win.settings.dropdown.as_mut() {
            let n = dd.options.len();
            match &event.logical_key {
                Key::Named(NamedKey::ArrowUp) => dd.highlight = dd.highlight.saturating_sub(1),
                Key::Named(NamedKey::ArrowDown) => {
                    dd.highlight = (dd.highlight + 1).min(n.saturating_sub(1))
                }
                Key::Named(NamedKey::Enter | NamedKey::Space) => {
                    let dd = self.win.settings.dropdown.take().unwrap_or_default();
                    if let Some((value, _)) = dd.options.get(dd.highlight) {
                        self.set_select(dd.id, value.clone());
                    }
                }
                _ => self.win.settings.dropdown = None,
            }
            return true;
        }
        // the relay field takes the keys while it is edited
        if self.relay_edit_key(event) {
            return true;
        }
        let recording = self.win.settings.recording.is_some();
        if !recording && self.handle_settings_focus_key(event, event_loop, window) {
            return true;
        }
        if self.win.settings.page == settings::Page::Keybindings
            && self.handle_keybindings_key(event)
        {
            return true;
        }
        let mods = self.win.modifiers;
        let action = input::app_action(event, mods, &self.keymap);
        // Esc clears the search first, then closes the Settings tab
        if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
            if self.win.settings.query.is_empty() {
                self.win.settings.close_tab();
            } else {
                self.win.settings.query.clear();
                self.win.settings.scroll = 0.0;
            }
            return true;
        }
        match action {
            Some(AppAction::OpenSettings) => {
                self.win.settings.open = false;
                self.win.settings.focus = None;
                return true;
            }
            // ⌘W in the Settings tab closes it
            Some(AppAction::ClosePane) => {
                self.win.settings.close_tab();
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
                self.win.settings.open = false;
                self.win.settings.focus = None;
                self.handle_action(a, event_loop);
                return true;
            }
            None => {}
        }
        // typing searches every page (Keybindings filters its own list)
        let plain = !mods.control_key() && !mods.super_key() && !mods.alt_key();
        if plain && self.win.settings.page != settings::Page::Keybindings {
            match &event.logical_key {
                Key::Named(NamedKey::Backspace) if !self.win.settings.query.is_empty() => {
                    self.win.settings.query.pop();
                    self.win.settings.scroll = 0.0;
                    return true;
                }
                Key::Named(NamedKey::Space) if !self.win.settings.query.is_empty() => {
                    self.win.settings.query.push(' ');
                    return true;
                }
                _ => {
                    if let Some(text) = &event.text
                        && !text.is_empty()
                        && !text.chars().any(|c| c.is_control() || c == ' ')
                    {
                        self.win.settings.query.push_str(text);
                        self.win.settings.scroll = 0.0;
                        self.win.settings.focus = None;
                        return true;
                    }
                }
            }
        }
        match event.logical_key {
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowLeft) => {
                self.win.settings.move_page(-1);
                true
            }
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowRight) => {
                self.win.settings.move_page(1);
                true
            }
            Key::Named(NamedKey::Home) => {
                self.win
                    .settings
                    .open_page(crate::views::settings::Page::General);
                true
            }
            Key::Named(NamedKey::End) => {
                self.win
                    .settings
                    .open_page(crate::views::settings::Page::Themes);
                true
            }
            Key::Named(NamedKey::PageUp | NamedKey::PageDown) => {
                let direction = if matches!(event.logical_key, Key::Named(NamedKey::PageUp)) {
                    10.0
                } else {
                    -10.0
                };
                self.win.settings.scroll_by(direction);
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
        let mods = self.win.modifiers;
        if mods.super_key() || mods.control_key() || mods.alt_key() {
            return false;
        }
        let order = settings::focus_order(&self.win.hits);
        // every control of the page, also scrolled away (page coordinates)
        let mut stops = settings::focus_order(
            &self
                .win
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
        if let Some(f) = &self.win.settings.focus
            && !stops.iter().any(|(t, _)| t == f)
        {
            self.win.settings.focus = None;
        }
        // on screen: what Space / Enter press
        let focused = self
            .win
            .settings
            .focus
            .clone()
            .and_then(|f| order.iter().find(|(t, _)| *t == f).cloned());
        match (&event.logical_key, focused) {
            (Key::Named(NamedKey::Tab), _) => {
                let delta = if mods.shift_key() { -1 } else { 1 };
                let next = settings::next_focus(&stops, self.win.settings.focus.as_ref(), delta);
                if let Some(rect) = next
                    .as_ref()
                    .and_then(|n| stops[..scrolled].iter().find(|(t, _)| t == n))
                    .map(|(_, r)| *r)
                {
                    self.scroll_settings_into_view(rect);
                }
                self.win.settings.focus = next;
            }
            (Key::Named(NamedKey::Escape), _) if self.win.settings.focus.is_some() => {
                self.win.settings.focus = None
            }
            (
                Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown),
                Some((ClickTarget::RemoteHistoryList, _)),
            ) => {
                let rows = if matches!(event.logical_key, Key::Named(NamedKey::ArrowUp)) {
                    1
                } else {
                    -1
                };
                self.win.settings.scroll_history(rows);
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
                    "inactive_pane_dim" => settings::dim_slider(self.config.inactive_pane_dim),
                    _ => return true,
                };
                self.set_slider(id, (t + step).clamp(0.0, 1.0));
                self.save_config();
            }
            (key, Some((target, rect)))
                if press_key(key) && !matches!(target, ClickTarget::SettingsSlider(_)) =>
            {
                // press it through the normal pointer routing
                let saved = self.win.mouse;
                self.win.mouse = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
                self.on_left(ElementState::Pressed, event_loop, window);
                self.on_left(ElementState::Released, event_loop, window);
                self.win.mouse = saved;
                // a pressed ↺ disappears: the focus moves on to its control
                if matches!(target, ClickTarget::SettingsReset(_)) {
                    self.win.settings.focus = settings::next_focus(&stops, Some(&target), 1);
                }
            }
            // focused but scrolled away: the first key brings it back
            (key, None) if press_key(key) && self.win.settings.focus.is_some() => {
                if let Some(rect) = self
                    .win
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
        let s = &mut self.win.settings;
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
        let mods = self.win.modifiers;
        if let Some(action) = self.win.settings.recording.clone() {
            let plain = !mods.control_key() && !mods.alt_key() && !mods.super_key();
            match &event.logical_key {
                Key::Named(NamedKey::Escape) if plain => {
                    self.win.settings.recording = None;
                    return true;
                }
                Key::Named(NamedKey::Backspace) if plain && !mods.shift_key() => {
                    self.win.settings.recording = None;
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
            let binding = rift_core::keybindings::Binding {
                ctrl: mods.control_key(),
                shift: mods.shift_key(),
                alt: mods.alt_key(),
                cmd: mods.super_key(),
                key,
            };
            self.win.settings.recording = None;
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
            self.toast_done(format!("{action} → {shown}{note}"));
            return true;
        }
        // type-to-filter
        if mods.super_key() || mods.control_key() {
            return false;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Backspace) if !self.win.settings.kb_filter.is_empty() => {
                self.win.settings.kb_filter.pop();
                self.win.settings.scroll = 0.0;
                true
            }
            Key::Named(NamedKey::Escape) if !self.win.settings.kb_filter.is_empty() => {
                self.win.settings.kb_filter.clear();
                true
            }
            Key::Named(NamedKey::Space) => {
                self.win.settings.kb_filter.push(' ');
                true
            }
            Key::Character(_) => match &event.text {
                Some(t) if !t.chars().any(char::is_control) => {
                    self.win.settings.kb_filter.push_str(t);
                    self.win.settings.scroll = 0.0;
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
            && let Some(pane) = self
                .win
                .tabs
                .get_mut(self.win.active)
                .and_then(|t| t.active_mut())
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
            && let Some(pane) = self
                .win
                .tabs
                .get_mut(self.win.active)
                .and_then(|t| t.active_mut())
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
        if self.win.completion.open {
            match key {
                EditorKey::HistoryUp => {
                    self.win.completion.selected = self.win.completion.selected.saturating_sub(1);
                    self.blink_epoch = Instant::now();
                    return;
                }
                EditorKey::HistoryDown => {
                    if self.win.completion.selected + 1 < self.win.completion.items.len() {
                        self.win.completion.selected += 1;
                    }
                    self.blink_epoch = Instant::now();
                    return;
                }
                EditorKey::Dismiss => {
                    self.win.completion.close();
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
            if self.win.completion.open {
                self.win.completion.cycle_source();
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
            && !self.win.completion.ghost.is_empty()
            && self.completion_pending.is_none()
            && self
                .win
                .tabs
                .get(self.win.active)
                .and_then(|t| t.active())
                .is_some_and(|p| {
                    p.editor.cursor == p.editor.buffer.len() && p.editor.selection().is_none()
                })
        {
            let ghost = self.win.completion.ghost.clone();
            if let Some(pane) = self
                .win
                .tabs
                .get_mut(self.win.active)
                .and_then(|t| t.active_mut())
            {
                pane.editor.insert(&ghost);
            }
            self.win.completion.close();
            self.update_completions();
            self.blink_epoch = Instant::now();
            return;
        }

        let Some(id) = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .map(|p| p.session.id)
        else {
            return;
        };
        self.editor_key_on(id, key, true);
    }

    /// Apply `key` to pane `id`'s command editor (what is the pane's own:
    /// editing, history, submit, ^C, ^D, ^L), then to its synchronized
    /// siblings. `active`: the pane the desktop types into, whose completion
    /// menu and cursor blink follow (the remote input of another pane
    /// leaves them alone).
    fn editor_key_on(&mut self, id: PaneId, key: EditorKey, active: bool) {
        let autocomplete_pairs = self.config.autocomplete_pairs;
        let mut submitted: Option<(PaneId, String)> = None;
        let mut signal: Option<(PaneId, &'static [u8])> = None;
        let mut cut: Option<String> = None;
        let Some(pane) = self.find_tab_of_pane(id).and_then(|(_, t)| t.pane_mut(id)) else {
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
            // nothing to run (Warp): no empty block, and no bare shell
            // prompt line from shells that skip their hooks for it
            EditorKey::Submit if pane.editor.buffer.trim().is_empty() => {
                pane.view_offset = 0;
                edited = false;
            }
            EditorKey::Submit => {
                pane.correction = None;
                let cmd = pane.editor.submit();
                pane.view_offset = 0;
                pane.selected_block = None;
                // stamp prompt-time context onto the block for its header
                let cwd = pane.session.cwd.lock().unwrap().clone();
                let branch = pane.git.as_ref().map(|g| g.branch.clone());
                stamp_submit(&pane.session, cwd, branch, &cmd);
                pane.session.write(format!("{cmd}\n").as_bytes());
                submitted = Some((pane.session.id, cmd));
                edited = false;
            }
            EditorKey::Interrupt => {
                pane.editor.clear();
                pane.session.write(b"\x03");
                signal = Some((pane.session.id, b"\x03"));
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
        if active && (submitted.is_some() || signal.is_some()) {
            self.win.completion.close();
        }
        if let Some(text) = cut {
            self.copy_to_clipboard(text);
        }
        if let Some((id, bytes)) = signal {
            self.write_synced(id, bytes, true);
        }
        if let Some((id, cmd)) = submitted {
            self.submit_synced(id, &cmd);
        }
        if edited {
            if let Some(pane) = self.find_tab_of_pane(id).and_then(|(_, t)| t.pane_mut(id)) {
                pane.clear_selection();
                pane.selected_block = None; // typing returns focus to the input
                pane.correction = None;
            }
            if active {
                self.update_completions();
            }
        }
        if active {
            self.blink_epoch = Instant::now(); // cursor solid while editing
        }
    }

    /// Synchronized inputs: a command submitted in `id` runs in every pane
    /// of its tab, but only in siblings at a prompt (a pane running vim or
    /// another program would receive the command as keystrokes).
    fn submit_synced(&mut self, id: PaneId, cmd: &str) {
        if let Some((_, tab)) = self.find_tab_of_pane(id) {
            tab.start_clock(cmd);
        }
        let ready = self.synced_siblings(id, true);
        let busy = self.input_targets(id).len() - 1 - ready.len();
        for other in ready {
            self.submit_to(other, cmd);
        }
        if busy > 0 {
            self.toast(format!(
                "Not sent to {busy} synchronized pane{} running a program",
                if busy == 1 { "" } else { "s" }
            ));
        }
    }

    fn find_tab_of_pane(&mut self, pane: PaneId) -> Option<(usize, &mut TabState)> {
        self.win
            .tabs
            .iter_mut()
            .enumerate()
            .find(|(_, t)| t.panes.iter().any(|p| p.session.id == pane))
    }

    /// Commands still running inside `scope` (the close warning lists them).
    fn running_in(&self, scope: CloseScope) -> Vec<String> {
        let panes: Vec<&PaneState> = match scope {
            CloseScope::Pane(id) => self.win.tabs.iter().filter_map(|t| t.pane(id)).collect(),
            CloseScope::Tab(i) => self
                .win
                .tabs
                .get(i)
                .map(|t| t.panes.iter().collect())
                .unwrap_or_default(),
            CloseScope::OtherTabs(keep) => self
                .win
                .tabs
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != keep)
                .flat_map(|(_, t)| t.panes.iter())
                .collect(),
            CloseScope::Window => self.win.tabs.iter().flat_map(|t| t.panes.iter()).collect(),
            CloseScope::App => self.all_panes().collect(),
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

    /// Closing `scope` quits Rift: the app, or what empties the last
    /// ordinary window (the drop-down hotkey window does not keep the app
    /// open, and its shells end with it).
    fn quits(&self, scope: CloseScope) -> bool {
        let last = self.ordinary_window_count() <= usize::from(self.win.dropdown.is_none());
        match scope {
            CloseScope::App => true,
            CloseScope::Window => last,
            CloseScope::Tab(_) => last && self.win.tabs.len() == 1,
            CloseScope::Pane(id) => {
                last && self.win.tabs.len() == 1
                    && self.win.tabs[0].panes.len() == 1
                    && self.win.tabs[0].pane(id).is_some()
            }
            CloseScope::OtherTabs(_) => false,
        }
    }

    /// Close/quit entry point: warns first when something is still running,
    /// and before a quit while a phone is connected.
    fn request_close(&mut self, scope: CloseScope, event_loop: &ActiveEventLoop) {
        if self.quits(scope) && self.update.busy {
            // quitting mid-install could leave a half-finished update
            self.update.quit_after = true;
            self.app_note(
                "Rift is updating…".into(),
                "It quits when the update is done".into(),
                AppNoteAction::None,
                false,
            );
            return;
        }
        // a quit ends every shell, so the phone loses what it shows
        let phones = self.quits(scope) && !self.connected_phones().is_empty();
        if self.config.confirm_close_running || phones {
            let commands = if self.config.confirm_close_running {
                // a quit ends the shells of every window
                self.running_in(if self.quits(scope) {
                    CloseScope::App
                } else {
                    scope
                })
            } else {
                Vec::new()
            };
            if !commands.is_empty() || phones {
                self.win.confirm = Some(ConfirmClose {
                    opened: Instant::now(),
                    action: ConfirmAction::Close(scope),
                    commands,
                });
                if let Some(w) = &self.win.window {
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
                    .win
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
                    .win
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != keep)
                    .flat_map(|(_, t)| t.panes.iter().map(|p| p.session.id))
                    .collect();
                for id in ids {
                    self.close_pane(id, event_loop);
                }
                self.win.active = 0;
                self.save_session();
            }
            // one of several windows: it closes, its shells end
            CloseScope::Window if self.close_window() => {}
            CloseScope::Window | CloseScope::App => {
                // a quit is intentional: every shell ends and the next launch
                // starts fresh (an update's restart, a crash or a reboot
                // keep the layout and reattach durable shells instead)
                for p in self.all_panes() {
                    p.session.terminate();
                }
                self.save_fresh_session();
                crate::quit::exit(event_loop);
            }
        }
        if let Some(w) = &self.win.window {
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
                    .win
                    .confirm
                    .as_ref()
                    .is_some_and(|c| confirm_accepts(event.repeat, c.opened.elapsed()))
                    && let Some(c) = self.win.confirm.take()
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
        let page = self.win.viewer.page();
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.win.viewer.close(),
            Key::Named(NamedKey::ArrowDown) => self.win.viewer.scroll_by(20.0),
            Key::Named(NamedKey::ArrowUp) => self.win.viewer.scroll_by(-20.0),
            // ← → scroll the source view sideways (rendered: the code block
            // under the pointer)
            Key::Named(NamedKey::ArrowRight) => self.win.viewer.scroll_x_by(40.0, self.win.mouse.1),
            Key::Named(NamedKey::ArrowLeft) => self.win.viewer.scroll_x_by(-40.0, self.win.mouse.1),
            Key::Named(NamedKey::PageDown | NamedKey::Space) => self.win.viewer.scroll_by(page),
            Key::Named(NamedKey::PageUp) => self.win.viewer.scroll_by(-page),
            Key::Named(NamedKey::Home) => self.win.viewer.scroll_by(f32::NEG_INFINITY),
            Key::Named(NamedKey::End) => self.win.viewer.scroll_by(f32::INFINITY),
            _ => return false,
        }
        true
    }

    /// A press in "Edit prompt": buttons act at once; a chip arms a drag
    /// (a release without travel is a click).
    fn prompt_editor_press(&mut self) {
        use crate::views::prompt_editor::{Drag, Target};
        let (mx, my) = self.win.mouse;
        let hit = self
            .win
            .prompt_editor_layout
            .hits
            .iter()
            .find(|(r, _)| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3])
            .map(|(_, t)| *t);
        let Some(ed) = &mut self.win.prompt_editor else {
            return;
        };
        match hit {
            Some(Target::Palette(chip)) => {
                ed.drag = Some(Drag {
                    chip,
                    from: None,
                    press: self.win.mouse,
                    live: false,
                })
            }
            Some(Target::Chip(i)) => {
                if let Some(&chip) = ed.draft.get(i) {
                    ed.drag = Some(Drag {
                        chip,
                        from: Some(i),
                        press: self.win.mouse,
                        live: false,
                    });
                }
            }
            Some(Target::Remove(i)) => ed.remove(i),
            Some(Target::Restore) => ed.restore_default(),
            Some(Target::Cancel) => self.win.prompt_editor = None,
            Some(Target::Save) => self.save_prompt_editor(),
            None => {}
        }
    }

    /// A release in "Edit prompt": a drag drops into the box (or, from the
    /// box, out of it to remove the chip); a palette chip clicked is added.
    fn prompt_editor_release(&mut self) {
        let layout = &self.win.prompt_editor_layout;
        let (mx, my) = self.win.mouse;
        let b = layout.drop_box;
        let in_box = mx >= b[0] && mx < b[0] + b[2] && my >= b[1] && my < b[1] + b[3];
        let Some(ed) = &mut self.win.prompt_editor else {
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
            let at = crate::views::prompt_editor::drop_index(&layout.slots, d.from, self.win.mouse);
            ed.place(d.chip, at);
        } else if let Some(i) = d.from {
            ed.remove(i);
        }
    }

    /// Keep "Edit prompt"'s chips and close it.
    fn save_prompt_editor(&mut self) {
        let Some(ed) = self.win.prompt_editor.take() else {
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
            .contains(&rift_core::PromptChip::Docker);
        for pane in self.all_panes() {
            let Some(cwd) = pane.session.local_cwd() else {
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

    /// Open a chip's menu for `pane` (focusing it).
    fn open_chip_menu(&mut self, pane: PaneId, kind: crate::views::chip_menu::ChipKind) {
        use crate::views::chip_menu::{ChipKind, ChipMenu};
        let target = ClickTarget::Chip(pane, kind.chip());
        let Some(anchor) = self
            .win
            .hits
            .iter()
            .find(|h| h.target == target)
            .map(|h| h.rect)
        else {
            return;
        };
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        tab.active_pane = pane;
        let Some(p) = tab.pane(pane) else {
            return;
        };
        let cwd = p.session.local_cwd();
        let Some(cwd) = cwd.map(std::path::PathBuf::from) else {
            return;
        };
        self.win.chip_menu = Some(match kind {
            ChipKind::Dir => ChipMenu::dirs(pane, anchor, &cwd),
            ChipKind::Branch => {
                let current = p.git.as_ref().map(|g| g.branch.clone()).unwrap_or_default();
                ChipMenu::branches(pane, anchor, rift_core::git::local_branches(&cwd), &current)
            }
            ChipKind::Php | ChipKind::Python => {
                let vars = p.session.user_vars.lock().unwrap().clone();
                let (list, active) = crate::views::chip_menu::runtime_choices(kind, &vars, &cwd);
                ChipMenu::runtimes(kind, pane, anchor, list, active.as_deref())
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
        let Some(menu) = self.win.chip_menu.take() else {
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
        let Some(menu) = &mut self.win.chip_menu else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.win.chip_menu = None,
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
                if !self.win.modifiers.super_key()
                    && !self.win.modifiers.control_key()
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
        let page = (self.win.review.view_h - 40.0).max(40.0);
        let y = self.win.mouse.1;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.win.review.maximized = false,
            Key::Named(NamedKey::ArrowDown) => self.win.review.scroll_by(40.0),
            Key::Named(NamedKey::ArrowUp) => self.win.review.scroll_by(-40.0),
            Key::Named(NamedKey::ArrowRight) => self.win.review.scroll_x_by(40.0, y),
            Key::Named(NamedKey::ArrowLeft) => self.win.review.scroll_x_by(-40.0, y),
            Key::Named(NamedKey::PageDown | NamedKey::Space) => self.win.review.scroll_by(page),
            Key::Named(NamedKey::PageUp) => self.win.review.scroll_by(-page),
            Key::Named(NamedKey::Home) => self.win.review.scroll_by(f32::NEG_INFINITY),
            Key::Named(NamedKey::End) => self.win.review.scroll_by(f32::INFINITY),
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
            Key::Named(NamedKey::ArrowDown) => self.win.explorer.move_selection(1),
            Key::Named(NamedKey::ArrowUp) => self.win.explorer.move_selection(-1),
            Key::Named(NamedKey::ArrowLeft) => self.win.explorer.left(),
            Key::Named(NamedKey::ArrowRight) => self.win.explorer.right(),
            Key::Named(NamedKey::Escape) => self.win.explorer.focused = false,
            Key::Named(NamedKey::Enter) => {
                let Some(row) = self.win.explorer.selected_row().cloned() else {
                    return true;
                };
                if self.win.modifiers.alt_key() {
                    self.win.explorer.focused = false;
                    self.drop_file(&row.path);
                } else if row.is_dir {
                    self.win.explorer.toggle(&row.path);
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
        let chord = self.win.modifiers.super_key()
            || self.win.modifiers.control_key()
            || self.win.modifiers.alt_key();
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
            Key::Named(NamedKey::ArrowDown) => self.win.notifications.move_selection(1, len),
            Key::Named(NamedKey::ArrowUp) => self.win.notifications.move_selection(-1, len),
            Key::Named(NamedKey::Tab) => {
                let delta = if self.win.modifiers.shift_key() {
                    -1
                } else {
                    1
                };
                let next = self.win.notifications.filter.cycle(delta);
                self.win.notifications.set_filter(next);
            }
            Key::Named(NamedKey::Enter) => {
                let target = self
                    .win
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
        let page = self.win.whats_new.page();
        match &event.logical_key {
            Key::Named(NamedKey::Escape | NamedKey::Enter) => self.win.whats_new.open = false,
            Key::Named(NamedKey::ArrowDown) => self.win.whats_new.scroll_by(40.0),
            Key::Named(NamedKey::ArrowUp) => self.win.whats_new.scroll_by(-40.0),
            Key::Named(NamedKey::PageDown | NamedKey::Space) => self.win.whats_new.scroll_by(page),
            Key::Named(NamedKey::PageUp) => self.win.whats_new.scroll_by(-page),
            Key::Named(NamedKey::Home) => self.win.whats_new.scroll_by(f32::NEG_INFINITY),
            Key::Named(NamedKey::End) => self.win.whats_new.scroll_by(f32::INFINITY),
            _ => {}
        }
    }

    fn perform_confirmed(&mut self, action: ConfirmAction, event_loop: &ActiveEventLoop) {
        match action {
            ConfirmAction::Close(scope) => self.perform_close(scope, event_loop),
            ConfirmAction::Paste { pane, text } => {
                self.paste_raw(pane, &text);
                if let Some(w) = &self.win.window {
                    w.request_redraw();
                }
            }
            ConfirmAction::InstallRemote(profile) => {
                let script = rift_core::shell_integration::data_dir()
                    .map(|d| d.join("remote-install.sh"))
                    .filter(|p| std::fs::write(p, rift_core::ssh::installer_script()).is_ok());
                match script {
                    Some(path) => {
                        let title = format!("install on {}", profile.name);
                        self.open_tab_running(&title, profile.install_command(&path));
                    }
                    None => self.toast_error("Could not write the installer script".into()),
                }
            }
            ConfirmAction::Revert { root, path, hunk } => {
                self.queue_review_write(root, vec![(rift_core::review::Op::Revert, path, hunk)]);
            }
            ConfirmAction::Update(plan) => self.install_update(*plan),
            ConfirmAction::RevokeDevice { id, name } => self.revoke_device(&id, &name),
            ConfirmAction::AllowOperate { id, name } => {
                self.set_device_permission(&id, &name, false)
            }
        }
    }

    /// New tab titled `title` that runs `command` at its first prompt (a
    /// normal block: ssh prompts and output stay visible and interactive).
    fn open_tab_running(&mut self, title: &str, command: String) {
        self.spawn_tab_at(None);
        if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
            tab.custom_title = Some(title.to_string());
            let pane = tab.active_pane;
            self.pending_commands.insert(pane, vec![command]);
        }
        self.save_session();
    }

    /// Directory for a new tab/pane per `working_directory`.
    fn start_dir(&self) -> Option<std::path::PathBuf> {
        let previous = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .and_then(|p| p.session.local_cwd());
        self.config.start_dir(previous.as_deref())
    }

    /// Panes that receive input typed into `pane` (itself, plus every other
    /// pane of its tab while synchronized inputs are on).
    fn input_targets(&self, pane: PaneId) -> Vec<PaneId> {
        match self.win.tabs.iter().find(|t| t.pane(pane).is_some()) {
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
        let Some(tab) = self.win.tabs.iter().find(|t| t.pane(pane).is_some()) else {
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
        let (text_x, input_y) = self.win.editor_anchor;
        let (mx, my) = self.win.mouse;
        let buffer = self
            .win
            .tabs
            .get(self.win.active)?
            .pane(pane_id)?
            .editor
            .buffer
            .clone();
        let rows = editor::wrap_rows(&buffer, self.win.editor_cols);
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
                rift_renderer::mono_family(),
                rift_renderer::glyphon::Weight::NORMAL,
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
        if let Some(tab) = self.win.tabs.get_mut(self.win.active)
            && tab.active_pane != pane_id
        {
            if let Some(prev) = tab.active_mut() {
                prev.selected_block = None;
            }
            tab.active_pane = pane_id;
            return; // first click only focuses; geometry is the old pane's
        }
        let shift = self.win.modifiers.shift_key();
        let Some(offset) = self.editor_offset_at_mouse(pane_id) else {
            return;
        };
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
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
        self.win.completion.close();
        self.blink_epoch = Instant::now();
    }

    fn drag_editor(&mut self, pane_id: PaneId) {
        let Some(offset) = self.editor_offset_at_mouse(pane_id) else {
            return;
        };
        if let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.pane_mut(pane_id))
        {
            pane.editor.click_to(offset, true);
        }
    }

    /// Begin an inline rename: the current title is pre-filled and fully
    /// selected, so the first keystroke replaces it (Backspace clears it).
    fn start_rename(&mut self, i: usize) {
        if let Some(t) = self.win.tabs.get(i) {
            self.win.rename = Some((i, t.display_title()));
            self.win.rename_fresh = true;
        }
    }

    /// Inline tab rename keys: type to edit, Enter commits, Esc cancels.
    fn handle_rename_key(&mut self, event: &input::KeyPress) {
        use winit::keyboard::{Key, NamedKey};
        let Some((i, buf)) = self.win.rename.as_mut() else {
            return;
        };
        let i = *i;
        let fresh = std::mem::take(&mut self.win.rename_fresh);
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
            Key::Named(NamedKey::Escape) => self.win.rename = None,
            Key::Named(NamedKey::Enter) => self.commit_rename(i),
            Key::Named(NamedKey::Backspace) => {
                if self.win.modifiers.super_key() || self.win.modifiers.alt_key() {
                    buf.clear();
                } else {
                    buf.pop();
                }
            }
            Key::Named(NamedKey::Space) => buf.push(' '),
            _ => {
                if !self.win.modifiers.super_key()
                    && !self.win.modifiers.control_key()
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
        if let Some((_, name)) = self.win.rename.take()
            && let Some(tab) = self.win.tabs.get_mut(i)
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
        tab.start_clock(cmd);
        let Some(pane) = tab.pane_mut(id) else { return };
        if !cmd.trim().is_empty() && pane.editor.history.last().map(String::as_str) != Some(cmd) {
            Arc::make_mut(&mut pane.editor.history).push(cmd.to_string());
        }
        pane.view_offset = 0;
        pane.selected_block = None;
        let cwd = pane.session.cwd.lock().unwrap().clone();
        let branch = pane.git.as_ref().map(|g| g.branch.clone());
        stamp_submit(&pane.session, cwd, branch, cmd);
        pane.session.write(format!("{cmd}\n").as_bytes());
    }
}

/// Stamp a submission from the app onto the prompt block — unless one is
/// already waiting for the shell to start it: a line typed ahead of that
/// must not relabel it, and is read back from the screen when it starts.
fn stamp_submit(session: &Session, cwd: Option<String>, branch: Option<String>, cmd: &str) {
    let mut blocks = session.blocks.lock().unwrap();
    if !blocks.submit_pending() {
        blocks.stamp_submit(cwd, branch, cmd);
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
        ) && (self.win.tab_drag.is_some_and(|d| d.live)
            || self.win.pane_drag.as_ref().is_some_and(|d| d.live))
        {
            if let Some(d) = self.win.tab_drag.take() {
                let tab = self.win.tabs.remove(d.index);
                let to = d.origin.min(self.win.tabs.len());
                self.win.tabs.insert(to, tab);
                self.win.active = to;
                self.save_session();
            }
            // a pane drag that opened another tab goes back to the pane's own
            if let Some(d) = self.win.pane_drag.take()
                && d.from_tab != self.win.active
                && d.from_tab < self.win.tabs.len()
            {
                self.switch_tab(d.from_tab);
            }
            self.update_cursor_icon(window);
            window.request_redraw();
            return;
        }
        if self.win.confirm.is_some() {
            self.handle_confirm_key(event, event_loop);
            window.request_redraw();
            return;
        }
        if self.win.prompt_editor.is_some() {
            // Esc cancels, Enter saves; the dialog owns the keyboard
            match event.logical_key {
                winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                    self.win.prompt_editor = None;
                }
                winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => {
                    self.save_prompt_editor();
                }
                _ => {}
            }
            window.request_redraw();
            return;
        }
        if self.win.whats_new.open {
            self.handle_whats_new_key(event);
            window.request_redraw();
            return;
        }
        if self.win.notifications.open && self.handle_notifications_key(event) {
            window.request_redraw();
            return;
        }
        if self.win.hints.is_some() {
            self.handle_hint_key(event);
            window.request_redraw();
            return;
        }
        if self.win.rename.is_some() {
            self.handle_rename_key(event);
            window.request_redraw();
            return;
        }
        if self.win.chip_menu.is_some() {
            self.handle_chip_menu_key(event);
            window.request_redraw();
            return;
        }
        if self.win.context_menu.open {
            if !self.handle_menu_key(event, event_loop) {
                self.win.context_menu.close();
            }
            window.request_redraw();
            return;
        }
        if self.win.palette.open {
            if input::app_action(event, self.win.modifiers, &self.keymap)
                == Some(AppAction::TogglePalette)
            {
                self.win.palette.open = false;
            } else {
                self.handle_palette_key(event, event_loop);
            }
            window.request_redraw();
            return;
        }
        // config.toml beside Settings takes the keys while it has focus
        if self.win.settings.open && !self.viewer_focused() {
            if self.handle_settings_key(event, event_loop, window) {
                window.request_redraw();
            }
            return;
        }
        if self.review_maximized() {
            // the panel covers the command line: it takes the keys it uses,
            // app shortcuts still work, typing is swallowed
            if !self.handle_review_key(event)
                && let Some(action) = input::app_action(event, self.win.modifiers, &self.keymap)
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
        if self.win.history.open && self.handle_history_key(event) {
            window.request_redraw();
            return;
        }
        if self.win.search.open && self.win.filter_editing {
            self.handle_filter_key(event);
            window.request_redraw();
            return;
        }
        if self.win.search.open {
            let action = input::app_action(event, self.win.modifiers, &self.keymap);
            if action == Some(AppAction::Search) {
                self.win.search.close();
            } else if action == Some(AppAction::Copy) {
                self.handle_action(AppAction::Copy, event_loop);
            } else {
                self.handle_search_key(event, self.win.modifiers);
            }
            window.request_redraw();
            return;
        }
        if self.viewer_focused() {
            // the file's find bar takes typing; chords stay app shortcuts
            let mods = self.win.modifiers;
            if self.win.viewer.find.is_some()
                && !mods.super_key()
                && !mods.control_key()
                && input::app_action(event, mods, &self.keymap).is_none()
                && self.win.viewer.handle_find_key(event, mods.shift_key())
            {
                window.request_redraw();
                return;
            }
            if !self.handle_viewer_key(event) {
                // app shortcuts still work; typing is swallowed (the command
                // line is under the viewer)
                if let Some(action) = input::app_action(event, self.win.modifiers, &self.keymap)
                    && !action.needs_editor()
                    && (!event.repeat || action.repeatable())
                {
                    self.handle_action(action, event_loop);
                }
            }
            window.request_redraw();
            return;
        }
        if self.win.explorer.focused && self.win.explorer.open {
            if self.handle_explorer_key(event) {
                window.request_redraw();
                return;
            }
            self.win.explorer.focused = false;
        }
        // Escape clears the keyboard block selection before anything
        // else; in raw mode it still falls through so vim gets it.
        if matches!(
            event.logical_key,
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        ) && let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
            && pane.selected_block.is_some()
        {
            pane.selected_block = None;
            window.request_redraw();
            if self.win.editor_active {
                return;
            }
        }
        // ⌘↑ / ⌘↓ (⇧ selects) move to the start / end of a multi-line
        // command while there are lines that way; on its first / last line
        // they stay the block-selection shortcuts
        if self.win.editor_active
            && self.win.modifiers.super_key()
            && !self.win.modifiers.control_key()
            && !self.win.modifiers.alt_key()
            && let winit::keyboard::Key::Named(
                named @ (winit::keyboard::NamedKey::ArrowUp | winit::keyboard::NamedKey::ArrowDown),
            ) = event.logical_key
            && let Some(pane) = self
                .win
                .tabs
                .get_mut(self.win.active)
                .and_then(|t| t.active_mut())
            // while walking blocks (a block is selected) the keys keep
            // stepping through them
            && pane.selected_block.is_none()
        {
            let up = named == winit::keyboard::NamedKey::ArrowUp;
            let ed = &mut pane.editor;
            if (up && ed.below_first_line()) || (!up && ed.above_last_line()) {
                let motion: fn(&mut crate::state::EditorState) = if up {
                    crate::state::EditorState::mv_doc_start
                } else {
                    crate::state::EditorState::mv_doc_end
                };
                if self.win.modifiers.shift_key() {
                    ed.extend(motion);
                } else {
                    ed.anchor = None;
                    motion(ed);
                }
                self.blink_epoch = Instant::now();
                window.request_redraw();
                return;
            }
        }
        if let Some(action) = input::app_action(event, self.win.modifiers, &self.keymap) {
            // plain terminal keys (PageUp, Home, ⌃M…) bound to app
            // actions still belong to vim/less when the editor isn't
            // active — fall through to raw byte passthrough
            if !action.needs_editor() || self.win.editor_active {
                if event.repeat && !action.repeatable() {
                    return;
                }
                log::debug!("key action: {action:?} (mods {:?})", self.win.modifiers);
                self.handle_action(action, event_loop);
                return;
            }
        }
        log::debug!(
            "key fallthrough: {:?} mods={:?} editor={}",
            event.logical_key,
            self.win.modifiers,
            self.win.editor_active
        );
        if self.win.editor_active {
            if self.vim_intercept(event) {
                window.request_redraw();
                return;
            }
            if let Some(key) = input::editor_key(event, self.win.modifiers) {
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
        let (mods, option_as_meta) = (self.win.modifiers, self.option_meta());
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
        if self.config.tab_layout == rift_core::TabLayout::Vertical {
            self.win.mouse.1
        } else {
            self.win.mouse.0
        }
    }

    /// Pointer moved during a pane-header press: go live past the slop and
    /// track the drop target under the pointer.
    fn track_pane_drag(&mut self) {
        let (mx, my) = self.win.mouse;
        let Some(d) = self.win.pane_drag.as_ref() else {
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
        if let Some(d) = self.win.pane_drag.as_mut() {
            d.live = live;
            d.target = target;
            d.hover_tab = hover_tab;
        }
        if let Some((j, since)) = hover_tab
            && since.elapsed() >= TAB_HOVER_OPEN
            && j != self.win.active
        {
            self.switch_tab(j);
            if let Some(d) = self.win.pane_drag.as_mut() {
                d.hover_tab = None;
            }
        }
    }

    /// Where `pane` (of tab `from`) would land if released at the point.
    fn pane_drop_at(&self, pane: PaneId, from: usize, mx: f32, my: f32) -> Option<PaneDrop> {
        let hit = self
            .win
            .hits
            .iter()
            .find(|h| h.contains(mx, my))
            .map(|h| h.target.clone());
        // the Settings tab takes no panes
        if let Some(ClickTarget::SettingsClose) = hit {
            return None;
        }
        if let Some(ClickTarget::TabSelect(j) | ClickTarget::TabClose(j)) = hit {
            return (j < self.win.tabs.len() && (j != from || j != self.win.active))
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
            let alone = self.win.tabs.get(from).is_none_or(|t| t.panes.len() <= 1);
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
        if from != self.win.active
            && (r.id == VIEWER_PANE
                || (pane == VIEWER_PANE
                    && self
                        .win
                        .tabs
                        .get(self.win.active)
                        .is_some_and(|t| t.root.contains(VIEWER_PANE))))
        {
            return None;
        }
        // the middle third swaps (same tab only); elsewhere the nearest half
        Some(
            if from == self.win.active && crate::pane::Edge::in_centre(r.rect, mx, my) {
                PaneDrop::Swap(r.id)
            } else {
                PaneDrop::Pane(r.id, crate::pane::Edge::nearest(r.rect, mx, my))
            },
        )
    }

    /// Move `pane` out of tab `from` to `target` (pane header drag and drop).
    /// A shell keeps its session; a tab left without shells goes away.
    fn drop_pane(&mut self, pane: PaneId, from: usize, target: PaneDrop) {
        if !self
            .win
            .tabs
            .get(from)
            .is_some_and(|t| t.root.contains(pane))
        {
            return;
        }
        let viewer = pane == VIEWER_PANE;
        // a pane of another tab (that tab was opened by hovering it): land
        // beside it there
        let mut beside = None;
        let to = match target {
            PaneDrop::Swap(onto) => {
                let tab = &mut self.win.tabs[from];
                if onto == pane || !tab.root.contains(onto) {
                    return;
                }
                tab.zoomed = None;
                tab.root.swap(pane, onto);
                self.save_session();
                return;
            }
            PaneDrop::Pane(onto, edge) if !self.win.tabs[from].root.contains(onto) => {
                let Some(to) = self.win.tabs.iter().position(|t| t.root.contains(onto)) else {
                    return;
                };
                beside = Some((onto, edge));
                Some(to)
            }
            PaneDrop::Pane(onto, edge) => {
                let tab = &mut self.win.tabs[from];
                if onto == pane {
                    return;
                }
                tab.zoomed = None;
                tab.root.remove(pane);
                tab.root.insert(onto, pane, edge);
                if viewer {
                    self.win.viewer_focus = true;
                } else {
                    tab.active_pane = pane;
                    self.win.viewer_focus = false;
                }
                self.save_session();
                return;
            }
            PaneDrop::Tab(to) if to == from || to >= self.win.tabs.len() => return,
            PaneDrop::Tab(to) => Some(to),
            PaneDrop::NewTab => None,
        };
        // out of the source tab
        let src = &mut self.win.tabs[from];
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
                let dst = &mut self.win.tabs[to];
                dst.zoomed = None;
                let anchor = dst.active_pane;
                let placed = match beside {
                    Some((onto, edge)) => dst.root.insert(onto, pane, edge),
                    None => dst.root.split(anchor, pane, Axis::Horizontal),
                };
                if !placed {
                    dst.root.insert_root(pane, crate::pane::Edge::Right, 0.5);
                }
                if let Some(state) = state {
                    dst.panes.push(state);
                    dst.active_pane = pane;
                }
                to
            }
            None => {
                let Some(state) = state else { return };
                self.win.tabs.insert(from + 1, TabState::new(state));
                from + 1
            }
        };
        let dest = if empties {
            self.win.tabs.remove(from);
            if dest > from { dest - 1 } else { dest }
        } else {
            dest
        };
        self.win.rename = None;
        self.win.viewer_focus = viewer;
        self.switch_tab(dest);
    }

    fn active_pane_id(&self) -> Option<PaneId> {
        Some(self.win.tabs.get(self.win.active)?.active()?.session.id)
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
        let (mods, option_as_meta) = (self.win.modifiers, self.option_meta());
        for entry in pressed {
            self.keys_down.remove(&entry);
            let target = entry.0;
            if Some(target) == active && self.win.editor_active {
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
        self.win.mouse = pos;
        self.update_cursor_icon(window);
        // tabs, welcome rows, pane controls and settings rows draw their
        // hover from the mouse at render time: repaint as soon as the pointer
        // enters or leaves one, not on the next unrelated frame
        let sig = self
            .win
            .hits
            .iter()
            .enumerate()
            .filter(|(_, h)| h.contains(pos.0, pos.1))
            .fold(0u64, |s, (i, _)| {
                s.wrapping_mul(31).wrapping_add(i as u64 + 1)
            });
        if sig != self.win.hover_sig {
            self.win.hover_sig = sig;
            self.win.hover_repaints += 1;
            window.request_redraw();
        }
        self.win.pointer_in = true;
        if let Some(ed) = &mut self.win.prompt_editor {
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
        let lit = self.win.review.hover_index();
        self.win.review.pointer = Some(pos);
        if self.win.review.hover_index() != lit {
            window.request_redraw();
        }
        let tip = self
            .win
            .tips
            .iter()
            .find(|t| t.contains(pos.0, pos.1))
            .map(|t| t.rect);
        if tip != self.win.tip_hover.map(|(r, _)| r)
            || (tip.is_none() && self.win.tip_shown.is_some())
        {
            window.request_redraw();
        }
        if self.win.mouse_report.is_some() {
            self.report_motion();
            return;
        }
        let idle = !self.win.palette.open
            && !self.win.settings.open
            && !self.win.context_menu.open
            && self.win.chip_menu.is_none()
            && self.win.prompt_editor.is_none()
            && self.win.confirm.is_none()
            && !self.win.whats_new.open
            && self.win.drag.is_none()
            && self.win.pending_sel.is_none()
            && self.win.editor_drag.is_none()
            && self.win.tab_drag.is_none()
            && self.win.pane_drag.is_none()
            && self.win.slider_drag.is_none();
        if idle && self.config.focus_follows_mouse && !self.win.notifications.open {
            self.focus_pane_under(pos, window);
        }
        if idle && self.report_motion() {
            return;
        }
        if let Some(drag) = &self.win.drag {
            let pos = match drag.axis {
                Axis::Horizontal => self.win.mouse.0,
                Axis::Vertical => self.win.mouse.1,
            };
            let (path, index, area) = (drag.path.clone(), drag.index, drag.area);
            if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
                tab.root.drag_divider(&path, index, area, pos);
            }
            window.request_redraw();
        }
        // the chip picker's highlight follows the pointer at once (its rows
        // are not frame hits, so nothing else repaints them)
        if let Some(menu) = &mut self.win.chip_menu
            && let Some(&(_, i)) = self.win.chip_menu_rows.iter().find(|(r, _)| {
                pos.0 >= r[0] && pos.0 < r[0] + r[2] && pos.1 >= r[1] && pos.1 < r[1] + r[3]
            })
            && i != menu.selected
        {
            menu.selected = i;
            window.request_redraw();
        }
        if self.win.context_menu.open {
            let (mx2, my2) = self.win.mouse;
            let hover =
                self.win.context_menu_rects.iter().position(|r| {
                    mx2 >= r[0] && mx2 < r[0] + r[2] && my2 >= r[1] && my2 < r[1] + r[3]
                });
            if hover != self.win.context_menu.selected {
                self.win.context_menu.selected = hover;
                window.request_redraw();
            }
        }
        if self.win.pending_sel.is_some() {
            self.drag_selection();
        }
        if let Some(pane) = self.win.editor_drag {
            self.drag_editor(pane);
            window.request_redraw();
        }
        if self.win.pane_drag.is_some() {
            self.track_pane_drag();
            self.update_cursor_icon(window);
            window.request_redraw();
        }
        if let Some(TabDrag {
            index: i,
            start,
            press,
            ..
        }) = self.win.tab_drag
        {
            // drag a tab past a neighbour's midpoint to swap (Warp reorder);
            // the vertical rail reorders along y
            let vertical = self.config.tab_layout == rift_core::TabLayout::Vertical;
            let pos = self.tab_drag_coord();
            if let Some(d) = self.win.tab_drag.as_mut()
                && !d.live
                && (pos - press).abs() > PANE_DRAG_SLOP
            {
                d.live = true;
            }
            if self.win.tab_drag.is_some_and(|d| d.live) {
                window.request_redraw();
            }
            let delta = pos - start;
            // shell tabs only: the Settings tab after them never swaps
            let n_tabs = self.win.tabs.len();
            let neighbour = self.win.hits.iter().find_map(|h| match h.target {
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
                    self.win.tabs.swap(i, j);
                    self.win.active = j;
                    if let Some(d) = self.win.tab_drag.as_mut() {
                        d.index = j;
                        d.start = pos;
                    }
                    self.save_session();
                    window.request_redraw();
                }
            }
        }
        if let Some((id, rect)) = self.win.slider_drag {
            let t = ((self.win.mouse.0 - rect[0] - 8.0) / (rect[2] - 16.0)).clamp(0.0, 1.0);
            self.set_slider(id, t);
            window.request_redraw();
        }
    }

    /// `focus_follows_mouse`: the split pane (or the file viewer's pane)
    /// under `pos` takes focus, as a click on it would, without selecting.
    fn focus_pane_under(&mut self, pos: (f32, f32), window: &Window) {
        let [vx, vy, vw, vh] = self.win.viewer_rect;
        if vw > 0.0 && pos.0 >= vx && pos.0 < vx + vw && pos.1 >= vy && pos.1 < vy + vh {
            if !self.win.viewer_focus {
                self.win.viewer_focus = true;
                window.request_redraw();
            }
            return;
        }
        let Some(id) = self.win.hits.iter().find_map(|h| match h.target {
            ClickTarget::PaneFocus(id) if h.contains(pos.0, pos.1) => Some(id),
            _ => None,
        }) else {
            return;
        };
        let viewer_had_focus = self.viewer_focused();
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        if tab.active_pane == id && !viewer_had_focus {
            return;
        }
        if tab.active_pane != id
            && let Some(prev) = tab.active_mut()
        {
            prev.selected_block = None;
        }
        tab.active_pane = id;
        self.win.viewer_focus = false;
        window.request_redraw();
    }

    pub(crate) fn on_right_press(&mut self, window: &Window) {
        self.hide_tooltip(window);
        if self.win.chip_menu.take().is_some() {
            window.request_redraw();
            return;
        }
        if self.win.prompt_editor.is_some() {
            return;
        }
        if self.win.palette.open || self.win.confirm.is_some() || self.win.whats_new.open {
            return;
        }
        if self.win.notifications.open {
            self.close_notifications();
            window.request_redraw();
            return;
        }
        if !self.win.context_menu.open
            && !self.win.settings.open
            && self.report_button(crate::mouse::RIGHT, true)
        {
            window.request_redraw();
            return;
        }
        if self.over_maximized_review() {
            return;
        }
        let (mx, my) = self.win.mouse;
        // tab bar: tab menu
        let tab_hit = self
            .win
            .hits
            .iter()
            .find(|h| h.contains(mx, my))
            .and_then(|h| match h.target {
                ClickTarget::TabSelect(i) | ClickTarget::TabClose(i) => Some(i),
                _ => None,
            });
        if let Some(i) = tab_hit {
            self.open_tab_menu(i, self.win.mouse);
            window.request_redraw();
            return;
        }
        // the Settings tab's page has no context menus
        if self.win.settings.open {
            return;
        }
        // resolve the block under the cursor via the frame row cache
        let target = self.win.tabs.get(self.win.active).and_then(|tab| {
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
            self.open_context_menu(pane_id, block, self.win.mouse);
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
            if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
                tab.active_pane = id;
            }
            self.open_terminal_menu(id, self.win.mouse);
            window.request_redraw();
        }
    }

    pub(crate) fn on_left(
        &mut self,
        state: ElementState,
        event_loop: &ActiveEventLoop,
        window: &Window,
    ) {
        if state == ElementState::Pressed && self.press_tip_card(window) {
            return;
        }
        if state == ElementState::Pressed {
            self.hide_tooltip(window);
        }
        if state == ElementState::Released && self.win.prompt_editor.is_some() {
            self.prompt_editor_release();
            window.request_redraw();
            return;
        }
        if state == ElementState::Released {
            if self.report_button(crate::mouse::LEFT, false) {
                window.request_redraw();
                return;
            }
            if self.win.drag.take().is_some() {
                self.save_session(); // divider position
            }
            self.win.tab_drag = None;
            if let Some(d) = self.win.pane_drag.take() {
                if d.live
                    && let Some(target) = d.target
                {
                    self.drop_pane(d.pane, d.from_tab, target);
                } else if d.live
                    && d.from_tab != self.win.active
                    && d.from_tab < self.win.tabs.len()
                {
                    // released nowhere after opening another tab: back home
                    self.switch_tab(d.from_tab);
                }
                self.update_cursor_icon(window);
                window.request_redraw();
                return;
            }
            if self.win.editor_drag.take().is_some() {
                return;
            }
            let pending = self.win.pending_sel.take();
            let selecting = pending.as_ref().is_some_and(|p| p.started);
            let link = self.win.link_press.take().filter(|_| !selecting);
            if let Some((pane, url)) = link {
                self.click_link(pane, &url);
                window.request_redraw();
            } else if let Some(p) = pending.filter(|p| !p.started) {
                // Warp: a click (no drag) on a command's output selects its block
                self.select_block_at(p.pane, p.abs);
                window.request_redraw();
            }
            if selecting
                && self.config.copy_on_select
                && let Some(text) = self.selection_text()
            {
                self.copy_to_clipboard(text);
            }
            if self.win.slider_drag.take().is_some() {
                self.save_config();
            }
            return;
        }
        let (mx, my) = self.win.mouse;
        let inside =
            |r: &[f32; 4]| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3];
        if self.win.confirm.is_some() {
            if inside(&self.win.confirm_rects[1]) {
                if let Some(c) = self.win.confirm.take() {
                    self.perform_confirmed(c.action, event_loop);
                }
            } else if inside(&self.win.confirm_rects[0]) {
                self.cancel_confirm();
            }
            window.request_redraw();
            return;
        }
        if self.win.prompt_editor.is_some() {
            self.prompt_editor_press();
            window.request_redraw();
            return;
        }
        if self.win.whats_new.open {
            let h = self.win.whats_new_hits;
            if inside(&h.close) || inside(&h.done) || !inside(&h.card) {
                self.win.whats_new.open = false;
            }
            window.request_redraw();
            return;
        }
        if self.win.notifications.open {
            let h = self.win.notifications_hits.clone();
            if inside(&h.close) {
                self.close_notifications();
            } else if inside(&h.mark_read) {
                self.mark_all_read();
            } else if let Some((_, f)) = h.chips.iter().find(|(r, _)| inside(r)) {
                self.win.notifications.set_filter(*f);
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
        // status toasts: ✕ dismisses, the button acts, the chip eats the click
        if let Some(t) = self
            .win
            .toast_hits
            .iter()
            .find(|t| inside(&t.rect))
            .copied()
        {
            let action = self
                .win
                .toasts
                .iter()
                .find(|x| x.id == t.id)
                .and_then(|x| x.action);
            if t.close.is_some_and(|r| inside(&r)) {
                self.win.toasts.retain(|x| x.id != t.id);
            } else if let (Some(r), Some(action)) = (t.action, action)
                && inside(&r)
            {
                self.win.toasts.retain(|x| x.id != t.id);
                match action {
                    // the file beside the Settings page, as its own
                    // "Open config.toml" does
                    ToastAction::OpenConfig => {
                        if let Some(path) = rift_core::config::config_path() {
                            self.win.settings.open_page(self.win.settings.page);
                            self.open_config_viewer(&path);
                        }
                    }
                }
            }
            window.request_redraw();
            return;
        }
        // notification toasts float over everything below the modals
        if let Some(t) = self
            .win
            .note_toast_hits
            .iter()
            .find(|t| inside(&t.rect))
            .copied()
        {
            let note = self
                .win
                .app_notes
                .iter()
                .find(|n| n.id == t.id)
                .map(|n| n.action);
            self.win.note_toasts.retain(|(id, _)| *id != t.id);
            self.win.app_notes.retain(|n| n.id != t.id);
            if !inside(&t.close) {
                match note {
                    Some(AppNoteAction::Update) => {
                        self.handle_action(AppAction::UpdateRift, event_loop);
                    }
                    Some(AppNoteAction::WhatsNew) => {
                        self.handle_action(AppAction::WhatsNew, event_loop);
                    }
                    Some(AppNoteAction::None) => {}
                    None => self.show_attention(t.id),
                }
            }
            window.request_redraw();
            return;
        }
        if let Some((i, _)) = &self.win.rename {
            // clicking anywhere commits the rename
            let i = *i;
            self.commit_rename(i);
        }
        if self.win.chip_menu.is_some() {
            match self.win.chip_menu_rows.iter().find(|(r, _)| inside(r)) {
                Some(&(_, i)) => self.apply_chip_menu(i),
                None if inside(&self.win.chip_menu_card) => {}
                None => self.win.chip_menu = None,
            }
            window.request_redraw();
            return;
        }
        if self.win.context_menu.open {
            let row = self
                .win
                .context_menu_rects
                .iter()
                .position(|r| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]);
            match row {
                Some(i) => {
                    let item = &self.win.context_menu.items[i];
                    if item.enabled {
                        let action = item.action;
                        self.run_menu_action(action, event_loop);
                    }
                }
                None => self.win.context_menu.close(),
            }
            window.request_redraw();
            return;
        }
        if self.win.palette.open {
            let chip = self.win.hits.iter().find_map(|h| match h.target {
                ClickTarget::PaletteMode(i) if h.contains(mx, my) => Some(i as usize),
                _ => None,
            });
            if let Some(mode) = chip.and_then(|i| crate::views::palette::MODES.get(i)) {
                self.open_palette(mode.0);
                window.request_redraw();
                return;
            }
            let row = self
                .win
                .palette_row_hits
                .iter()
                .find(|(r, _)| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3])
                .map(|(_, i)| *i);
            match row {
                Some(i) => {
                    if let Some(m) = self.win.palette_results.get(i) {
                        let action = m.item.action.clone();
                        self.run_palette_action(action, event_loop);
                    }
                }
                None => self.win.palette.open = false,
            }
            window.request_redraw();
            return;
        }
        // find-bar toggles
        if self.win.search.open
            && !self.win.filter_editing
            && !self.over_maximized_review()
            && let Some(i) = self.win.search_chips.iter().position(inside)
        {
            if i == 0 {
                self.win.search.case_sensitive = !self.win.search.case_sensitive;
            } else {
                self.win.search.regex = !self.win.search.regex;
            }
            self.refresh_search(true);
            window.request_redraw();
            return;
        }
        // config.toml beside Settings: its clicks, and focus back to the
        // page on a click beside it
        if self.win.settings.open && self.win.config_viewer {
            if inside(&self.win.viewer_rect) && self.on_panel_click(inside) {
                self.win.last_click = None;
                window.request_redraw();
                return;
            }
            self.win.viewer_focus = false;
        }
        if !self.win.settings.open && self.on_panel_click(inside) {
            self.win.last_click = None;
            window.request_redraw();
            return;
        }
        // a click anywhere else leaves the tree's keyboard focus, and one in
        // a shell pane takes focus from the viewer pane
        self.win.explorer.focused = false;
        if !self.win.settings.open && self.win.mouse.1 > TAB_BAR_H {
            self.win.viewer_focus = false;
        }
        // a program tracking the mouse (vim, tmux, htop) gets the click
        if !self.win.settings.open && self.report_button(crate::mouse::LEFT, true) {
            self.win.last_click = None;
            window.request_redraw();
            return;
        }
        let target = self
            .win
            .hits
            .iter()
            .find(|h| h.contains(mx, my))
            .map(|h| h.target.clone());
        // any click but another row ends a shortcut recording
        if !matches!(target, Some(ClickTarget::SettingsKeybind(_))) {
            self.win.settings.recording = None;
        }
        // a click outside the relay field ends its edit (saved when valid)
        if self.win.settings.remote.relay_edit.is_some()
            && !matches!(target, Some(ClickTarget::RemoteRelay))
        {
            self.commit_relay_edit();
        }
        // a click outside the history pane (and its command line) closes it
        if self.win.history.open
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
            self.win.history.close();
        }
        // an open select list: a click outside its options only closes it
        if self.win.settings.dropdown.is_some()
            && !matches!(target, Some(ClickTarget::SettingsOption(_)))
        {
            self.win.settings.dropdown = None;
            window.request_redraw();
            return;
        }
        // the tab strip leaves the Settings tab for a shell tab (its own
        // entry, after the shells, shows it again)
        if let Some(ClickTarget::TabSelect(i)) = target {
            if i >= self.win.tabs.len() {
                if self.win.settings.tab {
                    self.win.settings.open = true;
                }
                window.request_redraw();
                return;
            }
            self.win.settings.open = false;
        }
        match &target {
            Some(ClickTarget::Block(pane, id) | ClickTarget::BlockDuration(pane, id)) => {
                // Warp: clicking a block selects it (again deselects)
                let (pane, id) = (*pane, *id);
                if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
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
                if i < self.win.tabs.len() {
                    // double-click renames (Warp); a press arms drag-reorder
                    if self.click_count() == 2 {
                        self.start_rename(i);
                    }
                    if i != self.win.active {
                        self.switch_tab(i);
                    }
                    let pos = self.tab_drag_coord();
                    let vertical = self.config.tab_layout == rift_core::TabLayout::Vertical;
                    let grab = self
                        .win
                        .hits
                        .iter()
                        .find(|h| h.target == ClickTarget::TabSelect(i))
                        .map(|h| pos - if vertical { h.rect[1] } else { h.rect[0] })
                        .unwrap_or(0.0);
                    self.win.tab_drag = Some(TabDrag {
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
            Some(ClickTarget::TabNew) => self.spawn_new_tab(),
            Some(ClickTarget::ToggleExplorer) => {
                self.handle_action(AppAction::ToggleExplorer, event_loop)
            }
            Some(ClickTarget::ToggleReview) => {
                self.handle_action(AppAction::ToggleReview, event_loop)
            }
            Some(ClickTarget::ToggleNotifications) => {
                self.handle_action(AppAction::AttentionCenter, event_loop)
            }
            Some(ClickTarget::UpdateChip) => self.handle_action(AppAction::UpdateRift, event_loop),
            Some(ClickTarget::RemoteChip) => {
                self.win
                    .settings
                    .open_page(crate::views::settings::Page::Remote);
                self.poll_remote(true);
            }
            Some(ClickTarget::Chip(pane, rift_core::PromptChip::GitDiff)) => {
                // opens the review of that pane's repository (focused first);
                // a second click closes it, like the tab bar's button
                if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
                    tab.active_pane = *pane;
                }
                self.handle_action(AppAction::ToggleReview, event_loop)
            }
            Some(ClickTarget::Chip(pane, rift_core::PromptChip::GithubPr)) => {
                let url = self
                    .win
                    .tabs
                    .get(self.win.active)
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
                self.win.editor_drag = Some(pane);
            }
            Some(ClickTarget::PaneFocus(id)) => {
                let id = *id;
                if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
                    if tab.active_pane != id
                        && let Some(prev) = tab.active_mut()
                    {
                        prev.selected_block = None;
                    }
                    tab.active_pane = id;
                }
                let clicks = self.click_count();
                if self.try_open_url(id) {
                    self.win.last_click = None;
                } else {
                    self.press_selection(id, clicks);
                    self.press_link(id, clicks);
                }
            }
            Some(ClickTarget::PaneHeader(id)) => {
                // focus the pane; dragging the bar past the slop moves it
                let id = *id;
                if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
                    if tab.active_pane != id
                        && let Some(prev) = tab.active_mut()
                    {
                        prev.selected_block = None;
                    }
                    tab.active_pane = id;
                }
                self.win.pane_drag = Some(PaneDrag {
                    pane: id,
                    from_tab: self.win.active,
                    press: self.win.mouse,
                    live: false,
                    target: None,
                    hover_tab: None,
                });
            }
            Some(ClickTarget::PaneMenu(id)) => {
                let id = *id;
                if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
                    tab.active_pane = id;
                }
                // drops down from the ⋮ button
                let at = self
                    .win
                    .hits
                    .iter()
                    .find(|h| h.target == ClickTarget::PaneMenu(id))
                    .map_or(self.win.mouse, |h| (h.rect[0], h.rect[1] + h.rect[3] + 4.0));
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
            Some(ClickTarget::HistoryScope(scope)) => self.win.history.set_scope(*scope),
            Some(ClickTarget::HistoryClose) => self.win.history.close(),
            Some(ClickTarget::HistoryRow(i)) => {
                // a click picks the row onto the command line; a second runs it
                if self.win.history.selected == *i && self.click_count() >= 2 {
                    self.take_history(HistoryUse::Run);
                } else {
                    self.win.history.selected = *i;
                    self.take_history(HistoryUse::Edit);
                }
            }
            Some(ClickTarget::CompletionSource(i)) => {
                self.win.completion.pick_source(*i as usize);
            }
            Some(ClickTarget::CompletionRow(i)) => {
                self.win.completion.selected = *i;
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
                if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
                    tab.active_pane = *id;
                }
                self.win.viewer_focus = false;
                self.split_active(Axis::Horizontal);
            }
            Some(ClickTarget::HandleDrag(path, index)) => {
                // a double-click gives the split's panes equal shares (Warp)
                if self.click_count() == 2 {
                    if let Some(tab) = self.win.tabs.get_mut(self.win.active)
                        && tab.root.equalize(path)
                    {
                        self.save_session_file();
                    }
                } else {
                    // drag: find the split node's area from the current layout
                    let (lw, lh) = self.logical_size();
                    let (_, handles) = self.pane_rects(lw, lh);
                    if let Some(hr) = handles
                        .iter()
                        .find(|h| &h.path == path && h.index == *index)
                    {
                        self.win.drag = Some(DragState {
                            path: path.clone(),
                            index: *index,
                            axis: hr.axis,
                            area: hr.area,
                        });
                    }
                }
            }
            Some(ClickTarget::BlockCopy(pane, id)) => {
                let text = self
                    .find_tab_of_pane(*pane)
                    .and_then(|(_, tab)| tab.pane(*pane))
                    .and_then(|p| terminal::block_output_text(&p.session, *id));
                match text {
                    Some(text) => {
                        if self.copy_to_clipboard(text) {
                            // the button turns into a check for a moment
                            self.win.copied = Some((*pane, *id, Instant::now()));
                            self.toast_done("Copied output".into());
                        }
                    }
                    None => self.toast("Nothing to copy: this command printed no output".into()),
                }
            }
            Some(ClickTarget::BlockMenu(pane, id)) => {
                // drops down from the ⋮ button
                let at = self
                    .win
                    .hits
                    .iter()
                    .find(|h| h.target == ClickTarget::BlockMenu(*pane, *id))
                    .map_or(self.win.mouse, |h| (h.rect[0], h.rect[1] + h.rect[3] + 2.0));
                self.open_context_menu(*pane, *id, at);
            }
            Some(ClickTarget::TrafficLight(i)) => match i {
                0 => self.request_close(CloseScope::Window, event_loop),
                _ if self.headless => {}
                1 => window.set_minimized(true),
                _ => window.set_maximized(!window.is_maximized()),
            },
            // the empty strip: a double-click zooms the window (the titlebar's
            // own gesture), a press drags it
            // (the drop-down hotkey window stays where it hangs)
            Some(ClickTarget::WindowDrag) if !self.headless && self.win.dropdown.is_none() => {
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
            Some(ClickTarget::SettingsNoop | ClickTarget::RemoteHistoryList) => {}
            Some(ClickTarget::SettingsNav(i)) => {
                use crate::views::settings::Page;
                let page = Page::ALL.get(*i as usize).copied().unwrap_or(Page::General);
                self.win.settings.open_page(page);
            }
            Some(ClickTarget::SettingsClose) => self.win.settings.close_tab(),
            Some(ClickTarget::SettingsOption(i)) => {
                if let Some(dd) = self.win.settings.dropdown.take()
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
            Some(ClickTarget::SettingsUpdate) => self.update_rift(),
            Some(ClickTarget::SettingsCheckUpdates) => self.check_for_updates_now(),
            Some(ClickTarget::SettingsWhatsNew) => {
                self.handle_action(AppAction::WhatsNew, event_loop)
            }
            Some(ClickTarget::SettingsPlaySound(id)) => self.play_sound_setting(id),
            Some(ClickTarget::SettingsTestAlert(t)) => self.test_alert(*t),
            Some(ClickTarget::SettingsNotifyPermission) => {
                if crate::notify::permission() == crate::notify::Permission::NotAsked {
                    let proxy = std::sync::Mutex::new(self.proxy.clone());
                    crate::notify::request_permission(move || {
                        if let Ok(p) = proxy.lock() {
                            let _ = p.send_event(UserEvent::NotifyPermission);
                        }
                    });
                } else {
                    crate::notify::open_system_settings();
                }
            }
            Some(ClickTarget::SettingsRingBell) => {
                log::info!("bell: rung from Settings ({:?})", self.config.bell);
                self.ring_bell();
            }
            Some(ClickTarget::RemotePair) => {
                self.win.settings.remote.pairing = None;
                self.poll_remote(true);
                self.start_pairing();
            }
            Some(ClickTarget::RemoteViewOnly) => {
                let v = &mut self.win.settings.remote;
                v.view_only = !v.view_only;
                // a shown link keeps the permission it was made with
                v.pairing = None;
                v.pairing_wait = false;
            }
            Some(ClickTarget::RemoteInstallTunnel) => self.install_cloudflared(),
            Some(ClickTarget::RemoteCopyLink) => {
                if let Some(url) = self
                    .win
                    .settings
                    .remote
                    .pairing
                    .as_ref()
                    .map(|p| p.url.clone())
                    && self.copy_to_clipboard(url)
                {
                    self.toast_done("Copied pairing link".into());
                }
            }
            Some(ClickTarget::RemoteRelay) => {
                if self.win.settings.remote.relay_edit.is_none() {
                    self.win.settings.remote.relay_edit = Some(self.config.remote_relay.clone());
                }
                self.win.settings.focus = Some(ClickTarget::RemoteRelay);
            }
            Some(ClickTarget::RemoteRelayRetry) => {
                // the gateway's relay connection retries at once; its new
                // state shows with the next poll
                if let Some(dir) = crate::remote::registry_dir()
                    && let Err(e) = rift_remote::registry::request_relay_retry(&dir)
                {
                    self.toast_error(format!("Could not ask the gateway to retry: {e}"));
                } else {
                    // under the 1 s poll, so the next poll ends it
                    self.remote.retry_shown_until =
                        Some(Instant::now() + Duration::from_millis(900));
                }
                self.poll_remote(true);
            }
            Some(ClickTarget::RemoteRelaySettings) => {
                let v = &mut self.win.settings.remote;
                v.relay_open = !v.relay_open;
            }
            Some(ClickTarget::RemotePermission(id, view)) => {
                let name = self
                    .win
                    .settings
                    .remote
                    .devices
                    .iter()
                    .find(|d| d.id == *id)
                    .map_or_else(|| id.clone(), |d| d.name.clone());
                let already = self
                    .win
                    .settings
                    .remote
                    .devices
                    .iter()
                    .any(|d| d.id == *id && d.view_only == *view);
                if already {
                    // pressing the selected option changes nothing
                } else if *view {
                    // lowering needs no confirmation
                    self.set_device_permission(id, &name, true);
                } else {
                    self.win.confirm = Some(ConfirmClose {
                        opened: Instant::now(),
                        action: ConfirmAction::AllowOperate {
                            id: id.clone(),
                            name: name.clone(),
                        },
                        commands: vec![name],
                    });
                }
            }
            Some(ClickTarget::RemoteRevoke(id)) => {
                let name = self
                    .win
                    .settings
                    .remote
                    .devices
                    .iter()
                    .find(|d| d.id == *id)
                    .map_or_else(|| id.clone(), |d| d.name.clone());
                self.win.confirm = Some(ConfirmClose {
                    opened: Instant::now(),
                    action: ConfirmAction::RevokeDevice {
                        id: id.clone(),
                        name: name.clone(),
                    },
                    commands: vec![name],
                });
            }
            Some(ClickTarget::SettingsCopyVersion) => {
                if self.copy_to_clipboard(format!("v{}", env!("CARGO_PKG_VERSION"))) {
                    self.toast_done("Copied version".into());
                }
            }
            Some(ClickTarget::SettingsOpenConfig) => {
                if let Some(path) = rift_core::config::config_path() {
                    self.open_config_viewer(&path);
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
                    "shell_completions" => {
                        c.shell_completions = !c.shell_completions;
                        if !c.shell_completions {
                            // the warm shells are not needed any more
                            std::thread::spawn(rift_core::shell_complete::stop_all);
                        }
                    }
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
                    "hotkey_window" => c.hotkey_window = !c.hotkey_window,
                    "hotkey_window_hide_on_blur" => {
                        c.hotkey_window_hide_on_blur = !c.hotkey_window_hide_on_blur
                    }
                    "check_for_updates" => c.check_for_updates = !c.check_for_updates,
                    // on: the windows start a gateway; off: it exits by itself
                    "remote_access" => c.remote_access = !c.remote_access,
                    // the running gateway starts or stops the tunnel itself
                    "remote_tunnel" => c.remote_tunnel = !c.remote_tunnel,
                    // the running gateway reads it on every check
                    "remote_when_closed" => c.remote_when_closed = !c.remote_when_closed,
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
                    "tab_bar_with_one_tab" => c.tab_bar_with_one_tab = !c.tab_bar_with_one_tab,
                    "sticky_command_header" => c.sticky_command_header = !c.sticky_command_header,
                    "dim_inactive_panes" => c.dim_inactive_panes = !c.dim_inactive_panes,
                    "focus_follows_mouse" => c.focus_follows_mouse = !c.focus_follows_mouse,
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
                    "hotkey_window_height" => {
                        c.hotkey_window_height =
                            (i64::from(c.hotkey_window_height()) + d * 5).clamp(20, 100) as u8;
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
                self.win.prompt_editor = Some(crate::views::prompt_editor::PromptEditor::new(
                    &self.config.prompt_chips,
                ));
            }
            Some(ClickTarget::SettingsSegment(id, v)) => {
                use rift_core::{Bell, CursorStyle, InputPosition, PromptMode};
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
                            rift_core::BlocksStart::Top
                        } else {
                            rift_core::BlocksStart::Bottom
                        }
                    }
                    "control_api" => {
                        c.control_api = ["off", "navigate", "full"]
                            .get(*v as usize)
                            .unwrap_or(&"navigate")
                            .to_string();
                    }
                    "link_click" => {
                        c.link_click = if *v == 1 { "open" } else { "prompt" }.into();
                    }
                    "clipboard_access" => {
                        c.clipboard_access = ["off", "write", "read_write"]
                            .get(*v as usize)
                            .unwrap_or(&"write")
                            .to_string();
                    }
                    "tab_layout" => {
                        c.tab_layout = if *v == 1 {
                            rift_core::TabLayout::Vertical
                        } else {
                            rift_core::TabLayout::Horizontal
                        }
                    }
                    "tab_close_button" => {
                        c.tab_close_button = match v {
                            0 => rift_core::TabCloseButton::Hover,
                            1 => rift_core::TabCloseButton::Always,
                            _ => rift_core::TabCloseButton::Never,
                        }
                    }
                    "new_tab_placement" => {
                        c.new_tab_placement = if *v == 1 {
                            rift_core::NewTabPlacement::AfterCurrent
                        } else {
                            rift_core::NewTabPlacement::End
                        }
                    }
                    "elapsed_counts" => {
                        c.elapsed_counts = if *v == 1 {
                            rift_core::ElapsedCounts::Running
                        } else {
                            rift_core::ElapsedCounts::Always
                        }
                    }
                    "prompt" => {
                        c.prompt = if *v == 1 {
                            PromptMode::Shell
                        } else {
                            PromptMode::Rift
                        }
                    }
                    "working_directory" => match v {
                        0 => c.working_directory = "previous".into(),
                        1 => c.working_directory = "home".into(),
                        // a custom path is typed in config.toml; keep one already set
                        _ => {
                            if matches!(c.working_directory.trim(), "" | "previous" | "home") {
                                self.toast(
                                    "Set working_directory = \"/path\" in config.toml".into(),
                                );
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
                    .win
                    .hits
                    .iter()
                    .find(|h| matches!(&h.target, ClickTarget::SettingsSlider(s) if s == id))
                    .map(|h| h.rect);
                if let Some(rect) = rect {
                    self.win.slider_drag = Some((id, rect));
                    let t = ((mx - rect[0] - 8.0) / (rect[2] - 16.0)).clamp(0.0, 1.0);
                    self.set_slider(id, t);
                }
            }
            Some(ClickTarget::SettingsKeybind(action)) => {
                self.win.settings.recording = Some(action.clone());
            }
            Some(ClickTarget::SettingsKeybindReset(action)) => {
                self.win.settings.recording = None;
                self.config.keybindings.remove(action);
                self.save_config();
                self.apply_config();
                self.toast_done(format!("{action} restored to its default shortcut"));
            }
            Some(ClickTarget::SettingsTheme(i)) => {
                let names = rift_core::theme::available_themes();
                if let Some(name) = names.into_iter().nth(*i as usize) {
                    self.config.theme = name;
                    self.save_config();
                    self.apply_config();
                }
            }
            None => {
                let active_pane = self.win.tabs.get(self.win.active).map(|t| t.active_pane);
                if let Some(id) = active_pane {
                    let clicks = self.click_count();
                    if self.try_open_url(id) {
                        self.win.last_click = None;
                    } else {
                        self.press_selection(id, clicks);
                        self.press_link(id, clicks);
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
        if self.win.prompt_editor.is_some() {
            return;
        }
        use rift_pty::alacritty_terminal::term::TermMode;
        // the What's New body scrolls by pixels: trackpad deltas stay smooth
        // instead of snapping to whole lines
        if self.win.whats_new.open && self.win.confirm.is_none() {
            let px = match delta {
                MouseScrollDelta::LineDelta(_, y) => y * 3.0 * 24.0,
                MouseScrollDelta::PixelDelta(p) => p.y as f32,
            };
            // positive delta = wheel up = toward the top
            self.win.whats_new.scroll_by(-px);
            window.request_redraw();
            return;
        }
        if let Some(menu) = &mut self.win.chip_menu {
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
        if self.win.notifications.open && self.win.confirm.is_none() {
            let (mx, my) = self.win.mouse;
            let c = self.win.notifications_hits.card;
            if mx >= c[0] && mx < c[0] + c[2] && my >= c[1] && my < c[1] + c[3] {
                let px = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 3.0 * 20.0,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                self.win.notifications.scroll_by(-px);
                window.request_redraw();
                return;
            }
        }
        // the viewer and side panels scroll by pixels too (config.toml
        // beside Settings as well)
        let config_viewer_under = self.win.config_viewer && {
            let ([x, y, w, h], (mx, my)) = (self.win.viewer_rect, self.win.mouse);
            mx >= x && mx < x + w && my >= y && my < y + h
        };
        if !self.win.palette.open
            && (!self.win.settings.open || config_viewer_under)
            && self.win.confirm.is_none()
        {
            let (mx, my) = self.win.mouse;
            let inside = |r: &[f32; 4]| {
                r[2] > 0.0 && mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
            };
            let px = -match delta {
                MouseScrollDelta::LineDelta(_, y) => y * 3.0 * 20.0,
                MouseScrollDelta::PixelDelta(p) => p.y as f32,
            };
            let (ex_rect, rv_rect) = self.panel_rects();
            let handled = if self.win.viewer.is_open() && inside(&self.win.viewer_rect) {
                // sideways: a trackpad's x, or ⇧ + a mouse wheel
                let dx = -match delta {
                    MouseScrollDelta::LineDelta(x, _) => x * 3.0 * 20.0,
                    MouseScrollDelta::PixelDelta(p) => p.x as f32,
                };
                let (px, dx) = if self.win.modifiers.shift_key() && dx == 0.0 {
                    (0.0, px)
                } else {
                    (px, dx)
                };
                self.win.viewer.scroll_by(px);
                if dx != 0.0 {
                    self.win.viewer.scroll_x_by(dx, my);
                }
                true
            } else if inside(&ex_rect) {
                self.win.explorer.scroll_by(px);
                true
            } else if inside(&self.win.review.side_rect) {
                self.win.review.side_scroll_by(px);
                true
            } else if inside(&rv_rect) {
                // sideways: a trackpad's x, or ⇧ + a mouse wheel
                let dx = -match delta {
                    MouseScrollDelta::LineDelta(x, _) => x * 3.0 * 20.0,
                    MouseScrollDelta::PixelDelta(p) => p.x as f32,
                };
                let (px, dx) = if self.win.modifiers.shift_key() && dx == 0.0 {
                    (0.0, px)
                } else {
                    (px, dx)
                };
                self.win.review.scroll_by(px);
                if dx != 0.0 {
                    self.win.review.scroll_x_by(dx, my);
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
        self.win.scroll_accum += match delta {
            MouseScrollDelta::LineDelta(_, y) => y * self.config.wheel_lines(),
            MouseScrollDelta::PixelDelta(p) => p.y as f32 / self.line_h,
        };
        let lines = self.win.scroll_accum as i32;
        self.win.scroll_accum -= lines as f32;
        if lines == 0 {
            return;
        }
        if self.win.settings.open {
            // over Recent connections the list scrolls a row at a time; at
            // its ends the page takes over
            let (mx, my) = self.win.mouse;
            let over_list = self
                .win
                .hits
                .iter()
                .any(|h| h.target == ClickTarget::RemoteHistoryList && h.contains(mx, my));
            if !(over_list && self.win.settings.scroll_history(lines.signum() as isize)) {
                self.win.settings.scroll_by(lines as f32);
            }
            window.request_redraw();
            return;
        }
        if self.win.palette.open || self.win.confirm.is_some() || self.win.context_menu.open {
            return;
        }
        // the wheel scrolls the pane under the pointer (Warp), not the focused one
        let Some(target) = self
            .pane_under_mouse()
            .or_else(|| self.win.tabs.get(self.win.active).map(|t| t.active_pane))
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
            .win
            .tabs
            .get_mut(self.win.active)
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
        let (mx, my) = self.win.mouse;
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
    fn mouse_tracking(&self, pane: PaneId) -> Option<rift_pty::alacritty_terminal::term::TermMode> {
        use rift_pty::alacritty_terminal::term::TermMode;
        if !self.config.mouse_reporting || self.win.modifiers.shift_key() {
            return None;
        }
        let p = self.win.tabs.get(self.win.active)?.pane(pane)?;
        let mode = *p.session.term.lock().mode();
        mode.intersects(TermMode::MOUSE_MODE).then_some(mode)
    }

    /// 0-based screen cell under the pointer inside `pane`'s grid (None over
    /// history rows, block headers' gutters, or outside the grid).
    fn mouse_cell(&self, pane: PaneId) -> Option<(usize, usize)> {
        let p = self.win.tabs.get(self.win.active)?.pane(pane)?;
        let g = p.last_grid?;
        let (mx, my) = self.win.mouse;
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
        mode: rift_pty::alacritty_terminal::term::TermMode,
        button: u8,
        pressed: bool,
        motion: bool,
        cell: (usize, usize),
    ) {
        use rift_pty::alacritty_terminal::term::TermMode;
        let enc = crate::mouse::Encoding {
            sgr: mode.contains(TermMode::SGR_MOUSE),
            utf8: mode.contains(TermMode::UTF8_MOUSE),
        };
        if let Some(bytes) = crate::mouse::report(
            button,
            pressed,
            motion,
            self.win.modifiers,
            cell.0,
            cell.1,
            enc,
        ) && let Some(p) = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.pane(pane))
        {
            p.session.write_report(&bytes);
        }
    }

    /// Button press/release over a mouse-tracking program. Returns true when
    /// the event was reported (it then drives no local selection or menu).
    fn report_button(&mut self, button: u8, pressed: bool) -> bool {
        use rift_pty::alacritty_terminal::term::TermMode;
        if !pressed {
            let Some((pane, held, cell)) = self.win.mouse_report else {
                return false;
            };
            if held != button {
                return false;
            }
            self.win.mouse_report = None;
            let mode = self
                .win
                .tabs
                .get(self.win.active)
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
        // a link takes the left click over a program that tracks the mouse
        // too (Claude Code, Codex), as in Warp: it shows or opens the link
        // (Shift still reports nothing and selects)
        if button == crate::mouse::LEFT && self.url_under_mouse(pane).is_some() {
            return false;
        }
        let Some(cell) = self.mouse_cell(pane) else {
            return false;
        };
        // the click still focuses the pane it lands in
        if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
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
        self.win.mouse_report = Some((pane, button, cell));
        true
    }

    /// Pointer motion over a mouse-tracking program: drag reports (modes
    /// 1002/1003) while a button is held, hover reports (1003) otherwise.
    /// Returns true when the motion belongs to the program.
    fn report_motion(&mut self) -> bool {
        use rift_pty::alacritty_terminal::term::TermMode;
        if let Some((pane, button, last)) = self.win.mouse_report {
            let mode = self
                .win
                .tabs
                .get(self.win.active)
                .and_then(|t| t.pane(pane))
                .map(|p| *p.session.term.lock().mode())
                .unwrap_or(TermMode::empty());
            if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
                && let Some(cell) = self.mouse_cell(pane)
                && cell != last
            {
                self.send_mouse(pane, mode, button, true, true, cell);
                self.win.mouse_report = Some((pane, button, cell));
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
        if self.win.mouse_hover_cell != Some((pane, cell)) {
            self.win.mouse_hover_cell = Some((pane, cell));
            self.send_mouse(pane, mode, crate::mouse::NONE, true, true, cell);
        }
        true
    }

    /// Right/middle button release (press routing lives in on_right_press).
    pub(crate) fn on_other_button(&mut self, button: u8, pressed: bool, window: &Window) {
        let overlay = self.win.palette.open
            || self.win.settings.open
            || self.win.confirm.is_some()
            || self.win.whats_new.open
            || self.win.context_menu.open;
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
            if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
                if tab.active_pane != id
                    && let Some(prev) = tab.active_mut()
                {
                    prev.selected_block = None;
                }
                tab.active_pane = id;
            }
            self.win.viewer_focus = false;
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
        self.win.preedit.clear();
        if text.is_empty() {
            return;
        }
        let press = input::KeyPress::new(
            winit::keyboard::Key::Character(text.into()),
            Some(text.to_string()),
            None,
        );
        let saved = self.win.modifiers;
        self.win.modifiers = ModifiersState::empty();
        self.on_key(&press, event_loop, window);
        self.win.modifiers = saved;
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
        let Some(window) = self.win.window.clone() else {
            return;
        };
        let chord = match &cmd {
            MenuCommand::Action(a) => chord_for(&self.keymap, a),
            MenuCommand::Chord(c) => Some(c.clone()),
        };
        if let Some((press, mods)) = chord.as_deref().and_then(crate::script::key_press) {
            let saved = self.win.modifiers;
            self.win.modifiers = mods;
            self.on_key(&press, event_loop, &window);
            self.win.modifiers = saved;
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
        use rift_pty::alacritty_terminal::term::TermMode;
        let want = if self.win.focused {
            self.active_pane_id()
        } else {
            None
        };
        if want == self.win.focus_pane {
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
                p.session.write_report(seq);
            }
        };
        if let Some(old) = self.win.focus_pane {
            notify(&self.win.tabs, old, b"\x1b[O");
        }
        if let Some(new) = want {
            notify(&self.win.tabs, new, b"\x1b[I");
        }
        self.win.focus_pane = want;
    }
}

impl App {
    /// Open every tab/split of a launch configuration and queue its commands.
    fn apply_launch_config(&mut self, cfg: &rift_core::launch::LaunchConfig) {
        let first_new = self.win.tabs.len();
        for tab in &cfg.tabs {
            let (cwd, _) = Self::first_leaf(&tab.layout);
            self.spawn_tab_at(cwd.filter(|p| p.is_dir()));
            let root_pane = self.win.tabs[self.win.active].active_pane;
            if let Some(title) = &tab.title {
                self.win.tabs[self.win.active].custom_title = Some(title.clone());
            }
            // Warp's black/white have no Rift tab color; those stay unset.
            if let Some(color) = &tab.color {
                self.win.tabs[self.win.active].color = crate::state::TAB_COLORS
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case(color.trim()));
            }
            let mut focus = None;
            self.build_layout(&tab.layout, root_pane, &mut focus);
            // Splitting makes the new pane active, so the focused pane is
            // applied once the whole tree exists.
            if let Some(id) = focus {
                self.win.tabs[self.win.active].active_pane = id;
            }
        }
        self.win.active = (first_new + cfg.active_tab).min(self.win.tabs.len().saturating_sub(1));
        self.save_session();
    }

    /// cwd + commands of the first (top-left) pane of a layout.
    fn first_leaf(
        layout: &rift_core::launch::LaunchLayout,
    ) -> (Option<std::path::PathBuf>, Vec<String>) {
        use rift_core::launch::LaunchLayout;
        match layout {
            LaunchLayout::Pane { cwd, commands, .. } => (cwd.clone(), commands.clone()),
            LaunchLayout::Split { panes, .. } => panes
                .first()
                .map(Self::first_leaf)
                .unwrap_or((None, Vec::new())),
        }
    }

    /// Realize `layout` inside the region currently held by `pane`.
    fn build_layout(
        &mut self,
        layout: &rift_core::launch::LaunchLayout,
        pane: PaneId,
        focus: &mut Option<PaneId>,
    ) {
        use rift_core::launch::LaunchLayout;
        match layout {
            LaunchLayout::Pane {
                commands, focused, ..
            } => {
                if !commands.is_empty() {
                    self.pending_commands.insert(pane, commands.clone());
                }
                if *focused && focus.is_none() {
                    *focus = Some(pane);
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
                    self.build_layout(child, id, focus);
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
        let tab = &mut self.win.tabs[self.win.active];
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
            "inactive_pane_dim" => self.config.inactive_pane_dim = (1.0 + 39.0 * t).round() as u8,
            _ => return,
        }
        self.apply_config();
    }

    /// Vim modal editing: route the key through the editor's vim state
    /// first. Returns true when vim consumed it.
    fn vim_intercept(&mut self, event: &input::KeyPress) -> bool {
        use crate::vim::{VimKey, VimMode, VimOutcome};
        use winit::keyboard::{Key, NamedKey};
        let mods = self.win.modifiers;
        if mods.super_key() || mods.control_key() || mods.alt_key() {
            return false;
        }
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        else {
            return false;
        };
        if !pane.editor.vim_enabled() {
            return false;
        }
        let is_escape = matches!(event.logical_key, Key::Named(NamedKey::Escape));
        // Esc closes the completion menu before it changes modes
        if self.win.completion.open && is_escape {
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
                // completions follow typing in Insert only: a menu opened
                // in Normal/Replace would take the Esc meant for the mode
                if pane.editor.vim_mode() == Some(VimMode::Insert) {
                    self.update_completions();
                } else {
                    self.win.completion.close();
                    self.completion_pending = None;
                    self.completion_intent = None;
                }
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
        self.ring_bell();
        let active = self.win.active;
        if let Some((idx, tab)) = self.find_tab_of_pane(pane)
            && idx != active
        {
            tab.activity = true;
        }
    }

    /// The bell as `bell` sets it: the window flashes, a sound plays, both
    /// or neither. A BEL and Settings › Notifications › Ring.
    fn ring_bell(&mut self) {
        use rift_core::Bell;
        let bell = self.config.bell;
        if matches!(bell, Bell::Visual | Bell::Both) {
            self.win.bell_flash = Some(Instant::now());
        }
        if matches!(bell, Bell::Audible | Bell::Both)
            && self.last_beep.is_none_or(|t| t.elapsed() >= BELL_SOUND_GAP)
        {
            self.last_beep = Some(Instant::now());
            crate::notify::beep();
        }
        if let Some(w) = &self.win.window {
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
        use rift_core::notifications::{Event, Meaning, classify};
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
        loop {
            let now = Instant::now();
            let Some(step) = self.script.as_mut().and_then(|s| s.take_due(now)) else {
                break;
            };
            // steps act on the focused window, as typing does (`window <n>`
            // focuses one; a new window takes focus)
            self.enter_focused_window();
            let Some(window) = self.win.window.clone() else {
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
            // the step may have opened or closed a window: redraw the one
            // served now
            if let Some(w) = &self.win.window {
                w.request_redraw();
            }
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
            let saved = app.win.modifiers;
            app.win.modifiers = mods;
            app.on_key(&press, event_loop, window);
            app.win.modifiers = saved;
        };
        match step {
            Step::Wait(_) => {}
            Step::Key(chord) => press(self, &chord),
            Step::KeyRepeat(n, chord) => {
                if let Some((mut p, mods)) = key_press(&chord) {
                    let saved = self.win.modifiers;
                    self.win.modifiers = mods;
                    for i in 0..n {
                        p.repeat = i > 0;
                        self.on_key(&p, event_loop, window);
                    }
                    self.win.modifiers = saved;
                }
            }
            Step::KeyUp(chord) => {
                if let Some((mut p, mods)) = key_press(&chord) {
                    p.released = true;
                    p.text = None;
                    let saved = self.win.modifiers;
                    self.win.modifiers = mods;
                    self.on_key_release(&p);
                    self.win.modifiers = saved;
                }
            }
            Step::Type(text) => {
                for ch in text.chars() {
                    if ch == '\n' {
                        press(self, "enter");
                        continue;
                    }
                    let (p, mods) = char_press(ch);
                    let saved = self.win.modifiers;
                    self.win.modifiers = mods;
                    self.on_key(&p, event_loop, window);
                    self.win.modifiers = saved;
                }
            }
            Step::Shot(path) => self.win.capture = Some(path),
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
                self.win.focused = f;
                self.hotkey_window_focus(f);
                self.sync_focus();
            }
            Step::Hotkey => self.on_global_hotkey(event_loop),
            Step::Ime(text) => self.commit_text(&text, event_loop, window),
            Step::Preedit(text) => self.win.preedit = text,
            Step::Drop(path) => self.drop_file(&path),
            Step::Paste(text) => self.paste_text(&text),
            // the headless clipboard only: a script never touches the system's
            Step::CopyFiles(paths) => {
                if self.headless {
                    self.headless_files = paths;
                }
            }
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
                self.win.modifiers = m;
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
                if !self.win.editor_active
                    && let Some(s) = self.script.as_mut()
                {
                    let _ = s.rearm_wait(Duration::from_millis(50), Duration::from_secs(15));
                }
            }
            Step::NewWindow => self.open_new_window(event_loop),
            Step::Window(n) => {
                if self.enter_window_index(n)
                    && let Some(id) = self.win_id()
                {
                    self.note_window_focus(id);
                } else {
                    log::warn!("script: no window {n} ({} open)", self.window_count());
                }
            }
            Step::CloseWindow => self.request_close(CloseScope::Window, event_loop),
            Step::Quit => {
                self.perform_close(CloseScope::App, event_loop);
            }
        }
    }

    /// Tabs and panes as the control API reports them: every window's,
    /// window by window, numbered across all of them.
    fn tabs_info(&self) -> Vec<crate::control::TabInfo> {
        use crate::control::{PaneInfo, TabInfo};
        let now = Instant::now();
        let shell_prompt = self.config.prompt == rift_core::PromptMode::Shell;
        let tabs: Vec<(usize, bool, &TabState)> = self
            .windows_in_order()
            .into_iter()
            .enumerate()
            .flat_map(|(wi, w)| {
                w.tabs
                    .iter()
                    .enumerate()
                    .map(move |(i, t)| (wi, i == w.active, t))
            })
            .collect();
        tabs.into_iter()
            .enumerate()
            .map(|(i, (window, active, t))| TabInfo {
                index: i,
                window,
                title: t.display_title(),
                active,
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
                        // computed before the literal below: its `cwd` guard lives to
                        // the end of the statement, and the chips lock `cwd` again
                        let chips = if shell_prompt {
                            Vec::new()
                        } else {
                            let elapsed = t.elapsed(self.config.elapsed_counts, now);
                            editor::pane_chip_views(&self.config, &self.theme, p, elapsed)
                                .iter()
                                .map(|v| v.wire())
                                .collect()
                        };
                        PaneInfo {
                            id: p.session.id,
                            active: p.session.id == t.active_pane,
                            cwd: p.session.cwd.lock().unwrap().clone(),
                            job: p.session.foreground_job_name(),
                            last_command: last.as_ref().map(|l| l.0.clone()),
                            last_exit: last.and_then(|l| l.1),
                            durable: p.session.service_id.is_some(),
                            git: p.git.as_ref().map(|g| rift_remote::protocol::PaneGit {
                                branch: g.branch.clone(),
                                ahead: g.ahead as u64,
                                behind: g.behind as u64,
                                dirty: g.dirty,
                                files: g.files as u64,
                                added: g.added as u64,
                                removed: g.removed as u64,
                            }),
                            // what the desktop draws above the command line (none in
                            // shell-prompt mode), for a phone's chip row
                            chips: Some(chips),
                            waiting: self.pane_waiting(p),
                            running: Self::pane_running(p),
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    /// `p`'s running command for a paired phone's Running list: when it
    /// started and its latest line; none at the prompt. A coding agent also
    /// names its session and says whether it waits at its own prompt.
    fn pane_running(p: &PaneState) -> Option<rift_remote::protocol::PaneRunning> {
        use rift_core::notifications::{agent_title_idle, agent_title_working};
        let mut r = terminal::remote_running(&p.session)?;
        // a reattached agent's job is its version ("2.1.296"): its title
        // still carries the agent's mark
        let marked = agent_title_idle(&p.title) || agent_title_working(&p.title);
        if Self::pane_agent(p).is_some() || marked {
            let title = rift_core::notifications::agent_session_title(&p.title);
            r.title = (!title.is_empty()).then(|| title.to_string());
            r.idle = !p.running();
        }
        Some(r)
    }

    /// What `p`'s running command waits for (a paired phone's "Needs you"):
    /// a password, an agent's menu on the screen, else an unread "needs your
    /// input" for the pane in the attention center (its text is the
    /// program's, bounded on the wire; the phone answers with text its user
    /// types, as in the composer).
    fn pane_waiting(&self, p: &PaneState) -> Option<rift_remote::protocol::Waiting> {
        use rift_core::prompts::Waiting as W;
        use rift_remote::protocol::{Waiting, WaitingKind};
        let running = p.session.has_foreground_job()
            || p.session
                .blocks
                .lock()
                .unwrap()
                .last()
                .is_some_and(|b| matches!(b.state, BlockState::Running { .. }));
        if running {
            let hidden = p.session.input_hidden();
            let line = p.session.cursor_line_text();
            // menus only from a coding agent's screen: a phone's answer
            // types into the program, and any other program can print a
            // menu-shaped screen
            let rows = if Self::pane_agent(p).is_some() {
                p.session.screen_rows()
            } else {
                Vec::new()
            };
            match rift_core::prompts::waiting(hidden, &line, &rows) {
                Some(W::Password { prompt }) => {
                    return Some(Waiting {
                        kind: WaitingKind::Password,
                        prompt,
                        choices: Vec::new(),
                        selected: 0,
                        numbered: false,
                        context: Vec::new(),
                    });
                }
                Some(W::Choice(c)) => {
                    return Some(Waiting {
                        kind: WaitingKind::Choice,
                        prompt: c.prompt,
                        choices: c.choices,
                        selected: c.selected,
                        numbered: c.numbered,
                        context: c.context,
                    });
                }
                None => {}
            }
        }
        // a request the user answered since (typed anything into the pane)
        // or that outlived its command is not waiting any more
        if !running {
            return None;
        }
        let inputs = p.session.inputs();
        self.attention
            .iter()
            .rev()
            .find(|a| {
                a.pane == p.session.id
                    && !a.read
                    && a.kind == AttentionKind::Request
                    && a.inputs == inputs
            })
            .map(|a| Waiting {
                kind: WaitingKind::Input,
                prompt: if a.detail.is_empty() || a.title.is_empty() {
                    format!("{}{}", a.title, a.detail)
                } else {
                    format!("{} · {}", a.title, a.detail)
                },
                choices: Vec::new(),
                selected: 0,
                numbered: false,
                context: Vec::new(),
            })
    }

    /// `complete` from the remote gateway: the providers read the
    /// filesystem and Git, so they run on a thread of their own, which
    /// answers the request.
    fn complete_remote(&mut self, p: crate::control::Pending) {
        use crate::control::{Request, Response};
        let Request::Complete { pane, line, cursor } = p.request else {
            return;
        };
        let shell = self.shell_history.clone();
        let Some(pn) = self.find_tab_of_pane(pane).and_then(|(_, t)| t.pane(pane)) else {
            let _ = p.reply.send(Response::error("no such pane"));
            return;
        };
        if !line.is_char_boundary(cursor) {
            let _ = p.reply.send(Response::error("cursor inside a character"));
            return;
        }
        let unix = |t: std::time::SystemTime| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs())
        };
        let blocks: Vec<(String, Option<u64>)> = pn
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .filter(|b| !b.command.trim().is_empty())
            .map(|b| (b.command.clone(), b.finished_at.and_then(unix)))
            .collect();
        let editor: Vec<String> = pn
            .editor
            .history
            .iter()
            .rev()
            .take(200)
            .rev()
            .cloned()
            .collect();
        let cwd = pn.session.local_cwd();
        // the pane's own $PATH (`rift-use`, a venv), as the desktop menu uses;
        // a pane on another machine gets none (an empty one): this machine's
        // commands are not there
        let path = if pn.session.remote_cwd().is_some() {
            Some(String::new())
        } else {
            pn.session
                .user_vars
                .lock()
                .unwrap()
                .get(rift_core::osc::USER_VAR_PATH)
                .filter(|p| !p.is_empty())
                .cloned()
        };
        let reply = p.reply;
        let spawned = std::thread::Builder::new()
            .name("remote-complete".into())
            .spawn(move || {
                let page = crate::remote::complete_page(
                    &line,
                    cursor,
                    &blocks,
                    &editor,
                    &shell,
                    cwd.as_deref(),
                    path.as_deref().map(std::ffi::OsStr::new),
                );
                let _ = reply.send(Response {
                    ok: true,
                    pane: Some(pane),
                    complete: Some(page),
                    ..Default::default()
                });
            });
        if let Err(e) = spawned {
            log::warn!("remote completion thread: {e}");
        }
    }

    /// `menu` / `diff` from the remote gateway: they read the filesystem
    /// and diff the repository (a large one takes seconds), so only the
    /// pane's folder is read here; a thread of their own answers.
    fn chip_remote(&mut self, p: crate::control::Pending) {
        use crate::control::{Request, Response};
        use crate::views::chip_menu::{
            ChipKind, remote_diff, remote_docker, remote_menu, remote_pr, remote_runtimes,
        };
        use rift_remote::MenuKind;
        let pane = match &p.request {
            Request::Menu { pane, .. } | Request::Diff { pane, .. } => *pane,
            _ => return,
        };
        let mode = self.config.elapsed_counts;
        let Some((_, tab)) = self.find_tab_of_pane(pane) else {
            let _ = p.reply.send(Response::error("no such pane"));
            return;
        };
        let elapsed = tab.elapsed(mode, Instant::now());
        let started = tab.started;
        let Some(pn) = tab.pane(pane) else {
            let _ = p.reply.send(Response::error("no such pane"));
            return;
        };
        let cwd = pn.session.local_cwd();
        // the chips that read the window's own state answer at once; the
        // interpreters' installs are looked up on the thread below
        let detail = match &p.request {
            Request::Menu { kind, .. } => match kind {
                MenuKind::Pr => Some(pn.pr.as_ref().map(remote_pr).ok_or("no pull request")),
                MenuKind::Docker => Some(
                    pn.docker
                        .as_ref()
                        .map(remote_docker)
                        .ok_or("no compose project"),
                ),
                MenuKind::Elapsed => Some(
                    crate::views::prompt_chips::elapsed_line(mode, started, elapsed)
                        .zip(elapsed)
                        .map(|(line, e)| rift_remote::protocol::ChipDetail::Elapsed {
                            seconds: e.time.as_secs(),
                            counting: e.counting,
                            line,
                        })
                        .ok_or("no time on this tab yet"),
                ),
                _ => None,
            },
            _ => None,
        };
        if let Some(detail) = detail {
            let _ = p.reply.send(match detail {
                Ok(detail) => Response {
                    ok: true,
                    pane: Some(pane),
                    menu: Some(rift_remote::Menu::Chip { detail }),
                    ..Default::default()
                },
                Err(why) => Response::error(why),
            });
            // a fresher `docker compose ps` for the next look
            if let (Some(dir), Some(worker)) = (cwd, &self.docker_worker)
                && matches!(
                    &p.request,
                    Request::Menu {
                        kind: MenuKind::Docker,
                        ..
                    }
                )
            {
                worker.request(pane, dir.into());
            }
            return;
        }
        let vars = pn.session.user_vars.lock().unwrap().clone();
        let (request, reply) = (p.request, p.reply);
        let spawned = std::thread::Builder::new()
            .name("remote-chip".into())
            .spawn(move || {
                let answer = match request {
                    Request::Menu {
                        kind: kind @ (MenuKind::Python | MenuKind::Php),
                        ..
                    } => {
                        let chip = if kind == MenuKind::Php {
                            ChipKind::Php
                        } else {
                            ChipKind::Python
                        };
                        let dir = cwd.as_deref().map(std::path::Path::new);
                        Response {
                            ok: true,
                            pane: Some(pane),
                            menu: Some(rift_remote::Menu::Chip {
                                detail: remote_runtimes(
                                    chip,
                                    &vars,
                                    dir.unwrap_or(std::path::Path::new("/")),
                                ),
                            }),
                            ..Default::default()
                        }
                    }
                    Request::Menu { kind, .. } => Response {
                        ok: true,
                        pane: Some(pane),
                        menu: Some(remote_menu(kind, cwd.as_deref())),
                        ..Default::default()
                    },
                    Request::Diff { path, staged, .. } => {
                        match cwd.and_then(|c| remote_diff(&c, &path, staged)) {
                            Some(diff) => Response {
                                ok: true,
                                pane: Some(pane),
                                diff: Some(diff),
                                ..Default::default()
                            },
                            None => Response::error(crate::remote::NO_CHANGE),
                        }
                    }
                    _ => return,
                };
                let _ = reply.send(answer);
            });
        if let Err(e) = spawned {
            log::warn!("remote chip thread: {e}");
        }
    }

    /// Revoke the old listener and queued requests whenever the level changes.
    fn sync_control_server(&mut self) {
        let level = self.control_level();
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
        if req.level() > self.control_level() {
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
                windows: self
                    .windows_in_order()
                    .into_iter()
                    .enumerate()
                    .filter(|(_, w)| w.dropdown.is_some())
                    .map(|(window, _)| crate::control::WindowInfo {
                        window,
                        kind: "dropdown".into(),
                    })
                    .collect(),
                ..Default::default()
            },
            Request::Blocks {
                pane,
                before,
                limit,
                meta,
            } => match self.find_tab_of_pane(pane).and_then(|(_, t)| t.pane(pane)) {
                Some(p) => {
                    let page = terminal::remote_blocks(&p.session, before, limit, meta);
                    Response {
                        ok: true,
                        pane: Some(pane),
                        blocks: page.blocks,
                        has_older: Some(page.has_older),
                        oldest: page.oldest,
                        ..Default::default()
                    }
                }
                None => Response::error("no such pane"),
            },
            Request::Screen { pane, force, since } => {
                let theme = self.theme.clone();
                match self.find_tab_of_pane(pane).and_then(|(_, t)| t.pane(pane)) {
                    Some(p) => {
                        let (generation, screen) =
                            terminal::remote_screen(&p.session, &theme, force, since);
                        Response {
                            ok: true,
                            pane: Some(pane),
                            unchanged: screen.is_none(),
                            screen: screen.flatten(),
                            generation: Some(generation),
                            ..Default::default()
                        }
                    }
                    None => Response::error("no such pane"),
                }
            }
            Request::Output {
                pane,
                block,
                from,
                limit,
            } => match self.find_tab_of_pane(pane).and_then(|(_, t)| t.pane(pane)) {
                Some(p) => match terminal::block_output_range(&p.session, block, from, limit) {
                    Some(page) => Response {
                        ok: true,
                        pane: Some(pane),
                        output: Some(page),
                        ..Default::default()
                    },
                    None => Response::error(crate::remote::NO_BLOCK),
                },
                None => Response::error("no such pane"),
            },
            // answered off this thread (`complete_remote`, `chip_remote`)
            Request::Complete { .. } | Request::Menu { .. } | Request::Diff { .. } => {
                Response::error("answered by its own thread")
            }
            Request::Focus { tab, pane } => {
                self.win.settings.open = false;
                let target = match (tab, pane) {
                    (_, Some(p)) => self.find_tab_of_pane(p).map(|(i, _)| (i, Some(p))),
                    (Some(t), None) if t < self.win.tabs.len() => Some((t, None)),
                    (None, None) => Some((self.win.active, None)),
                    _ => None,
                };
                let Some((ti, pane)) = target else {
                    return Response::error("no such tab or pane");
                };
                self.win.active = ti;
                self.win.tabs[ti].activity = false;
                if let Some(p) = pane {
                    self.win.tabs[ti].active_pane = p;
                }
                // the window it is in comes forward (`route_control` serves
                // it); a hidden drop-down window drops down
                self.reveal_hotkey_window();
                if let Some(id) = self.win_id() {
                    self.note_window_focus(id);
                }
                if let Some(w) = self.win.window.as_ref().filter(|_| !self.headless) {
                    w.set_minimized(false);
                    w.focus_window();
                }
                ok(Some(self.win.tabs[ti].active_pane))
            }
            Request::Split { direction, cwd } => {
                let axis = match direction.as_str() {
                    "right" => Axis::Horizontal,
                    "down" => Axis::Vertical,
                    other => return Response::error(format!("direction {other:?}: right or down")),
                };
                let Some(pane) = self.win.tabs.get(self.win.active).map(|t| t.active_pane) else {
                    return Response::error("no tab");
                };
                // a missing or nonexistent --cwd starts where a new pane would
                let cwd = cwd
                    .map(std::path::PathBuf::from)
                    .filter(|p| p.is_dir())
                    .or_else(|| self.start_dir());
                let id = self.split_pane(pane, axis, cwd);
                if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
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
                let id = self.win.tabs[self.win.active].active_pane;
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
            Request::RemoteInput {
                pane,
                device,
                command,
                keys,
                text,
            } => self.remote_input(pane, &device, command, keys, text),
            Request::RemoteOpen { pane, device, kind } => self.remote_open(pane, &device, kind),
            // needs the event loop (closing the last tab quits): answered in `user_event`
            Request::RemoteClose { .. } => Response::error("remote-close needs the event loop"),
            Request::RemoteFit { pane, cols, rows } => self.remote_fit(pane, cols, rows),
            Request::RemoteResetElapsed { pane, .. } => {
                if !self.config.remote_access {
                    return Response::error("remote access is off");
                }
                if self.find_tab_of_pane(pane).is_none() {
                    return Response::error("no such pane");
                }
                self.reset_tab_timer_of(pane);
                if let Some(w) = &self.win.window {
                    w.request_redraw();
                }
                ok(Some(pane))
            }
            Request::RemoteStatus => {
                // the gateway's status changed: the chip follows at once
                self.remote.notices += 1;
                self.poll_remote(true);
                ok(None)
            }
            Request::PrepareRestart { token } => {
                // only for an update in flight from this install: its marker
                // (readable by the user only) holds the token
                use rift_core::update;
                if self.update.busy {
                    return Response::error("this window is updating Rift");
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
                if let Err(e) = rift_core::session_state::save_to(&file, &self.saved_session()) {
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
            self.win
                .hits
                .iter()
                .find(|h| pred(&h.target))
                .map(|h| h.rect)
        };
        let settings_shown = self.win.settings.open && self.win.settings.tab;
        let n = self.win.tabs.len();
        let tabs = self
            .win
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| TabNode {
                title: t.display_title(),
                selected: i == self.win.active && !settings_shown,
                rect: hit(&|c| *c == ClickTarget::TabSelect(i)),
            })
            .chain(self.win.settings.tab.then(|| TabNode {
                title: "Settings".into(),
                selected: settings_shown,
                rect: hit(&|c| *c == ClickTarget::TabSelect(n)),
            }))
            .collect();

        let mut panes = Vec::new();
        let mut finished = Vec::new();
        // the Settings tab covers the shell tab's panes
        if let Some(tab) = self
            .win
            .tabs
            .get(self.win.active)
            .filter(|_| !settings_shown)
        {
            for p in &tab.panes {
                let id = p.session.id;
                let visible = tab.zoomed.is_none_or(|z| z == id);
                if !visible {
                    continue;
                }
                let active = id == tab.active_pane;
                // no directory reported yet: the pane's own title, else the
                // tab's, never an empty "Pane "
                // (the lock is released first: display_title takes it again)
                let reported = p.session.cwd.lock().unwrap().clone();
                let cwd = reported
                    .filter(|c| !c.is_empty())
                    .or_else(|| Some(p.title.clone()).filter(|t| !t.is_empty()))
                    .unwrap_or_else(|| tab.display_title());
                let running = p.session.foreground_job_name();
                let snapshot = p.session.blocks.lock().unwrap().snapshot();
                let mut blocks = Vec::new();
                for b in &snapshot {
                    let status = match b.state {
                        rift_core::BlockState::Prompt => continue,
                        rift_core::BlockState::Running { .. } => "running".to_string(),
                        rift_core::BlockState::Done { exit, duration } => format!(
                            "{}, {:.1}s",
                            match exit {
                                Some(0) => "succeeded".to_string(),
                                Some(c) => format!("failed with exit code {c}"),
                                None => "finished".to_string(),
                            },
                            duration.as_secs_f32()
                        ),
                    };
                    if matches!(b.state, rift_core::BlockState::Done { .. }) {
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
                let editor = (active && self.win.editor_active).then(|| EditorNode {
                    value: p.editor.buffer.clone(),
                    rect: hit(&|c| *c == ClickTarget::Editor(id)),
                });
                // the "Time on this tab" chip, and its card's Reset button
                // while the card is open
                let mut chip_controls = Vec::new();
                for (rect, _, _) in self
                    .win
                    .info_chips
                    .iter()
                    .filter(|(_, pane, _)| *pane == id)
                {
                    let text = tab
                        .elapsed(self.config.elapsed_counts, Instant::now())
                        .map(|e| {
                            format!(
                                "{}, {}",
                                crate::views::prompt_chips::elapsed_text(e.time),
                                if e.counting { "counting" } else { "paused" }
                            )
                        })
                        .unwrap_or_default();
                    chip_controls.push(Control {
                        label: format!("Time on this tab, {text}"),
                        role: Role::Label,
                        toggled: None,
                        selected: false,
                        rect: Some(*rect),
                    });
                    if let Some(button) = self
                        .win
                        .tip_shown
                        .as_ref()
                        .filter(|t| t.action.is_some() && t.rect == *rect)
                        .and_then(|_| self.win.tip_drawn.as_ref()?.action)
                    {
                        chip_controls.push(Control {
                            label: "Reset time on this tab".into(),
                            role: Role::Button,
                            toggled: None,
                            selected: false,
                            rect: Some(button),
                        });
                    }
                }
                let mut label = format!("Pane {cwd}");
                if let Some(job) = running.filter(|_| !self.win.editor_active || !active) {
                    label.push_str(&format!(", running {job}"));
                }
                // title bar buttons, and the welcome rows of a fresh pane
                let welcome = welcome_entries(&self.keymap);
                let controls = self
                    .win
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
                    .chain(chip_controls)
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
            let prev = self.win.a11y_done.insert(pane, block);
            if prev.is_some_and(|b| b < block) {
                self.win.a11y_announcement = Some(format!("{command} {status}"));
            }
        }

        let overlay = if let Some(c) = &self.win.confirm {
            let (title, description) = match &c.action {
                ConfirmAction::Close(scope) if self.quits(*scope) => ("Quit Rift?".to_string(), {
                    let mut d = close_description(c.commands.len(), &self.connected_phones());
                    if !c.commands.is_empty() {
                        d.push_str(&format!(" Running: {}", c.commands.join(", ")));
                    }
                    d
                }),
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
                    format!("Update to Rift v{}?", plan.manifest.version),
                    format!("{} {}", plan.description, plan.note)
                        .trim()
                        .to_string(),
                ),
                ConfirmAction::RevokeDevice { name, .. } => (
                    format!("Revoke {name}?"),
                    "It disconnects now and cannot reconnect without pairing again.".into(),
                ),
                ConfirmAction::AllowOperate { name, .. } => {
                    (format!("Let {name} run commands?"), ALLOW_OPERATE.into())
                }
            };
            let label = match &c.action {
                ConfirmAction::Close(_) => "Close",
                ConfirmAction::Paste { .. } => "Paste",
                ConfirmAction::InstallRemote(_) => "Install",
                ConfirmAction::Revert { .. } => "Revert",
                ConfirmAction::Update(plan) if plan.open_page => "Open download page",
                ConfirmAction::Update(_) => "Update",
                ConfirmAction::RevokeDevice { .. } => "Revoke",
                ConfirmAction::AllowOperate { .. } => "Allow",
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
                    button(cancel, self.win.confirm_rects[0]),
                    button(label, self.win.confirm_rects[1]),
                ],
            })
        } else if let Some(ed) = &self.win.prompt_editor {
            use crate::views::prompt_editor::Target;
            let controls = self
                .win
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
                rect: Some(self.win.prompt_editor_layout.card),
            })
        } else if let Some(menu) = &self.win.chip_menu {
            let results = menu.results();
            let controls = self
                .win
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
                rect: Some(self.win.chip_menu_card),
            })
        } else if self.win.whats_new.open {
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
                    button("Close", self.win.whats_new_hits.close),
                    button("Got it", self.win.whats_new_hits.done),
                ],
                rect: Some(self.win.whats_new_hits.card),
            })
        } else if self.win.notifications.open {
            let h = &self.win.notifications_hits;
            let entries = self.notification_entries();
            let shown = self.filtered_notifications();
            let highlighted = self
                .win
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
                    toggled: Some(*f == self.win.notifications.filter),
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
        } else if self.win.context_menu.open {
            Some(Overlay::Menu {
                items: self
                    .win
                    .context_menu
                    .items
                    .iter()
                    .zip(&self.win.context_menu_rects)
                    .enumerate()
                    .filter(|(_, (item, _))| item.enabled)
                    .map(|(i, (item, r))| Control {
                        label: item.label.to_string(),
                        role: Role::MenuItem,
                        toggled: None,
                        selected: self.win.context_menu.selected == Some(i),
                        rect: Some(*r),
                    })
                    .collect(),
            })
        } else if self.win.palette.open {
            Some(Overlay::Dialog {
                label: "Command palette".into(),
                input: Some(("Search commands".into(), self.win.palette.query.clone())),
                status: Some(format!("{} results", self.win.palette_results.len())),
                // the mode chips, then the rows
                controls: self
                    .win
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
                                selected: *mode == self.win.palette.mode,
                                rect: Some(h.rect),
                            })
                        }
                        _ => None,
                    })
                    .chain(self.win.palette_row_hits.iter().filter_map(|(r, i)| {
                        let m = self.win.palette_results.get(*i)?;
                        let mut label = m.item.title.clone();
                        if !m.item.shortcut.is_empty() {
                            label.push_str(&format!(", {}", m.item.shortcut.join(" ")));
                        }
                        Some(Control {
                            label,
                            role: Role::ListBoxOption,
                            toggled: None,
                            selected: *i == self.win.palette.selected,
                            rect: Some(*r),
                        })
                    }))
                    .collect(),
                rect: None,
            })
        } else if self.win.settings.open {
            // switches read their state from the config; an unset right
            // Option key shows the value it follows
            let mut shown = self.config.clone();
            shown.right_option_as_meta = Some(shown.right_option_meta());
            let values = toml::Value::try_from(&shown).ok();
            let current = crate::views::settings::Page::ALL
                .iter()
                .position(|p| *p == self.win.settings.page);
            let controls = self
                .win
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
                        // the name says which rows show and how to scroll
                        ClickTarget::RemoteHistoryList => (Role::Label, None, false),
                        ClickTarget::SettingsOption(i) => (
                            Role::ListBoxOption,
                            None,
                            self.win
                                .settings
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
                        | ClickTarget::SettingsCheckUpdates
                        | ClickTarget::SettingsWhatsNew
                        | ClickTarget::SettingsPlaySound(_)
                        | ClickTarget::SettingsTestAlert(_)
                        | ClickTarget::SettingsRingBell
                        | ClickTarget::SettingsNotifyPermission
                        | ClickTarget::SettingsEditPrompt
                        | ClickTarget::SettingsKeybind(_)
                        | ClickTarget::SettingsKeybindReset(_)
                        | ClickTarget::RemotePair
                        | ClickTarget::RemoteCopyLink
                        | ClickTarget::RemoteInstallTunnel
                        | ClickTarget::RemoteRelayRetry
                        | ClickTarget::RemoteRelaySettings
                        | ClickTarget::RemoteRevoke(_) => (Role::Button, None, false),
                        ClickTarget::RemoteViewOnly => (
                            Role::Switch,
                            Some(self.win.settings.remote.view_only),
                            false,
                        ),
                        // the name carries the URL (or "editing: …")
                        ClickTarget::RemoteRelay => (Role::TextInput, None, false),
                        ClickTarget::RemotePermission(id, view) => (
                            Role::RadioButton,
                            None,
                            self.win
                                .settings
                                .remote
                                .devices
                                .iter()
                                .any(|d| d.id == *id && d.view_only == *view),
                        ),
                        _ => return None,
                    };
                    let label = self
                        .win
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
                label: format!("Settings, {}", self.win.settings.page.label()),
                input: Some(("Search settings".into(), self.win.settings.query.clone())),
                status: None,
                controls,
                rect: None,
            })
        } else if self.win.history.open {
            let hp = &self.win.history;
            let controls = self
                .win
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
        } else if self.win.search.open {
            let (label, input, status) = if self.win.filter_editing {
                let query = self
                    .win
                    .tabs
                    .get(self.win.active)
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
                    ("Find in terminal".into(), self.win.search.query.clone()),
                    match (&self.win.search.error, self.win.search.position()) {
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
            announcement: self.win.a11y_announcement.clone(),
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
                .win
                .explorer_hits
                .iter()
                .map(|(r, hit)| match hit {
                    ExplorerHit::Row(i) => {
                        let row = &self.win.explorer.rows[*i];
                        let kind = if !row.is_dir {
                            "file"
                        } else if self.win.explorer.expanded.contains(&row.path) {
                            "folder, expanded"
                        } else {
                            "folder, collapsed"
                        };
                        control(
                            format!("{}, {kind}", row.name),
                            Role::TreeItem,
                            self.win.explorer.selected.as_ref() == Some(&row.path),
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
            let notices: Vec<&str> = self.win.explorer.notices().collect();
            out.push(PanelNode {
                label: "File explorer".into(),
                rect: Some(ex_rect),
                text: (!notices.is_empty()).then(|| notices.join("\n")),
                controls,
                focused: self.win.explorer.focused,
            });
        }
        if rv_rect[2] > 0.0 {
            let hunk_label = |id: &Option<rift_core::review::HunkId>| {
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
                .win
                .review
                .latent
                .iter()
                .filter(|(_, l)| !self.win.review_hits.iter().any(|(_, h)| h == l));
            let controls = self
                .win
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
                            if self.win.review.expanded.contains(&(*staged, p.clone())) {
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
                        ReviewHit::Maximize if self.win.review.maximized => "Restore".into(),
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
        if let Some(path) = &self.win.viewer.path {
            let mut text = self
                .win
                .viewer
                .error
                .clone()
                .unwrap_or_else(|| self.win.viewer.lines.join("\n"));
            if text.len() > 20_000 {
                let cut = (0..=20_000)
                    .rev()
                    .find(|&i| text.is_char_boundary(i))
                    .unwrap_or(0);
                text.truncate(cut);
                text.push('…');
            }
            let controls = self
                .win
                .viewer_hits
                .iter()
                .map(|(r, hit)| match hit {
                    ViewerHit::Close => control("Close viewer".into(), Role::Button, false, *r),
                    ViewerHit::Menu => control("Viewer menu".into(), Role::Button, false, *r),
                    ViewerHit::Mode(m) => control(
                        if *m { "Rendered" } else { "Raw" }.into(),
                        Role::RadioButton,
                        self.win.viewer.rendered == *m,
                        *r,
                    ),
                })
                .collect();
            out.push(PanelNode {
                label: format!("File viewer, {}", path.display()),
                rect: Some(self.win.viewer_rect),
                text: Some(text),
                controls,
                // it takes the keys while open
                focused: true,
            });
        }
        // the tab bar's remote chip (a phone is connected)
        if let Some(label) = self.remote_chip()
            && let Some(h) = self
                .win
                .hits
                .iter()
                .find(|h| h.target == ClickTarget::RemoteChip)
        {
            out.push(PanelNode {
                label: "Remote access".into(),
                rect: Some(h.rect),
                text: None,
                controls: vec![control(
                    format!("{label}, connected: {}", self.remote_clients_text()),
                    Role::Button,
                    false,
                    h.rect,
                )],
                focused: false,
            });
        }
        // the tab bar's "Update Rift" chip
        if let Some(version) = self.update_chip()
            && let Some(h) = self
                .win
                .hits
                .iter()
                .find(|h| h.target == ClickTarget::UpdateChip)
        {
            out.push(PanelNode {
                label: "Update available".into(),
                rect: Some(h.rect),
                text: None,
                controls: vec![control(
                    format!("Update Rift, v{version} available"),
                    Role::Button,
                    false,
                    h.rect,
                )],
                focused: false,
            });
        }
        // status toasts: the message, its button and ✕
        for h in &self.win.toast_hits {
            let Some(t) = self.win.toasts.iter().find(|t| t.id == h.id) else {
                continue;
            };
            let mut controls = Vec::new();
            if let (Some(r), Some(a)) = (h.action, t.action) {
                controls.push(control(a.label().into(), Role::Button, false, r));
            }
            if let Some(r) = h.close {
                controls.push(control(
                    format!("Dismiss: {}", t.text),
                    Role::Button,
                    false,
                    r,
                ));
            }
            out.push(PanelNode {
                label: match t.kind {
                    crate::views::confirm::ToastKind::Error => "Error".into(),
                    _ => "Status".into(),
                },
                rect: Some(h.rect),
                text: Some(t.text.clone()),
                controls,
                focused: false,
            });
        }
        // notification cards (inbox entries, Rift's own notices): the card
        // acts on a click, ✕ dismisses
        for h in &self.win.note_toast_hits {
            let title = self
                .win
                .app_notes
                .iter()
                .find(|n| n.id == h.id)
                .map(|n| n.title.clone())
                .or_else(|| {
                    self.attention
                        .iter()
                        .find(|a| a.id == h.id)
                        .map(|a| a.title.clone())
                })
                .unwrap_or_default();
            out.push(PanelNode {
                label: "Notification".into(),
                rect: Some(h.rect),
                text: None,
                controls: vec![
                    control(title.clone(), Role::Button, false, h.rect),
                    control(format!("Dismiss: {title}"), Role::Button, false, h.close),
                ],
                focused: false,
            });
        }
        out
    }

    /// Push the accessibility tree when a screen reader is attached and
    /// something changed.
    fn update_a11y(&mut self, window: &Window) {
        let Some(mut adapter) = self.win.a11y.take() else {
            return;
        };
        adapter.update_if_active(|| {
            let snapshot = self.a11y_snapshot(window.scale_factor());
            let built = crate::a11y::build(&snapshot);
            self.win.a11y_targets = built.targets;
            if built.fingerprint == self.win.a11y_fingerprint {
                // unchanged: keep focus, send no nodes
                return accesskit::TreeUpdate {
                    nodes: Vec::new(),
                    tree: None,
                    tree_id: accesskit::TreeId::ROOT,
                    focus: built.update.focus,
                };
            }
            self.win.a11y_fingerprint = built.fingerprint;
            built.update
        });
        self.win.a11y = Some(adapter);
    }

    fn on_access_event(
        &mut self,
        event: accesskit_winit::WindowEvent,
        event_loop: &ActiveEventLoop,
    ) {
        use accesskit_winit::WindowEvent as A;
        let Some(window) = self.win.window.clone() else {
            return;
        };
        match event {
            A::InitialTreeRequested => {
                log::info!("accessibility: screen reader attached");
                // the next update must carry the full tree
                self.win.a11y_fingerprint.clear();
                self.update_a11y(&window);
            }
            A::ActionRequested(req) => {
                if !crate::a11y::activates(req.action) {
                    return;
                }
                let Some(r) = self.win.a11y_targets.get(&req.target_node).copied() else {
                    return;
                };
                // act exactly like a pointer click on the node
                let saved = self.win.mouse;
                self.win.mouse = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
                self.on_left(ElementState::Pressed, event_loop, &window);
                self.on_left(ElementState::Released, event_loop, &window);
                self.win.mouse = saved;
                window.request_redraw();
            }
            A::AccessibilityDeactivated => self.win.a11y_fingerprint.clear(),
        }
    }

    fn dump_state(&mut self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        // every window (opening order); the rest of the dump is this one's
        let _ = writeln!(
            out,
            "windows={} window={} focused_window={:?}",
            self.window_count(),
            self.window_index(),
            self.focused_window
                .and_then(|id| self.window_order.iter().position(|w| *w == id))
        );
        for (i, w) in self.windows_in_order().into_iter().enumerate() {
            let titles: Vec<String> = w.tabs.iter().map(|t| t.display_title()).collect();
            let panes: Vec<PaneId> = w
                .tabs
                .iter()
                .flat_map(|t| t.panes.iter().map(|p| p.session.id))
                .collect();
            let _ = writeln!(
                out,
                "window {i}: tabs={} active={} panes={panes:?} titles={titles:?} {}",
                w.tabs.len(),
                w.active,
                Self::window_kind_dump(w)
            );
        }
        let _ = writeln!(
            out,
            "theme_bg={:?} control_api={}",
            self.theme.bg, self.config.control_api
        );
        let _ = writeln!(
            out,
            "remote_access={} control_level={:?} gateway={:?} gateway_pid={:?} clients={:?} remote_chip={:?} chip_age_ms={:?} remote_notices={} pairing={:?} pairing_wait={} view_only={} devices={:?}",
            self.config.remote_access,
            self.control_level(),
            self.win.settings.remote.gateway.text(),
            self.remote.status.as_ref().map(|s| s.pid),
            self.remote
                .status
                .iter()
                .flat_map(|s| s.clients.iter().map(|c| c.name.clone()))
                .collect::<Vec<_>>(),
            self.remote_chip(),
            self.remote.chip_changed.map(|t| t.elapsed().as_millis()),
            self.remote.notices,
            self.win.settings.remote.pairing.as_ref().map(|p| &p.url),
            self.win.settings.remote.pairing_wait,
            self.win.settings.remote.view_only,
            self.win
                .settings
                .remote
                .devices
                .iter()
                .map(|d| format!("{} ({})", d.name, d.detail))
                .collect::<Vec<_>>(),
        );
        let _ = writeln!(
            out,
            "remote_relay={:?} relay_line={:?} relay_reason={:?} relay_open={} relay_edit={:?} clients_via={:?} chip_tip={:?} device_access={:?} history={:?}",
            self.config.remote_relay,
            self.win.settings.remote.relay.text(),
            self.win.settings.remote.relay.reason(),
            self.win.settings.remote.relay_open,
            self.win.settings.remote.relay_edit,
            self.remote
                .status
                .iter()
                .flat_map(|s| s
                    .clients
                    .iter()
                    .map(|c| format!("{}:{}", c.name, c.via.as_str())))
                .collect::<Vec<_>>(),
            self.remote_chip()
                .map(|_| format!("Connected: {}", self.remote_clients_text())),
            self.win
                .settings
                .remote
                .devices
                .iter()
                .map(|d| {
                    format!(
                        "{}={}",
                        d.name,
                        if d.view_only { "view" } else { "operate" }
                    )
                })
                .collect::<Vec<_>>(),
            self.win
                .settings
                .remote
                .history
                .iter()
                .map(|h| format!("{} ({})", h.name, h.detail))
                .collect::<Vec<_>>(),
        );
        let _ = writeln!(
            out,
            "tabs={} active={} editor_active={} any_running={} mouse=({:.0},{:.0}) mouse_cursor={:?}",
            self.win.tabs.len(),
            self.win.active,
            self.win.editor_active,
            self.win.any_running,
            self.win.mouse.0,
            self.win.mouse.1,
            self.win.cursor_icon
        );
        // the tabs drawn last frame (strip or rail), which carry a ×, the
        // rail's width and the unfocused panes washed
        let tab_slots = self
            .win
            .hits
            .iter()
            .filter(|h| matches!(h.target, ClickTarget::TabSelect(_)))
            .count();
        let tab_close: Vec<String> = self
            .win
            .hits
            .iter()
            .filter_map(|h| match h.target {
                ClickTarget::TabClose(i) => Some(i.to_string()),
                ClickTarget::SettingsClose => Some("settings".into()),
                _ => None,
            })
            .collect();
        let _ = writeln!(
            out,
            "tab_slots={tab_slots} tab_close={tab_close:?} rail_w={} dimmed_panes={}",
            self.rail_w(),
            self.win.dimmed_panes
        );
        let _ = writeln!(
            out,
            "font_size={} line_height={} theme={:?} cursor={:?}",
            self.font_size, self.config.line_height, self.config.theme, self.config.cursor_style
        );
        let _ = writeln!(
            out,
            "palette={} mode={:?} query={:?} selected={} results={} loading={}/{}",
            self.win.palette.open,
            self.win.palette.mode,
            self.win.palette.query,
            self.win.palette.selected,
            self.win.palette_results.len(),
            self.win.windows_loading,
            self.win.detached_loading
        );
        let _ = writeln!(
            out,
            "settings={} page={:?} search={} query={:?} matches={} total={} current={} menu={} completion={} ghost={:?} items={} tooltip={:?}",
            self.win.settings.open,
            self.win.settings.page,
            self.win.search.open,
            self.win.search.query,
            self.win.search.matches.len(),
            self.win.search.total,
            self.win.search.current,
            self.win.context_menu.open,
            self.win.completion.open,
            self.win.completion.ghost,
            self.win.completion.items.len(),
            self.win.tip_shown.as_ref().map(|t| format!(
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
        // the menu rows: `insert(source)` (a hint shows its text)
        let _ = writeln!(
            out,
            "completion_items={}",
            self.win
                .completion
                .items
                .iter()
                .take(20)
                .map(|c| if c.source == "hint" {
                    format!("{:?}(hint)", c.description)
                } else {
                    format!("{:?}({})", c.insert, c.source)
                })
                .collect::<Vec<_>>()
                .join(" ")
        );
        // every control with a tooltip: `text@x,y,w,h` (a script hovers one
        // by these, whatever the platform's layout)
        let tips: Vec<String> = self
            .win
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
            self.win.settings.tab,
            self.win.settings.query,
            self.win.settings.dropdown.as_ref().map(|d| d.id),
            self.win.completion.filter,
            self.win.history.open,
            self.win.history.scope,
            self.win.history.results.len(),
            self.win.history.current().map(|e| e.command.as_str()),
        );
        // links/paths as-is; blocks as `block <label> <block id>`
        let hint_targets: Vec<String> = self
            .win
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
            self.win.settings.focus, self.config_error
        );
        let _ = writeln!(
            out,
            "explorer={} focused={} root={:?} rows={} selected={:?} scroll={:.0}",
            self.win.explorer.open,
            self.win.explorer.focused,
            self.win.explorer.root,
            self.win.explorer.rows.len(),
            self.win
                .explorer
                .selected
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned()),
            self.win.explorer.scroll
        );
        let _ = writeln!(
            out,
            "review={} load={} staged={:?} unstaged={:?} expanded={} revealed={} notice={:?} scroll={:.0} shown={} hover_repaints={}",
            self.win.review.open,
            match &self.win.review.load {
                crate::views::review_panel::Load::Idle => "idle",
                crate::views::review_panel::Load::Loading => "loading",
                crate::views::review_panel::Load::NotARepo => "not-a-repo",
                crate::views::review_panel::Load::Failed(_) => "failed",
                crate::views::review_panel::Load::Ready(_) => "ready",
            },
            self.win.review.model().map(|m| m
                .staged
                .iter()
                .map(|f| format!("{}{}", f.status, f.path))
                .collect::<Vec<_>>()),
            self.win.review.model().map(|m| m
                .unstaged
                .iter()
                .map(|f| format!("{}{}", f.status, f.path))
                .collect::<Vec<_>>()),
            self.win.review.expanded.len(),
            self.win.review.revealed.values().sum::<u32>(),
            self.win.review.notice,
            self.win.review.scroll,
            self.panel_widths(self.logical_size().0).1 > 0.0,
            self.win.hover_repaints
        );
        // maximized, sideways scroll per file, the hunk toolbar's buttons
        // and the file list's rows (kind@x,y,w,h)
        let mut scroll_x: Vec<_> = self
            .win
            .review
            .scroll_x
            .iter()
            .filter(|(_, s)| **s > 0.0)
            .map(|((_, p), s)| format!("{p}:{s:.0}"))
            .collect();
        scroll_x.sort();
        let review_hits: Vec<String> = self
            .win
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
            self.win
                .prompt_editor
                .as_ref()
                .map(|e| (&e.draft, e.drag.as_ref().map(|d| (d.chip, d.live)))),
            self.win
                .prompt_editor_layout
                .hits
                .iter()
                .map(|(r, t)| format!("{t:?}@{:.0},{:.0},{:.0},{:.0}", r[0], r[1], r[2], r[3]))
                .collect::<Vec<_>>(),
        );
        // the active pane's prompt chips as drawn (their texts)
        if let Some(tab) = self.win.tabs.get(self.win.active)
            && let Some(pane) = tab.active()
        {
            let cwd = pane
                .session
                .cwd
                .lock()
                .unwrap()
                .as_deref()
                .map(editor::shorten_home)
                .unwrap_or_else(|| "~".into());
            let now = Instant::now();
            let elapsed = tab.elapsed(self.config.elapsed_counts, now);
            let chips: Vec<String> =
                editor::chip_views(&self.config, &self.theme, pane, elapsed, &cwd)
                    .iter()
                    .map(|v| {
                        let t: Vec<&str> = v.texts.iter().map(|(t, _)| t.as_str()).collect();
                        format!("{:?}:{}", v.chip, t.join(" "))
                    })
                    .collect();
            let _ = writeln!(out, "prompt_chips_shown={chips:?}");
            let _ = writeln!(
                out,
                "elapsed_secs={:?} elapsed_counting={:?} elapsed_button={:?} elapsed_tip={:?}",
                elapsed.map(|e| e.time.as_secs()),
                elapsed.map(|e| e.counting),
                self.win.tip_drawn.as_ref().and_then(|d| d.action),
                self.win.tips.iter().find(|t| t.action.is_some()).map(|t| t
                    .body
                    .as_deref()
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or(""))
            );
        }
        // the prompt chips' picker: kind, query, results, selection
        let _ = writeln!(
            out,
            "chip_menu={:?} chip_menu_rows={:?}",
            self.win.chip_menu.as_ref().map(|m| (
                m.kind,
                m.query.clone(),
                m.results().into_iter().map(|i| i.label).collect::<Vec<_>>(),
                m.selected
            )),
            self.win
                .chip_menu_rows
                .iter()
                .map(|(r, i)| format!("{i}@{:.0},{:.0},{:.0},{:.0}", r[0], r[1], r[2], r[3]))
                .collect::<Vec<_>>()
        );
        if let Some(m) = &self.win.chip_menu {
            let _ = writeln!(out, "chip_menu_scroll={:.1}", m.scroll);
        }
        let _ = writeln!(
            out,
            "review_maximized={} scroll_x={scroll_x:?} latent={} review_hits={review_hits:?}",
            self.review_maximized(),
            self.win.review.latent.len(),
        );
        let _ = writeln!(
            out,
            "viewer={:?} lines={} error={:?} scroll={:.0} find={:?}",
            self.win.viewer.path,
            self.win.viewer.lines.len(),
            self.win.viewer.error,
            self.win.viewer.scroll,
            self.win.viewer.find.as_ref().map(|f| f.counter())
        );
        let _ = writeln!(
            out,
            "whats_new={} scroll={:.0}/{:.0}",
            self.win.whats_new.open,
            self.win.whats_new.scroll,
            (self.win.whats_new.content_h - self.win.whats_new.view_h).max(0.0)
        );
        let _ = writeln!(
            out,
            "note_toasts={:?} notifications={} filter={:?} selected={:?} rows={} unread={} scroll={:.0}/{:.0}",
            self.win
                .note_toasts
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            self.win.notifications.open,
            self.win.notifications.filter,
            self.win.notifications.selected,
            self.win.notifications_hits.rows.len(),
            self.unread_attention(),
            self.win.notifications.scroll,
            (self.win.notifications.content_h - self.win.notifications.view_h).max(0.0)
        );
        let _ = writeln!(
            out,
            "update={} chip={:?} dismissed={:?} busy={} version={}",
            match &self.update.status {
                rift_core::update::Status::Idle => "idle".to_string(),
                rift_core::update::Status::Checking => "checking".into(),
                rift_core::update::Status::UpToDate => "up-to-date".into(),
                rift_core::update::Status::Available(m) => format!("available:{}", m.version),
                rift_core::update::Status::Failed(e) => format!("failed:{e:?}"),
            },
            self.update_chip(),
            self.update.dismissed,
            self.update.busy,
            env!("CARGO_PKG_VERSION"),
        );
        // status toasts, newest first: kind, pane, action, text; then where
        // each was drawn (chip, button, ✕)
        let _ = writeln!(
            out,
            "toasts={:?} toast_hits={:?}",
            self.win
                .toasts
                .iter()
                .map(|t| (t.kind, t.pane, t.action, t.text.clone()))
                .collect::<Vec<_>>(),
            self.win
                .toast_hits
                .iter()
                .map(|h| (h.rect, h.action, h.close))
                .collect::<Vec<_>>(),
        );
        let _ = writeln!(
            out,
            "update_ready={:?} update_fetching={:?} app_notes={:?}",
            self.update.ready,
            self.update.fetching,
            self.win
                .app_notes
                .iter()
                .map(|n| format!("{} · {}", n.title, n.detail))
                .collect::<Vec<_>>(),
        );
        let _ = writeln!(
            out,
            "confirm={:?} rename={:?} filter_editing={} toast={:?} shell_env={}",
            self.win
                .confirm
                .as_ref()
                .map(|c| (&c.action, c.commands.clone())),
            self.win.rename,
            self.win.filter_editing,
            self.win.toasts.first().map(|t| t.text.clone()),
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
        if self.win.context_menu.open {
            let labels: Vec<&str> = self
                .win
                .context_menu
                .items
                .iter()
                .map(|i| i.label)
                .collect();
            let _ = writeln!(
                out,
                "menu_items={labels:?} menu_selected={:?}",
                self.win.context_menu.selected
            );
        }
        for (i, tab) in self.win.tabs.iter().enumerate() {
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
            // the active tab's laid-out panes and dividers (logical px)
            if i == self.win.active {
                let (lw, lh) = self.logical_size();
                let (rects, handles) = self.pane_rects(lw, lh);
                for r in &rects {
                    let [x, y, w, h] = r.rect;
                    let _ = writeln!(out, "  pane_rect {} x={x} y={y} w={w} h={h}", r.id);
                }
                for d in &handles {
                    let [x, y, w, h] = d.rect;
                    let _ = writeln!(
                        out,
                        "  divider {:?}/{} x={x} y={y} w={w} h={h}",
                        d.path, d.index
                    );
                }
            }
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
                            rift_core::BlockState::Prompt => "prompt",
                            rift_core::BlockState::Running { .. } => "running",
                            rift_core::BlockState::Done { .. } => "done",
                        },
                        match b.state {
                            rift_core::BlockState::Done { duration, .. } => {
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
        let visible = self.win.window.as_ref().and_then(|w| w.is_visible());
        let _ = writeln!(out, "headless={} window_visible={visible:?}", self.headless);
        out
    }
}

/// Remote access, the window side of the mobile companion
/// (docs/remote-protocol.md › Desktop side): the control level, phones'
/// input, the gateway's start and status, pairing and devices.
impl App {
    /// The control socket's level: `control_api`, raised so the gateway's
    /// requests are served while `remote_access` is on.
    fn control_level(&self) -> crate::control::Level {
        crate::control::Level::effective(&self.config.control_api, self.config.remote_access)
    }

    /// Where a phone's input to `pane` goes right now.
    fn remote_route(pane: &PaneState) -> crate::remote::Route {
        use rift_pty::alacritty_terminal::term::TermMode;
        let alt = pane
            .session
            .term
            .lock()
            .mode()
            .contains(TermMode::ALT_SCREEN);
        let blocks = pane.session.blocks.lock().unwrap();
        crate::remote::route(
            alt,
            blocks.last().map(|b| &b.state),
            blocks.submit_pending(),
        )
    }

    /// `remote-input` (from the gateway): a phone's command, keys or text
    /// for `pane`, accepted only while `remote_access` is on. A command
    /// needs the pane at its prompt in the editor (else `busy`) and is
    /// submitted as if typed there and Enter pressed; keys act as if
    /// pressed on the desktop, except that none builds a command line at
    /// the prompt and Enter there needs the line empty; text needs a
    /// program (else `busy`, also while a submitted command waits for the
    /// shell) and is typed into it. The desktop shows a toast naming the
    /// device (never the text).
    /// A paired phone's size for `pane` (`remote-fit`): held for
    /// `FIT_LEASE` (the gateway sends it again to keep it), `None` releases.
    fn remote_fit(
        &mut self,
        pane: u64,
        cols: Option<u16>,
        rows: Option<u16>,
    ) -> crate::control::Response {
        use crate::control::Response;
        if !self.config.remote_access {
            return Response::error("remote access is off");
        }
        let size = match (cols, rows) {
            (Some(c), Some(r)) => Some((
                c.clamp(*rift_remote::FIT_COLS.start(), *rift_remote::FIT_COLS.end()),
                r.clamp(*rift_remote::FIT_ROWS.start(), *rift_remote::FIT_ROWS.end()),
            )),
            (None, None) => None,
            _ => {
                return Response::error("bad request: cols and rows are both numbers or both null");
            }
        };
        let active = self.win.active;
        let Some(i) = self.win.tabs.iter().position(|t| t.pane(pane).is_some()) else {
            return Response::error("no such pane");
        };
        if let Some(p) = self.win.tabs[i].pane_mut(pane) {
            p.fit = size.map(|(c, r)| (c, r, Instant::now()));
            // layout sizes the panes it draws; one in another tab is sized now
            if i != active {
                p.apply_fit();
            }
        }
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
        Response {
            ok: true,
            pane: Some(pane),
            ..Default::default()
        }
    }

    fn remote_input(
        &mut self,
        pane: u64,
        device: &str,
        command: Option<String>,
        keys: Option<Vec<String>>,
        text: Option<String>,
    ) -> crate::control::Response {
        use crate::control::Response;
        use crate::remote::Route;
        if !self.config.remote_access {
            return Response::error("remote access is off");
        }
        let route = match self.find_tab_of_pane(pane).and_then(|(_, t)| t.pane(pane)) {
            Some(p) => Self::remote_route(p),
            None => return Response::error("no such pane"),
        };
        match (command, keys, text) {
            (Some(cmd), None, None) => {
                // the gateway checked it too: never a line it would not type
                if cmd.trim().is_empty()
                    || cmd.len() > rift_remote::MAX_COMMAND
                    || cmd.chars().any(char::is_control)
                {
                    return Response::error(
                        "bad request: a command is one line of text without control characters",
                    );
                }
                if route != Route::Editor {
                    return Response::error("busy");
                }
                self.remote_submit(pane, &cmd);
                self.toast_phone(Some(pane), crate::remote::ran_toast(device, &cmd));
            }
            (None, Some(names), None) => {
                if names.is_empty() || names.len() > rift_remote::MAX_KEYS {
                    return Response::error("bad request: 1 to 20 keys");
                }
                let Some(keys) = names
                    .iter()
                    .map(|n| rift_remote::Key::parse(n))
                    .collect::<Option<Vec<_>>>()
                else {
                    return Response::error("bad request: unknown key");
                };
                // Enter at the prompt submits only an empty line: a command
                // with text goes through `run` (whole, checked, toasted)
                if route == Route::Editor
                    && keys
                        .iter()
                        .any(|k| input::remote_editor_key(*k) == Some(EditorKey::Submit))
                    && self
                        .find_tab_of_pane(pane)
                        .and_then(|(_, t)| t.pane(pane))
                        .is_some_and(|p| !p.editor.buffer.is_empty())
                {
                    return Response::error(crate::remote::LINE_HAS_TEXT);
                }
                for key in &keys {
                    self.remote_key(pane, *key);
                }
                self.toast_phone(Some(pane), crate::remote::sent_toast(device, &keys));
            }
            (None, None, Some(text)) => {
                // the gateway checked it too: printable, one line, ≤ 1 KiB
                if text.is_empty()
                    || text.len() > rift_remote::MAX_TEXT
                    || text.chars().any(char::is_control)
                {
                    return Response::error(
                        "bad request: text is one line without control characters",
                    );
                }
                // at a prompt a command line goes through `run`; while a
                // submitted command waits for the shell, text would land in
                // the shell's line
                if !route.takes_text() {
                    return Response::error("busy");
                }
                self.remote_text(pane, &text);
                self.toast_phone(Some(pane), crate::remote::typed_toast(device));
            }
            _ => return Response::error("bad request: send one of a command, keys or text"),
        }
        Response {
            ok: true,
            pane: Some(pane),
            ..Default::default()
        }
    }

    /// A phone's `open` next to `pane`: a new tab, or a split to its right,
    /// starting where a new pane from `pane` would. The desktop shows it, as
    /// for `rift-terminal ctl open` / `split`.
    fn remote_open(
        &mut self,
        pane: u64,
        device: &str,
        kind: rift_remote::protocol::OpenKind,
    ) -> crate::control::Response {
        use crate::control::Response;
        use rift_remote::protocol::OpenKind;
        if !self.config.remote_access {
            return Response::error("remote access is off");
        }
        let Some((ti, tab)) = self.find_tab_of_pane(pane) else {
            return Response::error("no such pane");
        };
        let previous = tab.pane(pane).and_then(|p| p.session.local_cwd());
        let cwd = self.config.start_dir(previous.as_deref());
        let id = match kind {
            OpenKind::Tab => {
                self.spawn_tab_at(cwd);
                self.win.tabs[self.win.active].active_pane
            }
            OpenKind::Pane => {
                self.win.settings.open = false;
                self.win.active = ti;
                let id = self.split_pane(pane, Axis::Horizontal, cwd);
                if self.win.tabs[ti].pane(id).is_none() {
                    return Response::error("cannot split this pane");
                }
                self.save_session();
                id
            }
        };
        self.toast_phone(Some(id), crate::remote::opened_toast(device, kind));
        Response {
            ok: true,
            pane: Some(id),
            ..Default::default()
        }
    }

    /// A phone's `close`: `pane`, or its whole tab, ends without the
    /// running-command confirmation (the phone asked its user). Like ⌘W,
    /// closing the window's last tab quits it.
    fn remote_close(
        &mut self,
        pane: u64,
        device: &str,
        kind: rift_remote::protocol::CloseKind,
        event_loop: &ActiveEventLoop,
    ) -> crate::control::Response {
        use crate::control::Response;
        use rift_remote::protocol::CloseKind;
        if !self.config.remote_access {
            return Response::error("remote access is off");
        }
        let Some((ti, _)) = self.find_tab_of_pane(pane) else {
            return Response::error("no such pane");
        };
        let scope = match kind {
            CloseKind::Pane => CloseScope::Pane(pane),
            CloseKind::Tab => CloseScope::Tab(ti),
        };
        // a phone never quits the window: closing its last shell leaves a
        // fresh tab behind (⌘W on the desktop would quit, and a quit ends
        // every shell and the saved layout); with other ordinary windows
        // open, the window closes instead (the drop-down hotkey window does
        // not count: it alone does not keep the app open)
        if self.quits(scope) {
            self.spawn_tab();
        }
        self.toast_phone(None, crate::remote::closed_toast(device, kind));
        self.perform_close(scope, event_loop);
        Response {
            ok: true,
            ..Default::default()
        }
    }

    /// A phone's command for pane `id` at its prompt: the editor's Enter
    /// path without taking the editor's draft (history, prompt-time
    /// context, the exact text for the block, synchronized siblings).
    fn remote_submit(&mut self, id: PaneId, cmd: &str) {
        if let Some(p) = self.find_tab_of_pane(id).and_then(|(_, t)| t.pane_mut(id)) {
            p.correction = None;
        }
        self.submit_to(id, cmd);
        if self.active_pane_id() == Some(id) {
            self.win.completion.close();
        }
        self.submit_synced(id, cmd);
    }

    /// A phone's key for pane `id`, as if pressed on the desktop: an editor
    /// key at a prompt (never one that inserts text: not the desktop's
    /// correction, ghost or completion paths; esc and shift+tab still act
    /// on the active pane's menu), else the bytes the pane's program asked
    /// for (cursor mode, kitty flags), to it and its raw synchronized
    /// siblings.
    fn remote_key(&mut self, id: PaneId, key: rift_remote::Key) {
        use crate::remote::Route;
        let Some(route) = self
            .find_tab_of_pane(id)
            .and_then(|(_, t)| t.pane(id))
            .map(Self::remote_route)
        else {
            return;
        };
        match route {
            Route::Editor => {
                let Some(ek) = input::remote_editor_key(key) else {
                    return;
                };
                let active = self.active_pane_id() == Some(id);
                if active && matches!(ek, EditorKey::Dismiss | EditorKey::NextCompletionSource) {
                    self.handle_editor_key(ek);
                } else {
                    self.editor_key_on(id, ek, active);
                }
            }
            Route::Pending | Route::Raw => {
                let press = input::remote_key_press(key);
                self.remote_press(id, |_| vec![press.clone()]);
            }
        }
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// A phone's text for pane `id` running a program: typed like committed
    /// (non-IME) desktop text, to it and its raw synchronized siblings, each
    /// in its program's keyboard mode ([`input::remote_text_presses`]).
    fn remote_text(&mut self, id: PaneId, text: &str) {
        self.remote_press(id, |enc| input::remote_text_presses(text, enc));
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Write a phone's presses (made for each program's keyboard mode) to
    /// pane `id` and its raw synchronized siblings, each encoded for its own
    /// program (cursor mode, kitty flags; [`input::remote_bytes`]). A
    /// phone's alt is always Meta: it never composes characters like macOS
    /// Option.
    fn remote_press(
        &mut self,
        id: PaneId,
        presses: impl Fn(input::KeyEncoding) -> Vec<(input::KeyPress, ModifiersState)>,
    ) {
        for target in self.raw_targets(id) {
            let Some(p) = self
                .find_tab_of_pane(target)
                .and_then(|(_, t)| t.pane_mut(target))
            else {
                continue;
            };
            let enc = key_encoding(p, true);
            let bytes = input::remote_bytes(&presses(enc), enc);
            if bytes.is_empty() {
                continue;
            }
            p.view_offset = 0;
            if target == id {
                p.clear_selection();
            }
            p.session.write(&bytes);
        }
    }

    /// The tab-bar chip's label while phones are connected.
    fn remote_chip(&self) -> Option<String> {
        let clients = &self.remote.status.as_ref()?.clients;
        match clients.as_slice() {
            [] => None,
            [one] => Some(format!("Remote · {}", one.name)),
            all => Some(format!("Remote · {} devices", all.len())),
        }
    }

    /// The phones connected right now (the quit warning names them).
    fn connected_phones(&self) -> Vec<String> {
        self.remote
            .status
            .iter()
            .flat_map(|s| s.clients.iter())
            .map(|c| c.name.clone())
            .collect()
    }

    /// The connected devices for the chip's tooltip: `Sam's iPhone via
    /// relay, Pixel` (direct ones plain).
    fn remote_clients_text(&self) -> String {
        self.remote
            .status
            .iter()
            .flat_map(|s| s.clients.iter())
            .map(|c| match c.via {
                rift_remote::registry::Via::Relay => format!("{} via relay", c.name),
                rift_remote::registry::Via::Lan => c.name.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Settings › Remote access is on screen.
    fn remote_page_open(&self) -> bool {
        self.win.settings.open && self.win.settings.page == crate::views::settings::Page::Remote
    }

    /// The gateway's files need reading every second: remote access is on,
    /// a gateway runs (its chip), or the page is open.
    fn remote_polling(&self) -> bool {
        self.config.remote_access || self.remote.status.is_some() || self.remote_page_open()
    }

    /// Read the gateway's state (at most once a second unless `force`):
    /// whether one holds the lock and, when `status.json` changed, its
    /// status; start one while remote access is on and none runs (at most
    /// every 5 s; a duplicate exits on the lock); refresh Settings › Remote
    /// access (a pairing that waited for the gateway's addresses, the
    /// devices while the page is open).
    fn poll_remote(&mut self, force: bool) {
        use rift_remote::registry;
        let now = Instant::now();
        if !force
            && self
                .remote
                .polled
                .is_some_and(|t| now.duration_since(t) < REMOTE_POLL)
        {
            return;
        }
        self.remote.polled = Some(now);
        let Some(dir) = crate::remote::registry_dir() else {
            return;
        };
        let chip = self.remote_chip();
        if registry::gateway_running(&dir) {
            let mtime = registry::status_modified(&dir);
            if self.remote.status.is_none() || mtime != self.remote.status_mtime {
                self.remote.status = registry::read_status(&dir);
                self.remote.status_mtime = mtime;
            }
        } else {
            self.remote.status = None;
            self.remote.status_mtime = None;
            if self.config.remote_access
                && self
                    .remote
                    .spawned
                    .is_none_or(|t| now.duration_since(t) >= REMOTE_SPAWN_EVERY)
            {
                self.remote.spawned = Some(now);
                match crate::remote::spawn_managed(&dir) {
                    Ok(()) => log::info!("remote: started the gateway"),
                    Err(e) => log::warn!("remote: cannot start the gateway: {e}"),
                }
            }
        }
        // every window's tab-bar chip and Settings page follow
        self.each_window(|app| app.sync_remote_view(&dir));
        let changed = chip != self.remote_chip();
        if changed {
            self.remote.chip_changed = Some(now);
        }
        self.each_window(|app| {
            if (changed || app.remote_page_open())
                && let Some(w) = &app.win.window
            {
                w.request_redraw();
            }
        });
    }

    /// Settings › Remote access's view of the last poll.
    fn sync_remote_view(&mut self, dir: &std::path::Path) {
        use crate::views::settings::{DeviceRow, GatewayLine};
        let on = self.config.remote_access;
        let gateway = match (&self.remote.status, on) {
            (_, false) => GatewayLine::Off,
            (None, true) => GatewayLine::Starting,
            (Some(s), true) if !s.clients.is_empty() => GatewayLine::Connected(s.clients.len()),
            (Some(s), true) => {
                GatewayLine::Listening(s.addresses.iter().map(|a| a.to_string()).collect())
            }
        };
        self.win.settings.remote.gateway = gateway;
        self.win.settings.remote.addresses = self
            .remote
            .status
            .iter()
            .flat_map(|s| s.addresses.iter().map(|a| a.to_string()))
            .collect();
        self.win.settings.remote.relay = self.relay_line();
        // a gateway that is not running leaves no tunnel to show
        self.win.settings.remote.tunnel = (on && self.config.remote_tunnel)
            .then(|| {
                self.remote
                    .status
                    .as_ref()
                    .and(crate::tunnel::read_state(dir))
            })
            .flatten();
        if !on {
            self.win.settings.remote.pairing = None;
            self.win.settings.remote.pairing_wait = false;
        }
        if self.win.settings.remote.pairing_wait
            && self
                .remote
                .status
                .as_ref()
                .is_some_and(|s| !s.addresses.is_empty())
        {
            self.start_pairing();
        }
        if !self.remote_page_open() {
            return;
        }
        if on && self.config.remote_tunnel {
            self.win.settings.remote.cloudflared = crate::tunnel::find_cloudflared();
            self.win.settings.remote.brew = crate::tunnel::has_brew();
        }
        use rift_remote::registry::Via;
        let connected: Vec<(String, Via)> = self
            .remote
            .status
            .iter()
            .flat_map(|s| s.clients.iter().map(|c| (c.device.clone(), c.via)))
            .collect();
        self.win.settings.remote.history = self.history_rows(dir);
        match rift_remote::registry::devices(dir) {
            Ok(all) => {
                self.win.settings.remote.devices_error = None;
                self.win.settings.remote.devices_loaded = true;
                self.win.settings.remote.devices = all
                    .into_iter()
                    .map(|d| {
                        use crate::views::prompt_chips::LocalTime;
                        let paired = LocalTime::at(d.paired_at).date();
                        let path = match connected.iter().find(|(id, _)| *id == d.id) {
                            Some((_, Via::Relay)) => settings::DevicePath::Relay,
                            Some((_, Via::Lan)) => settings::DevicePath::Lan,
                            None => settings::DevicePath::None,
                        };
                        let seen = if path == settings::DevicePath::None {
                            let t = LocalTime::at(d.last_seen);
                            format!("{} · last seen {} {}", path.text(), t.date(), t.time24())
                        } else {
                            path.text().to_string()
                        };
                        DeviceRow {
                            id: d.id,
                            name: d.name,
                            detail: format!(
                                "{seen} · {} · paired {paired}",
                                settings::platform_name(&d.platform)
                            ),
                            path,
                            view_only: d.permission == rift_remote::registry::Permission::View,
                        }
                    })
                    .collect();
            }
            Err(e) => {
                self.win.settings.remote.devices.clear();
                self.win.settings.remote.devices_error =
                    Some(format!("Could not read devices: {e}"));
            }
        }
    }

    /// Settings › Remote access › "Install" beside cloudflared: Homebrew's
    /// `brew install cloudflared` in a new tab (the gateway picks it up
    /// within seconds), else Cloudflare's download page.
    fn install_cloudflared(&mut self) {
        let brew = crate::tunnel::BREW
            .into_iter()
            .find(|p| std::path::Path::new(p).is_file());
        let Some(brew) = brew else {
            self.open_target(crate::tunnel::DOWNLOAD_URL);
            return;
        };
        self.win.settings.open = false;
        self.spawn_tab_at(None);
        if let Some(pane) = self.win.tabs.get(self.win.active).map(|t| t.active_pane) {
            self.pending_commands
                .insert(pane, vec![format!("{brew} install cloudflared")]);
        }
    }

    /// "Pair a phone": a one-time link for the gateway's addresses (waits
    /// for them when the gateway is still starting).
    fn start_pairing(&mut self) {
        use rift_remote::registry::{self, Permission};
        if !self.config.remote_access {
            return;
        }
        let Some(dir) = crate::remote::registry_dir() else {
            return;
        };
        let addresses = self
            .remote
            .status
            .as_ref()
            .map(|s| s.addresses.clone())
            .unwrap_or_default();
        // the running gateway's relay (the tunnel's while it is on), else
        // the configured one, as `rift-terminal remote pair` has it
        let tunnel = self.config.remote_tunnel;
        let relay = self
            .remote
            .status
            .as_ref()
            .and_then(|s| s.relay.as_ref())
            .map(|r| r.url.clone())
            .or_else(|| self.config.relay().filter(|_| !tunnel).map(str::to_string));
        let view = &mut self.win.settings.remote;
        // a link without the tunnel's address would only work on this network
        if addresses.is_empty() || (tunnel && relay.is_none()) {
            view.pairing = None;
            view.pairing_wait = true;
            return;
        }
        view.pairing_wait = false;
        let permission = if view.view_only {
            Permission::View
        } else {
            Permission::Operate
        };
        match registry::start_pairing_via(
            &dir,
            &addresses,
            relay.as_deref(),
            &crate::remote::hostname(),
            permission,
        ) {
            Ok(link) => {
                view.pairing = Some(crate::views::settings::PairingView {
                    qr: registry::qr_matrix(&link.url),
                    url: link.url,
                    expires_at: link.expires_at,
                });
            }
            Err(e) => {
                view.pairing = None;
                self.toast_error(format!("Could not start pairing: {e}"));
            }
        }
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Revoke (after the confirm dialog): the device's connections close
    /// within a second and its key is refused from then on.
    fn revoke_device(&mut self, id: &str, name: &str) {
        let Some(dir) = crate::remote::registry_dir() else {
            return;
        };
        match rift_remote::registry::revoke(&dir, id) {
            Ok(true) => self.toast_done(format!("Revoked {name}")),
            Ok(false) => self.toast(format!("{name} was already removed")),
            Err(e) => self.toast_error(format!("Could not revoke {name}: {e}")),
        }
        self.poll_remote(true);
    }

    /// Operate / View only (Operate after the confirm dialog): the device's
    /// connections close within a second and reconnect with it.
    fn set_device_permission(&mut self, id: &str, name: &str, view: bool) {
        use rift_remote::registry::{self, Permission};
        let Some(dir) = crate::remote::registry_dir() else {
            return;
        };
        let p = if view {
            Permission::View
        } else {
            Permission::Operate
        };
        match registry::set_permission(&dir, id, p) {
            Ok(true) if view => self.toast_done(format!("{name} can now only watch")),
            Ok(true) => self.toast_done(format!("{name} can now run commands")),
            Ok(false) => self.toast(format!("{name} was removed")),
            Err(e) => self.toast_error(format!("Could not change {name}: {e}")),
        }
        self.poll_remote(true);
    }

    /// End editing the relay field: save it when it is empty (no relay) or
    /// a ws:// / wss:// URL; anything else is dropped with a toast. Returns
    /// whether it was saved.
    fn commit_relay_edit(&mut self) -> bool {
        let Some(text) = self.win.settings.remote.relay_edit.take() else {
            return false;
        };
        let text = text.trim().to_string();
        if !text.is_empty()
            && let Err(e) = rift_remote::relay::RelayUrl::parse(&text)
        {
            self.toast_error(format!("Relay not saved: {e}"));
            return false;
        }
        if text != self.config.remote_relay {
            self.config.remote_relay = text;
            self.save_config();
            self.apply_config();
        }
        self.poll_remote(true);
        true
    }

    /// Keys while the relay field is edited: text, ⌫, ⏎ (save; an invalid
    /// URL stays in the field), Esc (cancel). Other keys end the edit
    /// (saved when valid) and act as usual.
    fn relay_edit_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let Some(draft) = self.win.settings.remote.relay_edit.as_mut() else {
            return false;
        };
        let mods = self.win.modifiers;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.win.settings.remote.relay_edit = None,
            Key::Named(NamedKey::Enter) => {
                let text = draft.trim().to_string();
                match rift_remote::relay::RelayUrl::parse(&text) {
                    Err(e) if !text.is_empty() => {
                        self.toast_error(format!("Relay: {e}"));
                    }
                    _ => {
                        self.commit_relay_edit();
                    }
                }
            }
            Key::Named(NamedKey::Backspace) => {
                if mods.super_key() || mods.alt_key() {
                    draft.clear();
                } else {
                    draft.pop();
                }
            }
            _ if mods.super_key() || mods.control_key() => {
                // ⌘V pastes into the field; other chords end the edit
                let action = input::app_action(event, mods, &self.keymap);
                if action == Some(AppAction::Paste) {
                    self.paste_from_clipboard();
                    return true;
                }
                self.commit_relay_edit();
                return false;
            }
            Key::Named(NamedKey::Tab) => {
                self.commit_relay_edit();
                return false;
            }
            _ => match &event.text {
                Some(t)
                    if !t.is_empty()
                        && !t.chars().any(char::is_control)
                        && draft.len() + t.len() <= MAX_RELAY_URL =>
                {
                    draft.push_str(t);
                }
                _ => {}
            },
        }
        true
    }

    /// Settings › Remote access's relay line from the gateway's status.
    fn relay_line(&self) -> crate::views::settings::RelayLine {
        use crate::views::settings::RelayLine;
        use rift_remote::registry::RelayState;
        if !self.config.remote_access
            || (self.config.relay().is_none() && !self.config.remote_tunnel)
        {
            if self.config.remote_access && !self.config.remote_relay.trim().is_empty() {
                // only the default relay under RIFT_NO_DEFAULT_RELAY
                return RelayLine::Scripted;
            }
            return RelayLine::Off;
        }
        let host = |url: &str| {
            rift_remote::relay::RelayUrl::parse(url)
                .map(|u| u.host)
                .unwrap_or_else(|_| url.to_string())
        };
        match self.remote.status.as_ref().and_then(|s| s.relay.as_ref()) {
            None => RelayLine::Connecting(self.config.relay().map(host).unwrap_or_default()),
            Some(r) => match r.state {
                RelayState::Connected => RelayLine::Connected {
                    host: host(&r.url),
                    latency_ms: r.latency_ms,
                },
                RelayState::Connecting => RelayLine::Connecting(host(&r.url)),
                RelayState::Error
                    if self
                        .remote
                        .retry_shown_until
                        .is_some_and(|t| Instant::now() < t) =>
                {
                    RelayLine::Connecting(host(&r.url))
                }
                RelayState::Error => RelayLine::Error {
                    host: host(&r.url),
                    reason: r.error.clone().unwrap_or_else(|| "Relay error".into()),
                    // whole seconds left, rounded up; none once it is due
                    retry_in: r.retry_at_ms.and_then(|at| {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_millis() as u64);
                        at.checked_sub(now)
                            .filter(|ms| *ms > 0)
                            .map(|ms| ms.div_ceil(1000))
                    }),
                },
            },
        }
    }

    /// Recent connections: the connected devices, then `history.jsonl`,
    /// newest first, at most [`HISTORY_ROWS`].
    ///
    /// [`HISTORY_ROWS`]: crate::views::settings::HISTORY_ROWS
    fn history_rows(&self, dir: &std::path::Path) -> Vec<crate::views::settings::HistoryRow> {
        use crate::views::prompt_chips::LocalTime;
        use crate::views::settings::{HISTORY_ROWS, HistoryRow};
        use rift_remote::registry::{self, Via};
        let via = |v: Via| match v {
            Via::Lan => "LAN",
            Via::Relay => "Relay",
        };
        let when = |t: std::time::SystemTime| {
            let l = LocalTime::at(t);
            format!("{} {}", l.date(), l.time24())
        };
        let mut rows: Vec<HistoryRow> = Vec::new();
        if let Some(st) = &self.remote.status {
            let mut live: Vec<_> = st.clients.iter().collect();
            live.sort_by_key(|c| std::cmp::Reverse(c.since));
            rows.extend(live.into_iter().map(|c| HistoryRow {
                name: c.name.clone(),
                detail: format!("{} · {} · connected", via(c.via), when(c.since)),
            }));
        }
        let all = registry::history(dir);
        for e in all.iter().rev() {
            if rows.len() >= HISTORY_ROWS {
                break;
            }
            let lasted =
                crate::remote::duration_text(e.end.duration_since(e.start).unwrap_or_default());
            let outcome = match e.reason.as_str() {
                "closed" | "stopped" | "forgotten" => lasted,
                "auth" => "refused: not paired".into(),
                "version" => "refused: other protocol version".into(),
                "handshake" => "handshake failed".into(),
                "busy" => "refused: too many connections".into(),
                "revoked" => format!("{lasted} · revoked"),
                "permission" => format!("{lasted} · permission changed"),
                "idle" => format!("{lasted} · timed out"),
                "too_slow" => format!("{lasted} · too slow"),
                "too_large" | "bad_request" => format!("{lasted} · protocol error"),
                _ => format!("{lasted} · connection lost"),
            };
            rows.push(HistoryRow {
                name: e.name.clone().unwrap_or_else(|| "Unknown device".into()),
                detail: format!(
                    "{} · {} · {} · {outcome}",
                    via(e.via),
                    e.address,
                    when(e.start)
                ),
            });
        }
        rows.truncate(HISTORY_ROWS);
        rows
    }
}

/// Updates: the check, the chip and dialog, the install and the restart.
impl App {
    /// Checks run outside scripted runs, and in one only under
    /// `RIFT_UPDATE_URL` (tests never reach the network).
    fn update_checks_allowed(&self) -> bool {
        self.script.is_none() || rift_core::update::url_overridden()
    }

    /// The version the tab-bar chip offers: an available update that was
    /// not dismissed, while nothing is being installed.
    fn update_chip(&self) -> Option<String> {
        match &self.update.status {
            rift_core::update::Status::Available(m)
                if !self.update.busy
                    && self.update.ready.as_deref() == Some(&m.version)
                    && self.update.dismissed.as_deref() != Some(&m.version) =>
            {
                Some(m.version.clone())
            }
            _ => None,
        }
    }

    /// Settings › About's line and the tab-bar chip of every window follow
    /// the status.
    fn sync_update_view(&mut self) {
        self.each_window(|app| app.sync_window_update_view());
    }

    fn sync_window_update_view(&mut self) {
        use crate::views::settings::UpdateLine;
        use rift_core::update::Status;
        self.win.settings.update = match &self.update.status {
            Status::Idle => UpdateLine::None,
            Status::Checking => UpdateLine::Checking,
            Status::UpToDate => UpdateLine::UpToDate,
            Status::Available(m) if self.update.busy => UpdateLine::Updating(m.version.clone()),
            Status::Available(m) if self.update.ready.as_deref() != Some(&m.version) => {
                UpdateLine::Downloading(m.version.clone())
            }
            Status::Available(m) => UpdateLine::Available(m.version.clone()),
            Status::Failed(_) => UpdateLine::Failed,
        };
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Start the periodic check when `check_for_updates` is on (and stop it
    /// when it is turned off). The cached answer shows at once.
    fn sync_update_checks(&mut self) {
        use rift_core::update;
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
            self.update.status = update::evaluate(m, rift_core::changelog::VERSION);
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

    /// "Check for updates" (palette, About): one check now; its answer is
    /// a notice (an update once it is downloaded).
    fn check_for_updates_now(&mut self) {
        use rift_core::update;
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
        result: Result<rift_core::update::Manifest, String>,
        manual: bool,
    ) {
        use rift_core::update::{self, Status};
        let current = rift_core::changelog::VERSION;
        match result {
            Ok(m) => {
                let status = update::evaluate(m, current);
                if manual {
                    match &status {
                        Status::Available(_) => {
                            // asked for: the chip and the notice come back
                            // even if dismissed
                            self.update.dismissed = None;
                            self.update.notified = None;
                            update::update_cache(|c| c.dismissed = None);
                        }
                        _ => {
                            self.app_note(
                                "Rift is up to date".into(),
                                format!("v{current} is the newest version"),
                                AppNoteAction::None,
                                false,
                            );
                        }
                    }
                }
                // this is the newest: a download left from before (updated
                // another way) is not needed
                if matches!(status, Status::UpToDate)
                    && let Some(dir) = update::updates_dir()
                {
                    update::drop_ready(&dir);
                }
                self.update.status = status;
                self.prefetch_update();
            }
            Err(e) => {
                log::info!("update check: {e}");
                if manual {
                    // curl's own wording stays in the log
                    let why = if e.starts_with("malformed manifest")
                        || e == update::SIG_MISSING
                        || e == update::SIG_INVALID
                    {
                        "The release information is invalid"
                    } else if e == "curl is not installed" {
                        "curl is not installed"
                    } else {
                        "Check your connection and try again"
                    };
                    self.app_note(
                        "Couldn't check for updates".into(),
                        why.into(),
                        AppNoteAction::None,
                        false,
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

    /// An available update is downloaded before it is offered: start that
    /// download (once per version) unless it is here already. A copy that
    /// cannot update itself has nothing to download: offered at once.
    fn prefetch_update(&mut self) {
        use rift_core::update::{self, InstallTarget, Status};
        let Status::Available(m) = &self.update.status else {
            return;
        };
        let version = m.version.clone();
        if self.update.ready.as_deref() == Some(&version) {
            // here already: a check asked for shows its notice again
            return self.notify_update_ready();
        }
        if self.update.busy || self.update.fetching.as_deref() == Some(&version) {
            return;
        }
        let asset = update::asset_for(m, &update::platform_key()).cloned();
        let target = update::install_target();
        let (Some(asset), Some(dir), false) = (
            asset,
            update::updates_dir(),
            matches!(target, InstallTarget::Unsupported(_)),
        ) else {
            return self.on_update_downloaded(version, Ok(false));
        };
        self.update.fetching = Some(version.clone());
        let proxy = self.proxy.clone();
        let started = std::thread::Builder::new()
            .name("update-prefetch".into())
            .spawn(move || {
                let result = update::prefetch(&dir, &asset).map(|_| true);
                let _ = proxy.send_event(UserEvent::UpdateDownloaded(version, result));
            });
        if started.is_err() {
            let version = self.update.fetching.take().unwrap_or_default();
            self.on_update_downloaded(version, Err("could not start the download".into()));
        }
    }

    /// The update is here (or could not be fetched ahead: the install
    /// downloads it then): offer it — the chip, About's button and, once
    /// per version, a notice.
    fn on_update_downloaded(&mut self, version: String, result: Result<bool, String>) {
        if self.update.fetching.as_deref() == Some(&version) {
            self.update.fetching = None;
        }
        if let Err(e) = &result {
            log::info!("update: downloading v{version} ahead: {e}");
        }
        let current = matches!(
            &self.update.status,
            rift_core::update::Status::Available(m) if m.version == version
        );
        if !current {
            return;
        }
        self.update.ready = Some(version);
        self.update.ready_detail = match result {
            Ok(true) => "Downloaded · click to restart into it",
            // the dialog says how (the release page)
            Ok(false) => "Click to see how to update",
            Err(_) => "Click to update",
        };
        self.notify_update_ready();
        self.sync_update_view();
    }

    /// The "ready" notice for the ready version, once per version (a check
    /// asked for clears `notified`), unless it was dismissed or installs.
    fn notify_update_ready(&mut self) {
        let Some(version) = self.update.ready.clone() else {
            return;
        };
        let dismissed = self.update.dismissed.as_deref() == Some(&version);
        if self.update.busy || dismissed || self.update.notified.as_deref() == Some(&version) {
            return;
        }
        self.update.notified = Some(version.clone());
        self.drop_app_notes(AppNoteAction::Update);
        self.app_note(
            format!("Rift v{version} is ready"),
            self.update.ready_detail.into(),
            AppNoteAction::Update,
            true,
        );
    }

    /// Chip, About's button, `update_rift`: the dialog for an available
    /// update (after finding the other windows), else a check.
    fn update_rift(&mut self) {
        if self.update.busy {
            self.app_note(
                "Rift is already updating".into(),
                String::new(),
                AppNoteAction::None,
                false,
            );
            return;
        }
        if !matches!(self.update.status, rift_core::update::Status::Available(_)) {
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
        manifest: rift_core::update::Manifest,
        survey: crate::updater::Survey,
    ) -> UpdatePlan {
        use rift_core::update::{self, InstallTarget};
        let mut target = update::install_target();
        let key = update::platform_key();
        if !matches!(target, InstallTarget::Unsupported(_))
            && update::asset_for(&manifest, &key).is_none()
        {
            target = InstallTarget::Unsupported(format!("this release has no download for {key}"));
        }
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        let panes: Vec<&PaneState> = self.all_panes().collect();
        let tabs = self.all_tabs().count() + survey.tabs();
        let shells = panes.len() + survey.shells();
        let durable = panes
            .iter()
            .filter(|p| p.session.service_id.is_some())
            .count()
            + survey.durable_shells();
        let compatible = update::protocol_compatible(&manifest, rift_pty::ipc::PROTOCOL_VERSION);
        let tabs_text = plural(tabs, "tab");
        let (description, mut note, open_page) = match &target {
            InstallTarget::Unsupported(why) if why == update::MOVE_TO_APPLICATIONS => (
                format!("{why}."),
                "It runs from the disk image or a temporary copy macOS made. Drag Rift into Applications, open it from there, then update.".to_string(),
                true,
            ),
            InstallTarget::Unsupported(why) => (
                format!("This copy of Rift can't update itself: {why}."),
                "Download the new version from the release page.".to_string(),
                true,
            ),
            _ if durable > 0 && !compatible => (
                format!("Rift restarts and reopens your {tabs_text}."),
                "This update can't reattach running shells; they keep running in the background but won't reopen.".to_string(),
                false,
            ),
            _ if durable > 0 && durable == shells => (
                format!(
                    "Rift restarts. Your {tabs_text} and {} keep running and come back where they were.",
                    plural(shells, "shell")
                ),
                String::new(),
                false,
            ),
            _ if durable > 0 => (
                format!(
                    "Rift restarts and reopens your {tabs_text}. {durable} of {} keep running; the others end.",
                    plural(shells, "shell")
                ),
                String::new(),
                false,
            ),
            _ => (
                format!("Rift restarts and reopens your {tabs_text}."),
                "Open shells will end when Rift restarts. Turn on “Keep shells running through updates” in Settings › General to keep them.".to_string(),
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
        let rift_core::update::Status::Available(m) = self.update.status.clone() else {
            return;
        };
        if self.update.busy || self.win.confirm.is_some() {
            return;
        }
        let plan = self.update_plan(m, survey);
        self.win.palette.open = false;
        self.win.context_menu.close();
        self.win.confirm = Some(ConfirmClose {
            opened: Instant::now(),
            action: ConfirmAction::Update(Box::new(plan)),
            commands: Vec::new(),
        });
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Cancel / Esc on the confirm modal; "Later" on the update dialog
    /// dismisses that version (no chip until a newer one).
    fn cancel_confirm(&mut self) {
        if let Some(ConfirmClose {
            action: ConfirmAction::Update(plan),
            ..
        }) = self.win.confirm.take()
        {
            let version = plan.manifest.version.clone();
            self.update.dismissed = Some(version.clone());
            if self.update_checks_allowed() {
                rift_core::update::update_cache(|c| c.dismissed = Some(version));
            }
        }
    }

    /// Update: take the install lock (one window at a time), then download,
    /// verify and install off the UI thread; Open download page: the
    /// release page in the browser.
    fn install_update(&mut self, plan: UpdatePlan) {
        use rift_core::update;
        if plan.open_page {
            let url = if plan.manifest.url.starts_with("https://") {
                plan.manifest.url.clone()
            } else {
                RELEASES_PAGE.to_string()
            };
            self.open_target_external(&url);
            return;
        }
        let failed = |app: &mut Self, why: String| {
            app.app_note("Update failed".into(), why, AppNoteAction::None, false);
        };
        let Some(dir) = update::updates_dir() else {
            return failed(self, "no data directory".into());
        };
        match update::try_lock(&dir) {
            Ok(lock) => self.update.lock = Some(lock),
            Err(update::LockError::Held(pid)) => {
                log::info!("update: install lock held by {pid}");
                self.app_note(
                    "Another window is updating Rift".into(),
                    String::new(),
                    AppNoteAction::None,
                    false,
                );
                return;
            }
            Err(update::LockError::Io(e)) => return failed(self, e),
        }
        self.update.busy = true;
        self.update.quit_after = false;
        self.drop_app_notes(AppNoteAction::Update);
        let version = plan.manifest.version.clone();
        let ready = update::asset_for(&plan.manifest, &update::platform_key())
            .and_then(|a| update::ready_for(&dir, a));
        if ready.is_none() {
            // not fetched ahead (it failed, or is still under way): the
            // install downloads it now
            self.app_note(
                format!("Downloading Rift v{version}…"),
                "Rift restarts when it is installed".into(),
                AppNoteAction::None,
                false,
            );
        }
        self.sync_update_view();
        let proxy = self.proxy.clone();
        let spawned = std::thread::Builder::new()
            .name("update-install".into())
            .spawn(move || {
                let result = (|| {
                    let asset = update::asset_for(&plan.manifest, &update::platform_key())
                        .ok_or("this release has no download for this computer")?;
                    // what an interrupted install left (the lock is held)
                    update::sweep(&dir, &plan.target);
                    // downloaded ahead (verified again here) and moved to
                    // the download's own name, so what an interrupted
                    // install leaves (its mount point) is swept like any;
                    // else downloaded now. A fixed name: nothing from the
                    // manifest reaches a path
                    let file = update::download_path(&dir, asset);
                    let taken = update::ready_for(&dir, asset)
                        .is_some_and(|ready| std::fs::rename(ready, &file).is_ok());
                    if !taken {
                        update::download(asset, &file)?;
                    }
                    let installed = update::install(
                        &plan.target,
                        &file,
                        &version,
                        rift_core::changelog::VERSION,
                    );
                    let _ = std::fs::remove_file(&file);
                    installed
                })();
                let _ = proxy.send_event(UserEvent::UpdateInstalled(result, plan.windows));
            });
        if spawned.is_err() {
            self.update.busy = false;
            self.update.lock = None;
            failed(self, "could not start the update".into());
            self.sync_update_view();
        }
    }

    /// The update stopped (failed, or undone before the restart): release
    /// the lock, say why, and quit now if a quit waited for it.
    fn abort_update(&mut self, message: String, event_loop: &ActiveEventLoop) {
        self.update.busy = false;
        self.update.lock = None;
        // "Update failed: why" reads as a title and its detail
        let (title, detail) = match message.split_once(": ") {
            Some(("Update failed", why)) => ("Update failed".to_string(), why.to_string()),
            _ => (message, String::new()),
        };
        self.app_note(title, detail, AppNoteAction::None, true);
        self.sync_update_view();
        if std::mem::take(&mut self.update.quit_after) {
            self.perform_close(CloseScope::App, event_loop);
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
        result: Result<rift_core::update::Pending, String>,
        windows: Vec<(u32, std::path::PathBuf)>,
        event_loop: &ActiveEventLoop,
    ) {
        use rift_core::update;
        let mut pending = match result {
            Ok(p) => p,
            Err(e) => return self.abort_update(format!("Update failed: {e}"), event_loop),
        };
        let to = pending.to.clone();
        // nothing will start the new version: the old one goes back
        let undo = |pending: &update::Pending, why: String| match update::restore_old(pending) {
            Ok(()) => format!("Couldn't restart into Rift v{to} ({why}); nothing was changed"),
            Err(e) => {
                log::warn!("update: restoring the previous version: {e}");
                format!("Couldn't restart into Rift v{to} ({why})")
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
            self.perform_close(CloseScope::App, event_loop);
            return;
        }
        // 2. this window's layout
        let own = crate::updater::own_restart_file(self.persist_session)
            .ok_or_else(|| "no data directory".to_string())
            .and_then(|f| {
                rift_core::session_state::save_to(&f, &self.saved_session())
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
            self.end_hotkey_window_shells();
            crate::quit::exit(event_loop);
        }
    }

    /// Start the detached helper that launches the new version (restoring
    /// this window from `own`, reopening the others) once this process is
    /// gone, and rolls back if it dies before confirming.
    fn spawn_update_helper(
        &self,
        pending: &rift_core::update::Pending,
        own: &std::path::Path,
    ) -> Result<(), String> {
        use rift_core::update;
        let exe = if pending.new_path.extension().is_some_and(|e| e == "app") {
            pending.new_path.join("Contents/MacOS/rift-terminal")
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
        if let Err(e) = rift_core::session_state::save_to(&own, &self.saved_session()) {
            log::warn!("restart file {}: {e}", own.display());
        }
        self.save_session();
        self.end_hotkey_window_shells();
        crate::quit::exit(event_loop);
    }

    /// At start: what waits for the first frame (`confirm_start`), and how
    /// long after it — [`rift_core::update::CONFIRM_AFTER`] when this is the
    /// version an update just installed, so a crash early on still rolls
    /// back (the other windows reopen only then: a rollback reopens them on
    /// the old version instead).
    fn plan_start_confirmation(&mut self) {
        use rift_core::update;
        let fresh_install = self.handles_update_marker()
            && update::pending_path()
                .and_then(|m| update::read_pending(&m))
                .is_some_and(|p| update::same_version(&p.to, rift_core::changelog::VERSION));
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
        // RIFT_TEST_NO_FIRST_FRAME (tests): a headless run that waits for
        // the fallback like a window that never draws
        if self.headless && std::env::var_os("RIFT_TEST_NO_FIRST_FRAME").is_none() {
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
        // RIFT_PERSIST_SESSION, so it counts as a plain launch then
        self.persist_session && !launch.relaunched && launch.restore.is_none()
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
        use rift_core::update;
        if std::mem::replace(&mut self.update.start_confirmed, true) {
            return;
        }
        self.update.confirm_after = None;
        let launch = crate::updater::launch();
        let current = rift_core::changelog::VERSION;
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
                self.app_note(
                    format!("Updated to Rift v{current}"),
                    "See what's new".into(),
                    AppNoteAction::WhatsNew,
                    false,
                );
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
                    self.app_note(
                        format!("Couldn't update to Rift v{}", p.to),
                        format!("Still on v{current}"),
                        AppNoteAction::None,
                        true,
                    );
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
            self.app_note(
                format!("Couldn't update to Rift v{}", p.to),
                format!("Still on v{current}"),
                AppNoteAction::None,
                true,
            );
        }
        if let Some(file) = &launch.restore {
            // validated in take_env: a restart file
            let _ = std::fs::remove_file(file);
        }
        // the other processes' windows open here, at the next turn of the
        // event loop (`open_queued_windows`)
        if launch.restart_others {
            let windows = crate::updater::restart_windows(launch.restore.as_deref());
            log::info!("update: reopening {} window(s)", windows.len());
            self.queued_windows.extend(windows);
        } else if self.plain_main_launch() && !marker_left {
            // windows a crashed update saved and never reopened
            let windows = crate::updater::restart_windows(None);
            if !windows.is_empty() {
                log::info!(
                    "reopening {} window(s) left by an interrupted update",
                    windows.len()
                );
            }
            self.queued_windows.extend(windows);
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

impl App {
    /// The window's paced output frame, tooltip and selection edge scroll:
    /// each repaints when due; returns when they are next due.
    fn frame_deadlines(&mut self) -> (Option<Instant>, Option<Instant>, Option<Instant>) {
        // a paced output frame whose time has come
        let output_frame = self.win.output_frame_due;
        if output_frame.is_some_and(|due| Instant::now() >= due) {
            self.win.output_frame_due = None;
            if let Some(w) = &self.win.window {
                w.request_redraw();
            }
        }
        // a tooltip whose rest time is up
        let tip_due = self.tooltip_due();
        if tip_due.is_some_and(|due| Instant::now() >= due)
            && let Some(w) = &self.win.window
        {
            w.request_redraw();
        }
        let tip_due = tip_due.filter(|due| Instant::now() < *due);
        let output_frame = self.win.output_frame_due;
        let select_due = self.tick_select_scroll();
        (output_frame, tip_due, select_due)
    }

    /// The window's timed repaints (boot grace, toasts, cursor blink): a
    /// repaint when one is due, and when the others are.
    fn window_deadlines(&mut self, now: Instant) -> Vec<Instant> {
        let mut repaint = false;
        let mut deadlines: Vec<Instant> = Vec::new();
        // boot placeholder grace ends (shell without a prompt marker)
        if let Some(until) = self.win.boot_until {
            if now >= until {
                self.win.boot_until = None;
                repaint = true;
            } else {
                deadlines.push(until);
            }
        }
        // a phone's pane size whose lease ran out: the shell goes back to the
        // window's own size
        for tab in &mut self.win.tabs {
            for p in &mut tab.panes {
                let Some((_, _, at)) = p.fit else { continue };
                let due = at + rift_remote::FIT_LEASE;
                if now >= due {
                    p.fit = None;
                    p.apply_fit();
                    repaint = true;
                } else {
                    deadlines.push(due);
                }
            }
        }
        // the Elapsed chip: a repaint when its minutes change
        let minutes = self
            .win
            .tabs
            .get(self.win.active)
            .filter(|_| {
                self.config
                    .prompt_chips
                    .contains(&rift_core::PromptChip::Elapsed)
            })
            .and_then(|t| t.elapsed(self.config.elapsed_counts, now));
        if self.win.elapsed_minutes != minutes.map(|e| e.time.as_secs() / 60) {
            self.win.elapsed_minutes = minutes.map(|e| e.time.as_secs() / 60);
            repaint = true;
        }
        if let Some(e) = minutes.filter(|e| e.counting) {
            deadlines.push(now + Duration::from_secs(60 - e.time.as_secs() % 60));
        }
        // a copy button's check goes back to the copy icon
        if let Some((_, _, at)) = self.win.copied {
            let until = at + COPIED_SHOWN;
            if now >= until {
                self.win.copied = None;
                repaint = true;
            } else {
                deadlines.push(until);
            }
        }
        // status toasts; the one under the pointer stays
        let (mx, my) = self.win.mouse;
        let hovered: Vec<u64> = self
            .win
            .toast_hits
            .iter()
            .filter(|h| {
                mx >= h.rect[0]
                    && mx < h.rect[0] + h.rect[2]
                    && my >= h.rect[1]
                    && my < h.rect[1] + h.rect[3]
            })
            .map(|h| h.id)
            .collect();
        let before = self.win.toasts.len();
        for t in &mut self.win.toasts {
            if hovered.contains(&t.id) {
                t.at = now;
            }
        }
        self.win
            .toasts
            .retain(|t| t.ttl().is_none_or(|ttl| now < t.at + ttl));
        if self.win.toasts.len() != before {
            repaint = true;
        }
        for t in &self.win.toasts {
            if let Some(ttl) = t.ttl() {
                deadlines.push(t.at + ttl);
            }
        }
        // a dragged pane held still over a tab opens it after TAB_HOVER_OPEN
        if let Some((_, since)) = self
            .win
            .pane_drag
            .as_ref()
            .and_then(|d| d.hover_tab)
            .filter(|(j, _)| *j != self.win.active)
        {
            let due = since + TAB_HOVER_OPEN;
            if now >= due {
                self.track_pane_drag();
                repaint = true;
            } else {
                deadlines.push(due);
            }
        }
        // Settings: the "config.toml updated" check goes after SAVED_SHOWN
        if let Some(at) = self
            .win
            .settings
            .saved_at
            .filter(|_| self.win.settings.open)
        {
            let until = at + settings::SAVED_SHOWN;
            if now >= until {
                self.win.settings.saved_at = None;
                repaint = true;
            } else {
                deadlines.push(until);
            }
        }
        // notification toasts (a hovered one stays)
        repaint |= self.prune_note_toasts();
        let ttl = self.config.notifications.toast_duration();
        deadlines.extend(self.win.note_toasts.iter().map(|(_, since)| *since + ttl));
        if (self.win.editor_active || self.win.palette.open || self.win.rename.is_some())
            && !self.win.any_running
        {
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
        if repaint && let Some(w) = &self.win.window {
            w.request_redraw();
        }
        deadlines
    }
}

/// The earlier of two optional deadlines.
fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.win.window.is_some() {
            return;
        }

        self.load_config(true);
        // an update's restart restores from its own file, whatever
        // restore_session says (the dialog promised the tabs come back)
        let restored = if let Some(file) = &crate::updater::launch().restore {
            rift_core::session_state::load_from(file)
        } else if self.config.restore_session && self.persist_session {
            rift_core::session_state::load()
        } else {
            None
        };
        // last window frame (only when it is still a sane size)
        let frame = restored
            .as_ref()
            .and_then(|s| s.window)
            .filter(|f| f.width >= 320.0 && f.height >= 200.0);
        self.create_window(event_loop, frame);

        let git_proxy = self.proxy.clone();
        self.git = Some(GitWorker::new(Arc::new(move |pane, info| {
            let _ = git_proxy.send_event(UserEvent::GitStatus(pane, info));
        })));
        let pr_proxy = self.proxy.clone();
        self.pr_worker = Some(rift_core::github::PrWorker::new(Arc::new(
            move |pane, info| {
                let _ = pr_proxy.send_event(UserEvent::PrStatus(pane, info));
            },
        )));
        let docker_proxy = self.proxy.clone();
        self.docker_worker = Some(rift_core::docker::DockerWorker::new(Arc::new(
            move |pane, info| {
                let _ = docker_proxy.send_event(UserEvent::DockerStatus(pane, info));
            },
        )));

        let watch_proxy = self.proxy.clone();
        self._watcher = rift_core::watch::watch(Arc::new(move || {
            let _ = watch_proxy.send_event(UserEvent::ConfigReloaded);
        }));

        #[cfg(target_os = "macos")]
        {
            let menu_proxy = self.proxy.clone();
            muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| {
                let _ = menu_proxy.send_event(UserEvent::Menu(e.id.0));
            }));
            // clicks on Rift's desktop notifications come back to the pane
            let notify_proxy = std::sync::Mutex::new(self.proxy.clone());
            crate::notify::set_response_handler(move |r| {
                if let Ok(p) = notify_proxy.lock() {
                    let _ = p.send_event(UserEvent::NotificationResponse(r));
                }
            });
            self.refresh_notify_permission();
            // the Dock's Quit and AppleScript's quit take the app's quit
            let quit_proxy = std::sync::Mutex::new(self.proxy.clone());
            crate::quit::route_app_quit(move || {
                if let Ok(p) = quit_proxy.lock() {
                    let _ = p.send_event(UserEvent::QuitRequested);
                }
            });
        }

        self.sync_control_server();

        self.apply_config();
        self.launch_configs = rift_core::launch::load_all();

        let hist_proxy = self.proxy.clone();
        let shell = rift_core::shell_integration::resolve_shell(self.config.shell.as_deref());
        std::thread::Builder::new()
            .name("history-import".into())
            .spawn(move || {
                let h = rift_core::history::load_shell_history(&shell, 5000);
                let _ = hist_proxy.send_event(UserEvent::HistoryLoaded(h));
            })
            .ok();

        // shells a phone started while Rift was closed (background.rs)
        let adopted = if self.config.durable_sessions && self.durable_allowed {
            crate::background::adopt()
        } else {
            Vec::new()
        };
        match restored {
            Some(saved) if !saved.tabs.is_empty() => {
                // restoring must not rewrite the file tab by tab
                let persist = std::mem::replace(&mut self.persist_session, false);
                for tab in &saved.tabs {
                    self.restore_tab(tab);
                }
                self.win.active = saved.active.min(self.win.tabs.len() - 1);
                // the other windows, in their order; the first is served
                let first = self.win_id();
                for other in &saved.other_windows {
                    self.open_saved_window(event_loop, other);
                }
                if let Some(id) = first {
                    self.enter_window(id);
                    self.note_window_focus(id);
                    if self.window_count() > 1
                        && let Some(w) = self.shown_window()
                    {
                        w.focus_window();
                    }
                }
                self.persist_session = persist;
                self.save_session();
            }
            _ if !adopted.is_empty() => {}
            _ => self.spawn_tab(),
        }
        if !adopted.is_empty() {
            for &id in &adopted {
                self.next_attach = Some(id);
                self.spawn_tab_at(None);
            }
            self.toast(match adopted.len() {
                1 => "A tab your phone opened while Rift was closed".into(),
                n => format!("{n} tabs your phone opened while Rift was closed"),
            });
            self.save_session();
        }
        self.load_shell_env();
        self.plan_start_confirmation();
        self.sync_update_checks();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // a window closed a moment ago may still have events queued
        if !self.enter_window(window_id) {
            return;
        }
        let Some(window) = self.win.window.clone() else {
            return;
        };
        if let Some(a11y) = &mut self.win.a11y {
            a11y.process_event(&window, &event);
        }

        match event {
            WindowEvent::CloseRequested => {
                self.request_close(CloseScope::Window, event_loop);
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.win.gpu {
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
                self.win.focused = f;
                if f {
                    self.note_window_focus(window_id);
                }
                // the button's release goes to whatever took focus: a drag
                // in progress ends here, keeping what it selected, or it
                // would follow (and edge-scroll) the bare pointer
                self.hotkey_window_focus(f);
                if !f {
                    self.win.pending_sel = None;
                } else if self.win.settings.open {
                    // back from System Settings: show what was chosen there
                    self.refresh_notify_permission();
                }
                self.sync_focus();
                // AppKit may lay the titlebar out again on key-state changes
                #[cfg(target_os = "macos")]
                place_traffic_lights(&window);
                window.request_redraw();
            }
            WindowEvent::Ime(ime) => match ime {
                winit::event::Ime::Preedit(text, _) => {
                    self.win.preedit = text;
                    window.request_redraw();
                }
                winit::event::Ime::Commit(text) => self.commit_text(&text, event_loop, &window),
                winit::event::Ime::Enabled | winit::event::Ime::Disabled => {
                    self.win.preedit.clear();
                    window.request_redraw();
                }
            },
            WindowEvent::DroppedFile(path) => {
                self.drop_file(&path);
                window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => {
                self.win.modifiers = m.state();
                use winit::keyboard::ModifiersKeyState::Pressed;
                self.win.alt_sides = (m.lalt_state() == Pressed, m.ralt_state() == Pressed);
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
                if !self.win.preedit.is_empty() {
                    return;
                }
                self.on_key(&press, event_loop, &window);
            }
            WindowEvent::CursorLeft { .. } => {
                self.win.pointer_in = false;
                if self.win.review.pointer.take().is_some()
                    && !self.win.review.hover_rows.is_empty()
                {
                    window.request_redraw();
                }
                if self.win.tip_shown.is_some() {
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
                if self.win.any_running {
                    // the running spinner animates; paced so back-to-back
                    // frames never starve the reader thread of the terminal
                    self.request_output_frame();
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, mut event: UserEvent) {
        self.route_user_event(&mut event);
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
                let active_idx = self.win.active;
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
                        if let Some(w) = &self.win.window {
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
                if let Some(w) = &self.win.window {
                    w.request_redraw();
                }
            }
            UserEvent::PtyBell(pane) => {
                self.on_bell(pane);
            }
            UserEvent::GlobalHotkey => self.on_global_hotkey(event_loop),
            #[cfg(target_os = "macos")]
            UserEvent::Menu(id) => self.run_menu_command(&id, event_loop),
            #[cfg(not(target_os = "macos"))]
            UserEvent::Menu(_) => {}
            UserEvent::NotificationResponse(r) => self.on_notification_response(r),
            UserEvent::NotifyPermission => {
                if let Some(w) = &self.win.window {
                    w.request_redraw();
                }
            }
            UserEvent::QuitRequested => {
                // the window may be hidden (global hotkey): the dialog must show
                if let Some(w) = &self.win.window
                    && !self.headless
                {
                    w.set_visible(true);
                    w.focus_window();
                }
                self.request_close(CloseScope::App, event_loop);
            }
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
                            .write_report((reply.0)(&text.unwrap_or_default()).as_bytes());
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
                    let rgb = rift_pty::alacritty_terminal::vte::ansi::Rgb {
                        r: to8(c.r),
                        g: to8(c.g),
                        b: to8(c.b),
                    };
                    p.session.write_report((reply.0)(rgb).as_bytes());
                }
            }
            UserEvent::PtyWindowOp(pane, params) => self.window_op(pane, &params),
            UserEvent::PtyExit(pane) => {
                self.close_pane(pane, event_loop);
                if let Some(w) = &self.win.window {
                    w.request_redraw();
                }
            }
            UserEvent::ShellEnvLoaded(env) => {
                self.shell_env = Some(env);
                self.each_window(|app| {
                    if let Some(w) = &app.win.window {
                        w.request_redraw();
                    }
                });
            }
            UserEvent::Access(e) => self.on_access_event(e.window_event, event_loop),
            UserEvent::PtyNotify(pane, title, body) => {
                self.on_program_notification(pane, title, body);
                if let Some(w) = &self.win.window {
                    w.request_redraw();
                }
            }
            UserEvent::PtyNotice(message) => self.toast(message),
            UserEvent::Completions(generation, items) => self.on_completions(generation, items),
            UserEvent::CompletionsPartial(generation, items) => {
                self.on_partial_completions(generation, items)
            }
            UserEvent::SshCommand(pane, command) => {
                // the tab may have been closed meanwhile
                if self.enter_pane_window(pane) {
                    self.pending_commands.insert(pane, vec![command]);
                    // sent now if its prompt is already up, else at its
                    // first prompt (`PtyWakeup`)
                    self.flush_pending_commands(pane);
                }
            }
            UserEvent::ReviewWritten(error) => {
                self.win.review.notice = error;
                self.win.explorer.refresh();
                self.request_review();
                if let Some(w) = &self.win.window {
                    w.request_redraw();
                }
            }
            UserEvent::ReviewLoaded(generation, result) => {
                use crate::views::review_panel::Load;
                if generation != self.win.review.generation {
                    return; // a newer request is in flight
                }
                self.win.review.load = match result {
                    None => Load::NotARepo,
                    Some(Err(e)) => Load::Failed(e),
                    Some(Ok(r)) => {
                        self.win.explorer.set_statuses(r.statuses(), r.dirty_dirs());
                        Load::Ready(Arc::new(r))
                    }
                };
                if !matches!(self.win.review.load, Load::Ready(_)) {
                    self.win
                        .explorer
                        .set_statuses(Default::default(), Default::default());
                }
                // listings may have changed with the files
                self.win.explorer.refresh();
                if let Some(w) = &self.win.window {
                    w.request_redraw();
                }
            }
            UserEvent::Control(p)
                if matches!(p.request, crate::control::Request::Complete { .. })
                    && p.allowed(self.control_level()) =>
            {
                self.complete_remote(p);
            }
            UserEvent::Control(p)
                if matches!(
                    p.request,
                    crate::control::Request::Menu { .. } | crate::control::Request::Diff { .. }
                ) && p.allowed(self.control_level()) =>
            {
                self.chip_remote(p);
            }
            UserEvent::Control(p)
                if matches!(p.request, crate::control::Request::RemoteClose { .. })
                    && p.allowed(self.control_level()) =>
            {
                let crate::control::Request::RemoteClose { pane, device, kind } = p.request else {
                    return;
                };
                let response = self.remote_close(pane, &device, kind, event_loop);
                let _ = p.reply.send(response);
                if let Some(w) = &self.win.window {
                    w.request_redraw();
                }
            }
            UserEvent::Control(p) => {
                let response = if p.allowed(self.control_level()) {
                    self.handle_control(p.request)
                } else {
                    crate::control::Response::error("not allowed: control permissions changed")
                };
                let _ = p.reply.send(response);
                if self.restart_requested {
                    // an updating window restarts this one: its layout is
                    // saved; durable shells keep running for the new version
                    log::info!("restarting for an update");
                    self.end_hotkey_window_shells();
                    crate::quit::exit(event_loop);
                    return;
                }
                if let Some(w) = &self.win.window {
                    w.request_redraw();
                }
            }
            UserEvent::UpdateChecked(result, manual) => self.on_update_checked(result, manual),
            UserEvent::UpdateSurvey(survey) => self.open_update_dialog(survey),
            UserEvent::UpdateDownloaded(version, result) => {
                self.on_update_downloaded(version, result)
            }
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
                let shells = (self.config.shell_completions, self.config.shell.clone());
                self.load_config(false);
                // shell completions turned off, or another shell: the warm
                // completion shells are not needed any more
                if shells != (self.config.shell_completions, self.config.shell.clone()) {
                    std::thread::spawn(rift_core::shell_complete::stop_all);
                }
                self.apply_config();
            }
            UserEvent::ViewerLoaded(loaded) => {
                if self.win.viewer.accept(*loaded)
                    && let Some(w) = &self.win.window
                {
                    w.request_redraw();
                }
            }
            // `route_user_event` served the asking window; one closed
            // meanwhile drops the answer
            UserEvent::WindowsLoaded(asker, ..) | UserEvent::DetachedLoaded(asker, _)
                if asker != self.win_id() => {}
            UserEvent::WindowsLoaded(_, windows, done) => {
                self.win.other_windows.extend(windows);
                self.win.windows_loading = !done;
                if self.win.palette.open {
                    self.refresh_palette();
                    if let Some(w) = &self.win.window {
                        w.request_redraw();
                    }
                }
            }
            UserEvent::DetachedLoaded(_, detached) => {
                self.win.detached = detached;
                self.win.detached_loading = false;
                if self.win.palette.open {
                    self.refresh_palette();
                    if let Some(w) = &self.win.window {
                        w.request_redraw();
                    }
                }
            }
            UserEvent::GitStatus(pane, info) => {
                let pr_chip = self
                    .config
                    .prompt_chips
                    .contains(&rift_core::PromptChip::GithubPr);
                let mut lookup = None;
                let mut reload_review = false;
                if let Some((_, tab)) = self.find_tab_of_pane(pane)
                    && let Some(p) = tab.pane_mut(pane)
                {
                    let branch = info.as_ref().map(|g| g.branch.clone());
                    // another branch's PR is not this one's
                    if p.git.as_ref().map(|g| &g.branch) != branch.as_ref() {
                        p.pr = None;
                    }
                    // the focused pane's changes moved (a poll saw an agent's
                    // edit): the review panel and the top bar follow
                    reload_review = p.git != info;
                    p.git = info;
                    let cwd = p.session.local_cwd();
                    if pr_chip && let (Some(cwd), Some(branch)) = (cwd, branch) {
                        lookup = Some((cwd, branch));
                    }
                }
                if let (Some((cwd, branch)), Some(worker)) = (lookup, &self.pr_worker) {
                    worker.request(pane, cwd.into(), branch);
                }
                if reload_review
                    && self
                        .win
                        .tabs
                        .get(self.win.active)
                        .is_some_and(|t| t.active_pane == pane)
                {
                    self.request_review();
                }
                if let Some(w) = &self.win.window {
                    w.request_redraw();
                }
            }
            UserEvent::PrStatus(pane, info) => {
                if let Some((_, tab)) = self.find_tab_of_pane(pane)
                    && let Some(p) = tab.pane_mut(pane)
                {
                    p.pr = info;
                }
                if let Some(w) = &self.win.window {
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
                if let Some(menu) = self.win.chip_menu.as_mut()
                    && menu.pane == pane
                    && menu.kind == crate::views::chip_menu::ChipKind::Docker
                {
                    let docker = self
                        .find_tab_of_pane(pane)
                        .and_then(|(_, t)| t.pane(pane))
                        .and_then(|p| p.docker.clone());
                    if let Some(menu) = self.win.chip_menu.as_mut() {
                        menu.set_containers(docker.as_ref());
                    }
                }
                if let Some(w) = &self.win.window {
                    w.request_redraw();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.open_queued_windows(event_loop);
        self.each_window(|app| app.sync_focus());
        // git and Docker status of the windows in use, on their beat
        self.each_window(|app| {
            if app.win.focused {
                app.request_git();
            }
        });
        // every tab's "Time on this tab" follows its panes' commands
        let clock_now = Instant::now();
        self.each_window(|app| {
            for tab in &mut app.win.tabs {
                tab.sync_clock(clock_now);
            }
        });
        // the counted time reaches the session file when a stretch ends and
        // every minute while one runs (no other event saves it); a frame also
        // syncs the clocks, so a stop is "fewer tabs counting than at the last
        // wake", not a before/after here
        let counting_tabs = self.all_tabs().filter(|t| t.clock.counting()).count();
        let stopped = counting_tabs < self.clock_counting;
        self.clock_counting = counting_tabs;
        let counting = counting_tabs > 0;
        let saved = self.clock_saved.filter(|_| counting);
        let save_due = saved.is_some_and(|at| clock_now.duration_since(at) >= CLOCK_SAVE_EVERY);
        if stopped || save_due {
            self.save_session_file();
        }
        self.clock_saved = match saved {
            _ if !counting => None,
            Some(at) if !save_due => Some(at),
            _ => Some(clock_now),
        };
        // a focus change (a click, ⌘[ / ⌘], ⌥⌘-arrows, focus follows mouse)
        // reaches the session file, so a restart focuses the pane last used:
        // one write per wake at most, and only when some tab's focus moved
        let focus: Vec<PaneId> = self.all_tabs().map(|t| t.active_pane).collect();
        if self.saved_focus.as_ref().is_some_and(|prev| *prev != focus) {
            self.save_session_file();
        }
        self.saved_focus = Some(focus);
        // each window's paced output frame, tooltip and selection scroll
        // ...and the drop-down window's slide
        let (mut output_frame, mut tip_due, mut select_due) = (None, None, None);
        self.each_window(|app| {
            let (o, t, s) = app.frame_deadlines();
            output_frame = earliest(output_frame, o);
            tip_due = earliest(tip_due, t);
            select_due = earliest(select_due, s);
            select_due = earliest(select_due, app.step_slide(Instant::now()));
        });
        // the remote gateway's files: at most once a second, on any wake
        self.poll_remote(false);
        if self.script.is_some() {
            self.drive_script(event_loop);
            if let Some(due) = self.script.as_ref().and_then(|s| s.due()) {
                let due = output_frame.map_or(due, |f| f.min(due));
                let due = tip_due.map_or(due, |t| t.min(due));
                let due = select_due.map_or(due, |t| t.min(due));
                event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(due));
                return;
            }
        }
        // every timed repaint (boot grace, toasts, cursor blink) wakes at the
        // earliest of its deadlines
        let now = Instant::now();
        let mut deadlines: Vec<Instant> = Vec::new();
        // remote access: the next read of the gateway's files (chip, the
        // Settings page and its countdown, the gateway's start)
        if self.remote_polling() {
            deadlines.push(self.remote.polled.map_or(now, |t| t + REMOTE_POLL));
        }
        deadlines.extend(select_due);
        if let Some(at) = self.clock_saved {
            deadlines.push(at + CLOCK_SAVE_EVERY);
        }
        self.each_window(|app| deadlines.extend(app.window_deadlines(now)));
        self.each_window(|app| deadlines.extend(app.status_poll_due()));
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
    fn the_quit_warning_names_connected_phones_before_processes() {
        let phone = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            close_description(2, &[]),
            "2 running processes will be terminated."
        );
        assert_eq!(
            close_description(0, &phone(&["iPhone"])),
            "iPhone is connected and loses its tabs."
        );
        assert_eq!(
            close_description(1, &phone(&["iPhone"])),
            "iPhone is connected and loses its tabs. 1 running process will be terminated."
        );
        assert_eq!(
            close_description(0, &phone(&["iPhone", "Pixel", "iPad"])),
            "iPhone and 2 other phones are connected and lose their tabs."
        );
    }

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
        // every spelling of a local file URL is a path, so a program behind
        // one is revealed (`file:/` with one slash used to reach the opener)
        for url in [
            "file:/x/Foo.app",
            "FILE:///x/Foo.app",
            "file://localhost/x/Foo.app",
            "File://LOCALHOST/x/Foo.app",
        ] {
            assert_eq!(local_path_of(url), Some("/x/Foo.app".into()), "{url}");
        }
        assert_eq!(local_path_of("file://server/x/Foo.app"), None);
        assert_eq!(local_path_of("file:x/Foo.app"), None);
        // what never reaches the OS opener
        for target in [
            "file://server/x/Foo.app",
            "x-apple.systempreferences:com.apple.preference",
            "ssh://host",
            "-a",
            "relative/doc.txt",
        ] {
            assert!(open_external(target).is_err(), "{target}");
        }
        assert_eq!(scheme_of("HTTPS://example.com").as_deref(), Some("https"));
        assert_eq!(scheme_of("/tmp/a:b"), None);
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
        let dir = std::env::temp_dir().join(format!("rift-exec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("tool");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        assert!(!opens_as_program(&exe));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(opens_as_program(&exe));
            // a harmless name linking to a bundle is the bundle
            let bundle = dir.join("Foo.app");
            std::fs::create_dir_all(&bundle).unwrap();
            let link = dir.join("readme.txt");
            std::os::unix::fs::symlink(&bundle, &link).unwrap();
            assert!(opens_as_program(&link));
        }
        // link files open whatever they point at
        for p in [
            "/x/a.fileloc",
            "/x/a.webloc",
            "/x/a.inetloc",
            "/x/A.prefPane",
        ] {
            assert!(opens_as_program(Path::new(p)), "{p}");
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
