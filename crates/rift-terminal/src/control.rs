//! Local control API: each Rift window listens on
//! `<data dir>/control/<pid>.sock` (a 0700 directory; other uids are
//! refused) for newline-delimited JSON requests, one response line each.
//!
//! `control_api` in config.toml gates what is accepted:
//!
//! | level | commands |
//! |---|---|
//! | `"off"` | no socket, unless `remote_access` is on: then only the remote gateway's requests: `status`, `blocks`, `screen`, `output`, `menu`, `diff`, `complete`, `remote-status`, `remote-input` and `remote-open` |
//! | `"navigate"` (default) | the gateway's requests, `list`, `focus`, `prepare-restart` |
//! | `"full"` | also `split`, `open`, `send-text` (types into a shell) |
//!
//! `remote-input` and `remote-open` are accepted only while `remote_access`
//! is on, whatever the level.
//!
//! `prepare-restart` is sent by a window that installs an update: the
//! receiver saves its layout to `<data dir>/restart/<pid>.toml`, leaves its
//! durable shells running and quits (the updated app reopens it). It must
//! carry the token from that update's marker (`update-pending.json`,
//! readable by the user only, under 60 s old); without it the request is
//! refused.
//!
//! `rift-terminal ctl …` is the client. Requests are answered on the UI
//! thread, so a response reflects exactly what the window shows — except
//! `menu`, `diff` and `complete`, which read the filesystem and Git: the
//! UI thread reads the pane's folder (and history), a thread of their own
//! answers.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Off,
    /// No `control_api`, but `remote_access` is on: only what the remote
    /// gateway asks (`status`, `blocks`, `remote-input`).
    Remote,
    Navigate,
    Full,
}

impl Level {
    /// An unknown value ("Off", "false", a typo) never grants access: it
    /// parses as `Off`, with a warning.
    pub fn parse(s: &str) -> Level {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Level::Off,
            "navigate" => Level::Navigate,
            "full" => Level::Full,
            other => {
                log::warn!("control_api = {other:?} is not off|navigate|full; control is off");
                Level::Off
            }
        }
    }

    /// The socket's level: `control_api`, raised to [`Level::Remote`] while
    /// `remote_access` is on (the gateway needs a socket even with
    /// `control_api = "off"`).
    pub fn effective(control_api: &str, remote_access: bool) -> Level {
        let level = Level::parse(control_api);
        if remote_access {
            level.max(Level::Remote)
        } else {
            level
        }
    }

    /// The `control_api` value that grants this level.
    pub fn name(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Remote | Level::Navigate => "navigate",
            Level::Full => "full",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    List,
    Status,
    /// Read one pane's command blocks (output bounded like the remote
    /// gateway's): older than block id `before` when given, at most `limit`
    /// (capped at 30), newest last.
    Blocks {
        pane: u64,
        #[serde(default)]
        before: Option<u64>,
        #[serde(default = "blocks_limit")]
        limit: usize,
        /// Leave the output out (`output` empty, `dropped` 0): the remote
        /// gateway's `done` probe needs only ids, states and exits.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        meta: bool,
    },
    Focus {
        #[serde(default)]
        tab: Option<usize>,
        #[serde(default)]
        pane: Option<u64>,
    },
    Split {
        /// "right" or "down".
        direction: String,
        #[serde(default)]
        cwd: Option<String>,
    },
    Open {
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        command: Option<String>,
    },
    SendText {
        pane: u64,
        text: String,
    },
    /// One pane's visible terminal screen for the remote gateway (text runs
    /// with resolved colours, at most 300 × 120): none unless the pane
    /// shows the alternate screen or `force`. Read-only. `since`: the
    /// `generation` of the screen the gateway holds; while the pane's is
    /// still that, the answer is `unchanged` (no screen built).
    Screen {
        pane: u64,
        #[serde(default)]
        force: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<u64>,
    },
    /// Up to `limit` (1 to 500) output lines of block `block` of `pane`
    /// from line `from` on, numbered like the gateway's `dropped`.
    /// Read-only.
    Output {
        pane: u64,
        block: u64,
        from: u64,
        limit: usize,
    },
    /// A prompt chip's menu for `pane` (a paired phone's chips): what the
    /// folder and branch chip menus and the code review panel list.
    /// Read-only.
    Menu {
        pane: u64,
        kind: rift_remote::MenuKind,
    },
    /// The diff of one changed file of `pane`'s repository (`path` and
    /// `staged` as `menu` `changes` lists it). Read-only.
    Diff {
        pane: u64,
        path: String,
        #[serde(default)]
        staged: bool,
    },
    /// Completions and history for a command line in `pane` (a paired
    /// phone's Run sheet): what the editor would complete for `line` with
    /// the cursor at byte `cursor`, and the commands that ran before.
    /// Read-only.
    Complete {
        pane: u64,
        line: String,
        cursor: usize,
    },
    /// Input from a paired phone, sent by the remote gateway: exactly one
    /// of `command` (a command line for a pane at its prompt, as if typed
    /// and Enter pressed), `keys` (protocol key names, as if pressed) or
    /// `text` (typed into the running program). `device` is the phone's
    /// name for the desktop's toast. The window accepts it only while
    /// `remote_access` is on (answering `remote access is off`), and
    /// answers `busy` for a command to a pane that is running something and
    /// for text to a pane at its prompt.
    RemoteInput {
        pane: u64,
        device: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keys: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// A paired phone's `open`, sent by the remote gateway: a new tab in
    /// this window, or a new pane split right from `pane`, starting in
    /// `pane`'s folder; answers the new pane. Accepted only while
    /// `remote_access` is on, like `remote-input`.
    RemoteOpen {
        pane: u64,
        device: String,
        kind: rift_remote::protocol::OpenKind,
    },
    /// A paired phone's size for `pane` (the gateway's `fit`, already the
    /// smallest of its phones and clamped): the pane's shell is sized to the
    /// smaller of the window's own grid and `cols` × `rows` until released
    /// (`null` both) or until `FIT_LEASE` passes without the gateway sending
    /// it again. Accepted only while `remote_access` is on, like
    /// `remote-input`.
    RemoteFit {
        pane: u64,
        #[serde(default)]
        cols: Option<u16>,
        #[serde(default)]
        rows: Option<u16>,
    },
    /// Sent by the remote gateway whenever its `status.json` changed (it
    /// started, a phone connected or left): the window re-reads it at once
    /// (the tab-bar chip), even when idle. Read-only.
    RemoteStatus,
    /// Save this window's layout for the restart after an update, keep its
    /// durable shells running, and quit (see `updater.rs`). Only honored
    /// with the token of an update in flight: the updating window writes it
    /// into its 0600 update marker, so a program in a pane cannot make
    /// windows quit.
    PrepareRestart {
        #[serde(default)]
        token: String,
    },
}

impl Request {
    /// Least level that may run this request.
    pub fn level(&self) -> Level {
        match self {
            // what the remote gateway asks; `remote-input` and `remote-open`
            // are also gated by `remote_access` in the window
            Request::Status
            | Request::Blocks { .. }
            | Request::Screen { .. }
            | Request::Output { .. }
            | Request::Menu { .. }
            | Request::Diff { .. }
            | Request::Complete { .. }
            | Request::RemoteStatus
            | Request::RemoteInput { .. }
            | Request::RemoteFit { .. }
            | Request::RemoteOpen { .. } => Level::Remote,
            Request::List | Request::Focus { .. } | Request::PrepareRestart { .. } => {
                Level::Navigate
            }
            _ => Level::Full,
        }
    }
}

fn blocks_limit() -> usize {
    rift_remote::MAX_BLOCKS
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneInfo {
    pub id: u64,
    pub active: bool,
    pub cwd: Option<String>,
    /// Foreground job other than the shell.
    pub job: Option<String>,
    pub last_command: Option<String>,
    pub last_exit: Option<i32>,
    pub durable: bool,
    /// The pane's prompt-chip git status (absent from older windows, outside
    /// a repository, and before the window looked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<rift_remote::protocol::PaneGit>,
    /// What the pane's running command waits for (a password, an agent's
    /// menu, an unread "needs your input"); absent when nothing waits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<rift_remote::protocol::Waiting>,
    /// The running command's start and last line; absent at the prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<rift_remote::protocol::PaneRunning>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabInfo {
    pub index: usize,
    pub title: String,
    pub active: bool,
    pub panes: Vec<PaneInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<TabInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<u64>,
    /// `blocks`: the pane's blocks, newest last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<rift_remote::protocol::Block>,
    /// `blocks`: older blocks exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_older: Option<bool>,
    /// `blocks`: the pane's oldest block id (any page); lower ids are gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest: Option<u64>,
    /// `screen`: the pane's screen, when it has one to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<rift_remote::Screen>,
    /// `screen`: the pane's output generation the answer is of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    /// `screen`: the generation is still `since`; the screen is the one the
    /// gateway holds (not sent again).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unchanged: bool,
    /// `output`: the requested lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<rift_remote::OutputPage>,
    /// `menu`: the chip menu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<rift_remote::Menu>,
    /// `diff`: the file's lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<rift_remote::DiffPage>,
    /// `complete`: completions and history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complete: Option<rift_remote::CompletePage>,
}

impl Response {
    pub fn error(msg: impl Into<String>) -> Response {
        Response {
            ok: false,
            error: Some(msg.into()),
            ..Default::default()
        }
    }
}

/// A request waiting for the UI thread.
#[derive(Debug)]
pub struct Pending {
    pub request: Request,
    pub reply: mpsc::Sender<Response>,
    active: Arc<AtomicBool>,
}

impl Pending {
    pub fn allowed(&self, level: Level) -> bool {
        self.active.load(Ordering::Acquire) && self.request.level() <= level
    }
}

pub fn dir() -> Option<PathBuf> {
    rift_pty::ipc::socket_dir("control")
}

fn own_path() -> Option<PathBuf> {
    dir().map(|d| d.join(format!("{}.sock", std::process::id())))
}

/// Start listening; every request is handed to `deliver` (which forwards
/// it to the UI thread). The socket file is removed by [`Server`]'s drop.
pub struct Server {
    path: PathBuf,
    pub level: Level,
    active: Arc<AtomicBool>,
    connections: Arc<Mutex<Vec<Weak<UnixStream>>>>,
    listener: Option<std::thread::JoinHandle<()>>,
    wake: UnixStream,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        let _ = self.wake.shutdown(std::net::Shutdown::Both);
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
        for stream in self
            .connections
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
        {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn serve(level: Level, deliver: impl Fn(Pending) + Send + Sync + 'static) -> Option<Server> {
    if level == Level::Off {
        return None;
    }
    let dir = dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    // sockets of windows that are gone
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if UnixStream::connect(&p).is_err() {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    let path = own_path()?;
    let _ = std::fs::remove_file(&path);
    start(path, level, deliver)
}

const MAX_CONNECTIONS: usize = 32;
const MAX_REQUEST: usize = 1 << 20;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

fn start(
    path: PathBuf,
    level: Level,
    deliver: impl Fn(Pending) + Send + Sync + 'static,
) -> Option<Server> {
    let listener = UnixListener::bind(&path).ok()?;
    listener.set_nonblocking(true).ok()?;
    let (wake, stop) = UnixStream::pair().ok()?;
    let deliver = Arc::new(deliver);
    let active = Arc::new(AtomicBool::new(true));
    let connections = Arc::new(Mutex::new(Vec::<Weak<UnixStream>>::new()));
    let running = active.clone();
    let clients = connections.clone();
    let listener = std::thread::Builder::new()
        .name("control".into())
        .spawn(move || {
            let me = unsafe { libc::getuid() };
            while running.load(Ordering::Acquire) {
                let mut ready = [
                    libc::pollfd {
                        fd: listener.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: stop.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                ];
                let result = unsafe { libc::poll(ready.as_mut_ptr(), 2, -1) };
                if !running.load(Ordering::Acquire) {
                    break;
                }
                if result < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    break;
                }
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        continue;
                    }
                    // transient (ECONNABORTED, EMFILE, EINTR ...): keep serving
                    Err(e) => {
                        log::warn!("control: accept failed: {e}");
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                };
                if rift_pty::ipc::peer_uid(&stream) != Some(me) {
                    continue;
                }
                let stream = Arc::new(stream);
                let mut clients = clients.lock().unwrap();
                clients.retain(|c| c.strong_count() > 0);
                if clients.len() >= MAX_CONNECTIONS {
                    continue;
                }
                clients.push(Arc::downgrade(&stream));
                let deliver = deliver.clone();
                let active = running.clone();
                std::thread::spawn(move || handle_conn(stream, level, active, &*deliver));
            }
        })
        .ok()?;
    Some(Server {
        path,
        level,
        active,
        connections,
        listener: Some(listener),
        wake,
    })
}

/// `take` bounds allocation even when no newline ever arrives.
fn read_line_bounded(reader: &mut impl BufRead, limit: usize) -> std::io::Result<Option<String>> {
    let mut line = String::new();
    let n = reader.take(limit as u64 + 1).read_line(&mut line)?;
    if n > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "control line too large",
        ));
    }
    Ok((n != 0).then_some(line))
}

fn handle_conn(
    stream: Arc<UnixStream>,
    level: Level,
    active: Arc<AtomicBool>,
    deliver: &dyn Fn(Pending),
) {
    // macOS (BSD) accepted sockets inherit the listener's O_NONBLOCK; the
    // bounded reader below needs blocking reads with a timeout, not WouldBlock
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(IO_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(IO_TIMEOUT)).is_err()
    {
        return;
    }
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(&*stream);
    while active.load(Ordering::Acquire) {
        let Ok(Some(line)) = read_line_bounded(&mut reader, MAX_REQUEST) else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Err(e) => Response::error(format!("bad request: {e}")),
            Ok(req) if req.level() > level => Response::error(format!(
                "not allowed: set control_api = \"{}\" in config.toml to use {:?}",
                req.level().name(),
                req
            )),
            Ok(request) => {
                let (tx, rx) = mpsc::channel();
                if !active.load(Ordering::Acquire) {
                    break;
                }
                deliver(Pending {
                    request,
                    reply: tx,
                    active: active.clone(),
                });
                rx.recv_timeout(Duration::from_secs(5))
                    .unwrap_or_else(|_| Response::error("window did not answer"))
            }
        };
        let Ok(mut body) = serde_json::to_string(&response) else {
            break;
        };
        body.push('\n');
        if out.write_all(body.as_bytes()).is_err() {
            break;
        }
    }
}

/// Every window's socket, newest first.
pub fn sockets() -> Vec<PathBuf> {
    let Some(dir) = dir() else { return Vec::new() };
    let mut socks: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "sock"))
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .collect()
        })
        .unwrap_or_default();
    socks.sort_by_key(|a| std::cmp::Reverse(a.0));
    socks.into_iter().map(|(_, p)| p).collect()
}

/// One request to one window's socket.
pub fn request(path: &std::path::Path, req: &Request) -> std::io::Result<Response> {
    request_within(path, req, Duration::from_secs(6))
}

/// [`request`] that gives up after `timeout` per read or write.
pub fn request_within(
    path: &std::path::Path,
    req: &Request,
    timeout: Duration,
) -> std::io::Result<Response> {
    let mut s = UnixStream::connect(path)?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    let mut body = serde_json::to_string(req).map_err(std::io::Error::other)?;
    body.push('\n');
    s.write_all(body.as_bytes())?;
    let line = read_line_bounded(&mut BufReader::new(s), 16 << 20)?
        .ok_or_else(|| std::io::Error::other("window disconnected"))?;
    serde_json::from_str(&line).map_err(std::io::Error::other)
}

/// Other windows' tabs (for the session switcher): (pid, socket, tabs).
pub fn visit_other_windows(mut found: impl FnMut((u32, PathBuf, Vec<TabInfo>))) {
    let me = std::process::id();
    for window in sockets().into_iter().filter_map(|p| {
        let pid: u32 = p.file_stem()?.to_str()?.parse().ok()?;
        if pid == me {
            return None;
        }
        let r = request(&p, &Request::List).ok()?;
        Some((pid, p, r.tabs))
    }) {
        found(window);
    }
}

const CLI_HELP: &str = "usage: rift-terminal ctl [--pid PID] <command>

  list                          tabs and panes of every window (JSON)
  status                        this window's pid, version, tabs
  blocks --pane ID [--before ID] [--limit N]
                                a pane's command blocks, newest last (at most
                                30; output: last 500 lines, 64 KiB)
  focus [--tab N] [--pane ID]   raise the window, select a tab (N counts from 0,
                                as in `list`) or a pane
  split right|down [--cwd DIR]  split the active pane        (control_api = \"full\")
  open [--cwd DIR] [-- CMD…]    new tab, optionally running CMD (\"full\")
  send-text --pane ID TEXT      type TEXT into a pane         (\"full\")
  request JSON                  send one raw request line, e.g.
                                '{\"cmd\":\"blocks\",\"pane\":3,\"meta\":true}'

The newest window answers unless --pid names one.";

/// `rift-terminal ctl …`. Returns the process exit code.
pub fn cli(args: &[String]) -> i32 {
    let mut args: Vec<String> = args.to_vec();
    let mut pid: Option<String> = None;
    if let Some(i) = args.iter().position(|a| a == "--pid") {
        pid = args.get(i + 1).cloned();
        args.drain(i..(i + 2).min(args.len()));
    }
    let flag = |args: &mut Vec<String>, name: &str| -> Option<String> {
        let i = args.iter().position(|a| a == name)?;
        let v = args.get(i + 1).cloned();
        args.drain(i..(i + 2).min(args.len()));
        v
    };
    let Some(cmd) = args.first().cloned() else {
        println!("{CLI_HELP}");
        return 2;
    };
    if matches!(cmd.as_str(), "help" | "--help" | "-h") {
        println!("{CLI_HELP}");
        return 0;
    }
    let mut rest: Vec<String> = args[1..].to_vec();
    let req = match cmd.as_str() {
        "list" => {
            // every window
            let mut all = Vec::new();
            for p in sockets() {
                if let Ok(r) = request(&p, &Request::Status) {
                    all.push(r);
                }
            }
            println!("{}", serde_json::to_string_pretty(&all).unwrap_or_default());
            return 0;
        }
        "status" => Request::Status,
        "blocks" => {
            let mut number = |name: &str| flag(&mut rest, name).and_then(|v| v.parse::<u64>().ok());
            let Some(pane) = number("--pane") else {
                eprintln!("blocks needs --pane ID");
                return 2;
            };
            Request::Blocks {
                pane,
                before: number("--before"),
                limit: number("--limit").map_or(rift_remote::MAX_BLOCKS, |n| n as usize),
                meta: false,
            }
        }
        "request" => match serde_json::from_str::<Request>(&rest.join(" ")) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("request: not a control request: {e}");
                return 2;
            }
        },
        "focus" => {
            // a given but unusable value is an error, never "focus the
            // current tab"
            let number = |rest: &mut Vec<String>, name: &str| -> Result<Option<u64>, String> {
                let given = rest.iter().any(|a| a == name);
                match flag(rest, name) {
                    None if given => Err(format!("{name} needs a number")),
                    None => Ok(None),
                    Some(v) => v
                        .parse()
                        .map(Some)
                        .map_err(|_| format!("{name} needs a number, got {v:?}")),
                }
            };
            match (number(&mut rest, "--tab"), number(&mut rest, "--pane")) {
                (Ok(tab), Ok(pane)) => Request::Focus {
                    tab: tab.map(|t| t as usize),
                    pane,
                },
                (Err(e), _) | (_, Err(e)) => {
                    eprintln!("focus: {e}");
                    return 2;
                }
            }
        }
        "split" => Request::Split {
            cwd: flag(&mut rest, "--cwd"),
            direction: rest.first().cloned().unwrap_or_else(|| "right".into()),
        },
        "open" => {
            let command = rest
                .iter()
                .position(|a| a == "--")
                .map(|i| rest[i + 1..].join(" "))
                .filter(|c| !c.is_empty());
            Request::Open {
                cwd: flag(&mut rest, "--cwd"),
                command,
            }
        }
        "send-text" => {
            let Some(pane) = flag(&mut rest, "--pane").and_then(|v| v.parse().ok()) else {
                eprintln!("send-text needs --pane ID");
                return 2;
            };
            Request::SendText {
                pane,
                text: rest.join(" "),
            }
        }
        _ => {
            println!("{CLI_HELP}");
            return 2;
        }
    };
    let target = match &pid {
        Some(p) => dir().map(|d| d.join(format!("{p}.sock"))),
        None => sockets().into_iter().next(),
    };
    let Some(target) = target else {
        eprintln!("no running Rift window (or control_api = \"off\")");
        return 1;
    };
    match request(&target, &req) {
        Ok(r) => {
            println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
            if r.ok { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("{}: {e}", target.display());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unterminated_requests_stop_at_the_limit() {
        let mut infinite = BufReader::new(std::io::repeat(b'x'));
        assert_eq!(
            read_line_bounded(&mut infinite, MAX_REQUEST)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
        let mut lines = std::io::Cursor::new(b"one\ntwo\n");
        assert_eq!(
            read_line_bounded(&mut lines, 4).unwrap().as_deref(),
            Some("one\n")
        );
        assert_eq!(
            read_line_bounded(&mut lines, 4).unwrap().as_deref(),
            Some("two\n")
        );
        assert!(read_line_bounded(&mut lines, 4).unwrap().is_none());
    }

    #[test]
    fn dropping_server_revokes_queued_work_and_open_connections() {
        let path = std::env::temp_dir().join(format!("rift-revoke-{}.sock", std::process::id()));
        let (tx, rx) = mpsc::channel();
        let server = start(path.clone(), Level::Full, move |p| {
            let _ = tx.send(p);
        })
        .unwrap();
        let mut peer = UnixStream::connect(&path).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        peer.write_all(b"{\"cmd\":\"send-text\",\"pane\":1,\"text\":\"x\"}\n")
            .unwrap();
        let pending = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(pending.allowed(Level::Full));
        assert!(!pending.allowed(Level::Navigate));
        drop(server);
        assert!(!pending.allowed(Level::Full));
        assert!(!path.exists());
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        let _ = pending.reply.send(Response::error("revoked"));
        let server = start(path.clone(), Level::Navigate, |p| {
            let _ = p.reply.send(Response {
                ok: true,
                ..Default::default()
            });
        })
        .unwrap();
        assert!(request(&path, &Request::Status).unwrap().ok);
        assert!(
            !pending.allowed(Level::Full),
            "rebinding cannot revive old work"
        );
        drop(server);
    }

    #[test]
    fn delayed_split_and_repeated_requests_are_served() {
        let path = std::env::temp_dir().join(format!("rift-delay-{}.sock", std::process::id()));
        let server = start(path.clone(), Level::Navigate, |p| {
            let _ = p.reply.send(Response {
                ok: true,
                pid: Some(7),
                ..Default::default()
            });
        })
        .unwrap();
        let mut peer = UnixStream::connect(&path).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut lines = BufReader::new(peer.try_clone().unwrap()).lines();
        // first byte arrives after accept
        std::thread::sleep(Duration::from_millis(150));
        peer.write_all(b"{\"cmd\":\"status\"}\n").unwrap();
        let r: Response = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert!(r.ok && r.pid == Some(7));
        // one request split across writes, then a later second request
        peer.write_all(b"{\"cmd\":").unwrap();
        std::thread::sleep(Duration::from_millis(150));
        peer.write_all(b"\"status\"}\n").unwrap();
        let r: Response = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert!(r.ok);
        std::thread::sleep(Duration::from_millis(150));
        peer.write_all(b"{\"cmd\":\"status\"}\n").unwrap();
        let r: Response = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert!(r.ok);
        // a healthy idle connection is still revoked when the server goes
        drop(server);
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn requests_parse_and_levels_gate() {
        let r: Request = serde_json::from_str(r#"{"cmd":"focus","pane":3}"#).unwrap();
        assert_eq!(
            r,
            Request::Focus {
                tab: None,
                pane: Some(3)
            }
        );
        assert_eq!(r.level(), Level::Navigate);
        let r: Request =
            serde_json::from_str(r#"{"cmd":"send-text","pane":1,"text":"ls\n"}"#).unwrap();
        assert_eq!(r.level(), Level::Full);
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"rm-rf"}"#).is_err());
        let r: Request = serde_json::from_str(r#"{"cmd":"prepare-restart"}"#).unwrap();
        assert_eq!(
            r,
            Request::PrepareRestart {
                token: String::new()
            }
        );
        assert_eq!(r.level(), Level::Navigate);
        // the remote gateway's read: navigate, limit defaults to 30
        let r: Request =
            serde_json::from_str(r#"{"cmd":"blocks","pane":12,"before":null,"limit":5}"#).unwrap();
        assert_eq!(
            r,
            Request::Blocks {
                pane: 12,
                before: None,
                limit: 5,
                meta: false
            }
        );
        assert_eq!(r.level(), Level::Remote);
        // the `done` probe's metadata-only read
        let line = r#"{"cmd":"blocks","pane":3,"limit":4,"meta":true}"#;
        let r: Request = serde_json::from_str(line).unwrap();
        assert_eq!(
            r,
            Request::Blocks {
                pane: 3,
                before: None,
                limit: 4,
                meta: true
            }
        );
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"cmd":"blocks","pane":3,"before":null,"limit":4,"meta":true}"#
        );
        // false is the default and is not written
        let r = Request::Blocks {
            pane: 3,
            before: None,
            limit: 4,
            meta: false,
        };
        assert!(!serde_json::to_string(&r).unwrap().contains("meta"));
        let r: Request = serde_json::from_str(r#"{"cmd":"blocks","pane":3,"before":87}"#).unwrap();
        assert_eq!(
            r,
            Request::Blocks {
                pane: 3,
                before: Some(87),
                limit: 30,
                meta: false
            }
        );
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"blocks"}"#).is_err());
        // the remote gateway's input, exactly as docs/remote-protocol.md shows it
        let line =
            r#"{"cmd":"remote-input","pane":12,"device":"Sam's iPhone","command":"cargo test"}"#;
        let r: Request = serde_json::from_str(line).unwrap();
        assert_eq!(
            r,
            Request::RemoteInput {
                pane: 12,
                device: "Sam's iPhone".into(),
                command: Some("cargo test".into()),
                keys: None,
                text: None
            }
        );
        assert_eq!(r.level(), Level::Remote);
        assert_eq!(serde_json::to_string(&r).unwrap(), line);
        for line in [
            r#"{"cmd":"remote-input","pane":12,"device":"d","keys":["ctrl+c","enter"]}"#,
            r#"{"cmd":"remote-input","pane":12,"device":"Sam's iPhone","text":":wq"}"#,
            r#"{"cmd":"remote-fit","pane":12,"cols":64,"rows":40}"#,
            r#"{"cmd":"remote-fit","pane":12,"cols":null,"rows":null}"#,
            r#"{"cmd":"screen","pane":12,"force":false}"#,
            r#"{"cmd":"screen","pane":12,"force":true,"since":41}"#,
            r#"{"cmd":"output","pane":12,"block":86,"from":0,"limit":500}"#,
        ] {
            let r: Request = serde_json::from_str(line).unwrap();
            assert_eq!(serde_json::to_string(&r).unwrap(), line);
            assert_eq!(r.level(), Level::Remote, "{line}");
        }
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"cmd":"screen","pane":3}"#).unwrap(),
            Request::Screen {
                pane: 3,
                force: false,
                since: None
            }
        );
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"remote-input","pane":1}"#).is_err());
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"output","pane":1,"block":2}"#).is_err());
        // blocks travel in the response, omitted when empty
        let resp = Response {
            ok: true,
            blocks: vec![rift_remote::protocol::Block {
                id: 1,
                command: "ls".into(),
                state: rift_remote::protocol::BlockState::Done,
                exit: Some(0),
                duration_ms: 3,
                cwd: None,
                branch: None,
                output: "a\n".into(),
                dropped: 0,
                restored: false,
            }],
            has_older: Some(false),
            oldest: Some(1),
            ..Default::default()
        };
        let line = serde_json::to_string(&resp).unwrap();
        assert_eq!(serde_json::from_str::<Response>(&line).unwrap(), resp);
        // a screen and an output page travel the same way
        let resp = Response {
            ok: true,
            screen: Some(rift_remote::Screen {
                cols: 80,
                rows: 1,
                alt: true,
                cursor: Some([0, 3]),
                lines: vec![vec![rift_remote::protocol::Run::plain("vim")]],
            }),
            generation: Some(7),
            output: Some(rift_remote::OutputPage {
                from: 2,
                text: "a\n".into(),
                total: 9,
            }),
            ..Default::default()
        };
        let line = serde_json::to_string(&resp).unwrap();
        assert_eq!(serde_json::from_str::<Response>(&line).unwrap(), resp);
        let empty = serde_json::to_string(&Response {
            ok: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(empty, r#"{"ok":true}"#);
        // the screen the gateway holds is not sent again
        let same = r#"{"ok":true,"generation":7,"unchanged":true}"#;
        let resp = serde_json::from_str::<Response>(same).unwrap();
        assert!(resp.unchanged && resp.screen.is_none());
        assert_eq!(serde_json::to_string(&resp).unwrap(), same);
        assert_eq!(Level::parse("full"), Level::Full);
        assert_eq!(Level::parse("navigate"), Level::Navigate);
        assert_eq!(Level::parse("Full"), Level::Full);
        for bad in ["nonsense", "false", "", "OFF "] {
            assert_eq!(Level::parse(bad), Level::Off, "{bad:?}");
        }
        // "remote" is no control_api value
        assert_eq!(Level::parse("remote"), Level::Off);
    }

    #[test]
    fn remote_access_opens_only_the_gateways_requests() {
        assert_eq!(Level::effective("off", false), Level::Off);
        assert_eq!(Level::effective("off", true), Level::Remote);
        assert_eq!(Level::effective("navigate", true), Level::Navigate);
        assert_eq!(Level::effective("full", true), Level::Full);
        assert_eq!(Level::effective("navigate", false), Level::Navigate);
        assert_eq!(Level::effective("bogus", true), Level::Remote);
        let parse = |s: &str| serde_json::from_str::<Request>(s).unwrap();
        let r = parse(r#"{"cmd":"remote-status"}"#);
        assert_eq!(r, Request::RemoteStatus);
        assert_eq!(r.level(), Level::Remote);
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"cmd":"remote-status"}"#
        );
        for ok in [
            r#"{"cmd":"status"}"#,
            r#"{"cmd":"blocks","pane":1}"#,
            r#"{"cmd":"remote-input","pane":1,"device":"d","keys":["ctrl+c"]}"#,
            r#"{"cmd":"screen","pane":1}"#,
            r#"{"cmd":"output","pane":1,"block":2,"from":0,"limit":10}"#,
            r#"{"cmd":"menu","pane":1,"kind":"changes"}"#,
            r#"{"cmd":"diff","pane":1,"path":"a.rs"}"#,
            r#"{"cmd":"complete","pane":1,"line":"ls","cursor":2}"#,
            r#"{"cmd":"remote-open","pane":1,"device":"d","kind":"tab"}"#,
            r#"{"cmd":"remote-open","pane":1,"device":"d","kind":"pane"}"#,
        ] {
            assert!(parse(ok).level() <= Level::Remote, "{ok}");
        }
        for denied in [
            r#"{"cmd":"list"}"#,
            r#"{"cmd":"focus","pane":1}"#,
            r#"{"cmd":"prepare-restart"}"#,
            r#"{"cmd":"send-text","pane":1,"text":"x"}"#,
            r#"{"cmd":"split","direction":"right"}"#,
            r#"{"cmd":"open"}"#,
        ] {
            assert!(parse(denied).level() > Level::Remote, "{denied}");
        }
        // over a socket served at Remote: the denial names the level to set
        let path =
            std::env::temp_dir().join(format!("rift-remote-lvl-{}.sock", std::process::id()));
        let server = start(path.clone(), Level::Remote, |p| {
            let _ = p.reply.send(Response {
                ok: true,
                ..Default::default()
            });
        })
        .unwrap();
        assert!(request(&path, &Request::Status).unwrap().ok);
        let r = request(
            &path,
            &Request::Focus {
                tab: None,
                pane: None,
            },
        )
        .unwrap();
        assert!(r.error.unwrap().contains(r#"control_api = "navigate""#));
        let r = request(
            &path,
            &Request::SendText {
                pane: 1,
                text: "x".into(),
            },
        )
        .unwrap();
        assert!(r.error.unwrap().contains(r#"control_api = "full""#));
        drop(server);
    }

    #[test]
    fn socket_round_trip_and_permission_gate() {
        let dir = std::env::temp_dir().join(format!("rift-ctl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.sock");
        let listener = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            handle_conn(
                Arc::new(s),
                Level::Navigate,
                Arc::new(AtomicBool::new(true)),
                &|p: Pending| {
                    let _ = p.reply.send(Response {
                        ok: true,
                        pid: Some(42),
                        ..Default::default()
                    });
                },
            );
        });
        let mut s = UnixStream::connect(&path).unwrap();
        s.write_all(
            b"{\"cmd\":\"status\"}\n{\"cmd\":\"send-text\",\"pane\":1,\"text\":\"x\"}\nnot json\n",
        )
        .unwrap();
        let mut lines = BufReader::new(s).lines();
        let ok: Response = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert!(ok.ok && ok.pid == Some(42));
        let denied: Response = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert!(!denied.ok && denied.error.unwrap().contains("not allowed"));
        let bad: Response = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert!(bad.error.unwrap().starts_with("bad request"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
