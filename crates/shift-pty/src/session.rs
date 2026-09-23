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

/// Bridges Term-emitted events to the observer + PTY write-back.
pub struct EventProxy {
    pane: PaneId,
    observer: Observer,
    writer: File,
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
            Event::PtyWrite(s) => {
                let _ = (&self.writer).write_all(s.as_bytes());
            }
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
                let _ = (&self.writer).write_all(format(size).as_bytes());
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
    writer: File,
    pty: tty::Pty,
    size: GridSize,
    window_size: Arc<Mutex<WindowSize>>,
    scrollback: usize,
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
        } = opts;

        let window_size = WindowSize {
            num_lines: size.rows as u16,
            num_cols: size.cols as u16,
            cell_width: cell_size.0,
            cell_height: cell_size.1,
        };

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
        let program = shift_core::shell_integration::resolve_shell(shell.as_deref());
        let args = shift_core::shell_integration::shell_args(&program);
        let tty_opts = tty::Options {
            shell: Some(tty::Shell::new(program, args)),
            working_directory,
            drain_on_exit: false,
            env: tty_env,
        };

        let pty = tty::new(&tty_opts, window_size, id)?;
        let window_size_shared = Arc::new(Mutex::new(window_size));

        // alacritty configures the master fd non-blocking (it polls with mio);
        // we use a dedicated blocking reader thread instead
        unsafe {
            use std::os::fd::AsRawFd;
            let fd = pty.file().as_raw_fd();
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
        }

        let reader_file = pty.file().try_clone()?;
        let writer = pty.file().try_clone()?;
        let proxy_writer = pty.file().try_clone()?;

        let term_config = TermConfig {
            scrolling_history: scrollback,
            ..TermConfig::default()
        };
        let term = Term::new(
            term_config,
            &size,
            EventProxy {
                pane: id,
                observer: observer.clone(),
                writer: proxy_writer,
                window_size: window_size_shared.clone(),
            },
        );
        let term = Arc::new(FairMutex::new(term));
        let dirty = Arc::new(AtomicBool::new(false));
        let saw_output = Arc::new(AtomicBool::new(false));
        let blocks = Arc::new(Mutex::new(BlockStore::default()));
        let scrolled = Arc::new(AtomicU64::new(0));
        let cwd = Arc::new(Mutex::new(None));

        {
            let term = term.clone();
            let dirty = dirty.clone();
            let saw_output = saw_output.clone();
            let observer = observer.clone();
            let blocks = blocks.clone();
            let scrolled_shared = scrolled.clone();
            let cwd = cwd.clone();
            let mut file = reader_file;
            std::thread::Builder::new()
                .name(format!("pty-reader-{id}"))
                .spawn(move || {
                    let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
                    let mut scanner = OscScanner::new();
                    let mut buf = [0u8; 65536];
                    loop {
                        match file.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                saw_output.store(true, Ordering::Release);
                                let segments = scanner.scan(&buf[..n]);
                                let mut term = term.lock();
                                // the atomic is canonical: resizes adjust it
                                // (under the same term lock) between chunks
                                let mut scrolled_cur = scrolled_shared.load(Ordering::Acquire);
                                let mut start = 0usize;
                                for seg in segments {
                                    let cleared = {
                                        let mut abs = AbsTerm {
                                            term: &mut term,
                                            scrolled: &mut scrolled_cur,
                                            scrollback_cap: scrollback,
                                            saw_clear_saved: false,
                                        };
                                        parser.advance(&mut abs, &buf[start..seg.end]);
                                        abs.saw_clear_saved
                                    };
                                    if cleared {
                                        // `clear` ran (ED3, suppressed in
                                        // AbsTerm): Warp-style soft clear —
                                        // blocks and scrollback stay; pad the
                                        // viewport at the first post-clear
                                        // screen row so older content moves
                                        // above the fold but stays scrollable.
                                        let rows = term.grid().screen_lines();
                                        blocks.lock().unwrap().set_clear_pad(scrolled_cur, rows);
                                        term.selection = None;
                                    }
                                    start = seg.end;
                                    if let Some(marker) = seg.marker {
                                        apply_marker(marker, &term, scrolled_cur, &blocks, &cwd);
                                    }
                                }
                                // prune blocks that fell out of scrollback
                                let oldest =
                                    scrolled_cur.saturating_sub(term.grid().history_size() as u64);
                                blocks.lock().unwrap().prune(oldest);
                                scrolled_shared.store(scrolled_cur, Ordering::Release);
                                drop(term);
                                if !dirty.swap(true, Ordering::AcqRel) {
                                    observer(id, SessionEvent::Wakeup);
                                }
                            }
                            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                            Err(_) => break,
                        }
                    }
                    observer(id, SessionEvent::Exit);
                })
                .expect("spawn pty reader");
        }

        Ok(Session {
            id,
            term,
            blocks,
            scrolled,
            cwd,
            dirty,
            saw_output,
            writer,
            pty,
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
        let _ = (&self.writer).write_all(bytes);
    }

    /// Process group owning the terminal's foreground, when it is not the
    /// shell itself (a running job: `vim`, `cargo build`, `ssh`, …).
    fn foreground_job(&self) -> Option<i32> {
        use std::os::fd::AsRawFd;
        let pgrp = unsafe { libc::tcgetpgrp(self.pty.file().as_raw_fd()) };
        let shell = self.pty.child().id() as i32;
        (pgrp > 0 && pgrp != shell).then_some(pgrp)
    }

    /// True while a job other than the shell runs in the foreground — the
    /// quit/close warning checks this, so it works without shell integration.
    pub fn has_foreground_job(&self) -> bool {
        self.foreground_job().is_some()
    }

    /// Executable name of the foreground job, if any.
    pub fn foreground_job_name(&self) -> Option<String> {
        let pid = self.foreground_job()?;
        #[cfg(target_os = "macos")]
        {
            let mut buf = [0u8; 256];
            let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
            (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
        }
        #[cfg(not(target_os = "macos"))]
        {
            std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .ok()
                .map(|s| s.trim().to_string())
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
        self.pty.on_resize(ws);
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
            writer: File::create("/dev/null").unwrap(),
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
}
