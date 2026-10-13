//! The gateway over real localhost sockets, fed by a fake source; the test
//! acts as the phone with `snow` through `rift_remote::transport::connect`.

use rift_remote::protocol::{Block, BlockState, ClientHello, Pane, Permission, Tab, Window};
use rift_remote::registry::{self, PairingLink};
use rift_remote::transport::{self, Channel, HandshakeError, Recv};
use rift_remote::{Config, Gateway, Input, InputError, Key, Page, Source, SourceError, serve};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::Message;

#[derive(Default)]
struct State {
    panes: BTreeMap<u64, Vec<Block>>,
    /// What `input` received: (pane, device name, input).
    inputs: Vec<(u64, String, Input)>,
    /// What `input` answers next (`None`: ok).
    input_error: Option<InputError>,
    /// Panes read through `blocks_meta` (the `done` probe), and through
    /// `blocks` (with output).
    meta_reads: Vec<u64>,
    full_reads: Vec<u64>,
    /// Client counts in `status.json` at each `status_changed` call.
    status_changes: Vec<usize>,
    /// Where `status.json` lives (set by `start`).
    dir: Option<PathBuf>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<State>>);

impl Fake {
    fn set(&self, pane: u64, blocks: Vec<Block>) {
        self.0.lock().unwrap().panes.insert(pane, blocks);
    }
    fn edit(&self, pane: u64, f: impl FnOnce(&mut Vec<Block>)) {
        f(self.0.lock().unwrap().panes.get_mut(&pane).unwrap());
    }
    fn close(&self, pane: u64) {
        self.0.lock().unwrap().panes.remove(&pane);
    }
    fn inputs(&self) -> Vec<(u64, String, Input)> {
        self.0.lock().unwrap().inputs.clone()
    }
    fn fail_input(&self, e: Option<InputError>) {
        self.0.lock().unwrap().input_error = e;
    }
}

impl Source for Fake {
    fn topology(&self) -> Vec<Window> {
        let st = self.0.lock().unwrap();
        vec![Window {
            pid: 4242,
            tabs: vec![Tab {
                index: 0,
                title: "fake".into(),
                active: true,
                panes: st
                    .panes
                    .keys()
                    .map(|&id| Pane {
                        id,
                        active: false,
                        cwd: None,
                        job: None,
                        last_command: None,
                        last_exit: None,
                    })
                    .collect(),
            }],
        }]
    }

    fn status_changed(&self) {
        let mut st = self.0.lock().unwrap();
        let clients = st
            .dir
            .as_ref()
            .and_then(|d| std::fs::read(d.join("status.json")).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .map_or(usize::MAX, |v| v["clients"].as_array().map_or(0, Vec::len));
        st.status_changes.push(clients);
    }

    fn blocks_meta(&self, pane: u64, limit: usize) -> Result<Page, SourceError> {
        self.0.lock().unwrap().meta_reads.push(pane);
        let mut page = self.page(pane, None, limit)?;
        for b in &mut page.blocks {
            b.output.clear();
        }
        Ok(page)
    }

    fn blocks(&self, pane: u64, before: Option<u64>, limit: usize) -> Result<Page, SourceError> {
        self.0.lock().unwrap().full_reads.push(pane);
        self.page(pane, before, limit)
    }

    fn input(&self, pane: u64, device: &str, input: Input) -> Result<(), InputError> {
        let mut st = self.0.lock().unwrap();
        if let Some(e) = st.input_error.clone() {
            return Err(e);
        }
        if !st.panes.contains_key(&pane) {
            return Err(InputError::NoPane);
        }
        st.inputs.push((pane, device.to_string(), input));
        Ok(())
    }
}

impl Fake {
    fn page(&self, pane: u64, before: Option<u64>, limit: usize) -> Result<Page, SourceError> {
        let st = self.0.lock().unwrap();
        let all = st.panes.get(&pane).ok_or(SourceError::NoPane)?;
        let older: Vec<&Block> = all
            .iter()
            .filter(|b| before.is_none_or(|x| b.id < x))
            .collect();
        let from = older.len().saturating_sub(limit);
        Ok(Page {
            blocks: older[from..].iter().map(|b| (*b).clone()).collect(),
            has_older: from > 0,
            oldest: all.first().map(|b| b.id),
        })
    }

    fn status_changes(&self) -> Vec<usize> {
        self.0.lock().unwrap().status_changes.clone()
    }

    /// Panes read without output (`blocks_meta`) and with it (`blocks`).
    fn reads(&self) -> (Vec<u64>, Vec<u64>) {
        let st = self.0.lock().unwrap();
        (st.meta_reads.clone(), st.full_reads.clone())
    }
}

fn block(id: u64, output: &str) -> Block {
    Block {
        id,
        command: format!("echo {id}"),
        state: BlockState::Done,
        exit: Some(0),
        duration_ms: 5,
        cwd: Some("/tmp".into()),
        branch: None,
        output: output.into(),
        dropped: 0,
        restored: false,
    }
}

struct Env {
    gw: Option<Gateway>,
    fake: Fake,
    dir: PathBuf,
    addr: SocketAddr,
    /// The gateway's static public key (what links carry and phones pin).
    pinned: [u8; 32],
}

impl Drop for Env {
    fn drop(&mut self) {
        drop(self.gw.take());
        if let Some(root) = self.dir.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

fn scratch() -> PathBuf {
    let tag: [u8; 6] = transport::random_bytes().unwrap();
    let root = std::env::temp_dir().join(format!(
        "rift-gw-{}-{}",
        std::process::id(),
        tag.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    registry::remote_dir(&root)
}

fn start(tweak: impl FnOnce(&mut Config)) -> Env {
    let fake = Fake::default();
    fake.set(12, vec![block(1, "one\n"), block(2, "two\n")]);
    let dir = scratch();
    fake.0.lock().unwrap().dir = Some(dir.clone());
    let mut cfg = Config::new(
        dir.clone(),
        "test-host".into(),
        "linux".into(),
        "9.9.9".into(),
    );
    cfg.pane_poll = Duration::from_millis(10);
    cfg.topology_poll = Duration::from_millis(50);
    cfg.devices_poll = Duration::from_millis(100);
    tweak(&mut cfg);
    let gw = serve(TcpListener::bind("127.0.0.1:0").unwrap(), fake.clone(), cfg).unwrap();
    let addr = gw.local_addr();
    let pinned = registry::gateway_public_key(&dir).unwrap();
    Env {
        gw: Some(gw),
        fake,
        dir,
        addr,
        pinned,
    }
}

type Ch = Channel<TcpStream>;

fn hello(pair: Option<&str>) -> ClientHello {
    ClientHello {
        v: 2,
        name: "Test Phone".into(),
        platform: "ios".into(),
        pair: pair.map(String::from),
    }
}

fn ws(addr: SocketAddr) -> tungstenite::WebSocket<TcpStream> {
    let stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    tungstenite::client(format!("ws://{addr}/"), stream)
        .unwrap()
        .0
}

fn link_secret(link: &PairingLink) -> String {
    link.url
        .split('&')
        .find_map(|kv| kv.strip_prefix("s="))
        .unwrap()
        .to_string()
}

fn new_key() -> [u8; 32] {
    transport::generate_private_key().unwrap()
}

impl Env {
    fn link(&self, permission: Permission) -> PairingLink {
        registry::start_pairing(&self.dir, &[self.addr], "test-host", permission).unwrap()
    }

    /// Handshake only.
    fn handshake(&self, key: &[u8; 32], hello: &ClientHello) -> Result<Ch, HandshakeError> {
        transport::connect(ws(self.addr), key, &self.pinned, hello, 4 << 20).map(|(c, _)| c)
    }

    /// Pair a new device with `permission`: its key, its channel after
    /// `welcome` and `topology`, and the welcome.
    fn pair(&self, permission: Permission) -> ([u8; 32], Ch, Value) {
        let key = new_key();
        let secret = link_secret(&self.link(permission));
        let mut ch = self.handshake(&key, &hello(Some(&secret))).unwrap();
        let welcome = recv(&mut ch);
        assert_eq!(welcome["t"], "welcome", "{welcome}");
        assert_eq!(recv(&mut ch)["t"], "topology");
        (key, ch, welcome)
    }

    /// Connect a paired device: its channel after `welcome` and `topology`.
    fn login(&self, key: &[u8; 32]) -> Ch {
        let mut ch = self.handshake(key, &hello(None)).unwrap();
        assert_eq!(recv(&mut ch)["t"], "welcome");
        assert_eq!(recv(&mut ch)["t"], "topology");
        ch
    }

    /// A connection that is refused after the handshake: the error.
    fn refused(&self, key: &[u8; 32], hello: &ClientHello) -> Value {
        let mut ch = self.handshake(key, hello).unwrap();
        let v = recv(&mut ch);
        assert_eq!(v["t"], "error", "{v}");
        assert_closed(&mut ch);
        v
    }
}

fn put(ch: &mut Ch, v: Value) {
    assert!(ch.send(&v.to_string()));
}

/// Next JSON message within 3 s.
fn recv(ch: &mut Ch) -> Value {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(Instant::now() < deadline, "no message within 3 s");
        match ch.recv() {
            Recv::Msg(m) => return serde_json::from_slice(&m).unwrap(),
            Recv::Idle | Recv::Alive | Recv::Partial => {}
            other => panic!("expected a message, got {other:?}"),
        }
    }
}

/// Next message of type `t` (topology refreshes and `done` skipped).
fn recv_t(ch: &mut Ch, t: &str) -> Value {
    loop {
        let v = recv(ch);
        if v["t"] == t {
            return v;
        }
        assert!(
            v["t"] == "topology" || v["t"] == "done",
            "expected {t}, got {v}"
        );
    }
}

/// The gateway closes the connection (within 3 s; messages before it must
/// be errors or topology).
fn assert_closed(ch: &mut Ch) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(Instant::now() < deadline, "socket still open");
        match ch.recv() {
            Recv::Closed => return,
            Recv::Msg(m) => {
                let v: Value = serde_json::from_slice(&m).unwrap();
                assert!(
                    v["t"] == "error" || v["t"] == "topology",
                    "unexpected {v} before close"
                );
            }
            _ => {}
        }
    }
}

fn ids(v: &Value) -> Vec<u64> {
    v["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_u64().unwrap())
        .collect()
}

fn code(v: &Value) -> (&str, &str) {
    (
        v["t"].as_str().unwrap_or(""),
        v["code"].as_str().unwrap_or(""),
    )
}

fn ack(v: &Value) -> (bool, &str) {
    assert_eq!(v["t"], "ack", "{v}");
    (
        v["ok"].as_bool().unwrap(),
        v["error"].as_str().unwrap_or(""),
    )
}

// --- handshake, pairing, authentication -----------------------------------

#[test]
fn pairing_then_reconnecting_gets_welcome_and_topology() {
    let env = start(|_| {});
    let key = new_key();
    let secret = link_secret(&env.link(Permission::Operate));
    let (mut ch, gateway) = transport::connect(
        ws(env.addr),
        &key,
        &env.pinned,
        &hello(Some(&secret)),
        1 << 20,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&gateway).unwrap(),
        json!({"v":2,"host":"test-host","os":"linux","version":"9.9.9"})
    );
    let welcome = recv(&mut ch);
    let devices = registry::devices(&env.dir).unwrap();
    assert_eq!(devices.len(), 1);
    let d = &devices[0];
    assert_eq!(
        welcome,
        json!({"t":"welcome","v":2,"host":"test-host","os":"linux","version":"9.9.9",
               "device":d.id,"permission":"operate"})
    );
    assert_eq!(
        (d.name.as_str(), d.platform.as_str()),
        ("Test Phone", "ios")
    );
    assert_eq!(d.key_bytes(), Some(transport::public_key(&key)));
    let topo = recv(&mut ch);
    assert_eq!(topo["t"], "topology");
    assert_eq!(topo["windows"][0]["pid"], 4242);
    assert_eq!(topo["windows"][0]["tabs"][0]["panes"][0]["id"], 12);
    // status.json lists the client while it is connected
    let st = registry::read_status(&env.dir).unwrap();
    assert_eq!(st.pid, std::process::id());
    assert_eq!(st.addresses, [env.addr]);
    assert_eq!(st.clients.len(), 1);
    assert_eq!(st.clients[0].device, d.id);
    ch.close();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !registry::read_status(&env.dir).unwrap().clients.is_empty() {
        assert!(Instant::now() < deadline, "status.json still lists it");
        std::thread::sleep(Duration::from_millis(20));
    }
    // every status.json write was announced, right after it: started (no
    // client), connected (1), left (0)
    let deadline = Instant::now() + Duration::from_secs(3);
    while env.fake.status_changes() != [0, 1, 0] {
        assert!(Instant::now() < deadline, "{:?}", env.fake.status_changes());
        std::thread::sleep(Duration::from_millis(20));
    }
    // the same key later, without a secret
    let mut again = env.handshake(&key, &hello(None)).unwrap();
    assert_eq!(recv(&mut again)["device"], json!(d.id));
}

#[test]
fn unpaired_key_is_refused() {
    let env = start(|_| {});
    // a paired device exists; another key is still refused
    env.pair(Permission::Operate);
    let v = env.refused(&new_key(), &hello(None));
    assert_eq!(code(&v), ("error", "auth"));
    assert_eq!(registry::devices(&env.dir).unwrap().len(), 1);
}

#[test]
fn pairing_secret_is_spent_and_expires() {
    let env = start(|_| {});
    let secret = link_secret(&env.link(Permission::Operate));
    let mut first = env.handshake(&new_key(), &hello(Some(&secret))).unwrap();
    assert_eq!(recv(&mut first)["t"], "welcome");
    // the second use of the same secret, by another key
    let v = env.refused(&new_key(), &hello(Some(&secret)));
    assert_eq!(code(&v), ("error", "auth"));
    // a secret that is not one, and garbage
    let v = env.refused(&new_key(), &hello(Some("AAAAAAAAAAAAAAAAAAAAAA")));
    assert_eq!(code(&v), ("error", "auth"));
    let v = env.refused(&new_key(), &hello(Some("%%%")));
    assert_eq!(code(&v), ("error", "auth"));
    // an expired secret
    let secret = link_secret(&env.link(Permission::Operate));
    let path = env.dir.join("pairing.json");
    let mut live: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(live.as_array().unwrap().len(), 1, "spent secrets are gone");
    live[0]["expires_at"] = json!("2001-01-01T00:00:00Z");
    std::fs::write(&path, live.to_string()).unwrap();
    let v = env.refused(&new_key(), &hello(Some(&secret)));
    assert_eq!(code(&v), ("error", "auth"));
    assert_eq!(registry::devices(&env.dir).unwrap().len(), 1);
}

#[test]
fn version_mismatch_is_refused() {
    let env = start(|_| {});
    let (key, ch, _) = env.pair(Permission::Operate);
    ch.close();
    for v in [1, 3] {
        let mut h = hello(None);
        h.v = v;
        let e = env.refused(&key, &h);
        assert_eq!(code(&e), ("error", "version"), "v{v}");
    }
    // the same key with v 2 is served
    let mut ch = env.handshake(&key, &hello(None)).unwrap();
    assert_eq!(recv(&mut ch)["t"], "welcome");
}

#[test]
fn the_client_pins_the_gateway_key_from_the_link() {
    let env = start(|_| {});
    let link = env.link(Permission::Operate);
    let k = link
        .url
        .split('&')
        .find_map(|kv| kv.strip_prefix("k="))
        .unwrap();
    assert_eq!(registry::decode_key(k), Some(env.pinned));
    // the gateway's handshake carries exactly that key
    let other = transport::public_key(&new_key());
    let r = transport::connect(
        ws(env.addr),
        &new_key(),
        &other,
        &hello(Some(&link_secret(&link))),
        1 << 20,
    );
    assert!(matches!(r, Err(HandshakeError::Pinning)), "{:?}", r.err());
    // the aborted attempt did not spend the secret
    let mut ch = env
        .handshake(&new_key(), &hello(Some(&link_secret(&link))))
        .unwrap();
    assert_eq!(recv(&mut ch)["t"], "welcome");
}

#[test]
fn text_frames_close_the_connection() {
    let env = start(|_| {});
    let (key, mut ch, _) = env.pair(Permission::Operate);
    ch.ws
        .send(Message::text(json!({"t":"ping","n":1}).to_string()))
        .unwrap();
    assert_closed(&mut ch);
    // during the handshake too
    let mut raw = ws(env.addr);
    raw.send(Message::text("hello")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(Instant::now() < deadline, "still open");
        match raw.read() {
            Ok(Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) => break,
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
            Ok(_) => {}
        }
    }
    env.login(&key);
}

#[test]
fn the_handshake_has_a_deadline() {
    let env = start(|c| c.handshake_deadline = Duration::from_millis(300));
    let mut raw = ws(env.addr);
    let t = Instant::now();
    loop {
        assert!(t.elapsed() < Duration::from_secs(2), "still open");
        match raw.read() {
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    assert!(
        t.elapsed() >= Duration::from_millis(250),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn an_upgrade_or_handshake_sent_a_byte_at_a_time_ends_at_the_deadline() {
    use std::io::{Read, Write};
    let env = start(|c| c.handshake_deadline = Duration::from_millis(500));
    let request = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n",
        env.addr
    );
    let mut s = TcpStream::connect(env.addr).unwrap();
    s.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    let t = Instant::now();
    'trickle: for b in request.bytes().cycle() {
        // each byte well within any single read's timeout
        if s.write_all(&[b]).is_err() {
            break;
        }
        let until = Instant::now() + Duration::from_millis(80);
        while Instant::now() < until {
            match s.read(&mut [0u8; 64]) {
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                _ => break 'trickle,
            }
        }
        assert!(t.elapsed() < Duration::from_secs(3), "still open after 3 s");
    }
    assert!(
        t.elapsed() >= Duration::from_millis(400) && t.elapsed() < Duration::from_millis(1500),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn failed_handshakes_are_rate_limited_per_address() {
    let env = start(|c| c.refusal = Duration::from_millis(800));
    let (key, ch, _) = env.pair(Permission::Operate);
    drop(ch);
    for _ in 0..rift_remote::MAX_FAILURES {
        let v = env.refused(&new_key(), &hello(None));
        assert_eq!(code(&v), ("error", "auth"));
    }
    // refused before the WebSocket upgrade, even for a paired key
    let stream = TcpStream::connect(env.addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    assert!(tungstenite::client(format!("ws://{}/", env.addr), stream).is_err());
    std::thread::sleep(Duration::from_millis(900));
    env.login(&key);
}

#[test]
fn the_ninth_connection_is_busy() {
    let env = start(|_| {});
    let (key, first, _) = env.pair(Permission::Operate);
    let mut open: Vec<Ch> = vec![first];
    open.extend((0..7).map(|_| env.login(&key)));
    let v = env.refused(&key, &hello(None));
    assert_eq!(code(&v), ("error", "busy"));
    // a slot frees when a client leaves
    open.pop().unwrap().close();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let mut ch = env.handshake(&key, &hello(None)).unwrap();
        if recv(&mut ch)["t"] == "welcome" {
            break;
        }
        assert!(Instant::now() < deadline, "slot never freed");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_permission_change_reconnects_the_device_with_the_new_permission() {
    let env = start(|c| c.devices_poll = rift_remote::DEVICES_POLL);
    let (key, mut ch, welcome) = env.pair(Permission::View);
    let (_other_key, mut other, _) = env.pair(Permission::View);
    let id = welcome["device"].as_str().unwrap();
    let t = Instant::now();
    assert!(registry::set_permission(&env.dir, id, Permission::Operate).unwrap());
    // closed without an error (it may reconnect), within a second
    loop {
        assert!(
            t.elapsed() < Duration::from_millis(1100),
            "{:?}",
            t.elapsed()
        );
        match ch.recv() {
            Recv::Closed => break,
            Recv::Msg(m) => {
                let v: Value = serde_json::from_slice(&m).unwrap();
                assert!(v["t"] == "topology", "unexpected {v}");
            }
            _ => {}
        }
    }
    let mut ch = env.handshake(&key, &hello(None)).unwrap();
    let w = recv(&mut ch);
    assert_eq!(
        (w["t"].as_str(), w["permission"].as_str()),
        (Some("welcome"), Some("operate"))
    );
    // the other device was not touched
    put(&mut other, json!({"t":"ping","n":6}));
    assert_eq!(recv_t(&mut other, "pong")["n"], 6);
    // unchanged permission: nobody is dropped
    assert!(registry::set_permission(&env.dir, id, Permission::Operate).unwrap());
    std::thread::sleep(Duration::from_millis(700));
    put(&mut ch, json!({"t":"ping","n":7}));
    assert_eq!(recv_t(&mut ch, "pong")["n"], 7);
}

#[test]
fn a_permission_change_applies_at_once_not_at_the_next_devices_poll() {
    // the watcher is slow here: what changes must not wait for it
    let env = start(|c| c.devices_poll = Duration::from_secs(30));
    let (key, mut ch, welcome) = env.pair(Permission::Operate);
    let id = welcome["device"].as_str().unwrap().to_string();
    assert!(registry::set_permission(&env.dir, &id, Permission::View).unwrap());
    // the open connection may not run any more ...
    put(
        &mut ch,
        json!({"t":"run","id":"r1","pane":12,"command":"echo late"}),
    );
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "forbidden"));
    assert!(
        env.fake.inputs().is_empty(),
        "a command ran after the change"
    );
    // ... and a new handshake gets the new permission
    let mut again = env.handshake(&key, &hello(None)).unwrap();
    assert_eq!(recv(&mut again)["permission"], "view");
    // a revoked device does not authenticate either
    assert!(registry::revoke(&env.dir, &id).unwrap());
    let v = env.refused(&key, &hello(None));
    assert_eq!(code(&v), ("error", "auth"));
}

#[test]
fn history_records_lan_connections_and_failed_handshakes() {
    let env = start(|_| {});
    let (key, ch, welcome) = env.pair(Permission::Operate);
    let id = welcome["device"].as_str().unwrap().to_string();
    let local = ch.ws.get_ref().local_addr().unwrap();
    ch.close();
    let wait = |n: usize| {
        let t = Instant::now();
        while registry::history(&env.dir).len() < n {
            assert!(t.elapsed() < Duration::from_secs(3), "no history entry");
            std::thread::sleep(Duration::from_millis(10));
        }
        registry::history(&env.dir)
    };
    let h = wait(1);
    assert_eq!(h[0].device.as_deref(), Some(id.as_str()));
    assert_eq!(h[0].name.as_deref(), Some("Test Phone"));
    assert_eq!(h[0].via, registry::Via::Lan);
    assert_eq!(h[0].address, local.to_string(), "the phone's address");
    assert_eq!(h[0].reason, "closed");
    // unpaired, wrong version, a broken handshake: no device
    env.refused(&new_key(), &hello(None));
    let mut v3 = hello(None);
    v3.v = 3;
    env.refused(&key, &v3);
    let mut raw = ws(env.addr);
    raw.send(Message::binary(vec![1, 2, 3])).unwrap();
    let h = wait(4);
    let failed: Vec<(Option<String>, &str)> = h[1..]
        .iter()
        .map(|e| (e.device.clone(), e.reason.as_str()))
        .collect();
    assert_eq!(
        failed,
        [(None, "auth"), (None, "version"), (None, "handshake")]
    );
    assert!(h.iter().all(|e| e.address.starts_with("127.0.0.1:")));
    let _ = key;
}

#[test]
fn a_revoked_device_is_dropped_within_a_second() {
    let env = start(|c| c.devices_poll = rift_remote::DEVICES_POLL);
    let (key, mut ch, welcome) = env.pair(Permission::Operate);
    let (_other_key, mut other, _) = env.pair(Permission::View);
    let id = welcome["device"].as_str().unwrap();
    let t = Instant::now();
    assert!(registry::revoke(&env.dir, id).unwrap());
    let v = recv_t(&mut ch, "error");
    assert_eq!(code(&v), ("error", "auth"));
    assert_closed(&mut ch);
    assert!(
        t.elapsed() < Duration::from_millis(1100),
        "{:?}",
        t.elapsed()
    );
    // refused from then on; the other device stays
    let v = env.refused(&key, &hello(None));
    assert_eq!(code(&v), ("error", "auth"));
    put(&mut other, json!({"t":"ping","n":5}));
    assert_eq!(recv_t(&mut other, "pong")["n"], 5);
}

#[test]
fn forget_removes_the_device_and_closes() {
    let env = start(|_| {});
    let (key, mut ch, welcome) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"forget"}));
    assert_closed(&mut ch);
    let left: Vec<String> = registry::devices(&env.dir)
        .unwrap()
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert!(!left.contains(&welcome["device"].as_str().unwrap().to_string()));
    let v = env.refused(&key, &hello(None));
    assert_eq!(code(&v), ("error", "auth"));
}

#[test]
fn one_gateway_per_registry_and_status_is_removed_on_exit() {
    let mut env = start(|_| {});
    let second = serve(
        TcpListener::bind("127.0.0.1:0").unwrap(),
        Fake::default(),
        Config::new(env.dir.clone(), "h".into(), "linux".into(), "0".into()),
    );
    assert_eq!(
        second.err().map(|e| e.kind()),
        Some(std::io::ErrorKind::AddrInUse)
    );
    assert!(registry::read_status(&env.dir).is_some());
    drop(env.gw.take());
    assert!(registry::read_status(&env.dir).is_none());
    assert!(!env.dir.join("status.json").exists());
}

// --- messages, framing ------------------------------------------------------

#[test]
fn messages_over_65000_bytes_are_chunked_both_ways() {
    let env = start(|_| {});
    // 3 blocks of 64 KiB: one snapshot of ~200 KB, several chunks
    let big = "x".repeat(rift_remote::MAX_OUTPUT_BYTES);
    env.fake
        .set(12, (1..=3).map(|i| block(i, &big)).collect::<Vec<_>>());
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    let s = recv_t(&mut ch, "snapshot");
    assert!(s.to_string().len() > 3 * transport::MAX_CHUNK);
    assert_eq!(ids(&s), [1, 2, 3]);
    assert_eq!(s["blocks"][2]["output"].as_str().unwrap().len(), big.len());
    // a client message in many small chunks is reassembled
    assert!(ch.send_chunked(&json!({"t":"ping","n":77}).to_string(), 3));
    assert_eq!(recv_t(&mut ch, "pong")["n"], 77);
}

#[test]
fn oversized_client_message_closes_the_connection() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    // just under the limit is served (unknown fields are ignored)
    let pad = "x".repeat(4096 - 40);
    let near = json!({"t":"ping","n":7,"pad":pad}).to_string();
    assert!(near.len() <= 4096, "{}", near.len());
    assert!(ch.send(&near));
    assert_eq!(recv_t(&mut ch, "pong")["n"], 7);
    // over it, even in chunks that are each small
    let big = json!({"t":"ping","n":8,"pad":"x".repeat(5000)}).to_string();
    assert!(ch.send_chunked(&big, 1000));
    let v = recv_t(&mut ch, "error");
    assert_eq!(code(&v), ("error", "too_large"));
    assert_closed(&mut ch);
}

#[test]
fn unknown_type_is_bad_request_and_the_connection_stays() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    put(
        &mut ch,
        json!({"t":"send_text","pane":12,"text":"rm -rf ~\n"}),
    );
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "bad_request"));
    put(&mut ch, json!({"t":"ping","n":3}));
    assert_eq!(recv_t(&mut ch, "pong"), json!({"t":"pong","n":3}));
    assert!(env.fake.inputs().is_empty());
}

// --- input ------------------------------------------------------------------

#[test]
fn run_and_keys_reach_the_source() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    put(
        &mut ch,
        json!({"t":"run","id":"r1","pane":12,"command":"cargo fmt --all"}),
    );
    assert_eq!(
        recv_t(&mut ch, "ack"),
        json!({"t":"ack","id":"r1","ok":true})
    );
    put(
        &mut ch,
        json!({"t":"keys","id":"k1","pane":12,"keys":["ctrl+c","esc","up","enter","shift+tab"]}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    assert_eq!(
        env.fake.inputs(),
        [
            (
                12,
                "Test Phone".to_string(),
                Input::Command("cargo fmt --all".into())
            ),
            (
                12,
                "Test Phone".to_string(),
                Input::Keys(vec![
                    Key::Ctrl('c'),
                    Key::Esc,
                    Key::Up,
                    Key::Enter,
                    Key::ShiftTab
                ])
            ),
        ]
    );
}

#[test]
fn source_errors_become_acks() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    for (e, want) in [
        (InputError::Busy, "busy"),
        (InputError::NoPane, "no_pane"),
        (InputError::Denied, "forbidden"),
        (
            InputError::Unavailable("window did not answer".into()),
            "busy",
        ),
    ] {
        env.fake.fail_input(Some(e.clone()));
        put(
            &mut ch,
            json!({"t":"run","id":"r2","pane":12,"command":"ls"}),
        );
        let v = recv_t(&mut ch, "ack");
        assert_eq!(
            (ack(&v), v["id"].as_str()),
            ((false, want), Some("r2")),
            "{e:?}"
        );
        assert!(v["message"].is_string());
    }
    env.fake.fail_input(None);
    std::thread::sleep(Duration::from_millis(1050));
    put(
        &mut ch,
        json!({"t":"run","id":"r3","pane":99,"command":"ls"}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "no_pane"));
    put(
        &mut ch,
        json!({"t":"run","id":"r4","pane":12,"command":"ls"}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
}

#[test]
fn view_devices_are_forbidden_to_run_or_send_keys() {
    let env = start(|_| {});
    let (_k, mut ch, welcome) = env.pair(Permission::View);
    assert_eq!(welcome["permission"], "view");
    put(
        &mut ch,
        json!({"t":"run","id":"r1","pane":12,"command":"ls"}),
    );
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "forbidden"));
    put(
        &mut ch,
        json!({"t":"keys","id":"k1","pane":12,"keys":["ctrl+c"]}),
    );
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "forbidden"));
    // reading still works
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    assert!(env.fake.inputs().is_empty());
}

#[test]
fn malformed_input_is_rejected_before_the_source() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    let mut sent = 0;
    let mut check = |ch: &mut Ch, v: Value, want: &str| {
        // stay under the per-second rates
        sent += 1;
        if sent % 4 == 0 {
            std::thread::sleep(Duration::from_millis(1050));
        }
        put(ch, v.clone());
        let r = recv_t(ch, "ack");
        assert_eq!(ack(&r), (false, want), "{v} → {r}");
    };
    for command in ["ls\nrm -rf ~", "ls\r", "a\u{1b}[2J", "tab\there", "", "   "] {
        check(
            &mut ch,
            json!({"t":"run","id":"r","pane":12,"command":command}),
            "bad_request",
        );
    }
    check(
        &mut ch,
        json!({"t":"run","id":"r","pane":12,"command":"x".repeat(2049)}),
        "too_large",
    );
    for keys in [
        json!(["ctrl+C"]),
        json!(["f1"]),
        json!(["enter", "nope"]),
        json!([]),
    ] {
        check(
            &mut ch,
            json!({"t":"keys","id":"k","pane":12,"keys":keys}),
            "bad_request",
        );
    }
    check(
        &mut ch,
        json!({"t":"keys","id":"k","pane":12,"keys":vec!["up"; 21]}),
        "too_large",
    );
    // a 2 KiB command and 20 keys are fine
    std::thread::sleep(Duration::from_millis(1050));
    put(
        &mut ch,
        json!({"t":"run","id":"ok","pane":12,"command":"x".repeat(2048)}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    put(
        &mut ch,
        json!({"t":"keys","id":"ok","pane":12,"keys":vec!["up"; 20]}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    // an id longer than 64 characters cannot be acknowledged
    put(
        &mut ch,
        json!({"t":"run","id":"i".repeat(65),"pane":12,"command":"ls"}),
    );
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "bad_request"));
    assert_eq!(env.fake.inputs().len(), 2);
}

#[test]
fn run_and_keys_are_rate_limited_per_connection() {
    let env = start(|_| {});
    let (key, mut ch, _) = env.pair(Permission::Operate);
    let t = Instant::now();
    let mut acks = Vec::new();
    for i in 0..5 {
        put(
            &mut ch,
            json!({"t":"run","id":format!("r{i}"),"pane":12,"command":"true"}),
        );
        acks.push(recv_t(&mut ch, "ack"));
    }
    assert!(
        t.elapsed() < Duration::from_millis(900),
        "{:?}",
        t.elapsed()
    );
    let got: Vec<(bool, &str)> = acks.iter().map(ack).collect();
    assert_eq!(got[..4], [(true, ""); 4]);
    assert_eq!(got[4], (false, "rate"));
    // keys: 20 messages per second, counted separately from run
    let mut acks = Vec::new();
    for i in 0..21 {
        put(
            &mut ch,
            json!({"t":"keys","id":format!("k{i}"),"pane":12,"keys":["up"]}),
        );
        acks.push(recv_t(&mut ch, "ack"));
    }
    let got: Vec<(bool, &str)> = acks.iter().map(ack).collect();
    assert_eq!(got[..20], [(true, ""); 20]);
    assert_eq!(got[20], (false, "rate"));
    // per connection: another connection of the same device is not limited
    let mut second = env.login(&key);
    put(
        &mut second,
        json!({"t":"run","id":"s","pane":12,"command":"true"}),
    );
    assert_eq!(ack(&recv_t(&mut second, "ack")), (true, ""));
    // and the window slides
    std::thread::sleep(Duration::from_millis(1050));
    put(
        &mut ch,
        json!({"t":"run","id":"later","pane":12,"command":"true"}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    assert_eq!(env.fake.inputs().len(), 4 + 20 + 1 + 1);
}

// --- done -------------------------------------------------------------------

fn running(id: u64) -> Block {
    Block {
        state: BlockState::Running,
        exit: None,
        ..block(id, "")
    }
}

#[test]
fn done_reaches_every_connection_including_commands_shorter_than_a_poll() {
    // a long probe interval: the fast command runs entirely between probes
    let env = start(|c| c.topology_poll = Duration::from_millis(300));
    env.fake.set(13, vec![block(1, "")]);
    let (_k, mut follower, _) = env.pair(Permission::View);
    let (_k2, mut other, _) = env.pair(Permission::Operate);
    put(
        &mut follower,
        json!({"t":"subscribe","pane":12,"after":null}),
    );
    recv_t(&mut follower, "snapshot");
    // let the probe see pane 13 as it stands
    std::thread::sleep(Duration::from_millis(700));
    // pane 13 (nobody follows it): a command appears already done
    let mut fast = block(2, "");
    fast.command = "true".into();
    fast.exit = Some(0);
    fast.duration_ms = 3;
    env.fake.edit(13, |b| b.push(fast));
    let want = json!({"t":"done","pane":13,"block":2,"command":"true","exit":0,"duration_ms":3});
    for ch in [&mut follower, &mut other] {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let v = recv(ch);
            if v["t"] == "done" {
                assert_eq!(v, want);
                break;
            }
            assert!(Instant::now() < deadline);
        }
    }
    // the probe of the unfollowed pane reads no output; the followed pane
    // is read whole
    let (meta, full) = env.fake.reads();
    assert!(meta.contains(&13), "{meta:?}");
    assert!(!full.contains(&13), "{full:?}");
    assert!(full.contains(&12), "{full:?}");
    // pane 12 (followed): running → done
    env.fake.edit(12, |b| b.push(running(3)));
    recv_t(&mut follower, "update");
    env.fake.edit(12, |b| {
        b[2].state = BlockState::Done;
        b[2].exit = Some(1);
    });
    let v = recv_t(&mut other, "done");
    assert_eq!(
        (v["pane"].as_u64(), v["block"].as_u64(), v["exit"].as_i64()),
        (Some(12), Some(3), Some(1))
    );
    // once only
    put(&mut other, json!({"t":"ping","n":1}));
    loop {
        let v = recv(&mut other);
        assert_ne!(v["t"], "done", "reported twice");
        if v["t"] == "pong" {
            break;
        }
    }
}

// --- reading (Phase 0 behaviour, now over Noise) -----------------------------

#[test]
fn subscribe_gets_a_snapshot() {
    let env = start(|_| {});
    let (_k, mut ws, _) = env.pair(Permission::View);
    put(&mut ws, json!({"t":"subscribe","pane":12,"after":null}));
    let s = recv_t(&mut ws, "snapshot");
    assert_eq!(s["pane"], 12);
    assert_eq!(s["seq"], 0);
    assert_eq!(s["has_older"], false);
    assert_eq!(ids(&s), [1, 2]);
    assert_eq!(s["blocks"][1]["output"], "two\n");
}

#[test]
fn source_changes_arrive_as_updates() {
    let env = start(|_| {});
    let fake = &env.fake;
    let (_k, mut ws, _) = env.pair(Permission::View);
    put(&mut ws, json!({"t":"subscribe","pane":12,"after":null}));
    assert_eq!(recv_t(&mut ws, "snapshot")["seq"], 0);
    fake.edit(12, |b| {
        b.remove(0);
        b[0].output = "two, more\n".into();
        b.push(block(3, "three\n"));
    });
    let u = recv_t(&mut ws, "update");
    assert_eq!(u["seq"], 1);
    assert_eq!(ids(&u), [2, 3]);
    assert_eq!(u["blocks"][0]["output"], "two, more\n");
    assert_eq!(u["removed"], json!([1]));
    fake.edit(12, |b| b[1].output = "three!\n".into());
    let u = recv_t(&mut ws, "update");
    assert_eq!((u["seq"].as_u64(), ids(&u)), (Some(2), vec![3]));
    assert_eq!(u["removed"], json!([]));
}

/// Wait until `observer` sees update `seq`.
fn await_seq(observer: &mut Ch, seq: u64) {
    let u = recv_t(observer, "update");
    assert_eq!(u["seq"], seq);
}

#[test]
fn resubscribe_with_after_replays_without_a_gap() {
    let env = start(|_| {});
    let fake = &env.fake;
    let (key, mut watcher, _) = env.pair(Permission::View);
    put(
        &mut watcher,
        json!({"t":"subscribe","pane":12,"after":null}),
    );
    recv_t(&mut watcher, "snapshot");
    let mut phone = env.login(&key);
    put(&mut phone, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut phone, "snapshot");
    for seq in 1..=3 {
        fake.edit(12, |b| b[1].output = format!("v{seq}\n"));
        await_seq(&mut watcher, seq);
        await_seq(&mut phone, seq);
    }
    phone.close();
    for seq in 4..=5 {
        fake.edit(12, |b| b[1].output = format!("v{seq}\n"));
        await_seq(&mut watcher, seq);
    }
    let mut phone = env.login(&key);
    put(&mut phone, json!({"t":"subscribe","pane":12,"after":3}));
    let u4 = recv_t(&mut phone, "update");
    let u5 = recv_t(&mut phone, "update");
    assert_eq!(u4["seq"], 4);
    assert_eq!(u5["seq"], 5);
    assert_eq!(u5["blocks"][0]["output"], "v5\n");
    fake.edit(12, |b| b[1].output = "v6\n".into());
    assert_eq!(recv_t(&mut phone, "update")["seq"], 6);
}

#[test]
fn after_older_than_the_ring_gets_a_snapshot() {
    let env = start(|_| {});
    let fake = &env.fake;
    let (key, mut watcher, _) = env.pair(Permission::View);
    put(
        &mut watcher,
        json!({"t":"subscribe","pane":12,"after":null}),
    );
    recv_t(&mut watcher, "snapshot");
    let n = rift_remote::REPLAY_RING as u64 + 4;
    for seq in 1..=n {
        fake.edit(12, |b| b[1].output = format!("v{seq}\n"));
        await_seq(&mut watcher, seq);
    }
    let mut phone = env.login(&key);
    put(&mut phone, json!({"t":"subscribe","pane":12,"after":2}));
    let s = recv_t(&mut phone, "snapshot");
    assert_eq!(s["seq"], n);
    assert_eq!(s["blocks"][1]["output"], format!("v{n}\n"));
    put(
        &mut phone,
        json!({"t":"subscribe","pane":12,"after":n - 256}),
    );
    let first = recv_t(&mut phone, "update");
    assert_eq!(first["seq"], n - 255);
    let mut other = env.login(&key);
    put(
        &mut other,
        json!({"t":"subscribe","pane":12,"after":n + 50}),
    );
    assert_eq!(recv_t(&mut other, "snapshot")["seq"], n);
}

#[test]
fn older_pages_back_30_at_a_time() {
    let env = start(|_| {});
    env.fake
        .set(12, (1..=70).map(|i| block(i, &format!("{i}\n"))).collect());
    let (_k, mut ws, _) = env.pair(Permission::View);
    put(&mut ws, json!({"t":"subscribe","pane":12,"after":null}));
    let s = recv_t(&mut ws, "snapshot");
    assert_eq!(ids(&s), (41..=70).collect::<Vec<_>>());
    assert_eq!(s["has_older"], true);
    put(&mut ws, json!({"t":"older","pane":12,"before":41}));
    let o = recv_t(&mut ws, "older");
    assert_eq!(ids(&o), (11..=40).collect::<Vec<_>>());
    assert_eq!(o["has_older"], true);
    put(&mut ws, json!({"t":"older","pane":12,"before":11}));
    let o = recv_t(&mut ws, "older");
    assert_eq!(ids(&o), (1..=10).collect::<Vec<_>>());
    assert_eq!(o["has_older"], false);
    put(&mut ws, json!({"t":"older","pane":99,"before":11}));
    assert_eq!(recv_t(&mut ws, "error")["code"], "no_pane");
}

#[test]
fn blocks_trimmed_below_the_window_are_removed_including_paged_ones() {
    let env = start(|_| {});
    let fake = &env.fake;
    fake.set(12, (1..=100).map(|i| block(i, &format!("{i}\n"))).collect());
    let (_k, mut ws, _) = env.pair(Permission::View);
    put(&mut ws, json!({"t":"subscribe","pane":12,"after":null}));
    assert_eq!(
        ids(&recv_t(&mut ws, "snapshot")),
        (71..=100).collect::<Vec<_>>()
    );
    put(&mut ws, json!({"t":"older","pane":12,"before":71}));
    assert_eq!(
        ids(&recv_t(&mut ws, "older")),
        (41..=70).collect::<Vec<_>>()
    );
    fake.edit(12, |b| b.retain(|b| b.id > 55));
    let u = recv_t(&mut ws, "update");
    assert_eq!(u["seq"], 1);
    assert_eq!(ids(&u), Vec::<u64>::new());
    assert_eq!(u["removed"], json!((41..=55).collect::<Vec<_>>()));
    fake.edit(12, |b| b.retain(|b| b.id > 60));
    let u = recv_t(&mut ws, "update");
    assert_eq!(
        (u["seq"].as_u64(), &u["removed"]),
        (Some(2), &json!([56, 57, 58, 59, 60]))
    );
}

#[test]
fn a_vanished_pane_ends_the_subscription_with_no_pane() {
    let env = start(|_| {});
    let (_k, mut ws, _) = env.pair(Permission::View);
    put(&mut ws, json!({"t":"subscribe","pane":99,"after":null}));
    assert_eq!(recv_t(&mut ws, "error")["code"], "no_pane");
    put(&mut ws, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ws, "snapshot");
    env.fake.close(12);
    assert_eq!(recv_t(&mut ws, "error")["code"], "no_pane");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let v = recv(&mut ws);
        if v["t"] == "topology" && v["windows"][0]["tabs"][0]["panes"] == json!([]) {
            break;
        }
        assert!(Instant::now() < deadline);
    }
}

#[test]
fn ping_gets_pong_and_silent_clients_are_dropped() {
    let env = start(|c| {
        c.ping_interval = Duration::from_millis(100);
        c.idle_timeout = Duration::from_millis(400);
    });
    let (key, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"ping","n":41}));
    assert_eq!(recv_t(&mut ch, "pong"), json!({"t":"pong","n":41}));
    // reading answers the gateway's pings: the client stays
    let until = Instant::now() + Duration::from_millis(800);
    let mut alive = 0;
    while Instant::now() < until {
        if let Recv::Alive = ch.recv() {
            alive += 1;
        }
    }
    assert!(alive >= 3, "{alive} pings");
    put(&mut ch, json!({"t":"ping","n":42}));
    assert_eq!(recv_t(&mut ch, "pong")["n"], 42);
    // a client that never reads (so never answers pings) is dropped
    let mut silent = env.login(&key);
    std::thread::sleep(Duration::from_millis(900));
    assert_closed(&mut silent);
}

// --- push alerts --------------------------------------------------------------

/// A loopback push service: every request body, each answered with
/// `reply(body)`.
struct PushSink {
    url: String,
    bodies: Arc<Mutex<Vec<Value>>>,
}

impl PushSink {
    fn start(reply: impl Fn(&Value) -> String + Send + 'static) -> PushSink {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/--/api/v2/push/send",
            listener.local_addr().unwrap()
        );
        let bodies: Arc<Mutex<Vec<Value>>> = Arc::default();
        let seen = bodies.clone();
        std::thread::spawn(move || {
            for s in listener.incoming() {
                let Ok(mut s) = s else { continue };
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; len];
                r.read_exact(&mut body).unwrap();
                let v: Value = serde_json::from_slice(&body).unwrap();
                let answer = reply(&v);
                seen.lock().unwrap().push(v);
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                );
            }
        });
        PushSink { url, bodies }
    }

    /// The `n`th request (waits up to 3 s).
    fn request(&self, n: usize) -> Value {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(v) = self.bodies.lock().unwrap().get(n) {
                return v.clone();
            }
            assert!(Instant::now() < deadline, "no push request {n} within 3 s");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn count(&self) -> usize {
        self.bodies.lock().unwrap().len()
    }
}

const TOKEN: &str = "ExponentPushToken[test-token_1]";

fn finished(id: u64, command: &str, exit: i32, duration_ms: u64) -> Block {
    Block {
        command: command.into(),
        exit: Some(exit),
        duration_ms,
        ..block(id, "")
    }
}

fn push_of(env: &Env, device: &str) -> Option<registry::PushReg> {
    registry::devices(&env.dir)
        .unwrap()
        .into_iter()
        .find(|d| d.id == device)
        .and_then(|d| d.push)
}

#[test]
fn a_device_with_a_push_token_is_alerted_while_its_app_is_closed() {
    let ok = r#"{"data":[{"status":"ok","id":"r"}]}"#;
    let sink = PushSink::start(move |_| ok.to_string());
    let url = sink.url.clone();
    let env = start(move |c| c.push_url = url);
    let (key, mut ch, welcome) = env.pair(Permission::View);
    let device = welcome["device"].as_str().unwrap().to_string();
    put(
        &mut ch,
        json!({"t":"push","token":TOKEN,"long_ms":30000,"show_command":false}),
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while push_of(&env, &device).is_none() {
        assert!(Instant::now() < deadline, "the token was not stored");
        std::thread::sleep(Duration::from_millis(10));
    }
    // let the probe see the panes as they stand, then close the app
    std::thread::sleep(Duration::from_millis(300));
    ch.close();
    std::thread::sleep(Duration::from_millis(300));
    // a short command: no alert
    env.fake.edit(12, |b| b.push(finished(3, "ls", 0, 20)));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(sink.count(), 0);
    // a long one: an alert without the command line
    env.fake
        .edit(12, |b| b.push(finished(4, "make secret-thing", 2, 31_000)));
    let host = registry::encode_key(&env.pinned);
    assert_eq!(
        sink.request(0),
        json!([{
            "to": TOKEN,
            "title": "A command failed on test-host (exit 2)",
            "data": {"host": host, "pane": 12},
            "priority": "high",
            "sound": "default",
        }])
    );
    // "Notify me" for a short command, with the command line when asked
    let mut ch = env.login(&key);
    put(
        &mut ch,
        json!({"t":"push","token":TOKEN,"long_ms":null,"show_command":true}),
    );
    put(&mut ch, json!({"t":"notify","pane":12}));
    std::thread::sleep(Duration::from_millis(300));
    ch.close();
    env.fake
        .edit(12, |b| b.push(finished(5, "cargo fmt", 0, 40)));
    let v = sink.request(1);
    assert_eq!(v[0]["title"], "A command finished on test-host");
    assert_eq!(v[0]["body"], "cargo fmt");
    // used up: the next short command does not alert
    env.fake.edit(12, |b| b.push(finished(6, "ls", 0, 40)));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(sink.count(), 2);
}

#[test]
fn push_tokens_are_validated_cleared_and_dropped_when_unregistered() {
    let gone =
        r#"{"data":[{"status":"error","message":"m","details":{"error":"DeviceNotRegistered"}}]}"#;
    let sink = PushSink::start(move |_| gone.to_string());
    let url = sink.url.clone();
    let env = start(move |c| c.push_url = url);
    let (_key, mut ch, welcome) = env.pair(Permission::Operate);
    let device = welcome["device"].as_str().unwrap().to_string();
    for bad in [
        json!({"t":"push","token":"fcm:abc","long_ms":null,"show_command":false}),
        json!({"t":"push","token":TOKEN,"long_ms":90_000_000u64,"show_command":false}),
    ] {
        put(&mut ch, bad);
        assert_eq!(recv_t(&mut ch, "error")["code"], "bad_request");
    }
    assert!(push_of(&env, &device).is_none());
    // a token, then null clears it
    put(
        &mut ch,
        json!({"t":"push","token":TOKEN,"long_ms":1,"show_command":false}),
    );
    put(&mut ch, json!({"t":"ping","n":1}));
    recv_t(&mut ch, "pong");
    assert!(push_of(&env, &device).is_some());
    put(
        &mut ch,
        json!({"t":"push","token":null,"long_ms":null,"show_command":false}),
    );
    put(&mut ch, json!({"t":"ping","n":2}));
    recv_t(&mut ch, "pong");
    assert!(push_of(&env, &device).is_none());
    // a token the push service no longer knows is removed after its alert
    put(
        &mut ch,
        json!({"t":"push","token":TOKEN,"long_ms":1,"show_command":false}),
    );
    put(&mut ch, json!({"t":"ping","n":3}));
    recv_t(&mut ch, "pong");
    std::thread::sleep(Duration::from_millis(300));
    env.fake.edit(12, |b| b.push(finished(3, "true", 0, 5)));
    sink.request(0);
    let deadline = Instant::now() + Duration::from_secs(3);
    while push_of(&env, &device).is_some() {
        assert!(Instant::now() < deadline, "the unregistered token stayed");
        std::thread::sleep(Duration::from_millis(10));
    }
}
