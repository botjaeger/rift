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
    /// OSC 4/10/11/12 query: reply with palette index `usize`'s color.
    ColorRequest(usize, ColorReply),
    /// The program asked for a notification (OSC 9 / 777 / 99).
    Notify {
        title: String,
        body: String,
    },
    Bell,
    Exit,
}

/// Formats a color into the reply escape sequence the program asked for.
#[derive(Clone)]
pub struct ColorReply(pub Arc<dyn Fn(Rgb) -> String + Send + Sync>);

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

/// Where bytes for the shell go: the local PTY, or the session service.
#[derive(Clone)]
pub(crate) enum PtyWriter {
    File(Arc<File>),
    Service {
        conn: Arc<Mutex<std::os::unix::net::UnixStream>>,
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
                let _ = crate::ipc::send_input(&mut *conn.lock().unwrap(), *session, bytes);
            }
        }
    }

    /// Replaying retained output after a (re)attach.
    fn muted(&self) -> bool {
        matches!(self, PtyWriter::Service { muted, .. } if muted.load(Ordering::Acquire))
    }

    /// Terminal-generated replies to the program (dropped during replay).
    fn reply(&self, bytes: &[u8]) {
        if !self.muted() {
            self.write(bytes);
        }
    }
}

/// Bridges Term-emitted events to the observer + PTY write-back.
pub struct EventProxy {
    pane: PaneId,
    observer: Observer,
    writer: PtyWriter,
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
            // copy-only OSC 52: reading the clipboard back would let any
            // program (or remote host) exfiltrate it
            Event::ClipboardStore(_, text) => {
                (self.observer)(self.pane, SessionEvent::Clipboard(text));
            }
            Event::ColorRequest(index, format) => {
                (self.observer)(
                    self.pane,
                    SessionEvent::ColorRequest(index, ColorReply(format)),
                );
            }
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

/// A transcript to replay: terminal bytes, and (duration, cwd) per block.
pub type Replay = (Vec<u8>, Vec<(std::time::Duration, Option<String>)>);

/// Who owns the shell.
enum Backend {
    /// This process: the PTY closes (and the shell gets SIGHUP) with it.
    Local(tty::Pty),
    /// The session service: the shell outlives this window.
    Service {
        conn: Arc<Mutex<std::os::unix::net::UnixStream>>,
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
}

/// Where the reader thread gets output from.
enum Source {
    Pty(File),
    Service {
        stream: std::os::unix::net::UnixStream,
        muted: Arc<AtomicBool>,
        job: Arc<Mutex<Option<String>>>,
    },
}

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
        };
        parser.stop_sync(&mut abs);
        self.ctx.scrolled.store(scrolled_cur, Ordering::Release);
        drop(term);
        self.wake();
        true
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
            } => {
                use crate::ipc::{Event as Ev, Frame, read_frame};
                loop {
                    if self.sync_timeout(&mut parser, &stream) {
                        continue;
                    }
                    let frame = match read_frame(&mut stream) {
                        Ok(Some(f)) => f,
                        _ => break,
                    };
                    match frame {
                        Frame::Output { bytes, at_ms, .. } => {
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
                let conn = Arc::new(Mutex::new(stream.try_clone()?));
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
            ..TermConfig::default()
        };
        let term = Term::new(
            term_config,
            &size,
            EventProxy {
                pane: id,
                observer: observer.clone(),
                writer: writer.clone(),
                window_size: window_size_shared.clone(),
            },
        );
        let term = Arc::new(FairMutex::new(term));
        let dirty = Arc::new(AtomicBool::new(false));
        let saw_output = Arc::new(AtomicBool::new(false));
        let blocks = Arc::new(Mutex::new(BlockStore::default()));
        let scrolled = Arc::new(AtomicU64::new(0));
        let cwd = Arc::new(Mutex::new(None));
        let graphics = Arc::new(Mutex::new(Graphics::default()));
        let ctx = FeedCtx {
            term: term.clone(),
            scrolled: scrolled.clone(),
            scrollback,
            blocks: blocks.clone(),
            cwd: cwd.clone(),
            graphics: graphics.clone(),
            window_size: window_size_shared.clone(),
            reply: Some(writer.clone()),
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
                &mut *conn.lock().unwrap(),
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
                &mut *conn.lock().unwrap(),
                &crate::ipc::Request::ClearHistory { session: *session },
            );
        }
        if was_prompt {
            blocks.on_prompt_start(abs_line);
            blocks.on_command_start(abs_line, cursor.column.0);
        }
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
        match &mut self.backend {
            Backend::Local(pty) => pty.on_resize(ws),
            Backend::Service { conn, session, .. } => {
                let _ = crate::ipc::send_json(
                    &mut *conn.lock().unwrap(),
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
        // Row-count changes move lines between history and screen; keep
        // `abs = scrolled + screen_row` coherent by folding the history
        // delta into the counter (same term lock the reader thread uses).
        let mut term = self.term.lock();
        let old = RowLayout::of(&term);
        term.resize(size);
        let new = RowLayout::of(&term);
        let cur = self.scrolled.load(std::sync::atomic::Ordering::Acquire);
        // the grid's top row keeps its abs index: base = scrolled - history
        let base = cur.saturating_sub(old.history as u64);
        self.scrolled.store(
            base + new.history as u64,
            std::sync::atomic::Ordering::Release,
        );
        // reflow (column change) re-wraps lines and moves every row after
        // them; carry the block anchors along (same lock order as the reader)
        if old.cols != new.cols || old.starts.len() != new.starts.len() {
            self.blocks
                .lock()
                .unwrap()
                .remap(|abs, col| old.map(&new, base, abs, col));
            self.graphics
                .lock()
                .unwrap()
                .remap(|abs, col| old.map(&new, base, abs, col));
        }
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
/// Everything the output parser writes to (shared with the app).
struct FeedCtx {
    term: Arc<FairMutex<Term<EventProxy>>>,
    scrolled: Arc<AtomicU64>,
    scrollback: usize,
    blocks: Arc<Mutex<BlockStore>>,
    cwd: Arc<Mutex<Option<String>>>,
    graphics: Arc<Mutex<Graphics>>,
    window_size: Arc<Mutex<WindowSize>>,
    /// PTY write-back for graphics protocol replies.
    reply: Option<PtyWriter>,
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
    for seg in segments {
        let was_alt = term.mode().contains(TermMode::ALT_SCREEN);
        let cleared = {
            let mut abs = AbsTerm {
                term: &mut term,
                scrolled: &mut scrolled_cur,
                scrollback_cap: ctx.scrollback,
                saw_clear_saved: false,
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
            let rows = term.grid().screen_lines();
            ctx.blocks.lock().unwrap().set_clear_pad(scrolled_cur, rows);
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
                let out = ctx.graphics.lock().unwrap().iterm(&body, cell, cols);
                if let Some(req) = out.place {
                    place_image(parser, &mut term, &mut scrolled_cur, ctx, &req);
                }
            }
            Some(Marker::Notify { title, body }) => {
                // replayed output already notified once
                let replaying = matches!(&ctx.reply, Some(w) if w.muted());
                if let (Some((id, observer)), false) = (&ctx.notify, replaying) {
                    observer(*id, SessionEvent::Notify { title, body });
                }
            }
            Some(marker) => apply_marker(marker, &term, scrolled_cur, &ctx.blocks, &ctx.cwd),
            None => {}
        }
    }
    // prune blocks and images that fell out of scrollback
    let oldest = scrolled_cur.saturating_sub(term.grid().history_size() as u64);
    ctx.blocks.lock().unwrap().prune(oldest);
    ctx.graphics.lock().unwrap().prune(oldest);
    ctx.scrolled.store(scrolled_cur, Ordering::Release);
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
    let mut seq = "\n".repeat(req.rows.saturating_sub(1));
    let col = (cursor.column.0 + req.cols).min(term.grid().columns() - 1);
    seq.push_str(&format!("\x1b[{}G", col + 1));
    let mut abs = AbsTerm {
        term,
        scrolled,
        scrollback_cap: ctx.scrollback,
        saw_clear_saved: false,
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
        Marker::Graphics(_) | Marker::InlineImage(_) | Marker::Notify { .. } => {}
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
        let mut text: String = (from..cols)
            .filter(|c| !row[Column(*c)].flags.contains(Flags::WIDE_CHAR_SPACER))
            .map(|c| row[Column(c)].c)
            .collect();
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
            writer: PtyWriter::File(Arc::new(File::create("/dev/null").unwrap())),
            window_size: Arc::new(Mutex::new(WindowSize {
                num_lines: rows as u16,
                num_cols: cols as u16,
                cell_width: 8,
                cell_height: 16,
            })),
        };
        Term::new(TermConfig::default(), &GridSize { cols, rows }, proxy)
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
            scrollback: 100,
            blocks: Arc::new(Mutex::new(BlockStore::default())),
            cwd: Arc::new(Mutex::new(None)),
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
