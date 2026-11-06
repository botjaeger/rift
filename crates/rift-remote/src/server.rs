//! The gateway: one thread per connection, one poller thread.
//!
//! Every connection runs the Noise handshake ([`crate::transport`]), then
//! authenticates the client's static key against the paired devices (or
//! pairs it with a live secret) in [`crate::registry`].
//!
//! The poller reads the blocks of every subscribed pane from the
//! [`Source`], diffs them against what it sent last (serialized form per
//! block), and broadcasts each change as an `update` with the pane's next
//! `seq`. The last [`REPLAY_RING`] updates per pane are kept so a client
//! that reconnects with `subscribe{after}` gets exactly what it missed. It
//! also re-reads `devices.json` (closing revoked devices' connections),
//! sends the topology when it changes, and detects finished commands for
//! `done` ([`DoneTracker`]) and for the devices' alerts while their app is
//! closed ([`crate::push`]; panes are watched then even with nobody
//! connected). For connections that set a `screen` mode it also reads the
//! subscribed pane's screen and queues each its own diff
//! ([`refresh_screen`]).
//!
//! A connection is the same whether its socket was accepted on the
//! listening address or dialed at a relay ([`crate::uplink`]): both go
//! through [`connection`] with every limit. Each one that ends (or fails
//! its handshake) is appended to `history.jsonl` with one of these
//! reasons: `closed` (the peer closed), `error` (the socket failed),
//! `idle`, `too_slow`, `too_large`, `bad_request`, `revoked`,
//! `permission` (its permission changed), `forgotten`, `stopped` (the
//! gateway stopped); failed handshakes: `handshake`, `version`, `auth`,
//! `busy`.

use crate::protocol::{
    Block, BlockState, ClientHello, ClientMsg, CloseKind, ClosingReason, ErrorCode, GatewayHello,
    MoveKind, OpenKind, Permission, Run, ScreenMode, ServerMsg, Theme, VERSION,
};
use crate::registry::{self, Device, RelayState, RelayStatus, Via};
use crate::transport::{self, Channel, KEY_LEN, Recv, Socket};
use crate::{
    FAILURE_WINDOW, FIT_COLS, FIT_RATE, FIT_ROWS, HANDSHAKE_DEADLINE, IDLE_TIMEOUT, Input,
    InputError, KEYS_RATE, Key, MAX_BLOCKS, MAX_CLIENT_MESSAGE, MAX_COMMAND, MAX_CONNECTIONS,
    MAX_DIFF_PATH, MAX_FAILURES, MAX_GATEWAY_MESSAGE, MAX_KEYS, MAX_OUTPUT_PAGE_LINES, MAX_PASTE,
    MAX_REQUEST_ID, MAX_SCREEN_COLS, MAX_SCREEN_ROWS, MAX_TEXT, MAX_TRACKED, Menu, MenuKind,
    OPEN_RATE, OUTPUT_RATE, OutputError, PANE_POLL, PING_INTERVAL, Page, REFUSAL, REPLAY_RING,
    RUN_RATE, Screen, Source, SourceError, TOPOLOGY_POLL,
};
use crate::{
    ImageKind, MAX_SCROLL_LINES, MAX_UPLOAD_BYTES, MAX_UPLOAD_CHUNK, MAX_UPLOAD_EXT,
    MAX_UPLOAD_MESSAGE, MAX_UPLOADS, UPLOAD_RATE, valid_upload_ext,
};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{HandshakeError, Message, WebSocket};

/// How long one socket write may block on a peer that stopped reading,
/// before the connection counts as failed (direct and relay sockets alike).
pub(crate) const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long one socket read waits before the client loop checks its queue.
pub(crate) const READ_SLICE: Duration = Duration::from_millis(25);
/// The read's wait while the client's pane was just scrolled or typed into:
/// its next screen is ready within a few ms, and waited out the rest of a
/// [`READ_SLICE`] before it went (measured: ready 3 ms after a scroll, sent 27 ms after).
const HOT_READ_SLICE: Duration = Duration::from_millis(3);
/// Frames queued per client before it counts as too slow and is dropped
/// (it resumes with `subscribe{after}`): a full replay plus headroom.
const CLIENT_QUEUE: usize = REPLAY_RING + 64;
/// Open sockets past which a new one is dropped without a handshake (the
/// ones between [`MAX_CONNECTIONS`] and this get `busy`).
pub(crate) const MAX_SOCKETS: usize = 2 * MAX_CONNECTIONS;
/// Blocks read per pane by the `done` probe of panes nobody follows: a
/// burst of up to this many commands between two probes is reported whole.
const DONE_PROBE: usize = 4;
/// Running block ids remembered per pane by the `done` tracker.
const DONE_RUNNING: usize = 64;
/// A client counts as following its pane only when heard from this
/// recently: the app pings every 15 s (`PING_INTERVAL_MS` in
/// `mobile/src/protocol/client.ts`), so a live one is never older than that
/// plus a round trip; a dropped connection stops counting after this, not
/// after the 45 s idle timeout.
pub const FOLLOW_FRESH: Duration = Duration::from_secs(20);
/// What this gateway serves beyond Phase 2 (`welcome.features`).
const FEATURES: [&str; 23] = [
    "screen",
    "text",
    "output",
    "push_failures",
    "push_ntfy",
    "git",
    "notify_off",
    "chips",
    "waiting",
    "complete",
    "running",
    "open",
    "close",
    "move",
    "fit",
    "prompt_chips",
    "closing",
    "scroll",
    "upload",
    "upload_files",
    "paste",
    "mouse",
    "clipboard",
];

/// Gateway settings. [`Config::new`] takes the limits from the protocol;
/// tests shorten the timers.
#[derive(Debug, Clone)]
pub struct Config {
    /// The registry directory (`<data dir>/remote`, see
    /// [`registry::remote_dir`]).
    pub dir: PathBuf,
    /// Handshake hello / `welcome` fields.
    pub host: String,
    pub os: String,
    pub version: String,
    /// Addresses written to `status.json` (what `remote pair` links list);
    /// `None`: [`registry::addresses`] of the bound address.
    pub addresses: Option<Vec<SocketAddr>>,
    pub handshake_deadline: Duration,
    pub ping_interval: Duration,
    pub idle_timeout: Duration,
    pub pane_poll: Duration,
    /// How long a client loop waits on its socket before it flushes what is
    /// pending (25 ms; a test widens it to tell the shortened wait of
    /// a kicked pane from it under load).
    pub read_slice: Duration,
    pub topology_poll: Duration,
    pub devices_poll: Duration,
    pub failure_window: Duration,
    pub refusal: Duration,
    /// How long one `upload` may take from its first chunk to its last
    /// ([`crate::UPLOAD_DEADLINE`]).
    pub upload_deadline: Duration,
    /// The relay to register at (`remote_relay`; `ws://` or `wss://`), or
    /// none. [`Gateway::set_relay`] changes it while running.
    pub relay: Option<String>,
    /// Reconnect delays of the relay connection: from the first, doubling
    /// up to the last (with jitter).
    pub relay_backoff: (Duration, Duration),
    /// Where alerts are posted ([`crate::push::EXPO_PUSH_URL`]; tests use a
    /// loopback `http://` server).
    pub push_url: String,
    /// Where ntfy alerts are posted instead of each device's topic URL
    /// (tests: a URL starting with `.0` goes to `.1` and the rest); empty in
    /// the app.
    pub ntfy_redirect: Vec<(String, String)>,
    /// A client counts as following its pane (no ntfy banner about it)
    /// only when heard from this recently (20 s by default): a dropped
    /// connection stays open until `idle_timeout`.
    pub follow_fresh: Duration,
    /// The `seq` a pane's feed starts from. `None` (the app): the gateway's
    /// start time in milliseconds, so a `subscribe{after}` from an earlier
    /// run of the gateway never lines up with this run's replay ring and
    /// gets a fresh `snapshot` (tests pin 0 for readable sequences).
    pub seq_base: Option<u64>,
}

impl Config {
    pub fn new(dir: PathBuf, host: String, os: String, version: String) -> Config {
        Config {
            dir,
            host,
            os,
            version,
            addresses: None,
            handshake_deadline: HANDSHAKE_DEADLINE,
            ping_interval: PING_INTERVAL,
            idle_timeout: IDLE_TIMEOUT,
            pane_poll: PANE_POLL,
            read_slice: READ_SLICE,
            topology_poll: TOPOLOGY_POLL,
            devices_poll: crate::DEVICES_POLL,
            failure_window: FAILURE_WINDOW,
            refusal: REFUSAL,
            upload_deadline: crate::UPLOAD_DEADLINE,
            relay: None,
            relay_backoff: (crate::RELAY_BACKOFF_MIN, crate::RELAY_BACKOFF_MAX),
            push_url: crate::push::EXPO_PUSH_URL.to_string(),
            ntfy_redirect: Vec::new(),
            follow_fresh: FOLLOW_FRESH,
            seq_base: None,
        }
    }
}

/// A running gateway. Dropping it stops accepting and polling, removes
/// `status.json` and releases `gateway.lock`; client threads end on their
/// next loop turn.
pub struct Gateway {
    addr: SocketAddr,
    shared: Arc<dyn Stop>,
    threads: Vec<JoinHandle<()>>,
    _lock: std::fs::File,
}

impl Gateway {
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Block until the gateway stops (see [`Gateway::stopper`]).
    pub fn join(mut self) {
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }

    /// A handle that stops the gateway from another thread (`join`
    /// returns, status and lock are released when it drops).
    pub fn stopper(&self) -> impl Fn() + Send + Sync + 'static {
        let shared = self.shared.clone();
        move || shared.stop()
    }

    /// Register at `relay` from now on (`None`: no relay). A change closes
    /// the current relay connection; phones spliced through it stay.
    pub fn set_relay(&self, relay: Option<String>) {
        self.shared.set_relay(relay);
    }

    /// Tell every connected device the gateway is about to stop
    /// (`closing`): `Quit` for good, `Update` back in about `back_in_s`
    /// seconds. Returns at once; each connection writes it on its next loop
    /// turn, after what was queued before it and even when its queue is
    /// full. A device that connects afterwards gets it right after its
    /// `topology`. [`Gateway::flush`] waits until it is written.
    pub fn announce_closing(&self, reason: ClosingReason, back_in_s: Option<u32>) {
        self.shared
            .announce_closing(frame(&ServerMsg::Closing { reason, back_in_s }));
    }

    /// Wait up to `timeout` until every connection has written the
    /// `closing` of [`Gateway::announce_closing`] (or ended). True when none
    /// is left to write.
    pub fn flush(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.shared.closing_unsent() == 0 {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.shared.stop();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        self.shared.finish();
    }
}

/// The type-erased part of [`Shared`] the [`Gateway`] handle needs.
trait Stop: Send + Sync {
    fn stop(&self);
    /// After the threads ended: remove `status.json`.
    fn finish(&self);
    fn set_relay(&self, relay: Option<String>);
    /// Owe every connection (and every later one) `frame`.
    fn announce_closing(&self, frame: Arc<str>);
    /// Connections that still owe their `closing`.
    fn closing_unsent(&self) -> usize;
}

impl<S: Source> Stop for Shared<S> {
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    fn finish(&self) {
        let _g = self.status_lock.lock().unwrap();
        registry::remove_status(&self.config.dir);
    }

    fn announce_closing(&self, frame: Arc<str>) {
        *self.closing.lock().unwrap() = Some(frame);
        for c in self.hub.lock().unwrap().clients.values() {
            c.closing.store(true, Ordering::Release);
        }
    }

    fn closing_unsent(&self) -> usize {
        self.hub
            .lock()
            .unwrap()
            .clients
            .values()
            .filter(|c| c.closing.load(Ordering::Acquire))
            .count()
    }

    fn set_relay(&self, relay: Option<String>) {
        let relay = relay
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty());
        {
            let mut link = self.relay.lock().unwrap();
            if link.url == relay {
                return;
            }
            link.generation += 1;
            link.status = relay.as_ref().map(|url| RelayStatus {
                url: url.clone(),
                state: RelayState::Connecting,
                error: None,
                latency_ms: None,
                retry_at_ms: None,
            });
            link.url = relay;
        }
        write_status(self);
    }
}

/// The gateway's relay setting and the state of its connection.
#[derive(Default)]
pub(crate) struct RelayLink {
    pub url: Option<String>,
    pub status: Option<RelayStatus>,
    /// Bumped on every change of `url`: the connection to an old URL ends.
    pub generation: u64,
}

/// Start serving `listener` from `source`. Fails when another gateway holds
/// `gateway.lock` (`ErrorKind::AddrInUse`), or the registry cannot be read.
pub fn serve<S: Source>(
    listener: TcpListener,
    source: S,
    mut config: Config,
) -> std::io::Result<Gateway> {
    let addr = listener.local_addr()?;
    let lock = registry::lock_gateway(&config.dir)?.ok_or_else(|| {
        std::io::Error::new(
            ErrorKind::AddrInUse,
            "another Rift remote gateway is running for this user",
        )
    })?;
    let key = registry::gateway_private_key(&config.dir)?;
    let devices = registry::devices(&config.dir).unwrap_or_else(|e| {
        log::warn!("remote: {e}; refusing every device until it is fixed");
        Vec::new()
    });
    let addresses = config
        .addresses
        .take()
        .unwrap_or_else(|| registry::addresses(addr));
    listener.set_nonblocking(true)?;
    let relay = config.relay.take();
    let pusher = crate::push::Pusher::start(
        config.push_url.clone(),
        config.ntfy_redirect.clone(),
        registry::encode_key(&transport::public_key(&key)),
        config.dir.clone(),
    )?;
    let seq_base = config.seq_base.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    });
    let shared = Arc::new(Shared {
        source,
        config,
        key,
        addresses,
        relay: Mutex::new(RelayLink::default()),
        stop: AtomicBool::new(false),
        hub: Mutex::new(Hub::default()),
        fit_lock: Mutex::new(()),
        poll_lock: Mutex::new(()),
        kicks: (Mutex::new(HashMap::new()), Condvar::new()),
        status_lock: Mutex::new(()),
        devices: Mutex::new(devices),
        permissions: Mutex::new(None),
        uploads: AtomicUsize::new(0),
        failures: Mutex::new(Failures::default()),
        done: Mutex::new(DoneTracker::default()),
        watches: Mutex::new(crate::push::Watches::default()),
        pusher,
        connections: AtomicUsize::new(0),
        relay_connections: AtomicUsize::new(0),
        next_client: AtomicU64::new(1),
        seq_base,
        closing: Mutex::new(None),
    });
    // writes status.json (with the relay, if any)
    shared.set_relay(relay);
    write_status(&shared);
    let uplink = {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("remote-relay".into())
            .spawn(move || crate::uplink::run(shared))?
    };
    let poller = {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("remote-poll".into())
            .spawn(move || poll_loop(&shared))?
    };
    let acceptor = {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("remote-accept".into())
            .spawn(move || accept_loop(listener, shared))?
    };
    Ok(Gateway {
        addr,
        shared,
        threads: vec![acceptor, poller, uplink],
        _lock: lock,
    })
}

pub(crate) struct Shared<S> {
    source: S,
    pub(crate) config: Config,
    /// The gateway's static private key.
    pub(crate) key: [u8; KEY_LEN],
    addresses: Vec<SocketAddr>,
    pub(crate) relay: Mutex<RelayLink>,
    pub(crate) stop: AtomicBool,
    hub: Mutex<Hub>,
    /// One [`sync_fits`] at a time, so the source hears them in order.
    fit_lock: Mutex<()>,
    /// One fetch-and-diff at a time, so updates apply in fetch order.
    poll_lock: Mutex<()>,
    /// Panes a device just scrolled or typed into, until when their screen
    /// is read every [`crate::KICK_POLL`]; the condvar wakes the poll loop.
    kicks: (Mutex<HashMap<u64, Instant>>, Condvar),
    /// Orders `status.json` writes (and the final removal).
    status_lock: Mutex<()>,
    /// The paired devices as last read.
    devices: Mutex<Vec<Device>>,
    /// `devices.json` as [`operates`] last read it, by its stamp: re-read
    /// only when the file changed, so a scroll does not parse it each time.
    permissions: Mutex<Option<(registry::DevicesStamp, Vec<Device>)>>,
    /// Images being received across every connection ([`MAX_UPLOADS`]).
    uploads: AtomicUsize,
    failures: Mutex<Failures>,
    done: Mutex<DoneTracker>,
    /// The panes devices asked about (`notify`).
    watches: Mutex<crate::push::Watches>,
    pusher: crate::push::Pusher,
    /// Open sockets, authenticated or not.
    pub(crate) connections: AtomicUsize,
    /// The same for sockets dialed at the relay (a separate budget: whoever
    /// knows the routing id can make the relay send `open`s).
    pub(crate) relay_connections: AtomicUsize,
    next_client: AtomicU64,
    /// Where every pane's feed `seq` starts in this run ([`Config::seq_base`]).
    seq_base: u64,
    /// The `closing` frame once announced ([`Gateway::announce_closing`]).
    closing: Mutex<Option<Arc<str>>>,
}

#[derive(Default)]
struct Hub {
    /// Authenticated clients.
    clients: HashMap<u64, Slot>,
    feeds: HashMap<u64, Feed>,
    /// Last topology frame sent.
    topology: Option<Arc<str>>,
    /// The size last handed to the source per pane ([`Source::fit`]).
    fits: HashMap<u64, (u16, u16)>,
}

impl Hub {
    /// Drop the feeds (and their replay rings) of panes that are gone:
    /// neither in the topology (`all`) nor followed by a client (`followed`).
    /// Only a followed pane is refreshed, and only that read notices a pane
    /// closing, so without this every pane a phone ever opened kept its
    /// feed for the gateway's life. A followed pane stays even when a window
    /// missed one topology read: its feed's `seq` goes on.
    fn forget_feeds(&mut self, all: &HashSet<u64>, followed: &HashSet<u64>) {
        self.feeds
            .retain(|p, _| all.contains(p) || followed.contains(p));
    }
}

/// Why a client's connection must end ([`Slot::end`]).
const LIVE: u8 = 0;
const TOO_SLOW: u8 = 1;
const REVOKED: u8 = 2;
/// Its device's permission changed: it reconnects with the new one.
const PERMISSION: u8 = 3;
/// Its device connected again with every slot taken ([`Shared::displace`]).
const DISPLACED: u8 = 4;

/// When a client was last heard from, readable from other threads.
struct Heard {
    opened: Instant,
    /// Milliseconds after `opened`.
    at_ms: AtomicU64,
}

impl Heard {
    fn new() -> Heard {
        Heard {
            opened: Instant::now(),
            at_ms: AtomicU64::new(0),
        }
    }

    fn mark(&self) {
        let ms = u64::try_from(self.opened.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.at_ms.store(ms, Ordering::Release);
    }

    /// Heard from within `fresh`.
    fn within(&self, fresh: Duration) -> bool {
        let at = self.opened + Duration::from_millis(self.at_ms.load(Ordering::Acquire));
        at.elapsed() < fresh
    }
}

/// A client's `fit`: the pane and the size (already clamped).
#[derive(Clone, Copy)]
struct Fit {
    pane: u64,
    cols: u16,
    rows: u16,
}

struct Slot {
    tx: SyncSender<Arc<str>>,
    pane: Option<u64>,
    /// The pane its last `subscribe` was refused (`no_pane`): the `screen`
    /// the app sends right after it is dropped, not refused again.
    refused: Option<u64>,
    /// The app is on screen (`visible`; true until it says otherwise).
    visible: bool,
    /// What it wants of `pane`'s screen (`off` after every (un)subscribe).
    screen: ScreenMode,
    /// Its screen frames, written by its own loop ([`flush_screen`]).
    out: ScreenOut,
    end: Arc<AtomicU8>,
    /// It owes the announced `closing` (written by its own loop, outside
    /// its queue).
    closing: Arc<AtomicBool>,
    /// When its app was last heard from (any Noise message).
    seen: Arc<Heard>,
    /// The device's static key and registry entry.
    key: [u8; KEY_LEN],
    device: String,
    name: String,
    /// The pane size it asked for (`fit`); counts while it follows that pane.
    fit: Option<Fit>,
    /// The permission it connected with.
    permission: Permission,
    since: SystemTime,
    via: Via,
}

impl Slot {
    /// Follow `pane` (or nothing): screens stop silently, mode `off`.
    fn follow(&mut self, pane: Option<u64>) {
        self.pane = pane;
        self.refused = None;
        self.screen = ScreenMode::Off;
        self.out.restart(false);
    }

    fn push(&self, frame: Arc<str>) {
        if let Err(TrySendError::Full(_)) = self.tx.try_send(frame) {
            // too slow: drop it; it resumes from its last seq
            let _ = self
                .end
                .compare_exchange(LIVE, TOO_SLOW, Ordering::AcqRel, Ordering::Acquire);
        }
    }
}

/// A connection's screen frames. The poll loop leaves the newest screen in
/// `pending` ([`ScreenOut::offer`]); the connection's own loop diffs it
/// against what it actually wrote when it writes it ([`flush_screen`]). A
/// client slower than the screen changes gets fewer, later frames, never a
/// queue of them: at most one screen waits per connection.
#[derive(Default)]
struct ScreenOut {
    /// The screen the client has: what was written (unstyled when it went
    /// out so). The next frame is a diff against it; `None`: none since the
    /// mode was set, screens stopped, or a frame was dropped (the next is
    /// full).
    shown: Option<Screen>,
    /// The source's screen `shown` was made from.
    basis: Option<Screen>,
    /// The newest screen not written yet (a newer one replaces it).
    pending: Option<Screen>,
    /// The client shows a screen, or one is on its way: screens stopping
    /// owe it a `screen_end`.
    showing: bool,
    /// A `screen_end` is due.
    end: bool,
    /// Bumped whenever it starts over: a write begun before does not set
    /// `shown`.
    epoch: u64,
}

impl ScreenOut {
    /// Start over: the next screen goes out full. `showing`: the client
    /// still shows the last one (the mode changed, the screen did not end).
    fn restart(&mut self, showing: bool) {
        *self = ScreenOut {
            showing,
            epoch: self.epoch + 1,
            ..ScreenOut::default()
        };
    }

    /// The source's screen for this connection now (`None`: none to show
    /// in its mode).
    fn offer(&mut self, s: Option<&Screen>) {
        match s {
            Some(s) => {
                self.end = false;
                self.showing = true;
                if self.pending.is_some() || self.basis.as_ref() != Some(s) {
                    self.pending = Some(s.clone());
                }
            }
            None if self.showing => {
                self.showing = false;
                self.pending = None;
                self.end = true;
            }
            None => {}
        }
    }
}

/// Failed handshakes per remote address.
#[derive(Default)]
struct Failures {
    by_ip: HashMap<IpAddr, (VecDeque<Instant>, Option<Instant>)>,
}

impl Failures {
    /// `ip` is refused now.
    fn refused(&mut self, ip: IpAddr, now: Instant) -> bool {
        self.by_ip
            .get(&ip)
            .and_then(|(_, until)| *until)
            .is_some_and(|until| now < until)
    }

    /// Count one failure of `ip`; the [`MAX_FAILURES`]th within `window`
    /// refuses it for `refusal`.
    fn fail(&mut self, ip: IpAddr, now: Instant, window: Duration, refusal: Duration) {
        // forget addresses that are quiet and not refused
        self.by_ip.retain(|_, (log, until)| {
            log.retain(|t| now.duration_since(*t) < window);
            !log.is_empty() || until.is_some_and(|u| now < u)
        });
        let (log, until) = self.by_ip.entry(ip).or_default();
        log.push_back(now);
        if log.len() >= MAX_FAILURES {
            log.clear();
            *until = Some(now + refusal);
            log::warn!("remote: {ip}: {MAX_FAILURES} failed handshakes, refused for {refusal:?}");
        }
    }
}

/// Requests per second of one kind on one connection.
struct Rate {
    limit: usize,
    hits: VecDeque<Instant>,
}

impl Rate {
    fn new(limit: usize) -> Rate {
        Rate {
            limit,
            hits: VecDeque::new(),
        }
    }

    fn allow(&mut self, now: Instant) -> bool {
        while self
            .hits
            .front()
            .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1))
        {
            self.hits.pop_front();
        }
        if self.hits.len() >= self.limit {
            return false;
        }
        self.hits.push_back(now);
        true
    }
}

/// Finds finished commands for `done`.
///
/// Every page of a pane's newest blocks the gateway reads (subscribed panes
/// every [`PANE_POLL`], the others through a [`DONE_PROBE`]-block probe every
/// [`TOPOLOGY_POLL`]) goes through [`DoneTracker::observe`]. A block is
/// reported once when it is `done` and either was seen `running` or is newer
/// than every block seen before in that pane. Block ids only grow, so a
/// command that started and finished between two reads (never seen
/// running) is still reported: its id is new and it is already done. The
/// first read of a pane only records where it stands (history is not
/// reported); restored blocks are never reported.
#[derive(Default)]
struct DoneTracker {
    panes: HashMap<u64, PaneDone>,
}

struct PaneDone {
    /// Highest block id seen (`None`: the pane had no block yet; ids start
    /// at 0, so 0 cannot stand for "none").
    newest: Option<u64>,
    /// Blocks last seen running.
    running: BTreeSet<u64>,
}

impl DoneTracker {
    fn observe(&mut self, pane: u64, blocks: &[Block]) -> Vec<ServerMsg> {
        let live = blocks.iter().filter(|b| !b.restored);
        let Some(st) = self.panes.get_mut(&pane) else {
            self.panes.insert(
                pane,
                PaneDone {
                    newest: blocks.iter().map(|b| b.id).max(),
                    running: live
                        .filter(|b| b.state == BlockState::Running)
                        .map(|b| b.id)
                        .collect(),
                },
            );
            return Vec::new();
        };
        let mut out = Vec::new();
        for b in live {
            match b.state {
                BlockState::Done => {
                    let was_running = st.running.remove(&b.id);
                    if was_running || st.newest.is_none_or(|n| b.id > n) {
                        out.push(ServerMsg::Done {
                            pane,
                            block: b.id,
                            command: b.command.clone(),
                            exit: b.exit,
                            duration_ms: b.duration_ms,
                        });
                    }
                }
                BlockState::Running => {
                    st.running.insert(b.id);
                }
            }
        }
        if let Some(max) = blocks.iter().map(|b| b.id).max() {
            st.newest = Some(st.newest.map_or(max, |n| n.max(max)));
        }
        while st.running.len() > DONE_RUNNING {
            st.running.pop_first();
        }
        out
    }

    /// Forget panes that are gone.
    fn retain(&mut self, panes: &HashSet<u64>) {
        self.panes.retain(|p, _| panes.contains(p));
    }
}

pub(crate) fn write_status<S: Source>(shared: &Shared<S>) {
    let _g = shared.status_lock.lock().unwrap();
    if shared.stop.load(Ordering::Acquire) {
        return;
    }
    let mut clients: Vec<registry::Client> = shared
        .hub
        .lock()
        .unwrap()
        .clients
        .values()
        .map(|c| registry::Client {
            device: c.device.clone(),
            name: c.name.clone(),
            since: c.since,
            via: c.via,
        })
        .collect();
    clients.sort_by_key(|c| c.since);
    let status = registry::Status {
        pid: std::process::id(),
        addresses: shared.addresses.clone(),
        clients,
        relay: shared.relay.lock().unwrap().status.clone(),
    };
    if let Err(e) = registry::write_status(&shared.config.dir, &status) {
        log::warn!("remote: cannot write status.json: {e}");
        return;
    }
    shared.source.status_changed();
}

/// Diff state of one pane.
#[derive(Default)]
struct Feed {
    seq: u64,
    blocks: Vec<Block>,
    has_older: bool,
    /// Serialized form of each block of the newest window.
    sent: HashMap<u64, String>,
    /// Every id a client may hold: the window and `older` pages, until
    /// reported removed (at most [`MAX_TRACKED`], lowest forgotten first).
    known: BTreeSet<u64>,
    /// (seq, frame) of the last updates.
    ring: VecDeque<(u64, Arc<str>)>,
}

fn frame(msg: &ServerMsg) -> Arc<str> {
    serde_json::to_string(msg).unwrap_or_default().into()
}

impl Feed {
    fn new(page: Page) -> Feed {
        let mut feed = Feed {
            sent: page.blocks.iter().map(|b| (b.id, diff_key(b))).collect(),
            has_older: page.has_older,
            ..Default::default()
        };
        feed.track(page.blocks.iter().map(|b| b.id));
        feed.blocks = page.blocks;
        feed
    }

    /// Remember ids a client received (window or `older` page).
    fn track(&mut self, ids: impl IntoIterator<Item = u64>) {
        self.known.extend(ids);
        while self.known.len() > MAX_TRACKED {
            self.known.pop_first();
        }
    }

    /// Diff against the last state; the updates to broadcast (each with the
    /// next seq, each within [`MAX_GATEWAY_MESSAGE`]).
    fn apply(&mut self, pane: u64, page: Page) -> Vec<Arc<str>> {
        let Page {
            blocks,
            has_older,
            oldest,
        } = page;
        let now: HashMap<u64, String> = blocks.iter().map(|b| (b.id, diff_key(b))).collect();
        let changed: Vec<&Block> = blocks
            .iter()
            .filter(|b| self.sent.get(&b.id) != now.get(&b.id))
            .collect();
        // gone: missing inside the window, or below the pane's oldest block;
        // a missing id between the two was only paged out of the window
        let floor = blocks.first().map(|b| b.id).filter(|_| has_older);
        let removed: Vec<u64> = self
            .known
            .iter()
            .filter(|&&id| {
                !now.contains_key(&id)
                    && (floor.is_none_or(|f| id > f) || oldest.is_none_or(|o| id < o))
            })
            .copied()
            .collect();
        for id in &removed {
            self.known.remove(id);
        }
        self.track(now.keys().copied());
        let mut frames = Vec::new();
        if !changed.is_empty() || !removed.is_empty() {
            for (chunk, removed) in chunk_blocks(pane, &changed, removed) {
                self.seq += 1;
                let f = frame(&ServerMsg::Update {
                    pane,
                    seq: self.seq,
                    blocks: chunk,
                    removed,
                });
                self.ring.push_back((self.seq, f.clone()));
                if self.ring.len() > REPLAY_RING {
                    self.ring.pop_front();
                }
                frames.push(f);
            }
        }
        self.blocks = blocks;
        self.has_older = has_older;
        self.sent = now;
        frames
    }

    /// Updates after `after`, when the ring still holds all of them.
    fn replay(&self, after: u64) -> Option<Vec<Arc<str>>> {
        if after > self.seq {
            return None;
        }
        if after == self.seq {
            return Some(Vec::new());
        }
        let first = self.ring.front()?.0;
        (first <= after + 1).then(|| {
            self.ring
                .iter()
                .filter(|(s, _)| *s > after)
                .map(|(_, f)| f.clone())
                .collect()
        })
    }

    fn snapshot(&self, pane: u64) -> Arc<str> {
        let (blocks, trimmed) = fit_newest(&self.blocks, |blocks| ServerMsg::Snapshot {
            pane,
            seq: self.seq,
            blocks,
            has_older: true,
        });
        frame(&ServerMsg::Snapshot {
            pane,
            seq: self.seq,
            blocks,
            has_older: self.has_older || trimmed,
        })
    }
}

fn block_json(b: &Block) -> String {
    serde_json::to_string(b).unwrap_or_default()
}

/// What makes a block "changed": its serialized form, except the elapsed
/// time of a running block. That ticks on every poll; diffing it would send
/// an update every 100 ms per running command (and push resumable history
/// out of the ring within 26 s). The client advances it locally; the next
/// real change carries the current value.
fn diff_key(b: &Block) -> String {
    if b.state == crate::protocol::BlockState::Running {
        block_json(&Block {
            duration_ms: 0,
            ..b.clone()
        })
    } else {
        block_json(b)
    }
}

/// Split `changed` into updates that each fit a gateway frame; `removed`
/// rides in the first.
fn chunk_blocks(pane: u64, changed: &[&Block], removed: Vec<u64>) -> Vec<(Vec<Block>, Vec<u64>)> {
    let base = |removed: &[u64]| {
        frame(&ServerMsg::Update {
            pane,
            seq: u64::MAX,
            blocks: Vec::new(),
            removed: removed.to_vec(),
        })
        .len()
    };
    let mut out: Vec<(Vec<Block>, Vec<u64>)> = Vec::new();
    let mut cur: Vec<Block> = Vec::new();
    let mut cur_removed = removed;
    let mut size = base(&cur_removed);
    for b in changed {
        let n = block_json(b).len() + 1;
        if !cur.is_empty() && size + n > MAX_GATEWAY_MESSAGE {
            out.push((std::mem::take(&mut cur), std::mem::take(&mut cur_removed)));
            size = base(&[]);
        }
        size += n;
        cur.push((*b).clone());
    }
    if !cur.is_empty() || !cur_removed.is_empty() {
        out.push((cur, cur_removed));
    }
    out
}

/// The newest of `blocks` whose message (built by `wrap`) fits a gateway
/// frame, and whether any were left out.
fn fit_newest(blocks: &[Block], wrap: impl Fn(Vec<Block>) -> ServerMsg) -> (Vec<Block>, bool) {
    let base = frame(&wrap(Vec::new())).len();
    let mut size = base;
    let mut keep = 0;
    for b in blocks.iter().rev() {
        let n = block_json(b).len() + 1;
        if size + n > MAX_GATEWAY_MESSAGE {
            break;
        }
        size += n;
        keep += 1;
    }
    let from = blocks.len() - keep;
    (blocks[from..].to_vec(), from > 0)
}

/// Count one more open socket: `None` past [`MAX_SOCKETS`] (drop it
/// unread), else whether it is past [`MAX_CONNECTIONS`] (`busy`) and the
/// guard that uncounts it.
pub(crate) fn admit(connections: &AtomicUsize) -> Option<(bool, ConnGuard<'_>)> {
    let open = connections.fetch_add(1, Ordering::AcqRel);
    let guard = ConnGuard(connections);
    (open < MAX_SOCKETS).then_some((open >= MAX_CONNECTIONS, guard))
}

impl<S> Shared<S> {
    /// With every slot taken, a device connecting again ends those of its
    /// connections gone silent for half the idle timeout (the app pings every
    /// 15 s): what a dropped network left behind, each otherwise taking a slot
    /// until the idle timeout. Never one still heard: a phone paired twice
    /// holds two live ones, and ending them brings both straight back. The
    /// key is proven by the handshake. False when it holds none: `busy`.
    fn displace(&self, key: &[u8; KEY_LEN]) -> bool {
        let silent = self.config.idle_timeout / 2;
        let hub = self.hub.lock().unwrap();
        let mut any = false;
        for c in hub
            .clients
            .values()
            .filter(|c| &c.key == key && !c.seen.within(silent))
        {
            let _ = c
                .end
                .compare_exchange(LIVE, DISPLACED, Ordering::AcqRel, Ordering::Acquire);
            any = true;
        }
        any
    }
}

fn accept_loop<S: Source>(listener: TcpListener, shared: Arc<Shared<S>>) {
    while !shared.stop.load(Ordering::Acquire) {
        let (stream, peer) = match listener.accept() {
            Ok(s) => s,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            Err(e) => {
                log::warn!("remote: accept failed: {e}");
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        let opened = Instant::now();
        if shared.failures.lock().unwrap().refused(peer.ip(), opened) {
            log::debug!("remote: {peer}: refused (failed handshakes)");
            continue;
        }
        let Some((busy, guard)) = admit(&shared.connections) else {
            continue;
        };
        // the client thread counts it from here
        std::mem::forget(guard);
        let thread_shared = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("remote-client".into())
            .spawn(move || {
                let shared = thread_shared;
                let _guard = ConnGuard(&shared.connections);
                client(&shared, stream, peer, opened, busy);
            });
        if let Err(e) = spawned {
            // the thread that would uncount it never ran
            shared.connections.fetch_sub(1, Ordering::AcqRel);
            log::warn!("remote: cannot start a client thread: {e}");
        }
    }
}

pub(crate) struct ConnGuard<'a>(&'a AtomicUsize);

impl Drop for ConnGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Whether client `id`'s pane was scrolled or typed into a moment ago (its
/// screen is read every [`crate::KICK_POLL`]): then its loop waits on the
/// socket [`HOT_READ_SLICE`] at most, so a frame ready meanwhile goes out at
/// once rather than after the rest of a [`READ_SLICE`]. A client showing
/// blocks (no screen subscribed) has no screen frame to hurry and keeps the
/// full slice.
fn pane_kicked<S>(shared: &Shared<S>, id: u64) -> bool {
    let Some(pane) = shared
        .hub
        .lock()
        .unwrap()
        .clients
        .get(&id)
        .filter(|c| c.screen != ScreenMode::Off)
        .and_then(|c| c.pane)
    else {
        return false;
    };
    shared
        .kicks
        .0
        .lock()
        .unwrap()
        .get(&pane)
        .is_some_and(|until| *until > Instant::now())
}

/// A device scrolled or typed into `pane`: read its screen every
/// [`crate::KICK_POLL`] for a while, starting now.
fn kick<S>(shared: &Shared<S>, pane: u64) {
    let (kicks, wake) = &shared.kicks;
    kicks
        .lock()
        .unwrap()
        .insert(pane, Instant::now() + crate::KICK_WINDOW);
    wake.notify_one();
}

/// Wait until the next full poll is due (`next`), refreshing the screens of
/// kicked panes every [`crate::KICK_POLL`] meanwhile.
fn wait_polling_kicks<S: Source>(shared: &Shared<S>, next: Instant) {
    let (kicks, wake) = &shared.kicks;
    loop {
        if shared.stop.load(Ordering::Acquire) {
            return;
        }
        let now = Instant::now();
        let hot: Vec<u64> = {
            let mut k = kicks.lock().unwrap();
            k.retain(|_, until| *until > now);
            k.keys().copied().collect()
        };
        if !hot.is_empty() {
            let screens = {
                let hub = shared.hub.lock().unwrap();
                let mut screens: HashMap<u64, bool> = HashMap::new();
                for c in hub.clients.values().filter(|c| c.screen != ScreenMode::Off) {
                    if let Some(p) = c.pane.filter(|p| hot.contains(p)) {
                        *screens.entry(p).or_default() |= c.screen == ScreenMode::On;
                    }
                }
                screens
            };
            for (pane, force) in screens {
                refresh_screen(shared, pane, force);
            }
        }
        let now = Instant::now();
        if now >= next {
            return;
        }
        let until = if hot.is_empty() {
            next
        } else {
            (now + crate::KICK_POLL).min(next)
        };
        let k = kicks.lock().unwrap();
        // a kick that came meanwhile is served at once
        if hot.is_empty() && !k.is_empty() {
            continue;
        }
        let _ = wake.wait_timeout(k, until.saturating_duration_since(now));
    }
}

fn poll_loop<S: Source>(shared: &Shared<S>) {
    let cfg = &shared.config;
    let mut last_topology: Option<Instant> = None;
    let mut last_devices = Instant::now();
    let mut copies = None;
    let mut last_copy = Instant::now();
    let mut theme = None;
    let mut last_theme = Instant::now();
    while !shared.stop.load(Ordering::Acquire) {
        wait_polling_kicks(shared, Instant::now() + cfg.pane_poll.min(cfg.devices_poll));
        if last_copy.elapsed() >= crate::COPY_POLL {
            last_copy = Instant::now();
            forward_copy(shared, &mut copies);
        }
        if last_theme.elapsed() >= crate::THEME_POLL {
            last_theme = Instant::now();
            forward_theme(shared, &mut theme);
        }
        if last_devices.elapsed() >= cfg.devices_poll {
            last_devices = Instant::now();
            reload_devices(shared);
        }
        let (panes, screens, connected) = {
            let hub = shared.hub.lock().unwrap();
            let panes: HashSet<u64> = hub.clients.values().filter_map(|c| c.pane).collect();
            // panes whose screen someone wants: forced when one wants `on`
            let mut screens: HashMap<u64, bool> = HashMap::new();
            for c in hub.clients.values().filter(|c| c.screen != ScreenMode::Off) {
                if let Some(p) = c.pane {
                    *screens.entry(p).or_default() |= c.screen == ScreenMode::On;
                }
            }
            (panes, screens, !hub.clients.is_empty())
        };
        // a device with a push token is told even while its app is closed
        let anyone = connected
            || shared
                .devices
                .lock()
                .unwrap()
                .iter()
                .any(|d| d.alerts().is_some());
        for &pane in &panes {
            if refresh(shared, pane).is_ok()
                && let Some(&force) = screens.get(&pane)
            {
                refresh_screen(shared, pane, force);
            }
        }
        // a client that stopped following its pane lets go of its fit
        let _ = sync_fits(shared, None);
        if !anyone {
            // nobody to tell: the next client starts from where panes stand
            shared.done.lock().unwrap().panes.clear();
            last_topology = None;
            continue;
        }
        if last_topology.is_none_or(|t| t.elapsed() >= cfg.topology_poll) {
            last_topology = Some(Instant::now());
            let windows = crate::bound_topology(shared.source.topology());
            let all: HashSet<u64> = windows
                .iter()
                .flat_map(|w| &w.tabs)
                .flat_map(|t| &t.panes)
                .map(|p| p.id)
                .collect();
            // `done` for the panes nobody follows
            for &pane in all.difference(&panes) {
                if let Ok(mut page) = shared.source.blocks_meta(pane, DONE_PROBE) {
                    crate::bound_blocks(&mut page.blocks);
                    let done = shared.done.lock().unwrap().observe(pane, &page.blocks);
                    broadcast(shared, &done);
                    alert(shared, &done);
                }
            }
            shared.done.lock().unwrap().retain(&all);
            let f = frame(&ServerMsg::Topology { windows });
            let mut hub = shared.hub.lock().unwrap();
            hub.forget_feeds(&all, &panes);
            if hub.topology.as_ref() != Some(&f) {
                for c in hub.clients.values() {
                    c.push(f.clone());
                }
                hub.topology = Some(f);
            }
        }
    }
}

/// Queue the push alerts the `done`s in `msgs` call for.
fn alert<S>(shared: &Shared<S>, msgs: &[ServerMsg]) {
    let now = Instant::now();
    let mut out = Vec::new();
    for m in msgs {
        if let ServerMsg::Done {
            pane,
            command,
            exit,
            duration_ms,
            ..
        } = m
        {
            let done = crate::push::Finished {
                pane: *pane,
                command,
                exit: *exit,
                duration_ms: *duration_ms,
            };
            let devices = shared.devices.lock().unwrap();
            let mut watches = shared.watches.lock().unwrap();
            out.extend(crate::push::alerts(
                &done,
                &devices,
                &mut watches,
                &shared.config.host,
                now,
            ));
        }
    }
    if out.is_empty() {
        return;
    }
    // the panes a device is looking at now (a live, on-screen app following
    // it): no ntfy banner for those
    let followed: HashSet<(String, u64)> = shared
        .hub
        .lock()
        .unwrap()
        .clients
        .values()
        .filter(|c| {
            c.end.load(Ordering::Acquire) == LIVE
                && c.visible
                && c.seen.within(shared.config.follow_fresh)
        })
        .filter_map(|c| c.pane.map(|p| (c.device.clone(), p)))
        .collect();
    crate::push::skip_followed(&mut out, &followed);
    shared.pusher.send(out);
}

/// Queue `msgs` to every connection.
/// What a program copied since the last look goes to the phones that may
/// operate; a phone connecting later does not get an older copy.
fn forward_copy<S: Source>(shared: &Shared<S>, copies: &mut Option<u64>) {
    if shared.hub.lock().unwrap().clients.is_empty() {
        // what is copied with nobody here is old by the time a phone connects
        *copies = None;
        return;
    }
    let found = shared.source.clipboard(copies.unwrap_or(0));
    // the first look only finds where things stand: a gateway starting anew
    // sends nobody an old copy
    let first = copies.is_none();
    *copies = Some(found.as_ref().map_or(copies.unwrap_or(0), |(seq, _)| *seq));
    let Some((_, text)) = found else {
        return;
    };
    if first {
        return;
    }
    let f = frame(&ServerMsg::Clipboard { text });
    for c in shared.hub.lock().unwrap().clients.values() {
        if c.permission == Permission::Operate {
            c.push(f.clone());
        }
    }
}

/// A theme that differs from the last one sent goes to every connection.
/// Nobody here: nothing is asked. A phone that has the theme from its
/// welcome may get it once more (the first look, or one after a change
/// while nobody was here), never miss a change made after its welcome.
fn forward_theme<S: Source>(shared: &Shared<S>, last: &mut Option<Theme>) {
    if shared.hub.lock().unwrap().clients.is_empty() {
        return;
    }
    let Some(now) = shared.source.theme() else {
        return;
    };
    if last.as_ref() == Some(&now) {
        return;
    }
    *last = Some(now.clone());
    broadcast(shared, &[ServerMsg::Theme { theme: now }]);
}

fn broadcast<S>(shared: &Shared<S>, msgs: &[ServerMsg]) {
    if msgs.is_empty() {
        return;
    }
    let hub = shared.hub.lock().unwrap();
    for m in msgs {
        let f = frame(m);
        for c in hub.clients.values() {
            c.push(f.clone());
        }
    }
}

/// Re-read `devices.json`; connections whose key is gone end (an unreadable
/// file revokes everyone until it is readable again), and so do those whose
/// device's permission changed (they reconnect with the new one).
fn reload_devices<S>(shared: &Shared<S>) {
    let now = registry::devices(&shared.config.dir).unwrap_or_else(|e| {
        log::warn!("remote: {e}");
        Vec::new()
    });
    let keys: HashMap<[u8; KEY_LEN], Permission> = now
        .iter()
        .filter_map(|d| Some((d.key_bytes()?, d.permission)))
        .collect();
    *shared.devices.lock().unwrap() = now;
    let hub = shared.hub.lock().unwrap();
    for c in hub.clients.values() {
        let why = match keys.get(&c.key) {
            None => REVOKED,
            Some(p) if *p != c.permission => PERMISSION,
            Some(_) => continue,
        };
        let _ = c
            .end
            .compare_exchange(LIVE, why, Ordering::AcqRel, Ordering::Acquire);
    }
}

/// Fetch `pane`'s newest blocks and broadcast what changed. A vanished pane
/// ends its subscriptions with `no_pane`.
fn refresh<S: Source>(shared: &Shared<S>, pane: u64) -> Result<(), SourceError> {
    let _one = shared.poll_lock.lock().unwrap();
    let result = shared
        .source
        .blocks(pane, None, MAX_BLOCKS)
        .map(|mut page| {
            crate::bound_blocks(&mut page.blocks);
            page
        });
    if let Ok(page) = &result {
        let done = shared.done.lock().unwrap().observe(pane, &page.blocks);
        broadcast(shared, &done);
        alert(shared, &done);
    }
    let mut hub = shared.hub.lock().unwrap();
    match result {
        Ok(page) => {
            let frames = match hub.feeds.get_mut(&pane) {
                Some(feed) => feed.apply(pane, page),
                None => {
                    let mut feed = Feed::new(page);
                    feed.seq = shared.seq_base;
                    hub.feeds.insert(pane, feed);
                    Vec::new()
                }
            };
            for c in hub.clients.values().filter(|c| c.pane == Some(pane)) {
                for f in &frames {
                    c.push(f.clone());
                }
            }
            Ok(())
        }
        Err(SourceError::NoPane) => {
            hub.feeds.remove(&pane);
            let f = frame(&ServerMsg::error(ErrorCode::NoPane, "pane closed"));
            for c in hub.clients.values_mut().filter(|c| c.pane == Some(pane)) {
                c.follow(None);
                c.push(f.clone());
            }
            Err(SourceError::NoPane)
        }
        Err(e) => {
            log::debug!("remote: pane {pane}: {e:?}");
            Err(e)
        }
    }
}

/// Read `pane`'s screen (`force`: also off the alternate screen) and offer
/// it to each connection that wants it ([`ScreenOut::offer`]); its loop
/// writes what changed since the client's last one: everything after a
/// mode change or a resize, else the changed rows; nothing when nothing
/// changed. `screen_end` when screens stop for it.
fn refresh_screen<S: Source>(shared: &Shared<S>, pane: u64, force: bool) {
    let screen = match shared.source.screen(pane, force) {
        Ok(s) => s.map(clamp_screen),
        Err(e) => {
            // a closed pane ends the subscription through `refresh`
            log::debug!("remote: screen of pane {pane}: {e:?}");
            return;
        }
    };
    let mut hub = shared.hub.lock().unwrap();
    for c in hub.clients.values_mut().filter(|c| c.pane == Some(pane)) {
        let wanted = match c.screen {
            ScreenMode::Off => continue,
            ScreenMode::Auto => screen.as_ref().filter(|s| s.alt),
            ScreenMode::On => screen.as_ref(),
        };
        c.out.offer(wanted);
    }
}

/// Write connection `id`'s due screen frame, if any: a `screen_end`, or
/// its pending screen diffed against the one it was last written. What
/// went out becomes `shown` (unless the mode changed meanwhile); a frame
/// over [`MAX_GATEWAY_MESSAGE`] is dropped and the next one is full. False
/// when the socket failed.
fn flush_screen<S, T: Socket>(shared: &Shared<S>, id: u64, ch: &mut Conn<T>) -> bool {
    let (pane, prev, s, epoch) = {
        let mut hub = shared.hub.lock().unwrap();
        let Some(c) = hub.clients.get_mut(&id) else {
            return true;
        };
        let Some(pane) = c.pane else {
            return true;
        };
        if std::mem::take(&mut c.out.end) {
            c.out.shown = None;
            c.out.basis = None;
            drop(hub);
            return send_msg(ch, &ServerMsg::ScreenEnd { pane });
        }
        let Some(s) = c.out.pending.take() else {
            return true;
        };
        (pane, c.out.shown.take(), s, c.out.epoch)
    };
    let (shown, basis) = match screen_frame(pane, prev.as_ref(), &s) {
        // nothing changed
        None => (prev, Some(s)),
        Some((f, plain)) if f.len() <= MAX_GATEWAY_MESSAGE => {
            if !ch.send(&f) {
                return false;
            }
            (Some(plain.unwrap_or_else(|| s.clone())), Some(s))
        }
        Some((f, _)) => {
            log::error!("remote: dropped a {} byte screen", f.len());
            (None, None)
        }
    };
    let mut hub = shared.hub.lock().unwrap();
    if let Some(c) = hub.clients.get_mut(&id)
        && c.out.epoch == epoch
    {
        c.out.shown = shown;
        c.out.basis = basis;
    }
    true
}

/// A source's screen within the protocol's bounds: at most
/// [`MAX_SCREEN_COLS`] × [`MAX_SCREEN_ROWS`], one entry per row, the
/// cursor inside it.
fn clamp_screen(mut s: Screen) -> Screen {
    s.cols = s.cols.min(MAX_SCREEN_COLS);
    s.rows = s.rows.min(MAX_SCREEN_ROWS);
    s.lines.resize(s.rows as usize, Vec::new());
    s.cursor = s.cursor.filter(|[r, c]| *r < s.rows && *c < s.cols);
    s
}

/// The `screen` frame that takes a client from `prev` to `s`, or none when
/// nothing changed. A full screen over [`MAX_GATEWAY_MESSAGE`] (or a diff
/// that big) goes out whole without styles, each row's text cut to its
/// share of the limit; that unstyled screen comes back with the frame (it
/// is what the client then has).
fn screen_frame(
    pane: u64,
    prev: Option<&Screen>,
    s: &Screen,
) -> Option<(Arc<str>, Option<Screen>)> {
    let full = prev.is_none_or(|p| (p.cols, p.rows) != (s.cols, s.rows));
    let lines: Vec<(u16, Vec<Run>)> = s
        .lines
        .iter()
        .enumerate()
        .filter(|&(i, l)| full || prev.is_some_and(|p| p.lines.get(i) != Some(l)))
        .map(|(i, l)| (i as u16, l.clone()))
        .collect();
    if !full
        && lines.is_empty()
        && prev.is_some_and(|p| {
            (p.alt, p.cursor, &p.fg, &p.bg, &p.rules, p.age_ms)
                == (s.alt, s.cursor, &s.fg, &s.bg, &s.rules, s.age_ms)
        })
    {
        return None;
    }
    let msg = |full: bool, lines: Vec<(u16, Vec<Run>)>| {
        frame(&ServerMsg::Screen {
            pane,
            cols: s.cols,
            rows: s.rows,
            alt: s.alt,
            cursor: s.cursor,
            full,
            fg: s.fg.clone(),
            bg: s.bg.clone(),
            rules: s.rules.clone(),
            lines,
            age_ms: s.age_ms,
        })
    };
    let f = msg(full, lines);
    if f.len() <= MAX_GATEWAY_MESSAGE {
        return Some((f, None));
    }
    // each row's share of the frame, its JSON envelope (`[i,[[…,null,null,0]]],`) aside
    let share = (MAX_GATEWAY_MESSAGE - 1024) / s.lines.len().max(1) - 32;
    let plain = Screen {
        lines: s
            .lines
            .iter()
            .map(|runs| {
                let text: String = runs.iter().map(|r| r.text.as_str()).collect();
                match fit_json(&text, share) {
                    "" => Vec::new(),
                    t => vec![Run::plain(t)],
                }
            })
            .collect(),
        cols: s.cols,
        rows: s.rows,
        alt: s.alt,
        cursor: s.cursor,
        fg: s.fg.clone(),
        bg: s.bg.clone(),
        rules: s.rules.clone(),
        age_ms: s.age_ms,
    };
    // the client has it already (an unstyled screen went out before)
    if prev == Some(&plain) {
        return None;
    }
    let lines = plain
        .lines
        .iter()
        .enumerate()
        .map(|(i, l)| (i as u16, l.clone()))
        .collect();
    Some((msg(true, lines), Some(plain)))
}

/// The longest prefix of `text` whose JSON string escape takes at most
/// `budget` bytes.
fn fit_json(text: &str, budget: usize) -> &str {
    let mut used = 0;
    for (i, c) in text.char_indices() {
        used += match c {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            c if (c as u32) < 0x20 => 6,
            c => c.len_utf8(),
        };
        if used > budget {
            return &text[..i];
        }
    }
    text
}

type Conn<T> = Channel<T>;

fn send_msg<T: Socket>(ch: &mut Conn<T>, msg: &ServerMsg) -> bool {
    send(ch, &frame(msg))
}

fn send<T: Socket>(ch: &mut Conn<T>, f: &str) -> bool {
    if f.len() > MAX_GATEWAY_MESSAGE {
        log::error!("remote: dropped a {} byte message", f.len());
        return true;
    }
    ch.send(f)
}

/// Send `msg` and close.
fn refuse<T: Socket>(mut ch: Conn<T>, msg: ServerMsg) {
    send_msg(&mut ch, &msg);
    ch.close();
}

/// Where a connection came from.
#[derive(Debug, Clone)]
pub(crate) struct Peer {
    pub via: Via,
    /// What history records: `IP:PORT`, or the relay's `host:port`.
    pub address: String,
    /// The address failed handshakes count against (none through a relay:
    /// every phone shares the relay's address there, and the relay limits
    /// `/connect` per phone address itself).
    pub ip: Option<IpAddr>,
}

impl<S: Source> Shared<S> {
    /// Append one history line (best effort).
    fn record(&self, device: Option<(&str, &str)>, peer: &Peer, start: SystemTime, reason: &str) {
        let entry = registry::Entry {
            device: device.map(|d| d.0.to_string()),
            name: device.map(|d| d.1.to_string()),
            via: peer.via,
            address: peer.address.clone(),
            start: registry::whole_secs(start),
            end: registry::whole_secs(SystemTime::now()),
            reason: reason.to_string(),
        };
        if let Err(e) = registry::append_history(&self.config.dir, &entry) {
            log::warn!("remote: cannot write history.jsonl: {e}");
        }
    }

    /// A handshake that did not authenticate: counted against the address
    /// (direct sockets) and recorded.
    fn failed(&self, peer: &Peer, start: SystemTime, reason: &str) {
        if let Some(ip) = peer.ip {
            self.failures.lock().unwrap().fail(
                ip,
                Instant::now(),
                self.config.failure_window,
                self.config.refusal,
            );
        }
        self.record(None, peer, start, reason);
    }
}

/// Drop the client's slot when its thread ends, and record the connection.
struct SlotGuard<'a, S: Source> {
    shared: &'a Shared<S>,
    id: u64,
    device: String,
    name: String,
    peer: Peer,
    since: SystemTime,
    reason: &'static str,
}

impl<S: Source> Drop for SlotGuard<'_, S> {
    fn drop(&mut self) {
        self.shared.hub.lock().unwrap().clients.remove(&self.id);
        let _ = sync_fits(self.shared, None);
        write_status(self.shared);
        let _ = registry::touch(&self.shared.config.dir, &self.device, SystemTime::now());
        self.shared.record(
            Some((&self.device, &self.name)),
            &self.peer,
            self.since,
            self.reason,
        );
    }
}

/// When `opened` was, by the wall clock.
fn wall(opened: Instant) -> SystemTime {
    SystemTime::now()
        .checked_sub(opened.elapsed())
        .unwrap_or_else(SystemTime::now)
}

/// A socket accepted on the listening address.
fn client<S: Source>(
    shared: &Shared<S>,
    stream: TcpStream,
    addr: SocketAddr,
    opened: Instant,
    busy: bool,
) {
    let cfg = &shared.config;
    let deadline = opened + cfg.handshake_deadline;
    let peer = Peer {
        via: Via::Lan,
        address: addr.to_string(),
        ip: Some(addr.ip()),
    };
    let _ = stream.set_nodelay(true);
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(1));
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(remaining)).is_err()
        || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
    {
        return;
    }
    let stream = transport::Timed::new(stream, deadline);
    let ws = match tungstenite::accept_with_config(stream, Some(client_ws_config())) {
        Ok(ws) => ws,
        Err(HandshakeError::Interrupted(_)) | Err(HandshakeError::Failure(_)) => {
            shared.failed(&peer, wall(opened), "handshake");
            return;
        }
    };
    connection(shared, ws, peer, opened, busy);
}

/// The WebSocket limits of a client connection (direct or through a relay)
/// until it authenticates: every frame within [`MAX_CLIENT_MESSAGE`]
/// (the handshake's are smaller still). An operate device's are raised
/// for `upload` chunks once it has ([`allow_uploads`]).
pub(crate) fn client_ws_config() -> WebSocketConfig {
    let max_frame = transport::max_client_frame(MAX_CLIENT_MESSAGE);
    WebSocketConfig::default()
        .max_frame_size(Some(max_frame))
        .max_message_size(Some(max_frame))
}

/// Raise an authenticated operate device's limits to an `upload` chunk's:
/// its frames and the messages they assemble ([`MAX_UPLOAD_MESSAGE`]).
/// Every other message is still held to [`MAX_CLIENT_MESSAGE`] in
/// [`session`] before it is parsed.
fn allow_uploads<T: Socket>(ch: &mut Conn<T>) {
    let max_frame = transport::max_client_frame(MAX_UPLOAD_MESSAGE);
    ch.ws.set_config(|c| {
        c.max_frame_size = Some(max_frame);
        c.max_message_size = Some(max_frame);
    });
    ch.set_limit(MAX_UPLOAD_MESSAGE);
}

/// One connection once its WebSocket is open (accepted here, or dialed at
/// the relay): the Noise handshake by `opened + handshake_deadline`,
/// `busy`, authentication, then the session.
pub(crate) fn connection<S: Source, T: Socket>(
    shared: &Shared<S>,
    mut ws: WebSocket<T>,
    peer: Peer,
    opened: Instant,
    busy: bool,
) {
    let cfg = &shared.config;
    let deadline = opened + cfg.handshake_deadline;
    let start = wall(opened);
    if ws.get_ref().set_read_timeout(Some(READ_SLICE)).is_err() {
        return;
    }
    let hello = serde_json::to_vec(&GatewayHello {
        v: VERSION,
        host: cfg.host.clone(),
        os: cfg.os.clone(),
        version: cfg.version.clone(),
    })
    .unwrap_or_default();
    let (noise, payload) = match transport::respond(&mut ws, &shared.key, &hello, deadline) {
        Ok(r) => {
            ws.get_mut().end_deadline();
            r
        }
        Err(e) => {
            log::info!("remote: {}: {e}", peer.address);
            shared.failed(&peer, start, "handshake");
            let _ = ws.close(None);
            let _ = ws.flush();
            return;
        }
    };
    // only an authenticated operate device may send more (`allow_uploads`)
    let ch = Channel::new(ws, noise, MAX_CLIENT_MESSAGE);
    if busy && !shared.displace(&ch.remote_static()) {
        refuse(
            ch,
            ServerMsg::error(ErrorCode::Busy, "too many connections"),
        );
        shared.record(None, &peer, start, "busy");
        return;
    }
    let (device, mut ch) = match authenticate(shared, ch, &payload, &peer.address) {
        Ok(r) => r,
        Err(reason) => {
            shared.failed(&peer, start, reason);
            return;
        }
    };
    // a permission change ends the connection: a view device never uploads
    if device.permission == Permission::Operate {
        allow_uploads(&mut ch);
    }
    serve_device(shared, ch, device, peer);
}

/// The client's hello and key against the paired devices and pairing
/// secrets. `Err(reason)` after refusing the connection.
fn authenticate<S: Source, T: Socket>(
    shared: &Shared<S>,
    ch: Conn<T>,
    payload: &[u8],
    ip: &str,
) -> Result<(Device, Conn<T>), &'static str> {
    let v = serde_json::from_slice::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get("v").and_then(serde_json::Value::as_u64));
    if v.is_some() && v != Some(u64::from(VERSION)) {
        refuse(
            ch,
            ServerMsg::error(
                ErrorCode::Version,
                format!("this gateway speaks protocol version {VERSION}"),
            ),
        );
        return Err("version");
    }
    let Ok(hello) = serde_json::from_slice::<ClientHello>(payload) else {
        refuse(ch, ServerMsg::error(ErrorCode::Auth, "malformed hello"));
        return Err("auth");
    };
    let key = ch.remote_static();
    // read now, not the watcher's copy: a device revoked or changed a
    // moment ago must not authenticate with what it had
    let devices = registry::devices(&shared.config.dir).unwrap_or_else(|e| {
        log::warn!("remote: {e}; refusing every device until it is fixed");
        Vec::new()
    });
    let paired = registry::find_device(&devices, &key).cloned();
    let device = match (paired, &hello.pair) {
        (Some(d), _) => Some(d),
        (None, Some(secret)) => {
            match registry::pair_device(
                &shared.config.dir,
                secret,
                &hello.name,
                &hello.platform,
                &key,
                SystemTime::now(),
            ) {
                Ok(Some(d)) => {
                    log::info!(
                        "remote: {ip}: paired {:?} ({}) as {} with {:?}",
                        d.name,
                        d.platform,
                        d.id,
                        d.permission
                    );
                    reload_devices(shared);
                    Some(d)
                }
                Ok(None) => None,
                Err(e) => {
                    log::warn!("remote: pairing: {e}");
                    None
                }
            }
        }
        (None, None) => None,
    };
    let Some(device) = device else {
        log::info!("remote: {ip}: refused an unpaired key");
        refuse(
            ch,
            ServerMsg::error(ErrorCode::Auth, "this device is not paired"),
        );
        return Err("auth");
    };
    Ok((device, ch))
}

fn serve_device<S: Source, T: Socket>(
    shared: &Shared<S>,
    mut ch: Conn<T>,
    device: Device,
    peer: Peer,
) {
    let cfg = &shared.config;
    let now = SystemTime::now();
    let _ = registry::touch(&cfg.dir, &device.id, now);
    let theme = shared.source.theme();
    if !send_msg(
        &mut ch,
        &ServerMsg::Welcome {
            v: VERSION,
            host: cfg.host.clone(),
            os: cfg.os.clone(),
            version: cfg.version.clone(),
            device: device.id.clone(),
            permission: device.permission,
            features: FEATURES
                .iter()
                .copied()
                // only where the computer allows shells while Rift is closed
                .chain(shared.source.can_start().then_some("start"))
                .chain(theme.is_some().then_some("theme"))
                .map(String::from)
                .collect(),
            theme,
        },
    ) {
        shared.record(Some((&device.id, &device.name)), &peer, now, "error");
        return;
    }
    let topology = frame(&ServerMsg::Topology {
        windows: crate::bound_topology(shared.source.topology()),
    });
    let id = shared.next_client.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = sync_channel(CLIENT_QUEUE);
    let end = Arc::new(AtomicU8::new(LIVE));
    let seen = Arc::new(Heard::new());
    let closing = Arc::new(AtomicBool::new(false));
    {
        let mut hub = shared.hub.lock().unwrap();
        // connected after the announcement: it owes one too
        closing.store(shared.closing.lock().unwrap().is_some(), Ordering::Release);
        hub.clients.insert(
            id,
            Slot {
                tx,
                pane: None,
                refused: None,
                visible: true,
                screen: ScreenMode::Off,
                out: ScreenOut::default(),
                end: end.clone(),
                closing: closing.clone(),
                seen: seen.clone(),
                key: ch.remote_static(),
                device: device.id.clone(),
                name: device.name.clone(),
                fit: None,
                permission: device.permission,
                since: now,
                via: peer.via,
            },
        );
        hub.topology = Some(topology.clone());
    }
    let via = peer.via;
    let mut slot = SlotGuard {
        shared,
        id,
        device: device.id.clone(),
        name: device.name.clone(),
        peer,
        since: now,
        reason: "error",
    };
    write_status(shared);
    log::info!(
        "remote: {:?} ({}) connected via {}",
        device.name,
        device.id,
        via.as_str()
    );
    if !send(&mut ch, &topology) {
        return;
    }
    slot.reason = session(shared, id, ch, &device, &rx, &end, via);
    log::info!(
        "remote: {:?} ({}) disconnected: {}",
        device.name,
        device.id,
        slot.reason
    );
}

/// An authenticated client's loop: queued frames out, requests in, pings.
/// Returns why it ended (a history reason).
fn session<S: Source, T: Socket>(
    shared: &Shared<S>,
    id: u64,
    mut ch: Conn<T>,
    device: &Device,
    rx: &Receiver<Arc<str>>,
    end: &AtomicU8,
    via: Via,
) -> &'static str {
    let cfg = &shared.config;
    // when this client was last heard from, for [`alert`], and whether it
    // owes a `closing`
    let (seen, closing) = shared
        .hub
        .lock()
        .unwrap()
        .clients
        .get(&id)
        .map(|c| (c.seen.clone(), c.closing.clone()))
        .unwrap_or_else(|| (Arc::new(Heard::new()), Arc::new(AtomicBool::new(false))));
    let closing = &*closing;
    // the phone's last sign of life: through a relay WebSocket pongs come
    // from the relay, so only Noise messages count (the app pings in them)
    let mut heard = Instant::now();
    let mut pinged = Instant::now();
    let mut runs = Rate::new(RUN_RATE);
    // `keys` and `text`
    let mut keys = Rate::new(KEYS_RATE);
    let mut fits = Rate::new(FIT_RATE);
    let mut opens = Rate::new(OPEN_RATE);
    let mut moves = Rate::new(FIT_RATE);
    let mut outputs = Rate::new(OUTPUT_RATE);
    let mut scrolls = Rate::new(crate::SCROLL_RATE);
    let mut uploads = Rate::new(UPLOAD_RATE);
    let mut upload = Uploads::default();
    // the read's wait, shortened while this client's pane is kicked (see `HOT_READ_SLICE`)
    let mut slice = READ_SLICE;
    loop {
        match end.load(Ordering::Acquire) {
            LIVE => {}
            REVOKED => {
                log::info!("remote: {:?} ({}) revoked", device.name, device.id);
                refuse(
                    ch,
                    ServerMsg::error(ErrorCode::Auth, "this device was revoked"),
                );
                return "revoked";
            }
            DISPLACED => {
                log::info!("remote: {:?} ({}) reconnected", device.name, device.id);
                ch.close();
                return "displaced";
            }
            PERMISSION => {
                // the next handshake carries the new permission
                log::info!(
                    "remote: {:?} ({}) permission changed; reconnect",
                    device.name,
                    device.id
                );
                ch.close();
                return "permission";
            }
            _ => {
                ch.close();
                return "too_slow";
            }
        }
        if shared.stop.load(Ordering::Acquire) {
            // a `closing` announced just before the stop still goes out
            send_closing(shared, &mut ch, closing);
            ch.close();
            return "stopped";
        }
        while let Ok(f) = rx.try_recv() {
            if !send(&mut ch, &f) {
                return "error";
            }
        }
        if !send_closing(shared, &mut ch, closing) {
            return "error";
        }
        if !flush_screen(shared, id, &mut ch) {
            return "error";
        }
        if heard.elapsed() >= cfg.idle_timeout {
            log::info!("remote: client {id} silent, dropped");
            ch.close();
            return "idle";
        }
        if let Some(r) = upload.expire(shared, Instant::now())
            && !send(&mut ch, &fit_reply(&r))
        {
            return "error";
        }
        if pinged.elapsed() >= cfg.ping_interval {
            pinged = Instant::now();
            if ch.ws.send(Message::Ping(Vec::new().into())).is_err() {
                return "error";
            }
        }
        let want = if pane_kicked(shared, id) {
            HOT_READ_SLICE
        } else {
            cfg.read_slice
        };
        if want != slice && ch.ws.get_ref().set_read_timeout(Some(want)).is_ok() {
            slice = want;
        }
        let text = match ch.recv() {
            Recv::Idle => continue,
            Recv::Alive => {
                if via == Via::Lan {
                    heard = Instant::now();
                    seen.mark();
                }
                continue;
            }
            Recv::Partial => {
                heard = Instant::now();
                seen.mark();
                continue;
            }
            Recv::Closed => return "closed",
            Recv::TooLarge => {
                refuse(
                    ch,
                    ServerMsg::error(ErrorCode::TooLarge, "message too large"),
                );
                return "too_large";
            }
            Recv::Violation(why) => {
                refuse(ch, ServerMsg::error(ErrorCode::BadRequest, why));
                return "bad_request";
            }
            Recv::Msg(m) => m,
        };
        heard = Instant::now();
        seen.mark();
        // only an `upload` chunk or a `paste` may pass MAX_CLIENT_MESSAGE,
        // told by its type alone before the message is built (`MsgType`
        // skips the rest without keeping any of it)
        if text.len() > MAX_CLIENT_MESSAGE
            && !serde_json::from_slice::<MsgType>(&text)
                .is_ok_and(|m| m.t == "upload" || m.t == "paste")
        {
            refuse(
                ch,
                ServerMsg::error(ErrorCode::TooLarge, "message too large"),
            );
            return "too_large";
        }
        let msg = serde_json::from_slice::<ClientMsg>(&text);
        if text.len() > MAX_CLIENT_MESSAGE
            && !matches!(msg, Ok(ClientMsg::Upload { .. } | ClientMsg::Paste { .. }))
        {
            refuse(
                ch,
                ServerMsg::error(ErrorCode::TooLarge, "message too large"),
            );
            return "too_large";
        }
        let reply = match msg {
            Err(e) => Some(ServerMsg::error(ErrorCode::BadRequest, e.to_string())),
            Ok(ClientMsg::Ping { n }) => Some(ServerMsg::Pong { n }),
            Ok(ClientMsg::Visible { on }) => {
                if let Some(c) = shared.hub.lock().unwrap().clients.get_mut(&id) {
                    c.visible = on;
                }
                None
            }
            Ok(ClientMsg::Unsubscribe) => {
                if let Some(c) = shared.hub.lock().unwrap().clients.get_mut(&id) {
                    c.follow(None);
                }
                let _ = sync_fits(shared, None);
                None
            }
            Ok(ClientMsg::Subscribe { pane, after }) => {
                let reply = subscribe(shared, id, pane, after);
                let _ = sync_fits(shared, None);
                reply
            }
            Ok(ClientMsg::Screen { pane, mode }) => set_screen(shared, id, pane, mode),
            Ok(ClientMsg::Output {
                id: rid,
                pane,
                block,
                from,
                limit,
            }) => Some(output(shared, &mut outputs, rid, pane, block, from, limit)),
            Ok(ClientMsg::Dirs { id: rid, pane }) => {
                Some(menu(shared, &mut outputs, rid, pane, MenuKind::Dirs))
            }
            Ok(ClientMsg::Branches { id: rid, pane }) => {
                Some(menu(shared, &mut outputs, rid, pane, MenuKind::Branches))
            }
            Ok(ClientMsg::Changes { id: rid, pane }) => {
                Some(menu(shared, &mut outputs, rid, pane, MenuKind::Changes))
            }
            Ok(ClientMsg::Chip {
                id: rid,
                pane,
                kind,
            }) => Some(chip(shared, &mut outputs, rid, pane, kind)),
            Ok(ClientMsg::ResetElapsed { id: rid, pane }) => {
                Some(reset_elapsed(shared, device, &mut opens, rid, pane))
            }
            Ok(ClientMsg::Diff {
                id: rid,
                pane,
                path,
                staged,
            }) => Some(diff(shared, device, &mut outputs, rid, pane, path, staged)),
            Ok(ClientMsg::Complete {
                id: rid,
                pane,
                line,
                cursor,
            }) => Some(complete(
                shared,
                device,
                &mut outputs,
                rid,
                pane,
                line,
                cursor,
            )),
            Ok(ClientMsg::Push {
                token,
                ntfy,
                long_ms,
                show_command,
                failures_only,
            }) => set_push(
                shared,
                device,
                registry::PushReg {
                    token,
                    ntfy,
                    long_ms,
                    show_command,
                    failures_only,
                },
            ),
            Ok(ClientMsg::Notify { pane, off: true }) => {
                shared
                    .watches
                    .lock()
                    .unwrap()
                    .take(&device.id, pane, Instant::now());
                None
            }
            Ok(ClientMsg::Notify { pane, off: false }) => {
                if shared
                    .watches
                    .lock()
                    .unwrap()
                    .add(&device.id, pane, Instant::now())
                {
                    None
                } else {
                    Some(ServerMsg::error(
                        ErrorCode::BadRequest,
                        "waiting on too many panes",
                    ))
                }
            }
            Ok(ClientMsg::Older { pane, before }) => Some(older(shared, pane, before)),
            Ok(ClientMsg::Forget) => {
                match registry::forget(&cfg.dir, &device.id) {
                    Ok(_) => log::info!("remote: {:?} ({}) forgot itself", device.name, device.id),
                    Err(e) => log::warn!("remote: forget {}: {e}", device.id),
                }
                reload_devices(shared);
                ch.close();
                return "forgotten";
            }
            Ok(ClientMsg::Run {
                id: rid,
                pane,
                command,
            }) => Some(input(shared, device, &mut runs, rid, pane, || {
                check_command(&command).map(|()| Input::Command(command.clone()))
            })),
            Ok(ClientMsg::Keys {
                id: rid,
                pane,
                keys: names,
            }) => Some(input(shared, device, &mut keys, rid, pane, || {
                parse_keys(&names).map(Input::Keys)
            })),
            Ok(ClientMsg::Text {
                id: rid,
                pane,
                text,
            }) => Some(input(shared, device, &mut keys, rid, pane, || {
                check_text(&text).map(|()| Input::Text(text.clone()))
            })),
            Ok(ClientMsg::Paste {
                id: rid,
                pane,
                text,
            }) => Some(input(shared, device, &mut keys, rid, pane, || {
                check_paste(&text).map(|()| Input::Paste(text.clone()))
            })),
            Ok(ClientMsg::Mouse {
                id: rid,
                pane,
                col,
                row,
            }) => Some(input(shared, device, &mut keys, rid, pane, || {
                Ok(Input::Mouse { col, row })
            })),
            Ok(ClientMsg::Fit {
                id: rid,
                pane,
                cols,
                rows,
            }) => Some(fit(shared, device, id, &mut fits, rid, pane, cols, rows)),
            Ok(ClientMsg::Open {
                id: rid,
                pane,
                kind,
            }) => Some(open(shared, device, &mut opens, rid, pane, kind)),
            Ok(ClientMsg::Close {
                id: rid,
                pane,
                kind,
            }) => Some(close(shared, device, &mut opens, rid, pane, kind)),
            Ok(ClientMsg::Move {
                id: rid,
                pane,
                kind,
                to,
            }) => Some(move_to(shared, device, &mut moves, rid, pane, kind, to)),
            Ok(ClientMsg::Start { id: rid }) => Some(start(shared, device, &mut opens, rid)),
            Ok(ClientMsg::Scroll {
                id: rid,
                pane,
                lines,
            }) => Some(scroll(shared, device, &mut scrolls, rid, pane, lines)),
            Ok(ClientMsg::Upload {
                id: rid,
                pane,
                size,
                offset,
                data,
                ext,
            }) => upload.chunk(
                shared,
                device,
                &mut uploads,
                Chunk {
                    rid,
                    pane,
                    size,
                    offset,
                    data,
                    ext,
                },
            ),
        };
        // what was queued meanwhile goes first (`open` queues the topology
        // with its new pane before its ack)
        while let Ok(f) = rx.try_recv() {
            if !send(&mut ch, &f) {
                return "error";
            }
        }
        if let Some(r) = reply
            && !send(&mut ch, &fit_reply(&r))
        {
            return "error";
        }
    }
}

/// Write the `closing` this connection owes, if any. False when the write
/// failed.
fn send_closing<S, T: Socket>(shared: &Shared<S>, ch: &mut Conn<T>, owed: &AtomicBool) -> bool {
    if !owed.load(Ordering::Acquire) {
        return true;
    }
    let Some(f) = shared.closing.lock().unwrap().clone() else {
        return true;
    };
    let ok = send(ch, &f);
    owed.store(false, Ordering::Release);
    ok
}

/// `reply` as a frame; an answer over [`MAX_GATEWAY_MESSAGE`] (JSON escapes
/// can grow a bounded page several times) is refused with a `too_large` ack
/// instead, so the request does not wait for an answer that never comes.
fn fit_reply(reply: &ServerMsg) -> Arc<str> {
    let f = frame(reply);
    if f.len() <= MAX_GATEWAY_MESSAGE {
        return f;
    }
    let id = match reply {
        ServerMsg::Output { id, .. }
        | ServerMsg::Dirs { id, .. }
        | ServerMsg::Branches { id, .. }
        | ServerMsg::Changes { id, .. }
        | ServerMsg::Chip { id, .. }
        | ServerMsg::Diff { id, .. }
        | ServerMsg::Complete { id, .. } => id,
        // the rest are small; `send` drops one with a log line
        _ => return f,
    };
    log::warn!("remote: a {} byte answer refused as too large", f.len());
    frame(&ServerMsg::ack_err(
        id.clone(),
        ErrorCode::TooLarge,
        "the answer is too large to send",
    ))
}

/// `push`: store (or, with neither a token nor an ntfy URL, clear) the
/// device's alert registration.
fn set_push<S>(shared: &Shared<S>, device: &Device, reg: registry::PushReg) -> Option<ServerMsg> {
    if reg
        .token
        .as_deref()
        .is_some_and(|t| !crate::push::valid_token(t))
    {
        return Some(ServerMsg::error(
            ErrorCode::BadRequest,
            "not an Expo push token",
        ));
    }
    if reg
        .ntfy
        .as_deref()
        .is_some_and(|u| !crate::push::valid_ntfy(u))
    {
        return Some(ServerMsg::error(
            ErrorCode::BadRequest,
            "not an ntfy topic URL",
        ));
    }
    let reg = if reg.token.is_none() && reg.ntfy.is_none() {
        shared.watches.lock().unwrap().forget(&device.id);
        None
    } else if reg.long_ms.is_some_and(|ms| ms > crate::push::MAX_LONG_MS) {
        return Some(ServerMsg::error(
            ErrorCode::BadRequest,
            "long_ms is over a day",
        ));
    } else {
        Some(reg)
    };
    match registry::set_push(&shared.config.dir, &device.id, reg) {
        Ok(_) => {
            reload_devices(shared);
            None
        }
        Err(e) => {
            log::warn!("remote: push for {}: {e}", device.id);
            Some(ServerMsg::error(
                ErrorCode::Busy,
                "cannot save the push token now",
            ))
        }
    }
}

/// `run` / `keys` / `text`: permission, rate, validation, then the source.
fn input<S: Source>(
    shared: &Shared<S>,
    device: &Device,
    rate: &mut Rate,
    rid: String,
    pane: u64,
    build: impl FnOnce() -> Result<Input, (ErrorCode, String)>,
) -> ServerMsg {
    if rid.chars().count() > MAX_REQUEST_ID {
        return ServerMsg::error(
            ErrorCode::BadRequest,
            format!("id is longer than {MAX_REQUEST_ID} characters"),
        );
    }
    if !operates(shared, device) {
        return ServerMsg::ack_err(rid, ErrorCode::Forbidden, VIEW_ONLY);
    }
    if !rate.allow(Instant::now()) {
        return ServerMsg::ack_err(rid, ErrorCode::Rate, "too many requests; slow down");
    }
    let input = match build() {
        Ok(i) => i,
        Err((code, msg)) => return ServerMsg::ack_err(rid, code, msg),
    };
    let busy = match input {
        Input::Text(_) | Input::Paste(_) => {
            format!("pane {pane} is at its prompt; send the command with run")
        }
        _ => format!("pane {pane} is running a command"),
    };
    match shared.source.input(pane, &device.name, input) {
        Ok(()) => {
            kick(shared, pane);
            ServerMsg::ack_ok(rid)
        }
        Err(e) => refused_ack(rid, e, busy, format!("no pane {pane}")),
    }
}

/// The ack of a request the source refused with `e`: `busy` says why a busy
/// pane (or desktop) refused it, `no_pane` what was missing.
fn refused_ack(rid: String, e: InputError, busy: String, no_pane: String) -> ServerMsg {
    match e {
        InputError::Busy => ServerMsg::ack_err(rid, ErrorCode::Busy, busy),
        InputError::NoPane => ServerMsg::ack_err(rid, ErrorCode::NoPane, no_pane),
        InputError::Denied => ServerMsg::ack_err(
            rid,
            ErrorCode::Forbidden,
            "remote access is off on the desktop",
        ),
        InputError::ViewOnly => ServerMsg::ack_err(rid, ErrorCode::Forbidden, VIEW_ONLY),
        InputError::Unavailable(why) => ServerMsg::ack_err(rid, ErrorCode::Busy, why),
    }
}

/// `scroll`: rate (`keys`' limit, counted apart), the clamp, then the
/// source, which knows what the wheel does in the pane: any device may move
/// a pane's own view, only an `operate` device reach its program.
fn scroll<S: Source>(
    shared: &Shared<S>,
    device: &Device,
    rate: &mut Rate,
    rid: String,
    pane: u64,
    lines: i32,
) -> ServerMsg {
    if rid.chars().count() > MAX_REQUEST_ID {
        return ServerMsg::error(
            ErrorCode::BadRequest,
            format!("id is longer than {MAX_REQUEST_ID} characters"),
        );
    }
    if !rate.allow(Instant::now()) {
        return ServerMsg::ack_err(rid, ErrorCode::Rate, "too many requests; slow down");
    }
    let lines = lines.clamp(-MAX_SCROLL_LINES, MAX_SCROLL_LINES);
    if lines == 0 {
        return ServerMsg::ack_ok(rid);
    }
    match shared.source.scroll(pane, lines, operates(shared, device)) {
        Ok(()) => {
            kick(shared, pane);
            ServerMsg::ack_ok(rid)
        }
        Err(e) => refused_ack(rid, e, "pane is busy".into(), format!("no pane {pane}")),
    }
}

/// One `upload` message's fields.
struct Chunk {
    rid: String,
    pane: u64,
    size: u64,
    offset: u64,
    data: String,
    ext: Option<String>,
}

/// The image or file one connection is receiving (`upload`), at most one at a
/// time: a chunk at offset 0 starts a new one. Across the gateway at most
/// [`MAX_UPLOADS`] are received at once, each within
/// [`Config::upload_deadline`] of its first chunk.
#[derive(Default)]
struct Uploads<'a> {
    current: Option<Receiving<'a>>,
    /// An upload refused part-way: its later chunks are dropped unanswered
    /// (the client has its error).
    refused: Option<String>,
}

struct Receiving<'a> {
    rid: String,
    pane: u64,
    size: u64,
    /// Reserved for `size` at once (at most [`MAX_UPLOAD_BYTES`]): never
    /// grown by doubling as chunks arrive.
    bytes: Vec<u8>,
    /// Told by the first bytes; never asked of a file (`ext`).
    kind: Option<ImageKind>,
    /// A file's extension ([`crate::valid_upload_ext`]); `None`: an image.
    ext: Option<String>,
    /// Past it what arrived is dropped ([`Uploads::expire`]).
    deadline: Instant,
    /// Its place among the gateway's [`MAX_UPLOADS`], given back on drop.
    _slot: ConnGuard<'a>,
}

/// The `busy` message of an upload past its deadline.
fn upload_too_slow(noun: &str, deadline: Duration) -> String {
    format!(
        "the {noun} took longer than {} s to arrive; try again",
        deadline.as_secs()
    )
}

/// What an upload carrying `ext` is called in its refusals.
fn upload_noun(ext: Option<&str>) -> &'static str {
    if ext.is_some() { "file" } else { "image" }
}

/// The `busy` message of an image or file for a pane at its prompt.
fn upload_at_prompt(pane: u64, ext: Option<&str>) -> String {
    let what = if ext.is_some() { "a file" } else { "an image" };
    format!("pane {pane} is at its prompt; {what} goes to a running program")
}

impl<'a> Uploads<'a> {
    /// Take one chunk: an ack once the last one is pasted (or at the first
    /// that fails), nothing in between.
    fn chunk<S: Source>(
        &mut self,
        shared: &'a Shared<S>,
        device: &Device,
        rate: &mut Rate,
        c: Chunk,
    ) -> Option<ServerMsg> {
        if c.rid.chars().count() > MAX_REQUEST_ID {
            return Some(ServerMsg::error(
                ErrorCode::BadRequest,
                format!("id is longer than {MAX_REQUEST_ID} characters"),
            ));
        }
        let now = Instant::now();
        if c.offset == 0 {
            self.current = None;
            self.refused = None;
            if !operates(shared, device) {
                return Some(self.refuse(c.rid, ErrorCode::Forbidden, VIEW_ONLY));
            }
            if !rate.allow(now) {
                return Some(self.refuse(c.rid, ErrorCode::Rate, "too many requests; slow down"));
            }
            let noun = upload_noun(c.ext.as_deref());
            if c.ext.as_deref().is_some_and(|e| !valid_upload_ext(e)) {
                return Some(self.refuse(
                    c.rid,
                    ErrorCode::BadRequest,
                    format!("ext holds 1 to {MAX_UPLOAD_EXT} of a-z and 0-9"),
                ));
            }
            if c.size == 0 {
                return Some(self.refuse(
                    c.rid,
                    ErrorCode::BadRequest,
                    format!("the {noun} is empty"),
                ));
            }
            if c.size > MAX_UPLOAD_BYTES {
                return Some(self.refuse(
                    c.rid,
                    ErrorCode::TooLarge,
                    format!("the {noun} is larger than {} MiB", MAX_UPLOAD_BYTES >> 20),
                ));
            }
            // a pane at its prompt is refused now, not after every chunk
            if let Err(e) = shared.source.upload_ready(c.pane) {
                let msg = refused_ack(
                    c.rid.clone(),
                    e,
                    upload_at_prompt(c.pane, c.ext.as_deref()),
                    format!("no pane {}", c.pane),
                );
                return Some(self.refuse_with(c.rid, msg));
            }
            let Some(slot) = upload_slot(&shared.uploads) else {
                return Some(self.refuse(
                    c.rid,
                    ErrorCode::Busy,
                    format!("other {noun}s are arriving; try again in a moment"),
                ));
            };
            let mut bytes = Vec::new();
            // within MAX_UPLOAD_BYTES, checked above
            if bytes.try_reserve_exact(c.size as usize).is_err() {
                return Some(self.refuse(
                    c.rid,
                    ErrorCode::Busy,
                    format!("the desktop cannot hold the {noun} now"),
                ));
            }
            self.current = Some(Receiving {
                rid: c.rid.clone(),
                pane: c.pane,
                size: c.size,
                bytes,
                kind: None,
                ext: c.ext.clone(),
                deadline: now + shared.config.upload_deadline,
                _slot: slot,
            });
        } else if self.refused.as_deref() == Some(c.rid.as_str()) {
            return None;
        }
        if let Some(msg) = self.expire(shared, now) {
            return Some(msg);
        }
        let fits = self.current.as_ref().is_some_and(|r| {
            r.rid == c.rid
                && r.pane == c.pane
                && r.size == c.size
                && r.ext == c.ext
                && r.bytes.len() as u64 == c.offset
        });
        if !fits {
            return Some(self.refuse(c.rid, ErrorCode::BadRequest, "an upload chunk out of order"));
        }
        use base64::Engine as _;
        let data = match base64::engine::general_purpose::STANDARD.decode(c.data.as_bytes()) {
            Ok(d) if !d.is_empty() && d.len() <= MAX_UPLOAD_CHUNK => d,
            Ok(_) => {
                return Some(self.refuse(
                    c.rid,
                    ErrorCode::BadRequest,
                    format!("a chunk holds 1 to {MAX_UPLOAD_CHUNK} bytes"),
                ));
            }
            Err(_) => {
                return Some(self.refuse(c.rid, ErrorCode::BadRequest, "data is not base64"));
            }
        };
        let r = self.current.as_mut().expect("checked above");
        if r.bytes.len() as u64 + data.len() as u64 > r.size {
            return Some(self.refuse(
                c.rid,
                ErrorCode::BadRequest,
                "more bytes than the upload's size",
            ));
        }
        r.bytes.extend_from_slice(&data);
        let done = r.bytes.len() as u64 == r.size;
        // an image's type from the signature as soon as it has arrived (8
        // bytes, or the whole of a smaller file); a file's bytes are its own
        if r.ext.is_none() && r.kind.is_none() && (r.bytes.len() >= 8 || done) {
            match ImageKind::sniff(&r.bytes) {
                Some(k) => r.kind = Some(k),
                None => {
                    return Some(self.refuse(
                        c.rid,
                        ErrorCode::BadRequest,
                        "not a PNG or JPEG image",
                    ));
                }
            }
        }
        if !done {
            return None;
        }
        let r = self.current.take().expect("checked above");
        // a permission lowered while it arrived
        if !operates(shared, device) {
            return Some(self.refuse(c.rid, ErrorCode::Forbidden, VIEW_ONLY));
        }
        let delivered = match (r.ext.as_deref(), r.kind) {
            (Some(ext), _) => shared
                .source
                .upload_file(r.pane, &device.name, ext, &r.bytes),
            (None, Some(kind)) => shared.source.upload(r.pane, &device.name, kind, &r.bytes),
            (None, None) => unreachable!("an image is sniffed when done"),
        };
        Some(match delivered {
            Ok(()) => ServerMsg::ack_ok(c.rid),
            Err(e) => refused_ack(
                c.rid,
                e,
                upload_at_prompt(r.pane, r.ext.as_deref()),
                format!("no pane {}", r.pane),
            ),
        })
    }

    /// The image being received, dropped and answered `busy` once past its
    /// deadline (also between chunks: the session calls it every slice, so
    /// a trickle of chunks holds no memory past it).
    fn expire<S>(&mut self, shared: &Shared<S>, now: Instant) -> Option<ServerMsg> {
        let r = self.current.as_ref().filter(|r| now >= r.deadline)?;
        let rid = r.rid.clone();
        let noun = upload_noun(r.ext.as_deref());
        Some(self.refuse(
            rid,
            ErrorCode::Busy,
            upload_too_slow(noun, shared.config.upload_deadline),
        ))
    }

    /// Drop what arrived of `rid` and answer `code`; its later chunks go
    /// unanswered.
    fn refuse(&mut self, rid: String, code: ErrorCode, msg: impl Into<String>) -> ServerMsg {
        let ack = ServerMsg::ack_err(rid.clone(), code, msg);
        self.refuse_with(rid, ack)
    }

    /// [`Uploads::refuse`] with the ack made already.
    fn refuse_with(&mut self, rid: String, ack: ServerMsg) -> ServerMsg {
        self.current = None;
        self.refused = Some(rid);
        ack
    }
}

/// One of the gateway's [`MAX_UPLOADS`] places, or none while they are all
/// taken.
fn upload_slot(n: &AtomicUsize) -> Option<ConnGuard<'_>> {
    n.fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
        (v < MAX_UPLOADS).then_some(v + 1)
    })
    .ok()
    .map(|_| ConnGuard(n))
}

/// `fit`: permission and rate like `keys`, then the smallest size of the
/// clients following the pane goes to the source ([`sync_fits`]); it is
/// handed over on every `fit`, which is how the desktop's lease is kept.
#[allow(clippy::too_many_arguments)]
fn fit<S: Source>(
    shared: &Shared<S>,
    device: &Device,
    client: u64,
    rate: &mut Rate,
    rid: String,
    pane: u64,
    cols: Option<u16>,
    rows: Option<u16>,
) -> ServerMsg {
    if rid.chars().count() > MAX_REQUEST_ID {
        return ServerMsg::error(
            ErrorCode::BadRequest,
            format!("id is longer than {MAX_REQUEST_ID} characters"),
        );
    }
    if !operates(shared, device) {
        return ServerMsg::ack_err(rid, ErrorCode::Forbidden, VIEW_ONLY);
    }
    if !rate.allow(Instant::now()) {
        return ServerMsg::ack_err(rid, ErrorCode::Rate, "too many requests; slow down");
    }
    let size = match (cols, rows) {
        (Some(c), Some(r)) => Some((
            c.clamp(*FIT_COLS.start(), *FIT_COLS.end()),
            r.clamp(*FIT_ROWS.start(), *FIT_ROWS.end()),
        )),
        (None, None) => None,
        _ => {
            return ServerMsg::ack_err(
                rid,
                ErrorCode::BadRequest,
                "cols and rows are both numbers or both null",
            );
        }
    };
    {
        let mut hub = shared.hub.lock().unwrap();
        let Some(c) = hub.clients.get_mut(&client) else {
            return ServerMsg::ack_err(rid, ErrorCode::NoPane, format!("no pane {pane}"));
        };
        if let Some((cols, rows)) = size {
            if c.pane != Some(pane) {
                return ServerMsg::ack_err(
                    rid,
                    ErrorCode::BadRequest,
                    "subscribe to the pane first",
                );
            }
            c.fit = Some(Fit { pane, cols, rows });
        } else if c.fit.is_some_and(|f| f.pane == pane) {
            c.fit = None;
        }
    }
    match sync_fits(shared, Some(pane)) {
        Ok(()) => ServerMsg::ack_ok(rid),
        Err(e) => refused_ack(rid, e, "pane is busy".into(), format!("no pane {pane}")),
    }
}

/// Bring the source's pane sizes in line with the clients': per pane the
/// smallest columns and rows among the clients that follow it with a `fit`
/// (one that stopped following, or left, is released). The source hears
/// every change, a release (`None`) for a pane nobody holds any more, and
/// `force`'s pane even when unchanged (a refreshed lease). The result is of
/// `force`'s pane; a pane the source refuses drops its clients' fits.
fn sync_fits<S: Source>(shared: &Shared<S>, force: Option<u64>) -> Result<(), InputError> {
    let _serial = shared.fit_lock.lock().unwrap();
    let todo: Vec<(u64, Option<(u16, u16)>)> = {
        let mut hub = shared.hub.lock().unwrap();
        let mut want: HashMap<u64, (u16, u16)> = HashMap::new();
        for c in hub.clients.values_mut() {
            // a client that follows another pane (or none) lets go of its fit
            if c.fit.is_some_and(|f| c.pane != Some(f.pane)) {
                c.fit = None;
            }
        }
        for f in hub.clients.values().filter_map(|c| c.fit) {
            let w = want.entry(f.pane).or_insert((f.cols, f.rows));
            *w = (w.0.min(f.cols), w.1.min(f.rows));
        }
        let mut todo: Vec<_> = want
            .iter()
            .filter(|(p, size)| hub.fits.get(p) != Some(size) || force == Some(**p))
            .map(|(p, size)| (*p, Some(*size)))
            .collect();
        todo.extend(
            hub.fits
                .keys()
                .filter(|p| !want.contains_key(p))
                .map(|p| (*p, None)),
        );
        todo
    };
    let mut result = Ok(());
    for (pane, size) in todo {
        let r = shared.source.fit(pane, size);
        let mut hub = shared.hub.lock().unwrap();
        match (&r, size) {
            (Ok(()), Some(size)) => {
                hub.fits.insert(pane, size);
            }
            (Ok(()), None) => {
                hub.fits.remove(&pane);
            }
            (Err(_), _) => {
                hub.fits.remove(&pane);
                for c in hub.clients.values_mut() {
                    if c.fit.is_some_and(|f| f.pane == pane) {
                        c.fit = None;
                    }
                }
            }
        }
        if force == Some(pane) {
            result = r;
        }
    }
    result
}

/// `open`: permission and rate like `run`, then the source. On success
/// every connection gets the new topology before the ack, so the client
/// already knows the new pane when it goes there.
fn open<S: Source>(
    shared: &Shared<S>,
    device: &Device,
    rate: &mut Rate,
    rid: String,
    pane: u64,
    kind: OpenKind,
) -> ServerMsg {
    if rid.chars().count() > MAX_REQUEST_ID {
        return ServerMsg::error(
            ErrorCode::BadRequest,
            format!("id is longer than {MAX_REQUEST_ID} characters"),
        );
    }
    if !operates(shared, device) {
        return ServerMsg::ack_err(rid, ErrorCode::Forbidden, VIEW_ONLY);
    }
    if !rate.allow(Instant::now()) {
        return ServerMsg::ack_err(rid, ErrorCode::Rate, "too many requests; slow down");
    }
    opened(
        shared,
        rid,
        shared.source.open(pane, &device.name, kind),
        || format!("no pane {pane}"),
    )
}

/// `close`: permission and rate like `open`, then the source. On success
/// every connection gets the new topology before the ack.
fn close<S: Source>(
    shared: &Shared<S>,
    device: &Device,
    rate: &mut Rate,
    rid: String,
    pane: u64,
    kind: CloseKind,
) -> ServerMsg {
    if rid.chars().count() > MAX_REQUEST_ID {
        return ServerMsg::error(
            ErrorCode::BadRequest,
            format!("id is longer than {MAX_REQUEST_ID} characters"),
        );
    }
    if !operates(shared, device) {
        return ServerMsg::ack_err(rid, ErrorCode::Forbidden, VIEW_ONLY);
    }
    if !rate.allow(Instant::now()) {
        return ServerMsg::ack_err(rid, ErrorCode::Rate, "too many requests; slow down");
    }
    match shared.source.close(pane, &device.name, kind) {
        Ok(()) => {
            push_topology(shared);
            ServerMsg::ack_ok(rid)
        }
        Err(e) => opened(shared, rid, Err(e), || format!("no pane {pane}")),
    }
}

/// `move`: permission as `open`, rate like `fit` (a screen reader's Move up /
/// down repeats quickly), then the source. On success every connection gets
/// the new topology before the ack.
fn move_to<S: Source>(
    shared: &Shared<S>,
    device: &Device,
    rate: &mut Rate,
    rid: String,
    pane: u64,
    kind: MoveKind,
    to: usize,
) -> ServerMsg {
    if rid.chars().count() > MAX_REQUEST_ID {
        return ServerMsg::error(
            ErrorCode::BadRequest,
            format!("id is longer than {MAX_REQUEST_ID} characters"),
        );
    }
    if !operates(shared, device) {
        return ServerMsg::ack_err(rid, ErrorCode::Forbidden, VIEW_ONLY);
    }
    if !rate.allow(Instant::now()) {
        return ServerMsg::ack_err(rid, ErrorCode::Rate, "too many requests; slow down");
    }
    match shared.source.move_to(pane, &device.name, kind, to) {
        Ok(()) => {
            push_topology(shared);
            ServerMsg::ack_ok(rid)
        }
        Err(e) => opened(shared, rid, Err(e), || format!("no pane {pane}")),
    }
}

/// `start`: permission and rate like `open`, then the source (which
/// refuses unless the computer allows shells while Rift is closed).
fn start<S: Source>(
    shared: &Shared<S>,
    device: &Device,
    rate: &mut Rate,
    rid: String,
) -> ServerMsg {
    if rid.chars().count() > MAX_REQUEST_ID {
        return ServerMsg::error(
            ErrorCode::BadRequest,
            format!("id is longer than {MAX_REQUEST_ID} characters"),
        );
    }
    if !operates(shared, device) {
        return ServerMsg::ack_err(rid, ErrorCode::Forbidden, VIEW_ONLY);
    }
    if !rate.allow(Instant::now()) {
        return ServerMsg::ack_err(rid, ErrorCode::Rate, "too many requests; slow down");
    }
    opened(shared, rid, shared.source.start(&device.name), || {
        "no shell started".into()
    })
}

/// The ack of `open` and `start`: on success every connection gets the new
/// topology first.
fn opened<S: Source>(
    shared: &Shared<S>,
    rid: String,
    result: Result<u64, InputError>,
    no_pane: impl FnOnce() -> String,
) -> ServerMsg {
    match result {
        Ok(new) => {
            push_topology(shared);
            ServerMsg::ack_opened(rid, new)
        }
        Err(e) => {
            let no_pane = if matches!(e, InputError::NoPane) {
                no_pane()
            } else {
                String::new()
            };
            refused_ack(rid, e, "the desktop is busy".into(), no_pane)
        }
    }
}

/// Send the source's topology to every connection now when it changed
/// (the poll loop's, without waiting for the next poll).
fn push_topology<S: Source>(shared: &Shared<S>) {
    let windows = crate::bound_topology(shared.source.topology());
    let f = frame(&ServerMsg::Topology { windows });
    let mut hub = shared.hub.lock().unwrap();
    if hub.topology.as_ref() != Some(&f) {
        for c in hub.clients.values() {
            c.push(f.clone());
        }
        hub.topology = Some(f);
    }
}

/// The `forbidden` ack's message for a view device.
const VIEW_ONLY: &str = "this device may only view";

/// A client message's type alone: what tells an `upload` chunk, the only
/// message that may pass [`MAX_CLIENT_MESSAGE`], before it is parsed.
#[derive(serde::Deserialize)]
struct MsgType<'a> {
    t: &'a str,
}

/// `device` may operate, by the permission it connected with and the one
/// as of now (the connection closes within a second of a change, but
/// nothing runs in between on the old one). The file is read again only
/// when its stamp changed (every write replaces it).
fn operates<S>(shared: &Shared<S>, device: &Device) -> bool {
    if device.permission != Permission::Operate {
        return false;
    }
    let dir = &shared.config.dir;
    let Some(stamp) = registry::devices_stamp(dir) else {
        return false;
    };
    let mut cache = shared.permissions.lock().unwrap();
    if cache.as_ref().is_none_or(|(s, _)| *s != stamp) {
        // read, then stamped as of before the read: a write in between
        // only makes the next call read again
        *cache = registry::devices(dir).ok().map(|all| (stamp, all));
    }
    cache
        .as_ref()
        .and_then(|(_, all)| all.iter().find(|d| d.id == device.id))
        .is_some_and(|d| d.permission == Permission::Operate)
}

/// A `run` command: one line, no control characters, at most
/// [`MAX_COMMAND`] bytes, not blank.
fn check_command(command: &str) -> Result<(), (ErrorCode, String)> {
    if command.len() > MAX_COMMAND {
        return Err((
            ErrorCode::TooLarge,
            format!("the command is longer than {MAX_COMMAND} bytes"),
        ));
    }
    if command.chars().any(char::is_control) {
        return Err((
            ErrorCode::BadRequest,
            "the command may not contain a newline or a control character".into(),
        ));
    }
    if command.trim().is_empty() {
        return Err((ErrorCode::BadRequest, "the command is empty".into()));
    }
    Ok(())
}

/// A `text`: one line without control characters (C0, DEL, C1), 1 to
/// [`MAX_TEXT`] bytes.
fn check_text(text: &str) -> Result<(), (ErrorCode, String)> {
    if text.len() > MAX_TEXT {
        return Err((
            ErrorCode::TooLarge,
            format!("the text is longer than {MAX_TEXT} bytes"),
        ));
    }
    if text.chars().any(char::is_control) {
        return Err((
            ErrorCode::BadRequest,
            "the text may not contain a control character (send enter, tab, … as keys)".into(),
        ));
    }
    if text.is_empty() {
        return Err((ErrorCode::BadRequest, "the text is empty".into()));
    }
    Ok(())
}

/// A `paste`: 1 to [`MAX_PASTE`] bytes, with line breaks and tabs the only
/// control characters (no escape sequence can leave the paste).
fn check_paste(text: &str) -> Result<(), (ErrorCode, String)> {
    if text.len() > MAX_PASTE {
        return Err((
            ErrorCode::TooLarge,
            format!("the text is longer than {MAX_PASTE} bytes"),
        ));
    }
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err((
            ErrorCode::BadRequest,
            "a paste may hold only line breaks and tabs as control characters".into(),
        ));
    }
    if text.is_empty() {
        return Err((ErrorCode::BadRequest, "the text is empty".into()));
    }
    Ok(())
}

/// `screen`: set the mode of this connection's subscription. `off` sends
/// `screen_end` (after any screen already written); `auto` / `on` start
/// over with a full screen, and a screen the client shows ends when the
/// new mode has none (`on` to `auto` on the primary screen).
fn set_screen<S>(shared: &Shared<S>, id: u64, pane: u64, mode: ScreenMode) -> Option<ServerMsg> {
    let mut hub = shared.hub.lock().unwrap();
    let c = hub.clients.get_mut(&id)?;
    // the `subscribe` just before it failed (a pane that closed while the
    // app was away) and already said `no_pane`
    if c.pane.is_none() && c.refused == Some(pane) {
        return None;
    }
    if c.pane != Some(pane) {
        return Some(ServerMsg::error(
            ErrorCode::BadRequest,
            format!("pane {pane} is not the subscribed pane"),
        ));
    }
    let was = std::mem::replace(&mut c.screen, mode);
    let showing = c.out.showing;
    c.out.restart(mode != ScreenMode::Off && showing);
    if mode == ScreenMode::Off && was != ScreenMode::Off {
        c.push(frame(&ServerMsg::ScreenEnd { pane }));
    }
    None
}

/// `output`: rate, validation, then the source; the page bounded again
/// here ([`crate::bound_page`]). Any device may page.
fn output<S: Source>(
    shared: &Shared<S>,
    rate: &mut Rate,
    rid: String,
    pane: u64,
    block: u64,
    from: u64,
    limit: usize,
) -> ServerMsg {
    if rid.chars().count() > MAX_REQUEST_ID {
        return ServerMsg::error(
            ErrorCode::BadRequest,
            format!("id is longer than {MAX_REQUEST_ID} characters"),
        );
    }
    if !rate.allow(Instant::now()) {
        return ServerMsg::ack_err(rid, ErrorCode::Rate, "too many requests; slow down");
    }
    if !(1..=MAX_OUTPUT_PAGE_LINES).contains(&limit) {
        return ServerMsg::ack_err(
            rid,
            ErrorCode::BadRequest,
            format!("limit is 1 to {MAX_OUTPUT_PAGE_LINES} lines"),
        );
    }
    match shared.source.output(pane, block, from, limit) {
        Ok(page) => ServerMsg::Output {
            id: rid,
            pane,
            block,
            from: page.from,
            text: crate::bound_page(&page.text, limit).to_string(),
            total: page.total,
        },
        Err(OutputError::NoBlock) => ServerMsg::ack_err(
            rid,
            ErrorCode::NoBlock,
            format!("block {block} is not in pane {pane}"),
        ),
        Err(OutputError::NoPane) => {
            ServerMsg::ack_err(rid, ErrorCode::NoPane, format!("no pane {pane}"))
        }
        Err(OutputError::Unavailable(e)) => ServerMsg::ack_err(
            rid,
            ErrorCode::NoPane,
            format!("pane {pane} unavailable: {e}"),
        ),
    }
}

/// The `ack` for a chip request the window could not answer.
fn chip_error(rid: String, pane: u64, e: OutputError) -> ServerMsg {
    match e {
        OutputError::NoBlock => {
            ServerMsg::ack_err(rid, ErrorCode::NoBlock, "no such change in the repository")
        }
        OutputError::NoPane => {
            ServerMsg::ack_err(rid, ErrorCode::NoPane, format!("no pane {pane}"))
        }
        OutputError::Unavailable(e) => ServerMsg::ack_err(
            rid,
            ErrorCode::NoPane,
            format!("pane {pane} unavailable: {e}"),
        ),
    }
}

/// The id check and the rate (`output`'s) every chip request goes through.
fn chip_admit(rid: &str, rate: &mut Rate) -> Option<ServerMsg> {
    if rid.chars().count() > MAX_REQUEST_ID {
        return Some(ServerMsg::error(
            ErrorCode::BadRequest,
            format!("id is longer than {MAX_REQUEST_ID} characters"),
        ));
    }
    if !rate.allow(Instant::now()) {
        return Some(ServerMsg::ack_err(
            rid,
            ErrorCode::Rate,
            "too many requests; slow down",
        ));
    }
    None
}

/// `dirs` / `branches` / `changes`: a prompt chip's menu, bounded.
fn menu<S: Source>(
    shared: &Shared<S>,
    rate: &mut Rate,
    rid: String,
    pane: u64,
    kind: MenuKind,
) -> ServerMsg {
    if let Some(refused) = chip_admit(&rid, rate) {
        return refused;
    }
    match shared.source.menu(pane, kind) {
        Ok(m) => match (kind, crate::bound_menu(m)) {
            (MenuKind::Dirs, (Menu::Dirs { cwd, parent, dirs }, more)) => ServerMsg::Dirs {
                id: rid,
                pane,
                cwd,
                parent,
                dirs,
                more,
            },
            (MenuKind::Branches, (Menu::Branches { current, branches }, more)) => {
                ServerMsg::Branches {
                    id: rid,
                    pane,
                    current,
                    branches,
                    more,
                }
            }
            (MenuKind::Changes, (Menu::Changes { branch, files }, more)) => ServerMsg::Changes {
                id: rid,
                pane,
                branch,
                files,
                more,
            },
            (kind, (Menu::Chip { detail }, more)) if chip_kind_of(&detail) == Some(kind) => {
                ServerMsg::Chip {
                    id: rid,
                    pane,
                    detail,
                    more,
                }
            }
            _ => ServerMsg::ack_err(
                rid,
                ErrorCode::NoPane,
                format!("pane {pane} answered another menu"),
            ),
        },
        Err(e) => chip_error(rid, pane, e),
    }
}

/// The `chip` request's kind an answer is of.
fn chip_kind_of(detail: &crate::protocol::ChipDetail) -> Option<MenuKind> {
    use crate::protocol::ChipDetail as D;
    Some(match detail {
        D::Pr { .. } => MenuKind::Pr,
        D::Python { .. } => MenuKind::Python,
        D::Php { .. } => MenuKind::Php,
        D::Docker { .. } => MenuKind::Docker,
        D::Elapsed { .. } => MenuKind::Elapsed,
    })
}

/// `chip`: the menu of a pull request, interpreter, Docker or Time on tab
/// chip (the folder, branch and changes chips have their own requests).
fn chip<S: Source>(
    shared: &Shared<S>,
    rate: &mut Rate,
    rid: String,
    pane: u64,
    kind: MenuKind,
) -> ServerMsg {
    if matches!(
        kind,
        MenuKind::Dirs | MenuKind::Branches | MenuKind::Changes
    ) {
        return ServerMsg::ack_err(
            rid,
            ErrorCode::BadRequest,
            "kind is pr, python, php, docker or elapsed",
        );
    }
    menu(shared, rate, rid, pane, kind)
}

/// `reset_elapsed`: permission and rate like `open`, then the source.
fn reset_elapsed<S: Source>(
    shared: &Shared<S>,
    device: &Device,
    rate: &mut Rate,
    rid: String,
    pane: u64,
) -> ServerMsg {
    if rid.chars().count() > MAX_REQUEST_ID {
        return ServerMsg::error(
            ErrorCode::BadRequest,
            format!("id is longer than {MAX_REQUEST_ID} characters"),
        );
    }
    if !operates(shared, device) {
        return ServerMsg::ack_err(rid, ErrorCode::Forbidden, VIEW_ONLY);
    }
    if !rate.allow(Instant::now()) {
        return ServerMsg::ack_err(rid, ErrorCode::Rate, "too many requests; slow down");
    }
    match shared.source.reset_elapsed(pane, &device.name) {
        Ok(()) => {
            push_topology(shared);
            ServerMsg::ack_ok(rid)
        }
        Err(e) => opened(shared, rid, Err(e), || format!("no pane {pane}")),
    }
}

/// `complete`: the desktop's completions and history for a command line,
/// bounded. Operate devices only: history can hold secrets typed on a
/// command line, and path completion lists any folder.
fn complete<S: Source>(
    shared: &Shared<S>,
    device: &Device,
    rate: &mut Rate,
    rid: String,
    pane: u64,
    line: String,
    cursor: usize,
) -> ServerMsg {
    if let Some(refused) = chip_admit(&rid, rate) {
        return refused;
    }
    if !operates(shared, device) {
        return ServerMsg::ack_err(rid, ErrorCode::Forbidden, VIEW_ONLY);
    }
    if line.len() > crate::MAX_COMPLETE_LINE {
        return ServerMsg::ack_err(
            rid,
            ErrorCode::BadRequest,
            format!("line is longer than {} bytes", crate::MAX_COMPLETE_LINE),
        );
    }
    if !line.is_char_boundary(cursor) {
        return ServerMsg::ack_err(
            rid,
            ErrorCode::BadRequest,
            "cursor is outside the line or inside a character",
        );
    }
    match shared.source.complete(pane, &line, cursor) {
        Ok(page) => {
            let (page, more) = crate::bound_complete(page);
            ServerMsg::Complete {
                id: rid,
                pane,
                items: page.items,
                history: page.history,
                more,
            }
        }
        Err(e) => chip_error(rid, pane, e),
    }
}

/// `diff`: one changed file's lines, bounded. An untracked file's lines
/// (its whole content: a `.env`, a key) go to operate devices only.
fn diff<S: Source>(
    shared: &Shared<S>,
    device: &Device,
    rate: &mut Rate,
    rid: String,
    pane: u64,
    path: String,
    staged: bool,
) -> ServerMsg {
    if let Some(refused) = chip_admit(&rid, rate) {
        return refused;
    }
    if path.is_empty() || path.len() > MAX_DIFF_PATH {
        return ServerMsg::ack_err(
            rid,
            ErrorCode::BadRequest,
            format!("path is 1 to {MAX_DIFF_PATH} bytes"),
        );
    }
    match shared.source.diff(pane, &path, staged) {
        Ok(page) if page.untracked && !operates(shared, device) => ServerMsg::ack_err(
            rid,
            ErrorCode::Forbidden,
            "an untracked file's content needs a device that may operate",
        ),
        Ok(page) => {
            let binary = page.binary;
            let (lines, truncated) = crate::bound_diff(page);
            ServerMsg::Diff {
                id: rid,
                pane,
                path,
                staged,
                binary,
                truncated,
                lines,
            }
        }
        Err(e) => chip_error(rid, pane, e),
    }
}

/// `keys` names: 1 to [`MAX_KEYS`], each one of [`Key`]'s.
fn parse_keys(names: &[String]) -> Result<Vec<Key>, (ErrorCode, String)> {
    if names.len() > MAX_KEYS {
        return Err((
            ErrorCode::TooLarge,
            format!("more than {MAX_KEYS} keys in one message"),
        ));
    }
    if names.is_empty() {
        return Err((ErrorCode::BadRequest, "no keys".into()));
    }
    names
        .iter()
        .map(|n| {
            Key::parse(n).ok_or_else(|| {
                let shown: String = n.chars().take(32).collect();
                (ErrorCode::BadRequest, format!("unknown key {shown:?}"))
            })
        })
        .collect()
}

/// Follow `pane`: queue a replay or a snapshot, or answer with an error.
fn subscribe<S: Source>(
    shared: &Shared<S>,
    id: u64,
    pane: u64,
    after: Option<u64>,
) -> Option<ServerMsg> {
    let fresh = refresh(shared, pane);
    let mut guard = shared.hub.lock().unwrap();
    let hub = &mut *guard;
    let Some(feed) = hub
        .feeds
        .get(&pane)
        .filter(|_| fresh != Err(SourceError::NoPane))
    else {
        if let Some(c) = hub.clients.get_mut(&id) {
            c.follow(None);
            c.refused = Some(pane);
        }
        let why = match fresh {
            Err(SourceError::Unavailable(e)) => format!("pane {pane} unavailable: {e}"),
            _ => format!("no pane {pane}"),
        };
        return Some(ServerMsg::error(ErrorCode::NoPane, why));
    };
    let frames = after
        .and_then(|a| feed.replay(a))
        .unwrap_or_else(|| vec![feed.snapshot(pane)]);
    let c = hub.clients.get_mut(&id)?;
    c.follow(Some(pane));
    for f in frames {
        c.push(f);
    }
    None
}

fn older<S: Source>(shared: &Shared<S>, pane: u64, before: u64) -> ServerMsg {
    match shared.source.blocks(pane, Some(before), MAX_BLOCKS) {
        Ok(Page {
            mut blocks,
            has_older,
            ..
        }) => {
            crate::bound_blocks(&mut blocks);
            let (blocks, trimmed) = fit_newest(&blocks, |blocks| ServerMsg::Older {
                pane,
                blocks,
                has_older: true,
            });
            // a paged block is reported when it is trimmed later
            if let Some(feed) = shared.hub.lock().unwrap().feeds.get_mut(&pane) {
                feed.track(blocks.iter().map(|b| b.id));
            }
            ServerMsg::Older {
                pane,
                blocks,
                has_older: has_older || trimmed,
            }
        }
        Err(SourceError::NoPane) => ServerMsg::error(ErrorCode::NoPane, format!("no pane {pane}")),
        Err(SourceError::Unavailable(e)) => {
            ServerMsg::error(ErrorCode::NoPane, format!("pane {pane} unavailable: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(id: u64, output: &str) -> Block {
        Block {
            id,
            command: format!("cmd {id}"),
            state: BlockState::Done,
            exit: Some(0),
            duration_ms: 1,
            cwd: None,
            branch: None,
            output: output.into(),
            dropped: 0,
            restored: false,
        }
    }

    #[test]
    fn diff_upserts_changes_and_reports_removals() {
        let mut feed = Feed::new(page(vec![block(1, "a"), block(2, "b")], false));
        assert!(
            feed.apply(5, page(vec![block(1, "a"), block(2, "b")], false))
                .is_empty()
        );
        let frames = feed.apply(5, page(vec![block(2, "b2"), block(3, "c")], false));
        assert_eq!(frames.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&frames[0]).unwrap();
        assert_eq!(v["seq"], 1);
        assert_eq!(v["removed"], serde_json::json!([1]));
        let ids: Vec<u64> = v["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["id"].as_u64().unwrap())
            .collect();
        assert_eq!(ids, [2, 3]);
        // paged out below the window (older ones exist): not removed
        let frames = feed.apply(5, page(vec![block(3, "c"), block(4, "d")], true));
        let v: serde_json::Value = serde_json::from_str(&frames[0]).unwrap();
        assert_eq!(v["removed"], serde_json::json!([]));
        // trimmed below the pane's oldest block: removed, and only once
        let mut p = page(vec![block(3, "c"), block(4, "d")], true);
        p.oldest = Some(3);
        let v: serde_json::Value = serde_json::from_str(&feed.apply(5, p.clone())[0]).unwrap();
        assert_eq!(v["removed"], serde_json::json!([2]));
        assert!(feed.apply(5, p).is_empty());
    }

    /// A page whose oldest block is the first one, or older when `has_older`.
    fn page(blocks: Vec<Block>, has_older: bool) -> Page {
        let oldest = if has_older {
            Some(0)
        } else {
            blocks.first().map(|b| b.id)
        };
        Page {
            blocks,
            has_older,
            oldest,
        }
    }

    #[test]
    fn tracked_ids_are_bounded() {
        let mut feed = Feed::new(page(vec![block(u64::MAX, "")], true));
        feed.track(0..(MAX_TRACKED as u64 * 2));
        assert_eq!(feed.known.len(), MAX_TRACKED);
        assert!(feed.known.contains(&u64::MAX), "the window is kept");
        assert!(!feed.known.contains(&0), "the lowest go first");
    }

    #[test]
    fn a_running_clock_alone_is_not_a_change() {
        let mut running = block(7, "1\n");
        running.state = BlockState::Running;
        running.exit = None;
        let mut feed = Feed::new(page(vec![running.clone()], false));
        running.duration_ms = 900;
        assert!(feed.apply(1, page(vec![running.clone()], false)).is_empty());
        assert_eq!(feed.snapshot(1).matches("\"duration_ms\":900").count(), 1);
        running.output = "1\n2\n".into();
        assert_eq!(feed.apply(1, page(vec![running.clone()], false)).len(), 1);
        // finishing changes the state: an update with the final duration
        running.state = BlockState::Done;
        running.exit = Some(0);
        assert_eq!(feed.apply(1, page(vec![running], false)).len(), 1);
    }

    #[test]
    fn replay_needs_a_contiguous_ring() {
        let mut feed = Feed::new(page(Vec::new(), false));
        for i in 0..(REPLAY_RING as u64 + 10) {
            feed.apply(1, page(vec![block(1, &i.to_string())], false));
        }
        assert_eq!(feed.seq, REPLAY_RING as u64 + 10);
        assert_eq!(feed.replay(feed.seq).unwrap().len(), 0);
        assert_eq!(feed.replay(feed.seq - 3).unwrap().len(), 3);
        assert_eq!(feed.replay(10).unwrap().len(), REPLAY_RING);
        assert!(feed.replay(9).is_none(), "seq 10 fell out of the ring");
        assert!(
            feed.replay(feed.seq + 1).is_none(),
            "from another gateway run"
        );
    }

    #[test]
    fn large_updates_and_snapshots_fit_a_frame() {
        let big = "x".repeat(crate::MAX_OUTPUT_BYTES);
        let blocks: Vec<Block> = (1..=30).map(|i| block(i, &big)).collect();
        let mut feed = Feed::new(page(Vec::new(), false));
        let frames = feed.apply(1, page(blocks, false));
        assert!(frames.len() > 1);
        assert!(frames.iter().all(|f| f.len() <= MAX_GATEWAY_MESSAGE));
        let total: usize = frames
            .iter()
            .map(|f| {
                let v: serde_json::Value = serde_json::from_str(f).unwrap();
                v["blocks"].as_array().unwrap().len()
            })
            .sum();
        assert_eq!(total, 30);
        assert_eq!(feed.seq, frames.len() as u64);
        let snap = feed.snapshot(1);
        assert!(snap.len() <= MAX_GATEWAY_MESSAGE);
        let v: serde_json::Value = serde_json::from_str(&snap).unwrap();
        assert_eq!(v["has_older"], true);
        assert_eq!(v["blocks"].as_array().unwrap().last().unwrap()["id"], 30);
    }

    fn running(id: u64) -> Block {
        Block {
            state: BlockState::Running,
            exit: None,
            ..block(id, "")
        }
    }

    fn done_ids(msgs: &[ServerMsg]) -> Vec<u64> {
        msgs.iter()
            .map(|m| match m {
                ServerMsg::Done { block, .. } => *block,
                other => panic!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn done_is_reported_once_including_commands_never_seen_running() {
        let mut t = DoneTracker::default();
        // a fresh pane: its first command is block 0
        assert!(t.observe(7, &[]).is_empty());
        assert_eq!(done_ids(&t.observe(7, &[block(0, "")])), [0]);
        assert!(t.observe(7, &[block(0, "")]).is_empty());
        // first sight: history is not reported
        assert!(
            t.observe(1, &[block(1, ""), block(2, ""), running(3)])
                .is_empty()
        );
        // running → done
        let d = t.observe(1, &[block(2, ""), block(3, "")]);
        assert_eq!(done_ids(&d), [3]);
        assert!(
            t.observe(1, &[block(2, ""), block(3, "")]).is_empty(),
            "once"
        );
        // started and finished between two reads: new id, already done
        let d = t.observe(1, &[block(3, ""), block(4, ""), block(5, ""), running(6)]);
        assert_eq!(done_ids(&d), [4, 5]);
        match &d[0] {
            ServerMsg::Done {
                pane,
                command,
                exit,
                duration_ms,
                ..
            } => assert_eq!(
                (pane, command.as_str(), exit, duration_ms),
                (&1, "cmd 4", &Some(0), &1)
            ),
            _ => unreachable!(),
        }
        // a page that no longer shows the running block keeps it pending
        assert!(t.observe(1, &[block(5, "")]).is_empty());
        assert_eq!(done_ids(&t.observe(1, &[block(6, "")])), [6]);
        // restored blocks are never reported
        let mut r = block(9, "");
        r.restored = true;
        assert!(t.observe(1, &[r]).is_empty());
        // panes are independent; a gone pane starts over
        assert!(t.observe(2, &[block(100, "")]).is_empty());
        t.retain(&HashSet::from([2]));
        assert!(t.observe(1, &[block(50, "")]).is_empty());
    }

    #[test]
    fn rates_and_failures_are_windowed() {
        let t0 = Instant::now();
        let mut r = Rate::new(4);
        assert!((0..4).all(|_| r.allow(t0)));
        assert!(!r.allow(t0 + Duration::from_millis(999)));
        assert!(r.allow(t0 + Duration::from_secs(1)));
        let ip: IpAddr = "10.0.0.9".parse().unwrap();
        let other: IpAddr = "10.0.0.8".parse().unwrap();
        let (w, refuse) = (Duration::from_secs(60), Duration::from_secs(60));
        let mut f = Failures::default();
        for i in 0..9 {
            f.fail(ip, t0 + Duration::from_secs(i), w, refuse);
        }
        assert!(!f.refused(ip, t0 + Duration::from_secs(9)));
        // the 10th within a minute
        f.fail(ip, t0 + Duration::from_secs(9), w, refuse);
        assert!(f.refused(ip, t0 + Duration::from_secs(10)));
        assert!(!f.refused(other, t0 + Duration::from_secs(10)));
        assert!(!f.refused(ip, t0 + Duration::from_secs(70)), "for a minute");
        // failures spread over more than a minute never add up
        let mut f = Failures::default();
        for i in 0..20 {
            f.fail(ip, t0 + Duration::from_secs(i * 7), w, refuse);
            assert!(!f.refused(ip, t0 + Duration::from_secs(i * 7)), "{i}");
        }
    }

    #[test]
    fn feeds_of_gone_panes_are_forgotten_unless_followed() {
        let mut hub = Hub::default();
        for pane in [1, 2, 3] {
            hub.feeds
                .insert(pane, Feed::new(page(vec![block(1, "a")], false)));
        }
        // pane 2 closed while nobody followed it; pane 3 is missing from
        // this topology read but a client follows it
        hub.forget_feeds(&HashSet::from([1]), &HashSet::from([3]));
        let mut kept: Vec<u64> = hub.feeds.keys().copied().collect();
        kept.sort_unstable();
        assert_eq!(kept, vec![1, 3]);
    }

    #[test]
    fn an_unstyled_screen_fits_the_limit_and_is_what_the_client_has() {
        // two cells each carrying 300k combining marks (a source that does
        // not cap them): the rows are cut to fit
        let mut big = String::from("a");
        big.extend(std::iter::repeat_n('\u{0301}', 300_000));
        let s = Screen {
            cols: 80,
            rows: 2,
            alt: true,
            cursor: None,
            fg: None,
            bg: None,
            rules: Vec::new(),
            lines: vec![vec![Run::plain(big.clone())], vec![Run::plain(big)]],
            age_ms: None,
        };
        let (f, plain) = screen_frame(1, None, &s).unwrap();
        assert!(f.len() <= MAX_GATEWAY_MESSAGE, "{} bytes", f.len());
        let plain = plain.expect("sent unstyled");
        assert!(plain.lines[1][0].text.starts_with("a\u{301}\u{301}"));
        // the client has that: the same screen again is no frame
        assert_eq!(screen_frame(1, Some(&plain), &s), None);
        // every character JSON escapes longer still fits
        let quotes = Screen {
            lines: vec![vec![Run::plain("\"\u{1}".repeat(400_000))]; 2],
            ..s.clone()
        };
        let (f, _) = screen_frame(1, None, &quotes).unwrap();
        assert!(f.len() <= MAX_GATEWAY_MESSAGE, "{} bytes", f.len());
        assert!(serde_json::from_str::<serde_json::Value>(&f).is_ok());
    }
}
