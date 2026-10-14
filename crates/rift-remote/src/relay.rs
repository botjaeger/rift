//! The relay (`docs/remote-protocol.md` § Relay): splices a phone's
//! WebSocket to the gateway it names, for use outside the local network.
//!
//! It holds no key and parses nothing inside the spliced frames: every
//! binary frame is forwarded unchanged (only WebSocket framing is redone
//! per hop), so it only ever sees Noise ciphertext and the routing id.
//!
//! Routes (plain `ws://`; a TLS reverse proxy goes in front):
//!
//! - `/gateway`: a gateway's control connection. Its first text frame is
//!   `register` with its routing id and static public key; the relay checks
//!   that the id is the key's and that the gateway holds the key (a
//!   `Noise_NK` handshake to it: `prove` / `proof`), so only the gateway
//!   can claim its id. The relay sends `open` on it for every phone waiting
//!   on that id.
//! - `/connect?id=`: a phone. Refused (HTTP) when the id is offline, holds
//!   [`MAX_PAIRS`] pairs, or the address passed [`CONNECT_RATE`] per
//!   [`CONNECT_WINDOW`]. Closed when the gateway does not accept within
//!   [`ACCEPT_DEADLINE`].
//! - `/accept?id=&conn=`: the gateway's side of one phone connection, with
//!   its secret in `Authorization: Bearer` (compared in constant time; kept
//!   out of the URL, which proxies log).
//!
//! Every socket must finish its HTTP upgrade (and a gateway its
//! registration) within [`REQUEST_DEADLINE`] in total, and one address
//! holds at most [`MAX_SOCKETS_PER_ADDRESS`] sockets (IPv6: per /64).
//!
//! One thread per socket; a spliced pair is one thread that `poll`s both
//! sockets (non-blocking, with write back-pressure: a side whose peer
//! cannot take more is not read). Nothing is logged about frame contents.
//!
//! This module also has the client side every relay user needs:
//! [`RelayUrl`] (`ws://` / `wss://` URLs) and [`dial`].

use crate::protocol::{RelayError, RelayMsg};
use crate::registry::{RELAY_SECRET_LEN, ROUTING_ID_LEN, ct_eq, decode_key, routing_id};
use crate::transport::{self, KEY_LEN, Socket, Timed};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::protocol::frame::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

/// Where `rift-terminal relay` / `rift-relay` listen by default.
pub const DEFAULT_LISTEN: &str = "0.0.0.0:8790";
/// Concurrent spliced (or waiting) pairs per routing id.
pub const MAX_PAIRS: usize = 32;
/// Largest WebSocket frame (and message) the relay forwards.
pub const MAX_FRAME: usize = 1024 * 1024;
/// A pair with no binary frame in either direction this long is closed.
pub const PAIR_IDLE: Duration = Duration::from_secs(60);
/// `/connect` requests per remote address within [`CONNECT_WINDOW`].
pub const CONNECT_RATE: usize = 30;
pub const CONNECT_WINDOW: Duration = Duration::from_secs(60);
/// Routing ids the relay remembers; past it, a new id replaces the one
/// offline (and without pairs) the longest.
pub const MAX_IDS: usize = 1000;
/// Open sockets per client address (IPv6: per /64), unless
/// `trust_forwarded` (then every socket comes from the proxy).
pub const MAX_SOCKETS_PER_ADDRESS: usize = 64;
/// Largest control message (text frame).
pub const MAX_CONTROL: usize = 1024;
/// A phone whose gateway did not `/accept` this soon is closed.
pub const ACCEPT_DEADLINE: Duration = Duration::from_secs(10);
/// The HTTP upgrade (and a gateway's registration and proof) must be
/// done this soon after the socket opens, in total.
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(10);
/// The relay pings each control connection this often ...
pub const CONTROL_PING: Duration = Duration::from_secs(20);
/// ... and drops one silent this long.
pub const CONTROL_IDLE: Duration = Duration::from_secs(60);
/// Open sockets past which new ones are dropped unread.
pub const MAX_SOCKETS: usize = 4096;
/// The same for a relay that serves one gateway (`only_ids` of one id, the
/// Cloudflare tunnel in the gateway's own process): its pairs
/// (two sockets each), the control connection and upgrades under way. Its
/// sockets come through one proxy, so no per-address limit applies; this
/// bound keeps a flood from the internet from spending the process's
/// threads, which the gateway's LAN connections need too.
pub const ONE_GATEWAY_SOCKETS: usize = 4 * MAX_PAIRS + 64;
/// Random bytes of a `conn` id.
pub const CONN_ID_LEN: usize = 16;

/// Which way a spliced frame went (for the test tap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leg {
    ToGateway,
    ToPhone,
}

/// Sees every binary frame the relay forwards (tests only: proving that
/// what passes through is opaque).
pub type Tap = Arc<dyn Fn(Leg, &[u8]) + Send + Sync>;

/// Relay settings. [`RelayConfig::default`] has the protocol's limits;
/// tests shorten the timers.
#[derive(Clone)]
pub struct RelayConfig {
    pub max_pairs: usize,
    pub max_frame: usize,
    pub pair_idle: Duration,
    pub connect_rate: usize,
    pub connect_window: Duration,
    pub max_ids: usize,
    pub max_per_address: usize,
    pub accept_deadline: Duration,
    pub request_deadline: Duration,
    /// Open sockets past which new ones are dropped unread ([`MAX_SOCKETS`];
    /// a relay serving one gateway needs far fewer).
    pub max_sockets: usize,
    pub control_ping: Duration,
    pub control_idle: Duration,
    /// Take the client address from the last `X-Forwarded-For` hop (only
    /// behind a reverse proxy you run: otherwise anyone picks their own
    /// address for the `/connect` rate limit).
    pub trust_forwarded: bool,
    /// Only these routing ids may register (None: any). A relay that serves
    /// one gateway (Rift's own Cloudflare tunnel) is no open
    /// relay for strangers' gateways.
    pub only_ids: Option<Vec<String>>,
    pub tap: Option<Tap>,
}

impl Default for RelayConfig {
    fn default() -> RelayConfig {
        RelayConfig {
            max_pairs: MAX_PAIRS,
            max_frame: MAX_FRAME,
            pair_idle: PAIR_IDLE,
            connect_rate: CONNECT_RATE,
            connect_window: CONNECT_WINDOW,
            max_ids: MAX_IDS,
            max_per_address: MAX_SOCKETS_PER_ADDRESS,
            accept_deadline: ACCEPT_DEADLINE,
            request_deadline: REQUEST_DEADLINE,
            max_sockets: MAX_SOCKETS,
            control_ping: CONTROL_PING,
            control_idle: CONTROL_IDLE,
            trust_forwarded: false,
            only_ids: None,
            tap: None,
        }
    }
}

// ── client side: relay URLs and dialing ──

/// A relay URL: `ws://` or `wss://`, a host, an optional port and path
/// prefix (`wss://example.com/rift` serves `/rift/gateway`, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayUrl {
    pub tls: bool,
    /// Host name or IP (IPv6 without brackets).
    pub host: String,
    pub port: u16,
    /// Path prefix without a trailing `/` (often empty).
    pub prefix: String,
}

impl RelayUrl {
    /// Parse a `ws://` / `wss://` URL (no user info, query or fragment).
    pub fn parse(text: &str) -> Result<RelayUrl, String> {
        let text = text.trim();
        let bad = || format!("{text:?} is not a ws:// or wss:// URL");
        let (tls, rest) = if let Some(r) = strip_prefix_ci(text, "wss://") {
            (true, r)
        } else if let Some(r) = strip_prefix_ci(text, "ws://") {
            (false, r)
        } else {
            return Err(bad());
        };
        if rest.contains(['?', '#', '@'])
            || rest.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(bad());
        }
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (h, after) = v6.split_once(']').ok_or_else(bad)?;
            let port = match after.strip_prefix(':') {
                Some(p) => Some(p),
                None if after.is_empty() => None,
                None => return Err(bad()),
            };
            (h.to_string(), port)
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), Some(p)),
                None => (authority.to_string(), None),
            }
        };
        if host.is_empty() {
            return Err(bad());
        }
        let port = match port {
            Some(p) => p.parse::<u16>().ok().filter(|&p| p > 0).ok_or_else(bad)?,
            None if tls => 443,
            None => 80,
        };
        Ok(RelayUrl {
            tls,
            host: host.to_ascii_lowercase(),
            port,
            prefix: path.trim_end_matches('/').to_string(),
        })
    }

    /// `host:port` (brackets around IPv6), as connection history shows it.
    pub fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// The URL of `path_query` (starting with `/`) on this relay.
    pub fn endpoint(&self, path_query: &str) -> String {
        format!(
            "{}://{}{}{}",
            if self.tls { "wss" } else { "ws" },
            self.authority(),
            self.prefix,
            path_query
        )
    }
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// A WebSocket to a relay (plain or TLS).
pub type RelaySocket = WebSocket<MaybeTlsStream<TcpStream>>;

/// Open `path_query` on `relay` as a WebSocket client with `config`, every
/// step (DNS, TCP, TLS, upgrade) within about `timeout`. The socket keeps
/// `timeout` as its read timeout; once open, writes may block for
/// 5 s, as on a direct socket
/// (a short dial timeout would end a session whose phone reads slowly).
pub fn dial(
    relay: &RelayUrl,
    path_query: &str,
    bearer: Option<&str>,
    config: WebSocketConfig,
    timeout: Duration,
) -> Result<RelaySocket, String> {
    let addrs: Vec<SocketAddr> = (relay.host.as_str(), relay.port)
        .to_socket_addrs()
        .map_err(|e| format!("{}: {e}", relay.host))?
        .collect();
    let mut last = format!("{}: no address", relay.host);
    let mut stream = None;
    for a in addrs {
        match TcpStream::connect_timeout(&a, timeout) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = format!("cannot reach {}: {e}", relay.authority()),
        }
    }
    let stream = stream.ok_or(last)?;
    let _ = stream.set_nodelay(true);
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|e| e.to_string())?;
    // the same socket, to lift the write timeout once the upgrade is done
    let raw = stream.try_clone().map_err(|e| e.to_string())?;
    let mut request = tungstenite::client::IntoClientRequest::into_client_request(
        relay.endpoint(path_query).as_str(),
    )
    .map_err(|e| e.to_string())?;
    if let Some(b) = bearer {
        let value = format!("Bearer {b}").parse().map_err(|_| "bad secret")?;
        request.headers_mut().insert("authorization", value);
    }
    let ws = if relay.tls {
        #[cfg(feature = "tls")]
        {
            tungstenite::client_tls_with_config(request, stream, Some(config), None)
                .map(|(ws, _)| ws)
                .map_err(|e| handshake_error(&e))?
        }
        #[cfg(not(feature = "tls"))]
        {
            let _ = (request, config, raw);
            return Err("wss:// needs a build with TLS support".into());
        }
    } else {
        tungstenite::client::client_with_config(
            request,
            MaybeTlsStream::Plain(stream),
            Some(config),
        )
        .map(|(ws, _)| ws)
        .map_err(|e| handshake_error(&e))?
    };
    raw.set_write_timeout(Some(crate::server::WRITE_TIMEOUT))
        .map_err(|e| e.to_string())?;
    Ok(ws)
}

fn handshake_error<R: tungstenite::handshake::HandshakeRole>(
    e: &tungstenite::HandshakeError<R>,
) -> String {
    match e {
        tungstenite::HandshakeError::Failure(tungstenite::Error::Http(r)) => {
            let body = r
                .body()
                .as_deref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            format!(
                "relay answered HTTP {} {}",
                r.status().as_u16(),
                body.trim()
            )
            .trim()
            .to_string()
        }
        tungstenite::HandshakeError::Failure(e) => e.to_string(),
        tungstenite::HandshakeError::Interrupted(_) => "the relay did not answer in time".into(),
    }
}

/// A routing id as the relay accepts it: base64url of 16 bytes.
pub fn valid_id(id: &str) -> bool {
    id.len() <= 32
        && B64
            .decode(id.as_bytes())
            .is_ok_and(|b| b.len() == ROUTING_ID_LEN)
}

/// The key proof of a registration: `Noise_NK_25519_ChaChaPoly_BLAKE2s`
/// from the relay to the key the gateway names. Only the key's holder can
/// read message 1 and write a message 2 that the relay accepts.
const PROOF_PATTERN: &str = "Noise_NK_25519_ChaChaPoly_BLAKE2s";
const PROOF_PROLOGUE: &[u8] = b"rift-relay/1";
/// Messages 1 and 2 are 48 bytes (an ephemeral key and an empty payload's tag).
const PROOF_MAX: usize = 128;

fn proof_builder<'a>() -> snow::Builder<'a> {
    snow::Builder::new(PROOF_PATTERN.parse().expect("a valid Noise pattern"))
        .prologue(PROOF_PROLOGUE)
        .expect("a prologue is set once")
}

/// The relay's side: message 1 to `key`, and the state that checks the
/// answer with [`verify_proof`].
pub fn challenge(key: &[u8; KEY_LEN]) -> Result<(snow::HandshakeState, Vec<u8>), snow::Error> {
    let mut hs = proof_builder()
        .remote_public_key(key)
        .and_then(|b| b.build_initiator())?;
    let mut buf = [0u8; PROOF_MAX];
    let n = hs.write_message(&[], &mut buf)?;
    Ok((hs, buf[..n].to_vec()))
}

/// Whether `answer` is message 2 from the holder of the challenged key.
pub fn verify_proof(mut hs: snow::HandshakeState, answer: &[u8]) -> bool {
    let mut buf = [0u8; PROOF_MAX];
    answer.len() <= PROOF_MAX && hs.read_message(answer, &mut buf).is_ok()
}

/// The gateway's side: message 2 for the relay's `challenge`.
pub fn prove(private: &[u8; KEY_LEN], challenge: &[u8]) -> Result<Vec<u8>, snow::Error> {
    let mut hs = proof_builder()
        .local_private_key(private)
        .and_then(|b| b.build_responder())?;
    let mut buf = [0u8; PROOF_MAX];
    if challenge.len() > PROOF_MAX {
        return Err(snow::Error::Input);
    }
    hs.read_message(challenge, &mut buf)?;
    let n = hs.write_message(&[], &mut buf)?;
    Ok(buf[..n].to_vec())
}

// ── the relay server ──

/// A running relay. Dropping it stops accepting; open sockets close
/// within about a second.
pub struct Relay {
    addr: SocketAddr,
    shared: Arc<Shared>,
    acceptor: Option<JoinHandle<()>>,
}

impl Relay {
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Block until the relay stops (it does not stop by itself).
    pub fn join(mut self) {
        if let Some(t) = self.acceptor.take() {
            let _ = t.join();
        }
    }

    /// A handle that stops the relay from another thread.
    pub fn stopper(&self) -> impl Fn() + Send + Sync + 'static {
        let shared = self.shared.clone();
        move || shared.stop.store(true, Ordering::Release)
    }

    /// `id` has a live control connection.
    pub fn online(&self, id: &str) -> bool {
        self.shared
            .hub
            .lock()
            .unwrap()
            .ids
            .get(id)
            .is_some_and(|g| g.control.is_some())
    }

    /// Pairs (waiting or spliced) of `id`.
    pub fn pairs(&self, id: &str) -> usize {
        self.shared
            .hub
            .lock()
            .unwrap()
            .ids
            .get(id)
            .map_or(0, |g| g.pairs)
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(t) = self.acceptor.take() {
            let _ = t.join();
        }
    }
}

struct Shared {
    config: RelayConfig,
    stop: AtomicBool,
    hub: Mutex<Hub>,
    sockets: AtomicUsize,
    /// Open sockets per [`bucket`].
    per_address: Mutex<HashMap<IpAddr, usize>>,
    generation: AtomicU64,
}

/// What the per-address limits count by: an IPv4 address, or an IPv6
/// address's /64 (one subscriber's prefix).
fn bucket(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
    }
}

#[derive(Default)]
struct Hub {
    ids: HashMap<String, Gw>,
    /// `/connect` times per client address.
    connects: HashMap<IpAddr, VecDeque<Instant>>,
}

/// An accepted relay socket (deadline-bound until its upgrade is done).
type Ws = WebSocket<Timed>;

type Control = Arc<Mutex<Ws>>;

struct Gw {
    secret: [u8; RELAY_SECRET_LEN],
    /// The live control connection and its generation.
    control: Option<(u64, Control)>,
    /// Phones waiting for `/accept`, by conn id.
    pending: HashMap<String, SyncSender<Ws>>,
    /// Waiting plus spliced pairs.
    pairs: usize,
    /// When `control` last went away (eviction order once full).
    offline_since: Instant,
}

/// Start the relay on `listener`.
pub fn serve_relay(listener: TcpListener, config: RelayConfig) -> std::io::Result<Relay> {
    let addr = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let shared = Arc::new(Shared {
        config,
        stop: AtomicBool::new(false),
        hub: Mutex::new(Hub::default()),
        sockets: AtomicUsize::new(0),
        per_address: Mutex::new(HashMap::new()),
        generation: AtomicU64::new(1),
    });
    let acceptor = {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("relay-accept".into())
            .spawn(move || accept_loop(listener, shared))?
    };
    Ok(Relay {
        addr,
        shared,
        acceptor: Some(acceptor),
    })
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    while !shared.stop.load(Ordering::Acquire) {
        let (stream, peer) = match listener.accept() {
            Ok(s) => s,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            Err(e) => {
                log::warn!("relay: accept failed: {e}");
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        if shared.sockets.fetch_add(1, Ordering::AcqRel) >= shared.config.max_sockets {
            shared.sockets.fetch_sub(1, Ordering::AcqRel);
            continue;
        }
        let Some(slot) = AddressSlot::take(&shared, peer.ip()) else {
            shared.sockets.fetch_sub(1, Ordering::AcqRel);
            continue;
        };
        let spawned = std::thread::Builder::new()
            .name("relay-conn".into())
            .spawn(move || {
                handle(&slot.shared, stream, peer);
                slot.shared.sockets.fetch_sub(1, Ordering::AcqRel);
            });
        if let Err(e) = spawned {
            // the thread that would uncount it never ran (its address slot
            // was dropped with the closure)
            shared.sockets.fetch_sub(1, Ordering::AcqRel);
            log::warn!("relay: cannot start a thread: {e}");
        }
    }
}

/// One socket counted against its address (released on drop).
struct AddressSlot {
    shared: Arc<Shared>,
    /// `None` when addresses are not counted (`trust_forwarded`).
    bucket: Option<IpAddr>,
}

impl AddressSlot {
    fn take(shared: &Arc<Shared>, ip: IpAddr) -> Option<AddressSlot> {
        let bucket = (!shared.config.trust_forwarded).then(|| bucket(ip));
        if let Some(b) = bucket {
            let mut open = shared.per_address.lock().unwrap();
            let n = open.entry(b).or_default();
            if *n >= shared.config.max_per_address {
                return None;
            }
            *n += 1;
        }
        Some(AddressSlot {
            shared: shared.clone(),
            bucket,
        })
    }
}

impl Drop for AddressSlot {
    fn drop(&mut self) {
        if let Some(b) = self.bucket {
            let mut open = self.shared.per_address.lock().unwrap();
            if let Some(n) = open.get_mut(&b) {
                *n -= 1;
                if *n == 0 {
                    open.remove(&b);
                }
            }
        }
    }
}

/// What an upgrade request asked for.
enum Route {
    Gateway,
    Connect(Waiting),
    Accept(SyncSender<Ws>),
}

/// A phone's reserved pair: released (pending entry and count) on drop.
struct Waiting {
    shared: Arc<Shared>,
    id: String,
    conn: String,
    rx: Receiver<Ws>,
    control: Control,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let mut hub = self.shared.hub.lock().unwrap();
        if let Some(g) = hub.ids.get_mut(&self.id) {
            g.pending.remove(&self.conn);
            g.pairs = g.pairs.saturating_sub(1);
        }
    }
}

/// An HTTP status and its body for a refused upgrade.
type Refusal = (u16, &'static str);

fn reject((status, why): Refusal) -> ErrorResponse {
    let mut r = ErrorResponse::new(Some(why.to_string()));
    *r.status_mut() = tungstenite::http::StatusCode::from_u16(status)
        .unwrap_or(tungstenite::http::StatusCode::BAD_REQUEST);
    r
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

/// The client address for the `/connect` rate limit: the peer, or behind
/// a proxy (`trust_forwarded`) the last `X-Forwarded-For` hop, which the
/// proxy wrote (an earlier one, or a last one that is not an address, may
/// be the client's own text). Per /64 for IPv6.
fn client_ip(shared: &Shared, peer: SocketAddr, req: &Request) -> IpAddr {
    if shared.config.trust_forwarded
        && let Some(ip) = req
            .headers()
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .next_back()
            .and_then(|s| s.trim().parse::<IpAddr>().ok())
    {
        return bucket(ip);
    }
    bucket(peer.ip())
}

fn route(shared: &Arc<Shared>, peer: SocketAddr, req: &Request) -> Result<Route, Refusal> {
    let cfg = &shared.config;
    let query = req.uri().query().unwrap_or("");
    match req.uri().path() {
        "/gateway" => Ok(Route::Gateway),
        "/connect" => {
            let ip = client_ip(shared, peer, req);
            let now = Instant::now();
            let mut hub = shared.hub.lock().unwrap();
            hub.connects.retain(|_, log| {
                while log
                    .front()
                    .is_some_and(|t| now.duration_since(*t) >= cfg.connect_window)
                {
                    log.pop_front();
                }
                !log.is_empty()
            });
            let log = hub.connects.entry(ip).or_default();
            if log.len() >= cfg.connect_rate {
                return Err((429, "too many connections; try again later"));
            }
            log.push_back(now);
            let id = query_param(query, "id").filter(|id| valid_id(id));
            let Some(id) = id else {
                return Err((400, "bad routing id"));
            };
            let Some(g) = hub.ids.get_mut(id) else {
                return Err((404, "gateway offline"));
            };
            let Some((_, control)) = g.control.clone() else {
                return Err((404, "gateway offline"));
            };
            if g.pairs >= cfg.max_pairs {
                return Err((503, "too many connections to this gateway"));
            }
            let conn = loop {
                let bytes: [u8; CONN_ID_LEN] =
                    transport::random_bytes().map_err(|_| (500, "no RNG"))?;
                let c = B64.encode(bytes);
                if !g.pending.contains_key(&c) {
                    break c;
                }
            };
            let (tx, rx) = sync_channel(1);
            g.pending.insert(conn.clone(), tx);
            g.pairs += 1;
            Ok(Route::Connect(Waiting {
                shared: shared.clone(),
                id: id.to_string(),
                conn,
                rx,
                control,
            }))
        }
        "/accept" => {
            let secret = req
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| strip_prefix_ci(v, "bearer "));
            let (Some(id), Some(conn), Some(secret)) =
                (query_param(query, "id"), query_param(query, "conn"), secret)
            else {
                return Err((400, "id, conn and the secret are required"));
            };
            let secret = B64.decode(secret.trim().as_bytes()).unwrap_or_default();
            let mut hub = shared.hub.lock().unwrap();
            let Some(g) = hub.ids.get_mut(id) else {
                return Err((403, "refused"));
            };
            if !ct_eq(&secret, &g.secret) {
                return Err((403, "refused"));
            }
            g.pending
                .remove(conn)
                .map(Route::Accept)
                .ok_or((404, "no such connection"))
        }
        _ => Err((404, "not found")),
    }
}

// the callback's error type is tungstenite's (an HTTP response)
#[allow(clippy::result_large_err)]
fn handle(shared: &Arc<Shared>, stream: TcpStream, peer: SocketAddr) {
    let cfg = &shared.config;
    let deadline = Instant::now() + cfg.request_deadline;
    let _ = stream.set_nodelay(true);
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(cfg.request_deadline)).is_err()
        || stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .is_err()
    {
        return;
    }
    let stream = Timed::new(stream, deadline);
    let ws_config = WebSocketConfig::default()
        .max_frame_size(Some(cfg.max_frame))
        .max_message_size(Some(cfg.max_frame));
    let mut chosen = None;
    let ws = tungstenite::accept_hdr_with_config(
        stream,
        |req: &Request, resp: Response| {
            chosen = Some(route(shared, peer, req).map_err(reject)?);
            Ok(resp)
        },
        Some(ws_config),
    );
    let Ok(mut ws) = ws else {
        // a reservation in `chosen` is released here
        return;
    };
    match chosen {
        // the deadline holds through registration
        Some(Route::Gateway) => control(shared, ws, peer, deadline),
        Some(Route::Connect(w)) => {
            ws.get_mut().end_deadline();
            connect(shared, ws, w)
        }
        // the phone's thread splices it (or it timed out: dropped)
        Some(Route::Accept(tx)) => {
            ws.get_mut().end_deadline();
            let _ = tx.send(ws);
        }
        None => {}
    }
}

fn text(msg: &RelayMsg) -> Message {
    Message::text(serde_json::to_string(msg).unwrap_or_default())
}

/// A write that only queued its frame (non-blocking socket) counts as sent.
fn sent(r: tungstenite::Result<()>) -> bool {
    match r {
        Ok(()) => true,
        Err(tungstenite::Error::Io(e)) => e.kind() == ErrorKind::WouldBlock,
        Err(_) => false,
    }
}

fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

/// The next control message before `deadline` (`None`: none, or not one).
fn read_control(ws: &mut Ws, deadline: Instant) -> Option<RelayMsg> {
    loop {
        if Instant::now() >= deadline {
            return None;
        }
        match ws.read() {
            Ok(Message::Text(t)) => return serde_json::from_str(t.as_str()).ok(),
            Ok(Message::Ping(_) | Message::Pong(_)) => {}
            Err(e) if would_block(&e) => {}
            _ => return None,
        }
    }
}

/// Read `register`, check that its id is its key's, and have the gateway
/// prove it holds the key: (id, secret).
fn register(ws: &mut Ws, deadline: Instant) -> Option<(String, [u8; RELAY_SECRET_LEN])> {
    let Some(RelayMsg::Register { id, key, secret }) = read_control(ws, deadline) else {
        return None;
    };
    let key = decode_key(&key)?;
    let secret: [u8; RELAY_SECRET_LEN] = B64.decode(secret.as_bytes()).ok()?.try_into().ok()?;
    if !valid_id(&id) || routing_id(&key) != id {
        return None;
    }
    let (hs, m) = challenge(&key).ok()?;
    ws.send(text(&RelayMsg::Prove { m: B64.encode(m) })).ok()?;
    let Some(RelayMsg::Proof { m }) = read_control(ws, deadline) else {
        return None;
    };
    let answer = B64.decode(m.as_bytes()).ok()?;
    verify_proof(hs, &answer).then_some((id, secret))
}

/// A gateway's control connection: registration, then `open`s (sent by
/// the phones' threads), pings and the idle limit.
fn control(shared: &Arc<Shared>, mut ws: Ws, peer: SocketAddr, deadline: Instant) {
    let cfg = &shared.config;
    ws.set_config(|c| {
        c.max_frame_size = Some(MAX_CONTROL);
        c.max_message_size = Some(MAX_CONTROL);
    });
    let allowed = |id: &String| cfg.only_ids.as_ref().is_none_or(|ids| ids.contains(id));
    let Some((id, secret)) = register(&mut ws, deadline).filter(|(id, _)| allowed(id)) else {
        log::debug!("relay: {peer}: registration refused (no valid key proof, or not allowed)");
        let _ = ws.close(None);
        let _ = ws.flush();
        return;
    };
    ws.get_mut().end_deadline();
    let generation = shared.generation.fetch_add(1, Ordering::Relaxed);
    let control: Control = Arc::new(Mutex::new(ws));
    let refused = {
        let mut hub = shared.hub.lock().unwrap();
        if !hub.ids.contains_key(&id) && hub.ids.len() >= cfg.max_ids {
            // make room: the id offline the longest, if one has no pairs
            let oldest = hub
                .ids
                .iter()
                .filter(|(_, g)| g.control.is_none() && g.pairs == 0)
                .min_by_key(|(_, g)| g.offline_since)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                hub.ids.remove(&k);
            }
        }
        if let Some(g) = hub.ids.get_mut(&id) {
            // the key's holder (a reconnect, or a restart with a new
            // secret): the new connection wins
            g.secret = secret;
            g.control = Some((generation, control.clone()));
            None
        } else if hub.ids.len() >= cfg.max_ids {
            Some(RelayError::Full)
        } else {
            hub.ids.insert(
                id.clone(),
                Gw {
                    secret,
                    control: Some((generation, control.clone())),
                    pending: HashMap::new(),
                    pairs: 0,
                    offline_since: Instant::now(),
                },
            );
            None
        }
    };
    let mut ws = control.lock().unwrap();
    if let Some(code) = refused {
        log::info!("relay: {peer}: refused a registration ({code:?})");
        let _ = ws.send(text(&RelayMsg::Error { code }));
        let _ = ws.close(None);
        let _ = ws.flush();
        return;
    }
    if ws.send(text(&RelayMsg::Registered)).is_err() || ws.get_ref().set_nonblocking(true).is_err()
    {
        drop(ws);
        release_control(shared, &id, generation);
        return;
    }
    let fd = ws.get_ref().as_raw_fd();
    drop(ws);
    log::info!("relay: gateway {id} online");
    let mut heard = Instant::now();
    let mut pinged = Instant::now();
    'run: loop {
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        let current = shared
            .hub
            .lock()
            .unwrap()
            .ids
            .get(&id)
            .and_then(|g| g.control.as_ref().map(|c| c.0))
            == Some(generation);
        if !current {
            break;
        }
        {
            let mut ws = control.lock().unwrap();
            loop {
                match ws.read() {
                    Ok(Message::Close(_)) => break 'run,
                    // the gateway sends nothing else; a binary frame here
                    // is a protocol violation
                    Ok(Message::Binary(_)) => break 'run,
                    Ok(_) => heard = Instant::now(),
                    Err(e) if would_block(&e) => break,
                    Err(_) => break 'run,
                }
            }
            if pinged.elapsed() >= cfg.control_ping {
                pinged = Instant::now();
                if !sent(ws.send(Message::Ping(Vec::new().into()))) {
                    break;
                }
            } else if !sent(ws.flush()) {
                break;
            }
        }
        if heard.elapsed() >= cfg.control_idle {
            log::info!("relay: gateway {id} silent, dropped");
            break;
        }
        poll_fds(
            &mut [pollfd(fd, libc::POLLIN)],
            Duration::from_secs(1).min(cfg.control_ping),
        );
    }
    {
        let mut ws = control.lock().unwrap();
        let _ = ws.close(None);
        let _ = ws.flush();
    }
    release_control(shared, &id, generation);
    log::info!("relay: gateway {id} offline");
}

/// Forget `generation` as `id`'s control connection (the secret stays).
fn release_control(shared: &Shared, id: &str, generation: u64) {
    let mut hub = shared.hub.lock().unwrap();
    if let Some(g) = hub.ids.get_mut(id)
        && g.control.as_ref().is_some_and(|c| c.0 == generation)
    {
        g.control = None;
        g.offline_since = Instant::now();
    }
}

/// A phone: ask the gateway to accept, wait for it, splice.
fn connect(shared: &Arc<Shared>, mut phone: Ws, w: Waiting) {
    let open = text(&RelayMsg::Open {
        conn: w.conn.clone(),
    });
    if !sent(w.control.lock().unwrap().send(open)) {
        let _ = phone.close(None);
        let _ = phone.flush();
        return;
    }
    let gateway = match w.rx.recv_timeout(shared.config.accept_deadline) {
        Ok(g) => Some(g),
        Err(_) => {
            // accepted at the last moment?
            shared
                .hub
                .lock()
                .unwrap()
                .ids
                .get_mut(&w.id)
                .map(|g| g.pending.remove(&w.conn));
            w.rx.try_recv().ok()
        }
    };
    let Some(gateway) = gateway else {
        let _ = phone.close(Some(CloseFrame {
            code: CloseCode::Again,
            reason: "the gateway did not answer".into(),
        }));
        let _ = phone.flush();
        return;
    };
    let why = splice(shared, phone, gateway);
    log::debug!("relay: {} pair ended: {why}", w.id);
    drop(w);
}

fn pollfd(fd: i32, events: libc::c_short) -> libc::pollfd {
    libc::pollfd {
        fd,
        events,
        revents: 0,
    }
}

fn poll_fds(fds: &mut [libc::pollfd], timeout: Duration) {
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: `fds` is a valid array of pollfd for the duration of the call
    unsafe {
        libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms);
    }
}

/// Forward binary frames between `phone` and `gateway` until either
/// closes, sends a text frame, breaks a limit, or both stay silent for the
/// idle limit. Returns why it ended.
fn splice(shared: &Shared, phone: Ws, gateway: Ws) -> &'static str {
    let cfg = &shared.config;
    let mut ws = [phone, gateway];
    for w in &ws {
        if w.get_ref().set_nonblocking(true).is_err() {
            return "socket error";
        }
    }
    let fds = [ws[0].get_ref().as_raw_fd(), ws[1].get_ref().as_raw_fd()];
    let legs = [Leg::ToGateway, Leg::ToPhone];
    // side i has frames queued that its socket did not take yet
    let mut backlog = [false, false];
    let mut last = Instant::now();
    let why = 'run: loop {
        if shared.stop.load(Ordering::Acquire) {
            break "relay stopping";
        }
        for (src, leg) in legs.into_iter().enumerate() {
            let dst = 1 - src;
            // back-pressure: do not read what the other side cannot take
            while !backlog[dst] {
                let (a, b) = ws.split_at_mut(1);
                let (from, to) = if src == 0 {
                    (&mut a[0], &mut b[0])
                } else {
                    (&mut b[0], &mut a[0])
                };
                match from.read() {
                    Ok(Message::Binary(frame)) => {
                        last = Instant::now();
                        if let Some(tap) = &cfg.tap {
                            tap(leg, &frame);
                        }
                        match to.send(Message::Binary(frame)) {
                            Ok(()) => {}
                            Err(e) if would_block(&e) => backlog[dst] = true,
                            Err(_) => break 'run "write failed",
                        }
                    }
                    Ok(Message::Text(_)) => break 'run "text frame",
                    Ok(Message::Close(_)) => break 'run "closed",
                    Ok(_) => {}
                    Err(e) if would_block(&e) => break,
                    Err(tungstenite::Error::Capacity(_)) => break 'run "frame too large",
                    Err(_) => break 'run "closed",
                }
            }
        }
        for (i, w) in ws.iter_mut().enumerate() {
            match w.flush() {
                Ok(()) => backlog[i] = false,
                Err(e) if would_block(&e) => backlog[i] = true,
                Err(_) => break 'run "write failed",
            }
        }
        let idle_left = cfg.pair_idle.saturating_sub(last.elapsed());
        if idle_left.is_zero() {
            break "idle";
        }
        let mut pfds = [0, 1].map(|i| {
            let mut ev = 0;
            if !backlog[1 - i] {
                ev |= libc::POLLIN;
            }
            if backlog[i] {
                ev |= libc::POLLOUT;
            }
            pollfd(fds[i], ev)
        });
        poll_fds(&mut pfds, idle_left.min(Duration::from_secs(1)));
    };
    for w in &mut ws {
        let _ = w.get_ref().set_nonblocking(false);
        let _ = w
            .get_ref()
            .set_write_timeout(Some(Duration::from_millis(500)));
        let _ = w.close(None);
        let _ = w.flush();
    }
    why
}

// ── the command line (`rift-relay`, `rift-terminal relay`) ──

pub const HELP: &str = "usage: rift-relay [--listen ADDR] [--trust-forwarded]
       rift-terminal relay [--listen ADDR] [--trust-forwarded]

Splice phones to Rift remote gateways outside the local network. The relay
forwards Noise-encrypted frames unchanged and never holds a key. It serves
plain ws:// on ADDR (default 0.0.0.0:8790): put a TLS reverse proxy in front
and give gateways (`remote_relay`) and phones its wss:// URL.

  --trust-forwarded  rate-limit by the last X-Forwarded-For address and
                     do not limit sockets per address (only behind your
                     own reverse proxy)";

/// Run the relay from command-line `args` (after the program or
/// subcommand name) until it is killed. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    let mut listen = DEFAULT_LISTEN.to_string();
    let mut config = RelayConfig::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--listen" => match it.next() {
                Some(v) => listen = v.clone(),
                None => {
                    eprintln!("{HELP}");
                    return 2;
                }
            },
            "--trust-forwarded" => config.trust_forwarded = true,
            "help" | "--help" | "-h" => {
                println!("{HELP}");
                return 0;
            }
            other => {
                eprintln!("relay: unknown argument {other:?}\n{HELP}");
                return 2;
            }
        }
    }
    let addr: SocketAddr = match listen.parse() {
        Ok(a) => a,
        Err(_) => {
            eprintln!("relay: listen address {listen:?}: expected IP:PORT");
            return 2;
        }
    };
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("relay: {addr}: {e}");
            return 1;
        }
    };
    let relay = match serve_relay(listener, config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("relay: {e}");
            return 1;
        }
    };
    println!(
        "Rift relay on ws://{} (put a TLS reverse proxy in front)",
        relay.local_addr()
    );
    relay.join();
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_urls_are_ws_or_wss() {
        let u = RelayUrl::parse("wss://relay.example.com").unwrap();
        assert_eq!(
            (u.tls, u.host.as_str(), u.port, u.prefix.as_str()),
            (true, "relay.example.com", 443, "")
        );
        assert_eq!(
            u.endpoint("/connect?id=x"),
            "wss://relay.example.com:443/connect?id=x"
        );
        let u = RelayUrl::parse(" ws://127.0.0.1:18790/rift/ ").unwrap();
        assert_eq!(
            (u.tls, u.port, u.prefix.as_str(), u.authority().as_str()),
            (false, 18790, "/rift", "127.0.0.1:18790")
        );
        assert_eq!(u.endpoint("/gateway"), "ws://127.0.0.1:18790/rift/gateway");
        let u = RelayUrl::parse("WSS://[::1]:9/").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("::1", 9));
        assert_eq!(u.authority(), "[::1]:9");
        assert_eq!(RelayUrl::parse("ws://h").unwrap().port, 80);
        for bad in [
            "",
            "relay.example.com",
            "http://relay.example.com",
            "https://relay.example.com",
            "wss://",
            "wss://:443",
            "wss://h:0",
            "wss://h:99999",
            "wss://h:x",
            "wss://user@h",
            "wss://h/?q=1",
            "wss://h/#x",
            "wss://h x",
            "wss://[::1",
            "wss://[::1]x",
        ] {
            assert!(RelayUrl::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_dialed_socket_writes_with_the_session_timeout_not_the_dial_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let _ws = tungstenite::accept(s).unwrap();
        });
        let url = RelayUrl::parse(&format!("ws://{addr}")).unwrap();
        let ws = dial(
            &url,
            "/accept",
            None,
            WebSocketConfig::default(),
            Duration::from_millis(100),
        )
        .unwrap();
        let MaybeTlsStream::Plain(s) = ws.get_ref() else {
            panic!("a ws:// relay is a plain socket");
        };
        assert_eq!(s.read_timeout().unwrap(), Some(Duration::from_millis(100)));
        assert_eq!(
            s.write_timeout().unwrap(),
            Some(crate::server::WRITE_TIMEOUT)
        );
        server.join().unwrap();
    }

    #[test]
    fn routing_ids_are_16_bytes_of_base64url() {
        assert!(valid_id("AAECAwQFBgcICQoLDA0ODw"));
        assert!(!valid_id("AAECAwQFBgcICQoLDA0O"));
        assert!(!valid_id("AAECAwQFBgcICQoLDA0ODw=="));
        assert!(!valid_id("AAECAwQFBgcICQoLDA0OD+"));
        assert!(!valid_id(""));
        assert_eq!(query_param("id=a&conn=b&idx=c", "id"), Some("a"));
        assert_eq!(query_param("idx=c", "id"), None);
    }
}
