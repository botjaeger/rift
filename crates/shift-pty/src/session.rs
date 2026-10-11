use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, OnResize, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte;
use alacritty_terminal::vte::ansi::Rgb;

use shift_core::{BlockStore, Marker, OscScanner};

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
    Exit,
}

/// Formats a color into the reply escape sequence the program asked for.
#[derive(Clone)]
pub struct ColorReply(pub Arc<dyn Fn(Rgb) -> String + Send + Sync>);

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
    stream: std::os::unix::net::UnixStream,
    /// `Some` while the connection is down: input waits here, in order, and
    /// goes out on the replacement (see [`ServiceConn::reconnected`]).
    pending: Option<Vec<u8>>,
    /// Input bytes dropped because `pending` was full.
    dropped: usize,
}

impl ServiceConn {
    fn new(stream: std::os::unix::net::UnixStream) -> Self {
        Self {
            stream,
            pending: None,
            dropped: 0,
        }
    }

    /// Send input, or hold it while the connection is down. A frame is
    /// written whole or not at all as far as the service is concerned (it
    /// drops a partial frame with the connection), so the chunk that failed
    /// and everything after it are held, never one already sent.
    fn input(&mut self, session: u64, bytes: &[u8]) {
        if self.pending.is_none() {
            for (i, chunk) in bytes.chunks(crate::ipc::MAX_INPUT_CHUNK).enumerate() {
                if crate::ipc::send_input(&mut self.stream, session, chunk).is_err() {
                    self.pending = Some(Vec::new());
                    self.hold(&bytes[i * crate::ipc::MAX_INPUT_CHUNK..]);
                    return;
                }
            }
            return;
        }
        self.hold(bytes);
    }

    fn hold(&mut self, bytes: &[u8]) {
        let pending = self.pending.get_or_insert_with(Vec::new);
        let room = MAX_PENDING_INPUT.saturating_sub(pending.len());
        pending.extend_from_slice(&bytes[..bytes.len().min(room)]);
        self.dropped += bytes.len().saturating_sub(room);
    }

    /// The connection dropped: hold input from now on.
    fn down(&mut self) {
        self.pending.get_or_insert_with(Vec::new);
    }

    /// Switch to `stream` and send the input held meanwhile. Returns the
    /// number of input bytes that were dropped (pending was full).
    fn reconnected(&mut self, stream: std::os::unix::net::UnixStream, session: u64) -> usize {
        self.stream = stream;
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
        session: u64,
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
                conn.lock().unwrap().input(*session, bytes);
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
            Event::ColorRequest(index, format) => {
                (self.observer)(
                    self.pane,
                    SessionEvent::ColorRequest(index, ColorReply(format)),
                );
            }
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
    /// Shell state reported by OSC 1337 SetUserVar (prompt chips).
    pub user_vars: UserVars,
    /// Set by the reader thread when new output arrived; cleared by the app
    /// when it consumes a snapshot. Coalesces wakeup events.
    pub dirty: Arc<AtomicBool>,
    /// True once the shell produced its first byte ("Starting zsh…" gate).
    pub saw_output: Arc<AtomicBool>,
    /// Id of the shell in the session service (durable sessions).
    pub service_id: Option<u64>,
    writer: PtyWriter,
    backend: Backend,
    size: GridSize,
    window_size: Arc<Mutex<WindowSize>>,
    scrollback: usize,
}

/// A transcript to replay: terminal bytes, and (duration, cwd, command) per
/// block.
pub type Replay = (Vec<u8>, Vec<shift_core::Restored>);

/// Who owns the shell.
enum Backend {
    /// This process: the PTY closes (and the shell gets SIGHUP) with it.
    Local(tty::Pty),
    /// The session service: the shell outlives this window.
    Service {
        conn: Arc<Mutex<ServiceConn>>,
        session: u64,
        /// Foreground job reported by the service.
        job: Arc<Mutex<Option<String>>>,
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

/// Start `shell` on a new PTY with Shift's environment (TERM, shell
/// integration). The master fd is left blocking for a reader thread.
pub(crate) fn open_pty(
    shell: Option<&str>,
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
    for (k, v) in shift_core::shell_integration::setup() {
        tty_env.entry(k).or_insert(v);
    }
    // always name the shell explicitly so bash can get its --rcfile shim
    let program = shift_core::shell_integration::resolve_shell(shell);
    let args = shift_core::shell_integration::shell_args(&program);
    let tty_opts = tty::Options {
        shell: Some(tty::Shell::new(program, args)),
        working_directory,
        drain_on_exit: false,
        env: tty_env,
    };
    let pty = tty::new(&tty_opts, window_size, id)?;
    // alacritty configures the master fd non-blocking (it polls with mio);
    // we use a dedicated blocking reader thread instead
    unsafe {
        use std::os::fd::AsRawFd;
        let fd = pty.file().as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
    }
    Ok(pty)
}

/// Executable name of the PTY's foreground job when it is not the shell.
pub(crate) fn foreground_job_name(pty: &tty::Pty) -> Option<String> {
    use std::os::fd::AsRawFd;
    let pgrp = unsafe { libc::tcgetpgrp(pty.file().as_raw_fd()) };
    let shell = pty.child().id() as i32;
    if pgrp <= 0 || pgrp == shell {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        let mut buf = [0u8; 256];
        let n = unsafe { libc::proc_name(pgrp, buf.as_mut_ptr().cast(), buf.len() as u32) };
        Some(if n > 0 {
            String::from_utf8_lossy(&buf[..n as usize]).into_owned()
        } else {
            String::new()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        Some(
            std::fs::read_to_string(format!("/proc/{pgrp}/comm"))
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
        )
    }
}

pub struct SpawnOptions {
    pub id: PaneId,
    pub size: GridSize,
    pub cell_size: (u16, u16),
    pub shell: Option<String>,
    pub working_directory: Option<std::path::PathBuf>,
    pub env: Vec<(String, String)>,
    pub scrollback: usize,
    pub observer: Observer,
    /// Saved transcript to show above the new shell: terminal bytes with
    /// OSC 133 block markers, parsed before the shell's first byte, and the
    /// (duration, cwd) of each replayed block.
    pub replay: Option<Replay>,
    /// Run the shell in the session service (durable sessions).
    pub service: Option<ServiceTarget>,
    /// What OSC 52 may do with the system clipboard.
    pub clipboard: ClipboardAccess,
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
    Pty(File),
    Service {
        stream: std::os::unix::net::UnixStream,
        muted: Arc<AtomicBool>,
        job: Arc<Mutex<Option<String>>>,
        /// The connection `PtyWriter` sends input on (replaced on resume).
        conn: Arc<Mutex<ServiceConn>>,
        session: u64,
        /// Reattach after the byte `u64` (see [`crate::service::resume`]).
        resume: Resume,
    },
}

/// Opens a fresh service connection streaming output after the given byte;
/// true when output before it was no longer retained.
type Resume = Box<dyn FnMut(u64) -> std::io::Result<(std::os::unix::net::UnixStream, bool)> + Send>;

/// Consecutive failed resumes (no output in between) before the pane gives up.
const MAX_RESUMES: u32 = 5;

/// The pane's reader thread: parses output into the Term and block model.
struct Reader {
    id: PaneId,
    ctx: FeedCtx,
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

    /// Before blocking on input: a synchronized update (DECSET 2026) holds
    /// output until its end marker; if the program stalls, show what it
    /// drew once vte's timeout expires. Returns true when it flushed.
    fn sync_timeout(
        &self,
        parser: &mut vte::ansi::Processor,
        fd: &impl std::os::fd::AsRawFd,
    ) -> bool {
        let Some(deadline) = parser.sync_timeout().sync_timeout() else {
            return false;
        };
        let wait = deadline.saturating_duration_since(std::time::Instant::now());
        if wait_readable(fd, wait) {
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
        drop(term);
        self.wake();
        true
    }

    fn notice(&self, message: &str) {
        (self.observer)(self.id, SessionEvent::Notice(message.into()));
    }

    fn run(self, source: Source) {
        let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
        let mut scanner = OscScanner::new();
        match source {
            Source::Pty(mut file) => {
                let mut buf = [0u8; 65536];
                loop {
                    if self.sync_timeout(&mut parser, &file) {
                        continue;
                    }
                    match file.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            self.saw_output.store(true, Ordering::Release);
                            feed(&mut parser, &mut scanner, &buf[..n], &self.ctx);
                            self.wake();
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            }
            Source::Service {
                mut stream,
                muted,
                job,
                conn,
                session,
                mut resume,
            } => {
                use crate::ipc::{Event as Ev, Frame, read_frame};
                // first byte not yet fed to the parser, and resumes since
                // the last output
                let (mut next_seq, mut resumes) = (0u64, 0u32);
                loop {
                    if self.sync_timeout(&mut parser, &stream) {
                        continue;
                    }
                    let frame = match read_frame(&mut stream) {
                        Ok(Some(f)) => f,
                        // The socket ended without an `Exited` frame: the
                        // service dropped this (slow) client, or it died.
                        // The shell may well be running: reattach after the
                        // last byte seen; only a session that is really gone
                        // closes the pane.
                        _ => {
                            resumes += 1;
                            if resumes > MAX_RESUMES {
                                break;
                            }
                            muted.store(true, Ordering::Release);
                            // typing goes on while we reconnect: hold it
                            conn.lock().unwrap().down();
                            std::thread::sleep(std::time::Duration::from_millis(
                                100 * u64::from(resumes),
                            ));
                            let Ok((fresh, truncated)) = resume(next_seq) else {
                                break;
                            };
                            let Ok(writer) = fresh.try_clone() else { break };
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
                            let replaying = muted.load(Ordering::Acquire);
                            if replaying {
                                let age = crate::ipc::now_ms().saturating_sub(at_ms);
                                let at = std::time::Instant::now()
                                    .checked_sub(std::time::Duration::from_millis(age));
                                self.ctx.blocks.lock().unwrap().set_clock(at);
                            }
                            feed(&mut parser, &mut scanner, &bytes, &self.ctx);
                            if !replaying {
                                self.wake();
                            }
                        }
                        Frame::Json(body) => match serde_json::from_slice::<Ev>(&body) {
                            Ok(Ev::ReplayDone { .. }) => {
                                self.ctx.blocks.lock().unwrap().set_clock(None);
                                muted.store(false, Ordering::Release);
                                self.wake();
                            }
                            Ok(Ev::Status { job: j, .. }) => {
                                *job.lock().unwrap() = j;
                                self.wake();
                            }
                            Ok(Ev::Exited { .. }) => break,
                            _ => {}
                        },
                        Frame::Input { .. } => {}
                    }
                }
            }
        }
        (self.observer)(self.id, SessionEvent::Exit);
    }
}

impl Session {
    pub fn spawn(opts: SpawnOptions) -> std::io::Result<Session> {
        let SpawnOptions {
            id,
            size,
            cell_size,
            shell,
            working_directory,
            env,
            scrollback,
            observer,
            replay,
            service,
            clipboard,
        } = opts;

        let window_size = WindowSize {
            num_lines: size.rows as u16,
            num_cols: size.cols as u16,
            cell_width: cell_size.0,
            cell_height: cell_size.1,
        };
        let window_size_shared = Arc::new(Mutex::new(window_size));

        // the shell: a local PTY, or one owned by the session service
        let (backend, writer, source) = match service {
            None => {
                let pty = open_pty(shell.as_deref(), working_directory, env, window_size, id)?;
                let writer = PtyWriter::File(Arc::new(pty.file().try_clone()?));
                let source = Source::Pty(pty.file().try_clone()?);
                (Backend::Local(pty), writer, source)
            }
            Some(target) => {
                let (stream, session) = crate::service::open(
                    target,
                    shell.as_deref(),
                    working_directory.as_deref(),
                    env,
                    window_size,
                )?;
                let conn = Arc::new(Mutex::new(ServiceConn::new(stream.try_clone()?)));
                let muted = Arc::new(AtomicBool::new(true));
                let job = Arc::new(Mutex::new(None));
                let writer = PtyWriter::Service {
                    conn: conn.clone(),
                    session,
                    muted: muted.clone(),
                };
                let source = Source::Service {
                    stream,
                    muted,
                    job: job.clone(),
                    conn: conn.clone(),
                    session,
                    resume: Box::new(move |from| crate::service::resume(session, from)),
                };
                (Backend::Service { conn, session, job }, writer, source)
            }
        };
        let service_id = match &backend {
            Backend::Service { session, .. } => Some(*session),
            Backend::Local(_) => None,
        };

        let term_config = TermConfig {
            scrolling_history: scrollback,
            // programs opt in with `CSI > flags u`; the app's key encoder
            // follows the resulting mode bits
            kitty_keyboard: true,
            xtversion: format!("Shift({})", env!("CARGO_PKG_VERSION")),
            osc52: clipboard.osc52(),
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
        let blocks = Arc::new(Mutex::new(BlockStore::default()));
        let scrolled = Arc::new(AtomicU64::new(0));
        let cwd = Arc::new(Mutex::new(None));
        let user_vars = UserVars::default();
        let graphics = Arc::new(Mutex::new(Graphics::default()));
        let ctx = FeedCtx {
            term: term.clone(),
            scrolled: scrolled.clone(),
            scrollback,
            blocks: blocks.clone(),
            cwd: cwd.clone(),
            user_vars: user_vars.clone(),
            graphics: graphics.clone(),
            window_size: window_size_shared.clone(),
            reply: Some(replier),
            notify: Some((id, observer.clone())),
        };

        // replay a saved transcript before the reader thread starts, so the
        // shell's first prompt lands below it
        if let Some((bytes, meta)) = replay.filter(|(b, _)| !b.is_empty()) {
            let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
            let mut scanner = OscScanner::new();
            feed(&mut parser, &mut scanner, &bytes, &ctx);
            blocks.lock().unwrap().mark_restored(&meta);
        }

        let reader = Reader {
            id,
            ctx,
            dirty: dirty.clone(),
            saw_output: saw_output.clone(),
            observer,
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
            user_vars,
            dirty,
            saw_output,
            service_id,
            writer,
            backend,
            size,
            window_size: window_size_shared,
            scrollback,
        })
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    /// Write user input to the shell.
    pub fn write(&self, bytes: &[u8]) {
        self.writer.write(bytes);
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
        if let Backend::Service { conn, session, .. } = &self.backend {
            let _ = crate::ipc::send_json(
                &mut conn.lock().unwrap().stream,
                &crate::ipc::Request::Kill { session: *session },
            );
        }
    }

    /// ⌘K Clear Blocks: wipe screen, scrollback and the block list in one
    /// atomic step (no-op on the alt screen). Fully app-side — the shell is
    /// never told, so no OSC re-emission race. The abs-line invariant holds:
    /// AbsTerm counts the rows ClearMode::All pushes into history, and
    /// ClearMode::Saved drops history without pushing, so `scrolled` stays
    /// monotone and `abs = scrolled + screen_row` remains valid.
    pub fn clear_screen_and_blocks(&self) {
        use alacritty_terminal::term::TermMode;
        use alacritty_terminal::vte::ansi::{ClearMode, Handler};

        let mut term = self.term.lock();
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        let mut scrolled = self.scrolled.load(Ordering::Acquire);
        {
            let mut abs = AbsTerm {
                term: &mut term,
                scrolled: &mut scrolled,
                scrollback_cap: self.scrollback,
                saw_clear_saved: false,
                on_resize: None,
            };
            abs.clear_screen(ClearMode::All);
            abs.clear_screen(ClearMode::Saved);
        }
        self.scrolled.store(scrolled, Ordering::Release);
        term.selection = None;

        // Re-anchor a fresh Prompt block at the post-clear cursor: the shell
        // still believes its prompt is on the (now cleared) row the cursor
        // occupies, so the next submitted command echoes exactly there and
        // header extraction stays correct.
        let cursor = term.grid().cursor.point;
        let abs_line = scrolled + cursor.line.0.max(0) as u64;
        let mut blocks = self.blocks.lock().unwrap();
        let was_prompt = matches!(
            blocks.last().map(|b| &b.state),
            Some(shift_core::BlockState::Prompt)
        );
        blocks.clear();
        self.graphics.lock().unwrap().clear();
        // a reattach must not bring the cleared output back
        if let Backend::Service { conn, session, .. } = &self.backend {
            let _ = crate::ipc::send_json(
                &mut conn.lock().unwrap().stream,
                &crate::ipc::Request::ClearHistory { session: *session },
            );
        }
        if was_prompt {
            blocks.on_prompt_start(abs_line);
            blocks.on_command_start(abs_line, cursor.column.0);
        }
    }

    /// Change the OSC 52 policy of the running shell.
    pub fn set_clipboard_access(&self, access: ClipboardAccess) {
        self.term.lock().set_osc52(access.osc52());
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
        );
        match &mut self.backend {
            Backend::Local(pty) => pty.on_resize(ws),
            Backend::Service { conn, session, .. } => {
                let _ = crate::ipc::send_json(
                    &mut conn.lock().unwrap().stream,
                    &crate::ipc::Request::Resize {
                        session: *session,
                        cols: ws.num_cols,
                        rows: ws.num_lines,
                        cell_w: ws.cell_width,
                        cell_h: ws.cell_height,
                    },
                );
            }
        }
        drop(term);
    }
}

/// Resize the grid and keep every abs anchor on its line. Row-count changes
/// move lines between history and screen; keep `abs = scrolled + screen_row`
/// coherent by folding the history delta into the counter (same term lock
/// the reader thread uses).
fn resize_term(
    term: &mut Term<EventProxy>,
    size: GridSize,
    scrolled: &AtomicU64,
    blocks: &Mutex<BlockStore>,
    graphics: &Mutex<Graphics>,
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
    scrolled.store(base + new.history as u64, Ordering::Release);
    // reflow (column change) re-wraps lines and moves every row after
    // them; carry the block anchors along (same lock order as the reader)
    // a prompt redrawn after this keeps its pre-resize anchor
    blocks.lock().unwrap().note_resize();
    if old.cols != new.cols || old.starts.len() != new.starts.len() {
        blocks
            .lock()
            .unwrap()
            .remap(|abs, col| old.map(&new, base, abs, col));
        graphics
            .lock()
            .unwrap()
            .remap(|abs, col| old.map(&new, base, abs, col));
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
        }
    }

    /// Where (`abs`, `col`) in this layout lands in `new`.
    fn map(&self, new: &RowLayout, base: u64, abs: u64, col: usize) -> (u64, usize) {
        let Some(row) = abs.checked_sub(base).map(|r| r as usize) else {
            return (abs, col); // already trimmed out of history
        };
        if row >= self.rows || self.starts.is_empty() || new.starts.is_empty() {
            // past the grid (an end anchor on the last row): keep its
            // distance from the bottom
            let shifted = row as i64 + new.rows as i64 - self.rows as i64;
            return (base + shifted.max(0) as u64, col);
        }
        let line = self.starts.partition_point(|&s| s <= row) - 1;
        let offset = (row - self.starts[line]) * self.cols + col.min(self.cols);
        let target = line as i64 - self.cursor_line as i64 + new.cursor_line as i64;
        if target < 0 {
            return (base.saturating_sub(1), 0); // reflow pushed it out of history
        }
        let target = target as usize;
        if target >= new.starts.len() {
            return (base + new.rows as u64, 0);
        }
        let start = new.starts[target];
        let next = new.starts.get(target + 1).copied().unwrap_or(new.rows);
        let cols = new.cols.max(1);
        let sub = (offset / cols).min(next - start - 1);
        (base + (start + sub) as u64, (offset - sub * cols).min(cols))
    }
}

/// Parse one chunk of terminal output: split it at shell markers, feed each
/// piece to the VT parser, apply the markers to the block model, then prune
/// blocks that left the scrollback.
/// Shell variables from OSC 1337 SetUserVar, by name.
pub type UserVars = Arc<Mutex<std::collections::HashMap<String, String>>>;

/// Distinct user variables a session keeps.
const MAX_USER_VARS: usize = 32;

/// Everything the output parser writes to (shared with the app).
struct FeedCtx {
    term: Arc<FairMutex<Term<EventProxy>>>,
    scrolled: Arc<AtomicU64>,
    scrollback: usize,
    blocks: Arc<Mutex<BlockStore>>,
    cwd: Arc<Mutex<Option<String>>>,
    user_vars: UserVars,
    graphics: Arc<Mutex<Graphics>>,
    window_size: Arc<Mutex<WindowSize>>,
    /// PTY write-back for graphics protocol replies.
    reply: Option<Replier>,
    /// Where notification requests (OSC 9/777/99) go.
    notify: Option<(PaneId, Observer)>,
}

/// Parse one chunk of terminal output: split it at shell markers, feed each
/// piece to the VT parser, apply the markers to the block and image models,
/// then prune what left the scrollback.
fn feed(parser: &mut vte::ansi::Processor, scanner: &mut OscScanner, bytes: &[u8], ctx: &FeedCtx) {
    use alacritty_terminal::term::TermMode;
    let segments = scanner.scan(bytes);
    let mut term = ctx.term.lock();
    // the atomic is canonical: resizes adjust it (under the same term lock)
    // between chunks
    let mut scrolled_cur = ctx.scrolled.load(Ordering::Acquire);
    let mut start = 0usize;
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
        );
        *scrolled = counter.load(Ordering::Acquire);
        let mut ws = ctx.window_size.lock().unwrap();
        ws.num_lines = rows as u16;
        ws.num_cols = cols as u16;
    };
    for seg in segments {
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
            Some(Marker::Notify { title, body }) => {
                // replayed output already notified once
                let replaying = matches!(&ctx.reply, Some(w) if w.muted());
                if let (Some((id, observer)), false) = (&ctx.notify, replaying) {
                    observer(*id, SessionEvent::Notify { title, body });
                }
            }
            Some(marker) => {
                if matches!(marker, Marker::Done { .. })
                    && ctx.blocks.lock().unwrap().any_running()
                    && end_partial_line(parser, &mut term, &mut scrolled_cur, ctx.scrollback)
                {
                    ctx.blocks.lock().unwrap().mark_unterminated();
                }
                apply_marker(marker, &term, scrolled_cur, &ctx.blocks, &ctx.cwd)
            }
            None => {}
        }
    }
    // prune blocks and images that fell out of scrollback; the alt grid has
    // no history, so only the primary grid says what is still kept
    if !term.mode().contains(TermMode::ALT_SCREEN) {
        let oldest = scrolled_cur.saturating_sub(term.grid().history_size() as u64);
        ctx.blocks.lock().unwrap().prune(oldest);
        ctx.graphics.lock().unwrap().prune(oldest);
    }
    ctx.scrolled.store(scrolled_cur, Ordering::Release);
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

/// Block until `file` has bytes to read or `wait` passes. False on timeout.
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

fn apply_marker(
    marker: Marker,
    term: &Term<EventProxy>,
    scrolled: u64,
    blocks: &Arc<Mutex<BlockStore>>,
    cwd: &Arc<Mutex<Option<String>>>,
) {
    let cursor = term.grid().cursor.point;
    let abs_line = scrolled + cursor.line.0.max(0) as u64;
    log::debug!(
        "marker {marker:?} at abs={abs_line} (cursor line {}, scrolled {scrolled})",
        cursor.line.0
    );
    let mut store = blocks.lock().unwrap();
    match marker {
        Marker::PromptStart => store.on_prompt_start(abs_line),
        Marker::CommandStart => store.on_command_start(abs_line, cursor.column.0),
        Marker::OutputStart => {
            let command = store
                .last()
                .and_then(|b| b.command_start)
                .map(|(line, col)| extract_text(term, scrolled, line, col, abs_line))
                .unwrap_or_default();
            store.on_output_start(abs_line, command);
            // header context fallback for raw-passthrough commands (the app
            // stamps cwd+branch at submit; this only fills gaps)
            store.stamp_context(cwd.lock().unwrap().clone(), None);
        }
        Marker::Done { exit } => store.on_done(abs_line, exit),
        Marker::Cwd(path) => {
            *cwd.lock().unwrap() = Some(path);
        }
        // handled by `feed` (they need the parser to move the cursor)
        Marker::Graphics(_)
        | Marker::InlineImage(_)
        | Marker::Notify { .. }
        | Marker::UserVar { .. } => {}
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
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn term(cols: usize, rows: usize) -> Term<EventProxy> {
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
        Term::new(TermConfig::default(), &GridSize { cols, rows }, proxy)
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
        assert_eq!(old.map(&new, 0, 2, 0), (3, 0));
        assert_eq!(old.map(&new, 0, 3, 0), (4, 0));
        // a point inside the wrapped line keeps its character offset
        assert_eq!(old.map(&new, 0, 1, 5), (2, 5));
        // below the tracked base: untouched
        assert_eq!(old.map(&new, 10, 3, 0), (3, 0));
    }

    fn ctx(cols: usize, rows: usize) -> FeedCtx {
        FeedCtx {
            term: Arc::new(FairMutex::new(term(cols, rows))),
            scrolled: Arc::new(AtomicU64::new(0)),
            // the grid's own cap (TermConfig::default)
            scrollback: 10_000,
            blocks: Arc::new(Mutex::new(BlockStore::default())),
            cwd: Arc::new(Mutex::new(None)),
            user_vars: UserVars::default(),
            graphics: Arc::new(Mutex::new(Graphics::default())),
            window_size: Arc::new(Mutex::new(WindowSize {
                num_lines: rows as u16,
                num_cols: cols as u16,
                cell_width: 8,
                cell_height: 16,
            })),
            reply: None,
            notify: None,
        }
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

    /// A pane's reader over a fake service connection; the returned receiver
    /// yields the pane's `Exit` events.
    fn service_reader(
        c: FeedCtx,
        stream: std::os::unix::net::UnixStream,
        resume: Resume,
    ) -> (std::thread::JoinHandle<()>, std::sync::mpsc::Receiver<()>) {
        let (conn, _conn_peer) = std::os::unix::net::UnixStream::pair().unwrap();
        std::mem::forget(_conn_peer);
        let (exit_tx, exit_rx) = std::sync::mpsc::channel();
        let exit_tx = Mutex::new(exit_tx);
        let reader = Reader {
            id: 1,
            ctx: c,
            dirty: Arc::new(AtomicBool::new(false)),
            saw_output: Arc::new(AtomicBool::new(false)),
            observer: Arc::new(move |_, ev| {
                if matches!(ev, SessionEvent::Exit) {
                    let _ = exit_tx.lock().unwrap().send(());
                }
            }),
        };
        let source = Source::Service {
            stream,
            muted: Arc::new(AtomicBool::new(false)),
            job: Arc::new(Mutex::new(None)),
            conn: Arc::new(Mutex::new(ServiceConn::new(conn))),
            session: 9,
            resume,
        };
        (std::thread::spawn(move || reader.run(source)), exit_rx)
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
        send_json(&mut second_peer, &Ev::Exited { session: 9 }).unwrap();
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
        conn.stream.shutdown(std::net::Shutdown::Write).unwrap();
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
    fn input_held_past_the_bound_is_cut_and_counted() {
        use std::os::unix::net::UnixStream;
        let (old, _old_peer) = UnixStream::pair().unwrap();
        let mut conn = ServiceConn::new(old);
        conn.down();
        conn.input(9, &vec![b'x'; MAX_PENDING_INPUT - 1]);
        conn.input(9, b"yz!");
        let (new, new_peer) = UnixStream::pair().unwrap();
        // more than a socket buffer: read while it is sent
        let got = std::thread::spawn(move || received_input(new_peer));
        assert_eq!(conn.reconnected(new, 9), 2, "two bytes did not fit");
        drop(conn);
        let got = got.join().unwrap();
        assert_eq!(got.len(), MAX_PENDING_INPUT);
        assert!(got.ends_with(b"xy"));
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
        send_json(&mut second_peer, &Ev::Exited { session: 9 }).unwrap();
        let (conn, _conn_peer) = UnixStream::pair().unwrap();
        let notices = Arc::new(Mutex::new(Vec::new()));
        let (seen, (done_tx, done)) = (notices.clone(), std::sync::mpsc::channel());
        let done_tx = Mutex::new(done_tx);
        let reader = Reader {
            id: 1,
            ctx: c,
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
        let source = Source::Service {
            stream: first,
            muted: Arc::new(AtomicBool::new(false)),
            job: Arc::new(Mutex::new(None)),
            conn: Arc::new(Mutex::new(ServiceConn::new(conn))),
            session: 9,
            resume: Box::new(move |_| {
                let s = second.take().ok_or(std::io::ErrorKind::NotFound)?;
                Ok((s, true))
            }),
        };
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

    #[test]
    fn vanished_service_session_still_closes_the_pane() {
        use crate::ipc::send_output;
        use std::os::unix::net::UnixStream;
        let (first, mut first_peer) = UnixStream::pair().unwrap();
        send_output(&mut first_peer, 9, 0, 1, b"ab").unwrap();
        drop(first_peer);
        let (reader, exited) = service_reader(
            ctx(20, 5),
            first,
            Box::new(|_| Err(std::io::Error::from(std::io::ErrorKind::NotFound))),
        );
        exited
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        reader.join().unwrap();
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
            user_vars: c.user_vars.clone(),
            dirty: Arc::new(AtomicBool::new(false)),
            saw_output: Arc::new(AtomicBool::new(false)),
            service_id: None,
            writer,
            backend: Backend::Local(pty),
            size: GridSize { cols: 80, rows: 24 },
            window_size: c.window_size.clone(),
            scrollback: 100,
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
}
