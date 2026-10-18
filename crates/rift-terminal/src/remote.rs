//! `rift-terminal remote`: the gateway for the mobile companion
//! (`rift_remote`, protocol v2), fed by the windows' control sockets
//! (`control.rs`), and its pairing / device commands.
//!
//! Topology is each window's `status`; blocks, screens and output pages are
//! the owning window's `blocks`, `screen` and `output` requests; chip menus,
//! diffs and completions its `menu`, `diff` and `complete`; `run` /
//! `keys` / `text` from operate devices become the owning window's
//! `remote-input` request, which the window accepts only while
//! `remote_access` is on.

use crate::control::{self, Request, Response};
use rift_remote::protocol::{CloseKind, OpenKind, Pane, Tab, Window};
use rift_remote::registry::{self, Permission};
use rift_remote::{
    CompletePage, DiffPage, Input, InputError, Menu, MenuKind, OutputError, OutputPage, Page,
    Screen, Source, SourceError,
};
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

const DEFAULT_LISTEN: &str = "0.0.0.0:7878";
/// `--managed`: how often the config and the windows are checked.
const MANAGED_CHECK: Duration = Duration::from_millis(500);
/// `--managed`: exit this long after the last window is gone.
const NO_WINDOWS_EXIT: Duration = Duration::from_secs(30);
/// Set (to anything) by `--headless` runs and inherited by the gateway they
/// start: it shows no menu bar icon.
pub const HEADLESS_ENV: &str = "RIFT_HEADLESS";
/// Set by the gateway's SMAppService agent (`packaging/make-app.sh`), whose
/// fixed property list cannot name the log: the gateway appends its output
/// to `<data dir>/remote/gateway.log` itself.
const LOG_ENV: &str = "RIFT_GATEWAY_LOG";
/// `<data dir>/remote/stop`: a managed gateway that sees it removes it and
/// exits ("Quit Rift Completely").
const STOP_FILE: &str = "stop";

const HELP: &str = "usage: rift-terminal remote [--listen ADDR] [--managed]
       rift-terminal remote pair [--view]
       rift-terminal remote devices
       rift-terminal remote revoke ID
       rift-terminal remote permission ID operate|view
       rift-terminal remote history [--limit N]

Serve this user's Rift windows to paired phones (Noise-encrypted WebSocket,
protocol v2). ADDR defaults to `remote_listen` in config.toml, else
0.0.0.0:7878. With `remote_relay` set, the gateway also registers at that
relay; with `remote_tunnel` on, at a relay it runs behind a Cloudflare
quick tunnel instead (needs cloudflared). Both are followed while it runs.
One gateway runs per user.

  --managed      started by a Rift window: exits within 2 s after
                 `remote_access` becomes false, and 30 s after the last
                 window is gone
  pair [--view]  print a one-time pairing QR code and link (valid 2 minutes);
                 --view pairs a device that can only watch
  devices        list paired devices
  revoke ID      unpair a device (its connections close within a second)
  permission ID operate|view
                 change what a device may do (its connections close within
                 a second and reconnect with it)
  history        recent connections and failed handshakes, newest first
                 (--limit N, default 20)";

/// The windows' control sockets as a gateway source.
#[derive(Default)]
pub struct ControlSource {
    /// Which window's socket owns each pane (from the last topology).
    owner: Mutex<HashMap<u64, PathBuf>>,
    /// `remote-status` notices: (one is being sent, another is due after it).
    notifying: std::sync::Arc<Mutex<(bool, bool)>>,
    /// The last screen read of each pane, asked for again with its
    /// generation: an unchanged pane's window builds none.
    screens: Mutex<HashMap<u64, Held>>,
    /// Shells phones started while no window is open (`remote_when_closed`).
    background: crate::background::Background,
}

/// A pane's last screen read ([`ControlSource::screens`]).
struct Held {
    /// The window's socket that answered.
    owner: PathBuf,
    force: bool,
    generation: u64,
    screen: Option<Screen>,
}

/// How long one window may take to answer `remote-status`.
const NOTICE_TIMEOUT: Duration = Duration::from_millis(500);

/// Bits of a phone-visible pane id that hold the window's own pane id; the
/// window's pid is above them. Every window numbers its panes from 1, so
/// the window's id alone would name a pane of each window at once. Pids
/// fit 22 bits (Linux `pid_max`), so ids stay below 2^46, exact in the
/// app's JavaScript numbers.
const LOCAL_BITS: u32 = 24;

/// The pane id phones see for pane `local` of the window `pid`.
fn remote_pane(pid: u32, local: u64) -> u64 {
    (u64::from(pid) << LOCAL_BITS) | (local & ((1 << LOCAL_BITS) - 1))
}

/// A pane of the background window (pid 0): a shell the gateway started.
fn is_background(pane: u64) -> bool {
    pane >> LOCAL_BITS == 0
}

/// The theme background shells' screens resolve colours with.
fn background_theme() -> rift_core::Theme {
    rift_core::Config::try_load()
        .ok()
        .flatten()
        .unwrap_or_else(rift_core::Config::fallback)
        .resolve_theme_for(false)
}

/// Tell the computer's user that a phone started a shell.
fn started_notice(device: &str) {
    let who = shown(device, TOAST_DEVICE);
    log::info!("remote: {who} started a shell while Rift is closed");
    crate::notify::send(
        "Rift",
        &format!("{who} opened a terminal tab. Open Rift to see it."),
    );
}

/// The window's own id of the phone-visible pane `pane`.
fn local_pane(pane: u64) -> u64 {
    pane & ((1 << LOCAL_BITS) - 1)
}

impl ControlSource {
    fn owner_of(&self, pane: u64) -> Option<PathBuf> {
        self.owner.lock().unwrap().get(&pane).cloned()
    }

    /// `req` to the window owning `pane`: the cached owner first, after a
    /// miss (pane moved, window gone) look again once. `Ok(None)`: no
    /// window has the pane.
    fn ask(&self, pane: u64, req: &Request) -> std::io::Result<Option<Response>> {
        for attempt in 0..2 {
            let path = match self.owner_of(pane) {
                Some(p) if attempt == 0 => p,
                _ => {
                    self.topology();
                    match self.owner_of(pane) {
                        Some(p) => p,
                        None => return Ok(None),
                    }
                }
            };
            match control::request(&path, req) {
                Ok(r) if !r.ok && r.error.as_deref() == Some("no such pane") => continue,
                Ok(r) => return Ok(Some(r)),
                // the window is gone
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }
}

impl Source for ControlSource {
    fn topology(&self) -> Vec<Window> {
        let mut owner = HashMap::new();
        let mut windows = Vec::new();
        for path in control::sockets() {
            let Ok(r) = control::request(&path, &Request::Status) else {
                continue;
            };
            let Some(pid) = r.pid.filter(|_| r.ok) else {
                continue;
            };
            let tabs = r
                .tabs
                .into_iter()
                .map(|t| Tab {
                    index: t.index,
                    title: t.title,
                    active: t.active,
                    panes: t
                        .panes
                        .into_iter()
                        .filter(|p| p.id < 1 << LOCAL_BITS)
                        .map(|p| {
                            let id = remote_pane(pid, p.id);
                            owner.insert(id, path.clone());
                            Pane {
                                id,
                                active: p.active,
                                cwd: p.cwd,
                                job: p.job,
                                last_command: p.last_command,
                                last_exit: p.last_exit,
                                git: p.git,
                                chips: p.chips,
                                waiting: p.waiting,
                                running: p.running,
                            }
                        })
                        .collect(),
                })
                .collect();
            windows.push(Window {
                pid,
                background: false,
                tabs,
            });
        }
        if windows.is_empty() {
            windows.extend(self.background.window());
        } else {
            // Rift is open: it shows the background shells as tabs
            self.background.release();
        }
        // screens of panes that are gone are dropped
        self.screens
            .lock()
            .unwrap()
            .retain(|pane, _| owner.contains_key(pane) || is_background(*pane));
        *self.owner.lock().unwrap() = owner;
        windows
    }

    fn blocks(&self, pane: u64, before: Option<u64>, limit: usize) -> Result<Page, SourceError> {
        if is_background(pane) {
            return self
                .background
                .with(pane, |s| {
                    crate::views::terminal::remote_blocks(s, before, limit, false)
                })
                .ok_or(SourceError::NoPane);
        }
        self.read(
            pane,
            Request::Blocks {
                pane: local_pane(pane),
                before,
                limit,
                meta: false,
            },
        )
    }

    fn blocks_meta(&self, pane: u64, limit: usize) -> Result<Page, SourceError> {
        if is_background(pane) {
            return self
                .background
                .with(pane, |s| {
                    crate::views::terminal::remote_blocks(s, None, limit, true)
                })
                .ok_or(SourceError::NoPane);
        }
        self.read(pane, meta_request(local_pane(pane), limit))
    }

    /// Tell every window to re-read `status.json` (its chip), on a thread of
    /// its own: best effort, never blocking the gateway. Changes while a
    /// round is out coalesce into one more round.
    fn status_changed(&self) {
        let state = self.notifying.clone();
        {
            let mut s = state.lock().unwrap();
            if s.0 {
                s.1 = true;
                return;
            }
            s.0 = true;
        }
        let spawned = std::thread::Builder::new()
            .name("remote-notice".into())
            .spawn({
                let state = state.clone();
                move || {
                    loop {
                        for path in control::sockets() {
                            let _ = control::request_within(
                                &path,
                                &Request::RemoteStatus,
                                NOTICE_TIMEOUT,
                            );
                        }
                        let mut s = state.lock().unwrap();
                        if std::mem::take(&mut s.1) {
                            continue;
                        }
                        s.0 = false;
                        break;
                    }
                }
            });
        if spawned.is_err() {
            state.lock().unwrap().0 = false;
        }
    }

    fn input(&self, pane: u64, device: &str, input: Input) -> Result<(), InputError> {
        if is_background(pane) {
            return self.background.input(pane, input);
        }
        let (mut command, mut keys, mut text) = (None, None, None);
        match input {
            Input::Command(c) => command = Some(c),
            Input::Keys(k) => keys = Some(k.into_iter().map(|k| k.name()).collect()),
            Input::Text(t) => text = Some(t),
        }
        let req = Request::RemoteInput {
            pane: local_pane(pane),
            device: device.to_string(),
            command,
            keys,
            text,
        };
        match self.ask(pane, &req) {
            Ok(Some(r)) if r.ok => Ok(()),
            Ok(Some(r)) => Err(input_error(r.error.as_deref().unwrap_or_default())),
            Ok(None) => Err(InputError::NoPane),
            Err(e) => Err(InputError::Unavailable(e.to_string())),
        }
    }

    fn fit(&self, pane: u64, size: Option<(u16, u16)>) -> Result<(), InputError> {
        // a background shell has no window to size it for
        if is_background(pane) {
            return Ok(());
        }
        let req = Request::RemoteFit {
            pane: local_pane(pane),
            cols: size.map(|s| s.0),
            rows: size.map(|s| s.1),
        };
        match self.ask(pane, &req) {
            Ok(Some(r)) if r.ok => Ok(()),
            Ok(Some(r)) => Err(input_error(r.error.as_deref().unwrap_or_default())),
            Ok(None) => Err(InputError::NoPane),
            Err(e) => Err(InputError::Unavailable(e.to_string())),
        }
    }

    fn reset_elapsed(&self, pane: u64, device: &str) -> Result<(), InputError> {
        // a background shell has no tab clock
        if is_background(pane) {
            return Err(InputError::NoPane);
        }
        let req = Request::RemoteResetElapsed {
            pane: local_pane(pane),
            device: device.to_string(),
        };
        match self.ask(pane, &req) {
            Ok(Some(r)) if r.ok => Ok(()),
            Ok(Some(r)) => Err(input_error(r.error.as_deref().unwrap_or_default())),
            Ok(None) => Err(InputError::NoPane),
            Err(e) => Err(InputError::Unavailable(e.to_string())),
        }
    }

    fn open(&self, pane: u64, device: &str, kind: OpenKind) -> Result<u64, InputError> {
        if is_background(pane) {
            let id = self.background.open(pane, kind)?;
            started_notice(device);
            return Ok(id);
        }
        let req = Request::RemoteOpen {
            pane: local_pane(pane),
            device: device.to_string(),
            kind,
        };
        match self.ask(pane, &req) {
            // the new pane is in the same window as `pane`
            Ok(Some(r)) if r.ok => match (r.pane, u32::try_from(pane >> LOCAL_BITS)) {
                (Some(local), Ok(pid)) if local < 1 << LOCAL_BITS => Ok(remote_pane(pid, local)),
                _ => Err(InputError::Unavailable(
                    "the window did not name the new pane".into(),
                )),
            },
            Ok(Some(r)) => Err(input_error(r.error.as_deref().unwrap_or_default())),
            Ok(None) => Err(InputError::NoPane),
            Err(e) => Err(InputError::Unavailable(e.to_string())),
        }
    }

    fn close(&self, pane: u64, device: &str, kind: CloseKind) -> Result<(), InputError> {
        if is_background(pane) {
            // a background tab holds one shell: both kinds end it
            self.background.close(pane)?;
            log::info!(
                "remote: {} closed a shell while Rift is closed",
                shown(device, TOAST_DEVICE)
            );
            return Ok(());
        }
        let req = Request::RemoteClose {
            pane: local_pane(pane),
            device: device.to_string(),
            kind,
        };
        match self.ask(pane, &req) {
            Ok(Some(r)) if r.ok => Ok(()),
            Ok(Some(r)) => Err(input_error(r.error.as_deref().unwrap_or_default())),
            Ok(None) => Err(InputError::NoPane),
            Err(e) => Err(InputError::Unavailable(e.to_string())),
        }
    }

    /// The pane's screen; the window is told the generation of the one held
    /// from it (same window, same `force`), and answers `unchanged` instead
    /// of building it again while its output did not move.
    fn screen(&self, pane: u64, force: bool) -> Result<Option<Screen>, SourceError> {
        if is_background(pane) {
            let theme = background_theme();
            return self
                .background
                .with(pane, |s| {
                    crate::views::terminal::remote_screen(s, &theme, force, None)
                        .1
                        .flatten()
                })
                .ok_or(SourceError::NoPane);
        }
        let owner = self.owner_of(pane);
        let since = self
            .screens
            .lock()
            .unwrap()
            .get(&pane)
            .filter(|h| Some(&h.owner) == owner.as_ref() && h.force == force)
            .map(|h| h.generation);
        let req = Request::Screen {
            pane: local_pane(pane),
            force,
            since,
        };
        let r = match self.ask(pane, &req) {
            Ok(Some(r)) if r.ok => r,
            Ok(Some(r)) => return Err(SourceError::Unavailable(r.error.unwrap_or_default())),
            Ok(None) => return Err(SourceError::NoPane),
            Err(e) => return Err(SourceError::Unavailable(e.to_string())),
        };
        let mut screens = self.screens.lock().unwrap();
        // `ask` may have found the pane in another window: its `unchanged`
        // is about a generation of the other one's
        let now = self.owner_of(pane);
        if r.unchanged {
            return match screens.get(&pane) {
                Some(h) if now == owner && since.is_some() => Ok(h.screen.clone()),
                _ => {
                    screens.remove(&pane);
                    Err(SourceError::Unavailable(
                        "the screen moved; read again".into(),
                    ))
                }
            };
        }
        match (now, r.generation) {
            (Some(owner), Some(generation)) => {
                screens.insert(
                    pane,
                    Held {
                        owner,
                        force,
                        generation,
                        screen: r.screen.clone(),
                    },
                );
            }
            _ => {
                screens.remove(&pane);
            }
        }
        Ok(r.screen)
    }

    fn output(
        &self,
        pane: u64,
        block: u64,
        from: u64,
        limit: usize,
    ) -> Result<OutputPage, OutputError> {
        if is_background(pane) {
            return match self.background.with(pane, |s| {
                crate::views::terminal::block_output_range(s, block, from, limit)
            }) {
                Some(Some(page)) => Ok(page),
                Some(None) => Err(OutputError::NoBlock),
                None => Err(OutputError::NoPane),
            };
        }
        let req = Request::Output {
            pane: local_pane(pane),
            block,
            from,
            limit,
        };
        match self.ask(pane, &req) {
            Ok(Some(r)) if r.ok => Ok(r.output.unwrap_or_default()),
            Ok(Some(r)) if r.error.as_deref() == Some(NO_BLOCK) => Err(OutputError::NoBlock),
            Ok(Some(r)) => Err(OutputError::Unavailable(r.error.unwrap_or_default())),
            Ok(None) => Err(OutputError::NoPane),
            Err(e) => Err(OutputError::Unavailable(e.to_string())),
        }
    }

    fn menu(&self, pane: u64, kind: MenuKind) -> Result<Menu, OutputError> {
        let req = Request::Menu {
            pane: local_pane(pane),
            kind,
        };
        match self.ask(pane, &req) {
            Ok(Some(r)) if r.ok => r
                .menu
                .ok_or_else(|| OutputError::Unavailable("no menu in the answer".into())),
            Ok(Some(r)) => Err(OutputError::Unavailable(r.error.unwrap_or_default())),
            Ok(None) => Err(OutputError::NoPane),
            Err(e) => Err(OutputError::Unavailable(e.to_string())),
        }
    }

    fn complete(&self, pane: u64, line: &str, cursor: usize) -> Result<CompletePage, OutputError> {
        let req = Request::Complete {
            pane: local_pane(pane),
            line: line.to_string(),
            cursor,
        };
        match self.ask(pane, &req) {
            Ok(Some(r)) if r.ok => Ok(r.complete.unwrap_or_default()),
            Ok(Some(r)) => Err(OutputError::Unavailable(r.error.unwrap_or_default())),
            Ok(None) => Err(OutputError::NoPane),
            Err(e) => Err(OutputError::Unavailable(e.to_string())),
        }
    }

    fn can_start(&self) -> bool {
        let c = remote_config();
        c.access && c.when_closed
    }

    /// A shell for a phone: a tab in an open window when there is one,
    /// else a background shell (Rift shows it when it opens). The computer
    /// is told either way.
    fn start(&self, device: &str) -> Result<u64, InputError> {
        if !self.can_start() {
            return Err(InputError::Denied);
        }
        let windows = self.topology();
        let in_window = windows
            .iter()
            .filter(|w| !w.background)
            .flat_map(|w| &w.tabs)
            .find(|t| t.active)
            .and_then(|t| t.panes.iter().find(|p| p.active).or(t.panes.first()))
            .map(|p| p.id);
        if let Some(pane) = in_window {
            return self.open(pane, device, OpenKind::Tab);
        }
        let id = self.background.start()?;
        started_notice(device);
        Ok(id)
    }

    fn diff(&self, pane: u64, path: &str, staged: bool) -> Result<DiffPage, OutputError> {
        let req = Request::Diff {
            pane: local_pane(pane),
            path: path.to_string(),
            staged,
        };
        match self.ask(pane, &req) {
            Ok(Some(r)) if r.ok => Ok(r.diff.unwrap_or_default()),
            Ok(Some(r)) if r.error.as_deref() == Some(NO_CHANGE) => Err(OutputError::NoBlock),
            Ok(Some(r)) => Err(OutputError::Unavailable(r.error.unwrap_or_default())),
            Ok(None) => Err(OutputError::NoPane),
            Err(e) => Err(OutputError::Unavailable(e.to_string())),
        }
    }
}

/// `complete` for the phone (a window's `Request::Complete`): `line`'s
/// completions, each as the whole resulting line (the word at `cursor`
/// replaced, as the desktop's Tab does), then the commands starting with
/// `line` that ran before, newest first and each once: the pane's blocks
/// (`(command, finished at)`, oldest first), its editor history, the
/// imported shell history (both oldest first, without times).
pub fn complete_page(
    line: &str,
    cursor: usize,
    blocks: &[(String, Option<u64>)],
    editor_history: &[String],
    shell_history: &[String],
    cwd: Option<&str>,
    path: Option<&std::ffi::OsStr>,
) -> CompletePage {
    use rift_core::completion::{complete_on_path, current_token, token_end};
    use rift_remote::protocol::{Completion, CompletionKind, HistoryEntry};
    // a few more than the wire keeps, so `more` is set when some are cut
    const KEEP: usize = rift_remote::MAX_COMPLETE_ITEMS + 1;
    let mut items = Vec::new();
    if !line.trim().is_empty() && line.is_char_boundary(cursor) {
        let (start, _) = current_token(line, cursor);
        let end = token_end(line, cursor);
        // the whole word around the cursor, as the desktop's Tab completes it
        for c in complete_on_path(line, end, editor_history, shell_history, cwd, path) {
            let kind = match c.source {
                "history" => continue,
                "command" => CompletionKind::Command,
                "shell" => CompletionKind::Subcommand,
                "option" => CompletionKind::Option,
                "path" => CompletionKind::Path,
                _ => CompletionKind::Value,
            };
            let text = format!("{}{}{}", &line[..start], c.insert, &line[end..]);
            if !items.iter().any(|i: &Completion| i.text == text) {
                items.push(Completion { text, kind });
            }
            if items.len() >= KEEP {
                break;
            }
        }
    }
    let mut history: Vec<HistoryEntry> = Vec::new();
    let ran = blocks
        .iter()
        .rev()
        .map(|(c, at)| (c.as_str(), *at))
        .chain(editor_history.iter().rev().map(|c| (c.as_str(), None)))
        .chain(shell_history.iter().rev().map(|c| (c.as_str(), None)));
    for (text, at) in ran {
        let text = text.trim();
        if text.is_empty() || !text.starts_with(line) || text.contains('\n') {
            continue;
        }
        if history.iter().any(|h| h.text == text) {
            continue;
        }
        history.push(HistoryEntry {
            text: text.to_string(),
            at,
        });
        if history.len() >= KEEP {
            break;
        }
    }
    CompletePage { items, history }
}

/// A window's `diff` error for a path that is not among the changes.
pub const NO_CHANGE: &str = "no such change";

/// A window's `output` error for a block the pane does not have.
pub const NO_BLOCK: &str = "no such block";

impl ControlSource {
    /// A `blocks` request to the window owning `pane` (the phone's id).
    fn read(&self, pane: u64, req: Request) -> Result<Page, SourceError> {
        match self.ask(pane, &req) {
            Ok(Some(r)) if r.ok => Ok(Page {
                blocks: r.blocks,
                has_older: r.has_older.unwrap_or(false),
                oldest: r.oldest,
            }),
            Ok(Some(r)) => Err(SourceError::Unavailable(r.error.unwrap_or_default())),
            Ok(None) => Err(SourceError::NoPane),
            Err(e) => Err(SourceError::Unavailable(e.to_string())),
        }
    }
}

/// The `done` probe's read: the newest `limit` blocks without output.
fn meta_request(pane: u64, limit: usize) -> Request {
    Request::Blocks {
        pane,
        before: None,
        limit,
        meta: true,
    }
}

/// A window's `remote-input` error for Enter at a prompt with text on the
/// command line (the gateway's `busy` with this message).
pub const LINE_HAS_TEXT: &str = "the command line has text; send the command with run";

/// A window's `remote-input` error as the gateway's.
fn input_error(msg: &str) -> InputError {
    match msg {
        "busy" => InputError::Busy,
        "no such pane" => InputError::NoPane,
        "remote access is off" => InputError::Denied,
        // control_api changed under the gateway
        m if m.starts_with("not allowed") => InputError::Denied,
        m => InputError::Unavailable(m.to_string()),
    }
}

/// The keys `remote` reads from config.toml.
#[derive(Debug, Default, PartialEq)]
struct RemoteConfig {
    access: bool,
    listen: Option<String>,
    /// `remote_relay` when set.
    relay: Option<String>,
    /// `remote_tunnel`: run a relay here behind a Cloudflare tunnel
    /// (`tunnel.rs`) and register there instead.
    tunnel: bool,
    /// `remote_when_closed` (with `durable_sessions`): phones start shells
    /// while no window is open, and the gateway stays up for them.
    when_closed: bool,
    /// `menu_bar_icon`: the gateway shows it while no window is open.
    menu_icon: bool,
}

/// `remote_access` and `remote_listen` from config.toml, read with the
/// app's own loader each time so a running gateway sees every save. A file
/// the app cannot load (no file, unreadable, invalid) means access off, as
/// the windows' fallback has it.
fn remote_config() -> RemoteConfig {
    let loaded = rift_core::Config::try_load();
    if let Ok(Some(c)) = &loaded {
        follow_masking(c);
    }
    remote_config_from(loaded)
}

/// The shells phones start while no window is open send their screens
/// masked like the windows' (`mask_secrets`): the gateway's own masker
/// follows config.toml, compiled again only when it changes.
fn follow_masking(c: &rift_core::Config) {
    type Wanted = (bool, Vec<rift_core::secrets::SecretPattern>);
    static LAST: Mutex<Option<Wanted>> = Mutex::new(None);
    let want = (c.mask_secrets, c.secret_patterns.clone());
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_ref() == Some(&want) {
        return;
    }
    let masker = want.0.then(|| {
        rift_core::secrets::Masker::new(&want.1)
            .or_else(|_| rift_core::secrets::Masker::new(&[]))
            .map(std::sync::Arc::new)
            .ok()
    });
    rift_core::secrets::set_active(masker.flatten());
    *last = Some(want);
}

fn remote_config_from(loaded: Result<Option<rift_core::Config>, String>) -> RemoteConfig {
    match loaded {
        Ok(Some(c)) => RemoteConfig {
            access: c.remote_access,
            relay: c.relay().map(str::to_string),
            listen: Some(c.remote_listen),
            tunnel: c.remote_tunnel,
            when_closed: c.remote_when_closed && c.durable_sessions,
            menu_icon: c.menu_bar_icon,
        },
        _ => RemoteConfig::default(),
    }
}

/// This user's registry directory (`<data dir>/remote`).
pub(crate) fn registry_dir() -> Option<PathBuf> {
    rift_core::shell_integration::data_dir().map(|d| registry::remote_dir(&d))
}

/// `rift-terminal remote …`. Returns the process exit code.
pub fn cli(args: &[String]) -> i32 {
    let Some(dir) = registry_dir() else {
        eprintln!("rift-terminal remote: no data directory (HOME is not set)");
        return 1;
    };
    match args.first().map(String::as_str) {
        Some("pair") => return pair(&dir, &args[1..]),
        Some("devices") if args.len() == 1 => return devices(&dir),
        Some("revoke") if args.len() == 2 => return revoke(&dir, &args[1]),
        Some("permission") if args.len() == 3 => return permission(&dir, &args[1], &args[2]),
        Some("history") => return history(&dir, &args[1..]),
        Some("help" | "--help" | "-h") => {
            println!("{HELP}");
            return 0;
        }
        _ => {}
    }
    let mut listen: Option<String> = None;
    let mut managed = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--listen" => match it.next() {
                Some(v) => listen = Some(v.clone()),
                None => {
                    eprintln!("{HELP}");
                    return 2;
                }
            },
            "--managed" => managed = true,
            other => {
                eprintln!("rift-terminal remote: unknown argument {other:?}\n{HELP}");
                return 2;
            }
        }
    }
    let config = remote_config();
    if managed && !config.access {
        return 0;
    }
    if managed && std::env::var_os(LOG_ENV).is_some() {
        log_to_file(&dir);
    }
    let listen = listen
        .or(config.listen.clone())
        .unwrap_or_else(|| DEFAULT_LISTEN.into());
    let addr: SocketAddr = match listen.parse() {
        Ok(a) => a,
        Err(_) => {
            eprintln!("rift-terminal remote: listen address {listen:?}: expected IP:PORT");
            return 2;
        }
    };
    // one gateway per user: `gateway.lock` decides; status.json only names it
    let already = |dir: &Path| -> i32 {
        if managed {
            return 0;
        }
        match registry::read_status(dir) {
            Some(st) => eprintln!(
                "rift-terminal remote: a gateway is already running (pid {})",
                st.pid
            ),
            None => eprintln!("rift-terminal remote: a gateway is already running"),
        }
        1
    };
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e)
            if e.kind() == std::io::ErrorKind::AddrInUse
                && registry::read_status(&dir).is_some() =>
        {
            return already(&dir);
        }
        Err(e) => {
            eprintln!("rift-terminal remote: {addr}: {e}");
            return 1;
        }
    };
    let mut gw_config = rift_remote::Config::new(
        dir.clone(),
        hostname(),
        os().into(),
        env!("CARGO_PKG_VERSION").into(),
    );
    // with the tunnel, the relay is set once its address is known; it starts
    // only after this gateway holds the lock
    gw_config.relay = config.relay.clone().filter(|_| !config.tunnel);
    let gateway = match rift_remote::serve(listener, ControlSource::default(), gw_config) {
        Ok(g) => g,
        // another gateway holds the lock
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => return already(&dir),
        Err(e) => {
            eprintln!("rift-terminal remote: {e}");
            return 1;
        }
    };
    let bound = gateway.local_addr();
    log::info!("remote: gateway on {bound} (pid {})", std::process::id());
    let mut tunnel = crate::tunnel::Tunnel::new(&dir);
    gateway.set_relay(relay_for(&mut tunnel, &config));
    if !managed {
        println!("Rift remote gateway on {bound} (protocol v2, Noise-encrypted)");
        for a in registry::addresses(bound) {
            println!("  reachable at {a}");
        }
        if config.tunnel {
            println!("  through a relay behind a Cloudflare tunnel (starting)");
        } else if let Some(r) = &config.relay {
            println!("  through the relay {r}");
        }
        println!("Pair a phone: rift-terminal remote pair");
        if control::sockets().is_empty() {
            eprintln!("note: no Rift window is listening yet (control_api = \"off\"?)");
        }
        // follow `remote_relay` and `remote_tunnel` until killed
        loop {
            std::thread::sleep(MANAGED_CHECK);
            gateway.set_relay(relay_for(&mut tunnel, &remote_config()));
        }
    }
    // a stop left from before this gateway started is not for it
    let _ = std::fs::remove_file(dir.join(STOP_FILE));
    // managed, on macOS: the loop on a thread, the menu bar icon on the
    // main thread (AppKit's)
    #[cfg(target_os = "macos")]
    if std::env::var_os(HEADLESS_ENV).is_none() {
        let tray = std::sync::Arc::new(Tray::default());
        let shared = tray.clone();
        let spawned = std::thread::Builder::new()
            .name("gateway".into())
            .spawn(move || {
                follow_managed(gateway, tunnel, &dir, Some(&shared));
                shared.done.store(true, Ordering::Release);
            });
        match spawned {
            Ok(_) => crate::status_item::run_helper(&tray),
            Err(e) => eprintln!("rift-terminal remote: {e}"),
        }
        return 0;
    }
    follow_managed(gateway, tunnel, &dir, None);
    0
}

/// What a managed gateway's loop and its menu bar icon (macOS:
/// `status_item::run_helper` on the main thread) tell each other.
#[derive(Default)]
pub struct Tray {
    /// Show the icon: `menu_bar_icon` is on and no window is open.
    pub want: AtomicBool,
    /// "Quit Rift Completely" was picked.
    pub quit: AtomicBool,
    /// The loop ended: the main thread returns.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub done: AtomicBool,
}

/// A managed gateway's life: follow the config and the windows until
/// `remote_access` turns off, no window has been open for
/// [`NO_WINDOWS_EXIT`] (unless phones may start shells meanwhile), a
/// [`STOP_FILE`] appears, or the menu bar icon's full quit is picked.
fn follow_managed(
    gateway: rift_remote::Gateway,
    mut tunnel: crate::tunnel::Tunnel,
    dir: &Path,
    tray: Option<&Tray>,
) {
    // when no window was first seen (the clock starts at launch too, so a
    // gateway started with no window also ends)
    let mut windowless: Option<Instant> = Some(Instant::now());
    let mut checked_windows: Option<Instant> = None;
    loop {
        std::thread::sleep(MANAGED_CHECK);
        let now = remote_config();
        if !now.access {
            log::info!("remote: remote_access is off; stopping");
            break;
        }
        if std::fs::remove_file(dir.join(STOP_FILE)).is_ok() {
            log::info!("remote: asked to stop");
            break;
        }
        gateway.set_relay(relay_for(&mut tunnel, &now));
        if let Some(tray) = tray {
            let window = any_window();
            if tray.quit.swap(false, Ordering::AcqRel) {
                log::info!("remote: Quit Rift Completely");
                if !window {
                    end_background_shells();
                }
                break;
            }
            // the app shows its own while it runs
            tray.want.store(now.menu_icon && !window, Ordering::Release);
        }
        if checked_windows.is_none_or(|t| t.elapsed() >= Duration::from_secs(2)) {
            checked_windows = Some(Instant::now());
            if any_window() || now.when_closed {
                windowless = None;
            } else if windowless.get_or_insert_with(Instant::now).elapsed() >= NO_WINDOWS_EXIT {
                log::info!("remote: no Rift window for {NO_WINDOWS_EXIT:?}; stopping");
                break;
            }
        }
    }
    drop(gateway);
    drop(tunnel);
}

/// "Quit Rift Completely" in the app: the gateway stops (within
/// [`MANAGED_CHECK`]), and the shells phones started in the session service
/// end with the window's own.
pub fn stop_background() {
    if let Some(dir) = registry_dir()
        && registry::read_status(&dir).is_some()
    {
        let _ = std::fs::write(dir.join(STOP_FILE), b"");
    }
    end_background_shells();
}

/// End every shell the session service runs (never starts it).
fn end_background_shells() {
    for s in rift_pty::service::list() {
        rift_pty::service::kill(s.id);
    }
}

/// Append stdout and stderr to `<dir>/gateway.log` (started over past
/// 1 MiB), as the bootstrapped job's `StandardOutPath` does.
fn log_to_file(dir: &Path) {
    use std::os::fd::AsRawFd;
    let path = dir.join("gateway.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1 << 20) {
        let _ = std::fs::write(&path, b"");
    }
    let opened = std::fs::create_dir_all(dir).and_then(|()| {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&path)
    });
    match opened {
        // SAFETY: dup2 onto this process's own stdout/stderr descriptors
        Ok(f) => unsafe {
            libc::dup2(f.as_raw_fd(), 1);
            libc::dup2(f.as_raw_fd(), 2);
        },
        Err(e) => eprintln!("rift-terminal remote: {}: {e}", path.display()),
    }
}

/// The relay the gateway registers at: the tunnel's once it is ready while
/// `remote_tunnel` is on, else `remote_relay`. Starts and stops the tunnel.
fn relay_for(tunnel: &mut crate::tunnel::Tunnel, config: &RemoteConfig) -> Option<String> {
    let url = tunnel.poll(config.tunnel);
    if config.tunnel {
        url
    } else {
        config.relay.clone()
    }
}

/// A window's control socket answers.
fn any_window() -> bool {
    control::sockets()
        .iter()
        .any(|p| std::os::unix::net::UnixStream::connect(p).is_ok())
}

fn pair(dir: &Path, args: &[String]) -> i32 {
    let permission = match args {
        [] => Permission::Operate,
        [v] if v == "--view" => Permission::View,
        _ => {
            eprintln!("{HELP}");
            return 2;
        }
    };
    let running = registry::read_status(dir);
    // the running gateway's relay, else the configured one (never while the
    // tunnel replaces it: its address is known only once it runs)
    let config = remote_config();
    let relay = running
        .as_ref()
        .and_then(|st| st.relay.as_ref().map(|r| r.url.clone()))
        .or_else(|| config.relay.filter(|_| !config.tunnel));
    if config.tunnel && relay.is_none() {
        eprintln!(
            "note: the tunnel is not ready yet (Settings › Remote access shows why); \
             this link works only on this network"
        );
    }
    let addresses = match &running {
        Some(st) => st.addresses.clone(),
        None => {
            let listen = remote_config()
                .listen
                .unwrap_or_else(|| DEFAULT_LISTEN.into());
            match listen.parse() {
                Ok(a) => registry::addresses(a),
                Err(_) => {
                    eprintln!("rift-terminal remote: remote_listen {listen:?}: expected IP:PORT");
                    return 1;
                }
            }
        }
    };
    if addresses.is_empty() && relay.is_none() {
        eprintln!("rift-terminal remote: no network address to pair over (not on a network?)");
        return 1;
    }
    let link = match registry::start_pairing_via(
        dir,
        &addresses,
        relay.as_deref(),
        &hostname(),
        permission,
    ) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("rift-terminal remote: {e}");
            return 1;
        }
    };
    print!("{}", registry::qr_terminal(&link.url));
    println!("{}", link.url);
    let left = link
        .expires_at
        .duration_since(SystemTime::now())
        .unwrap_or_default()
        .as_secs();
    println!(
        "Scan with the Rift app. One use, {} access, valid {left} s (until {}).",
        permission.as_str(),
        registry::format_rfc3339(link.expires_at)
    );
    if running.is_none() {
        eprintln!(
            "note: no gateway is running; set remote_access = true in config.toml \
             (or run `rift-terminal remote`) before scanning"
        );
    }
    0
}

fn devices(dir: &Path) -> i32 {
    let all = match registry::devices(dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("rift-terminal remote: {e}");
            return 1;
        }
    };
    if all.is_empty() {
        println!("no paired devices (pair one with `rift-terminal remote pair`)");
        return 0;
    }
    let connected: Vec<String> = registry::read_status(dir)
        .map(|s| s.clients.into_iter().map(|c| c.device).collect())
        .unwrap_or_default();
    println!(
        "{:<10} {:<24} {:<9} {:<8} {:<21} {:<21}",
        "ID", "NAME", "PLATFORM", "ACCESS", "PAIRED", "LAST SEEN"
    );
    for d in all {
        let live = if connected.contains(&d.id) {
            "  connected"
        } else {
            ""
        };
        println!(
            "{:<10} {:<24} {:<9} {:<8} {:<21} {:<21}{live}",
            d.id,
            d.name,
            d.platform,
            d.permission.as_str(),
            registry::format_rfc3339(d.paired_at),
            registry::format_rfc3339(d.last_seen)
        );
    }
    0
}

fn revoke(dir: &Path, id: &str) -> i32 {
    match registry::revoke(dir, id) {
        Ok(true) => {
            println!("revoked {id}");
            0
        }
        Ok(false) => {
            eprintln!("rift-terminal remote: no paired device {id:?} (see `remote devices`)");
            1
        }
        Err(e) => {
            eprintln!("rift-terminal remote: {e}");
            1
        }
    }
}

fn permission(dir: &Path, id: &str, value: &str) -> i32 {
    let Some(p) = Permission::parse(value) else {
        eprintln!("rift-terminal remote: permission is operate or view, not {value:?}");
        return 2;
    };
    match registry::set_permission(dir, id, p) {
        Ok(true) => {
            println!("{id} may now {}", p.as_str());
            0
        }
        Ok(false) => {
            eprintln!("rift-terminal remote: no paired device {id:?} (see `remote devices`)");
            1
        }
        Err(e) => {
            eprintln!("rift-terminal remote: {e}");
            1
        }
    }
}

/// How long a history entry lasted: `45s`, `12m 3s`, `2h 5m`.
pub(crate) fn duration_text(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

fn history(dir: &Path, args: &[String]) -> i32 {
    let limit = match args {
        [] => 20,
        [flag, n] if flag == "--limit" => match n.parse::<usize>() {
            Ok(n) if n > 0 => n,
            _ => {
                eprintln!("rift-terminal remote: --limit takes a positive number");
                return 2;
            }
        },
        _ => {
            eprintln!("{HELP}");
            return 2;
        }
    };
    let all = registry::history(dir);
    if all.is_empty() {
        println!("no connections yet");
        return 0;
    }
    println!(
        "{:<21} {:<9} {:<24} {:<6} {:<26} {:<9} REASON",
        "START", "DEVICE", "NAME", "VIA", "ADDRESS", "DURATION"
    );
    for e in all.iter().rev().take(limit) {
        println!(
            "{:<21} {:<9} {:<24} {:<6} {:<26} {:<9} {}",
            registry::format_rfc3339(e.start),
            e.device.as_deref().unwrap_or("-"),
            e.name.as_deref().unwrap_or("(unknown)"),
            e.via.as_str(),
            e.address,
            duration_text(e.end.duration_since(e.start).unwrap_or_default()),
            e.reason
        );
    }
    0
}

fn os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// The computer's name as its owner sees it: macOS's ComputerName ("Sam's MacBook
/// Pro"), Linux's `PRETTY_HOSTNAME` in /etc/machine-info, else the network hostname
/// (which screen readers spell out: "Sams dash MacBook dash Pro dot local").
pub(crate) fn hostname() -> String {
    friendly_name()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(network_hostname)
}

#[cfg(target_os = "macos")]
fn friendly_name() -> Option<String> {
    let out = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(not(target_os = "macos"))]
fn friendly_name() -> Option<String> {
    pretty_hostname(&std::fs::read_to_string("/etc/machine-info").ok()?)
}

/// `PRETTY_HOSTNAME=…` from machine-info(5) text (shell-style, optionally quoted).
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn pretty_hostname(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let v = l.trim().strip_prefix("PRETTY_HOSTNAME=")?.trim();
        let v = v
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(v);
        Some(v.to_string())
    })
}

fn network_hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer outlives the call and its length is passed
    let r = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if r != 0 {
        return "unknown".into();
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

// ── the window side ──

/// Where a phone's input to a pane goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The app's command editor owns the line (the last block is at its
    /// prompt, no full-screen program): a command is accepted and keys are
    /// editor keys.
    Editor,
    /// The shell has not started the command submitted last (no 133;C
    /// yet): a command and text are `busy` (text would be typed into the
    /// shell's line, not a program); keys go to the shell.
    Pending,
    /// A program has the pane, or there is no shell integration, or a
    /// prompt is drawn on the alternate screen (tmux, ssh: the gateway
    /// cannot tell them from a program): a command is `busy`; keys and
    /// text go to it.
    Raw,
}

impl Route {
    /// Whether a phone's `text` is typed into the pane.
    pub fn takes_text(self) -> bool {
        self == Route::Raw
    }
}

/// [`Route`] of a pane: `alt_screen` (a full-screen program), the state of
/// its last block (`None`: no shell integration) and whether a submitted
/// command is still waiting for the shell to start it.
pub fn route(
    alt_screen: bool,
    last: Option<&rift_core::BlockState>,
    submit_pending: bool,
) -> Route {
    match last {
        Some(rift_core::BlockState::Prompt) if !alt_screen && submit_pending => Route::Pending,
        Some(rift_core::BlockState::Prompt) if !alt_screen => Route::Editor,
        _ => Route::Raw,
    }
}

/// Longest command shown in the desktop's toast, in characters.
const TOAST_COMMAND: usize = 60;
/// Longest device name shown in the toast, in characters.
const TOAST_DEVICE: usize = 40;

/// `text` without control or bidirectional formatting characters
/// ([`registry::shown_char`]), cut to `max` characters with `…`.
fn shown(text: &str, max: usize) -> String {
    let clean: String = text.chars().filter(|&c| registry::shown_char(c)).collect();
    let clean = clean.trim();
    if clean.chars().count() <= max {
        return clean.to_string();
    }
    let mut out: String = clean.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// The desktop's toast for a phone's command: `<device> ran <command>`.
pub fn ran_toast(device: &str, command: &str) -> String {
    format!(
        "{} ran {}",
        shown(device, TOAST_DEVICE),
        shown(command, TOAST_COMMAND)
    )
}

/// The desktop's toast for a phone's keys: `<device> sent ctrl+c enter`.
pub fn sent_toast(device: &str, keys: &[rift_remote::Key]) -> String {
    let names: Vec<String> = keys.iter().map(|k| k.name()).collect();
    format!(
        "{} sent {}",
        shown(device, TOAST_DEVICE),
        shown(&names.join(" "), TOAST_COMMAND)
    )
}

/// The desktop's toast for a phone's `open`: `<device> opened a tab`.
pub fn opened_toast(device: &str, kind: OpenKind) -> String {
    let what = match kind {
        OpenKind::Tab => "a tab",
        OpenKind::Pane => "a pane",
    };
    format!("{} opened {what}", shown(device, TOAST_DEVICE))
}

/// The desktop's toast for a phone's `close`: `<device> closed a tab`.
pub fn closed_toast(device: &str, kind: CloseKind) -> String {
    let what = match kind {
        CloseKind::Tab => "a tab",
        CloseKind::Pane => "a pane",
    };
    format!("{} closed {what}", shown(device, TOAST_DEVICE))
}

/// The desktop's toast for a phone's text: `<device> typed into a program`.
/// Never the text itself: it may be a password.
pub fn typed_toast(device: &str) -> String {
    format!("{} typed into a program", shown(device, TOAST_DEVICE))
}

/// Start a managed gateway for this user (`<this exe> remote --managed`,
/// the same environment), detached so it outlives the window (a launchd job
/// on macOS: `rift_pty::launchd`), its output appended to
/// `<dir>/gateway.log` (started over past 1 MiB). A gateway already holding
/// the lock makes the new one exit at once.
pub fn spawn_managed(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    let path = dir.join("gateway.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1 << 20) {
        std::fs::write(&path, b"")?;
    }
    rift_pty::launchd::spawn_helper(
        "gateway",
        &std::env::current_exe()?,
        &["remote", "--managed"],
        Some(&path),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completions_are_whole_lines_and_history_is_newest_first_once() {
        let dir = std::env::temp_dir().join(format!("rift-complete-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        let cwd = dir.to_str();
        let blocks = [
            ("cargo test -p a".to_string(), Some(100)),
            ("ls".to_string(), Some(150)),
            ("cargo test --workspace".to_string(), Some(200)),
        ];
        let editor = ["cargo test --workspace".to_string()];
        let shell = ["cargo test -p a".to_string(), "cargo tree".to_string()];
        let page = complete_page("cargo t", 7, &blocks, &editor, &shell, cwd, None);
        let texts: Vec<&str> = page.items.iter().map(|i| i.text.as_str()).collect();
        assert!(texts.contains(&"cargo test"), "{texts:?}");
        assert!(texts.contains(&"cargo tree"), "{texts:?}");
        assert!(
            page.items
                .iter()
                .all(|i| i.kind == rift_remote::protocol::CompletionKind::Subcommand)
        );
        let history: Vec<(&str, Option<u64>)> = page
            .history
            .iter()
            .map(|h| (h.text.as_str(), h.at))
            .collect();
        assert_eq!(
            history,
            [
                ("cargo test --workspace", Some(200)),
                ("cargo test -p a", Some(100)),
                ("cargo tree", None)
            ]
        );
        // the word under a mid-line cursor is replaced, the rest kept
        let page = complete_page("ls sr | wc", 5, &[], &[], &[], cwd, None);
        assert!(
            page.items.iter().any(|i| i.text == "ls src/ | wc"),
            "{:?}",
            page.items
        );
        // a cursor inside the word completes the whole word (`sr`, not `s`)
        let page = complete_page("ls sr | wc", 4, &[], &[], &[], cwd, None);
        let texts: Vec<&str> = page.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["ls src/ | wc"]);
        // an empty line: no completions, every command ran before
        let page = complete_page("", 0, &blocks, &[], &[], cwd, None);
        assert!(page.items.is_empty());
        assert_eq!(page.history.len(), 3);
        // a cursor inside a character completes nothing (and does not panic)
        assert!(
            complete_page("é", 1, &[], &[], &[], cwd, None)
                .items
                .is_empty()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// rift-remote has no rift-core dependency (the relay builds without
    /// it), so the window's menu bounds and the wire's are two constants
    /// each: they must agree.
    #[test]
    fn waiting_bounds_match_the_wire() {
        use rift_core::prompts as p;
        assert_eq!(
            (
                p::MAX_PROMPT_CHARS,
                p::MAX_CHOICES,
                p::MAX_CHOICE_CHARS,
                p::MAX_CONTEXT_LINES
            ),
            (
                rift_remote::MAX_WAITING_PROMPT,
                rift_remote::MAX_WAITING_CHOICES,
                rift_remote::MAX_WAITING_CHOICE,
                rift_remote::MAX_WAITING_CONTEXT
            )
        );
    }

    #[test]
    fn pretty_hostname_comes_from_machine_info() {
        let t = "ICON_NAME=computer-laptop\nPRETTY_HOSTNAME=\"Sam's Laptop\"\nCHASSIS=laptop\n";
        assert_eq!(pretty_hostname(t).as_deref(), Some("Sam's Laptop"));
        assert_eq!(
            pretty_hostname("PRETTY_HOSTNAME=build-box").as_deref(),
            Some("build-box")
        );
        assert_eq!(pretty_hostname("CHASSIS=desktop\n"), None);
        // whatever the source, the name is never empty
        assert!(!hostname().trim().is_empty());
    }

    #[test]
    fn commands_need_the_editor_at_a_prompt() {
        use rift_core::BlockState as B;
        let running = B::Running {
            started: Instant::now(),
        };
        let done = B::Done {
            exit: Some(0),
            duration: Duration::ZERO,
        };
        assert_eq!(route(false, Some(&B::Prompt), false), Route::Editor);
        // a full-screen program, a running command, a command the shell has
        // not started yet, no shell integration: busy for commands
        assert_eq!(route(true, Some(&B::Prompt), false), Route::Raw);
        assert_eq!(route(false, Some(&running), false), Route::Raw);
        assert_eq!(route(false, Some(&B::Prompt), true), Route::Pending);
        assert_eq!(route(false, Some(&done), false), Route::Raw);
        assert_eq!(route(false, None, false), Route::Raw);
        // text: only into a program; never into the shell's line while the
        // command submitted last waits for it, nor into the editor; the
        // alternate screen and a shell without integration take it (the
        // gateway cannot tell their prompts from programs)
        assert!(!route(false, Some(&B::Prompt), true).takes_text());
        assert!(!route(false, Some(&B::Prompt), false).takes_text());
        assert!(route(false, Some(&running), false).takes_text());
        assert!(route(true, Some(&B::Prompt), true).takes_text());
        assert!(route(false, None, false).takes_text());
    }

    #[test]
    fn toasts_name_the_device_and_stay_short() {
        assert_eq!(
            ran_toast("Sam's iPhone", "cargo test"),
            "Sam's iPhone ran cargo test"
        );
        let long = "x".repeat(200);
        let t = ran_toast("p", &long);
        assert_eq!(t.chars().count(), "p ran ".len() + TOAST_COMMAND);
        assert!(t.ends_with('…'));
        assert_eq!(ran_toast("a\u{1b}[31mb\n", "ls"), "a[31mb ran ls");
        // bidirectional overrides cannot make the toast show another command
        assert_eq!(
            ran_toast("ph\u{202E}one", "rm -rf x \u{2067}#\u{202E}sl\u{2069}"),
            "phone ran rm -rf x #sl"
        );
        let keys = [
            rift_remote::Key::ctrl('c'),
            rift_remote::Key::plain(rift_remote::KeyBase::Enter),
        ];
        assert_eq!(sent_toast("phone", &keys), "phone sent ctrl+c enter");
        // typed text never shows: it may be a password
        assert_eq!(
            typed_toast("Sam's iPhone"),
            "Sam's iPhone typed into a program"
        );
        assert_eq!(
            typed_toast(&"p".repeat(100)).chars().count(),
            TOAST_DEVICE + 21
        );
        assert_eq!(opened_toast("phone", OpenKind::Tab), "phone opened a tab");
        assert_eq!(opened_toast("phone", OpenKind::Pane), "phone opened a pane");
        assert_eq!(closed_toast("phone", CloseKind::Tab), "phone closed a tab");
    }

    #[test]
    fn pane_ids_name_one_window_each() {
        // every window numbers its panes from 1: two windows' pane 1 differ
        let (a, b) = (remote_pane(4242, 1), remote_pane(4243, 1));
        assert_ne!(a, b);
        assert_eq!((local_pane(a), local_pane(b)), (1, 1));
        // the largest Linux pid keeps ids exact as JavaScript numbers
        let big = remote_pane(4_194_304, (1 << LOCAL_BITS) - 1);
        assert!(big < 1 << 53);
        assert_eq!(local_pane(big), (1 << LOCAL_BITS) - 1);
    }

    #[test]
    fn window_errors_map_to_input_errors() {
        assert_eq!(input_error("busy"), InputError::Busy);
        // Enter with a draft at the prompt: `busy` with its own message
        assert_eq!(
            input_error(LINE_HAS_TEXT),
            InputError::Unavailable(LINE_HAS_TEXT.into())
        );
        assert_eq!(input_error("no such pane"), InputError::NoPane);
        assert_eq!(input_error("remote access is off"), InputError::Denied);
        assert_eq!(
            input_error("not allowed: control permissions changed"),
            InputError::Denied
        );
        assert_eq!(
            input_error("remote input is not available"),
            InputError::Unavailable("remote input is not available".into())
        );
        assert!(!hostname().is_empty());
    }

    fn from_text(text: &str) -> RemoteConfig {
        let dir = std::env::temp_dir().join(format!(
            "rift-remote-cfg-{}-{}",
            std::process::id(),
            text.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, text).unwrap();
        let got = remote_config_from(rift_core::Config::try_load_from(&path));
        std::fs::remove_dir_all(&dir).ok();
        got
    }

    #[test]
    fn remote_keys_are_read_with_the_apps_loader() {
        // no file: defaults (off)
        assert_eq!(remote_config_from(Ok(None)), RemoteConfig::default());
        assert_eq!(
            from_text(""),
            RemoteConfig {
                access: false,
                listen: Some("0.0.0.0:7878".into()),
                relay: Some(rift_core::config::DEFAULT_RELAY.into()),
                tunnel: false,
                when_closed: false,
                menu_icon: true,
            }
        );
        assert!(!from_text("menu_bar_icon = false\n").menu_icon);
        assert_eq!(from_text("remote_relay = \"\"\n").relay, None);
        assert_eq!(
            from_text(
                "remote_access = true\nremote_listen = \"127.0.0.1:9\"\n\
                 remote_relay = \" wss://relay.example.com \"\nremote_tunnel = true\n"
            ),
            RemoteConfig {
                access: true,
                listen: Some("127.0.0.1:9".into()),
                relay: Some("wss://relay.example.com".into()),
                tunnel: true,
                when_closed: false,
                menu_icon: true,
            }
        );
        assert_eq!(from_text("remote_relay = \"  \"\n").relay, None);
        // shells while Rift is closed live in the session service
        assert!(from_text("remote_when_closed = true\n").when_closed);
        assert!(!from_text("remote_when_closed = true\ndurable_sessions = false\n").when_closed);
        // the tunnel replaces the configured relay; off, the relay is used
        let mut t = crate::tunnel::Tunnel::new(
            &std::env::temp_dir().join(format!("rift-relay-for-{}", std::process::id())),
        );
        let relayed = from_text("remote_relay = \"wss://relay.example.com\"\n");
        assert_eq!(
            relay_for(&mut t, &relayed).as_deref(),
            Some("wss://relay.example.com")
        );
        // only a real boolean turns it on, and a file the app cannot load
        // keeps it off (the windows fall back the same way)
        for text in [
            "remote_access = \"true\"",
            "remote_access = 1",
            "[x]\nremote_access = true",
            "remote_access = true\nbroken = [",
            "remote_access = true\nfont_size = \"big\"",
        ] {
            assert!(!from_text(text).access, "{text}");
        }
    }

    #[test]
    fn durations_read_short() {
        assert_eq!(duration_text(Duration::from_secs(0)), "0s");
        assert_eq!(duration_text(Duration::from_secs(59)), "59s");
        assert_eq!(duration_text(Duration::from_secs(60)), "1m 0s");
        assert_eq!(duration_text(Duration::from_secs(3599)), "59m 59s");
        assert_eq!(duration_text(Duration::from_secs(7500)), "2h 5m");
    }

    #[test]
    fn the_done_probe_asks_for_metadata_only() {
        let line = serde_json::to_string(&meta_request(12, 4)).unwrap();
        assert_eq!(
            line,
            r#"{"cmd":"blocks","pane":12,"before":null,"limit":4,"meta":true}"#
        );
    }
}
