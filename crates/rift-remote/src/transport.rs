//! Noise over WebSocket (`docs/remote-protocol.md` § Transport).
//!
//! `Noise_XX_25519_ChaChaPoly_BLAKE2s`, prologue `rift-remote/2`. Handshake
//! messages 1–3 are one binary WebSocket frame each (message 2 carries the
//! gateway's hello, message 3 the client's); after it every binary frame is
//! one Noise transport message whose plaintext is a flag byte (`0x00` last
//! chunk, `0x01` more follow) and at most [`MAX_CHUNK`] bytes of a JSON
//! message. A text frame is a protocol violation: the reader reports it and
//! the connection closes.
//!
//! Both roles are here: the gateway uses [`respond`], tests and tools acting
//! as a phone use [`connect`] (which pins the gateway's static key).

use crate::protocol::{ClientHello, GatewayHello};
use std::io::{ErrorKind, Read, Write};
use std::time::Instant;
use tungstenite::{Message, WebSocket};

/// The Noise protocol name.
pub const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
/// The handshake prologue (ASCII).
pub const PROLOGUE: &[u8] = b"rift-remote/2";
/// JSON bytes per transport message (after the flag byte).
pub const MAX_CHUNK: usize = 65_000;
/// The flag of a message's last chunk.
pub const LAST: u8 = 0x00;
/// The flag of a chunk that more chunks follow.
pub const MORE: u8 = 0x01;
/// Largest Noise message (the spec's limit).
pub const NOISE_MAX: usize = 65_535;
/// AEAD tag per encrypted payload.
const TAG: usize = 16;
/// Static and ephemeral X25519 keys.
pub const KEY_LEN: usize = 32;

/// A builder for [`PATTERN`] with [`PROLOGUE`] set.
pub fn builder<'a>() -> snow::Builder<'a> {
    let params = PATTERN.parse().expect("a valid Noise pattern");
    snow::Builder::new(params)
        .prologue(PROLOGUE)
        .expect("a prologue is set once")
}

/// A fresh X25519 private key from the OS RNG.
pub fn generate_private_key() -> Result<[u8; KEY_LEN], snow::Error> {
    let kp = builder().generate_keypair()?;
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&kp.private);
    Ok(key)
}

/// The X25519 public key of `private`.
pub fn public_key(private: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
    use snow::resolvers::{CryptoResolver, DefaultResolver};
    let mut dh = DefaultResolver
        .resolve_dh(&snow::params::DHChoice::Curve25519)
        .expect("curve25519 is compiled in");
    dh.set(private);
    let mut out = [0u8; KEY_LEN];
    out.copy_from_slice(dh.pubkey());
    out
}

/// `n` random bytes from the OS RNG.
pub fn random_bytes<const N: usize>() -> std::io::Result<[u8; N]> {
    use snow::resolvers::{CryptoResolver, DefaultResolver};
    let mut out = [0u8; N];
    DefaultResolver
        .resolve_rng()
        .ok_or_else(|| std::io::Error::other("no RNG"))?
        .try_fill_bytes(&mut out)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(out)
}

/// Encrypt one JSON message as transport messages (one per chunk).
pub fn seal(noise: &mut snow::TransportState, json: &[u8]) -> Result<Vec<Vec<u8>>, snow::Error> {
    seal_with(noise, json, MAX_CHUNK)
}

fn seal_with(
    noise: &mut snow::TransportState,
    json: &[u8],
    chunk: usize,
) -> Result<Vec<Vec<u8>>, snow::Error> {
    let mut out = Vec::with_capacity(json.len() / chunk + 1);
    let mut chunks = json.chunks(chunk).peekable();
    let mut plain = Vec::with_capacity(chunk + 1);
    let mut buf = vec![0u8; NOISE_MAX];
    loop {
        let chunk = chunks.next().unwrap_or_default();
        let more = chunks.peek().is_some();
        plain.clear();
        plain.push(if more { MORE } else { LAST });
        plain.extend_from_slice(chunk);
        let n = noise.write_message(&plain, &mut buf)?;
        out.push(buf[..n].to_vec());
        if !more {
            return Ok(out);
        }
    }
}

/// Reassembles chunked JSON messages from decrypted transport plaintexts.
#[derive(Debug)]
pub struct Reassembler {
    buf: Vec<u8>,
    limit: usize,
}

/// A transport plaintext that breaks the framing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The message being assembled passed the limit.
    TooLarge,
    /// Empty plaintext, or a flag other than `0x00` / `0x01`.
    BadFlag,
}

impl Reassembler {
    /// Messages longer than `limit` bytes of JSON are refused.
    pub fn new(limit: usize) -> Reassembler {
        Reassembler {
            buf: Vec::new(),
            limit,
        }
    }

    /// Add one chunk; the whole message once its last chunk arrives.
    pub fn push(&mut self, plain: &[u8]) -> Result<Option<Vec<u8>>, FrameError> {
        let (&flag, data) = plain.split_first().ok_or(FrameError::BadFlag)?;
        if flag != LAST && flag != MORE {
            return Err(FrameError::BadFlag);
        }
        if data.len() > MAX_CHUNK || self.buf.len() + data.len() > self.limit {
            self.buf.clear();
            return Err(FrameError::TooLarge);
        }
        self.buf.extend_from_slice(data);
        Ok((flag == LAST).then(|| std::mem::take(&mut self.buf)))
    }
}

/// One read from a [`Channel`].
#[derive(Debug)]
pub enum Recv {
    /// A whole JSON message.
    Msg(Vec<u8>),
    /// A WebSocket ping or pong: the socket's other end is alive (through a
    /// relay, that is the relay, which answers pings itself).
    Alive,
    /// A non-final chunk: the Noise peer is alive.
    Partial,
    /// Nothing within the socket's read timeout.
    Idle,
    /// The message passed the limit (or the frame the WebSocket limit).
    TooLarge,
    /// A text frame, a frame that does not decrypt, or broken chunk flags.
    Violation(&'static str),
    Closed,
}

/// A byte stream a connection runs over: a socket the gateway accepted
/// ([`Timed`]) or one it dialed at a relay (plain or TLS). The gateway
/// reads in short slices, so it needs the read timeout.
pub trait Socket: Read + Write + Send + 'static {
    fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> std::io::Result<()>;
    /// Lift the deadline of a [`Timed`] socket (others have none).
    fn end_deadline(&mut self) {}
}

impl Socket for std::net::TcpStream {
    fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> std::io::Result<()> {
        std::net::TcpStream::set_read_timeout(self, timeout)
    }
}

/// An accepted socket whose reads fail with `TimedOut` once its deadline
/// passed: a read timeout alone restarts with every byte, so a peer that
/// sends its HTTP upgrade or handshake a byte at a time would hold the
/// socket for ever. The deadline holds until [`Socket::end_deadline`].
pub struct Timed {
    stream: std::net::TcpStream,
    deadline: Option<Instant>,
    /// The read timeout the owner set (each read waits at most the smaller
    /// of it and the time left).
    timeout: std::cell::Cell<Option<std::time::Duration>>,
}

impl Timed {
    pub fn new(stream: std::net::TcpStream, deadline: Instant) -> Timed {
        let timeout = stream.read_timeout().ok().flatten();
        Timed {
            stream,
            deadline: Some(deadline),
            timeout: std::cell::Cell::new(timeout),
        }
    }
}

impl std::ops::Deref for Timed {
    type Target = std::net::TcpStream;
    fn deref(&self) -> &std::net::TcpStream {
        &self.stream
    }
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(std::io::Error::from(ErrorKind::TimedOut));
            }
            let wait = self.timeout.get().map_or(left, |t| t.min(left));
            self.stream.set_read_timeout(Some(wait))?;
        }
        self.stream.read(buf)
    }
}

impl Write for Timed {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stream.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

impl Socket for Timed {
    fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> std::io::Result<()> {
        self.timeout.set(timeout);
        self.stream.set_read_timeout(timeout)
    }
    fn end_deadline(&mut self) {
        if self.deadline.take().is_some() {
            let _ = self.stream.set_read_timeout(self.timeout.get());
        }
    }
}

impl Socket for tungstenite::stream::MaybeTlsStream<std::net::TcpStream> {
    fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> std::io::Result<()> {
        use tungstenite::stream::MaybeTlsStream;
        #[allow(unreachable_patterns)]
        match self {
            MaybeTlsStream::Plain(s) => s.set_read_timeout(timeout),
            #[cfg(feature = "tls")]
            MaybeTlsStream::Rustls(s) => s.sock.set_read_timeout(timeout),
            _ => Err(std::io::Error::other("unsupported stream")),
        }
    }
}

/// An established connection: binary frames of Noise transport messages.
pub struct Channel<S: Read + Write> {
    pub ws: WebSocket<S>,
    noise: snow::TransportState,
    rx: Reassembler,
    remote: [u8; KEY_LEN],
}

impl<S: Read + Write> Channel<S> {
    /// `limit`: the largest JSON message accepted from the peer.
    pub fn new(ws: WebSocket<S>, noise: snow::TransportState, limit: usize) -> Channel<S> {
        let mut remote = [0u8; KEY_LEN];
        if let Some(k) = noise.get_remote_static() {
            remote.copy_from_slice(k);
        }
        Channel {
            ws,
            noise,
            rx: Reassembler::new(limit),
            remote,
        }
    }

    /// The peer's static public key.
    pub fn remote_static(&self) -> [u8; KEY_LEN] {
        self.remote
    }

    /// Accept JSON messages up to `limit` bytes from now on (a message
    /// being assembled is held to the new limit too).
    pub fn set_limit(&mut self, limit: usize) {
        self.rx.limit = limit;
    }

    /// Send one JSON message (chunked, encrypted). False when the socket or
    /// the cipher failed.
    pub fn send(&mut self, json: &str) -> bool {
        self.send_chunked(json, MAX_CHUNK)
    }

    /// [`Channel::send`] in chunks of at most `chunk` bytes (≤
    /// [`MAX_CHUNK`]); tests and interop tools use small ones.
    pub fn send_chunked(&mut self, json: &str, chunk: usize) -> bool {
        let Ok(frames) = seal_with(&mut self.noise, json.as_bytes(), chunk.clamp(1, MAX_CHUNK))
        else {
            return false;
        };
        for f in frames {
            if self.ws.write(Message::binary(f)).is_err() {
                return false;
            }
        }
        self.ws.flush().is_ok()
    }

    /// Read the next frame.
    pub fn recv(&mut self) -> Recv {
        let frame = match self.ws.read() {
            Ok(Message::Binary(b)) => b,
            Ok(Message::Text(_)) => return Recv::Violation("text frame"),
            Ok(Message::Close(_)) => return Recv::Closed,
            Ok(_) => return Recv::Alive,
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                return Recv::Idle;
            }
            Err(tungstenite::Error::Capacity(_)) => return Recv::TooLarge,
            Err(_) => return Recv::Closed,
        };
        let mut plain = vec![0u8; frame.len()];
        let Ok(n) = self.noise.read_message(&frame, &mut plain) else {
            return Recv::Violation("frame does not decrypt");
        };
        match self.rx.push(&plain[..n]) {
            Ok(Some(msg)) => Recv::Msg(msg),
            Ok(None) => Recv::Partial,
            Err(FrameError::TooLarge) => Recv::TooLarge,
            Err(FrameError::BadFlag) => Recv::Violation("bad chunk flag"),
        }
    }

    /// Close the WebSocket (best effort).
    pub fn close(mut self) {
        let _ = self.ws.close(None);
        let _ = self.ws.flush();
    }
}

/// Why a handshake did not complete.
#[derive(Debug)]
pub enum HandshakeError {
    /// The deadline passed.
    Timeout,
    /// The peer closed, or the socket failed.
    Closed,
    /// A text frame or a frame larger than allowed.
    Violation(&'static str),
    /// Noise rejected a message (wrong prologue, pattern, corrupt data).
    Noise(snow::Error),
    /// The gateway's static key is not the pinned one (client side).
    Pinning,
    /// A handshake payload is not the expected JSON.
    Payload(String),
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandshakeError::Timeout => write!(f, "handshake timed out"),
            HandshakeError::Closed => write!(f, "connection closed during the handshake"),
            HandshakeError::Violation(w) => write!(f, "handshake: {w}"),
            HandshakeError::Noise(e) => write!(f, "handshake: {e}"),
            HandshakeError::Pinning => write!(f, "the gateway's key is not the paired one"),
            HandshakeError::Payload(e) => write!(f, "handshake payload: {e}"),
        }
    }
}

impl std::error::Error for HandshakeError {}

/// The next binary frame before `deadline` (the socket's read timeout must
/// be short: it is the polling slice).
fn read_binary<S: Read + Write>(
    ws: &mut WebSocket<S>,
    deadline: Option<Instant>,
) -> Result<Vec<u8>, HandshakeError> {
    loop {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(HandshakeError::Timeout);
        }
        match ws.read() {
            Ok(Message::Binary(b)) => return Ok(b.to_vec()),
            Ok(Message::Text(_)) => return Err(HandshakeError::Violation("text frame")),
            Ok(Message::Close(_)) => return Err(HandshakeError::Closed),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(tungstenite::Error::Capacity(_)) => {
                return Err(HandshakeError::Violation("frame too large"));
            }
            Err(_) => return Err(HandshakeError::Closed),
        }
    }
}

fn write_binary<S: Read + Write>(ws: &mut WebSocket<S>, b: &[u8]) -> Result<(), HandshakeError> {
    ws.send(Message::binary(b.to_vec()))
        .map_err(|_| HandshakeError::Closed)
}

/// The gateway's side: run the handshake on `ws` with the static key
/// `private`, sending `hello` (JSON) in message 2. Returns the transport, the
/// client's message-3 payload and the client's static key.
pub fn respond<S: Read + Write>(
    ws: &mut WebSocket<S>,
    private: &[u8; KEY_LEN],
    hello: &[u8],
    deadline: Instant,
) -> Result<(snow::TransportState, Vec<u8>), HandshakeError> {
    let mut hs = builder()
        .local_private_key(private)
        .and_then(|b| b.build_responder())
        .map_err(HandshakeError::Noise)?;
    let mut buf = vec![0u8; NOISE_MAX];
    let m1 = read_binary(ws, Some(deadline))?;
    hs.read_message(&m1, &mut buf)
        .map_err(HandshakeError::Noise)?;
    let n = hs
        .write_message(hello, &mut buf)
        .map_err(HandshakeError::Noise)?;
    write_binary(ws, &buf[..n])?;
    let m3 = read_binary(ws, Some(deadline))?;
    let n = hs
        .read_message(&m3, &mut buf)
        .map_err(HandshakeError::Noise)?;
    let payload = buf[..n].to_vec();
    let noise = hs.into_transport_mode().map_err(HandshakeError::Noise)?;
    Ok((noise, payload))
}

/// The client's side (phones, tests, tools): run the handshake with the
/// device key `private`, abort when the gateway's static key is not
/// `pinned`, send `hello` in message 3. Returns the channel and the
/// gateway's hello.
pub fn connect<S: Read + Write>(
    mut ws: WebSocket<S>,
    private: &[u8; KEY_LEN],
    pinned: &[u8; KEY_LEN],
    hello: &ClientHello,
    limit: usize,
) -> Result<(Channel<S>, GatewayHello), HandshakeError> {
    let mut hs = builder()
        .local_private_key(private)
        .and_then(|b| b.build_initiator())
        .map_err(HandshakeError::Noise)?;
    let mut buf = vec![0u8; NOISE_MAX];
    let n = hs
        .write_message(&[], &mut buf)
        .map_err(HandshakeError::Noise)?;
    write_binary(&mut ws, &buf[..n])?;
    let m2 = read_binary(&mut ws, None)?;
    let n = hs
        .read_message(&m2, &mut buf)
        .map_err(HandshakeError::Noise)?;
    if hs.get_remote_static() != Some(&pinned[..]) {
        return Err(HandshakeError::Pinning);
    }
    let gateway: GatewayHello =
        serde_json::from_slice(&buf[..n]).map_err(|e| HandshakeError::Payload(e.to_string()))?;
    let body = serde_json::to_vec(hello).map_err(|e| HandshakeError::Payload(e.to_string()))?;
    let n = hs
        .write_message(&body, &mut buf)
        .map_err(HandshakeError::Noise)?;
    write_binary(&mut ws, &buf[..n])?;
    let noise = hs.into_transport_mode().map_err(HandshakeError::Noise)?;
    Ok((Channel::new(ws, noise, limit), gateway))
}

/// Largest WebSocket frame a client may send the gateway: the handshake's
/// message 3 (a 48-byte encrypted static key, a hello of at most
/// `max_message` bytes and its tag) or one transport chunk of a
/// `max_message`-byte JSON message.
pub fn max_client_frame(max_message: usize) -> usize {
    KEY_LEN + TAG + max_message + 1 + TAG
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (snow::TransportState, snow::TransportState) {
        let a = generate_private_key().unwrap();
        let b = generate_private_key().unwrap();
        let mut i = builder()
            .local_private_key(&a)
            .unwrap()
            .build_initiator()
            .unwrap();
        let mut r = builder()
            .local_private_key(&b)
            .unwrap()
            .build_responder()
            .unwrap();
        let (mut m, mut p) = (vec![0u8; NOISE_MAX], vec![0u8; NOISE_MAX]);
        let n = i.write_message(&[], &mut m).unwrap();
        r.read_message(&m[..n], &mut p).unwrap();
        let n = r.write_message(b"{}", &mut m).unwrap();
        i.read_message(&m[..n], &mut p).unwrap();
        let n = i.write_message(b"{}", &mut m).unwrap();
        r.read_message(&m[..n], &mut p).unwrap();
        (
            i.into_transport_mode().unwrap(),
            r.into_transport_mode().unwrap(),
        )
    }

    fn open(r: &mut snow::TransportState, frames: &[Vec<u8>], limit: usize) -> Vec<Vec<u8>> {
        let mut asm = Reassembler::new(limit);
        let mut out = Vec::new();
        for f in frames {
            assert!(f.len() <= NOISE_MAX);
            let mut p = vec![0u8; f.len()];
            let n = r.read_message(f, &mut p).unwrap();
            if let Some(m) = asm.push(&p[..n]).unwrap() {
                out.push(m);
            }
        }
        out
    }

    #[test]
    fn messages_are_chunked_at_65000_bytes() {
        let (mut i, mut r) = pair();
        for len in [
            0,
            1,
            MAX_CHUNK - 1,
            MAX_CHUNK,
            MAX_CHUNK + 1,
            3 * MAX_CHUNK + 7,
        ] {
            let json: Vec<u8> = (0..len).map(|x| b'a' + (x % 26) as u8).collect();
            let frames = seal(&mut i, &json).unwrap();
            assert_eq!(frames.len(), len.div_ceil(MAX_CHUNK).max(1), "{len}");
            // every chunk but the last is full
            for f in &frames[..frames.len() - 1] {
                assert_eq!(f.len(), 1 + MAX_CHUNK + TAG);
            }
            assert_eq!(open(&mut r, &frames, usize::MAX), vec![json]);
        }
    }

    #[test]
    fn reassembly_bounds_size_and_flags() {
        let mut asm = Reassembler::new(10);
        assert_eq!(asm.push(b"\x01abcde"), Ok(None));
        assert_eq!(asm.push(b"\x00fghij"), Ok(Some(b"abcdefghij".to_vec())));
        assert_eq!(asm.push(b"\x01abcdef"), Ok(None));
        assert_eq!(asm.push(b"\x00ghijk"), Err(FrameError::TooLarge));
        // a refused message leaves nothing behind
        assert_eq!(asm.push(b"\x00ok"), Ok(Some(b"ok".to_vec())));
        assert_eq!(asm.push(b""), Err(FrameError::BadFlag));
        assert_eq!(asm.push(b"\x02x"), Err(FrameError::BadFlag));
        assert_eq!(asm.push(b"{}"), Err(FrameError::BadFlag));
    }

    #[test]
    fn public_key_matches_the_handshake() {
        let private = generate_private_key().unwrap();
        let (i, _) = {
            let other = generate_private_key().unwrap();
            let mut i = builder()
                .local_private_key(&other)
                .unwrap()
                .build_initiator()
                .unwrap();
            let mut r = builder()
                .local_private_key(&private)
                .unwrap()
                .build_responder()
                .unwrap();
            let (mut m, mut p) = (vec![0u8; NOISE_MAX], vec![0u8; NOISE_MAX]);
            let n = i.write_message(&[], &mut m).unwrap();
            r.read_message(&m[..n], &mut p).unwrap();
            let n = r.write_message(&[], &mut m).unwrap();
            i.read_message(&m[..n], &mut p).unwrap();
            (i, r)
        };
        assert_eq!(i.get_remote_static().unwrap(), &public_key(&private));
        assert_ne!(random_bytes::<16>().unwrap(), random_bytes::<16>().unwrap());
    }
}
