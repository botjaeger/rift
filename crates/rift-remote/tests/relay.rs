//! The relay over real localhost sockets: routing, limits, splicing, and
//! a real gateway reached through it (the test acts as the phone). The
//! opacity test taps every frame the relay forwards and checks that no
//! plaintext of a known message passes through.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use rift_remote::protocol::{
    Block, BlockState, ClientHello, Pane, Permission, RelayError, RelayMsg, Tab, Window,
};
use rift_remote::registry::{self, RelayState, Via};
use rift_remote::relay::{Leg, Relay, RelayConfig, serve_relay};
use rift_remote::transport::{self, Channel, HandshakeError, Recv};
use rift_remote::{Config, Gateway, Input, InputError, Page, Source, SourceError, serve};
use serde_json::{Value, json};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

type Ws = WebSocket<TcpStream>;

/// A gateway key ([7; 32]) and its routing id; [8; 32] and [9; 32] for others.
const K7: [u8; 32] = [7; 32];
const ID: &str = "D20lU_8MD0R64gLt2EOD4Q";
const K8: [u8; 32] = [8; 32];
const ID8: &str = "2U12mkdCIJEC1i9llxDb_A";
const K9: [u8; 32] = [9; 32];
const ID9: &str = "BxDn3oghGeMxYQyscgwbFQ";
const SECRET: &str = "q83vEjRWeJCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJA";
const OTHER_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn start_relay(tweak: impl FnOnce(&mut RelayConfig)) -> Relay {
    let mut cfg = RelayConfig::default();
    tweak(&mut cfg);
    serve_relay(TcpListener::bind("127.0.0.1:0").unwrap(), cfg).unwrap()
}

/// A WebSocket client to `path` (reads time out after 50 ms).
fn open(addr: SocketAddr, path: &str) -> Result<Ws, tungstenite::Error> {
    open_with(addr, path, &[])
}

fn open_with(
    addr: SocketAddr,
    path: &str,
    headers: &[(&str, &str)],
) -> Result<Ws, tungstenite::Error> {
    use tungstenite::client::IntoClientRequest;
    let stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let mut req = format!("ws://{addr}{path}").into_client_request().unwrap();
    for (k, v) in headers {
        let name = tungstenite::http::HeaderName::from_bytes(k.as_bytes()).unwrap();
        req.headers_mut().insert(name, v.parse().unwrap());
    }
    tungstenite::client(req, stream)
        .map(|(ws, _)| ws)
        .map_err(|e| match e {
            tungstenite::HandshakeError::Failure(e) => e,
            tungstenite::HandshakeError::Interrupted(_) => panic!("interrupted"),
        })
}

/// The HTTP status a refused upgrade got.
fn status(r: Result<Ws, tungstenite::Error>) -> u16 {
    match r {
        Err(tungstenite::Error::Http(resp)) => resp.status().as_u16(),
        Err(e) => panic!("expected an HTTP refusal, got {e}"),
        Ok(_) => panic!("expected an HTTP refusal, got a WebSocket"),
    }
}

fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

/// The next text or binary message within 3 s.
fn next(ws: &mut Ws) -> Message {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(Instant::now() < deadline, "no message within 3 s");
        match ws.read() {
            Ok(m @ (Message::Text(_) | Message::Binary(_) | Message::Close(_))) => return m,
            Ok(_) => {}
            Err(e) if would_block(&e) => {}
            Err(e) => panic!("socket ended: {e}"),
        }
    }
}

fn next_relay(ws: &mut Ws) -> RelayMsg {
    match next(ws) {
        Message::Text(t) => serde_json::from_str(t.as_str()).unwrap(),
        other => panic!("expected a control message, got {other:?}"),
    }
}

/// The socket is closed within `within`.
fn closed_within(ws: &mut Ws, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        match ws.read() {
            Ok(Message::Close(_)) => return true,
            Ok(_) => {}
            Err(e) if would_block(&e) => {}
            Err(_) => return true,
        }
    }
    false
}

fn key_text(private: &[u8; 32]) -> String {
    registry::encode_key(&transport::public_key(private))
}

/// Register the gateway key `private` with `secret`, answering the key
/// challenge: the relay's reply after it.
fn register(addr: SocketAddr, private: &[u8; 32], secret: &str) -> (Ws, RelayMsg) {
    let mut ws = open(addr, "/gateway").unwrap();
    let id = registry::routing_id(&transport::public_key(private));
    let key = key_text(private);
    ws.send(Message::text(
        json!({"t":"register","id":id,"key":key,"secret":secret}).to_string(),
    ))
    .unwrap();
    let RelayMsg::Prove { m } = next_relay(&mut ws) else {
        panic!("expected a key challenge")
    };
    let proof = rift_remote::relay::prove(private, &B64.decode(m).unwrap()).unwrap();
    ws.send(Message::text(
        json!({"t":"proof","m":B64.encode(proof)}).to_string(),
    ))
    .unwrap();
    let reply = next_relay(&mut ws);
    (ws, reply)
}

/// `/accept` for `conn` of `id` with `secret` in the Authorization header.
fn accept(addr: SocketAddr, id: &str, conn: &str, secret: &str) -> Result<Ws, tungstenite::Error> {
    open_with(
        addr,
        &format!("/accept?id={id}&conn={conn}"),
        &[("authorization", &format!("Bearer {secret}"))],
    )
}

fn wait_until(what: &str, within: Duration, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + within;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A phone on `/connect` and the gateway's `/accept`, spliced.
fn pair(relay: &Relay, control: &mut Ws) -> (Ws, Ws) {
    let addr = relay.local_addr();
    let phone = open(addr, &format!("/connect?id={ID}")).unwrap();
    let RelayMsg::Open { conn } = next_relay(control) else {
        panic!("expected open")
    };
    let gw = accept(addr, ID, &conn, SECRET).unwrap();
    (phone, gw)
}

fn binary(ws: &mut Ws) -> Vec<u8> {
    match next(ws) {
        Message::Binary(b) => b.to_vec(),
        other => panic!("expected binary, got {other:?}"),
    }
}

// --- registration ----------------------------------------------------------

#[test]
fn only_the_holder_of_the_key_registers_its_id() {
    let relay = start_relay(|_| {});
    let addr = relay.local_addr();
    assert!(!relay.online(ID));
    let (mut a, reply) = register(addr, &K7, SECRET);
    assert_eq!(reply, RelayMsg::Registered);
    assert!(relay.online(ID));
    // someone who knows the key (it is in every pairing link) but not its
    // private half: challenged, cannot answer, closed; the gateway stays
    let mut squat = open(addr, "/gateway").unwrap();
    squat
        .send(Message::text(
            json!({"t":"register","id":ID,"key":key_text(&K7),"secret":OTHER_SECRET}).to_string(),
        ))
        .unwrap();
    let RelayMsg::Prove { .. } = next_relay(&mut squat) else {
        panic!("expected a key challenge")
    };
    let other = rift_remote::relay::challenge(&transport::public_key(&K7))
        .unwrap()
        .1;
    for m in [vec![0u8; 48], other] {
        let mut s = open(addr, "/gateway").unwrap();
        s.send(Message::text(
            json!({"t":"register","id":ID,"key":key_text(&K7),"secret":OTHER_SECRET}).to_string(),
        ))
        .unwrap();
        let _ = next_relay(&mut s);
        s.send(Message::text(
            json!({"t":"proof","m":B64.encode(&m)}).to_string(),
        ))
        .unwrap();
        assert!(closed_within(&mut s, Duration::from_secs(2)));
    }
    squat
        .send(Message::text(
            json!({"t":"proof","m":B64.encode([1u8; 48])}).to_string(),
        ))
        .unwrap();
    assert!(closed_within(&mut squat, Duration::from_secs(2)));
    // an id that is not the key's: closed before any challenge
    let mut wrong = open(addr, "/gateway").unwrap();
    wrong
        .send(Message::text(
            json!({"t":"register","id":ID,"key":key_text(&K8),"secret":OTHER_SECRET}).to_string(),
        ))
        .unwrap();
    assert!(closed_within(&mut wrong, Duration::from_secs(2)));
    assert!(relay.online(ID));
    assert!(
        status(accept(addr, ID, "EBESExQVFhcYGRobHB0eHw", SECRET)) == 404,
        "the first secret still holds"
    );
    // the holder again, with a new secret (a restart): it takes over and
    // the old connection and secret end
    let (_c, reply) = register(addr, &K7, OTHER_SECRET);
    assert_eq!(reply, RelayMsg::Registered);
    assert!(closed_within(&mut a, Duration::from_secs(3)));
    assert!(relay.online(ID));
    assert_eq!(
        status(accept(addr, ID, "EBESExQVFhcYGRobHB0eHw", SECRET)),
        403
    );
}

#[test]
fn malformed_or_oversized_registrations_are_closed() {
    let relay = start_relay(|_| {});
    let addr = relay.local_addr();
    let key = key_text(&K7);
    for bad in [
        json!({"t":"register","id":"short","key":key,"secret":SECRET}).to_string(),
        json!({"t":"register","id":ID,"key":key,"secret":"c2hvcnQ"}).to_string(),
        json!({"t":"register","id":ID,"key":"c2hvcnQ","secret":SECRET}).to_string(),
        json!({"t":"register","id":ID,"secret":SECRET}).to_string(),
        json!({"t":"proof","m":"AAAA"}).to_string(),
        json!({"t":"hello"}).to_string(),
        "not json".to_string(),
        json!({"t":"register","id":ID,"key":key,"secret":SECRET,"pad":"x".repeat(1100)})
            .to_string(),
    ] {
        let mut ws = open(addr, "/gateway").unwrap();
        ws.send(Message::text(bad.clone())).unwrap();
        assert!(closed_within(&mut ws, Duration::from_secs(2)), "{bad}");
    }
    assert!(!relay.online(ID));
    // a binary frame where the registration belongs
    let mut ws = open(addr, "/gateway").unwrap();
    ws.send(Message::binary(vec![1, 2, 3])).unwrap();
    assert!(closed_within(&mut ws, Duration::from_secs(2)));
    // an unknown route
    assert_eq!(status(open(addr, "/admin")), 404);
}

#[test]
fn a_full_relay_makes_room_by_forgetting_the_longest_offline_id() {
    let relay = start_relay(|c| c.max_ids = 2);
    let addr = relay.local_addr();
    let (_a, r) = register(addr, &K7, SECRET);
    assert_eq!(r, RelayMsg::Registered);
    let (b, r) = register(addr, &K8, SECRET);
    assert_eq!(r, RelayMsg::Registered);
    // both online: a third id is refused
    let (_c, r) = register(addr, &K9, SECRET);
    assert_eq!(
        r,
        RelayMsg::Error {
            code: RelayError::Full
        }
    );
    // a known id still re-registers
    let (_a, r) = register(addr, &K7, SECRET);
    assert_eq!(r, RelayMsg::Registered);
    // one goes offline: the new id takes its place
    drop(b);
    wait_until("offline", Duration::from_secs(3), || !relay.online(ID8));
    let (_c, r) = register(addr, &K9, SECRET);
    assert_eq!(r, RelayMsg::Registered);
    assert!(relay.online(ID9) && relay.online(ID));
    let (_b, r) = register(addr, &K8, SECRET);
    assert_eq!(
        r,
        RelayMsg::Error {
            code: RelayError::Full
        }
    );
}

#[test]
fn an_upgrade_sent_a_byte_at_a_time_is_cut_at_the_deadline() {
    use std::io::{Read, Write};
    let relay = start_relay(|c| c.request_deadline = Duration::from_millis(600));
    let mut s = TcpStream::connect(relay.local_addr()).unwrap();
    s.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    let t0 = Instant::now();
    let request = b"GET /connect?id=D20lU_8MD0R64gLt2EOD4Q HTTP/1.1\r\nHost: x\r\n";
    let mut closed = None;
    'trickle: for b in request.iter().cycle() {
        // each byte well within a per-read timeout
        if s.write_all(&[*b]).is_err() {
            closed = Some(t0.elapsed());
            break;
        }
        let until = Instant::now() + Duration::from_millis(100);
        while Instant::now() < until {
            match s.read(&mut [0u8; 64]) {
                Ok(0) => {
                    closed = Some(t0.elapsed());
                    break 'trickle;
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                _ => {
                    closed = Some(t0.elapsed());
                    break 'trickle;
                }
            }
        }
        assert!(
            t0.elapsed() < Duration::from_secs(3),
            "still open after 3 s"
        );
    }
    let took = closed.unwrap();
    assert!(
        took >= Duration::from_millis(500) && took < Duration::from_millis(1500),
        "{took:?}"
    );
}

#[test]
fn sockets_per_address_are_limited() {
    use std::io::Read;
    let relay = start_relay(|c| c.max_per_address = 3);
    let addr = relay.local_addr();
    let held: Vec<TcpStream> = (0..3).map(|_| TcpStream::connect(addr).unwrap()).collect();
    std::thread::sleep(Duration::from_millis(100));
    // the fourth is dropped unread
    let mut extra = TcpStream::connect(addr).unwrap();
    extra
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let t0 = Instant::now();
    assert!(matches!(extra.read(&mut [0u8; 8]), Ok(0) | Err(_)));
    assert!(t0.elapsed() < Duration::from_secs(1), "{:?}", t0.elapsed());
    drop(held);
    wait_until("a slot freed", Duration::from_secs(3), || {
        open(addr, "/admin").is_err_and(|e| matches!(e, tungstenite::Error::Http(_)))
    });
    // behind a trusted proxy every socket shares its address: not counted
    let proxied = start_relay(|c| {
        c.max_per_address = 1;
        c.trust_forwarded = true;
    });
    let _a = open(proxied.local_addr(), "/gateway").unwrap();
    assert_eq!(status(open(proxied.local_addr(), "/admin")), 404);
}

// --- connect, accept, splice ----------------------------------------------

#[test]
fn connect_open_accept_splices_binary_frames_both_ways() {
    let relay = start_relay(|_| {});
    let (mut control, _) = register(relay.local_addr(), &K7, SECRET);
    let (mut phone, mut gw) = pair(&relay, &mut control);
    assert_eq!(relay.pairs(ID), 1);
    phone.send(Message::binary(vec![1, 2, 3])).unwrap();
    assert_eq!(binary(&mut gw), [1, 2, 3]);
    // large frames, unchanged, in order
    let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    for i in 0..5u8 {
        let mut b = big.clone();
        b[0] = i;
        gw.send(Message::binary(b)).unwrap();
    }
    for i in 0..5u8 {
        let got = binary(&mut phone);
        assert_eq!((got.len(), got[0], &got[1..]), (big.len(), i, &big[1..]));
    }
    // the phone closes: the gateway's side closes too
    phone.close(None).unwrap();
    let _ = phone.flush();
    assert!(closed_within(&mut gw, Duration::from_secs(3)));
    wait_until("pair released", Duration::from_secs(3), || {
        relay.pairs(ID) == 0
    });
}

#[test]
fn accept_needs_the_secret_and_a_waiting_conn() {
    let relay = start_relay(|c| c.accept_deadline = Duration::from_secs(2));
    let addr = relay.local_addr();
    let (mut control, _) = register(addr, &K7, SECRET);
    let mut phone = open(addr, &format!("/connect?id={ID}")).unwrap();
    let RelayMsg::Open { conn } = next_relay(&mut control) else {
        panic!()
    };
    for (id, secret) in [(ID, OTHER_SECRET), (ID, &SECRET[..42]), (ID8, SECRET)] {
        assert_eq!(
            status(accept(addr, id, &conn, secret)),
            403,
            "{id} {secret}"
        );
    }
    // no or an empty secret, or the secret in the URL (proxies log URLs)
    assert_eq!(status(accept(addr, ID, &conn, "")), 400);
    assert_eq!(
        status(open(addr, &format!("/accept?id={ID}&conn={conn}"))),
        400
    );
    assert_eq!(
        status(open(
            addr,
            &format!("/accept?id={ID}&conn={conn}&secret={SECRET}")
        )),
        400
    );
    // the right secret for an unknown conn
    assert_eq!(
        status(accept(addr, ID, "EBESExQVFhcYGRobHB0eHw", SECRET)),
        404
    );
    // refusals spent nothing: the right secret still accepts
    let mut gw = accept(addr, ID, &conn, SECRET).unwrap();
    phone.send(Message::binary(b"x".to_vec())).unwrap();
    assert_eq!(binary(&mut gw), b"x");
    // a conn is accepted once
    assert_eq!(status(accept(addr, ID, &conn, SECRET)), 404);
}

#[test]
fn a_phone_whose_gateway_does_not_accept_is_closed_at_the_deadline() {
    let relay = start_relay(|c| c.accept_deadline = Duration::from_millis(400));
    let addr = relay.local_addr();
    let (mut control, _) = register(addr, &K7, SECRET);
    let t0 = Instant::now();
    let mut phone = open(addr, &format!("/connect?id={ID}")).unwrap();
    let RelayMsg::Open { conn } = next_relay(&mut control) else {
        panic!()
    };
    assert!(closed_within(&mut phone, Duration::from_secs(3)));
    let took = t0.elapsed();
    assert!(
        took >= Duration::from_millis(350) && took < Duration::from_secs(2),
        "{took:?}"
    );
    wait_until("pair released", Duration::from_secs(2), || {
        relay.pairs(ID) == 0
    });
    // too late to accept now
    assert_eq!(status(accept(addr, ID, &conn, SECRET)), 404);
    // an offline id is refused at once
    assert_eq!(status(open(addr, &format!("/connect?id={ID8}"))), 404);
    assert_eq!(status(open(addr, "/connect?id=nope")), 400);
}

#[test]
fn pairs_per_id_are_limited() {
    let relay = start_relay(|c| c.max_pairs = 2);
    let addr = relay.local_addr();
    let (mut control, _) = register(addr, &K7, SECRET);
    let _p1 = pair(&relay, &mut control);
    let _p2 = open(addr, &format!("/connect?id={ID}")).unwrap(); // waiting counts
    let _ = next_relay(&mut control);
    assert_eq!(status(open(addr, &format!("/connect?id={ID}"))), 503);
    drop(_p1);
    wait_until("a pair freed", Duration::from_secs(3), || {
        relay.pairs(ID) == 1
    });
    assert!(open(addr, &format!("/connect?id={ID}")).is_ok());
}

#[test]
fn connects_are_rate_limited_per_address() {
    let relay = start_relay(|c| {
        c.connect_rate = 3;
        c.connect_window = Duration::from_millis(800);
    });
    let addr = relay.local_addr();
    // refused or not, every /connect counts
    for _ in 0..3 {
        assert_eq!(status(open(addr, &format!("/connect?id={ID}"))), 404);
    }
    assert_eq!(status(open(addr, &format!("/connect?id={ID}"))), 429);
    std::thread::sleep(Duration::from_millis(850));
    assert_eq!(status(open(addr, &format!("/connect?id={ID}"))), 404);
    // X-Forwarded-For is ignored unless trusted
    for _ in 0..2 {
        let _ = open_with(
            addr,
            &format!("/connect?id={ID}"),
            &[("x-forwarded-for", "1.2.3.4")],
        );
    }
    assert_eq!(
        status(open_with(
            addr,
            &format!("/connect?id={ID}"),
            &[("x-forwarded-for", "5.6.7.8")]
        )),
        429
    );
    let trusting = start_relay(|c| {
        c.connect_rate = 1;
        c.trust_forwarded = true;
    });
    let a = trusting.local_addr();
    let path = format!("/connect?id={ID}");
    assert_eq!(
        status(open_with(
            a,
            &path,
            &[("x-forwarded-for", "9.9.9.9, 1.2.3.4")]
        )),
        404
    );
    assert_eq!(
        status(open_with(a, &path, &[("x-forwarded-for", "1.2.3.4")])),
        429
    );
    assert_eq!(
        status(open_with(a, &path, &[("x-forwarded-for", "1.2.3.5")])),
        404
    );
    // only the last hop counts: one that is not an address (here the
    // proxy wrote `ip:port`) falls back to the peer, never to an earlier,
    // client-written entry
    let strict = start_relay(|c| {
        c.connect_rate = 1;
        c.trust_forwarded = true;
    });
    let a = strict.local_addr();
    assert_eq!(
        status(open_with(
            a,
            &path,
            &[("x-forwarded-for", "1.1.1.1, 7.7.7.7:443")]
        )),
        404
    );
    assert_eq!(
        status(open_with(
            a,
            &path,
            &[("x-forwarded-for", "2.2.2.2, unknown")]
        )),
        429
    );
    // IPv6 counts per /64
    assert_eq!(
        status(open_with(
            a,
            &path,
            &[("x-forwarded-for", "2001:db8:1:2::1")]
        )),
        404
    );
    assert_eq!(
        status(open_with(
            a,
            &path,
            &[("x-forwarded-for", "2001:db8:1:2:ffff::9")]
        )),
        429
    );
}

#[test]
fn text_frames_oversized_frames_and_idle_pairs_close_both_sides() {
    let relay = start_relay(|c| {
        c.max_frame = 4096;
        c.pair_idle = Duration::from_millis(500);
    });
    let (mut control, _) = register(relay.local_addr(), &K7, SECRET);
    // a text frame on a spliced pair
    let (mut phone, mut gw) = pair(&relay, &mut control);
    phone.send(Message::text("hello")).unwrap();
    assert!(closed_within(&mut gw, Duration::from_secs(2)));
    assert!(closed_within(&mut phone, Duration::from_secs(2)));
    // a frame over the limit
    let (mut phone, mut gw) = pair(&relay, &mut control);
    gw.send(Message::binary(vec![0; 4096])).unwrap();
    assert_eq!(binary(&mut phone).len(), 4096);
    gw.send(Message::binary(vec![0; 5000])).unwrap();
    assert!(closed_within(&mut phone, Duration::from_secs(2)));
    assert!(closed_within(&mut gw, Duration::from_secs(2)));
    // idle: traffic keeps it open, silence closes it
    let (mut phone, mut gw) = pair(&relay, &mut control);
    for _ in 0..4 {
        std::thread::sleep(Duration::from_millis(250));
        phone.send(Message::binary(vec![7])).unwrap();
        assert_eq!(binary(&mut gw), [7]);
    }
    let t0 = Instant::now();
    assert!(closed_within(&mut phone, Duration::from_secs(3)));
    assert!(
        t0.elapsed() >= Duration::from_millis(400),
        "{:?}",
        t0.elapsed()
    );
    assert!(closed_within(&mut gw, Duration::from_secs(2)));
}

// --- a real gateway through the relay ---------------------------------------

const NEEDLES: [&str; 5] = [
    "echo opaque-needle-cmd",
    "opaque-needle-output",
    "Needle Phone",
    "needle-host",
    "needle-tab",
];

/// A one-pane source: a `run` becomes a block with known output.
#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Vec<Block>>>);

impl Source for Fake {
    fn topology(&self) -> Vec<Window> {
        vec![Window {
            pid: 1,
            tabs: vec![Tab {
                index: 0,
                title: "needle-tab".into(),
                active: true,
                panes: vec![Pane {
                    id: 7,
                    active: true,
                    cwd: None,
                    job: None,
                    last_command: None,
                    last_exit: None,
                    git: None,
                    waiting: None,
                    running: None,
                }],
            }],
        }]
    }

    fn blocks(&self, pane: u64, _: Option<u64>, _: usize) -> Result<Page, SourceError> {
        if pane != 7 {
            return Err(SourceError::NoPane);
        }
        let blocks = self.0.lock().unwrap().clone();
        Ok(Page {
            oldest: blocks.first().map(|b| b.id),
            blocks,
            has_older: false,
        })
    }

    fn input(&self, _: u64, _: &str, input: Input) -> Result<(), InputError> {
        if let Input::Command(c) = input {
            let mut b = self.0.lock().unwrap();
            let id = b.len() as u64;
            b.push(Block {
                id,
                command: c,
                state: BlockState::Done,
                exit: Some(0),
                duration_ms: 3,
                cwd: None,
                branch: None,
                output: "opaque-needle-output\n".into(),
                dropped: 0,
                restored: false,
            });
        }
        Ok(())
    }
}

struct Env {
    gw: Option<Gateway>,
    dir: PathBuf,
    pinned: [u8; 32],
    id: String,
}

impl Drop for Env {
    fn drop(&mut self) {
        drop(self.gw.take());
        let _ = std::fs::remove_dir_all(self.dir.parent().unwrap());
    }
}

fn gateway(relay: &str) -> Env {
    gateway_with(relay, |_| {})
}

fn gateway_with(relay: &str, tweak: impl FnOnce(&mut Config)) -> Env {
    let tag: [u8; 6] = transport::random_bytes().unwrap();
    let root = std::env::temp_dir().join(format!(
        "rift-relay-{}-{}",
        std::process::id(),
        tag.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    let dir = registry::remote_dir(&root);
    let mut cfg = Config::new(
        dir.clone(),
        "needle-host".into(),
        "linux".into(),
        "9.9.9".into(),
    );
    cfg.pane_poll = Duration::from_millis(10);
    cfg.topology_poll = Duration::from_millis(50);
    cfg.devices_poll = Duration::from_millis(100);
    cfg.relay = Some(relay.to_string());
    cfg.relay_backoff = (Duration::from_millis(50), Duration::from_millis(300));
    tweak(&mut cfg);
    let gw = serve(
        TcpListener::bind("127.0.0.1:0").unwrap(),
        Fake::default(),
        cfg,
    )
    .unwrap();
    let pinned = registry::gateway_public_key(&dir).unwrap();
    Env {
        gw: Some(gw),
        id: registry::routing_id(&pinned),
        dir,
        pinned,
    }
}

impl Env {
    fn relay_state(&self) -> Option<RelayState> {
        registry::read_status(&self.dir).and_then(|s| s.relay.map(|r| r.state))
    }

    fn wait_connected(&self) {
        wait_until("relay connected", Duration::from_secs(5), || {
            self.relay_state() == Some(RelayState::Connected)
        });
    }

    /// A phone handshake through the relay.
    fn dial(
        &self,
        relay: SocketAddr,
        key: &[u8; 32],
        pair: Option<&str>,
    ) -> Result<Channel<TcpStream>, HandshakeError> {
        let ws = open(relay, &format!("/connect?id={}", self.id)).unwrap();
        let hello = ClientHello {
            v: 2,
            name: "Needle Phone".into(),
            platform: "ios".into(),
            pair: pair.map(String::from),
        };
        transport::connect(ws, key, &self.pinned, &hello, 4 << 20).map(|(c, _)| c)
    }

    fn secret(&self, permission: Permission) -> String {
        let link = registry::start_pairing(&self.dir, &[], "needle-host", permission).unwrap();
        link.url
            .split('&')
            .find_map(|kv| kv.strip_prefix("s="))
            .unwrap()
            .to_string()
    }
}

fn recv(ch: &mut Channel<TcpStream>) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "no message within 5 s");
        match ch.recv() {
            Recv::Msg(m) => return serde_json::from_slice(&m).unwrap(),
            Recv::Idle | Recv::Alive | Recv::Partial => {}
            other => panic!("expected a message, got {other:?}"),
        }
    }
}

fn recv_t(ch: &mut Channel<TcpStream>, t: &str) -> Value {
    loop {
        let v = recv(ch);
        if v["t"] == t {
            return v;
        }
    }
}

fn ch_closed(ch: &mut Channel<TcpStream>, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        match ch.recv() {
            Recv::Closed | Recv::Violation(_) => return true,
            _ => {}
        }
    }
    false
}

/// Every needle found in what the tap saw, per direction (the frames of a
/// direction joined, so a needle cut across frames is found too).
fn leaks(tapped: &[(Leg, Vec<u8>)], needles: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for leg in [Leg::ToGateway, Leg::ToPhone] {
        let stream: Vec<u8> = tapped
            .iter()
            .filter(|(l, _)| *l == leg)
            .flat_map(|(_, b)| b.iter().copied())
            .collect();
        for n in needles {
            if stream.windows(n.len()).any(|w| w == n.as_bytes()) {
                found.push(format!("{leg:?}: {n}"));
            }
        }
    }
    found
}

/// Every frame the relay forwarded, with its direction.
type Seen = Arc<Mutex<Vec<(Leg, Vec<u8>)>>>;

fn tapped_relay() -> (Relay, Seen) {
    let seen: Seen = Arc::default();
    let tap = seen.clone();
    let relay = start_relay(move |c| {
        c.tap = Some(Arc::new(move |leg, b: &[u8]| {
            tap.lock().unwrap().push((leg, b.to_vec()));
        }));
    });
    (relay, seen)
}

#[test]
fn the_relay_only_ever_forwards_ciphertext() {
    let (relay, seen) = tapped_relay();
    let env = gateway(&format!("ws://{}", relay.local_addr()));
    env.wait_connected();
    let key = transport::generate_private_key().unwrap();
    let secret = env.secret(Permission::Operate);
    let mut ch = env.dial(relay.local_addr(), &key, Some(&secret)).unwrap();
    let mut plain = Vec::new();
    let welcome = recv(&mut ch);
    assert_eq!(welcome["t"], "welcome");
    let topology = recv(&mut ch);
    plain.push(welcome.to_string());
    plain.push(topology.to_string());
    assert!(ch.send(&json!({"t":"subscribe","pane":7,"after":null}).to_string()));
    assert!(ch.send(
        &json!({"t":"run","id":"r1","pane":7,"command":"echo opaque-needle-cmd"}).to_string()
    ));
    let ack = recv_t(&mut ch, "ack");
    assert_eq!(ack["ok"], true, "{ack}");
    let update = loop {
        let v = recv(&mut ch);
        plain.push(v.to_string());
        if v["t"] == "update" && v.to_string().contains("opaque-needle-output") {
            break v;
        }
    };
    assert!(update.to_string().contains("echo opaque-needle-cmd"));
    ch.close();
    // the needles really travelled (as plaintext at the ends) ...
    let ends = plain.join("\n");
    for n in [
        "opaque-needle-output",
        "echo opaque-needle-cmd",
        "needle-host",
        "needle-tab",
    ] {
        assert!(ends.contains(n), "{n} never reached the phone");
    }
    // ... and the relay forwarded plenty, none of it readable
    let tapped = seen.lock().unwrap().clone();
    let to_gw = tapped.iter().filter(|(l, _)| *l == Leg::ToGateway).count();
    let to_phone = tapped.iter().filter(|(l, _)| *l == Leg::ToPhone).count();
    assert!(to_gw >= 4 && to_phone >= 4, "{to_gw} / {to_phone} frames");
    let mut needles: Vec<&str> = NEEDLES.to_vec();
    needles.push(&secret);
    assert_eq!(leaks(&tapped, &needles), Vec::<String>::new());
    // nor the JSON framing of any message
    assert_eq!(
        leaks(&tapped, &["\"t\":\"", "{\"v\":2", "\"command\""]),
        Vec::<String>::new()
    );
}

#[test]
fn the_opacity_tap_detects_plaintext() {
    // negative control: the same tap and detector over a pair that sends
    // plaintext must report it
    let (relay, seen) = tapped_relay();
    let (mut control, _) = register(relay.local_addr(), &K7, SECRET);
    let (mut phone, mut gw) = pair(&relay, &mut control);
    let run = json!({"t":"run","id":"r1","pane":7,"command":"echo opaque-needle-cmd"}).to_string();
    // cut across two frames
    let (a, b) = run.as_bytes().split_at(run.len() / 2 + 3);
    phone.send(Message::binary(a.to_vec())).unwrap();
    phone.send(Message::binary(b.to_vec())).unwrap();
    let _ = binary(&mut gw);
    let _ = binary(&mut gw);
    gw.send(Message::binary(
        b"Needle Phone ran it: opaque-needle-output".to_vec(),
    ))
    .unwrap();
    let _ = binary(&mut phone);
    let found = leaks(&seen.lock().unwrap(), &NEEDLES);
    assert_eq!(
        found,
        [
            "ToGateway: echo opaque-needle-cmd",
            "ToPhone: opaque-needle-output",
            "ToPhone: Needle Phone"
        ]
    );
}

#[test]
fn relay_clients_show_in_status_and_history() {
    let relay = start_relay(|_| {});
    let env = gateway(&format!("ws://{}/", relay.local_addr()));
    env.wait_connected();
    let st = registry::read_status(&env.dir).unwrap();
    let r = st.relay.unwrap();
    assert_eq!(r.url, format!("ws://{}/", relay.local_addr()));
    assert!(relay.online(&env.id));
    let key = transport::generate_private_key().unwrap();
    let mut ch = env
        .dial(
            relay.local_addr(),
            &key,
            Some(&env.secret(Permission::Operate)),
        )
        .unwrap();
    let welcome = recv(&mut ch);
    let device = welcome["device"].as_str().unwrap().to_string();
    wait_until("client in status", Duration::from_secs(3), || {
        registry::read_status(&env.dir).is_some_and(|s| s.clients.len() == 1)
    });
    let st = registry::read_status(&env.dir).unwrap();
    assert_eq!(
        (st.clients[0].device.as_str(), st.clients[0].via),
        (device.as_str(), Via::Relay)
    );
    ch.close();
    wait_until("history entry", Duration::from_secs(3), || {
        !registry::history(&env.dir).is_empty()
    });
    let h = registry::history(&env.dir);
    let e = h.last().unwrap();
    assert_eq!(e.device.as_deref(), Some(device.as_str()));
    assert_eq!(e.name.as_deref(), Some("Needle Phone"));
    assert_eq!(e.via, Via::Relay);
    assert_eq!(e.address, relay.local_addr().to_string());
    assert_eq!(e.reason, "closed");
    assert!(e.end >= e.start);
    // an unpaired key through the relay: refused, recorded without a device
    let stranger = transport::generate_private_key().unwrap();
    let mut ch = env.dial(relay.local_addr(), &stranger, None).unwrap();
    assert_eq!(recv(&mut ch)["code"], "auth");
    wait_until("failed handshake recorded", Duration::from_secs(3), || {
        registry::history(&env.dir).len() == 2
    });
    let e = registry::history(&env.dir).pop().unwrap();
    assert_eq!(
        (e.device, e.name, e.via, e.reason.as_str()),
        (None, None, Via::Relay, "auth")
    );
}

#[test]
fn a_permission_change_closes_relay_clients_and_the_next_welcome_carries_it() {
    let relay = start_relay(|_| {});
    let env = gateway(&format!("ws://{}", relay.local_addr()));
    env.wait_connected();
    let key = transport::generate_private_key().unwrap();
    let mut ch = env
        .dial(
            relay.local_addr(),
            &key,
            Some(&env.secret(Permission::Operate)),
        )
        .unwrap();
    let welcome = recv(&mut ch);
    assert_eq!(welcome["permission"], "operate");
    let device = welcome["device"].as_str().unwrap().to_string();
    let t0 = Instant::now();
    assert!(registry::set_permission(&env.dir, &device, Permission::View).unwrap());
    assert!(ch_closed(&mut ch, Duration::from_secs(2)));
    assert!(
        t0.elapsed() < Duration::from_millis(1500),
        "{:?}",
        t0.elapsed()
    );
    let mut ch = env.dial(relay.local_addr(), &key, None).unwrap();
    let welcome = recv(&mut ch);
    assert_eq!(welcome["permission"], "view");
    let _ = recv(&mut ch);
    assert!(ch.send(&json!({"t":"run","id":"r","pane":7,"command":"true"}).to_string()));
    assert_eq!(recv_t(&mut ch, "ack")["error"], "forbidden");
    // the closed session's entry is written after its connection ended
    wait_until("permission close recorded", Duration::from_secs(3), || {
        registry::history(&env.dir)
            .iter()
            .any(|e| e.reason == "permission")
    });
}

#[test]
fn the_gateway_re_registers_after_the_relay_restarts() {
    let relay = start_relay(|_| {});
    let addr = relay.local_addr();
    let env = gateway(&format!("ws://{addr}"));
    env.wait_connected();
    let key = transport::generate_private_key().unwrap();
    let mut ch = env
        .dial(addr, &key, Some(&env.secret(Permission::Operate)))
        .unwrap();
    assert_eq!(recv(&mut ch)["t"], "welcome");
    drop(relay);
    // the spliced phone goes with it; the gateway notices and retries
    assert!(ch_closed(&mut ch, Duration::from_secs(3)));
    wait_until("relay lost", Duration::from_secs(5), || {
        env.relay_state() != Some(RelayState::Connected)
    });
    let relay = serve_relay(TcpListener::bind(addr).unwrap(), RelayConfig::default()).unwrap();
    env.wait_connected();
    // same id (relay.json), and phones get through again
    assert!(relay.online(&env.id));
    let mut ch = env.dial(addr, &key, None).unwrap();
    assert_eq!(recv(&mut ch)["t"], "welcome");
}

#[test]
fn a_full_relay_and_a_bad_url_show_as_relay_errors() {
    let full = start_relay(|c| c.max_ids = 1);
    let env = gateway("ftp://nowhere");
    wait_until("bad url error", Duration::from_secs(3), || {
        env.relay_state() == Some(RelayState::Error)
    });
    let st = registry::read_status(&env.dir).unwrap().relay.unwrap();
    assert!(st.error.unwrap().contains("ws://"));
    // the relay's only id is taken by another, online gateway
    let (_other, r) = register(full.local_addr(), &K8, SECRET);
    assert_eq!(r, RelayMsg::Registered);
    env.gw
        .as_ref()
        .unwrap()
        .set_relay(Some(format!("ws://{}", full.local_addr())));
    wait_until("full error", Duration::from_secs(5), || {
        registry::read_status(&env.dir)
            .and_then(|s| s.relay)
            .is_some_and(|r| {
                r.state == RelayState::Error && r.error.is_some_and(|e| e.contains("full"))
            })
    });
    // relay off: no `relay` in status
    env.gw.as_ref().unwrap().set_relay(None);
    wait_until("relay off", Duration::from_secs(3), || {
        registry::read_status(&env.dir).is_some_and(|s| s.relay.is_none())
    });
}

#[test]
fn a_relay_flood_never_refuses_lan_phones() {
    let relay = start_relay(|c| c.connect_rate = 1000);
    let env = gateway_with(&format!("ws://{}", relay.local_addr()), |c| {
        c.handshake_deadline = Duration::from_secs(5);
    });
    env.wait_connected();
    let key = transport::generate_private_key().unwrap();
    let secret = env.secret(Permission::Operate);
    // phones through the relay that never start the handshake: they hold
    // the gateway's relay sockets for the handshake deadline
    let silent: Vec<Ws> = (0..30)
        .filter_map(|_| open(relay.local_addr(), &format!("/connect?id={}", env.id)).ok())
        .collect();
    assert!(silent.len() >= 20, "{} connects", silent.len());
    std::thread::sleep(Duration::from_millis(300));
    // a phone on the LAN still gets in at once
    let lan = env.gw.as_ref().unwrap().local_addr();
    let ws = open(lan, "/").unwrap();
    let hello = ClientHello {
        v: 2,
        name: "LAN Phone".into(),
        platform: "android".into(),
        pair: Some(secret),
    };
    let (mut ch, _) = transport::connect(ws, &key, &env.pinned, &hello, 4 << 20).unwrap();
    assert_eq!(recv(&mut ch)["t"], "welcome");
}

#[test]
fn a_relay_phone_that_goes_silent_is_dropped_at_the_idle_limit() {
    // the relay answers the gateway's WebSocket pings itself: only the
    // phone's Noise messages may count as a sign of life
    let relay = start_relay(|_| {});
    let env = gateway_with(&format!("ws://{}", relay.local_addr()), |c| {
        c.ping_interval = Duration::from_millis(100);
        c.idle_timeout = Duration::from_millis(600);
    });
    env.wait_connected();
    let key = transport::generate_private_key().unwrap();
    let mut ch = env
        .dial(
            relay.local_addr(),
            &key,
            Some(&env.secret(Permission::Operate)),
        )
        .unwrap();
    assert_eq!(recv(&mut ch)["t"], "welcome");
    let t0 = Instant::now();
    assert!(ch_closed(&mut ch, Duration::from_secs(3)));
    assert!(
        t0.elapsed() >= Duration::from_millis(400),
        "{:?}",
        t0.elapsed()
    );
    wait_until("idle recorded", Duration::from_secs(3), || {
        registry::history(&env.dir)
            .iter()
            .any(|e| e.reason == "idle")
    });
}

#[test]
fn revoking_a_relay_device_closes_it_at_once() {
    let relay = start_relay(|_| {});
    let env = gateway(&format!("ws://{}", relay.local_addr()));
    env.wait_connected();
    let key = transport::generate_private_key().unwrap();
    let mut ch = env
        .dial(
            relay.local_addr(),
            &key,
            Some(&env.secret(Permission::Operate)),
        )
        .unwrap();
    let device = recv(&mut ch)["device"].as_str().unwrap().to_string();
    let t0 = Instant::now();
    assert!(registry::revoke(&env.dir, &device).unwrap());
    assert_eq!(recv_t(&mut ch, "error")["code"], "auth");
    assert!(ch_closed(&mut ch, Duration::from_secs(2)));
    assert!(
        t0.elapsed() < Duration::from_millis(1500),
        "{:?}",
        t0.elapsed()
    );
    wait_until("revoked recorded", Duration::from_secs(3), || {
        registry::history(&env.dir)
            .iter()
            .any(|e| e.reason == "revoked")
    });
    // and it cannot come back
    if let Ok(mut ch) = env.dial(relay.local_addr(), &key, None) {
        assert_eq!(recv(&mut ch)["code"], "auth");
    }
}
