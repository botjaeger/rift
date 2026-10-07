//! The session service (`shift-terminal --session-daemon`): owns durable
//! shells so they keep running when every Shift window is gone.
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
    clients: Mutex<Vec<Client>>,
    cwd: Mutex<Option<String>>,
    job: Mutex<Option<String>>,
}

impl Shell {
    fn broadcast(&self, make: impl Fn() -> Out) {
        self.clients.lock().unwrap().retain(|c| c.send(make()));
    }

    fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id,
            pid: self.pid,
            cwd: self.cwd.lock().unwrap().clone(),
            job: self.job.lock().unwrap().clone().filter(|j| !j.is_empty()),
            started: self.started,
            clients: self.clients.lock().unwrap().iter().map(|c| c.pid).collect(),
            retained_bytes: self.ring.lock().unwrap().bytes as u64,
        }
    }
}

#[derive(Default)]
struct State {
    shells: Mutex<HashMap<u64, Arc<Shell>>>,
    next_id: Mutex<u64>,
    clients: AtomicUsize,
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
    if UnixStream::connect(&path).is_ok() {
        return Ok(()); // already running
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
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
                    let job = crate::session::foreground_job_name(&sh.pty.lock().unwrap());
                    let changed = {
                        let mut cur = sh.job.lock().unwrap();
                        let changed = *cur != job;
                        *cur = job.clone();
                        changed
                    };
                    if changed {
                        let id = sh.id;
                        sh.broadcast(|| {
                            Out::Event(Event::Status {
                                session: id,
                                job: job.clone(),
                            })
                        });
                    }
                }
                let idle = shells.is_empty() && state.clients.load(Ordering::Acquire) == 0;
                match (idle, idle_since) {
                    (true, None) => idle_since = Some(Instant::now()),
                    (true, Some(t)) if t.elapsed() > IDLE_EXIT => {
                        let _ = std::fs::remove_file(&path);
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
            });
        }
        Request::Spawn {
            shell,
            cwd,
            env,
            cols,
            rows,
            cell_w,
            cell_h,
        } => match spawn(state, shell, cwd, env, cols, rows, cell_w, cell_h) {
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
            reply(Event::Status { session, job });
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
            let mut sessions: Vec<SessionInfo> = state
                .shells
                .lock()
                .unwrap()
                .values()
                .map(|s| s.info())
                .collect();
            sessions.sort_by_key(|s| s.id);
            reply(Event::Sessions { sessions });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    state: &Arc<State>,
    shell: Option<String>,
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
    let pty = crate::session::open_pty(shell.as_deref(), dir, env, ws, id)?;
    let mut reader = pty.file().try_clone()?;
    let writer = pty.file().try_clone()?;
    let shell = Arc::new(Shell {
        id,
        pid: pty.child().id(),
        started: ipc::now_ms() / 1000,
        pty: Mutex::new(pty),
        writer: Mutex::new(writer),
        ring: Mutex::new(Ring::default()),
        clients: Mutex::new(Vec::new()),
        cwd: Mutex::new(None),
        job: Mutex::new(None),
    });
    state.shells.lock().unwrap().insert(id, shell.clone());
    let state = state.clone();
    std::thread::Builder::new()
        .name(format!("session-{id}"))
        .spawn(move || {
            let mut scanner = shift_core::OscScanner::new();
            let mut buf = vec![0u8; 65536];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let bytes = buf[..n].to_vec();
                        for seg in scanner.scan(&bytes) {
                            if let Some(shift_core::Marker::Cwd(p)) = seg.marker {
                                *shell.cwd.lock().unwrap() = Some(p);
                            }
                        }
                        let at = ipc::now_ms();
                        let mut ring = shell.ring.lock().unwrap();
                        let (seq, bytes) = ring.push(at, bytes);
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
            shell.broadcast(|| Out::Event(Event::Exited { session: id }));
            state.shells.lock().unwrap().remove(&id);
        })?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

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
