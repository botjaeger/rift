//! The tunnel to the computer's local ports (feature `tunnel`).
//!
//! A paired phone with the `operate` permission opens a second connection
//! whose hello carries `"role":"tunnel"`. Its messages are binary
//! [`Frame`]s (one per Noise message, chunked like JSON ones), not JSON:
//! the phone runs a listener on its own loopback, and every connection a
//! browser makes to it becomes a stream here, connected to the same port on
//! this computer's loopback (`127.0.0.1`, else `::1`). Never to another
//! host: the tunnel reaches what a command on this computer could.
//!
//! Streams are numbered by the phone. Each side sends at most [`WINDOW`]
//! bytes of a stream the other has not acknowledged (`ack` after writing
//! them on), so a slow phone or local server stalls one stream, never the
//! connection. Spec: `docs/remote-protocol.md`, "Local web servers".

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::transport::{Channel, Recv, Socket};

/// Unacknowledged bytes of one stream either side may have in flight.
pub const WINDOW: u32 = 256 * 1024;
/// The most data one `data` frame carries.
pub const MAX_DATA: usize = 32 * 1024;
/// The largest frame either side sends: `data` with its header.
pub const MAX_FRAME: usize = MAX_DATA + 5;
/// Streams open at once on one tunnel.
pub const MAX_STREAMS: usize = 64;
/// Streams a tunnel may open per second (a page loading many files opens
/// a handful of connections, a dev server's reloads a few more).
pub const OPEN_RATE: usize = 50;
/// How long connecting to a local port may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a write to a local server may block before its stream is reset.
const LOCAL_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// The read's wait while streams are open, and while none are.
const BUSY_SLICE: Duration = Duration::from_millis(2);
const IDLE_SLICE: Duration = Duration::from_millis(25);

/// Why a stream was reset (`reset`'s reason byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetReason {
    /// Nothing listens on that port (or it refused).
    Refused = 1,
    /// Too many streams, or opened too fast.
    Limit = 2,
    /// Reading or writing the connection failed.
    Error = 3,
    /// The peer broke the protocol on this stream (an id in use, data past
    /// the window, data before `opened`).
    Protocol = 4,
    /// The peer gave the stream up.
    Aborted = 5,
}

impl ResetReason {
    fn from_byte(b: u8) -> Option<ResetReason> {
        Some(match b {
            1 => ResetReason::Refused,
            2 => ResetReason::Limit,
            3 => ResetReason::Error,
            4 => ResetReason::Protocol,
            5 => ResetReason::Aborted,
            _ => return None,
        })
    }
}

/// One tunnel message. Every frame starts with its op byte; stream ids and
/// counts are big-endian.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// `0x01 stream:u32 port:u16` phone → computer: connect `port`.
    Open {
        stream: u32,
        port: u16,
    },
    /// `0x02 stream:u32` computer → phone: connected; data may flow.
    Opened {
        stream: u32,
    },
    /// `0x03 stream:u32 bytes…` either way, 1 to [`MAX_DATA`] bytes.
    Data {
        stream: u32,
        data: Vec<u8>,
    },
    /// `0x04 stream:u32 bytes:u32` either way: that many were written on.
    Ack {
        stream: u32,
        bytes: u32,
    },
    /// `0x05 stream:u32` either way: the sender sends no more (half close).
    End {
        stream: u32,
    },
    /// `0x06 stream:u32 reason:u8` either way: the stream is gone.
    Reset {
        stream: u32,
        reason: ResetReason,
    },
    /// `0x07` phone → computer, answered by `0x08`.
    Ping,
    Pong,
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let head = |op: u8, stream: u32| {
            let mut v = Vec::with_capacity(9);
            v.push(op);
            v.extend_from_slice(&stream.to_be_bytes());
            v
        };
        match self {
            Frame::Open { stream, port } => {
                let mut v = head(1, *stream);
                v.extend_from_slice(&port.to_be_bytes());
                v
            }
            Frame::Opened { stream } => head(2, *stream),
            Frame::Data { stream, data } => {
                let mut v = head(3, *stream);
                v.extend_from_slice(data);
                v
            }
            Frame::Ack { stream, bytes } => {
                let mut v = head(4, *stream);
                v.extend_from_slice(&bytes.to_be_bytes());
                v
            }
            Frame::End { stream } => head(5, *stream),
            Frame::Reset { stream, reason } => {
                let mut v = head(6, *stream);
                v.push(*reason as u8);
                v
            }
            Frame::Ping => vec![7],
            Frame::Pong => vec![8],
        }
    }

    pub fn decode(b: &[u8]) -> Result<Frame, &'static str> {
        let (&op, rest) = b.split_first().ok_or("empty frame")?;
        if op == 7 || op == 8 {
            if !rest.is_empty() {
                return Err("bad length");
            }
            return Ok(if op == 7 { Frame::Ping } else { Frame::Pong });
        }
        if rest.len() < 4 {
            return Err("bad length");
        }
        let stream = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
        let body = &rest[4..];
        if stream == 0 {
            return Err("stream 0");
        }
        let exact = |n: usize| {
            if body.len() == n {
                Ok(())
            } else {
                Err("bad length")
            }
        };
        Ok(match op {
            1 => {
                exact(2)?;
                let port = u16::from_be_bytes([body[0], body[1]]);
                if port == 0 {
                    return Err("port 0");
                }
                Frame::Open { stream, port }
            }
            2 => {
                exact(0)?;
                Frame::Opened { stream }
            }
            3 => {
                if body.is_empty() || body.len() > MAX_DATA {
                    return Err("bad length");
                }
                Frame::Data {
                    stream,
                    data: body.to_vec(),
                }
            }
            4 => {
                exact(4)?;
                let bytes = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                if bytes == 0 || bytes > WINDOW {
                    return Err("bad ack");
                }
                Frame::Ack { stream, bytes }
            }
            5 => {
                exact(0)?;
                Frame::End { stream }
            }
            6 => {
                exact(1)?;
                Frame::Reset {
                    stream,
                    reason: ResetReason::from_byte(body[0]).ok_or("bad reason")?,
                }
            }
            _ => return Err("unknown op"),
        })
    }
}

/// Connect `port` on this computer's loopback: IPv4 first, then IPv6 (a
/// dev server bound to `localhost` may listen on `::1` only).
pub fn connect_local(port: u16) -> std::io::Result<TcpStream> {
    let v4 = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    match TcpStream::connect_timeout(&v4, CONNECT_TIMEOUT) {
        Ok(s) => Ok(s),
        Err(e4) => {
            let v6 = SocketAddr::from((Ipv6Addr::LOCALHOST, port));
            TcpStream::connect_timeout(&v6, CONNECT_TIMEOUT).map_err(|_| e4)
        }
    }
}

/// Bytes a stream may still send to the phone.
#[derive(Default)]
struct Credit {
    state: Mutex<(u64, bool)>,
    cv: Condvar,
}

impl Credit {
    fn new(n: u64) -> Credit {
        Credit {
            state: Mutex::new((n, false)),
            cv: Condvar::new(),
        }
    }

    /// Wait for credit; at most `max` of it. None once closed.
    fn take(&self, max: usize) -> Option<usize> {
        let mut s = self.state.lock().unwrap();
        while s.0 == 0 && !s.1 {
            s = self.cv.wait(s).unwrap();
        }
        if s.1 {
            return None;
        }
        let n = (s.0 as usize).min(max);
        s.0 -= n as u64;
        Some(n)
    }

    /// Give `n` back (an `ack`); false when that is past the window.
    fn give(&self, n: u64) -> bool {
        let mut s = self.state.lock().unwrap();
        if s.0 + n > u64::from(WINDOW) {
            return false;
        }
        s.0 += n;
        self.cv.notify_all();
        true
    }

    fn close(&self) {
        self.state.lock().unwrap().1 = true;
        self.cv.notify_all();
    }
}

/// What a stream's threads hand the connection's loop.
enum Out {
    Connected(u32, std::io::Result<TcpStream>),
    Data(u32, Vec<u8>),
    /// Written to the local server: the phone may send that many more.
    Wrote(u32, u32),
    /// The local server ended its side.
    Eof(u32),
    Failed(u32),
}

struct Stream {
    tcp: Option<TcpStream>,
    /// To the writer thread; an empty buffer ends the local side.
    local: Option<Sender<Vec<u8>>>,
    credit: Arc<Credit>,
    /// The phone's bytes not yet acknowledged.
    unacked: u64,
    /// `end` sent to the phone / received from it.
    sent_end: bool,
    got_end: bool,
}

impl Stream {
    fn shut(&mut self) {
        self.credit.close();
        self.local = None;
        if let Some(t) = self.tcp.take() {
            let _ = t.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.shut();
    }
}

/// One tunnel connection's state; [`serve`] drives it.
struct Tunnel<C> {
    streams: HashMap<u32, Stream>,
    /// Streams connecting: no data until `opened`.
    pending: HashMap<u32, ()>,
    tx: Sender<Out>,
    rx: Receiver<Out>,
    opens: std::collections::VecDeque<Instant>,
    connect: Arc<C>,
}

impl<C: Fn(u16) -> std::io::Result<TcpStream> + Send + Sync + 'static> Tunnel<C> {
    fn new(connect: C) -> Self {
        let (tx, rx) = channel();
        Tunnel {
            streams: HashMap::new(),
            pending: HashMap::new(),
            tx,
            rx,
            opens: Default::default(),
            connect: Arc::new(connect),
        }
    }

    fn open_allowed(&mut self, now: Instant) -> bool {
        while self
            .opens
            .front()
            .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1))
        {
            self.opens.pop_front();
        }
        if self.opens.len() >= OPEN_RATE || self.streams.len() + self.pending.len() >= MAX_STREAMS {
            return false;
        }
        self.opens.push_back(now);
        true
    }

    /// A frame from the phone; the frames to answer with.
    fn on_frame(&mut self, f: Frame, out: &mut Vec<Frame>) {
        match f {
            Frame::Open { stream, port } => {
                if self.streams.contains_key(&stream) || self.pending.contains_key(&stream) {
                    // the id is the phone's mistake: whatever had it goes
                    out.push(reset(stream, ResetReason::Protocol));
                    self.pending.remove(&stream);
                    self.drop_stream(stream);
                } else if !self.open_allowed(Instant::now()) {
                    out.push(reset(stream, ResetReason::Limit));
                } else {
                    self.pending.insert(stream, ());
                    let (tx, connect) = (self.tx.clone(), self.connect.clone());
                    std::thread::spawn(move || {
                        let _ = tx.send(Out::Connected(stream, connect(port)));
                    });
                }
            }
            Frame::Data { stream, data } => {
                let Some(s) = self.streams.get_mut(&stream) else {
                    if self.pending.remove(&stream).is_some() {
                        out.push(reset(stream, ResetReason::Protocol));
                    }
                    return;
                };
                s.unacked += data.len() as u64;
                // data after its `end`, or past the window: the phone broke
                // the protocol; a writer gone: writing failed
                let why = if s.got_end || s.unacked > u64::from(WINDOW) {
                    Some(ResetReason::Protocol)
                } else {
                    match s.local.as_ref().map(|l| l.send(data)) {
                        Some(Ok(())) => None,
                        Some(Err(_)) | None => Some(ResetReason::Error),
                    }
                };
                if let Some(why) = why {
                    out.push(reset(stream, why));
                    self.drop_stream(stream);
                }
            }
            Frame::Ack { stream, bytes } => {
                if let Some(s) = self.streams.get(&stream)
                    && !s.credit.give(u64::from(bytes))
                {
                    out.push(reset(stream, ResetReason::Protocol));
                    self.drop_stream(stream);
                }
            }
            Frame::End { stream } => {
                if let Some(s) = self.streams.get_mut(&stream) {
                    s.got_end = true;
                    if let Some(local) = &s.local {
                        let _ = local.send(Vec::new());
                    }
                    self.finish(stream);
                } else if self.pending.remove(&stream).is_some() {
                    out.push(reset(stream, ResetReason::Protocol));
                }
            }
            Frame::Reset { stream, .. } => {
                self.pending.remove(&stream);
                self.drop_stream(stream);
            }
            Frame::Ping => out.push(Frame::Pong),
            Frame::Opened { .. } | Frame::Pong => {}
        }
    }

    /// What the streams' threads reported; the frames for the phone.
    fn on_out(&mut self, o: Out, out: &mut Vec<Frame>) {
        match o {
            Out::Connected(stream, result) => {
                if self.pending.remove(&stream).is_none() {
                    return; // reset while connecting
                }
                match result {
                    Ok(tcp) => {
                        out.push(Frame::Opened { stream });
                        self.start(stream, tcp);
                    }
                    Err(_) => out.push(reset(stream, ResetReason::Refused)),
                }
            }
            Out::Data(stream, data) => {
                if self.streams.contains_key(&stream) {
                    out.push(Frame::Data { stream, data });
                }
            }
            Out::Wrote(stream, n) => {
                if let Some(s) = self.streams.get_mut(&stream) {
                    s.unacked = s.unacked.saturating_sub(u64::from(n));
                    out.push(Frame::Ack { stream, bytes: n });
                }
            }
            Out::Eof(stream) => {
                if let Some(s) = self.streams.get_mut(&stream) {
                    s.sent_end = true;
                    out.push(Frame::End { stream });
                    self.finish(stream);
                }
            }
            Out::Failed(stream) => {
                if self.streams.contains_key(&stream) {
                    out.push(reset(stream, ResetReason::Error));
                    self.drop_stream(stream);
                }
            }
        }
    }

    /// Connected: a reader (local → phone, within the credit) and a writer
    /// (phone → local) thread.
    fn start(&mut self, stream: u32, tcp: TcpStream) {
        let _ = tcp.set_nodelay(true);
        let _ = tcp.set_write_timeout(Some(LOCAL_WRITE_TIMEOUT));
        let (Ok(mut reader), Ok(mut writer)) = (tcp.try_clone(), tcp.try_clone()) else {
            let _ = self.tx.send(Out::Failed(stream));
            return;
        };
        let credit = Arc::new(Credit::new(u64::from(WINDOW)));
        // unbounded by count: the window bounds its bytes (data past it resets the stream), and
        // a burst of small frames (a page's WebSocket messages) or the `end` after a full window
        // must never be refused for the number of frames
        let (local_tx, local_rx) = channel::<Vec<u8>>();
        let tx = self.tx.clone();
        let c = credit.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; MAX_DATA];
            loop {
                let Some(n) = c.take(MAX_DATA) else { return };
                match reader.read(&mut buf[..n]) {
                    Ok(0) => {
                        let _ = tx.send(Out::Eof(stream));
                        return;
                    }
                    Ok(got) => {
                        // what was not read goes back
                        if got < n {
                            c.give((n - got) as u64);
                        }
                        if tx.send(Out::Data(stream, buf[..got].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                        c.give(n as u64);
                    }
                    Err(_) => {
                        if !c.state.lock().unwrap().1 {
                            let _ = tx.send(Out::Failed(stream));
                        }
                        return;
                    }
                }
            }
        });
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            for data in local_rx {
                if data.is_empty() {
                    let _ = writer.shutdown(Shutdown::Write);
                    return;
                }
                if writer.write_all(&data).is_err() {
                    let _ = tx.send(Out::Failed(stream));
                    return;
                }
                let _ = tx.send(Out::Wrote(stream, data.len() as u32));
            }
        });
        self.streams.insert(
            stream,
            Stream {
                tcp: Some(tcp),
                local: Some(local_tx),
                credit,
                unacked: 0,
                sent_end: false,
                got_end: false,
            },
        );
    }

    /// Both sides ended: the stream is done.
    fn finish(&mut self, stream: u32) {
        if self
            .streams
            .get(&stream)
            .is_some_and(|s| s.sent_end && s.got_end)
            && let Some(mut s) = self.streams.remove(&stream)
        {
            // ended both ways, not aborted: the writer still writes what the phone sent before
            // its `end`, then shuts its side; only a reset shuts the socket at once
            s.tcp = None;
        }
    }

    fn drop_stream(&mut self, stream: u32) {
        self.streams.remove(&stream);
    }
}

fn reset(stream: u32, reason: ResetReason) -> Frame {
    Frame::Reset { stream, reason }
}

/// Drive an authenticated tunnel connection until it ends: `stop` turns
/// nonzero (revoked, displaced, …), the phone goes quiet for `idle`, or the
/// socket fails. `heard` is called on every message from the phone.
/// `connect` opens a local port ([`connect_local`]; tests pass their own).
/// Returns why it ended (a history reason).
pub(crate) fn serve<T: Socket, C>(
    ch: &mut Channel<T>,
    stop: &AtomicU8,
    idle: Duration,
    heard: impl Fn(),
    connect: C,
) -> &'static str
where
    C: Fn(u16) -> std::io::Result<TcpStream> + Send + Sync + 'static,
{
    let mut t = Tunnel::new(connect);
    let mut last = Instant::now();
    let mut slice = IDLE_SLICE;
    let mut out = Vec::new();
    loop {
        if stop.load(Ordering::Acquire) != 0 {
            return "stopped";
        }
        loop {
            match t.rx.try_recv() {
                Ok(o) => t.on_out(o, &mut out),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
            if out.len() >= 64 {
                break;
            }
        }
        for f in out.drain(..) {
            if !ch.send_bytes(&f.encode()) {
                return "error";
            }
        }
        let want = if t.streams.is_empty() && t.pending.is_empty() {
            IDLE_SLICE
        } else {
            BUSY_SLICE
        };
        if want != slice && ch.ws.get_ref().set_read_timeout(Some(want)).is_ok() {
            slice = want;
        }
        match ch.recv() {
            Recv::Msg(m) => {
                last = Instant::now();
                heard();
                match Frame::decode(&m) {
                    Ok(f) => t.on_frame(f, &mut out),
                    Err(e) => {
                        log::info!("remote: tunnel: {e}");
                        return "bad_request";
                    }
                }
            }
            Recv::Partial | Recv::Alive => {}
            Recv::Idle => {
                if last.elapsed() > idle {
                    return "idle";
                }
            }
            Recv::Closed => return "closed",
            Recv::TooLarge => return "too_large",
            Recv::Violation(why) => {
                log::info!("remote: tunnel: {why}");
                return "bad_request";
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> Vec<(String, serde_json::Value)> {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/tunnel/frames.json");
        let all: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        all.as_array()
            .unwrap()
            .iter()
            .map(|c| (c["name"].as_str().unwrap().to_string(), c.clone()))
            .collect()
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// The frames the phone's code is tested against too
    /// (`mobile/src/tunnel/frames.test.ts`): each decodes to its fields and
    /// encodes back byte for byte; the bad ones are refused.
    #[test]
    fn every_tunnel_fixture_round_trips() {
        let all = fixtures();
        assert!(all.len() >= 12, "tunnel fixtures");
        for (name, c) in all {
            let bytes = hex(c["hex"].as_str().unwrap());
            if c.get("error").is_some() {
                assert!(Frame::decode(&bytes).is_err(), "{name} decoded");
                continue;
            }
            let f = Frame::decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            let stream = c["stream"].as_u64().map(|s| s as u32);
            let want = match c["op"].as_str().unwrap() {
                "open" => Frame::Open {
                    stream: stream.unwrap(),
                    port: c["port"].as_u64().unwrap() as u16,
                },
                "opened" => Frame::Opened {
                    stream: stream.unwrap(),
                },
                "data" => Frame::Data {
                    stream: stream.unwrap(),
                    data: c["data"].as_str().unwrap().as_bytes().to_vec(),
                },
                "ack" => Frame::Ack {
                    stream: stream.unwrap(),
                    bytes: c["bytes"].as_u64().unwrap() as u32,
                },
                "end" => Frame::End {
                    stream: stream.unwrap(),
                },
                "reset" => Frame::Reset {
                    stream: stream.unwrap(),
                    reason: ResetReason::from_byte(c["reason"].as_u64().unwrap() as u8).unwrap(),
                },
                "ping" => Frame::Ping,
                "pong" => Frame::Pong,
                other => panic!("{name}: op {other}"),
            };
            assert_eq!(f, want, "{name}");
            assert_eq!(f.encode(), bytes, "{name}");
        }
    }

    #[test]
    fn data_is_bounded() {
        let mut big = vec![3, 0, 0, 0, 1];
        big.extend(std::iter::repeat_n(b'x', MAX_DATA));
        assert!(Frame::decode(&big).is_ok());
        big.push(b'x');
        assert!(Frame::decode(&big).is_err());
        assert_eq!(MAX_FRAME, MAX_DATA + 5);
    }

    /// A phone that sends past its window (no write acknowledged it) gets the
    /// stream reset `protocol`; one window exactly is taken. The writer here
    /// never drains, as a local server that stopped reading.
    #[test]
    fn data_past_the_window_resets_the_stream() {
        let mut t = Tunnel::new(|_| Err(std::io::ErrorKind::ConnectionRefused.into()));
        let (local, _held) = channel::<Vec<u8>>();
        t.streams.insert(
            1,
            Stream {
                tcp: None,
                local: Some(local),
                credit: Arc::new(Credit::new(u64::from(WINDOW))),
                unacked: 0,
                sent_end: false,
                got_end: false,
            },
        );
        let mut out = Vec::new();
        for _ in 0..WINDOW as usize / MAX_DATA {
            t.on_frame(
                Frame::Data {
                    stream: 1,
                    data: vec![b'w'; MAX_DATA],
                },
                &mut out,
            );
        }
        assert!(out.is_empty(), "{out:?}");
        assert!(t.streams.contains_key(&1));
        t.on_frame(
            Frame::Data {
                stream: 1,
                data: vec![b'w'],
            },
            &mut out,
        );
        assert_eq!(
            out,
            [Frame::Reset {
                stream: 1,
                reason: ResetReason::Protocol
            }]
        );
        assert!(!t.streams.contains_key(&1));
    }
}
