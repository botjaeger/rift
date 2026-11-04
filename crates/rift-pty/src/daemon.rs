//! The session service (`rift-terminal --session-daemon`): owns durable
//! shells so they keep running when every Rift window is gone.
//!
//! One service per user, listening on [`ipc::socket_path`] in a 0700
//! directory; connections from other uids are refused. Each shell's output
//! is retained in a ring of at most [`MAX_RETAINED`] bytes, so a window that
//! attaches (or reattaches after a restart) replays it into a fresh
//! terminal, then follows live output. Replay and live output are
//! sequenced under one lock: no gap, no duplicate.
//!
//! Backpressure: each client has a bounded queue; a client that falls
//! behind by [`CLIENT_QUEUE`] frames is disconnected (its window reattaches
//! and replays) instead of letting memory grow. The service exits once it
//! has no shells and no clients for [`IDLE_EXIT`].

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty;

use crate::ipc::{self, Event, Frame, Request, SessionInfo};

pub const MAX_RETAINED: usize = 8 << 20;
pub const CLIENT_QUEUE: usize = 4096;
pub const IDLE_EXIT: Duration = Duration::from_secs(10);
/// Replay is sent in frames of at most this many bytes.
const REPLAY_CHUNK: usize = 1 << 20;
const REPLAY_MERGE_MS: u64 = 50;

/// Something queued for a client's writer thread.
enum Out {
    Event(Event),
    Output {
        session: u64,
        seq: u64,
        at_ms: u64,
        bytes: Arc<Vec<u8>>,
    },
}

/// Retained output: chunks (first byte's seq, wall ms, bytes).
#[derive(Default)]
struct Ring {
    chunks: VecDeque<(u64, u64, Arc<Vec<u8>>)>,
    bytes: usize,
    /// Sequence number of the next byte.
    end: u64,
}

impl Ring {
    fn push(&mut self, at_ms: u64, bytes: Vec<u8>) -> (u64, Arc<Vec<u8>>) {
        let seq = self.end;
        self.end += bytes.len() as u64;
        self.bytes += bytes.len();
        let bytes = Arc::new(bytes);
        self.chunks.push_back((seq, at_ms, bytes.clone()));
        while self.bytes > MAX_RETAINED && self.chunks.len() > 1 {
            if let Some((_, _, b)) = self.chunks.pop_front() {
                self.bytes -= b.len();
            }
        }
        (seq, bytes)
    }

    fn start(&self) -> u64 {
        self.chunks.front().map(|c| c.0).unwrap_or(self.end)
    }

    fn clear(&mut self) {
        self.chunks.clear();
        self.bytes = 0;
    }

    /// Retained chunks after `from` (a chunk straddling it is cut), merged
    /// into frames of at most [`REPLAY_CHUNK`] bytes when they arrived
    /// within [`REPLAY_MERGE_MS`] of each other (so replayed block
    /// timestamps stay accurate).
    fn replay(&self, from: u64) -> Vec<(u64, u64, Vec<u8>)> {
        let mut out: Vec<(u64, u64, Vec<u8>)> = Vec::new();
        for (seq, at, b) in &self.chunks {
            let end = seq + b.len() as u64;
            if end <= from {
                continue;
            }
            let skip = from.saturating_sub(*seq) as usize;
            let mut pos = seq + skip as u64;
            for part in b[skip..].chunks(REPLAY_CHUNK) {
                match out.last_mut() {
                    Some(last)
                        if last.2.len() + part.len() <= REPLAY_CHUNK
                            && at.saturating_sub(last.1) <= REPLAY_MERGE_MS =>
                    {
                        last.2.extend_from_slice(part);
                    }
                    _ => out.push((pos, *at, part.to_vec())),
                }
                pos += part.len() as u64;
            }
        }
        out
    }
}

struct Client {
    pid: u32,
    tx: SyncSender<Out>,
    alive: Arc<AtomicBool>,
    socket: Arc<UnixStream>,
}

impl Client {
    fn send(&self, out: Out) -> bool {
        if !self.alive.load(Ordering::Acquire) {
            return false;
        }
        match self.tx.try_send(out) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.alive.store(false, Ordering::Release);
                let _ = self.socket.shutdown(std::net::Shutdown::Both);
                false
            }
        }
    }
}

struct Shell {
    id: u64,
    pid: u32,
    started: u64,
    pty: Mutex<tty::Pty>,
    writer: Mutex<std::fs::File>,
    ring: Mutex<Ring>,
    /// `ring`'s size, readable while an attach holds the ring to replay
    /// it (as long as that window takes to read it).
    retained: AtomicUsize,
    clients: Mutex<Vec<Client>>,
    cwd: Mutex<Option<String>>,
    job: Mutex<Option<String>>,
    /// The terminal reads a line without echo (polled with `job`).
    hidden: AtomicBool,
}

impl Shell {
    fn broadcast(&self, make: impl Fn() -> Out) {
        self.clients.lock().unwrap().retain(|c| c.send(make()));
    }

    fn info(&self) -> SessionInfo {
        // One lock per statement: temporaries in a struct literal live to its
        // end, so `clients` would be held while taking `ring` — the reader
        // thread takes them the other way round (ring, then clients).
        let cwd = self.cwd.lock().unwrap().clone();
        let job = self.job.lock().unwrap().clone().filter(|j| !j.is_empty());
        let clients = self.clients.lock().unwrap().iter().map(|c| c.pid).collect();
        let retained_bytes = self.retained.load(Ordering::Relaxed) as u64;
        SessionInfo {
            id: self.id,
            pid: self.pid,
            cwd,
            job,
            started: self.started,
            clients,
            retained_bytes,
        }
    }
}

#[derive(Default)]
struct State {
    shells: Mutex<HashMap<u64, Arc<Shell>>>,
    next_id: Mutex<u64>,
    clients: AtomicUsize,
}

/// How long a starter waits for a running (or exiting) service to resolve.
const LOCK_WAIT: Duration = Duration::from_secs(3);

/// Take the service lock (`flock` on `lock_path`, released when the returned
/// file or the process goes away). `None` when a live service holds it and
/// answers on `socket`. While the holder has not bound yet (it just won the
/// race) or is exiting (it unlinked the socket but has not quit), wait up to
/// `wait` for either outcome.
fn acquire_lock(
    lock_path: &std::path::Path,
    socket: &std::path::Path,
    wait: Duration,
) -> std::io::Result<Option<std::fs::File>> {
    use std::os::fd::AsRawFd;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    let deadline = Instant::now() + wait;
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(Some(file));
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::WouldBlock {
            return Err(err);
        }
        if UnixStream::connect(socket).is_ok() {
            return Ok(None);
        }
        if Instant::now() > deadline {
            return Err(std::io::Error::other("session service lock is busy"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Identity of the file at `path` (device, inode).
fn file_id(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

/// Remove `path` only when it is still the file we bound (`id`): never
/// another instance's socket.
fn unlink_if_ours(path: &std::path::Path, id: Option<(u64, u64)>) -> bool {
    if id.is_some() && file_id(path) == id {
        let _ = std::fs::remove_file(path);
        return true;
    }
    false
}

/// Run the service until it is idle. Returns early (Ok) when another
/// instance already serves the socket.
pub fn run() -> std::io::Result<()> {
    let path = ipc::socket_path().ok_or_else(|| std::io::Error::other("no data dir"))?;
    let dir = path.parent().expect("socket dir");
    std::fs::create_dir_all(dir)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    // One service at a time: the lock is held for this process's lifetime,
    // so a second starter can neither unlink the socket we serve nor bind
    // over it.
    let Some(_lock) = acquire_lock(&dir.join("lock"), &path, LOCK_WAIT)? else {
        return Ok(()); // already running
    };
    if UnixStream::connect(&path).is_ok() {
        return Ok(()); // an instance from before the lock existed
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    let bound = file_id(&path);
    let state = Arc::new(State::default());
    *state.next_id.lock().unwrap() = ipc::now_ms(); // ids stay unique across restarts

    // idle exit + foreground-job polling
    {
        let state = state.clone();
        let path = path.clone();
        std::thread::spawn(move || {
            let mut idle_since: Option<Instant> = None;
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let shells: Vec<Arc<Shell>> =
                    state.shells.lock().unwrap().values().cloned().collect();
                for sh in &shells {
                    let (job, hidden) = {
                        let pty = sh.pty.lock().unwrap();
                        (
                            crate::session::foreground_job_name(&pty),
                            crate::session::input_hidden(&pty),
                        )
                    };
                    let changed = {
                        let mut cur = sh.job.lock().unwrap();
                        let changed = *cur != job;
                        *cur = job.clone();
                        changed
                    } | (sh.hidden.swap(hidden, Ordering::AcqRel) != hidden);
                    if changed {
                        let id = sh.id;
                        sh.broadcast(|| {
                            Out::Event(Event::Status {
                                session: id,
                                job: job.clone(),
                                hidden,
                            })
                        });
                    }
                }
                let idle = shells.is_empty() && state.clients.load(Ordering::Acquire) == 0;
                match (idle, idle_since) {
                    (true, None) => idle_since = Some(Instant::now()),
                    (true, Some(t)) if t.elapsed() > IDLE_EXIT => {
                        unlink_if_ours(&path, bound);
                        std::process::exit(0);
                    }
                    (false, _) => idle_since = None,
                    _ => {}
                }
            }
        });
    }

    let me = unsafe { libc::getuid() };
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if ipc::peer_uid(&stream) != Some(me) {
            continue; // not ours: drop the connection
        }
        let state = state.clone();
        std::thread::spawn(move || serve(state, stream));
    }
    Ok(())
}

fn serve(state: Arc<State>, stream: UnixStream) {
    let stream = Arc::new(stream);
    state.clients.fetch_add(1, Ordering::AcqRel);
    let (tx, rx) = sync_channel::<Out>(CLIENT_QUEUE);
    let alive = Arc::new(AtomicBool::new(true));
    let Ok(write_half) = stream.try_clone() else {
        state.clients.fetch_sub(1, Ordering::AcqRel);
        return;
    };
    let writer = {
        let alive = alive.clone();
        std::thread::spawn(move || write_loop(write_half, rx, alive))
    };
    let mut peer_pid = 0u32;
    let mut read_half = &*stream;
    while alive.load(Ordering::Acquire) {
        let frame = match ipc::read_frame(&mut read_half) {
            Ok(Some(f)) => f,
            _ => break,
        };
        match frame {
            Frame::Input { session, bytes } => {
                let shell = state.shells.lock().unwrap().get(&session).cloned();
                if let Some(sh) = shell {
                    use std::io::Write;
                    let _ = sh.writer.lock().unwrap().write_all(&bytes);
                }
            }
            Frame::Json(body) => {
                let Ok(req) = serde_json::from_slice::<Request>(&body) else {
                    let _ = tx.send(Out::Event(Event::Error {
                        message: "bad request".into(),
                    }));
                    continue;
                };
                handle(&state, req, &tx, &alive, &stream, &mut peer_pid);
            }
            Frame::Output { .. } => {}
        }
    }
    alive.store(false, Ordering::Release);
    drop(tx);
    // a shell that prints nothing would list this window as attached
    // forever (only a failed send drops a client), and the session
    // switcher offers only shells without one
    let shells: Vec<Arc<Shell>> = state.shells.lock().unwrap().values().cloned().collect();
    for sh in shells {
        sh.clients
            .lock()
            .unwrap()
            .retain(|c| !Arc::ptr_eq(&c.alive, &alive));
    }
    let _ = read_half.shutdown(std::net::Shutdown::Both);
    let _ = writer.join();
    state.clients.fetch_sub(1, Ordering::AcqRel);
}

fn write_loop(mut s: UnixStream, rx: Receiver<Out>, alive: Arc<AtomicBool>) {
    let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
    while alive.load(Ordering::Acquire) {
        let out = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(out) => out,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        let r = match out {
            Out::Event(e) => ipc::send_json(&mut s, &e),
            Out::Output {
                session,
                seq,
                at_ms,
                bytes,
            } => ipc::send_output(&mut s, session, seq, at_ms, &bytes),
        };
        if r.is_err() {
            break;
        }
    }
    alive.store(false, Ordering::Release);
    let _ = s.shutdown(std::net::Shutdown::Both);
}

fn handle(
    state: &Arc<State>,
    req: Request,
    tx: &SyncSender<Out>,
    alive: &Arc<AtomicBool>,
    socket: &Arc<UnixStream>,
    peer_pid: &mut u32,
) {
    let reply = |e: Event| {
        let _ = tx.send(Out::Event(e));
    };
    let shell_of = |id: u64| state.shells.lock().unwrap().get(&id).cloned();
    match req {
        Request::Hello { pid, .. } => {
            *peer_pid = pid;
            reply(Event::Hello {
                version: ipc::PROTOCOL_VERSION,
                pid: std::process::id(),
                features: vec![ipc::FEATURE_ARGS.into(), ipc::FEATURE_PRIVACY.into()],
            });
        }
        Request::Privacy => reply(Event::Privacy {
            full_disk: crate::privacy::full_disk_access(),
        }),
        Request::Spawn {
            shell,
            args,
            cwd,
            env,
            cols,
            rows,
            cell_w,
            cell_h,
        } => match spawn(state, shell, args, cwd, env, cols, rows, cell_w, cell_h) {
            Ok(id) => reply(Event::Spawned { session: id }),
            Err(e) => reply(Event::Error {
                message: format!("spawn failed: {e}"),
            }),
        },
        Request::Attach { session, from_seq } => {
            let Some(sh) = shell_of(session) else {
                reply(Event::Error {
                    message: format!("no session {session}"),
                });
                return;
            };
            // replay and subscribe under the ring lock: the reader pushes
            // and broadcasts under the same lock, so nothing is missed or
            // sent twice
            let ring = sh.ring.lock().unwrap();
            let start = ring.start();
            let from = from_seq.max(start);
            reply(Event::Attached {
                session,
                from_seq: from,
                truncated: from_seq < start,
            });
            for (seq, at_ms, bytes) in ring.replay(from) {
                let _ = tx.send(Out::Output {
                    session,
                    seq,
                    at_ms,
                    bytes: Arc::new(bytes),
                });
            }
            reply(Event::ReplayDone { session });
            let job = sh.job.lock().unwrap().clone();
            let hidden = sh.hidden.load(Ordering::Acquire);
            reply(Event::Status {
                session,
                job,
                hidden,
            });
            sh.clients.lock().unwrap().push(Client {
                pid: *peer_pid,
                tx: tx.clone(),
                alive: alive.clone(),
                socket: socket.clone(),
            });
        }
        Request::Resize {
            session,
            cols,
            rows,
            cell_w,
            cell_h,
        } => {
            if let Some(sh) = shell_of(session) {
                sh.pty.lock().unwrap().on_resize(WindowSize {
                    num_lines: rows.max(2),
                    num_cols: cols.max(2),
                    cell_width: cell_w,
                    cell_height: cell_h,
                });
            }
        }
        Request::ClearHistory { session } => {
            if let Some(sh) = shell_of(session) {
                sh.ring.lock().unwrap().clear();
            }
        }
        Request::Kill { session } => {
            // hang up the shell (what closing a local PTY does); its reader
            // then sees EOF, reports the exit and removes the session
            if let Some(sh) = shell_of(session) {
                unsafe { libc::kill(sh.pid as i32, libc::SIGHUP) };
            }
        }
        Request::List => {
            // not under the shell table: `info` waits on each shell's own
            // locks, and spawn and attach need the table meanwhile
            let shells: Vec<Arc<Shell>> = state.shells.lock().unwrap().values().cloned().collect();
            let mut sessions: Vec<SessionInfo> = shells.iter().map(|s| s.info()).collect();
            sessions.sort_by_key(|s| s.id);
            reply(Event::Sessions { sessions });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    state: &Arc<State>,
    shell: Option<String>,
    args: Vec<String>,
    cwd: Option<String>,
    env: Vec<(String, String)>,
    cols: u16,
    rows: u16,
    cell_w: u16,
    cell_h: u16,
) -> std::io::Result<u64> {
    let id = {
        let mut n = state.next_id.lock().unwrap();
        *n += 1;
        *n
    };
    let ws = WindowSize {
        num_lines: rows.max(2),
        num_cols: cols.max(2),
        cell_width: cell_w,
        cell_height: cell_h,
    };
    let dir = cwd.map(std::path::PathBuf::from).filter(|p| p.is_dir());
    let env = service_env(env, id);
    let pty = crate::session::open_pty(shell.as_deref(), &args, dir, env, ws, id)?;
    let mut reader = pty.file().try_clone()?;
    let writer = pty.file().try_clone()?;
    let shell = Arc::new(Shell {
        id,
        pid: pty.child().id(),
        started: ipc::now_ms() / 1000,
        pty: Mutex::new(pty),
        writer: Mutex::new(writer),
        ring: Mutex::new(Ring::default()),
        retained: AtomicUsize::new(0),
        clients: Mutex::new(Vec::new()),
        cwd: Mutex::new(None),
        job: Mutex::new(None),
        hidden: AtomicBool::new(false),
    });
    state.shells.lock().unwrap().insert(id, shell.clone());
    let state = state.clone();
    std::thread::Builder::new()
        .name(format!("session-{id}"))
        .spawn(move || {
            let mut scanner = rift_core::OscScanner::new();
            let mut buf = vec![0u8; 65536];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let bytes = buf[..n].to_vec();
                        for seg in scanner.scan(&bytes) {
                            if let Some(rift_core::Marker::Cwd { path: p, .. }) = seg.marker {
                                *shell.cwd.lock().unwrap() = Some(p);
                            }
                        }
                        let at = ipc::now_ms();
                        let mut ring = shell.ring.lock().unwrap();
                        let (seq, bytes) = ring.push(at, bytes);
                        shell.retained.store(ring.bytes, Ordering::Relaxed);
                        shell.broadcast(|| Out::Output {
                            session: id,
                            seq,
                            at_ms: at,
                            bytes: bytes.clone(),
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let code = crate::exit_code_of(shell.pid);
            shell.broadcast(|| Out::Event(Event::Exited { session: id, code }));
            state.shells.lock().unwrap().remove(&id);
        })?;
    Ok(id)
}

/// `env` naming the pane by service session `id` instead of the window's
/// per-run pane id: the shell outlives the window process, and a later run
/// reattaching it still finds the pane by this name.
fn service_env(mut env: Vec<(String, String)>, id: u64) -> Vec<(String, String)> {
    use rift_core::uri::{PANE_ID_ENV, PANE_URL_ENV, PaneRef};
    env.retain(|(k, _)| k != PANE_ID_ENV && k != PANE_URL_ENV);
    env.extend(PaneRef::Service(id).env());
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_shell_is_named_by_its_session() {
        let window = vec![
            ("A".to_string(), "1".to_string()),
            ("RIFT_PANE_ID".to_string(), "abc.3".to_string()),
            (
                "RIFT_PANE_URL".to_string(),
                "rift://action/focus?pane=abc.3".to_string(),
            ),
        ];
        let env = service_env(window, 9);
        assert_eq!(
            env,
            vec![
                ("A".to_string(), "1".to_string()),
                ("RIFT_PANE_ID".to_string(), "service.9".to_string()),
                (
                    "RIFT_PANE_URL".to_string(),
                    "rift://action/focus?pane=service.9".to_string()
                ),
            ]
        );
    }

    /// A real (idle) shell on a PTY, without shell integration.
    fn test_shell() -> Shell {
        let opts = tty::Options {
            // dies on the SIGHUP that dropping the Pty sends (an
            // interactive sh ignores it and the drop would wait forever)
            shell: Some(tty::Shell::new("/bin/sleep".into(), vec!["600".into()])),
            working_directory: None,
            drain_on_exit: false,
            env: HashMap::new(),
        };
        let ws = WindowSize {
            num_lines: 24,
            num_cols: 80,
            cell_width: 8,
            cell_height: 16,
        };
        let pty = tty::new(&opts, ws, 0).unwrap();
        let writer = pty.file().try_clone().unwrap();
        Shell {
            id: 1,
            pid: pty.child().id(),
            started: 0,
            pty: Mutex::new(pty),
            writer: Mutex::new(writer),
            ring: Mutex::new(Ring::default()),
            retained: AtomicUsize::new(0),
            clients: Mutex::new(Vec::new()),
            cwd: Mutex::new(None),
            job: Mutex::new(None),
            hidden: AtomicBool::new(false),
        }
    }

    #[test]
    fn info_does_not_deadlock_against_the_reader_lock_order() {
        let sh = Arc::new(test_shell());
        let (done, finished) = std::sync::mpsc::channel();
        // the reader thread's order: ring, then clients (push + broadcast)
        let reader = {
            let (sh, done) = (sh.clone(), done.clone());
            std::thread::spawn(move || {
                for _ in 0..50_000 {
                    let _ring = sh.ring.lock().unwrap();
                    sh.broadcast(|| Out::Event(Event::ReplayDone { session: 1 }));
                }
                done.send(()).unwrap();
            })
        };
        let lister = {
            let sh = sh.clone();
            std::thread::spawn(move || {
                for _ in 0..50_000 {
                    sh.info();
                }
                done.send(()).unwrap();
            })
        };
        for _ in 0..2 {
            finished
                .recv_timeout(Duration::from_secs(20))
                .expect("Shell::info deadlocked against the reader's ring -> clients order");
        }
        reader.join().unwrap();
        lister.join().unwrap();
    }

    #[test]
    fn a_list_answers_while_a_window_has_not_read_its_replay() {
        // a replay longer than the client queue: the attach holds the ring
        // until the window reads it (restored windows asked for the list
        // first, and every other pane's spawn and attach timed out behind it)
        let sh = Arc::new(test_shell());
        {
            let mut ring = sh.ring.lock().unwrap();
            for i in 0..(CLIENT_QUEUE as u64 * 3) {
                ring.push(i * (REPLAY_MERGE_MS + 1), vec![b'x'; 64]);
            }
            sh.retained.store(ring.bytes, Ordering::Relaxed);
        }
        let state = Arc::new(State::default());
        state.shells.lock().unwrap().insert(1, sh.clone());
        let connect = || {
            let (ours, theirs) = UnixStream::pair().unwrap();
            let state = state.clone();
            std::thread::spawn(move || serve(state, theirs));
            ours
        };
        let mut stuck = connect();
        ipc::send_json(
            &mut stuck,
            &Request::Attach {
                session: 1,
                from_seq: 0,
            },
        )
        .unwrap();
        // the attach is replaying (and blocked) once it holds the ring
        let t = Instant::now();
        while sh.ring.try_lock().is_ok() {
            assert!(
                t.elapsed() < Duration::from_secs(5),
                "the attach never started"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(200));
        assert!(sh.ring.try_lock().is_err(), "the replay did not block");
        let mut other = connect();
        ipc::send_json(&mut other, &Request::List).unwrap();
        let t = Instant::now();
        let answer = crate::service::next_event(&mut other);
        // the replay's write timeout frees the ring after 5 s, exactly when
        // a window's setup request gives up
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "list stalled behind the replay: {:?}",
            t.elapsed()
        );
        assert!(
            sh.ring.try_lock().is_err(),
            "the replay ended before the list"
        );
        match answer.unwrap() {
            Event::Sessions { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(
                    sessions[0].retained_bytes,
                    sh.retained.load(Ordering::Relaxed) as u64
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        drop(stuck);
    }

    #[test]
    fn the_service_lists_the_privacy_feature_and_answers_for_itself() {
        let state = Arc::new(State::default());
        let (mut ours, theirs) = UnixStream::pair().unwrap();
        std::thread::spawn(move || serve(state, theirs));
        ipc::send_json(
            &mut ours,
            &Request::Hello {
                version: ipc::PROTOCOL_VERSION,
                pid: 1,
            },
        )
        .unwrap();
        match crate::service::next_event(&mut ours).unwrap() {
            Event::Hello { features, .. } => {
                assert!(features.iter().any(|f| f == ipc::FEATURE_PRIVACY));
                assert!(features.iter().any(|f| f == ipc::FEATURE_ARGS));
            }
            other => panic!("unexpected {other:?}"),
        }
        ipc::send_json(&mut ours, &Request::Privacy).unwrap();
        match crate::service::next_event(&mut ours).unwrap() {
            Event::Privacy { full_disk } => {
                assert_eq!(full_disk, crate::privacy::full_disk_access())
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_window_that_disconnects_leaves_an_idle_shell() {
        let sh = Arc::new(test_shell());
        let state = Arc::new(State::default());
        state.shells.lock().unwrap().insert(1, sh.clone());
        let (mut ours, theirs) = UnixStream::pair().unwrap();
        let server = {
            let state = state.clone();
            std::thread::spawn(move || serve(state, theirs))
        };
        ipc::send_json(
            &mut ours,
            &Request::Attach {
                session: 1,
                from_seq: 0,
            },
        )
        .unwrap();
        let t = Instant::now();
        while sh.clients.lock().unwrap().is_empty() {
            assert!(t.elapsed() < Duration::from_secs(5), "never attached");
            std::thread::sleep(Duration::from_millis(5));
        }
        // the shell prints nothing: no failed send ever drops the window
        drop(ours);
        server.join().unwrap();
        assert!(sh.clients.lock().unwrap().is_empty());
        assert!(sh.info().clients.is_empty());
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rift-daemon-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_one_starter_holds_the_service_lock() {
        let dir = scratch("lock");
        let (lock, sock) = (dir.join("lock"), dir.join("sock"));
        let first = acquire_lock(&lock, &sock, Duration::from_millis(100)).unwrap();
        assert!(first.is_some());
        // a second starter must not get past the lock (nothing answers on
        // the socket, so it times out instead of unlinking/binding)
        assert!(acquire_lock(&lock, &sock, Duration::from_millis(100)).is_err());
        // with a live service on the socket it backs off quietly
        let listener = UnixListener::bind(&sock).unwrap();
        assert!(
            acquire_lock(&lock, &sock, Duration::from_millis(100))
                .unwrap()
                .is_none()
        );
        // and once the holder (and its service) is gone the lock is free
        // again. A child another test forks shares the lock's open file
        // until its exec closes it, so the lock may stay busy a moment: wait
        drop(first);
        drop(listener);
        let _ = std::fs::remove_file(&sock);
        assert!(
            acquire_lock(&lock, &sock, Duration::from_secs(10))
                .unwrap()
                .is_some()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn idle_exit_only_unlinks_the_socket_it_bound() {
        let dir = scratch("unlink");
        let sock = dir.join("sock");
        std::fs::write(&sock, b"a").unwrap();
        let mine = file_id(&sock);
        // keep the first inode alive so the replacement cannot reuse it
        let _held = std::fs::File::open(&sock).unwrap();
        // another instance replaced the socket
        std::fs::write(dir.join("other"), b"b").unwrap();
        std::fs::rename(dir.join("other"), &sock).unwrap();
        assert!(!unlink_if_ours(&sock, mine));
        assert!(sock.exists(), "the other instance's socket survives");
        assert!(unlink_if_ours(&sock, file_id(&sock)));
        assert!(!sock.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn full_client_queue_disconnects_the_socket() {
        let (socket, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let (tx, _rx) = sync_channel(1);
        let alive = Arc::new(AtomicBool::new(true));
        let c = Client {
            pid: 1,
            tx,
            alive: alive.clone(),
            socket: Arc::new(socket),
        };
        assert!(c.send(Out::Event(Event::ReplayDone { session: 1 })));
        assert!(!c.send(Out::Event(Event::ReplayDone { session: 1 })));
        assert!(!alive.load(Ordering::Acquire));
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn idle_writer_exits_even_while_a_sender_survives() {
        let (socket, _peer) = UnixStream::pair().unwrap();
        let (_tx, rx) = sync_channel(1);
        let alive = Arc::new(AtomicBool::new(true));
        let running = alive.clone();
        let (done, finished) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            write_loop(socket, rx, running);
            done.send(()).unwrap();
        });
        alive.store(false, Ordering::Release);
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn ring_bounds_and_replays_from_any_point() {
        let mut r = Ring::default();
        r.push(1, b"hello ".to_vec());
        r.push(2, b"world".to_vec());
        assert_eq!(r.start(), 0);
        assert_eq!(r.end, 11);
        // from the middle of the first chunk: merged (1 ms apart), cut at
        // the offset
        assert_eq!(r.replay(3), vec![(3, 1, b"lo world".to_vec())]);
        // chunks far apart in time stay separate frames
        r.push(1_000, b"!".to_vec());
        assert_eq!(
            r.replay(10),
            vec![(10, 2, b"d".to_vec()), (11, 1_000, b"!".to_vec())]
        );
        r.clear();
        let mut r = Ring::default();
        r.push(1, b"hello ".to_vec());
        r.push(2, b"world".to_vec());
        assert!(r.replay(11).is_empty());
        // past the budget the oldest chunks go
        let big = vec![b'x'; MAX_RETAINED];
        r.push(3, big);
        assert_eq!(r.start(), 11);
        assert!(r.bytes <= MAX_RETAINED);
        let replay = r.replay(0);
        assert_eq!(replay[0].0, 11, "older bytes are gone");
        assert!(replay.iter().all(|c| c.2.len() <= REPLAY_CHUNK));
        assert_eq!(
            replay.iter().map(|c| c.2.len()).sum::<usize>(),
            MAX_RETAINED
        );
        r.clear();
        assert!(r.replay(0).is_empty());
    }
}
