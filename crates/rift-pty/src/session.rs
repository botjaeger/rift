use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, OnResize, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte;
use alacritty_terminal::vte::ansi::Rgb;

use rift_core::{BlockStore, Marker, OscScanner, Progress};

use crate::abs_term::AbsTerm;
use crate::graphics::Graphics;

pub type PaneId = u64;

#[derive(Debug)]
pub enum SessionEvent {
    /// New content — coalesced via the session's dirty flag.
    Wakeup,
    Title(String),
    /// The program reset its title (tab falls back to the automatic one).
    ResetTitle,
    /// OSC 52: the program copied text to the system clipboard.
    Clipboard(String),
    /// OSC 52 read (`ClipboardAccess::ReadWrite` only): reply with the
    /// clipboard text.
    ClipboardLoad(ClipboardReply),
    /// OSC 4/10/11/12 query: reply with palette index `usize`'s color.
    ColorRequest(usize, ColorReply),
    /// XTWINOPS (or DECCOLM/DECSLPP) the window must carry out or answer:
    /// parameters as sent, `None` = omitted.
    WindowOp(Vec<Option<u16>>),
    /// The program asked for a notification (OSC 9 / 777 / 99).
    Notify {
        title: String,
        body: String,
    },
    Bell,
    /// Something the user should know about the pane's connection (a
    /// durable session reconnected with output or input lost).
    Notice(String),
    /// A durable shell could not be started or reattached in the session
    /// service (asked off the window's thread, see [`Session::spawn`]): the
    /// window replaces this session, e.g. with a local shell.
    ServiceFailed(ServiceFailure),
    /// The session service answered: [`Session::service_id`] is known now
    /// (the window saves it, so a crash still reattaches this shell).
    ServiceReady,
    Exit,
}

/// Why a pane's session-service shell did not start, and what it had.
#[derive(Debug, Clone)]
pub struct ServiceFailure {
    pub error: String,
    /// The service accepted but did not answer in time ([`crate::service::SETUP_TIMEOUT`]):
    /// later panes would wait the same.
    pub stalled: bool,
    /// It was a reattach (the shell may have ended while detached).
    pub attach: bool,
    /// Where the shell was to start.
    pub cwd: Option<std::path::PathBuf>,
    /// Input typed meanwhile, for the replacement.
    pub held: Vec<u8>,
    /// Input bytes that did not fit while waiting.
    pub dropped: usize,
}

/// Formats a color into the reply escape sequence the program asked for.
#[derive(Clone)]
pub struct ColorReply(pub Arc<dyn Fn(Rgb) -> String + Send + Sync>);

/// The theme's colors by OSC index (0-255 palette, 256 foreground, 257
/// background, 258 cursor), set by the app. With them a color query (OSC
/// 4/10/11/12) is answered on the reader thread, in order with the replies
/// around it: Codex probes `OSC 10;? OSC 11;? DA1` and reads up to DA1.
static DEFAULT_COLORS: Mutex<Option<Vec<Rgb>>> = Mutex::new(None);

/// Set the colors queries fall back to when a program set none of its own.
pub fn set_default_colors(colors: Vec<Rgb>) {
    *DEFAULT_COLORS.lock().unwrap_or_else(|e| e.into_inner()) = Some(colors);
}

/// Colors of the panes with a colour scheme of their own (a profile's or the
/// palette's `theme`), by pane id: they answer that pane's queries in place
/// of [`DEFAULT_COLORS`].
static PANE_COLORS: Mutex<Option<std::collections::HashMap<PaneId, Vec<Rgb>>>> = Mutex::new(None);

/// Set (or with `None` clear) the colors `pane`'s queries fall back to.
pub fn set_pane_colors(pane: PaneId, colors: Option<Vec<Rgb>>) {
    let mut map = PANE_COLORS.lock().unwrap_or_else(|e| e.into_inner());
    match colors {
        Some(c) => {
            map.get_or_insert_with(Default::default).insert(pane, c);
        }
        None => {
            if let Some(m) = map.as_mut() {
                m.remove(&pane);
            }
        }
    }
}

fn default_color(pane: PaneId, index: usize) -> Option<Rgb> {
    let own = PANE_COLORS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(&pane).and_then(|c| c.get(index).copied()));
    own.or_else(|| {
        DEFAULT_COLORS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|c| c.get(index).copied())
    })
}

/// Formats the OSC 52 read reply for the clipboard text.
#[derive(Clone)]
pub struct ClipboardReply(pub Arc<dyn Fn(&str) -> String + Send + Sync>);

impl std::fmt::Debug for ClipboardReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClipboardReply")
    }
}

impl std::fmt::Debug for ColorReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ColorReply")
    }
}

/// App-provided sink for session events (called from the reader thread).
pub type Observer = Arc<dyn Fn(PaneId, SessionEvent) + Send + Sync>;

/// Grid dimensions in cells.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridSize {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Input typed while a dropped service connection is being replaced, at
/// most this many bytes (a paste larger than this is cut and reported).
const MAX_PENDING_INPUT: usize = 64 * 1024;

/// The connection to the session service that input and requests go on.
pub(crate) struct ServiceConn {
    /// `None` until the service answered the spawn or attach.
    stream: Option<crate::uds::UnixStream>,
    /// `Some` while the connection is down: input waits here, in order, and
    /// goes out on the replacement (see [`ServiceConn::reconnected`]).
    pending: Option<Vec<u8>>,
    /// Input bytes dropped because `pending` was full.
    dropped: usize,
    /// ⌘K came while not connected: the reader clears the service's
    /// history once it is.
    clear_history: bool,
}

impl ServiceConn {
    #[cfg(all(test, unix))]
    fn new(stream: crate::uds::UnixStream) -> Self {
        Self {
            stream: Some(stream),
            pending: None,
            dropped: 0,
            clear_history: false,
        }
    }

    /// Not connected yet: input is held until [`ServiceConn::reconnected`].
    fn connecting() -> Self {
        Self {
            stream: None,
            pending: Some(Vec::new()),
            dropped: 0,
            clear_history: false,
        }
    }

    /// Send input, or hold it while the connection is down. A frame is
    /// written whole or not at all as far as the service is concerned (it
    /// drops a partial frame with the connection), so the chunk that failed
    /// and everything after it are held, never one already sent.
    fn input(&mut self, session: u64, bytes: &[u8]) {
        if self.pending.is_none()
            && let Some(stream) = self.stream.as_mut()
        {
            for (i, chunk) in bytes.chunks(crate::ipc::MAX_INPUT_CHUNK).enumerate() {
                if crate::ipc::send_input(stream, session, chunk).is_err() {
                    self.pending = Some(Vec::new());
                    self.hold(&bytes[i * crate::ipc::MAX_INPUT_CHUNK..]);
                    return;
                }
            }
            return;
        }
        self.hold(bytes);
    }

    /// Whole or not at all: half a paste would leave the shell waiting for
    /// its end marker.
    fn hold(&mut self, bytes: &[u8]) {
        let pending = self.pending.get_or_insert_with(Vec::new);
        if pending.len() + bytes.len() > MAX_PENDING_INPUT {
            self.dropped += bytes.len();
            return;
        }
        pending.extend_from_slice(bytes);
    }

    /// The connection dropped: hold input from now on.
    fn down(&mut self) {
        self.pending.get_or_insert_with(Vec::new);
    }

    /// Switch to `stream` and send the input held meanwhile. Returns the
    /// number of input bytes that were dropped (pending was full).
    fn reconnected(&mut self, stream: crate::uds::UnixStream, session: u64) -> usize {
        self.stream = Some(stream);
        let held = self.pending.take().unwrap_or_default();
        self.input(session, &held);
        std::mem::take(&mut self.dropped)
    }
}

/// Where bytes for the shell go: the local PTY, or the session service.
#[derive(Clone)]
pub(crate) enum PtyWriter {
    File(Arc<File>),
    Service {
        conn: Arc<Mutex<ServiceConn>>,
        /// The service's id for the shell; 0 until it answered (input is
        /// held until then).
        session: Arc<AtomicU64>,
        /// Set while retained output is replayed after (re)attaching: the
        /// terminal's automatic replies (device attributes, graphics OKs)
        /// were answered when the output first arrived.
        muted: Arc<AtomicBool>,
    },
}

impl PtyWriter {
    /// User input.
    fn write(&self, bytes: &[u8]) {
        match self {
            PtyWriter::File(f) => {
                let _ = (&**f).write_all(bytes);
            }
            PtyWriter::Service { conn, session, .. } => {
                conn.lock()
                    .unwrap()
                    .input(session.load(Ordering::Acquire), bytes);
            }
        }
    }

    /// A request to the session service about this shell (nothing for a
    /// local PTY).
    fn request(&self, req: &crate::ipc::Request) {
        // not connected yet: the reader sends the size, and clears the
        // history, once it is
        if let PtyWriter::Service { conn, .. } = self {
            let mut c = conn.lock().unwrap();
            match c.stream.as_mut() {
                Some(stream) => {
                    let _ = crate::ipc::send_json(stream, req);
                }
                None => {
                    if matches!(req, crate::ipc::Request::ClearHistory { .. }) {
                        c.clear_history = true;
                    }
                }
            }
        }
    }

    /// Replaying retained output after a (re)attach.
    fn muted(&self) -> bool {
        matches!(self, PtyWriter::Service { muted, .. } if muted.load(Ordering::Acquire))
    }
}

/// Replies the terminal sends to the program on its own (device attributes,
/// cursor position, graphics OKs), queued to a writer thread. They are
/// produced while the term lock is held; a blocking write there would stall
/// the reader (and with it the program, which is then never read from) as soon
/// as a program floods queries without reading its stdin. A full queue drops
/// the reply: a program that stopped reading input does not want it.
#[derive(Clone)]
pub(crate) struct Replier {
    writer: PtyWriter,
    queue: std::sync::mpsc::SyncSender<Vec<u8>>,
}

/// Replies waiting for the writer thread.
const REPLY_QUEUE: usize = 256;

impl Replier {
    pub(crate) fn new(writer: PtyWriter) -> Self {
        let (queue, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(REPLY_QUEUE);
        let sink = writer.clone();
        // ends when the last Replier (the Term's event proxy) is dropped
        std::thread::Builder::new()
            .name("pty-reply".into())
            .spawn(move || {
                for bytes in rx {
                    sink.write(&bytes);
                }
            })
            .expect("spawn pty reply writer");
        Self { writer, queue }
    }

    /// Replaying retained output after a (re)attach.
    fn muted(&self) -> bool {
        self.writer.muted()
    }

    /// Terminal-generated replies to the program (dropped during replay and
    /// when the queue is full). Never blocks.
    fn reply(&self, bytes: &[u8]) {
        if !self.muted() {
            let _ = self.queue.try_send(bytes.to_vec());
        }
    }
}

/// Bridges Term-emitted events to the observer + PTY write-back.
pub struct EventProxy {
    pane: PaneId,
    observer: Observer,
    writer: Replier,
    /// Current grid/cell size (CSI 14 t / 18 t text-area queries).
    window_size: Arc<Mutex<WindowSize>>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup => (self.observer)(self.pane, SessionEvent::Wakeup),
            Event::Title(t) => (self.observer)(self.pane, SessionEvent::Title(t)),
            Event::ResetTitle => (self.observer)(self.pane, SessionEvent::ResetTitle),
            Event::Bell => (self.observer)(self.pane, SessionEvent::Bell),
            Event::Exit | Event::ChildExit(_) => (self.observer)(self.pane, SessionEvent::Exit),
            Event::PtyWrite(s) => self.writer.reply(s.as_bytes()),
            // the Term's osc52 mode (`ClipboardAccess`) decides which of these
            // arrive; reading is opt-in, since any program (or remote host)
            // could exfiltrate the clipboard with it
            Event::ClipboardStore(_, text) => {
                (self.observer)(self.pane, SessionEvent::Clipboard(text));
            }
            Event::ClipboardLoad(_, format) => {
                (self.observer)(
                    self.pane,
                    SessionEvent::ClipboardLoad(ClipboardReply(format)),
                );
            }
            // the Term answers colors the program set itself; the rest are
            // the theme's
            Event::ColorRequest(index, format) => match default_color(self.pane, index) {
                Some(rgb) => self.writer.reply(format(rgb).as_bytes()),
                None => (self.observer)(
                    self.pane,
                    SessionEvent::ColorRequest(index, ColorReply(format)),
                ),
            },
            Event::WindowOp(params) => (self.observer)(self.pane, SessionEvent::WindowOp(params)),
            Event::TextAreaSizeRequest(format) => {
                let size = *self.window_size.lock().unwrap();
                self.writer.reply(format(size).as_bytes());
            }
            _ => {}
        }
    }
}

/// A live shell: PTY + alacritty Term + reader thread + block model.
pub struct Session {
    pub id: PaneId,
    pub term: Arc<FairMutex<Term<EventProxy>>>,
    /// Block model, updated on the reader thread at OSC 133 boundaries.
    pub blocks: Arc<Mutex<BlockStore>>,
    /// Inline images (kitty graphics, iTerm2) and their placements.
    pub graphics: Arc<Mutex<Graphics>>,
    /// Lines ever pushed into scrollback history (monotone). Read together
    /// with the term lock for a consistent view: abs = scrolled + screen_row.
    pub scrolled: Arc<AtomicU64>,
    /// Last OSC 7 working directory.
    pub cwd: Arc<Mutex<Option<String>>>,
    /// The last OSC 7 named another machine (an ssh session): `cwd` is a
    /// path there, not on this disk. See [`Session::local_cwd`].
    pub cwd_remote: Arc<AtomicBool>,
    /// Shell state reported by OSC 1337 SetUserVar (prompt chips).
    pub user_vars: UserVars,
    /// The shell's own ⌃R / ⌃T / ⌥C widgets and a hand-off to one in
    /// flight (OSC 6973, `rift_core::shell_widget`).
    pub widgets: Widgets,
    /// The last OSC 9;4 progress report still standing (tab bar, Dock).
    pub progress: ProgressSlot,
    /// Set by the reader thread when new output arrived; cleared by the app
    /// when it consumes a snapshot. Coalesces wakeup events.
    pub dirty: Arc<AtomicBool>,
    /// True once the shell produced its first byte ("Starting zsh…" gate).
    pub saw_output: Arc<AtomicBool>,
    /// Bumped by everything that can change the screen or its cursor: each
    /// chunk of output parsed, a resize, a clear. Equal values: the same
    /// screen (the remote gateway's `screen` reads skip rebuilding it).
    pub generation: Arc<AtomicU64>,
    /// Writes of user input so far (`write`; never the terminal's own
    /// replies): a request for input raised before the last one was answered.
    inputs: AtomicU64,
    /// Read-only pane: user input is refused (`write`); the terminal's own
    /// reports still go.
    read_only: AtomicBool,
    /// Input was refused since the app last asked (`take_refused`).
    refused: AtomicBool,
    /// How the shell ended (set by the reader thread before `Exit`).
    exit: ExitCell,
    writer: InputQueue,
    backend: Backend,
    size: GridSize,
    window_size: Arc<Mutex<WindowSize>>,
    scrollback: usize,
    /// Captures at another size than this pane's ([`Session::shadow_capture`]).
    shadow: Arc<Shadow>,
    /// What a capture's driver needs to give held output back to the Term.
    feed_ctx: Arc<FeedCtx>,
    observer: Observer,
    /// The Term's setting, for a capture's own.
    ambiguous_wide: AtomicBool,
}

impl Drop for Session {
    /// A closed pane's own colors go with it, and any capture of it ends.
    fn drop(&mut self) {
        set_pane_colors(self.id, None);
        self.shadow.close();
    }
}

/// Input bytes waiting for a program that is not reading, at most (a single
/// larger paste still goes when nothing waits before it).
const MAX_QUEUED_INPUT: usize = 16 << 20;

/// User input and the terminal's reports, written to the shell in order on
/// the pane's own thread. A program that stops reading its input fills the
/// kernel's buffer (or the session service stops reading this socket), and a
/// write that blocks there froze the whole window: every pane shares one
/// event loop. Bounded: input that would pass [`MAX_QUEUED_INPUT`] is
/// refused whole (never half a paste) with a notice.
pub(crate) struct InputQueue {
    tx: std::sync::mpsc::Sender<Queued>,
    queued: Arc<AtomicUsize>,
    notice: Box<dyn Fn(String) + Send + Sync>,
}

/// What the pane's writer thread sends, in order.
enum Queued {
    Input(Vec<u8>),
    /// A request to the session service (resize, clear history): on the same
    /// connection as the input, so it waits behind input the service has not
    /// taken yet instead of blocking the window on the connection's lock.
    Request(crate::ipc::Request),
}

impl InputQueue {
    pub(crate) fn new(writer: PtyWriter, notice: impl Fn(String) + Send + Sync + 'static) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Queued>();
        let queued = Arc::new(AtomicUsize::new(0));
        let left = queued.clone();
        // ends once the session (the sender) is gone and the queue is written
        std::thread::Builder::new()
            .name("pty-input".into())
            .spawn(move || {
                for item in rx {
                    match item {
                        Queued::Input(bytes) => {
                            writer.write(&bytes);
                            left.fetch_sub(bytes.len(), Ordering::AcqRel);
                        }
                        Queued::Request(req) => writer.request(&req),
                    }
                }
            })
            .expect("spawn pty input writer");
        Self {
            tx,
            queued,
            notice: Box::new(notice),
        }
    }

    /// Queue `bytes`; false (and a notice) when refused.
    fn write(&self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return true;
        }
        let before = self.queued.fetch_add(bytes.len(), Ordering::AcqRel);
        if before > 0 && before + bytes.len() > MAX_QUEUED_INPUT {
            self.queued.fetch_sub(bytes.len(), Ordering::AcqRel);
            (self.notice)(format!(
                "Input not sent: the program is not reading ({} MB already waiting)",
                before.div_ceil(1 << 20)
            ));
            return false;
        }
        if self.tx.send(Queued::Input(bytes.to_vec())).is_err() {
            self.queued.fetch_sub(bytes.len(), Ordering::AcqRel);
            return false;
        }
        true
    }

    /// Queue a request to the session service, after the input before it.
    fn request(&self, req: crate::ipc::Request) {
        let _ = self.tx.send(Queued::Request(req));
    }

    /// Bytes queued and not yet written.
    #[cfg(all(test, unix))]
    fn queued(&self) -> usize {
        self.queued.load(Ordering::Acquire)
    }
}

/// A transcript to replay: terminal bytes, and (duration, cwd, command) per
/// block.
pub type Replay = (Vec<u8>, Vec<rift_core::Restored>);

/// Who owns the shell.
enum Backend {
    /// This process: the PTY closes (and the shell gets SIGHUP) with it.
    Local(tty::Pty),
    /// The session service: the shell outlives this window.
    Service {
        /// 0 until the service answered (see `PtyWriter::Service`).
        session: Arc<AtomicU64>,
        /// The pane was closed: a shell the service starts after that is
        /// ended at once.
        closed: Arc<AtomicBool>,
        /// Where input waits until the service answered.
        conn: Arc<Mutex<ServiceConn>>,
        /// The reader is done asking the service (answered or failed).
        settled: Arc<AtomicBool>,
        /// Foreground job reported by the service.
        job: Arc<Mutex<Option<String>>>,
        /// Input hidden, reported by the service.
        hidden: Arc<AtomicBool>,
    },
}

/// Run the shell in the session service instead of this process.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ServiceTarget {
    /// Start a new shell there.
    Spawn,
    /// Reattach to a shell that is still running there.
    Attach(u64),
}

/// Start `shell` on a new PTY with Rift's environment (TERM, shell
/// integration), its integration arguments, then `extra_args`. The master
/// fd is left blocking for a reader thread.
pub(crate) fn open_pty(
    shell: Option<&str>,
    extra_args: &[String],
    working_directory: Option<std::path::PathBuf>,
    env: Vec<(String, String)>,
    window_size: WindowSize,
    id: u64,
) -> std::io::Result<tty::Pty> {
    let mut tty_env: std::collections::HashMap<String, String> = env.into_iter().collect();
    tty_env
        .entry("TERM".into())
        .or_insert_with(|| "xterm-256color".into());
    tty_env
        .entry("COLORTERM".into())
        .or_insert_with(|| "truecolor".into());
    for (k, v) in rift_core::shell_integration::setup() {
        tty_env.entry(k).or_insert(v);
    }
    // always name the shell explicitly so bash can get its --rcfile shim
    let program = rift_core::shell_integration::resolve_shell(shell);
    let mut args = rift_core::shell_integration::shell_args(&program);
    args.extend(extra_args.iter().cloned());
    let tty_opts = tty::Options {
        shell: Some(tty::Shell::new(program, args)),
        working_directory,
        drain_on_exit: false,
        env: tty_env,
        // quote each argument for CreateProcess the way std does
        #[cfg(windows)]
        escape_args: true,
    };
    let pty = tty::new(&tty_opts, window_size, id)?;
    // alacritty configures the master fd non-blocking (it polls with mio);
    // we use a dedicated blocking reader thread instead (Windows: the
    // vendored ConPTY pipes are blocking already)
    #[cfg(unix)]
    unsafe {
        use std::os::fd::AsRawFd;
        let fd = pty.file().as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
    }
    Ok(pty)
}

/// The PTY's output, for the reader thread.
fn pty_reader(pty: &tty::Pty) -> std::io::Result<File> {
    #[cfg(unix)]
    return pty.file().try_clone();
    #[cfg(windows)]
    return pty.reader_file().try_clone();
}

/// The PTY's input, for keystrokes and replies.
fn pty_writer(pty: &tty::Pty) -> std::io::Result<File> {
    #[cfg(unix)]
    return pty.file().try_clone();
    #[cfg(windows)]
    return pty.writer_file().try_clone();
}

/// The shell's process id.
fn pty_pid(pty: &tty::Pty) -> u32 {
    #[cfg(unix)]
    return pty.child().id();
    #[cfg(windows)]
    return pty.child_pid();
}

/// Windows: ConPTY does not tell the console's foreground process.
#[cfg(windows)]
pub(crate) fn foreground_job_name(_pty: &tty::Pty) -> Option<String> {
    None
}

/// Windows: ConPTY does not expose the console's input mode.
#[cfg(windows)]
pub(crate) fn input_hidden(_pty: &tty::Pty) -> bool {
    false
}

/// Executable name of the PTY's foreground job when it is not the shell.
#[cfg(unix)]
pub(crate) fn foreground_job_name(pty: &tty::Pty) -> Option<String> {
    use std::os::fd::AsRawFd;
    let pgrp = unsafe { libc::tcgetpgrp(pty.file().as_raw_fd()) };
    let shell = pty.child().id() as i32;
    if pgrp <= 0 || pgrp == shell {
        return None;
    }
    Some(process_name(pgrp))
}

/// A process's executable name, or the name it was run by when the
/// executable is named after a version: Claude Code's native install runs
/// `~/.local/share/claude/versions/2.1.296` through a `claude` link, and
/// the job must still read as the agent. Empty when the OS does not say.
#[cfg(unix)]
fn process_name(pid: i32) -> String {
    #[cfg(target_os = "macos")]
    let name = {
        let mut buf = [0u8; 256];
        let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if n > 0 {
            String::from_utf8_lossy(&buf[..n as usize]).into_owned()
        } else {
            String::new()
        }
    };
    #[cfg(not(target_os = "macos"))]
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let versioned = name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_digit() || c == '.');
    if !versioned {
        return name;
    }
    argv0(pid)
        .and_then(|a| a.rsplit('/').next().map(str::to_string))
        .filter(|a| !a.is_empty())
        .unwrap_or(name)
}

/// A process's `argv[0]` (only the user's own processes are readable).
#[cfg(target_os = "macos")]
fn argv0(pid: i32) -> Option<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    let none = std::ptr::null_mut();
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, none, &mut size, none, 0) } != 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut size,
            none,
            0,
        )
    } != 0
    {
        return None;
    }
    buf.truncate(size);
    // argc (an int), the executable's path, NUL padding, then argv[0]
    let rest = buf.get(std::mem::size_of::<libc::c_int>()..)?;
    let rest = &rest[rest.iter().position(|&b| b == 0)?..];
    let rest = &rest[rest.iter().position(|&b| b != 0)?..];
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    Some(String::from_utf8_lossy(&rest[..end]).into_owned())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn argv0(pid: i32) -> Option<String> {
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let first = cmdline.split(|&b| b == 0).next()?;
    Some(String::from_utf8_lossy(first).into_owned())
}

/// The terminal reads a line without echo: ECHO off in canonical mode, as
/// `sudo`, `ssh` and `read -s` set it (full-screen programs turn ICANON
/// off too). The master shares the line discipline's settings.
#[cfg(unix)]
pub(crate) fn input_hidden(pty: &tty::Pty) -> bool {
    use std::os::fd::AsRawFd;
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(pty.file().as_raw_fd(), &mut t) } != 0 {
        return false;
    }
    t.c_lflag & libc::ECHO == 0 && t.c_lflag & libc::ICANON != 0
}

pub struct SpawnOptions {
    pub id: PaneId,
    pub size: GridSize,
    pub cell_size: (u16, u16),
    pub shell: Option<String>,
    /// Arguments after the shell-integration ones (a profile's).
    pub args: Vec<String>,
    pub working_directory: Option<std::path::PathBuf>,
    /// Extra environment for this shell only (a profile's).
    pub env: Vec<(String, String)>,
    pub scrollback: usize,
    pub observer: Observer,
    /// Saved transcript to show above the new shell: terminal bytes with
    /// OSC 133 block markers, parsed before the shell's first byte, and the
    /// (duration, cwd) of each replayed block.
    pub replay: Option<Replay>,
    /// Run the shell in the session service (durable sessions).
    pub service: Option<ServiceTarget>,
    /// With a reattach: the scrollback its window kept across an update
    /// restart ([`crate::kept::take`]), shown above what the service replays.
    pub kept: Option<crate::kept::Kept>,
    /// What OSC 52 may do with the system clipboard.
    pub clipboard: ClipboardAccess,
    /// East Asian Ambiguous characters take two cells (`ambiguous_width`).
    pub ambiguous_wide: bool,
}

/// OSC 52 policy: programs (and remote hosts) may copy to the clipboard,
/// also read it back, or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClipboardAccess {
    Off,
    #[default]
    Write,
    ReadWrite,
}

impl ClipboardAccess {
    /// `clipboard_access` config value; unknown values keep the default.
    pub fn from_config(value: &str) -> Self {
        match value.trim() {
            "off" => Self::Off,
            "read_write" => Self::ReadWrite,
            _ => Self::Write,
        }
    }

    fn osc52(self) -> alacritty_terminal::term::Osc52 {
        use alacritty_terminal::term::Osc52;
        match self {
            Self::Off => Osc52::Disabled,
            Self::Write => Osc52::OnlyCopy,
            Self::ReadWrite => Osc52::CopyPaste,
        }
    }
}

/// Where the reader thread gets output from.
enum Source {
    /// The PTY master and the shell's pid (its exit code is read there).
    Pty(File, u32),
    Service {
        /// Spawn or reattach (`service::open`), run first on the reader
        /// thread: a slow or stalled service never holds the window.
        open: Open,
        target: ServiceTarget,
        cwd: Option<std::path::PathBuf>,
        session_id: Arc<AtomicU64>,
        closed: Arc<AtomicBool>,
        settled: Arc<AtomicBool>,
        muted: Arc<AtomicBool>,
        job: Arc<Mutex<Option<String>>>,
        hidden: Arc<AtomicBool>,
        /// The connection `PtyWriter` sends input on (replaced on resume).
        conn: Arc<Mutex<ServiceConn>>,
        /// Scrollback the window before an update restart kept.
        kept: Option<Box<crate::kept::Kept>>,
    },
}

/// Connects to the session service and spawns or reattaches the shell.
type Open = Box<dyn FnOnce() -> std::io::Result<Opened> + Send>;

/// A shell the session service started or reattached.
struct Opened {
    stream: crate::uds::UnixStream,
    /// The service's id for it.
    session: u64,
    /// The reattached shell's directory as the service last saw it:
    /// fills the pane's cwd once the replay is done when no OSC 7 in
    /// the retained output did (a long-running job prints no prompt).
    known_cwd: Option<String>,
    /// The byte the replay starts at.
    from: u64,
    /// Reattach after the byte `u64` (see [`crate::service::resume`]).
    resume: Resume,
}

/// Opens a fresh service connection streaming output after the given byte;
/// true when output before it was no longer retained.
type Resume = Box<dyn FnMut(u64) -> std::io::Result<(crate::uds::UnixStream, bool)> + Send>;

/// Consecutive resumes (no output in between) before the pane gives up.
const MAX_RESUMES: u32 = 5;

/// How long a resume keeps trying a service that does not answer (a
/// machine swapping hard, a service restarting) before the pane counts the
/// service as lost.
const RESUME_WINDOW: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(600)
} else {
    std::time::Duration::from_secs(30)
};

/// `resume` until it answers, backing off up to 2 s between tries, for at
/// most [`RESUME_WINDOW`]. A service that answers that the session is gone
/// (`NotFound`) is final at once.
fn resume_patiently(
    resume: &mut Resume,
    from: u64,
) -> std::io::Result<(crate::uds::UnixStream, bool)> {
    let until = std::time::Instant::now() + RESUME_WINDOW;
    let mut wait = std::time::Duration::from_millis(100);
    loop {
        match resume(from) {
            Ok(v) => return Ok(v),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(e),
            Err(e) if std::time::Instant::now() + wait > until => return Err(e),
            Err(_) => {
                std::thread::sleep(wait);
                wait = (wait * 2).min(std::time::Duration::from_secs(2));
            }
        }
    }
}

/// The pane's reader thread: parses output into the Term and block model.
struct Reader {
    id: PaneId,
    /// How the shell ended, set before `SessionEvent::Exit` is sent.
    exit: ExitCell,
    ctx: Arc<FeedCtx>,
    /// A capture in progress takes the program's output ([`Shadow`]).
    shadow: Arc<Shadow>,
    dirty: Arc<AtomicBool>,
    saw_output: Arc<AtomicBool>,
    observer: Observer,
}

impl Reader {
    fn wake(&self) {
        if !self.dirty.swap(true, Ordering::AcqRel) {
            (self.observer)(self.id, SessionEvent::Wakeup);
        }
    }

    /// After a chunk is fed. A chunk with a shell marker always wakes: the
    /// app acts on what a marker changed (a shell widget's line, a prompt
    /// for queued commands) in its wakeup handler, and the coalesced wakeup
    /// it would otherwise ride on may already have been handled (the flag
    /// stays set until the next frame).
    fn wake_after(&self, marked: bool) {
        if self.dirty.swap(true, Ordering::AcqRel) && !marked {
            return;
        }
        (self.observer)(self.id, SessionEvent::Wakeup);
    }

    /// Before blocking on input: a synchronized update (DECSET 2026) holds
    /// output until its end marker; if the program stalls, show what it
    /// drew once vte's timeout expires. Returns true when it flushed.
    fn sync_timeout(&self, parser: &mut vte::ansi::Processor, fd: &impl WaitReadable) -> bool {
        let Some(deadline) = parser.sync_timeout().sync_timeout() else {
            return false;
        };
        let wait = deadline.saturating_duration_since(std::time::Instant::now());
        if fd.wait_readable(wait) {
            return false;
        }
        let mut term = self.ctx.term.lock();
        let mut scrolled_cur = self.ctx.scrolled.load(Ordering::Acquire);
        let mut abs = AbsTerm {
            term: &mut term,
            scrolled: &mut scrolled_cur,
            scrollback_cap: self.ctx.scrollback,
            saw_clear_saved: false,
            on_resize: None,
        };
        parser.stop_sync(&mut abs);
        self.ctx.scrolled.store(scrolled_cur, Ordering::Release);
        self.ctx.generation.fetch_add(1, Ordering::AcqRel);
        drop(term);
        self.wake();
        true
    }

    fn notice(&self, message: &str) {
        (self.observer)(self.id, SessionEvent::Notice(message.into()));
    }

    /// Output up to byte `seq` is parsed: mark the lines above the cursor's
    /// as drawn ([`crate::kept::Marks`]). On the alternate screen, the
    /// primary's history (its screen waits unchanged below).
    fn note_mark(&self, seq: u64) {
        use alacritty_terminal::term::TermMode;
        let term = self.ctx.term.lock();
        let scrolled = self.ctx.scrolled.load(Ordering::Acquire);
        let alt = term.mode().contains(TermMode::ALT_SCREEN);
        let boundary = if alt {
            scrolled
        } else {
            scrolled + term.grid().cursor.point.line.0.max(0) as u64
        };
        let mut marks = self.ctx.marks.lock().unwrap();
        marks.note(seq, boundary);
        marks.note_alt(seq, alt);
    }

    /// Parse kept lines ([`crate::kept::Kept::prelude`]) and scroll them
    /// all into history: the replay then starts on an empty screen, with
    /// the cursor home, as on a fresh one.
    fn feed_kept(&self, bytes: &[u8]) {
        let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
        let mut scanner = OscScanner::new();
        feed(&mut parser, &mut scanner, bytes, &self.ctx);
        let row = self.ctx.term.lock().grid().cursor.point.line.0.max(0);
        let home = if row > 0 {
            format!("\x1b[{row}S\x1b[H")
        } else {
            "\x1b[H".into()
        };
        feed(&mut parser, &mut scanner, home.as_bytes(), &self.ctx);
    }

    /// One read's worth of output: to the shadow while a capture takes it,
    /// else parsed. A capture given up here hands over all it held, in order.
    /// `None`: the Term did not change. Else true when it held a marker.
    fn take_output(
        &self,
        parser: &mut vte::ansi::Processor,
        scanner: &mut OscScanner,
        bytes: &[u8],
    ) -> Option<bool> {
        match self.shadow.on_output(bytes) {
            Output::Diverted => None,
            Output::Real => Some(feed(parser, scanner, bytes, &self.ctx)),
            Output::Flush(held) => Some(feed(parser, scanner, &held, &self.ctx)),
        }
    }

    fn run(self, source: Source) {
        let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
        let mut scanner = OscScanner::new();
        // the session service stopped answering: the shell may well run on
        let mut lost = false;
        match source {
            Source::Pty(mut file, pid) => {
                let mut buf = [0u8; 65536];
                loop {
                    if self.sync_timeout(&mut parser, &file) {
                        continue;
                    }
                    match file.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            self.saw_output.store(true, Ordering::Release);
                            if let Some(marked) =
                                self.take_output(&mut parser, &mut scanner, &buf[..n])
                            {
                                self.wake_after(marked);
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                self.exit.set(crate::exit_code_of(pid));
            }
            Source::Service {
                open,
                target,
                cwd,
                session_id,
                closed,
                settled,
                muted,
                job,
                hidden,
                conn,
                kept,
            } => {
                use crate::ipc::{Event as Ev, Frame, read_frame};
                let attach = matches!(target, ServiceTarget::Attach(_));
                let opened = open().and_then(|o| Ok((o.stream.try_clone()?, o)));
                let (
                    writer,
                    Opened {
                        mut stream,
                        session,
                        mut known_cwd,
                        from,
                        mut resume,
                    },
                ) = match opened {
                    Ok(v) => v,
                    Err(e) => {
                        settled.store(true, Ordering::SeqCst);
                        let (held, dropped) = {
                            let mut c = conn.lock().unwrap();
                            (
                                c.pending.take().unwrap_or_default(),
                                std::mem::take(&mut c.dropped),
                            )
                        };
                        (self.observer)(
                            self.id,
                            SessionEvent::ServiceFailed(ServiceFailure {
                                error: e.to_string(),
                                stalled: e.kind() == std::io::ErrorKind::TimedOut,
                                attach,
                                cwd,
                                held,
                                dropped,
                            }),
                        );
                        return;
                    }
                };
                session_id.store(session, Ordering::SeqCst);
                if closed.load(Ordering::SeqCst) {
                    // closed while the service was starting the shell
                    crate::service::kill(session);
                    settled.store(true, Ordering::SeqCst);
                    return;
                }
                settled.store(true, Ordering::SeqCst);
                (self.observer)(self.id, SessionEvent::ServiceReady);
                let dropped = {
                    let mut c = conn.lock().unwrap();
                    // the window may have changed size while it waited
                    let ws = *self.ctx.window_size.lock().unwrap();
                    let _ = crate::ipc::send_json(
                        &mut &writer,
                        &crate::ipc::Request::Resize {
                            session,
                            cols: ws.num_cols,
                            rows: ws.num_lines,
                            cell_w: ws.cell_width,
                            cell_h: ws.cell_height,
                        },
                    );
                    if c.clear_history {
                        let _ = crate::ipc::send_json(
                            &mut &writer,
                            &crate::ipc::Request::ClearHistory { session },
                        );
                    }
                    c.reconnected(writer, session)
                };
                // ⌘K before the service answered: the replay it is sending
                // is cleared too, once it is all in (the prompt stays)
                let mut clear_after_replay =
                    std::mem::take(&mut conn.lock().unwrap().clear_history);
                if dropped > 0 {
                    self.notice(&format!(
                        "The session service was slow to start this shell; {dropped} bytes \
                         of input typed meanwhile were dropped."
                    ));
                }
                // the lines before the replay's first byte, as the window
                // before an update restart had them
                if let Some(bytes) = kept.as_ref().and_then(|k| k.prelude(from)) {
                    self.feed_kept(&bytes);
                }
                // and the modes a full-screen program set in output the
                // service no longer holds: its replay goes on from there
                let mut redraw = false;
                // replayed bytes before this are already on the screen: the
                // program's picture as the window before the restart showed it
                let mut skip_to = 0u64;
                if let Some((k, modes)) =
                    kept.as_ref().and_then(|k| Some((k, k.modes_before(from)?)))
                {
                    let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
                    let mut scanner = OscScanner::new();
                    // the shell's screen first: the program covers it
                    feed(&mut parser, &mut scanner, &k.screen_before(from), &self.ctx);
                    feed(&mut parser, &mut scanner, modes, &self.ctx);
                    // A program that repaints only what changes may coalesce
                    // the size signals below and draw nothing: without its
                    // picture the replay's changes would land on an empty screen.
                    let (cols, rows) = {
                        let t = self.ctx.term.lock();
                        (t.columns(), t.screen_lines())
                    };
                    if let Some((seq, screen)) = k.alt_screen_from(from, cols, rows) {
                        feed(&mut parser, &mut scanner, screen, &self.ctx);
                        skip_to = seq;
                    }
                    redraw = true;
                }
                // first byte not yet fed to the parser, and resumes since
                // the last output
                let (mut next_seq, mut resumes) = (0u64, 0u32);
                // Replaying what the service kept after a resume: timed like the
                // original output, but never seen here, so the terminal still
                // answers its queries and its notifications still fire (only the
                // first attach's replay is `muted`).
                let mut resumed = false;
                loop {
                    if self.sync_timeout(&mut parser, &stream) {
                        continue;
                    }
                    let frame = match read_frame(&mut stream) {
                        Ok(Some(f)) => f,
                        // The socket ended without an `Exited` frame: the
                        // service dropped this (slow) client, or it died.
                        // The shell may well be running: reattach after the
                        // last byte seen; only a session the service says is
                        // gone ends the pane. A service that never answers
                        // leaves it lost, never ended: closing it would hang
                        // up a shell that may still run.
                        _ => {
                            resumes += 1;
                            if resumes > MAX_RESUMES {
                                lost = true;
                                break;
                            }
                            resumed = true;
                            // typing goes on while we reconnect: hold it
                            conn.lock().unwrap().down();
                            std::thread::sleep(std::time::Duration::from_millis(
                                100 * u64::from(resumes),
                            ));
                            let (fresh, truncated) = match resume_patiently(&mut resume, next_seq) {
                                Ok(v) => v,
                                Err(e) => {
                                    lost = e.kind() != std::io::ErrorKind::NotFound;
                                    break;
                                }
                            };
                            let Ok(writer) = fresh.try_clone() else {
                                lost = true;
                                break;
                            };
                            let dropped = {
                                let mut c = conn.lock().unwrap();
                                // the window may have changed while down
                                let ws = *self.ctx.window_size.lock().unwrap();
                                let _ = crate::ipc::send_json(
                                    &mut &writer,
                                    &crate::ipc::Request::Resize {
                                        session,
                                        cols: ws.num_cols,
                                        rows: ws.num_lines,
                                        cell_w: ws.cell_width,
                                        cell_h: ws.cell_height,
                                    },
                                );
                                c.reconnected(writer, session)
                            };
                            stream = fresh;
                            if truncated {
                                // the replay starts at an arbitrary byte,
                                // maybe inside an escape sequence
                                parser = vte::ansi::Processor::new();
                                scanner = OscScanner::new();
                                self.notice(
                                    "Reconnected to the session service; output produced \
                                     meanwhile was too long to keep and is missing.",
                                );
                            }
                            if dropped > 0 {
                                self.notice(&format!(
                                    "Reconnected to the session service; {dropped} bytes of \
                                     input typed meanwhile were dropped."
                                ));
                            }
                            continue;
                        }
                    };
                    match frame {
                        Frame::Output {
                            bytes, at_ms, seq, ..
                        } => {
                            resumes = 0;
                            next_seq = seq + bytes.len() as u64;
                            self.saw_output.store(true, Ordering::Release);
                            // replayed output keeps its original timing,
                            // so block durations survive a reattach
                            let first = muted.load(Ordering::Acquire);
                            let replaying = resumed || first;
                            if replaying {
                                let age = crate::ipc::now_ms().saturating_sub(at_ms);
                                let at = std::time::Instant::now()
                                    .checked_sub(std::time::Duration::from_millis(age));
                                let mut blocks = self.ctx.blocks.lock().unwrap();
                                blocks.set_clock(at);
                                // the first attach's commands finished before
                                // this window: no notifications for them
                                blocks.set_replaying(first);
                            }
                            let fresh = skip_to
                                .checked_sub(seq)
                                .map_or(0, |n| (n as usize).min(bytes.len()));
                            let marked = if fresh < bytes.len() {
                                self.take_output(&mut parser, &mut scanner, &bytes[fresh..])
                            } else {
                                None
                            };
                            self.note_mark(next_seq);
                            if let (false, Some(marked)) = (replaying, marked) {
                                self.wake_after(marked);
                            }
                        }
                        Frame::Json(body) => match serde_json::from_slice::<Ev>(&body) {
                            Ok(Ev::ReplayDone { .. }) => {
                                resumed = false;
                                {
                                    let mut blocks = self.ctx.blocks.lock().unwrap();
                                    blocks.set_clock(None);
                                    blocks.set_replaying(false);
                                }
                                if std::mem::take(&mut clear_after_replay) {
                                    clear_all(&self.ctx);
                                }
                                // after the replay, so its blocks keep the
                                // directories they ran in
                                if let Some(dir) = known_cwd.take() {
                                    self.ctx.cwd.lock().unwrap().get_or_insert(dir);
                                }
                                // A replay that started inside a full-screen
                                // program's run drew its changes over an empty
                                // screen: one row less and back makes the
                                // program draw it whole (two size signals).
                                if std::mem::take(&mut redraw) {
                                    let ws = *self.ctx.window_size.lock().unwrap();
                                    let c = conn.lock().unwrap();
                                    for rows in
                                        [ws.num_lines.saturating_sub(1).max(1), ws.num_lines]
                                    {
                                        let Some(stream) = c.stream.as_ref() else {
                                            break;
                                        };
                                        let _ = crate::ipc::send_json(
                                            &mut &*stream,
                                            &crate::ipc::Request::Resize {
                                                session,
                                                cols: ws.num_cols,
                                                rows,
                                                cell_w: ws.cell_width,
                                                cell_h: ws.cell_height,
                                            },
                                        );
                                    }
                                }
                                muted.store(false, Ordering::Release);
                                self.wake();
                            }
                            Ok(Ev::Status {
                                job: j, hidden: h, ..
                            }) => {
                                *job.lock().unwrap() = j;
                                hidden.store(h, Ordering::Release);
                                self.wake();
                            }
                            Ok(Ev::Exited { code, .. }) => {
                                self.exit.set(code);
                                break;
                            }
                            _ => {}
                        },
                        Frame::Input { .. } => {}
                    }
                }
            }
        }
        // the program's last words may be in a capture's hands
        if let Some(held) = self.shadow.close() {
            feed(&mut parser, &mut scanner, &held, &self.ctx);
        }
        // ended without a code (an older service): exited all the same; a
        // lost service is told apart, as its shell may still be running
        if lost {
            self.exit.set_lost();
        } else {
            self.exit.set_if_unset(None);
        }
        (self.observer)(self.id, SessionEvent::Exit);
    }
}

/// How a pane's shell ended: not yet, with a code, or with none known.
#[derive(Clone)]
pub struct ExitCell(Arc<AtomicI64>);

impl ExitCell {
    const RUNNING: i64 = i64::MIN;
    const UNKNOWN: i64 = i64::MIN + 1;
    /// The session service stopped answering: no exit seen, the shell may
    /// still be running there.
    const LOST: i64 = i64::MIN + 2;

    fn set_lost(&self) {
        let _ = self.0.compare_exchange(
            Self::RUNNING,
            Self::LOST,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// The pane ended because the session service stopped answering, not
    /// because its shell exited.
    pub fn lost(&self) -> bool {
        self.0.load(Ordering::Acquire) == Self::LOST
    }

    fn set(&self, code: Option<i32>) {
        self.0
            .store(code.map_or(Self::UNKNOWN, i64::from), Ordering::Release);
    }

    fn set_if_unset(&self, code: Option<i32>) {
        let v = code.map_or(Self::UNKNOWN, i64::from);
        let _ = self
            .0
            .compare_exchange(Self::RUNNING, v, Ordering::AcqRel, Ordering::Acquire);
    }

    /// `None` while the shell runs; `Some(None)` when it ended with no code
    /// known.
    pub fn get(&self) -> Option<Option<i32>> {
        match self.0.load(Ordering::Acquire) {
            Self::RUNNING => None,
            Self::UNKNOWN | Self::LOST => Some(None),
            v => Some(Some(v as i32)),
        }
    }
}

impl Default for ExitCell {
    fn default() -> Self {
        ExitCell(Arc::new(AtomicI64::new(Self::RUNNING)))
    }
}

/// ⌘K on the reader's side: a clear asked for while the service had not
/// answered, applied once its replay is in.
fn clear_all(ctx: &FeedCtx) {
    clear_all_of(
        &ctx.term,
        &ctx.scrolled,
        ctx.scrollback,
        &ctx.generation,
        &ctx.blocks,
        &ctx.graphics,
        |_| {},
    );
}

/// Wipe screen, scrollback and the block list in one atomic step (no-op on
/// the alt screen; false then). `while_locked` runs with the blocks lock
/// held, before the prompt is re-anchored.
fn clear_all_of(
    term: &FairMutex<Term<EventProxy>>,
    scrolled_at: &AtomicU64,
    scrollback: usize,
    generation: &AtomicU64,
    blocks: &Mutex<BlockStore>,
    graphics: &Mutex<Graphics>,
    while_locked: impl FnOnce(&mut BlockStore),
) -> bool {
    use alacritty_terminal::term::TermMode;
    use alacritty_terminal::vte::ansi::{ClearMode, Handler};

    let mut term = term.lock();
    if term.mode().contains(TermMode::ALT_SCREEN) {
        return false;
    }
    let mut scrolled = scrolled_at.load(Ordering::Acquire);
    {
        let mut abs = AbsTerm {
            term: &mut term,
            scrolled: &mut scrolled,
            scrollback_cap: scrollback,
            saw_clear_saved: false,
            on_resize: None,
        };
        abs.clear_screen(ClearMode::All);
    }
    // straight to the terminal: AbsTerm keeps the history on a Saved clear
    // (the shell's `clear` is a soft one), ⌘K drops it. Dropping lines
    // pushes none, so `scrolled` stays as it is.
    term.clear_screen(ClearMode::Saved);
    scrolled_at.store(scrolled, Ordering::Release);
    generation.fetch_add(1, Ordering::AcqRel);
    term.selection = None;

    // Re-anchor a fresh Prompt block at the post-clear cursor: the shell
    // still believes its prompt is on the (now cleared) row the cursor
    // occupies, so the next submitted command echoes exactly there and
    // header extraction stays correct.
    let cursor = term.grid().cursor.point;
    let abs_line = scrolled + cursor.line.0.max(0) as u64;
    let mut blocks = blocks.lock().unwrap();
    let was_prompt = matches!(
        blocks.last().map(|b| &b.state),
        Some(rift_core::BlockState::Prompt)
    );
    blocks.clear();
    graphics.lock().unwrap().clear();
    while_locked(&mut blocks);
    if was_prompt {
        blocks.on_prompt_start(abs_line);
        blocks.on_command_start(abs_line, cursor.column.0);
    }
    true
}

impl Session {
    pub fn spawn(opts: SpawnOptions) -> std::io::Result<Session> {
        let SpawnOptions {
            id,
            size,
            cell_size,
            shell,
            args,
            working_directory,
            env,
            scrollback,
            observer,
            replay,
            service,
            kept,
            clipboard,
            ambiguous_wide,
        } = opts;
        // a pane coming back under its full-screen program has the rows it
        // had: no editor takes them, and the program sees no size change
        let size = match (&service, &kept) {
            (Some(ServiceTarget::Attach(_)), Some(k)) => GridSize {
                rows: k.start_rows(size.cols, size.rows),
                ..size
            },
            _ => size,
        };

        let window_size = WindowSize {
            num_lines: size.rows as u16,
            num_cols: size.cols as u16,
            cell_width: cell_size.0,
            cell_height: cell_size.1,
        };
        let window_size_shared = Arc::new(Mutex::new(window_size));
        // a shell from before this window: used, even when the markers of
        // its commands have left the service's ring
        let reattached = matches!(service, Some(ServiceTarget::Attach(_)));

        // the shell: a local PTY, or one owned by the session service
        let (backend, writer, source) = match service {
            None => {
                let pty = open_pty(
                    shell.as_deref(),
                    &args,
                    working_directory,
                    env,
                    window_size,
                    id,
                )?;
                let writer = PtyWriter::File(Arc::new(pty_writer(&pty)?));
                let source = Source::Pty(pty_reader(&pty)?, pty_pid(&pty));
                (Backend::Local(pty), writer, source)
            }
            Some(target) => {
                // started (or reattached) by the reader thread, so the
                // window never waits on the service (starting it, a stalled
                // one); input is held until it answered, and a failure is
                // reported as `SessionEvent::ServiceFailed`
                let cwd = working_directory.clone();
                let open: Open = Box::new(move || {
                    // a pane split from a reattached shell starts in the
                    // directory the service knows (`known_cwd`) instead of
                    // at the fallback
                    let o = crate::service::open(
                        target,
                        shell.as_deref(),
                        args,
                        working_directory.as_deref(),
                        env,
                        window_size,
                    )?;
                    let session = o.session;
                    Ok(Opened {
                        stream: o.stream,
                        session,
                        known_cwd: o.known_cwd,
                        from: o.from,
                        resume: Box::new(move |from| crate::service::resume(session, from)),
                    })
                });
                let conn = Arc::new(Mutex::new(ServiceConn::connecting()));
                // a reattach knows its id already (the layout keeps it while
                // the service answers); a new shell's comes with the answer
                let session = Arc::new(AtomicU64::new(match target {
                    ServiceTarget::Attach(id) => id,
                    ServiceTarget::Spawn => 0,
                }));
                let closed = Arc::new(AtomicBool::new(false));
                let settled = Arc::new(AtomicBool::new(false));
                let muted = Arc::new(AtomicBool::new(true));
                let job = Arc::new(Mutex::new(None));
                let hidden = Arc::new(AtomicBool::new(false));
                let writer = PtyWriter::Service {
                    conn: conn.clone(),
                    session: session.clone(),
                    muted: muted.clone(),
                };
                let source = Source::Service {
                    open,
                    target,
                    cwd,
                    session_id: session.clone(),
                    closed: closed.clone(),
                    settled: settled.clone(),
                    muted,
                    job: job.clone(),
                    hidden: hidden.clone(),
                    conn: conn.clone(),
                    kept: kept
                        .filter(|_| matches!(target, ServiceTarget::Attach(_)))
                        .map(Box::new),
                };
                let conn = conn.clone();
                (
                    Backend::Service {
                        session,
                        closed,
                        conn,
                        settled,
                        job,
                        hidden,
                    },
                    writer,
                    source,
                )
            }
        };

        let term_config = TermConfig {
            scrolling_history: scrollback,
            // programs opt in with `CSI > flags u`; the app's key encoder
            // follows the resulting mode bits
            kitty_keyboard: true,
            xtversion: format!("Rift({})", env!("CARGO_PKG_VERSION")),
            osc52: clipboard.osc52(),
            ambiguous_wide,
            ..TermConfig::default()
        };
        let replier = Replier::new(writer.clone());
        let term = Term::new(
            term_config,
            &size,
            EventProxy {
                pane: id,
                observer: observer.clone(),
                writer: replier.clone(),
                window_size: window_size_shared.clone(),
            },
        );
        let term = Arc::new(FairMutex::new(term));
        let dirty = Arc::new(AtomicBool::new(false));
        let saw_output = Arc::new(AtomicBool::new(false));
        let mut store = BlockStore::default();
        if reattached {
            store.note_activity();
        }
        let blocks = Arc::new(Mutex::new(store));
        let scrolled = Arc::new(AtomicU64::new(0));
        let generation = Arc::new(AtomicU64::new(0));
        let cwd = Arc::new(Mutex::new(None));
        let cwd_remote = Arc::new(AtomicBool::new(false));
        let user_vars = UserVars::default();
        let widgets = Widgets::default();
        let progress = ProgressSlot::default();
        let graphics = Arc::new(Mutex::new(Graphics::default()));
        let ctx = FeedCtx {
            term: term.clone(),
            scrolled: scrolled.clone(),
            generation: generation.clone(),
            scrollback,
            blocks: blocks.clone(),
            cwd: cwd.clone(),
            cwd_remote: cwd_remote.clone(),
            cwd_host: Mutex::new(None),
            user_vars: user_vars.clone(),
            widgets: widgets.clone(),
            progress: progress.clone(),
            graphics: graphics.clone(),
            window_size: window_size_shared.clone(),
            reply: Some(replier),
            notify: Some((id, observer.clone())),
            marks: Mutex::default(),
        };

        // replay a saved transcript before the reader thread starts, so the
        // shell's first prompt lands below it
        if let Some((bytes, meta)) = replay.filter(|(b, _)| !b.is_empty()) {
            let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
            let mut scanner = OscScanner::new();
            feed(&mut parser, &mut scanner, &bytes, &ctx);
            blocks.lock().unwrap().mark_restored(&meta);
        }

        let exit = ExitCell::default();
        let ctx = Arc::new(ctx);
        let writer = InputQueue::new(writer, {
            let observer = observer.clone();
            move |m| observer(id, SessionEvent::Notice(m))
        });
        let shadow = Arc::new(Shadow::new(
            shadow_signal(&backend, &writer),
            window_size_shared.clone(),
        ));
        let reader = Reader {
            id,
            exit: exit.clone(),
            ctx: ctx.clone(),
            shadow: shadow.clone(),
            dirty: dirty.clone(),
            saw_output: saw_output.clone(),
            observer: observer.clone(),
        };
        std::thread::Builder::new()
            .name(format!("pty-reader-{id}"))
            .spawn(move || reader.run(source))
            .expect("spawn pty reader");

        Ok(Session {
            id,
            term,
            blocks,
            graphics,
            scrolled,
            cwd,
            cwd_remote,
            user_vars,
            widgets,
            progress,
            dirty,
            saw_output,
            generation,
            inputs: AtomicU64::new(0),
            read_only: AtomicBool::new(false),
            refused: AtomicBool::new(false),
            exit,
            writer,
            backend,
            size,
            window_size: window_size_shared,
            scrollback,
            shadow,
            feed_ctx: ctx,
            observer,
            ambiguous_wide: AtomicBool::new(ambiguous_wide),
        })
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    /// The cell size in pixels the shell was last told (TIOCSWINSZ).
    pub fn cell_size(&self) -> (u16, u16) {
        let ws = *self.window_size.lock().unwrap();
        (ws.cell_width, ws.cell_height)
    }

    /// Id of the shell in the session service (durable sessions); `None`
    /// for a local shell, and for a new one until the service answered.
    pub fn service_id(&self) -> Option<u64> {
        match &self.backend {
            Backend::Service { session, .. } => {
                Some(session.load(Ordering::SeqCst)).filter(|&id| id != 0)
            }
            Backend::Local(_) => None,
        }
    }

    /// Input held for a service that never answered (typed after the
    /// failure was reported, before the window replaced this session).
    pub fn take_held_input(&self) -> Vec<u8> {
        match &self.backend {
            Backend::Service { conn, .. } => {
                let mut c = conn.lock().unwrap();
                if c.stream.is_some() {
                    return Vec::new();
                }
                c.pending.replace(Vec::new()).unwrap_or_default()
            }
            Backend::Local(_) => Vec::new(),
        }
    }

    /// Not waiting on the session service any more (or never was): a quit
    /// waits for this so a shell the service is just starting is ended,
    /// not left running unseen.
    pub fn settled(&self) -> bool {
        match &self.backend {
            Backend::Service { settled, .. } => settled.load(Ordering::SeqCst),
            Backend::Local(_) => true,
        }
    }

    /// The shell runs in the session service (or is being started there).
    pub fn durable(&self) -> bool {
        matches!(self.backend, Backend::Service { .. })
    }

    /// This durable pane's scrollback for the window that reattaches its
    /// shell after an update restart ([`crate::kept`]); None for a local
    /// shell. The primary screen's, also under a full-screen program.
    pub fn kept_scrollback(&self) -> Option<crate::kept::Kept> {
        if !self.durable() {
            return None;
        }
        // under the term lock, as a resize remaps the marks
        let term = self.term.lock();
        let marks = self.feed_ctx.marks.lock().unwrap();
        Some(kept_of_term(
            &term,
            &marks,
            self.scrolled.load(Ordering::Acquire),
        ))
    }

    /// The working directory when it is on this machine: `None` while the
    /// shell reports a directory on another host (ssh), so nothing reads the
    /// local disk at a remote path. Display uses `cwd` itself.
    pub fn local_cwd(&self) -> Option<String> {
        if self.cwd_remote.load(Ordering::Relaxed) {
            return None;
        }
        self.cwd.lock().unwrap().clone()
    }

    /// The working directory when it is on another host (an ssh session
    /// with Rift's remote hooks): a path there, never read locally.
    pub fn remote_cwd(&self) -> Option<String> {
        if !self.cwd_remote.load(Ordering::Relaxed) {
            return None;
        }
        self.cwd.lock().unwrap().clone()
    }

    /// Write user input to the shell (refused while the pane is
    /// read-only: see [`Session::take_refused`]).
    pub fn write(&self, bytes: &[u8]) {
        if self.read_only() {
            if !bytes.is_empty() {
                self.refused.store(true, Ordering::Release);
            }
            return;
        }
        // keys typed into a running command are read by the shell's next
        // prompt: its echo there is not a background job's output. A lock
        // held by this thread (or the reader for a moment) skips the note;
        // the next key makes it.
        if let Ok(mut blocks) = self.blocks.try_lock() {
            blocks.note_input();
        }
        self.write_scroll(bytes);
    }

    /// Input that only scrolls (the wheel as arrow keys on the alternate
    /// screen): goes even to a read-only pane.
    pub fn write_scroll(&self, bytes: &[u8]) {
        // what the program answers belongs on the Term, at once
        self.shadow.on_input();
        self.inputs.fetch_add(1, Ordering::AcqRel);
        self.writer.write(bytes);
    }

    /// Refuse (or accept again) user input.
    pub fn set_read_only(&self, on: bool) {
        self.read_only.store(on, Ordering::Release);
    }

    pub fn read_only(&self) -> bool {
        self.read_only.load(Ordering::Acquire)
    }

    /// Whether input was refused since the last call (the app's toast).
    pub fn take_refused(&self) -> bool {
        self.refused.swap(false, Ordering::AcqRel)
    }

    /// `None` while the shell runs; `Some(code)` once it ended (`None`
    /// inside: no code known).
    pub fn exit_status(&self) -> Option<Option<i32>> {
        self.exit.get()
    }

    /// The pane stopped because the session service stopped answering: its
    /// shell may still run there (reattach it; never hang it up).
    pub fn service_lost(&self) -> bool {
        self.exit.lost()
    }

    /// Write a report the terminal sends on its own (focus in/out, mouse
    /// reports, window and colour replies): not the user answering the
    /// program, so `inputs` stays as it is.
    pub fn write_report(&self, bytes: &[u8]) {
        self.writer.write(bytes);
    }

    /// How many times user input was written (see `inputs`).
    pub fn inputs(&self) -> u64 {
        self.inputs.load(Ordering::Acquire)
    }

    /// Foreground job other than the shell (name may be empty when the OS
    /// will not tell): `vim`, `cargo build`, `ssh`, …
    fn foreground_job(&self) -> Option<String> {
        match &self.backend {
            Backend::Local(pty) => foreground_job_name(pty),
            Backend::Service { job, .. } => job.lock().unwrap().clone(),
        }
    }

    /// True while a job other than the shell runs in the foreground — the
    /// quit/close warning checks this, so it works without shell integration.
    pub fn has_foreground_job(&self) -> bool {
        self.foreground_job().is_some()
    }

    /// Executable name of the foreground job, if any.
    pub fn foreground_job_name(&self) -> Option<String> {
        self.foreground_job().filter(|n| !n.is_empty())
    }

    /// The running program reads a line without echo (a password prompt):
    /// ECHO off in canonical mode on the PTY's line discipline; a durable
    /// session reports it from the service.
    pub fn input_hidden(&self) -> bool {
        match &self.backend {
            Backend::Local(pty) => input_hidden(pty),
            Backend::Service { hidden, .. } => hidden.load(Ordering::Acquire),
        }
    }

    /// The visible screen's rows as text, top to bottom, without trailing
    /// blanks (what an agent's menu is read from).
    pub fn screen_rows(&self) -> Vec<String> {
        use alacritty_terminal::grid::Dimensions;
        use alacritty_terminal::index::{Column, Line};
        use alacritty_terminal::term::cell::Flags;
        let term = self.term.lock();
        let grid = term.grid();
        (0..grid.screen_lines() as i32)
            .map(|l| {
                let row = &grid[Line(l)];
                let mut s = String::new();
                for c in 0..grid.columns() {
                    let cell = &row[Column(c)];
                    if cell
                        .flags
                        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                    {
                        continue;
                    }
                    s.push(cell.c);
                    s.extend(cell.zerowidth().unwrap_or_default());
                }
                s.truncate(s.trim_end().len());
                s
            })
            .collect()
    }

    /// The screen's last non-empty line at or above the cursor and at or
    /// below absolute line `from` (a running command's latest output: `from`
    /// is its first output line, so the prompt and the command are never
    /// taken), without trailing blanks; empty while a full-screen program
    /// (the alternate screen) runs.
    pub fn last_line(&self, from: Option<rift_core::block::AbsLine>) -> String {
        use alacritty_terminal::index::{Column, Line};
        use alacritty_terminal::term::TermMode;
        use alacritty_terminal::term::cell::Flags;
        let term = self.term.lock();
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return String::new();
        }
        let grid = term.grid();
        let scrolled = self.scrolled.load(Ordering::Acquire) as i64;
        // the first screen row that may hold output
        let top = from.map_or(0, |f| (f as i64 - scrolled).max(0));
        let mut line = grid.cursor.point.line.0;
        while line >= 0 && i64::from(line) >= top {
            let row = &grid[Line(line)];
            let mut s = String::new();
            for c in 0..grid.columns() {
                let cell = &row[Column(c)];
                if !cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    s.push(cell.c);
                }
            }
            let t = s.trim();
            if !t.is_empty() {
                return t.to_string();
            }
            line -= 1;
        }
        String::new()
    }

    /// Text of the cursor's row up to the cursor (password prompt check).
    pub fn cursor_line_text(&self) -> String {
        use alacritty_terminal::index::Column;
        let term = self.term.lock();
        let grid = term.grid();
        let cursor = grid.cursor.point;
        let row = &grid[cursor.line];
        let end = cursor.column.0.min(grid.columns());
        (0..end).map(|c| row[Column(c)].c).collect()
    }

    /// The pane was closed on purpose: end a service-owned shell too (a
    /// local shell ends when the PTY drops). Quitting the app does not
    /// call this, which is what keeps durable sessions running.
    pub fn terminate(&self) {
        if let Backend::Service {
            session, closed, ..
        } = &self.backend
        {
            // a shell the service is still starting is ended by the reader
            closed.store(true, Ordering::SeqCst);
            let session = session.load(Ordering::SeqCst);
            if session == 0 {
                return;
            }
            // on a connection of its own, off this thread: the pane's own
            // may be held by input the service is not taking
            let _ = std::thread::Builder::new()
                .name("session-kill".into())
                .spawn(move || crate::service::kill(session));
        }
    }

    /// ⌘K Clear Blocks: wipe screen, scrollback and the block list in one
    /// atomic step (no-op on the alt screen). Fully app-side — the shell is
    /// never told, so no OSC re-emission race. The abs-line invariant holds:
    /// AbsTerm counts the rows ClearMode::All pushes into history, and
    /// ClearMode::Saved drops history without pushing, so `scrolled` stays
    /// monotone and `abs = scrolled + screen_row` remains valid.
    pub fn clear_screen_and_blocks(&self) {
        // a reattach must not bring the cleared output back
        clear_all_of(
            &self.term,
            &self.scrolled,
            self.scrollback,
            &self.generation,
            &self.blocks,
            &self.graphics,
            |_| {
                if let Some(session) = self.service_id() {
                    self.writer
                        .request(crate::ipc::Request::ClearHistory { session });
                }
            },
        );
    }

    /// Change the OSC 52 policy of the running shell.
    pub fn set_clipboard_access(&self, access: ClipboardAccess) {
        self.term.lock().set_osc52(access.osc52());
    }

    /// Measure East Asian Ambiguous characters two cells wide (or one) from
    /// the shell's next output on.
    pub fn set_ambiguous_wide(&self, wide: bool) {
        self.ambiguous_wide.store(wide, Ordering::Relaxed);
        self.term.lock().set_ambiguous_wide(wide);
    }

    pub fn resize(&mut self, size: GridSize, cell_size: (u16, u16)) {
        if size == self.size || size.cols < 2 || size.rows < 2 {
            return;
        }
        self.size = size;
        let ws = WindowSize {
            num_lines: size.rows as u16,
            num_cols: size.cols as u16,
            cell_width: cell_size.0,
            cell_height: cell_size.1,
        };
        // a capture under way ends here: this signal is its way back. Held to
        // the end, so its driver cannot signal a size this one replaced
        let mut shadow = self.shadow.state.lock().unwrap();
        if let Some(c) = shadow.capture.as_mut()
            && matches!(c.phase, Phase::Diverting)
        {
            c.usable = false;
            c.return_now();
        }
        *self.window_size.lock().unwrap() = ws;
        // Resize the Term first and signal the PTY while still holding the
        // term lock: the program's redraw after SIGWINCH is then parsed by the
        // reader against the new grid, never the old one.
        let mut term = self.term.lock();
        resize_term(
            &mut term,
            size,
            &self.scrolled,
            &self.blocks,
            &self.graphics,
            &self.feed_ctx.marks,
        );
        match &mut self.backend {
            Backend::Local(pty) => pty.on_resize(ws),
            // not answered yet: the reader sends `window_size` then
            Backend::Service { session, .. } if session.load(Ordering::SeqCst) == 0 => {}
            Backend::Service { session, .. } => {
                self.writer.request(crate::ipc::Request::Resize {
                    session: session.load(Ordering::SeqCst),
                    cols: ws.num_cols,
                    rows: ws.num_lines,
                    cell_w: ws.cell_width,
                    cell_h: ws.cell_height,
                });
            }
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        drop(term);
        drop(shadow);
    }

    /// Whether a capture at `size` is wanted, and when. Only a program on the
    /// alternate screen is ever signalled: it repaints everything when told
    /// its own size back, so what it drew meanwhile is not lost. On the
    /// primary screen output is history, and a line that went to the shadow
    /// is gone (measured: docs/audit/two-screens-spike).
    pub fn shadow_due(&self, size: GridSize, now: std::time::Instant) -> ShadowDue {
        use alacritty_terminal::term::TermMode;
        if size == self.size || size.cols < 2 || size.rows < 2 || !self.settled() {
            return ShadowDue::No;
        }
        if self.feed_ctx.reply.as_ref().is_some_and(Replier::muted) {
            return ShadowDue::No;
        }
        if !self.term.lock().mode().contains(TermMode::ALT_SCREEN) {
            // the program left: its picture and its refusal go with it
            let mut s = self.shadow.state.lock().unwrap();
            s.frame = None;
            s.blocked_until = None;
            return ShadowDue::No;
        }
        let generation = self.generation.load(Ordering::Acquire);
        let s = self.shadow.state.lock().unwrap();
        if s.capture.is_some() {
            return ShadowDue::No;
        }
        if s.frame
            .as_ref()
            .is_some_and(|f| f.size == size && f.generation == generation)
        {
            return ShadowDue::No;
        }
        let mut due = s.last_output + SHADOW_QUIET;
        if let Some(at) = s.last_start {
            due = due.max(at + SHADOW_EVERY);
        }
        if let Some(until) = s.blocked_until {
            due = due.max(until);
        }
        if now >= due {
            ShadowDue::Now
        } else {
            ShadowDue::At(due)
        }
    }

    /// Ask the program to draw at `size` and keep what it draws: one PTY has
    /// one size, so this signals `size` with TIOCSWINSZ only, sends the
    /// redraw to an empty [`Term`] of that size, and signals the pane's own
    /// size back, whose redraw lands on the real Term as any output does. The
    /// real Term and its scrollback are never touched. Returns at once; false
    /// when [`Self::shadow_due`] would not say `Now`.
    pub fn shadow_capture(&self, size: GridSize) -> bool {
        let now = std::time::Instant::now();
        if !matches!(self.shadow_due(size, now), ShadowDue::Now) {
            return false;
        }
        let ambiguous_wide = self.ambiguous_wide.load(Ordering::Relaxed);
        let (wrap, cursor, colors) = {
            use alacritty_terminal::term::TermMode;
            let term = self.term.lock();
            let mode = term.mode();
            (
                mode.contains(TermMode::LINE_WRAP),
                mode.contains(TermMode::SHOW_CURSOR),
                *term.colors(),
            )
        };
        let Some(reply) = self.feed_ctx.reply.clone() else {
            return false;
        };
        let ws = {
            let mut ws = *self.window_size.lock().unwrap();
            ws.num_cols = size.cols as u16;
            ws.num_lines = size.rows as u16;
            ws
        };
        let mut term = Term::new(
            TermConfig {
                scrolling_history: 0,
                kitty_keyboard: true,
                xtversion: format!("Rift({})", env!("CARGO_PKG_VERSION")),
                ambiguous_wide,
                ..TermConfig::default()
            },
            &size,
            ShadowListener {
                pane: self.id,
                writer: reply,
                window_size: ws,
            },
        );
        let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
        // the modes a redraw takes for granted: an empty alternate screen,
        // a cursor the program hid long ago stays hidden, its palette
        parser.advance(&mut term, b"\x1b[?1049h");
        if !cursor {
            parser.advance(&mut term, b"\x1b[?25l");
        }
        if !wrap {
            parser.advance(&mut term, b"\x1b[?7l");
        }
        for i in 0..alacritty_terminal::term::color::COUNT {
            if let Some(rgb) = colors[i] {
                vte::ansi::Handler::set_color(&mut term, i, rgb);
            }
        }
        {
            let mut s = self.shadow.state.lock().unwrap();
            if s.capture.is_some() || s.closed {
                return false;
            }
            s.last_start = Some(now);
            s.stats.started += 1;
            s.capture = Some(Capture {
                size,
                term,
                parser,
                scanner: OscScanner::new(),
                held: Vec::new(),
                started: now,
                last_byte: None,
                phase: Phase::Diverting,
                usable: true,
                back_generation: 0,
            });
            self.shadow.signal(&mut s, ws);
        }
        let shadow = self.shadow.clone();
        let ctx = self.feed_ctx.clone();
        let observer = self.observer.clone();
        let dirty = self.dirty.clone();
        let id = self.id;
        let spawned = std::thread::Builder::new()
            .name(format!("pty-shadow-{id}"))
            .spawn(move || shadow.drive(id, &ctx, &observer, &dirty));
        if spawned.is_err() {
            // nothing will end it: back to the pane's own size now
            let mut s = self.shadow.state.lock().unwrap();
            s.capture = None;
            let own = *self.window_size.lock().unwrap();
            self.shadow.signal(&mut s, own);
            return false;
        }
        true
    }

    /// Read the capture at `size`, if one is held, with how long the pane's
    /// output has moved on from it (`None`: it is the program's latest).
    pub fn shadow_frame<R>(
        &self,
        size: GridSize,
        read: impl FnOnce(&Term<ShadowListener>, u64, Option<std::time::Duration>) -> R,
    ) -> Option<R> {
        let generation = self.generation.load(Ordering::Acquire);
        let mut s = self.shadow.state.lock().unwrap();
        let f = s.frame.as_mut().filter(|f| f.size == size)?;
        let behind = if f.generation == generation {
            f.behind_since = None;
            None
        } else {
            let since = *f.behind_since.get_or_insert_with(std::time::Instant::now);
            Some(since.elapsed())
        };
        Some(read(&f.term, f.seq, behind))
    }

    /// No phone wants a capture any more: let go of the one held.
    pub fn shadow_clear(&self) {
        self.shadow.state.lock().unwrap().frame = None;
    }

    /// How this pane's captures went.
    pub fn shadow_stats(&self) -> ShadowStats {
        self.shadow.state.lock().unwrap().stats
    }
}

/// Output quiet this long before a capture: a program in mid-redraw is left alone.
pub const SHADOW_QUIET: std::time::Duration = std::time::Duration::from_millis(250);
/// At most one capture per pane this often.
pub const SHADOW_EVERY: std::time::Duration = std::time::Duration::from_secs(2);
/// A program that does not redraw when told its size is not asked again for this long.
pub const SHADOW_BLOCKED: std::time::Duration = std::time::Duration::from_secs(60);
/// A redraw is over once no byte came for this long.
const SHADOW_SETTLED: std::time::Duration = std::time::Duration::from_millis(100);
/// No redraw in this long: the program ignores SIGWINCH. Also the longest a
/// capture holds the program at the other size.
const SHADOW_NO_ANSWER: std::time::Duration = std::time::Duration::from_secs(1);
/// More than this is no redraw: the capture ends and all of it goes on the Term.
const SHADOW_MAX_BYTES: usize = 4 << 20;
const SHADOW_TICK: std::time::Duration = std::time::Duration::from_millis(5);

/// Whether and when a capture is wanted ([`Session::shadow_due`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowDue {
    No,
    At(std::time::Instant),
    Now,
}

/// What a pane's captures came to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShadowStats {
    /// Started: each one signals the program twice.
    pub started: u64,
    /// Ended with a frame.
    pub framed: u64,
    /// Given up because of a shell marker, leaving the alternate screen or too
    /// much output.
    pub aborted: u64,
    /// The program did not answer: it is blocked for [`SHADOW_BLOCKED`].
    pub no_answer: u64,
    /// TIOCSWINSZ signals sent by captures.
    pub signals: u64,
}

/// The shadow Term's events: only what the program waits for is answered.
pub struct ShadowListener {
    pane: PaneId,
    writer: Replier,
    window_size: WindowSize,
}

impl EventListener for ShadowListener {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(s) => self.writer.reply(s.as_bytes()),
            Event::ColorRequest(index, format) => {
                if let Some(rgb) = default_color(self.pane, index) {
                    self.writer.reply(format(rgb).as_bytes());
                }
            }
            Event::TextAreaSizeRequest(format) => {
                self.writer.reply(format(self.window_size).as_bytes());
            }
            _ => {}
        }
    }
}

enum Phase {
    /// The program was told the other size: its output comes here.
    Diverting,
    /// It was told its own size back: its output is the Term's again.
    Returning {
        since: std::time::Instant,
        answered: bool,
    },
}

struct Capture {
    size: GridSize,
    term: Term<ShadowListener>,
    parser: vte::ansi::Processor,
    scanner: OscScanner,
    /// Everything diverted, for the Term if this turns out to be no redraw.
    held: Vec<u8>,
    started: std::time::Instant,
    last_byte: Option<std::time::Instant>,
    phase: Phase,
    /// A frame may come of it.
    usable: bool,
    /// The pane's generation when the program was told its size back.
    back_generation: u64,
}

impl Capture {
    fn return_now(&mut self) {
        self.phase = Phase::Returning {
            since: std::time::Instant::now(),
            answered: false,
        };
    }

    /// The program ended the screen this capture is about.
    fn left_alt_screen(&self, new: usize) -> bool {
        let from = self.held.len().saturating_sub(new + 16);
        let tail = &self.held[from..];
        [&b"\x1b[?1049l"[..], b"\x1b[?1047l", b"\x1b[?47l", b"\x1bc"]
            .iter()
            .any(|seq| tail.windows(seq.len()).any(|w| w == *seq))
    }
}

struct ShadowFrame {
    size: GridSize,
    term: Term<ShadowListener>,
    /// Numbers the captures of this pane.
    seq: u64,
    /// The pane's generation when it was taken.
    generation: u64,
    /// When a reader first found the pane's output past it.
    behind_since: Option<std::time::Instant>,
}

struct ShadowState {
    capture: Option<Capture>,
    frame: Option<ShadowFrame>,
    last_output: std::time::Instant,
    last_start: Option<std::time::Instant>,
    blocked_until: Option<std::time::Instant>,
    closed: bool,
    stats: ShadowStats,
}

/// How a capture reaches the program.
enum ShadowSignal {
    /// Unix only: ConPTY has no way to tell a size without resizing.
    #[cfg(unix)]
    Local(Arc<std::fs::File>),
    Service {
        session: Arc<AtomicU64>,
        queue: std::sync::mpsc::Sender<Queued>,
    },
}

fn shadow_signal(backend: &Backend, writer: &InputQueue) -> Option<ShadowSignal> {
    Some(match backend {
        #[cfg(unix)]
        Backend::Local(pty) => ShadowSignal::Local(Arc::new(pty.file().try_clone().ok()?)),
        #[cfg(windows)]
        Backend::Local(_) => return None,
        Backend::Service { session, .. } => ShadowSignal::Service {
            session: session.clone(),
            queue: writer.tx.clone(),
        },
    })
}

/// A pane's captures: the state the reader, the app and the capture's own
/// thread share. Lock order: this, then the term lock; never the other way.
struct Shadow {
    state: Mutex<ShadowState>,
    signal: Option<ShadowSignal>,
    window_size: Arc<Mutex<WindowSize>>,
}

/// What became of a chunk of output ([`Shadow::on_output`]).
enum Output {
    Real,
    Diverted,
    Flush(Vec<u8>),
}

impl Shadow {
    fn new(signal: Option<ShadowSignal>, window_size: Arc<Mutex<WindowSize>>) -> Self {
        Self {
            state: Mutex::new(ShadowState {
                capture: None,
                frame: None,
                last_output: std::time::Instant::now(),
                last_start: None,
                blocked_until: None,
                closed: false,
                stats: ShadowStats::default(),
            }),
            signal,
            window_size,
        }
    }

    /// Tell the program a size; nothing else changes. The caller holds the
    /// state, so two signals never cross.
    fn signal(&self, state: &mut ShadowState, ws: WindowSize) {
        state.stats.signals += 1;
        match &self.signal {
            #[cfg(unix)]
            Some(ShadowSignal::Local(file)) => {
                use alacritty_terminal::tty::ToWinsize;
                use std::os::fd::AsRawFd;
                let win = ws.to_winsize();
                // never `Pty::on_resize`: it ends the process when this fails,
                // and a pane may be closing
                unsafe { libc::ioctl(file.as_raw_fd(), libc::TIOCSWINSZ, &win as *const _) };
            }
            Some(ShadowSignal::Service { session, queue }) => {
                let _ = queue.send(Queued::Request(crate::ipc::Request::Resize {
                    session: session.load(Ordering::SeqCst),
                    cols: ws.num_cols,
                    rows: ws.num_lines,
                    cell_w: ws.cell_width,
                    cell_h: ws.cell_height,
                }));
            }
            None => {}
        }
    }

    fn signal_back(&self, state: &mut ShadowState) {
        let own = *self.window_size.lock().unwrap();
        self.signal(state, own);
    }

    /// A chunk of the program's output, on the reader thread.
    fn on_output(&self, bytes: &[u8]) -> Output {
        let mut guard = self.state.lock().unwrap();
        let s = &mut *guard;
        let now = std::time::Instant::now();
        let Some(c) = s.capture.as_mut() else {
            s.last_output = now;
            return Output::Real;
        };
        if let Phase::Returning { answered, .. } = &mut c.phase {
            *answered = true;
            s.last_output = now;
            return Output::Real;
        }
        c.last_byte = Some(now);
        c.held.extend_from_slice(bytes);
        let marked = c.scanner.scan(bytes).iter().any(|seg| seg.marker.is_some());
        if marked || c.left_alt_screen(bytes.len()) || c.held.len() > SHADOW_MAX_BYTES {
            let held = std::mem::take(&mut c.held);
            s.capture = None;
            s.stats.aborted += 1;
            s.last_output = now;
            self.signal_back(s);
            return Output::Flush(held);
        }
        let Capture { parser, term, .. } = c;
        parser.advance(term, bytes);
        Output::Diverted
    }

    /// Someone typed: the capture ends and the answer goes on the Term.
    fn on_input(&self) {
        let mut guard = self.state.lock().unwrap();
        let s = &mut *guard;
        if let Some(c) = s.capture.as_mut()
            && matches!(c.phase, Phase::Diverting)
        {
            c.usable = false;
            c.back_generation = 0;
            c.return_now();
            self.signal_back(s);
        }
    }

    /// The pane is gone (or its program ended): what a capture held.
    fn close(&self) -> Option<Vec<u8>> {
        let mut s = self.state.lock().unwrap();
        s.closed = true;
        s.frame = None;
        s.capture.take().map(|c| c.held)
    }

    /// A capture's thread: ends the redraw, signals the size back, watches
    /// that redraw land, publishes the frame.
    fn drive(&self, id: PaneId, ctx: &FeedCtx, observer: &Observer, dirty: &AtomicBool) {
        loop {
            std::thread::sleep(SHADOW_TICK);
            let now = std::time::Instant::now();
            let mut guard = self.state.lock().unwrap();
            let s = &mut *guard;
            if s.closed {
                return;
            }
            // given up on the reader thread
            let Some(c) = s.capture.as_mut() else { return };
            match c.phase {
                Phase::Diverting => {
                    let settled = c
                        .last_byte
                        .is_some_and(|at| now.duration_since(at) >= SHADOW_SETTLED);
                    let out_of_time = now.duration_since(c.started) >= SHADOW_NO_ANSWER;
                    if !settled && !out_of_time {
                        continue;
                    }
                    c.usable = settled;
                    c.back_generation = ctx.generation.load(Ordering::Acquire);
                    c.return_now();
                    let silent = c.last_byte.is_none();
                    self.signal_back(s);
                    if silent {
                        // no redraw, so none to wait for
                        s.capture = None;
                        s.blocked_until = Some(now + SHADOW_BLOCKED);
                        s.stats.no_answer += 1;
                        return;
                    }
                }
                Phase::Returning { since, answered } => {
                    let waited = now.duration_since(since);
                    if !answered {
                        if waited < SHADOW_NO_ANSWER {
                            continue;
                        }
                        // no redraw for the Term: what the program printed
                        // meanwhile was never one either, and the Term needs it
                        let held = std::mem::take(&mut c.held);
                        s.capture = None;
                        s.blocked_until = Some(now + SHADOW_BLOCKED);
                        s.stats.no_answer += 1;
                        drop(guard);
                        let mut parser = vte::ansi::Processor::new();
                        let mut scanner = OscScanner::new();
                        feed(&mut parser, &mut scanner, &held, ctx);
                        dirty.store(true, Ordering::Release);
                        observer(id, SessionEvent::Wakeup);
                        return;
                    }
                    let quiet = now.duration_since(s.last_output) >= SHADOW_SETTLED;
                    if !quiet && waited < SHADOW_NO_ANSWER {
                        continue;
                    }
                    let mut c = s.capture.take().expect("matched above");
                    if !c.usable {
                        return;
                    }
                    if c.parser.sync_bytes_count() > 0 {
                        c.parser.stop_sync(&mut c.term);
                    }
                    s.stats.framed += 1;
                    s.frame = Some(ShadowFrame {
                        size: c.size,
                        term: c.term,
                        seq: s.stats.framed,
                        // still talking: the frame is behind already
                        generation: if quiet {
                            ctx.generation.load(Ordering::Acquire)
                        } else {
                            c.back_generation
                        },
                        behind_since: None,
                    });
                    return;
                }
            }
        }
    }
}

/// Resize the grid and keep every abs anchor on its line. Row-count changes
/// move lines between history and screen; keep `abs = scrolled + screen_row`
/// coherent by folding the history delta into the counter (same term lock
/// the reader thread uses).
/// What a window keeps of a durable pane for an update restart
/// ([`crate::kept`]): the primary screen's lines, the marks, the modes and,
/// on the alternate screen, its picture with the output position parsed.
fn kept_of_term(
    term: &Term<EventProxy>,
    marks: &crate::kept::Marks,
    scrolled: u64,
) -> crate::kept::Kept {
    let alt_since = marks.alt_since();
    crate::kept::Kept {
        marks: marks.list(),
        lines: crate::kept::lines_of(term.primary_grid(), scrolled),
        modes: crate::kept::modes_of(*term.mode()),
        alt_since,
        alt_screen: alt_since.map(|_| crate::kept::KeptScreen {
            seq: marks.parsed(),
            cols: term.columns(),
            rows: term.screen_lines(),
            draw: crate::kept::screen_of(term.grid()),
        }),
    }
}

fn resize_term(
    term: &mut Term<EventProxy>,
    size: GridSize,
    scrolled: &AtomicU64,
    blocks: &Mutex<BlockStore>,
    graphics: &Mutex<Graphics>,
    marks: &Mutex<crate::kept::Marks>,
) {
    use alacritty_terminal::term::TermMode;
    // Anchors live on the primary grid, which `Term::resize` reflows even
    // while a full-screen program shows the alt grid. Measure the primary:
    // leave the alt screen (no reset that way), resize, then return and put
    // the alt contents back (entering the alt screen clears it).
    let alt = term
        .mode()
        .contains(TermMode::ALT_SCREEN)
        .then(|| term.grid().clone());
    if alt.is_some() {
        term.swap_alt();
    }
    let old = RowLayout::of(term);
    term.resize(size);
    let new = RowLayout::of(term);
    if let Some(mut grid) = alt {
        grid.resize(false, size.rows, size.cols);
        term.swap_alt();
        *term.grid_mut() = grid;
    }
    let cur = scrolled.load(Ordering::Acquire);
    // the grid's top row keeps its abs index: base = scrolled - history
    let base = cur.saturating_sub(old.history as u64);
    let new_base = base + old.trimmed_by(&new);
    scrolled.store(new_base + new.history as u64, Ordering::Release);
    // reflow (column change) re-wraps lines and moves every row after
    // them; carry the block anchors along (same lock order as the reader)
    // a prompt redrawn after this keeps its pre-resize anchor
    blocks.lock().unwrap().note_resize();
    if old.cols != new.cols || old.starts.len() != new.starts.len() {
        blocks
            .lock()
            .unwrap()
            .remap(|abs, col| old.map(&new, base, new_base, abs, col));
        graphics
            .lock()
            .unwrap()
            .remap(|abs, col| old.map(&new, base, new_base, abs, col));
        // a boundary follows the end of the line above it
        marks.lock().unwrap().remap(|b| match b.checked_sub(1) {
            Some(last) => old.map(&new, base, new_base, last, old.cols).0 + 1,
            None => 0,
        });
    }
}

/// Grid rows grouped into logical (unwrapped) lines, for mapping positions
/// across a reflowing resize. Row 0 is the oldest history row.
struct RowLayout {
    /// First row of each logical line.
    starts: Vec<usize>,
    rows: usize,
    cols: usize,
    history: usize,
    /// Logical line holding the cursor (the fixed point between layouts:
    /// reflow may trim history at the top and blank rows at the bottom).
    cursor_line: usize,
    /// The cursor's row, counted from the oldest history row.
    cursor_row: usize,
}

impl RowLayout {
    fn of(term: &Term<EventProxy>) -> Self {
        use alacritty_terminal::term::cell::Flags;
        let grid = term.grid();
        let history = grid.history_size();
        let rows = history + grid.screen_lines();
        let cols = grid.columns();
        let mut starts = Vec::new();
        let mut wrapped = false;
        for r in 0..rows {
            if !wrapped {
                starts.push(r);
            }
            let row = &grid[Line(r as i32 - history as i32)];
            wrapped = cols > 0 && row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        }
        let cursor_row = history + grid.cursor.point.line.0.max(0) as usize;
        let cursor_line = starts
            .partition_point(|&s| s <= cursor_row)
            .saturating_sub(1);
        Self {
            starts,
            rows,
            cols,
            history,
            cursor_line,
            cursor_row,
        }
    }

    /// Rows dropped from the top of the history by the resize to `new`: a
    /// row-only shrink with a full history pushes screen rows into it and
    /// drops as many of its oldest, so the new grid's top row is a later
    /// line. Without a reflow every row stays itself, so the rows above the
    /// cursor's tell (not the start of its logical line: a long wrapped
    /// line may lose its first rows with the history); a reflow (column
    /// change) re-wraps them, and counts as none (the top row keeps its abs
    /// index).
    fn trimmed_by(&self, new: &RowLayout) -> u64 {
        if self.cols != new.cols {
            return 0;
        }
        self.cursor_row.saturating_sub(new.cursor_row) as u64
    }

    /// Where (`abs`, `col`) in this layout (top row at `base`) lands in
    /// `new` (top row at `new_base`).
    fn map(&self, new: &RowLayout, base: u64, new_base: u64, abs: u64, col: usize) -> (u64, usize) {
        let Some(row) = abs.checked_sub(base).map(|r| r as usize) else {
            return (abs, col); // already trimmed out of history
        };
        if row >= self.rows || self.starts.is_empty() || new.starts.is_empty() {
            // past the grid (an end anchor on the last row): keep its
            // distance from the bottom
            let shifted = row as i64 + new.rows as i64 - self.rows as i64;
            return (new_base + shifted.max(0) as u64, col);
        }
        let line = self.starts.partition_point(|&s| s <= row) - 1;
        let offset = (row - self.starts[line]) * self.cols + col.min(self.cols);
        let target = line as i64 - self.cursor_line as i64 + new.cursor_line as i64;
        if target < 0 {
            return (new_base.saturating_sub(1), 0); // reflow pushed it out of history
        }
        let target = target as usize;
        if target >= new.starts.len() {
            return (new_base + new.rows as u64, 0);
        }
        let start = new.starts[target];
        let next = new.starts.get(target + 1).copied().unwrap_or(new.rows);
        let cols = new.cols.max(1);
        let sub = (offset / cols).min(next - start - 1);
        (
            new_base + (start + sub) as u64,
            (offset - sub * cols).min(cols),
        )
    }
}

/// Parse one chunk of terminal output: split it at shell markers, feed each
/// piece to the VT parser, apply the markers to the block model, then prune
/// blocks that left the scrollback.
/// Shell variables from OSC 1337 SetUserVar, by name.
pub type UserVars = Arc<Mutex<std::collections::HashMap<String, String>>>;

/// A pane's shell-widget state (OSC 6973).
pub type Widgets = Arc<Mutex<rift_core::shell_widget::WidgetState>>;

/// Distinct user variables a session keeps.
const MAX_USER_VARS: usize = 32;

/// A pane's standing OSC 9;4 progress report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressReport {
    pub progress: Progress,
    /// Set while the alternate screen showed: leaving it clears the report
    /// (a full-screen program's progress ends with the program).
    pub alt: bool,
    /// Process-wide order of reports: the newest wins a tie between panes.
    pub seq: u64,
}

/// Where the reader thread keeps a pane's progress report.
pub type ProgressSlot = Arc<Mutex<Option<ProgressReport>>>;

static PROGRESS_SEQ: AtomicU64 = AtomicU64::new(0);

/// Apply one OSC 9;4 report: `None` (state 0) clears. An error or paused
/// report without a value keeps the previous one's (ConEmu).
fn set_progress(slot: &ProgressSlot, report: Option<Progress>, alt: bool) {
    let mut cur = slot.lock().unwrap();
    *cur = report.map(|mut progress| {
        if progress.pct.is_none() && progress.state != rift_core::ProgressState::Indeterminate {
            progress.pct = cur.and_then(|c| c.progress.pct);
        }
        ProgressReport {
            progress,
            alt,
            seq: PROGRESS_SEQ.fetch_add(1, Ordering::Relaxed),
        }
    });
}

/// Everything the output parser writes to (shared with the app).
struct FeedCtx {
    term: Arc<FairMutex<Term<EventProxy>>>,
    scrolled: Arc<AtomicU64>,
    /// [`Session::generation`].
    generation: Arc<AtomicU64>,
    scrollback: usize,
    blocks: Arc<Mutex<BlockStore>>,
    cwd: Arc<Mutex<Option<String>>>,
    cwd_remote: Arc<AtomicBool>,
    /// The last OSC 7's host name while it is another machine (blocks
    /// record it: two hosts' runs of one command stay apart).
    cwd_host: Mutex<Option<String>>,
    user_vars: UserVars,
    widgets: Widgets,
    progress: ProgressSlot,
    graphics: Arc<Mutex<Graphics>>,
    window_size: Arc<Mutex<WindowSize>>,
    /// PTY write-back for graphics protocol replies.
    reply: Option<Replier>,
    /// Where notification requests (OSC 9/777/99) go.
    notify: Option<(PaneId, Observer)>,
    /// Output positions and the history they had built, for an update
    /// restart ([`crate::kept`]; durable sessions only).
    marks: Mutex<crate::kept::Marks>,
}

/// Parse one chunk of terminal output: split it at shell markers, feed each
/// piece to the VT parser, apply the markers to the block and image models,
/// then prune what left the scrollback. True when the chunk held a marker.
fn feed(
    parser: &mut vte::ansi::Processor,
    scanner: &mut OscScanner,
    bytes: &[u8],
    ctx: &FeedCtx,
) -> bool {
    use alacritty_terminal::term::TermMode;
    let segments = scanner.scan(bytes);
    let mut term = ctx.term.lock();
    // the atomic is canonical: resizes adjust it (under the same term lock)
    // between chunks
    let mut scrolled_cur = ctx.scrolled.load(Ordering::Acquire);
    let mut start = 0usize;
    let mut marked = false;
    // a program's resize request (XTWINOPS 8, DECCOLM, DECSLPP) applies to
    // the grid at once, with the anchor bookkeeping of any resize; the app
    // resizes the window (and the PTY) to match after
    let resize = |term: &mut Term<EventProxy>, rows: usize, cols: usize, scrolled: &mut u64| {
        let counter = AtomicU64::new(*scrolled);
        resize_term(
            term,
            GridSize { cols, rows },
            &counter,
            &ctx.blocks,
            &ctx.graphics,
            &ctx.marks,
        );
        *scrolled = counter.load(Ordering::Acquire);
        let mut ws = ctx.window_size.lock().unwrap();
        ws.num_lines = rows as u16;
        ws.num_cols = cols as u16;
    };
    for seg in segments {
        marked |= seg.marker.is_some();
        let was_alt = term.mode().contains(TermMode::ALT_SCREEN);
        let cleared = {
            let mut abs = AbsTerm {
                term: &mut term,
                scrolled: &mut scrolled_cur,
                scrollback_cap: ctx.scrollback,
                saw_clear_saved: false,
                on_resize: Some(&resize),
            };
            parser.advance(&mut abs, &bytes[start..seg.end]);
            // a shell marker anchors to the cursor: flush a pending
            // synchronized update first
            if seg.marker.is_some() && parser.sync_bytes_count() > 0 {
                parser.stop_sync(&mut abs);
            }
            abs.saw_clear_saved
        };
        if was_alt && !term.mode().contains(TermMode::ALT_SCREEN) {
            ctx.graphics.lock().unwrap().drop_alt();
            let mut progress = ctx.progress.lock().unwrap();
            if progress.is_some_and(|p| p.alt) {
                *progress = None;
            }
        }
        if cleared {
            // `clear` ran (ED3, suppressed in AbsTerm): Warp-style soft
            // clear — blocks and scrollback stay; pad the viewport at the
            // first post-clear screen row so older content moves above the
            // fold but stays scrollable.
            ctx.blocks.lock().unwrap().set_clear_pad(scrolled_cur);
            term.selection = None;
        }
        start = seg.end;
        match seg.marker {
            Some(Marker::Graphics(body)) => {
                let cell = cell_px(&ctx.window_size);
                let out = ctx.graphics.lock().unwrap().kitty(&body, cell);
                if let (Some(reply), Some(w)) = (&out.reply, &ctx.reply) {
                    w.reply(reply.as_bytes());
                }
                if let Some(req) = out.place {
                    place_image(parser, &mut term, &mut scrolled_cur, ctx, &req);
                }
            }
            Some(Marker::InlineImage(body)) => {
                let cell = cell_px(&ctx.window_size);
                let cols = term.grid().columns();
                let rows = term.grid().screen_lines();
                let out = ctx.graphics.lock().unwrap().iterm(&body, cell, cols, rows);
                if let Some(req) = out.place {
                    place_image(parser, &mut term, &mut scrolled_cur, ctx, &req);
                }
            }
            Some(Marker::UserVar { name, value }) => {
                let mut vars = ctx.user_vars.lock().unwrap();
                // bounded: a program cannot grow the map without limit
                if vars.len() < MAX_USER_VARS || vars.contains_key(&name) {
                    vars.insert(name, value);
                }
            }
            Some(Marker::ShellWidget(msg)) => ctx.widgets.lock().unwrap().on_msg(msg),
            Some(Marker::Progress(report)) => {
                let alt = term.mode().contains(TermMode::ALT_SCREEN);
                set_progress(&ctx.progress, report, alt);
            }
            Some(Marker::Notify { title, body }) => {
                // replayed output already notified once
                let replaying = matches!(&ctx.reply, Some(w) if w.muted());
                if let (Some((id, observer)), false) = (&ctx.notify, replaying) {
                    observer(*id, SessionEvent::Notify { title, body });
                }
            }
            Some(marker) => {
                // a finished command's progress ends with it, and so do the
                // kitty keyboard flags it set on the main screen: one killed
                // before popping them left the shell getting CSI u codes (key
                // releases too), which its line editor keeps as text
                if matches!(marker, Marker::Done { .. }) {
                    *ctx.progress.lock().unwrap() = None;
                    if !term
                        .mode()
                        .contains(alacritty_terminal::term::TermMode::ALT_SCREEN)
                    {
                        vte::ansi::Handler::pop_keyboard_modes(&mut *term, u16::MAX);
                    }
                }
                if matches!(marker, Marker::Done { .. })
                    && ctx.blocks.lock().unwrap().any_running()
                    && end_partial_line(parser, &mut term, &mut scrolled_cur, ctx.scrollback)
                {
                    ctx.blocks.lock().unwrap().mark_unterminated();
                }
                // the shell prompts again after its command finished, still on
                // the alternate screen: that program died without leaving it
                // (a crash, kill -9). The prompt belongs on the main screen,
                // under the blocks; a program still running (tmux, vim) has no
                // finished block, so it is never touched
                if matches!(marker, Marker::PromptStart)
                    && term.mode().contains(TermMode::ALT_SCREEN)
                    && ctx
                        .blocks
                        .lock()
                        .unwrap()
                        .last()
                        .is_some_and(|b| matches!(b.state, rift_core::BlockState::Done { .. }))
                {
                    leave_stranded_alt(parser, &mut term, &mut scrolled_cur, ctx);
                }
                apply_marker(marker, &term, scrolled_cur, ctx)
            }
            None => {}
        }
    }
    // output the shell printed below its prompt while it waited for input:
    // a background job's (Warp background blocks). Not while a shell widget
    // runs: it draws there (fzf below the prompt, the line it accepts)
    let widget_busy = ctx.widgets.lock().unwrap().busy();
    if !widget_busy && !term.mode().contains(TermMode::ALT_SCREEN) {
        let cursor = term.grid().cursor.point;
        let line = scrolled_cur + cursor.line.0.max(0) as u64;
        let mut blocks = ctx.blocks.lock().unwrap();
        // did the output use the prompt's row after the input point?
        let used = blocks
            .last()
            .and_then(|b| b.command_start)
            .filter(|&(l, _)| l < line)
            .is_some_and(|(l, c)| {
                let row = l as i64 - scrolled_cur as i64;
                let g = term.grid();
                row >= -(g.history_size() as i64)
                    && row < g.screen_lines() as i64
                    && (c..g.columns()).any(|col| {
                        let ch = g[alacritty_terminal::index::Line(row as i32)]
                            [alacritty_terminal::index::Column(col)]
                        .c;
                        ch != ' ' && ch != '\0'
                    })
            });
        blocks.on_background_output(line, cursor.column.0, used);
    }
    // prune blocks and images that fell out of scrollback; the alt grid has
    // no history, so only the primary grid says what is still kept
    if !term.mode().contains(TermMode::ALT_SCREEN) {
        let oldest = scrolled_cur.saturating_sub(term.grid().history_size() as u64);
        ctx.blocks.lock().unwrap().prune(oldest);
        ctx.graphics.lock().unwrap().prune(oldest);
    }
    ctx.scrolled.store(scrolled_cur, Ordering::Release);
    // under the term lock: a reader of the screen sees the new value with it
    ctx.generation.fetch_add(1, Ordering::AcqRel);
    marked
}

/// Output that ended without a newline leaves the cursor mid-row when the
/// command finishes (133;D). Move to a fresh row locally (zsh PROMPT_SP
/// style, never written to the PTY) so the partial row stays inside the
/// block's output range and the next prompt does not share it. True when it
/// moved.
fn end_partial_line(
    parser: &mut vte::ansi::Processor,
    term: &mut Term<EventProxy>,
    scrolled: &mut u64,
    scrollback: usize,
) -> bool {
    use alacritty_terminal::term::TermMode;
    let cursor = &term.grid().cursor;
    if term.mode().contains(TermMode::ALT_SCREEN)
        || (cursor.point.column.0 == 0 && !cursor.input_needs_wrap)
    {
        return false;
    }
    let mut abs = AbsTerm {
        term,
        scrolled,
        scrollback_cap: scrollback,
        saw_clear_saved: false,
        on_resize: None,
    };
    parser.advance(&mut abs, b"\r\n");
    true
}

/// Undo what a full-screen program that died left on: the alternate screen
/// (back to the main one and its cursor), mouse reporting, a hidden cursor,
/// application cursor keys and keypad, and kitty keyboard flags, as `reset`
/// would for these. Through the parser, so the line count stays right.
fn leave_stranded_alt(
    parser: &mut vte::ansi::Processor,
    term: &mut Term<EventProxy>,
    scrolled: &mut u64,
    ctx: &FeedCtx,
) {
    vte::ansi::Handler::pop_keyboard_modes(&mut *term, u16::MAX);
    let mut abs = AbsTerm {
        term,
        scrolled,
        scrollback_cap: ctx.scrollback,
        saw_clear_saved: false,
        on_resize: None,
    };
    parser.advance(
        &mut abs,
        b"\x1b[?1049l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\
          \x1b[?1015l\x1b[?1004l\x1b[?1l\x1b>\x1b[?25h",
    );
    ctx.graphics.lock().unwrap().drop_alt();
    let mut progress = ctx.progress.lock().unwrap();
    if progress.is_some_and(|p| p.alt) {
        *progress = None;
    }
}

fn cell_px(ws: &Mutex<WindowSize>) -> (u32, u32) {
    let ws = *ws.lock().unwrap();
    (ws.cell_width.max(1) as u32, ws.cell_height.max(1) as u32)
}

/// Anchor an image at the cursor, then (unless asked not to) move the
/// cursor to the image's last row, one column past it — through the parser,
/// so rows scrolled into history are counted like any other output.
fn place_image(
    parser: &mut vte::ansi::Processor,
    term: &mut Term<EventProxy>,
    scrolled: &mut u64,
    ctx: &FeedCtx,
    req: &crate::graphics::PlaceRequest,
) {
    use alacritty_terminal::term::TermMode;
    let cursor = term.grid().cursor.point;
    let line = *scrolled + cursor.line.0.max(0) as u64;
    let alt = term.mode().contains(TermMode::ALT_SCREEN);
    ctx.graphics
        .lock()
        .unwrap()
        .place(req, line, cursor.column.0, alt);
    if !req.move_cursor {
        return;
    }
    let mut seq = "\n".repeat(
        req.rows
            .min(crate::graphics::MAX_CELL_SPAN)
            .saturating_sub(1),
    );
    let col = cursor
        .column
        .0
        .saturating_add(req.cols)
        .min(term.grid().columns() - 1);
    seq.push_str(&format!("\x1b[{}G", col + 1));
    let mut abs = AbsTerm {
        term,
        scrolled,
        scrollback_cap: ctx.scrollback,
        saw_clear_saved: false,
        on_resize: None,
    };
    parser.advance(&mut abs, seq.as_bytes());
}

/// What the reader blocks on: a PTY or a service socket.
trait WaitReadable {
    /// Block until there are bytes to read or `wait` passes. False on
    /// timeout; errors and hangups return true (read() reports them).
    fn wait_readable(&self, wait: std::time::Duration) -> bool;
}

#[cfg(unix)]
impl<T: std::os::fd::AsRawFd> WaitReadable for T {
    fn wait_readable(&self, wait: std::time::Duration) -> bool {
        wait_readable(self, wait)
    }
}

/// Windows: an anonymous pipe cannot be polled; peek at it until `wait`
/// passes (only while a synchronized update stalls).
#[cfg(windows)]
impl WaitReadable for File {
    fn wait_readable(&self, wait: std::time::Duration) -> bool {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Pipes::PeekNamedPipe;
        let deadline = std::time::Instant::now() + wait;
        loop {
            let mut avail = 0u32;
            // SAFETY: a valid pipe handle owned by `self`; no buffer is read
            let ok = unsafe {
                PeekNamedPipe(
                    self.as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut avail,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 || avail > 0 {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

#[cfg(windows)]
impl WaitReadable for crate::uds::UnixStream {
    fn wait_readable(&self, _: std::time::Duration) -> bool {
        match *self {}
    }
}

/// Block until `file` has bytes to read or `wait` passes. False on timeout.
#[cfg(unix)]
fn wait_readable(file: &impl std::os::fd::AsRawFd, wait: std::time::Duration) -> bool {
    let mut pfd = libc::pollfd {
        fd: file.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = wait.as_millis().min(i32::MAX as u128) as i32;
    loop {
        let r = unsafe { libc::poll(&mut pfd, 1, ms) };
        if r < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        // errors and hangups fall through to read(), which reports them
        return r != 0;
    }
}

/// This machine's hostname, read once (empty when unavailable, which makes
/// only empty and `localhost` OSC 7 hosts local).
fn local_host() -> &'static str {
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOST.get_or_init(rift_core::sys::hostname)
}

fn apply_marker(marker: Marker, term: &Term<EventProxy>, scrolled: u64, ctx: &FeedCtx) {
    let (blocks, cwd) = (&ctx.blocks, &ctx.cwd);
    let cursor = term.grid().cursor.point;
    let abs_line = scrolled + cursor.line.0.max(0) as u64;
    log::debug!(
        "marker {marker:?} at abs={abs_line} (cursor line {}, scrolled {scrolled})",
        cursor.line.0
    );
    // a shell-widget hand-off ends with a new prompt or a command
    match marker {
        Marker::PromptStart => ctx.widgets.lock().unwrap().on_prompt_start(),
        Marker::OutputStart { .. } => ctx.widgets.lock().unwrap().on_output_start(),
        _ => {}
    }
    let mut store = blocks.lock().unwrap();
    match marker {
        Marker::PromptStart => store.on_prompt_start(abs_line),
        Marker::CommandStart => store.on_command_start(abs_line, cursor.column.0),
        Marker::OutputStart { command } => {
            // the shell's own copy of the line is exact; the screen's echo
            // is not when replayed at another width (readline and zle place
            // the line for the width they ran at)
            // a second C right after the first (the user's own integration
            // sent a bare one): never one printed later by the command's own
            // output, which must not rename its block
            if let Some(line) = &command
                && store.last().and_then(|b| b.output_start) == Some(abs_line)
            {
                store.on_command_line(line);
            }
            let command = command.unwrap_or_else(|| {
                store
                    .last()
                    .and_then(|b| b.command_start)
                    .map(|(line, col)| extract_text(term, scrolled, line, col, abs_line))
                    .unwrap_or_default()
            });
            store.on_output_start(abs_line, command);
            // header context fallback for raw-passthrough commands (the app
            // stamps cwd+branch at submit; this only fills gaps)
            store.stamp_context(
                cwd.lock().unwrap().clone(),
                ctx.cwd_remote.load(Ordering::Relaxed),
                None,
            );
            store.stamp_host(ctx.cwd_host.lock().unwrap().clone());
        }
        Marker::Done { exit } => store.on_done(abs_line, exit),
        Marker::Cwd { path, host } => {
            let remote = !rift_core::host_is_local(&host, local_host());
            ctx.cwd_remote.store(remote, Ordering::Relaxed);
            *ctx.cwd_host.lock().unwrap() = remote.then_some(host);
            *cwd.lock().unwrap() = Some(path);
        }
        // handled by `feed` (they need the parser to move the cursor)
        Marker::Graphics(_)
        | Marker::InlineImage(_)
        | Marker::Notify { .. }
        | Marker::UserVar { .. }
        | Marker::Progress(_)
        | Marker::ShellWidget(_) => {}
    }
}

/// Collect trimmed text from (start_line, start_col) up to end_line
/// (exclusive of the line the cursor sits on when it is the marker line).
fn extract_text(
    term: &Term<EventProxy>,
    scrolled: u64,
    start_line: u64,
    start_col: usize,
    end_line: u64,
) -> String {
    use alacritty_terminal::term::cell::Flags;
    let grid = term.grid();
    let mut out = String::new();
    // the previous row soft-wrapped into this one: join without a space
    let mut continued = false;
    for abs in start_line..=end_line.min(start_line + 32) {
        let rel = abs as i64 - scrolled as i64;
        if rel < -(grid.history_size() as i64) || rel >= grid.screen_lines() as i64 {
            continue;
        }
        let line = Line(rel as i32);
        let row = &grid[line];
        let cols = grid.columns();
        let from = if abs == start_line { start_col } else { 0 };
        let wraps = cols > 0 && row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        let mut text = String::new();
        for c in from..cols {
            let cell = &row[Column(c)];
            // a wide character that did not fit at the row's end leaves a
            // blank there and continues on the next row
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            text.push(cell.c);
            // combining marks belong to the command (e + U+0301 is "é")
            text.extend(cell.zerowidth().unwrap_or_default());
        }
        if !wraps {
            while text.ends_with(' ') {
                text.pop();
            }
        }
        if !text.is_empty() {
            if !out.is_empty() && !continued {
                out.push(' ');
            }
            out.push_str(&text);
        }
        continued = wraps;
    }
    out.trim().to_string()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn term(cols: usize, rows: usize) -> Term<EventProxy> {
        term_with(cols, rows, TermConfig::default())
    }

    fn term_with(cols: usize, rows: usize, config: TermConfig) -> Term<EventProxy> {
        let proxy = EventProxy {
            pane: 0,
            observer: Arc::new(|_, _| {}),
            writer: Replier::new(PtyWriter::File(Arc::new(
                File::create("/dev/null").unwrap(),
            ))),
            window_size: Arc::new(Mutex::new(WindowSize {
                num_lines: rows as u16,
                num_cols: cols as u16,
                cell_width: 8,
                cell_height: 16,
            })),
        };
        Term::new(config, &GridSize { cols, rows }, proxy)
    }

    /// OSC 52 copy and read requests that reach the observer under each
    /// `ClipboardAccess`, the policy switched on the live Term.
    #[test]
    fn clipboard_access_gates_osc52() {
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let log = seen.clone();
        let proxy = EventProxy {
            pane: 0,
            observer: Arc::new(move |_, e| {
                let what = match e {
                    SessionEvent::Clipboard(text) => format!("store {text}"),
                    SessionEvent::ClipboardLoad(reply) => format!("load {}", (reply.0)("hi")),
                    _ => return,
                };
                log.lock().unwrap().push(what);
            }),
            writer: Replier::new(PtyWriter::File(Arc::new(
                File::create("/dev/null").unwrap(),
            ))),
            window_size: Arc::new(Mutex::new(WindowSize {
                num_lines: 4,
                num_cols: 20,
                cell_width: 8,
                cell_height: 16,
            })),
        };
        let mut t = Term::new(
            TermConfig::default(),
            &GridSize { cols: 20, rows: 4 },
            proxy,
        );
        let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
        // "eA==" is base64 for "x"
        let osc = b"\x1b]52;c;eA==\x07\x1b]52;c;?\x07";
        let mut run = |t: &mut Term<EventProxy>, access: ClipboardAccess| {
            t.set_osc52(access.osc52());
            seen.lock().unwrap().clear();
            parser.advance(t, osc);
            seen.lock().unwrap().clone()
        };
        assert!(run(&mut t, ClipboardAccess::Off).is_empty());
        assert_eq!(run(&mut t, ClipboardAccess::Write), ["store x"]);
        let rw = run(&mut t, ClipboardAccess::ReadWrite);
        assert_eq!(rw[0], "store x");
        // the reply carries the clipboard text, base64 ("aGk=" = "hi")
        assert!(rw[1].starts_with("load \x1b]52;c;aGk="), "{rw:?}");
        assert_eq!(ClipboardAccess::from_config("off"), ClipboardAccess::Off);
        assert_eq!(
            ClipboardAccess::from_config("read_write"),
            ClipboardAccess::ReadWrite
        );
        assert_eq!(
            ClipboardAccess::from_config("bogus"),
            ClipboardAccess::Write
        );
    }

    #[test]
    fn anchors_follow_reflow() {
        let mut t = term(20, 6);
        let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
        // row 0-1: one 30-char logical line (wraps), row 2: "b", row 3: "c"
        parser.advance(&mut t, format!("{}\r\nb\r\nc", "a".repeat(30)).as_bytes());
        let old = RowLayout::of(&t);
        assert_eq!(old.starts, vec![0, 2, 3, 4, 5]);
        t.resize(GridSize { cols: 10, rows: 6 });
        let new = RowLayout::of(&t);
        // "a…" now takes 3 rows, so "b" and "c" move down one row each
        assert_eq!(old.map(&new, 0, 0, 2, 0), (3, 0));
        assert_eq!(old.map(&new, 0, 0, 3, 0), (4, 0));
        // a point inside the wrapped line keeps its character offset
        assert_eq!(old.map(&new, 0, 0, 1, 5), (2, 5));
        // below the tracked base: untouched
        assert_eq!(old.map(&new, 10, 10, 3, 0), (3, 0));
    }

    fn test_shadow() -> Arc<Shadow> {
        Arc::new(Shadow::new(
            None,
            Arc::new(Mutex::new(WindowSize {
                num_lines: 5,
                num_cols: 20,
                cell_width: 8,
                cell_height: 16,
            })),
        ))
    }

    fn ctx(cols: usize, rows: usize) -> FeedCtx {
        FeedCtx {
            term: Arc::new(FairMutex::new(term(cols, rows))),
            scrolled: Arc::new(AtomicU64::new(0)),
            generation: Arc::new(AtomicU64::new(0)),
            // the grid's own cap (TermConfig::default)
            scrollback: 10_000,
            blocks: Arc::new(Mutex::new(BlockStore::default())),
            cwd: Arc::new(Mutex::new(None)),
            cwd_remote: Arc::new(AtomicBool::new(false)),
            cwd_host: Mutex::new(None),
            user_vars: UserVars::default(),
            widgets: Widgets::default(),
            progress: ProgressSlot::default(),
            graphics: Arc::new(Mutex::new(Graphics::default())),
            window_size: Arc::new(Mutex::new(WindowSize {
                num_lines: rows as u16,
                num_cols: cols as u16,
                cell_width: 8,
                cell_height: 16,
            })),
            reply: None,
            notify: None,
            marks: Mutex::default(),
        }
    }

    /// A program that turned on kitty key reporting (`CSI > 11 u`: event
    /// types, so releases too) and died without popping it: its command's
    /// 133;D gives the shell plain keys again. A full-screen program's flags
    /// (alternate screen) are its own.
    #[test]
    fn a_finished_command_takes_its_kitty_keyboard_flags_with_it() {
        use alacritty_terminal::term::TermMode;
        let kitty = || TermConfig {
            kitty_keyboard: true,
            ..TermConfig::default()
        };
        let c = FeedCtx {
            term: Arc::new(FairMutex::new(term_with(20, 5, kitty()))),
            ..ctx(20, 5)
        };
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        let flags = |c: &FeedCtx| {
            c.term
                .lock()
                .mode()
                .intersection(TermMode::KITTY_KEYBOARD_PROTOCOL)
        };
        feed(&mut p, &mut s, b"\x1b]133;A\x07$ \x1b]133;B\x07", &c);
        feed(&mut p, &mut s, b"\r\n\x1b]133;C\x07\x1b[>11u", &c);
        assert!(
            flags(&c).contains(TermMode::REPORT_EVENT_TYPES),
            "the program's flags apply"
        );
        feed(&mut p, &mut s, b"\x1b]133;D;137\x07", &c);
        assert!(flags(&c).is_empty(), "gone with the command");
        // on the alternate screen they stay the program's
        feed(
            &mut p,
            &mut s,
            b"\x1b]133;C\x07\x1b[?1049h\x1b[>11u\x1b]133;D;0\x07",
            &c,
        );
        assert!(flags(&c).contains(TermMode::REPORT_EVENT_TYPES));
    }

    /// A chunk with a shell marker (a widget's line, a prompt) reports it, so
    /// its reader wakes the app even while a coalesced wakeup is pending.
    #[test]
    fn feed_reports_chunks_with_markers() {
        let c = ctx(20, 5);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        assert!(!feed(&mut p, &mut s, b"plain output\r\n", &c));
        assert!(feed(&mut p, &mut s, b"x\x1b]6973;W;aFILE;%20b\x07", &c));
        assert!(feed(&mut p, &mut s, b"\x1b]133;A\x07", &c));
        // a marker split across chunks counts where it ends
        assert!(!feed(&mut p, &mut s, b"\x1b]133;", &c));
        assert!(feed(&mut p, &mut s, b"B\x07", &c));
    }

    /// An OSC 7 from an ssh session names another host: its path is never
    /// treated as a local directory; the next local report clears that.
    #[test]
    fn a_cwd_on_another_host_is_remote() {
        let c = ctx(20, 5);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        let remote = || c.cwd_remote.load(Ordering::Relaxed);
        feed(
            &mut p,
            &mut s,
            b"\x1b]7;file://build-01.example/srv/app\x07",
            &c,
        );
        assert!(remote());
        assert_eq!(c.cwd.lock().unwrap().as_deref(), Some("/srv/app"));
        let here = format!("\x1b]7;file://{}/tmp\x07", local_host());
        feed(&mut p, &mut s, here.as_bytes(), &c);
        assert!(!remote());
        feed(&mut p, &mut s, b"\x1b]7;file://localhost/tmp\x07", &c);
        assert!(!remote());
    }

    /// A block run on another host records that host's name (two hosts'
    /// runs of one command stay apart in search); a local one records none.
    #[test]
    fn blocks_record_the_remote_host() {
        let c = ctx(40, 8);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        let run = |p: &mut vte::ansi::Processor, s: &mut OscScanner, osc7: &str| {
            let bytes = format!(
                "{osc7}\x1b]133;A\x07$ \x1b]133;B\x07deploy\r\n\x1b]133;C\x07ok\r\n\x1b]133;D;0\x07"
            );
            feed(p, s, bytes.as_bytes(), &c);
            let blocks = c.blocks.lock().unwrap();
            let b = blocks.iter().rfind(|b| b.command == "deploy").unwrap();
            (b.cwd_remote, b.host.clone())
        };
        assert_eq!(
            run(&mut p, &mut s, "\x1b]7;file://prod-1/srv\x07"),
            (true, Some("prod-1".into()))
        );
        assert_eq!(
            run(&mut p, &mut s, "\x1b]7;file://staging/srv\x07"),
            (true, Some("staging".into()))
        );
        let here = format!("\x1b]7;file://{}/tmp\x07", local_host());
        assert_eq!(run(&mut p, &mut s, &here), (false, None));
    }

    /// OSC 9;4 progress stands until state 0, the command's end (133;D), or
    /// leaving the alternate screen it was set in.
    #[test]
    fn progress_reports_and_their_clearing() {
        use rift_core::ProgressState::*;
        let c = ctx(20, 5);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        let get = || {
            c.progress
                .lock()
                .unwrap()
                .map(|r| (r.progress.state, r.progress.pct))
        };
        feed(&mut p, &mut s, b"\x1b]9;4;1;42\x1b\\", &c);
        assert_eq!(get(), Some((Normal, Some(42))));
        // an error without a value keeps the last one's
        feed(&mut p, &mut s, b"\x1b]9;4;2\x07", &c);
        assert_eq!(get(), Some((Error, Some(42))));
        feed(&mut p, &mut s, b"\x1b]9;4;0\x07", &c);
        assert_eq!(get(), None);
        // the running command finishes
        feed(&mut p, &mut s, b"\x1b]9;4;3\x07", &c);
        assert_eq!(get(), Some((Indeterminate, None)));
        feed(&mut p, &mut s, b"\x1b]133;D;0\x07", &c);
        assert_eq!(get(), None);
        // set on the primary screen: survives an alt-screen round trip
        feed(
            &mut p,
            &mut s,
            b"\x1b]9;4;1;10\x07\x1b[?1049h\x1b[?1049l",
            &c,
        );
        assert_eq!(get(), Some((Normal, Some(10))));
        // set inside the alt screen: leaving it clears it
        feed(&mut p, &mut s, b"\x1b[?1049h\x1b]9;4;4;60\x07", &c);
        assert_eq!(get(), Some((Paused, Some(60))));
        feed(&mut p, &mut s, b"\x1b[?1049l", &c);
        assert_eq!(get(), None);
        // malformed: nothing changes
        feed(
            &mut p,
            &mut s,
            b"\x1b]9;4;1;5\x07\x1b]9;4;9;999\x07\x1b]9;4;1;abc\x07",
            &c,
        );
        assert_eq!(get(), Some((Normal, Some(5))));
    }

    #[test]
    fn decscnm_sets_and_resets_the_reverse_screen() {
        let c = ctx(20, 5);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        let on = || c.term.lock().reverse_screen();
        feed(&mut p, &mut s, b"a\x1b[?5hb", &c);
        assert!(on());
        // a mode list carrying 5, as `tput flash` and vttest send it
        feed(&mut p, &mut s, b"\x1b[?7;5l", &c);
        assert!(!on());
        feed(&mut p, &mut s, b"\x1b[?5h", &c);
        // RIS resets it with the rest of the terminal
        feed(&mut p, &mut s, b"\x1bc", &c);
        assert!(!on());
    }

    #[test]
    fn images_place_at_cursor_move_it_and_scroll_away() {
        let c = ctx(20, 5);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        // 16x32 px RGB image = 2 cols x 2 rows at 8x16 cells
        let px = "AAAA".repeat(16 * 32 * 3 / 3);
        let cmd = format!("ab\x1b_Ga=T,f=24,s=16,v=32,q=2;{px}\x1b\\cd");
        feed(&mut p, &mut s, cmd.as_bytes(), &c);
        {
            let g = c.graphics.lock().unwrap();
            assert_eq!(g.placements.len(), 1);
            let pl = &g.placements[0];
            assert_eq!((pl.line, pl.col, pl.cols, pl.rows), (0, 2, 2, 2));
        }
        // cursor moved to the image's last row, one column past it
        let cur = c.term.lock().grid().cursor.point;
        assert_eq!((cur.line.0, cur.column.0), (1, 6), "after 'cd'");
        // scrolling past the (default 10k-line) history drops the image
        feed(&mut p, &mut s, "\n".repeat(10_100).as_bytes(), &c);
        assert!(c.graphics.lock().unwrap().placements.is_empty());
    }

    #[test]
    fn alt_screen_output_keeps_blocks_in_primary_history() {
        let c = ctx(20, 5);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        let block = "\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07";
        feed(&mut p, &mut s, block.as_bytes(), &c);
        // push the finished block into scrollback
        feed(&mut p, &mut s, "\r\n".repeat(20).as_bytes(), &c);
        assert_eq!(c.blocks.lock().unwrap().iter().count(), 1);
        // a full-screen program draws on the alt screen (history 0)
        feed(&mut p, &mut s, b"\x1b[?1049hvim\r\n", &c);
        feed(&mut p, &mut s, b"more\x1b[?1049l", &c);
        assert_eq!(
            c.blocks.lock().unwrap().iter().count(),
            1,
            "block pruned on alt screen"
        );
    }

    #[test]
    fn output_without_final_newline_stays_in_its_block() {
        let c = ctx(20, 6);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        let run = |cmd: &str, out: &str| {
            format!(
                "\x1b]133;A\x07$ \x1b]133;B\x07{cmd}\r\n\x1b]133;C\x07{out}\x1b]133;D;0\x07\x1b]133;A\x07$ "
            )
        };
        feed(&mut p, &mut s, run("printf X", "NO_NEWLINE").as_bytes(), &c);
        let b = c.blocks.lock().unwrap().snapshot();
        let (o, e) = b[0].output_range().expect("partial row is output");
        assert_eq!((o, e), (1, 2));
        assert_eq!(abs_text(&c, 1), "NO_NEWLINE");
        assert!(b[0].unterminated);
        // the next prompt starts on its own row
        assert_eq!(b[1].prompt_start, 2);
        assert_eq!(abs_text(&c, 2), "$");
        // newline-terminated and multi-line-plus-tail output are unchanged
        let c = ctx(20, 8);
        feed(&mut p, &mut s, run("a", "one\r\n").as_bytes(), &c);
        feed(&mut p, &mut s, b"\r", &c);
        feed(&mut p, &mut s, run("b", "x\r\ny\r\ntail").as_bytes(), &c);
        let b = c.blocks.lock().unwrap().snapshot();
        assert_eq!(b[0].output_range(), Some((1, 2)));
        assert!(!b[0].unterminated);
        assert_eq!(b[1].output_range(), Some((3, 6)));
        assert!(b[1].unterminated);
        assert_eq!(abs_text(&c, 5), "tail");
        assert_eq!(b[2].prompt_start, 6);
        // empty output: no range, no extra row
        let c = ctx(20, 6);
        feed(&mut p, &mut s, run("true", "").as_bytes(), &c);
        let b = c.blocks.lock().unwrap().snapshot();
        assert_eq!(b[0].output_range(), None);
        assert_eq!(b[1].prompt_start, 1);
    }

    /// Text of abs line `abs` on the active grid.
    fn abs_text(c: &FeedCtx, abs: u64) -> String {
        let t = c.term.lock();
        let line = abs as i64 - c.scrolled.load(Ordering::Acquire) as i64;
        let row = &t.grid()[Line(line as i32)];
        let s: String = (0..t.grid().columns()).map(|i| row[Column(i)].c).collect();
        s.trim_end().to_string()
    }

    #[test]
    fn resize_on_alt_screen_keeps_primary_anchors() {
        for (rows, cols) in [(5, 20), (14, 20), (8, 6), (8, 40)] {
            let c = ctx(20, 8);
            let mut p = vte::ansi::Processor::new();
            let mut s = OscScanner::new();
            let block = "\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07";
            feed(&mut p, &mut s, block.as_bytes(), &c);
            feed(&mut p, &mut s, "x\r\n".repeat(4).as_bytes(), &c);
            feed(&mut p, &mut s, b"\x1b[?1049h\x1b[Hvim", &c);
            resize_term(
                &mut c.term.lock(),
                GridSize { cols, rows },
                &c.scrolled,
                &c.blocks,
                &c.graphics,
                &c.marks,
            );
            // the alt screen keeps what the program drew
            assert_eq!(abs_text(&c, c.scrolled.load(Ordering::Acquire)), "vim");
            feed(&mut p, &mut s, b"\x1b[?1049l", &c);
            let b = c.blocks.lock().unwrap().snapshot()[0].clone();
            let case = format!("{rows}x{cols}");
            assert_eq!(abs_text(&c, b.prompt_start), "$ ls", "{case}");
            assert_eq!(abs_text(&c, b.output_start.unwrap()), "out", "{case}");
        }
    }

    #[test]
    fn a_transcript_longer_than_the_scrollback_restores_matching_commands() {
        // the oldest replayed blocks scroll out of a small history: each
        // block that remains must keep its own command (reinput runs it)
        let mut c = ctx(20, 5);
        c.scrollback = 20;
        let config = TermConfig {
            scrolling_history: 20,
            ..TermConfig::default()
        };
        *c.term.lock() = term_with(20, 5, config);
        let block = |cmd: &str| rift_core::transcript::SavedBlock {
            command: cmd.into(),
            exit: Some(0),
            duration_ms: 1,
            cwd: None,
            cwd_remote: false,
            output: (0..15).map(|i| format!("{cmd}{i}\n")).collect(),
            finished: None,
            branch: None,
            host: None,
        };
        let t = rift_core::transcript::Transcript::bounded(
            vec![block("first"), block("second"), block("third")],
            0,
        );
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        feed(&mut p, &mut s, &t.replay_bytes(), &c);
        let mut blocks = c.blocks.lock().unwrap();
        blocks.mark_restored(&t.meta());
        let commands: Vec<String> = blocks.iter().map(|b| b.command.clone()).collect();
        assert!(
            commands.len() < 3,
            "the history dropped a block: {commands:?}"
        );
        assert_eq!(commands.last().map(String::as_str), Some("third"));
        assert!(commands.iter().all(|c| c != "first"), "{commands:?}");
    }

    #[test]
    fn the_shells_command_line_beats_an_echo_replayed_at_another_width() {
        // readline placed the echo for 30 columns (it moved the cursor back
        // over its wrap); replayed into 20, the screen's text is not the
        // command, the C marker's copy is
        let echo = "\x1b]133;A\x07$ \x1b]133;B\x07echo LONG_LONG_LONG_LONG_LONG\r\x1b[24C_\r\n";
        for (marker, want) in [
            (
                "\x1b]133;C;cmdline_url=echo%20LONG_LONG_LONG_LONG_LONG_\x07",
                Some("echo LONG_LONG_LONG_LONG_LONG_"),
            ),
            // the user's own integration sent a bare C first
            (
                "\x1b]133;C\x07\x1b]133;C;cmdline_url=echo%20LONG_LONG_LONG_LONG_LONG_\x07",
                Some("echo LONG_LONG_LONG_LONG_LONG_"),
            ),
            ("\x1b]133;C\x07", None),
        ] {
            let c = ctx(20, 6);
            let mut p = vte::ansi::Processor::new();
            let mut s = OscScanner::new();
            let bytes = format!("{echo}{marker}out\r\n\x1b]133;D;0\x07");
            feed(&mut p, &mut s, bytes.as_bytes(), &c);
            let command = c.blocks.lock().unwrap().snapshot()[0].command.clone();
            match want {
                Some(w) => assert_eq!(command, w),
                // without it the screen's (wrong) text stands in
                None => assert_ne!(command, "echo LONG_LONG_LONG_LONG_LONG_"),
            }
        }
    }

    #[test]
    fn a_clear_asked_for_while_connecting_clears_the_replay() {
        // ⌘K before the service answered: the replay it then sends is
        // cleared once it is in, the shell's prompt row kept
        use crate::ipc::{Event as Ev, send_json, send_output};
        use std::os::unix::net::UnixStream;
        let c = ctx(20, 5);
        let term = c.term.clone();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        send_output(&mut peer, 9, 0, 1, b"old output\r\n$ ").unwrap();
        send_json(&mut peer, &Ev::ReplayDone { session: 9 }).unwrap();
        send_json(
            &mut peer,
            &Ev::Exited {
                session: 9,
                code: None,
            },
        )
        .unwrap();
        let conn = Arc::new(Mutex::new(ServiceConn::connecting()));
        conn.lock().unwrap().clear_history = true;
        let reader = Reader {
            id: 1,
            exit: Default::default(),
            ctx: Arc::new(c),
            shadow: test_shadow(),
            dirty: Arc::new(AtomicBool::new(false)),
            saw_output: Arc::new(AtomicBool::new(false)),
            observer: Arc::new(|_, _| {}),
        };
        let source = fake_service(
            Box::new(move || {
                Ok(Opened {
                    stream,
                    session: 9,
                    from: 0,
                    known_cwd: None,
                    resume: Box::new(|_| Err(std::io::ErrorKind::NotFound.into())),
                })
            }),
            Arc::new(AtomicBool::new(true)),
            conn,
        );
        reader.run(source);
        let t = term.lock();
        let history = t.grid().history_size() as i32;
        let text: String = (-history..5)
            .map(|r| {
                let row = &t.grid()[Line(r)];
                (0..20).map(|col| row[Column(col)].c).collect::<String>()
            })
            .collect();
        assert!(!text.contains("old output"), "{text:?}");
    }

    #[test]
    fn output_cannot_rename_its_running_block() {
        // a command's own output printing a C with a line (a raw-mode
        // program, a remote host) is not the shell's: the block keeps its
        // command, so a re-run never runs text the output chose
        let c = ctx(30, 6);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        let bytes = "\x1b]133;A\x07$ \x1b]133;B\x07cat x\r\n\x1b]133;C;cmdline_url=cat%20x\x07\
                     line\r\n\x1b]133;C;cmdline_url=rm%20-rf%20~\x07more\r\n";
        feed(&mut p, &mut s, bytes.as_bytes(), &c);
        assert_eq!(c.blocks.lock().unwrap().snapshot()[0].command, "cat x");
    }

    #[test]
    fn a_wide_character_pushed_to_the_next_row_adds_no_space() {
        // 8 columns: `$ ls 漢` fills 7, 字 does not fit in the last one and
        // goes to the next row, leaving a blank (no marker command: the
        // screen's text stands)
        let c = ctx(8, 4);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        let bytes = "\x1b]133;A\x07$ \x1b]133;B\x07ls 漢字\r\n\x1b]133;C\x07\x1b]133;D;0\x07";
        feed(&mut p, &mut s, bytes.as_bytes(), &c);
        assert_eq!(c.blocks.lock().unwrap().snapshot()[0].command, "ls 漢字");
    }

    #[test]
    fn a_shrink_with_a_full_history_keeps_abs_lines() {
        // the editor showing shrinks the pane; with the history at its cap the
        // shrink drops its oldest rows, and every line keeps its abs index
        let mut c = ctx(20, 8);
        c.scrollback = 10;
        let config = TermConfig {
            scrolling_history: 10,
            ..TermConfig::default()
        };
        *c.term.lock() = term_with(20, 8, config);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        let text: String = (1..=30).map(|i| format!("l{i}\r\n")).collect();
        feed(&mut p, &mut s, text.as_bytes(), &c);
        let block = "\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07";
        feed(&mut p, &mut s, block.as_bytes(), &c);
        assert_eq!(abs_text(&c, 24), "l25");
        for rows in [5, 8, 3] {
            resize_term(
                &mut c.term.lock(),
                GridSize { cols: 20, rows },
                &c.scrolled,
                &c.blocks,
                &c.graphics,
                &c.marks,
            );
            assert_eq!(abs_text(&c, 24), "l25", "{rows} rows");
            assert_eq!(abs_text(&c, 29), "l30", "{rows} rows");
            let b = c.blocks.lock().unwrap().snapshot()[0].clone();
            assert_eq!(abs_text(&c, b.prompt_start), "$ ls", "{rows} rows");
            assert_eq!(abs_text(&c, b.output_start.unwrap()), "out", "{rows} rows");
        }
    }

    #[test]
    fn a_shrink_keeps_abs_lines_of_a_long_wrapped_cursor_line() {
        // the cursor's line wraps over 20 rows and starts in the history: the
        // shrink trims rows of that same line, so only the cursor row's
        // position tells how many went
        let mut c = ctx(20, 8);
        c.scrollback = 10;
        let config = TermConfig {
            scrolling_history: 10,
            ..TermConfig::default()
        };
        *c.term.lock() = term_with(20, 8, config);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        let text: String = (1..=30).map(|i| format!("l{i}\r\n")).collect();
        feed(&mut p, &mut s, text.as_bytes(), &c);
        // 20 rows of 20: "aaa…", "bbb…", … "ttt…"
        let long: String = (0..400)
            .map(|i| char::from(b'a' + (i / 20) as u8))
            .collect();
        feed(&mut p, &mut s, long.as_bytes(), &c);
        let held = |c: &FeedCtx| {
            let sc = c.scrolled.load(Ordering::Acquire);
            let (history, rows) = {
                let t = c.term.lock();
                (
                    t.grid().history_size() as u64,
                    t.grid().screen_lines() as u64,
                )
            };
            (sc - history..sc + rows)
                .map(|abs| (abs, abs_text(c, abs)))
                .collect::<Vec<_>>()
        };
        let before = held(&c);
        resize_term(
            &mut c.term.lock(),
            GridSize { cols: 20, rows: 5 },
            &c.scrolled,
            &c.blocks,
            &c.graphics,
            &c.marks,
        );
        let after = held(&c);
        // 3 rows went: the rest keep their abs index and text
        assert_eq!(after.len(), 15);
        assert_eq!(after[..], before[3..]);
    }

    /// A pane's reader over a fake service connection; the returned receiver
    /// yields the pane's `Exit` events.
    fn service_reader(
        c: FeedCtx,
        stream: std::os::unix::net::UnixStream,
        resume: Resume,
        known_cwd: Option<String>,
    ) -> (std::thread::JoinHandle<()>, std::sync::mpsc::Receiver<()>) {
        let (reader, exited, _) = service_reader_exit(c, stream, resume, known_cwd);
        (reader, exited)
    }

    /// [`service_reader`], also handing back how the pane ended.
    fn service_reader_exit(
        c: FeedCtx,
        stream: std::os::unix::net::UnixStream,
        resume: Resume,
        known_cwd: Option<String>,
    ) -> (
        std::thread::JoinHandle<()>,
        std::sync::mpsc::Receiver<()>,
        ExitCell,
    ) {
        service_reader_kept(Arc::new(c), stream, resume, known_cwd, 0, None)
    }

    /// [`service_reader_exit`] of a replay starting at byte `from`, with
    /// the scrollback an earlier window `kept`.
    fn service_reader_kept(
        c: Arc<FeedCtx>,
        stream: std::os::unix::net::UnixStream,
        resume: Resume,
        known_cwd: Option<String>,
        from: u64,
        kept: Option<crate::kept::Kept>,
    ) -> (
        std::thread::JoinHandle<()>,
        std::sync::mpsc::Receiver<()>,
        ExitCell,
    ) {
        let (conn, _conn_peer) = std::os::unix::net::UnixStream::pair().unwrap();
        std::mem::forget(_conn_peer);
        let (exit_tx, exit_rx) = std::sync::mpsc::channel();
        let exit_tx = Mutex::new(exit_tx);
        let exit = ExitCell::default();
        let reader = Reader {
            id: 1,
            exit: exit.clone(),
            ctx: c,
            shadow: test_shadow(),
            dirty: Arc::new(AtomicBool::new(false)),
            saw_output: Arc::new(AtomicBool::new(false)),
            observer: Arc::new(move |_, ev| {
                if matches!(ev, SessionEvent::Exit) {
                    let _ = exit_tx.lock().unwrap().send(());
                }
            }),
        };
        let mut source = fake_service(
            Box::new(move || {
                Ok(Opened {
                    stream,
                    session: 9,
                    from,
                    known_cwd,
                    resume,
                })
            }),
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(ServiceConn::new(conn))),
        );
        if let Source::Service { kept: k, .. } = &mut source {
            *k = kept.map(Box::new);
        }
        (
            std::thread::spawn(move || reader.run(source)),
            exit_rx,
            exit,
        )
    }

    /// Every row of the primary grid, history first, right-trimmed.
    fn all_rows(term: &Term<EventProxy>) -> Vec<String> {
        let g = term.grid();
        let h = g.history_size() as i32;
        (-h..g.screen_lines() as i32)
            .map(|r| {
                let row: String = (0..g.columns()).map(|c| g[Line(r)][Column(c)].c).collect();
                row.trim_end().to_string()
            })
            .collect()
    }

    /// Run a fake service's output, one frame per chunk with its byte
    /// position, through a reader whose replay starts at `from`; the pane's
    /// context afterwards.
    fn reattach(
        c: FeedCtx,
        chunks: &[(u64, Vec<u8>)],
        from: u64,
        kept: Option<crate::kept::Kept>,
    ) -> Arc<FeedCtx> {
        use crate::ipc::{Event as Ev, send_json, send_output};
        let (stream, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        for (seq, bytes) in chunks {
            send_output(&mut peer, 9, *seq, 1, bytes).unwrap();
        }
        send_json(&mut peer, &Ev::ReplayDone { session: 9 }).unwrap();
        send_json(
            &mut peer,
            &Ev::Exited {
                session: 9,
                code: None,
            },
        )
        .unwrap();
        let c = Arc::new(c);
        let (reader, exited, _) = service_reader_kept(
            c.clone(),
            stream,
            Box::new(|_| Err(std::io::ErrorKind::NotFound.into())),
            None,
            from,
            kept,
        );
        exited
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        reader.join().unwrap();
        c
    }

    /// What a window keeps of its pane for an update restart.
    fn kept_of(c: &FeedCtx) -> crate::kept::Kept {
        let term = c.term.lock();
        let marks = c.marks.lock().unwrap();
        kept_of_term(&term, &marks, c.scrolled.load(Ordering::Acquire))
    }

    /// A full-screen program that has run for long set its modes in output the
    /// service no longer holds (an agent redraws megabytes an hour): after an
    /// update restart its pane is on the alternate screen again, with its
    /// mouse and paste modes, the shell's lines still in the scrollback.
    #[test]
    fn a_reattach_inside_a_full_screen_programs_run_restores_its_modes() {
        use alacritty_terminal::term::TermMode;
        let mut seq = 0u64;
        let mut chunk = |text: String| {
            let at = seq;
            seq += text.len() as u64;
            (at, text.into_bytes())
        };
        let mut chunks = vec![chunk("shell one\r\nshell two\r\n".into())];
        chunks.push(chunk(
            "\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[?25l".into(),
        ));
        for i in 0..20 {
            chunks.push(chunk(format!("\x1b[2;1Hframe {i}  ")));
        }
        let modes = |c: &FeedCtx| {
            let m = *c.term.lock().mode();
            (
                m.contains(TermMode::ALT_SCREEN),
                m.contains(TermMode::MOUSE_REPORT_CLICK),
                m.contains(TermMode::SGR_MOUSE),
                m.contains(TermMode::BRACKETED_PASTE),
                m.contains(TermMode::SHOW_CURSOR),
            )
        };
        let on = (true, true, true, true, false);
        let history = |c: &FeedCtx| -> Vec<String> {
            let t = c.term.lock();
            let g = t.primary_grid();
            let h = g.history_size() as i32;
            (-h..g.screen_lines() as i32)
                .map(|r| {
                    let row: String = (0..g.columns()).map(|c| g[Line(r)][Column(c)].c).collect();
                    row.trim_end().to_string()
                })
                .filter(|r| !r.is_empty())
                .collect()
        };
        let before = reattach(ctx(20, 5), &chunks, 0, None);
        assert_eq!(modes(&before), on);
        let kept = kept_of(&before);
        assert_eq!(kept.alt_since, Some(chunks[2].0));
        // the service retained only the program's last frames
        let from = chunks[12].0;
        let after = reattach(ctx(20, 5), &chunks[12..], from, Some(kept.clone()));
        assert_eq!(modes(&after), on);
        assert_eq!(history(&after), ["shell one", "shell two"]);
        // without what the window kept: the frames land on the shell's screen
        let bare = reattach(ctx(20, 5), &chunks[12..], from, None);
        assert!(!modes(&bare).0);
        // a replay that still holds the program's start sets the modes itself: nothing
        // is parsed before it (the shell's lines would land on the alternate screen)
        assert_eq!(kept.modes_before(0), None);
        assert_eq!(kept.modes_before(chunks[1].0), None);
        let full = reattach(ctx(20, 5), &chunks, 0, Some(kept));
        assert_eq!(modes(&full), on);
        assert_eq!(history(&full), ["shell one", "shell two"]);
        // a program that left the alternate screen keeps nothing to restore
        let mut left = chunks.clone();
        left.push(chunk("\x1b[?1049l".into()));
        let gone = reattach(ctx(20, 5), &left, 0, None);
        assert_eq!(kept_of(&gone).alt_since, None);
    }

    /// A full-screen program that repaints only the cells that change (an
    /// agent's spinner) and does not draw its whole screen again when told
    /// the same size twice: after an update restart its pane shows the
    /// picture the window before had, with the replay's later changes on it,
    /// not those changes alone on an empty screen.
    #[test]
    fn a_reattach_inside_a_full_screen_program_keeps_its_picture() {
        let mut seq = 0u64;
        let mut chunk = |text: String| {
            let at = seq;
            seq += text.len() as u64;
            (at, text.into_bytes())
        };
        let mut chunks = vec![chunk("shell one\r\n".into())];
        chunks.push(chunk(
            "\x1b[?1049h\x1b[?1000h\x1b[1;1Htop rule\x1b[5;1Hstatus line".into(),
        ));
        for i in 0..20 {
            chunks.push(chunk(format!("\x1b[3;1Hspin {i:02}")));
            // a scroll inside the replay: applied twice, it would move
            // the picture once more
            if i == 14 {
                chunks.push(chunk("\x1b[S".into()));
            }
        }
        let screen = |c: &FeedCtx| -> Vec<String> {
            let t = c.term.lock();
            let g = t.grid();
            (0..g.screen_lines() as i32)
                .map(|r| {
                    let row: String = (0..g.columns()).map(|c| g[Line(r)][Column(c)].c).collect();
                    row.trim_end().to_string()
                })
                .collect()
        };
        let want = ["", "spin 14", "spin 19", "status line", ""];
        let before = reattach(ctx(20, 5), &chunks, 0, None);
        assert_eq!(screen(&before), want);
        let kept = kept_of(&before);
        let picture = kept
            .alt_screen
            .clone()
            .expect("a picture of the alternate screen");
        assert_eq!((picture.seq, picture.cols, picture.rows), (seq, 20, 5));
        // the service retained only the spinner's last frames
        let from = chunks[12].0;
        let after = reattach(ctx(20, 5), &chunks[12..], from, Some(kept.clone()));
        assert_eq!(screen(&after), want);
        // output the window had not parsed yet comes on top of the picture,
        // also when its frame starts before the picture's position
        let mut later = chunks[12..].to_vec();
        let (at, last) = later.pop().unwrap();
        let mut straddling = last.clone();
        straddling.extend_from_slice(b"\x1b[3;1Hspin 20");
        later.push((at, straddling));
        assert_eq!(
            screen(&reattach(ctx(20, 5), &later, from, Some(kept.clone())))[2],
            "spin 20"
        );
        // without the picture: the changes alone, on an empty screen
        let bare = crate::kept::Kept {
            alt_screen: None,
            ..kept.clone()
        };
        assert_eq!(
            screen(&reattach(ctx(20, 5), &chunks[12..], from, Some(bare))),
            ["", "spin 14", "spin 19", "", ""]
        );
        // a pane of another size is not given it: the program draws anew
        // for the size it is told
        let other = reattach(ctx(20, 6), &chunks[12..], from, Some(kept.clone()));
        assert_eq!(screen(&other)[3], "");
        // and a pane comes back at the picture's size while its program runs
        assert_eq!(kept.start_rows(20, 3), 5);
        assert_eq!(kept.start_rows(30, 3), 3);
    }

    /// A shell whose service kept only its last lines comes back after an
    /// update restart with every line once: the kept ones above the
    /// replay, none twice, none lost.
    #[test]
    fn a_reattach_shows_kept_lines_above_a_shortened_replay() {
        let mut seq = 0u64;
        let chunks: Vec<(u64, Vec<u8>)> = (0..30)
            .map(|i| {
                let bytes = format!("line {i}\r\n").into_bytes();
                let at = seq;
                seq += bytes.len() as u64;
                (at, bytes)
            })
            .collect();
        let want: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
        let text = |c: &FeedCtx| -> Vec<String> {
            all_rows(&c.term.lock())
                .into_iter()
                .filter(|r| !r.is_empty())
                .collect()
        };
        // the window before the restart saw it all
        let before = reattach(ctx(20, 5), &chunks, 0, None);
        assert_eq!(text(&before), want);
        let kept = kept_of(&before);
        // the service retained the output from line 20 on
        let from = chunks[20].0;
        let after = reattach(ctx(20, 5), &chunks[20..], from, Some(kept.clone()));
        assert_eq!(text(&after), want);
        // without the kept lines, only what the service retained
        let bare = reattach(ctx(20, 5), &chunks[20..], from, None);
        assert_eq!(text(&bare), want[20..].to_vec());
        // a replay from the shell's first byte draws everything itself
        let full = reattach(ctx(20, 5), &chunks, 0, Some(kept));
        assert_eq!(text(&full), want);
        // the window before was narrowed after the output (every line
        // wraps over two rows): its marks follow the reflow
        let narrowed = reattach(ctx(20, 5), &chunks, 0, None);
        resize_term(
            &mut narrowed.term.lock(),
            GridSize { cols: 4, rows: 5 },
            &narrowed.scrolled,
            &narrowed.blocks,
            &narrowed.graphics,
            &narrowed.marks,
        );
        let after = reattach(ctx(20, 5), &chunks[20..], from, Some(kept_of(&narrowed)));
        assert_eq!(text(&after), want);
    }

    /// Kept lines come back as they were drawn: colors, attributes, wide
    /// characters and a line wrapped over several rows (rewrapped at
    /// another width).
    #[test]
    fn kept_lines_keep_their_styles_and_wrapping() {
        let source = b"\x1b[1;31mred\x1b[0m plain \x1b[38;2;1;2;3;48;5;200mrgb\x1b[0m\r\n\
            \xe6\x97\xa5\xe6\x9c\xac wide\r\n\
            abcdefghijklmnopqrstuvwxyz0123456789\r\n";
        let a = ctx(20, 4);
        feed(
            &mut vte::ansi::Processor::new(),
            &mut OscScanner::new(),
            source,
            &a,
        );
        // every line above the cursor's, as a mark at the replay's start
        let kept = {
            let term = a.term.lock();
            let scrolled = a.scrolled.load(Ordering::Acquire);
            crate::kept::Kept {
                marks: vec![(0, scrolled + term.grid().cursor.point.line.0 as u64)],
                lines: crate::kept::lines_of(term.grid(), scrolled),
                ..Default::default()
            }
        };
        let bytes = kept.prelude(0).unwrap();
        for cols in [20, 13] {
            let (want, got) = (ctx(cols, 6), ctx(cols, 6));
            let mut p = vte::ansi::Processor::new();
            feed(&mut p, &mut OscScanner::new(), source, &want);
            feed(
                &mut vte::ansi::Processor::new(),
                &mut OscScanner::new(),
                &bytes,
                &got,
            );
            let cells = |c: &FeedCtx| {
                let t = c.term.lock();
                let g = t.grid();
                let h = g.history_size() as i32;
                (-h..g.screen_lines() as i32)
                    .flat_map(|r| (0..g.columns()).map(move |col| (r, col)))
                    .map(|(r, col)| {
                        let cell = &g[Line(r)][Column(col)];
                        (cell.c, cell.fg, cell.bg, cell.flags)
                    })
                    .collect::<Vec<_>>()
            };
            let (got, want) = (cells(&got), cells(&want));
            let first = got.iter().zip(&want).position(|(g, w)| g != w);
            assert_eq!(
                (first, got.len()),
                (None, want.len()),
                "at {cols} columns: {:?} != {:?}",
                first.map(|i| &got[i.saturating_sub(2)..(i + 3).min(got.len())]),
                first.map(|i| &want[i.saturating_sub(2)..(i + 3).min(want.len())]),
            );
        }
    }

    #[test]
    fn a_service_that_does_not_answer_hands_the_pane_back_with_its_input() {
        // the window never waits for the service: input typed meanwhile is
        // held, and a stalled open reports a failure (never `Exit`, which
        // would close the pane) carrying that input for the local shell
        let conn = Arc::new(Mutex::new(ServiceConn::connecting()));
        conn.lock().unwrap().input(0, b"ls\r");
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = events.clone();
        let reader = Reader {
            id: 1,
            exit: Default::default(),
            ctx: Arc::new(ctx(20, 5)),
            shadow: test_shadow(),
            dirty: Arc::new(AtomicBool::new(false)),
            saw_output: Arc::new(AtomicBool::new(false)),
            observer: Arc::new(move |_, ev| seen.lock().unwrap().push(ev)),
        };
        let mut source = fake_service(
            Box::new(|| Err(std::io::ErrorKind::TimedOut.into())),
            Arc::new(AtomicBool::new(true)),
            conn,
        );
        if let Source::Service { cwd, .. } = &mut source {
            *cwd = Some("/tmp".into());
        }
        reader.run(source);
        let events = events.lock().unwrap();
        match events.as_slice() {
            [SessionEvent::ServiceFailed(f)] => {
                assert!(f.stalled && f.attach);
                assert_eq!(f.held, b"ls\r");
                assert_eq!(f.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
            }
            other => panic!("{other:?}"),
        }
    }

    /// A reader's source for reattaching shell 9 through `open`.
    fn fake_service(open: Open, muted: Arc<AtomicBool>, conn: Arc<Mutex<ServiceConn>>) -> Source {
        Source::Service {
            open,
            target: ServiceTarget::Attach(9),
            cwd: None,
            session_id: Arc::new(AtomicU64::new(9)),
            closed: Arc::new(AtomicBool::new(false)),
            settled: Arc::new(AtomicBool::new(false)),
            muted,
            job: Arc::new(Mutex::new(None)),
            hidden: Arc::new(AtomicBool::new(false)),
            conn,
            kept: None,
        }
    }

    #[test]
    fn dropped_service_connection_resumes_instead_of_closing_the_pane() {
        use crate::ipc::{Event as Ev, send_json, send_output};
        use std::os::unix::net::UnixStream;
        use std::sync::atomic::AtomicU64;
        let c = ctx(20, 5);
        let term = c.term.clone();
        let (first, mut first_peer) = UnixStream::pair().unwrap();
        let (second, mut second_peer) = UnixStream::pair().unwrap();
        send_output(&mut first_peer, 9, 0, 1, b"ab").unwrap();
        // the service drops this client: EOF without an `Exited` frame
        drop(first_peer);
        send_output(&mut second_peer, 9, 2, 1, b"cd").unwrap();
        send_json(&mut second_peer, &Ev::ReplayDone { session: 9 }).unwrap();
        send_json(
            &mut second_peer,
            &Ev::Exited {
                session: 9,
                code: None,
            },
        )
        .unwrap();
        let asked = Arc::new(AtomicU64::new(u64::MAX));
        let (asked_in, mut second) = (asked.clone(), Some(second));
        let (reader, exited) = service_reader(
            c,
            first,
            Box::new(move |from| {
                asked_in.store(from, Ordering::Release);
                let s = second.take().ok_or_else(|| std::io::Error::other("gone"))?;
                Ok((s, false))
            }),
            None,
        );
        exited
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        reader.join().unwrap();
        assert_eq!(
            asked.load(Ordering::Acquire),
            2,
            "resumes after the last byte seen"
        );
        let row: String = {
            let t = term.lock();
            (0..4).map(|i| t.grid()[Line(0)][Column(i)].c).collect()
        };
        assert_eq!(row, "abcd", "output before and after the drop, once each");
        assert!(exited.try_recv().is_err(), "one Exit");
    }

    /// Every input byte the service side of `peer` received, in order.
    fn received_input(peer: std::os::unix::net::UnixStream) -> Vec<u8> {
        let mut peer = peer;
        let mut got = Vec::new();
        while let Ok(Some(f)) = crate::ipc::read_frame(&mut peer) {
            if let crate::ipc::Frame::Input { session: 9, bytes } = f {
                got.extend(bytes);
            }
        }
        got
    }

    #[test]
    fn input_typed_while_reconnecting_arrives_once_in_order() {
        use std::os::unix::net::UnixStream;
        let (old, old_peer) = UnixStream::pair().unwrap();
        let mut conn = ServiceConn::new(old);
        conn.input(9, b"one ");
        let old_got = std::thread::spawn(move || received_input(old_peer));
        // the service drops this client after reading what it was sent
        conn.stream
            .as_ref()
            .unwrap()
            .shutdown(std::net::Shutdown::Write)
            .unwrap();
        assert_eq!(old_got.join().unwrap(), b"one ");
        // the write fails on the dead connection: held, not lost
        conn.input(9, b"two ");
        assert!(conn.pending.is_some());
        conn.down();
        conn.input(9, b"three ");
        let (new, new_peer) = UnixStream::pair().unwrap();
        assert_eq!(conn.reconnected(new, 9), 0);
        conn.input(9, b"four");
        drop(conn);
        // sent on the old connection is not sent again
        assert_eq!(received_input(new_peer), b"two three four");
    }

    #[test]
    fn input_past_the_bound_is_refused_whole_and_counted() {
        use std::os::unix::net::UnixStream;
        let (old, _old_peer) = UnixStream::pair().unwrap();
        let mut conn = ServiceConn::new(old);
        conn.down();
        conn.input(9, &vec![b'x'; MAX_PENDING_INPUT - 1]);
        conn.input(9, b"yz!");
        let (new, new_peer) = UnixStream::pair().unwrap();
        // more than a socket buffer: read while it is sent
        let got = std::thread::spawn(move || received_input(new_peer));
        assert_eq!(conn.reconnected(new, 9), 3, "the write did not fit");
        drop(conn);
        let got = got.join().unwrap();
        assert_eq!(got.len(), MAX_PENDING_INPUT - 1);
        assert!(got.ends_with(b"xx"));
    }

    #[test]
    fn truncated_resume_resets_the_parser_and_tells_the_user() {
        use crate::ipc::{Event as Ev, send_json, send_output};
        use std::os::unix::net::UnixStream;
        let c = ctx(20, 5);
        let term = c.term.clone();
        let (first, mut first_peer) = UnixStream::pair().unwrap();
        let (second, mut second_peer) = UnixStream::pair().unwrap();
        // the drop cuts an escape sequence (ECH, CSI 3 X) in half
        send_output(&mut first_peer, 9, 0, 1, b"ab\x1b[3").unwrap();
        drop(first_peer);
        send_output(&mut second_peer, 9, 900, 1, b"Xy").unwrap();
        send_json(&mut second_peer, &Ev::ReplayDone { session: 9 }).unwrap();
        send_json(
            &mut second_peer,
            &Ev::Exited {
                session: 9,
                code: None,
            },
        )
        .unwrap();
        let (conn, _conn_peer) = UnixStream::pair().unwrap();
        let notices = Arc::new(Mutex::new(Vec::new()));
        let (seen, (done_tx, done)) = (notices.clone(), std::sync::mpsc::channel());
        let done_tx = Mutex::new(done_tx);
        let reader = Reader {
            id: 1,
            exit: Default::default(),
            ctx: Arc::new(c),
            shadow: test_shadow(),
            dirty: Arc::new(AtomicBool::new(false)),
            saw_output: Arc::new(AtomicBool::new(false)),
            observer: Arc::new(move |_, ev| match ev {
                SessionEvent::Notice(m) => seen.lock().unwrap().push(m),
                SessionEvent::Exit => {
                    let _ = done_tx.lock().unwrap().send(());
                }
                _ => {}
            }),
        };
        let mut second = Some(second);
        let source = fake_service(
            Box::new(move || {
                Ok(Opened {
                    stream: first,
                    session: 9,
                    from: 0,
                    known_cwd: None,
                    resume: Box::new(move |_| {
                        let s = second.take().ok_or(std::io::ErrorKind::NotFound)?;
                        Ok((s, true))
                    }),
                })
            }),
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(ServiceConn::new(conn))),
        );
        let handle = std::thread::spawn(move || reader.run(source));
        done.recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        handle.join().unwrap();
        let row: String = {
            let t = term.lock();
            (0..4).map(|i| t.grid()[Line(0)][Column(i)].c).collect()
        };
        // without the reset, "X" would end the cut sequence and vanish
        assert_eq!(row, "abXy");
        let notices = notices.lock().unwrap();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("missing"));
    }

    /// Output replayed after a resume was never parsed by this pane: a query
    /// in it is answered and a notification in it fires (only the first
    /// attach's replay was seen, and answered, by an earlier window).
    #[test]
    fn resumed_output_is_answered_and_notifies() {
        use crate::ipc::{Event as Ev, Frame, read_frame, send_json, send_output};
        use std::os::unix::net::UnixStream;
        let (conn_stream, _conn_peer) = UnixStream::pair().unwrap();
        let conn = Arc::new(Mutex::new(ServiceConn::new(conn_stream)));
        let muted = Arc::new(AtomicBool::new(false));
        let replier = Replier::new(PtyWriter::Service {
            conn: conn.clone(),
            session: Arc::new(AtomicU64::new(9)),
            muted: muted.clone(),
        });
        let mut c = ctx(20, 5);
        let notified = Arc::new(AtomicU64::new(0));
        let n = notified.clone();
        c.notify = Some((
            1,
            Arc::new(move |_, ev| {
                if matches!(ev, SessionEvent::Notify { .. }) {
                    n.fetch_add(1, Ordering::AcqRel);
                }
            }),
        ));
        c.term = Arc::new(FairMutex::new(Term::new(
            TermConfig::default(),
            &GridSize { cols: 20, rows: 5 },
            EventProxy {
                pane: 1,
                observer: Arc::new(|_, _| {}),
                writer: replier.clone(),
                window_size: c.window_size.clone(),
            },
        )));
        c.reply = Some(replier);
        let (first, mut first_peer) = UnixStream::pair().unwrap();
        let (second, mut second_peer) = UnixStream::pair().unwrap();
        send_output(&mut first_peer, 9, 0, 1, b"a").unwrap();
        drop(first_peer);
        // sent while this pane was disconnected: a DA1 query, a notification
        send_output(&mut second_peer, 9, 1, 1, b"\x1b[c\x1b]9;done\x07").unwrap();
        send_json(&mut second_peer, &Ev::ReplayDone { session: 9 }).unwrap();
        send_json(
            &mut second_peer,
            &Ev::Exited {
                session: 9,
                code: None,
            },
        )
        .unwrap();
        second_peer
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut second = Some(second);
        let (done_tx, done) = std::sync::mpsc::channel();
        let done_tx = Mutex::new(done_tx);
        let reader = Reader {
            id: 1,
            exit: Default::default(),
            ctx: Arc::new(c),
            shadow: test_shadow(),
            dirty: Arc::new(AtomicBool::new(false)),
            saw_output: Arc::new(AtomicBool::new(false)),
            observer: Arc::new(move |_, ev| {
                if matches!(ev, SessionEvent::Exit) {
                    let _ = done_tx.lock().unwrap().send(());
                }
            }),
        };
        let source = fake_service(
            Box::new(move || {
                Ok(Opened {
                    stream: first,
                    session: 9,
                    from: 0,
                    known_cwd: None,
                    resume: Box::new(move |_| {
                        let s = second.take().ok_or(std::io::ErrorKind::NotFound)?;
                        Ok((s, false))
                    }),
                })
            }),
            muted,
            conn,
        );
        let handle = std::thread::spawn(move || reader.run(source));
        done.recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        handle.join().unwrap();
        // the reply reaches the service on the resumed connection
        let mut input = Vec::new();
        while !input.starts_with(b"\x1b[?") {
            match read_frame(&mut second_peer) {
                Ok(Some(Frame::Input { bytes, .. })) => input.extend(bytes),
                Ok(Some(_)) => {}
                _ => break,
            }
        }
        assert!(input.starts_with(b"\x1b[?"), "DA1 answered: {input:?}");
        assert_eq!(notified.load(Ordering::Acquire), 1);
    }

    #[test]
    fn vanished_service_session_still_closes_the_pane() {
        use crate::ipc::send_output;
        use std::os::unix::net::UnixStream;
        let (first, mut first_peer) = UnixStream::pair().unwrap();
        send_output(&mut first_peer, 9, 0, 1, b"ab").unwrap();
        drop(first_peer);
        let (reader, exited, exit) = service_reader_exit(
            ctx(20, 5),
            first,
            Box::new(|_| Err(std::io::Error::from(std::io::ErrorKind::NotFound))),
            None,
        );
        exited
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        reader.join().unwrap();
        // the service answered that the shell is gone: ended, not lost
        assert!(!exit.lost());
        assert_eq!(exit.get(), Some(None));
    }

    /// A service that stops answering (refused, timing out) leaves the pane
    /// lost, not ended: the app must neither close it nor hang up a shell
    /// that may still run.
    #[test]
    fn a_service_that_never_answers_leaves_the_pane_lost_not_ended() {
        use crate::ipc::send_output;
        use std::os::unix::net::UnixStream;
        let (first, mut first_peer) = UnixStream::pair().unwrap();
        send_output(&mut first_peer, 9, 0, 1, b"ab").unwrap();
        drop(first_peer);
        let tries = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counted = tries.clone();
        let (reader, exited, exit) = service_reader_exit(
            ctx(20, 5),
            first,
            Box::new(move |_| {
                counted.fetch_add(1, Ordering::AcqRel);
                Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
            }),
            None,
        );
        exited
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        reader.join().unwrap();
        assert!(exit.lost(), "lost, not ended");
        assert_eq!(exit.get(), Some(None));
        assert!(tries.load(Ordering::Acquire) > 1, "kept trying");
    }

    /// A service that is slow to answer the reconnect (refused twice, then
    /// back) keeps the pane: its output goes on.
    #[test]
    fn a_service_back_after_a_few_refusals_keeps_the_pane() {
        use crate::ipc::{Event as Ev, send_json, send_output};
        use std::os::unix::net::UnixStream;
        let c = ctx(20, 5);
        let term = c.term.clone();
        let (first, mut first_peer) = UnixStream::pair().unwrap();
        send_output(&mut first_peer, 9, 0, 1, b"ab").unwrap();
        drop(first_peer);
        let (second, mut second_peer) = UnixStream::pair().unwrap();
        send_output(&mut second_peer, 9, 2, 1, b"cd").unwrap();
        send_json(&mut second_peer, &Ev::ReplayDone { session: 9 }).unwrap();
        send_json(
            &mut second_peer,
            &Ev::Exited {
                session: 9,
                code: Some(0),
            },
        )
        .unwrap();
        let mut refusals = 2;
        let mut second = Some(second);
        let (reader, exited, exit) = service_reader_exit(
            c,
            first,
            Box::new(move |_| {
                if refusals > 0 {
                    refusals -= 1;
                    return Err(std::io::ErrorKind::ConnectionRefused.into());
                }
                let s = second.take().ok_or_else(|| std::io::Error::other("gone"))?;
                Ok((s, false))
            }),
            None,
        );
        exited
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        reader.join().unwrap();
        assert!(!exit.lost());
        assert_eq!(exit.get(), Some(Some(0)), "the shell's own exit");
        let row: String = {
            let t = term.lock();
            (0..4).map(|i| t.grid()[Line(0)][Column(i)].c).collect()
        };
        assert_eq!(row, "abcd");
    }

    /// A reattached shell whose last OSC 7 left the service's ring: the
    /// pane takes the directory the service reports once the replay is
    /// done, and the replayed block keeps its own (unknown) one.
    #[test]
    fn reattached_pane_takes_the_service_cwd_after_the_replay() {
        use crate::ipc::{Event as Ev, send_json, send_output};
        use std::os::unix::net::UnixStream;
        let run = |replay: &[u8]| {
            let c = ctx(40, 5);
            let (cwd, blocks) = (c.cwd.clone(), c.blocks.clone());
            let (stream, mut peer) = UnixStream::pair().unwrap();
            send_output(&mut peer, 9, 0, 1, replay).unwrap();
            send_json(&mut peer, &Ev::ReplayDone { session: 9 }).unwrap();
            send_json(
                &mut peer,
                &Ev::Exited {
                    session: 9,
                    code: None,
                },
            )
            .unwrap();
            let (reader, exited) = service_reader(
                c,
                stream,
                Box::new(|_| Err(std::io::Error::from(std::io::ErrorKind::NotFound))),
                Some("/srv/now".into()),
            );
            exited
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            reader.join().unwrap();
            let block_cwd = blocks.lock().unwrap().last().and_then(|b| b.cwd.clone());
            let pane_cwd = cwd.lock().unwrap().clone();
            (pane_cwd, block_cwd)
        };
        let job = b"\x1b]133;A\x07$ \x1b]133;B\x07job\r\n\x1b]133;C\x07out\r\n";
        assert_eq!(run(job), (Some("/srv/now".into()), None));
        // an OSC 7 in the replay is newer than nothing: it stays
        let mut with_osc7 = b"\x1b]7;file://h/srv/then\x07".to_vec();
        with_osc7.extend_from_slice(job);
        assert_eq!(
            run(&with_osc7),
            (Some("/srv/then".into()), Some("/srv/then".into()))
        );
    }

    /// Theme color queries answer on the reader thread, in order with the
    /// replies around them (Codex reads `OSC 10;? OSC 11;? CSI c` up to the
    /// DA1 reply), and `CSI ? 996 n` reports the scheme of the background.
    #[test]
    fn color_queries_answer_in_order() {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        use std::os::fd::FromRawFd;
        let mut read_end = unsafe { File::from_raw_fd(fds[0]) };
        let sink = unsafe { File::from_raw_fd(fds[1]) };
        let mut colors = vec![Rgb { r: 0, g: 0, b: 0 }; 259];
        colors[256] = Rgb {
            r: 0xd4,
            g: 0xd4,
            b: 0xd4,
        };
        colors[257] = Rgb {
            r: 0x1e,
            g: 0x1e,
            b: 0x1e,
        };
        set_default_colors(colors);
        let proxy = EventProxy {
            pane: 0,
            // the app's asynchronous fallback must not be needed
            observer: Arc::new(|_, e| {
                assert!(!matches!(e, SessionEvent::ColorRequest(..)));
            }),
            writer: Replier::new(PtyWriter::File(Arc::new(sink))),
            window_size: Arc::new(Mutex::new(WindowSize {
                num_lines: 4,
                num_cols: 20,
                cell_width: 8,
                cell_height: 16,
            })),
        };
        let mut t = Term::new(
            TermConfig::default(),
            &GridSize { cols: 20, rows: 4 },
            proxy,
        );
        let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
        parser.advance(&mut t, b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b[?996n\x1b[c");
        // the reply thread ends (and closes the pipe) with the Term
        drop(t);
        let mut out = String::new();
        read_end.read_to_string(&mut out).unwrap();
        let da1 = out.find("\x1b[?").filter(|&i| out[i..].ends_with('c'));
        assert!(
            out.starts_with(
                "\x1b]10;rgb:d4d4/d4d4/d4d4\x1b\\\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\\x1b[?997;1n"
            ),
            "{out:?}"
        );
        assert!(da1.is_some() && out.ends_with('c'), "{out:?}");
    }

    /// A pane with a colour scheme of its own answers queries with it; the
    /// other panes keep the window theme's, and closing it forgets it.
    #[test]
    fn a_panes_own_colors_answer_its_queries() {
        let red = Rgb {
            r: 0xff,
            g: 0,
            b: 0,
        };
        let mut colors = vec![Rgb { r: 1, g: 2, b: 3 }; 259];
        colors[257] = red;
        set_pane_colors(4242, Some(colors));
        assert_eq!(default_color(4242, 257), Some(red));
        assert_ne!(default_color(4243, 257), Some(red));
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        use std::os::fd::FromRawFd;
        let mut read_end = unsafe { File::from_raw_fd(fds[0]) };
        let sink = unsafe { File::from_raw_fd(fds[1]) };
        let proxy = EventProxy {
            pane: 4242,
            observer: Arc::new(|_, e| {
                assert!(!matches!(e, SessionEvent::ColorRequest(..)));
            }),
            writer: Replier::new(PtyWriter::File(Arc::new(sink))),
            window_size: Arc::new(Mutex::new(WindowSize {
                num_lines: 4,
                num_cols: 20,
                cell_width: 8,
                cell_height: 16,
            })),
        };
        let mut t = Term::new(
            TermConfig::default(),
            &GridSize { cols: 20, rows: 4 },
            proxy,
        );
        let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
        parser.advance(&mut t, b"\x1b]11;?\x1b\\");
        drop(t);
        let mut out = String::new();
        read_end.read_to_string(&mut out).unwrap();
        assert_eq!(out, "\x1b]11;rgb:ffff/0000/0000\x1b\\");
        set_pane_colors(4242, None);
        assert_ne!(default_color(4242, 257), Some(red));
    }

    #[test]
    fn reply_flood_never_blocks_the_reader() {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        use std::os::fd::FromRawFd;
        // nobody reads the pipe: the program stopped reading its stdin
        let _stuck_reader = unsafe { File::from_raw_fd(fds[0]) };
        let sink = unsafe { File::from_raw_fd(fds[1]) };
        let replier = Replier::new(PtyWriter::File(Arc::new(sink)));
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // far more than a pipe (or a PTY input queue) holds
            for _ in 0..200_000 {
                replier.reply(b"\x1b[?62;4c");
            }
            done.send(()).unwrap();
        });
        finished
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("replying blocked on a program that does not read input");
    }

    #[test]
    fn input_hidden_reads_the_line_discipline() {
        let spawn = |cmd: &str| {
            let opts = tty::Options {
                shell: Some(tty::Shell::new(
                    "/bin/sh".into(),
                    vec!["-c".into(), cmd.into()],
                )),
                working_directory: None,
                drain_on_exit: false,
                env: std::collections::HashMap::new(),
            };
            let ws = WindowSize {
                num_lines: 24,
                num_cols: 80,
                cell_width: 8,
                cell_height: 16,
            };
            tty::new(&opts, ws, 0).unwrap()
        };
        let until = |pty: &tty::Pty, want: bool| {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while input_hidden(pty) != want && std::time::Instant::now() < end {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            input_hidden(pty)
        };
        // `exec sleep`: dies on the SIGHUP that dropping the Pty sends
        let password = spawn("stty -echo; exec sleep 600");
        assert!(until(&password, true), "echo off in line mode");
        let raw = spawn("stty -echo -icanon; exec sleep 600");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            !until(&raw, false),
            "a full-screen program is not a password"
        );
        let plain = spawn("exec sleep 600");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!input_hidden(&plain));
    }

    #[test]
    fn a_job_named_after_a_version_reads_as_the_name_it_ran_by() {
        // Claude Code's native install: `claude` links to `versions/2.1.296`
        let dir = std::env::temp_dir().join(format!("rift-job-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("9.9.9");
        std::fs::copy("/bin/sleep", &exe).unwrap();
        std::os::unix::fs::symlink(&exe, dir.join("claude")).unwrap();
        // until the child's exec lands it still carries this test thread's
        // name (Linux reads /proc right after fork): poll for the expected
        // one, then report whatever it settled on
        let name_of = |program: &std::path::Path, want: &str| {
            let mut child = std::process::Command::new(program)
                .arg("30")
                .spawn()
                .unwrap();
            let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut name = process_name(child.id() as i32);
            while name != want && std::time::Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(10));
                name = process_name(child.id() as i32);
            }
            let _ = child.kill();
            let _ = child.wait();
            name
        };
        let linked = name_of(&dir.join("claude"), "claude");
        let plain = name_of(&exe, "9.9.9");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(linked, "claude");
        // run by its own name, it is what it says
        assert_eq!(plain, "9.9.9");
    }

    #[test]
    fn input_to_a_program_that_never_reads_returns_at_once() {
        use std::os::fd::AsRawFd;
        let opts = tty::Options {
            // never reads its input; dies on the drop's SIGHUP
            shell: Some(tty::Shell::new("/bin/sleep".into(), vec!["600".into()])),
            working_directory: None,
            drain_on_exit: false,
            env: std::collections::HashMap::new(),
        };
        let ws = WindowSize {
            num_lines: 24,
            num_cols: 80,
            cell_width: 8,
            cell_height: 16,
        };
        let pty = tty::new(&opts, ws, 0).unwrap();
        // blocking, as `Session::spawn` leaves it for the reader
        let fd = pty.file().as_raw_fd();
        unsafe {
            let fl = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, fl & !libc::O_NONBLOCK);
            // raw, as vim or an agent's TUI sets it: a canonical tty drops
            // what overflows its line buffer instead of blocking the writer
            let mut tio: libc::termios = std::mem::zeroed();
            assert_eq!(libc::tcgetattr(fd, &mut tio), 0);
            libc::cfmakeraw(&mut tio);
            assert_eq!(libc::tcsetattr(fd, libc::TCSANOW, &tio), 0);
        }
        let notices = Arc::new(Mutex::new(Vec::new()));
        let seen = notices.clone();
        let q = InputQueue::new(
            PtyWriter::File(Arc::new(pty.file().try_clone().unwrap())),
            move |m| seen.lock().unwrap().push(m),
        );
        let t = std::time::Instant::now();
        // far past the kernel's tty buffer: a direct write would block here
        assert!(q.write(&vec![b'x'; 256 * 1024]));
        assert!(q.write(b"\x1b[201~"));
        assert!(
            t.elapsed() < std::time::Duration::from_millis(500),
            "{:?}",
            t.elapsed()
        );
        assert!(q.queued() > 0, "the program never read: bytes still wait");
        // bounded: more than the cap is refused whole, and said so
        assert!(!q.write(&vec![b'y'; MAX_QUEUED_INPUT]));
        assert!(t.elapsed() < std::time::Duration::from_millis(500));
        let notices = notices.lock().unwrap();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("not reading"), "{notices:?}");
        drop(pty);
    }

    /// A resize (or clear) on a durable pane goes out after the input before
    /// it, and queuing it never waits on the connection: the writer thread
    /// may be stuck sending input the service is not taking.
    #[test]
    fn service_requests_follow_input_and_never_block_the_caller() {
        use crate::ipc::{Frame, Request, read_frame};
        use std::os::unix::net::UnixStream;
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        let conn = Arc::new(Mutex::new(ServiceConn::new(ours)));
        let q = InputQueue::new(
            PtyWriter::Service {
                conn: conn.clone(),
                session: Arc::new(AtomicU64::new(9)),
                muted: Arc::new(AtomicBool::new(false)),
            },
            |_| {},
        );
        // the connection is busy: queuing must still return at once
        let held = conn.lock().unwrap();
        let t = std::time::Instant::now();
        assert!(q.write(b"abc"));
        q.request(Request::ClearHistory { session: 9 });
        assert!(t.elapsed() < std::time::Duration::from_millis(200));
        drop(held);
        drop(q);
        theirs
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let first = read_frame(&mut theirs).unwrap().unwrap();
        assert!(matches!(first, Frame::Input { session: 9, ref bytes } if bytes == b"abc"));
        let Frame::Json(body) = read_frame(&mut theirs).unwrap().unwrap() else {
            panic!("the request comes second");
        };
        assert!(String::from_utf8_lossy(&body).contains("clear_history"));
    }

    #[test]
    fn queued_input_arrives_once_and_in_order() {
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        // a File over the socket stands in for the PTY master
        let file = File::from(std::os::fd::OwnedFd::from(ours));
        let q = InputQueue::new(PtyWriter::File(Arc::new(file)), |_| {});
        let mut sent = Vec::new();
        for i in 0..200u32 {
            let chunk = format!("<{i}>").repeat(50).into_bytes();
            assert!(q.write(&chunk));
            sent.extend(chunk);
        }
        drop(q);
        let mut got = Vec::new();
        std::io::Read::read_to_end(&mut &theirs, &mut got).unwrap();
        assert_eq!(got, sent);
    }

    #[test]
    fn resize_signals_the_pty_only_after_the_term_is_resized() {
        use std::os::fd::AsRawFd;
        let ws = WindowSize {
            num_lines: 24,
            num_cols: 80,
            cell_width: 8,
            cell_height: 16,
        };
        let opts = tty::Options {
            // dies on the SIGHUP that dropping the Pty sends (an
            // interactive sh ignores it and the drop would wait forever)
            shell: Some(tty::Shell::new("/bin/sleep".into(), vec!["600".into()])),
            working_directory: None,
            drain_on_exit: false,
            env: std::collections::HashMap::new(),
        };
        let pty = tty::new(&opts, ws, 0).unwrap();
        let fd = pty.file().as_raw_fd();
        let writer = PtyWriter::File(Arc::new(pty.file().try_clone().unwrap()));
        let c = ctx(80, 24);
        let rows = move || {
            let mut w: libc::winsize = unsafe { std::mem::zeroed() };
            assert_eq!(unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut w) }, 0);
            w.ws_row
        };
        let mut session = Session {
            id: 0,
            term: c.term.clone(),
            blocks: c.blocks.clone(),
            graphics: c.graphics.clone(),
            scrolled: c.scrolled.clone(),
            cwd: c.cwd.clone(),
            cwd_remote: c.cwd_remote.clone(),
            user_vars: c.user_vars.clone(),
            widgets: c.widgets.clone(),
            progress: c.progress.clone(),
            dirty: Arc::new(AtomicBool::new(false)),
            saw_output: Arc::new(AtomicBool::new(false)),
            generation: c.generation.clone(),
            inputs: AtomicU64::new(0),
            read_only: AtomicBool::new(false),
            refused: AtomicBool::new(false),
            exit: Default::default(),
            writer: InputQueue::new(writer, |_| {}),
            backend: Backend::Local(pty),
            size: GridSize { cols: 80, rows: 24 },
            window_size: c.window_size.clone(),
            scrollback: 100,
            shadow: test_shadow(),
            feed_ctx: Arc::new(ctx(80, 24)),
            observer: Arc::new(|_, _| {}),
            ambiguous_wide: AtomicBool::new(false),
        };
        assert_eq!(rows(), 24);
        // the reader is mid-parse (holds the term lock) while the window resizes
        let term = c.term.clone();
        let held = term.lock();
        let resizing = std::thread::spawn(move || {
            session.resize(
                GridSize {
                    cols: 100,
                    rows: 30,
                },
                (8, 16),
            );
            session
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(
            rows(),
            24,
            "the PTY was resized before the term lock was free"
        );
        drop(held);
        let session = resizing.join().unwrap();
        assert_eq!(rows(), 30);
        assert_eq!(term.lock().grid().screen_lines(), 30);
        // a resize is a new screen for the remote gateway
        assert_eq!(c.generation.load(Ordering::Acquire), 1);
        // the terminal's own reports are not the user answering the program
        session.write_report(b"\x1b[I");
        assert_eq!(session.inputs(), 0);
        session.write(b"y");
        assert_eq!(session.inputs(), 1);
        drop(session);
    }

    #[test]
    fn autowrap_on_the_bottom_row_counts_the_scrolled_line() {
        let c = ctx(10, 3);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        // fill the screen, then a 25-char line on the bottom row wraps twice
        feed(&mut p, &mut s, b"a\r\nb\r\n", &c);
        feed(&mut p, &mut s, "x".repeat(25).as_bytes(), &c);
        // each parsed chunk is a new screen for the remote gateway
        assert_eq!(c.generation.load(Ordering::Acquire), 2);
        let history = c.term.lock().grid().history_size() as u64;
        assert_eq!(history, 2);
        assert_eq!(c.scrolled.load(Ordering::Acquire), history);
        // CSI M on the top row of the full screen scrolls into history too
        feed(&mut p, &mut s, b"\x1b[H\x1b[M", &c);
        let history = c.term.lock().grid().history_size() as u64;
        assert_eq!(c.scrolled.load(Ordering::Acquire), history, "delete_lines");
    }

    #[test]
    fn resize_on_alt_screen_rewraps_a_running_command() {
        // the command line fits at 40 cols, wraps at 20, fits again at 40;
        // the program (less) entered the alt screen right after 133;C
        let c = ctx(40, 8);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        let cmd = "less /some/long/path/f.txt"; // 2 + 26 = 28 cols
        let start = format!(
            "{}\x1b]133;A\x07$ \x1b]133;B\x07{cmd}\r\n\x1b]133;C\x07",
            "x\r\n".repeat(20)
        );
        feed(&mut p, &mut s, start.as_bytes(), &c);
        feed(&mut p, &mut s, b"\x1b[?1049h\x1b[Hpage", &c);
        let resize = |cols, rows| {
            resize_term(
                &mut c.term.lock(),
                GridSize { cols, rows },
                &c.scrolled,
                &c.blocks,
                &c.graphics,
                &c.marks,
            )
        };
        resize(20, 5);
        let b = c.blocks.lock().unwrap().snapshot()[0].clone();
        assert_eq!(
            b.output_start.unwrap(),
            b.prompt_start + 2,
            "narrow: command wraps onto two rows"
        );
        resize(40, 12);
        feed(&mut p, &mut s, b"\x1b[?1049l\x1b]133;D;0\x07", &c);
        let b = c.blocks.lock().unwrap().snapshot()[0].clone();
        assert_eq!(abs_text(&c, b.prompt_start), format!("$ {cmd}"));
        assert_eq!(b.output_start.unwrap(), b.prompt_start + 1, "wide again");
    }

    #[test]
    fn malformed_graphics_never_panic_or_place() {
        let c = ctx(20, 5);
        let mut p = vte::ansi::Processor::new();
        let mut s = OscScanner::new();
        for junk in [
            &b"\x1b_Ga=T,f=100;!!!!not base64\x1b\\"[..],
            b"\x1b_Ga=T,f=32,s=0,v=0;AAAA\x1b\\",
            b"\x1b_Ga=p,i=4294967295\x1b\\",
            b"\x1b_Gm=1;AAAA\x1b\\", // stray continuation
            b"\x1b_Ga=T,f=100,m=1;iVBORw0KGgo=\x1b\\\x1b_Gm=0;\x1b\\", // truncated png
            b"\x1b]1337;File=inline=1:////\x07",
            b"\x1b]1337;File=:\x07",
            b"\x1b_G", // split: the terminator arrives in the next chunk
            b"a=q\x1b\\",
            b"text continues\r\n",
        ] {
            feed(&mut p, &mut s, junk, &c);
        }
        assert!(c.graphics.lock().unwrap().placements.is_empty());
        // the parser recovered: plain text still lands on the grid
        let text: String = {
            let t = c.term.lock();
            let row = &t.grid()[Line(0)];
            (0..14).map(|i| row[Column(i)].c).collect()
        };
        assert_eq!(text, "text continues");
    }

    // Captures against a real program on a real pty; with RIFT_PERSIST_SESSION
    // the shell runs under the session service at XDG_DATA_HOME (started by
    // whoever runs the test: this binary is not the service).
    mod shadow {
        use super::*;
        use std::time::{Duration, Instant};

        const PHONE: GridSize = GridSize { cols: 44, rows: 30 };
        const OWN: GridSize = GridSize {
            cols: 136,
            rows: 43,
        };

        /// `draw` is a shell function run at start and on every SIGWINCH; a
        /// line of `size_log` per draw, as `stty` read it.
        fn program(dir: &std::path::Path, setup: &str, on_winch: &str) -> String {
            format!(
                "exec 2>/dev/null; {setup}\n\
                 draw() {{ s=$(stty size </dev/tty); echo \"$s\" >> {log}; \
                 printf '\\033[2J\\033[HSIZE %s' \"$s\"; {on_winch} }}\n\
                 trap draw WINCH; draw\n\
                 while :; do sleep 0.02 & wait $!; done\n",
                log = dir.join("sizes").display(),
            )
        }

        fn test_dir(name: &str) -> std::path::PathBuf {
            let dir =
                std::env::temp_dir().join(format!("rift-shadow-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn spawn(name: &str, script: &str) -> (Session, std::path::PathBuf) {
            let dir = test_dir(name);
            let session = Session::spawn(SpawnOptions {
                id: 900_000 + u64::from(std::process::id()) % 1000,
                size: OWN,
                cell_size: (8, 16),
                shell: Some("/bin/sh".into()),
                args: vec!["-c".into(), script.to_string()],
                working_directory: None,
                env: Default::default(),
                scrollback: 1000,
                observer: Arc::new(|_, _| {}),
                replay: None,
                service: std::env::var_os("RIFT_PERSIST_SESSION").map(|_| ServiceTarget::Spawn),
                kept: None,
                clipboard: ClipboardAccess::Off,
                ambiguous_wide: false,
            })
            .unwrap();
            (session, dir)
        }

        fn until(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
            let end = Instant::now() + Duration::from_secs(secs);
            while !ok() {
                assert!(Instant::now() < end, "timed out: {what}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn real_text(s: &Session) -> String {
            s.screen_rows().join("\n")
        }

        /// Up on the alternate screen, drawn, quiet long enough to capture.
        fn ready(s: &Session) {
            until("program up", 10, || {
                real_text(s).contains("SIZE 43 136")
                    && matches!(s.shadow_due(PHONE, Instant::now()), ShadowDue::Now)
            });
        }

        fn frame_text(s: &Session) -> Option<String> {
            s.shadow_frame(PHONE, |t, _, _| {
                let g = t.grid();
                (0..g.screen_lines())
                    .map(|l| {
                        (0..g.columns())
                            .map(|c| g[Line(l as i32)][Column(c)].c)
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        }

        fn sizes(dir: &std::path::Path) -> Vec<String> {
            std::fs::read_to_string(dir.join("sizes"))
                .unwrap_or_default()
                .lines()
                .map(String::from)
                .collect()
        }

        fn stats_after(s: &Session, what: &str, ok: impl Fn(&ShadowStats) -> bool) -> ShadowStats {
            until(what, 5, || ok(&s.shadow_stats()));
            s.shadow_stats()
        }

        const ALT: &str = "printf '\\033[?1049h';";

        /// The frame is the phone's size and its redraw; the Term never left
        /// its own, was repainted, and the program ends at its own size.
        #[test]
        fn shadow_frame_is_phone_size_and_the_term_stays() {
            let dir = test_dir("frame");
            let (s, dir) = spawn("frame", &program(&dir, ALT, ""));
            ready(&s);
            let generation = s.generation.load(Ordering::Acquire);
            assert!(s.shadow_capture(PHONE));
            let st = stats_after(&s, "framed", |st| st.framed == 1);
            assert_eq!(
                (st.started, st.signals, st.aborted, st.no_answer),
                (1, 2, 0, 0)
            );
            let text = frame_text(&s).unwrap();
            assert!(text.contains("SIZE 30 44"), "{text}");
            {
                let term = s.term.lock();
                assert_eq!(term.grid().columns(), 136);
                assert_eq!(term.grid().screen_lines(), 43);
            }
            assert!(real_text(&s).contains("SIZE 43 136"));
            assert!(
                s.generation.load(Ordering::Acquire) > generation,
                "repainted"
            );
            assert_eq!(
                sizes(&dir),
                ["43 136", "30 44", "43 136"],
                "told the phone's size once and its own back"
            );
            assert_eq!(s.size(), OWN);
            // nothing moved since: no second capture
            assert_eq!(s.shadow_due(PHONE, Instant::now()), ShadowDue::No);
            assert!(!s.shadow_capture(PHONE));
            s.terminate();
        }

        /// A program that ignores SIGWINCH: no hang, no frame, blocked a minute.
        #[test]
        fn shadow_program_ignoring_winch_is_blocked() {
            let (s, _dir) = spawn(
                "ignore",
                "trap '' WINCH; printf '\\033[?1049hSIZE 43 136'; sleep 60",
            );
            ready(&s);
            let at = Instant::now();
            assert!(s.shadow_capture(PHONE));
            let st = stats_after(&s, "gave up", |st| st.no_answer == 1);
            assert!(at.elapsed() < Duration::from_secs(2), "{:?}", at.elapsed());
            assert_eq!((st.framed, st.signals), (0, 2));
            assert!(frame_text(&s).is_none());
            match s.shadow_due(PHONE, Instant::now()) {
                ShadowDue::At(when) => {
                    assert!(when > Instant::now() + Duration::from_secs(55));
                }
                other => panic!("{other:?}"),
            }
            assert!(!s.shadow_capture(PHONE));
            s.terminate();
        }

        /// Ignoring SIGWINCH but printing: what it printed in the window is
        /// no redraw and reaches the Term, in order, none lost.
        #[test]
        fn shadow_held_output_reaches_the_term_without_an_answer() {
            // One write, released by the test once the capture is open: on a
            // busy machine a timed program printed across the signal back,
            // which reads as an answer.
            let go = test_dir("print").join("go");
            let (s, _dir) = spawn(
                "print",
                &format!(
                    "trap '' WINCH; printf '\\033[?1049hSIZE 43 136'; \
                     while [ ! -e {go} ]; do sleep 0.01; done; \
                     printf '\\r\\nLINE1\\r\\nLINE2\\r\\nLINE3'; sleep 60",
                    go = go.display()
                ),
            );
            until("up", 10, || real_text(&s).contains("SIZE 43 136"));
            std::thread::sleep(Duration::from_millis(300));
            assert!(s.shadow_capture(PHONE));
            std::fs::write(&go, "").unwrap();
            let st = stats_after(&s, "gave up", |st| st.no_answer == 1);
            assert_eq!(st.framed, 0);
            until("held lines on the term", 3, || {
                let t = real_text(&s);
                t.contains("LINE1") && t.contains("LINE2") && t.contains("LINE3")
            });
            s.terminate();
        }

        /// The primary screen is never signalled.
        #[test]
        fn shadow_primary_screen_is_never_signalled() {
            let (s, dir) = spawn(
                "primary",
                &format!(
                    "trap 'echo x >> {}/winch' WINCH; printf 'PLAIN\\n'; \
                     while :; do sleep 0.02 & wait $!; done",
                    test_dir("primary").display()
                ),
            );
            until("up", 10, || real_text(&s).contains("PLAIN"));
            std::thread::sleep(Duration::from_millis(400));
            assert_eq!(s.shadow_due(PHONE, Instant::now()), ShadowDue::No);
            assert!(!s.shadow_capture(PHONE));
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(s.shadow_stats(), ShadowStats::default());
            assert!(!dir.join("winch").exists());
            s.terminate();
        }

        /// A shell marker in the window: the capture ends and everything
        /// goes on the Term, the marker's block too.
        #[test]
        fn shadow_marker_aborts_and_feeds_the_term() {
            let dir = test_dir("marker");
            let (s, _dir) = spawn(
                "marker",
                &program(
                    &dir,
                    ALT,
                    "case $s in 30*) printf 'BEFORE\\033]133;A\\007AFTER';; esac;",
                ),
            );
            ready(&s);
            assert!(s.shadow_capture(PHONE));
            let st = stats_after(&s, "aborted", |st| st.aborted == 1);
            assert_eq!((st.framed, st.signals), (0, 2));
            assert!(frame_text(&s).is_none());
            // told its own size back: it repaints; the marker reached the store
            until("repainted", 3, || real_text(&s).contains("SIZE 43 136"));
            // the reader thread applies the marker after feeding the bytes before it
            until("the marker's block", 3, || {
                s.blocks.lock().unwrap().last().is_some()
            });
            s.terminate();
        }

        /// The program leaves the alternate screen in the window: the Term
        /// leaves it too, with no shell marker at all.
        #[test]
        fn shadow_leaving_the_alt_screen_reaches_the_term() {
            use alacritty_terminal::term::TermMode;
            let dir = test_dir("leave");
            let (s, _dir) = spawn(
                "leave",
                &program(
                    &dir,
                    ALT,
                    "case $s in 30*) printf '\\033[?1049lBYE';; esac;",
                ),
            );
            ready(&s);
            assert!(s.shadow_capture(PHONE));
            stats_after(&s, "aborted", |st| st.aborted == 1);
            until("left", 3, || {
                !s.term.lock().mode().contains(TermMode::ALT_SCREEN)
            });
            s.terminate();
        }

        /// The Mac window resizes in the window: the program ends at the new
        /// size, not at one a late signal brought back.
        #[test]
        fn shadow_resize_during_a_capture_wins() {
            let dir = test_dir("resize");
            let (mut s, dir) = spawn("resize", &program(&dir, ALT, ""));
            ready(&s);
            assert!(s.shadow_capture(PHONE));
            s.resize(
                GridSize {
                    cols: 100,
                    rows: 40,
                },
                (8, 16),
            );
            until("redrew at the new size", 3, || {
                real_text(&s).contains("SIZE 40 100")
            });
            until("capture over", 3, || {
                s.shadow.state.lock().unwrap().capture.is_none()
            });
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(sizes(&dir).last().unwrap(), "40 100", "{:?}", sizes(&dir));
            assert_eq!(s.shadow_stats().framed, 0);
            // the resize's own redraw went to the Term: nothing to give back
            assert_eq!(s.shadow_stats().no_answer, 0);
            s.terminate();
        }

        /// The pane closes in the window: no panic, no thread left holding it.
        #[test]
        fn shadow_pane_closed_during_a_capture() {
            let dir = test_dir("close");
            let (s, _dir) = spawn("close", &program(&dir, ALT, ""));
            ready(&s);
            assert!(s.shadow_capture(PHONE));
            let shadow = s.shadow.clone();
            s.terminate();
            drop(s);
            until("driver gone", 3, || Arc::strong_count(&shadow) == 1);
            // given up, not finished: no frame for a pane that is gone
            let state = shadow.state.lock().unwrap();
            assert!(state.closed && state.frame.is_none() && state.capture.is_none());
        }

        /// Input in the window: no frame, the answer lands on the Term.
        #[test]
        fn shadow_input_ends_a_capture() {
            let dir = test_dir("input");
            let (s, dir) = spawn("input", &program(&dir, ALT, ""));
            ready(&s);
            assert!(s.shadow_capture(PHONE));
            s.write(b"x");
            until("capture over", 3, || {
                s.shadow.state.lock().unwrap().capture.is_none()
            });
            assert_eq!(s.shadow_stats().framed, 0);
            until("repainted", 3, || real_text(&s).contains("SIZE 43 136"));
            assert_eq!(sizes(&dir).last().unwrap(), "43 136");
            s.terminate();
        }

        /// Output moved on: the frame says how far behind, and a new one is due.
        #[test]
        fn shadow_frame_falls_behind_output() {
            let (s, _d) = spawn(
                "behind",
                &format!(
                    "exec 2>/dev/null; {ALT}\n\
                     draw() {{ s=$(stty size </dev/tty); printf '\\033[2J\\033[HSIZE %s' \"$s\"; }}\n\
                     trap draw WINCH; draw\n\
                     while :; do read -r l && printf '\\r\\nGOT%s' \"$l\"; done\n"
                ),
            );
            ready(&s);
            assert!(s.shadow_capture(PHONE));
            stats_after(&s, "framed", |st| st.framed == 1);
            assert_eq!(s.shadow_frame(PHONE, |_, _, b| b), Some(None));
            s.write(b"1\n");
            until("output", 3, || real_text(&s).contains("GOT1"));
            let behind = s.shadow_frame(PHONE, |_, _, b| b).unwrap();
            assert!(behind.is_some());
            match s.shadow_due(PHONE, Instant::now()) {
                ShadowDue::At(_) | ShadowDue::Now => {}
                ShadowDue::No => panic!("no new capture due"),
            }
            s.terminate();
        }
    }
}
