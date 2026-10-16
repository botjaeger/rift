//! The gateway over real localhost sockets, fed by a fake source; the test
//! acts as the phone with `snow` through `rift_remote::transport::connect`.

use rift_remote::protocol::{
    Block, BlockState, ClientHello, CloseKind, OpenKind, Pane, Permission, Run, Tab, Window,
};
use rift_remote::protocol::{
    ChangedFile, Completion, CompletionKind, DiffLine, HistoryEntry, Waiting, WaitingKind,
};
use rift_remote::registry::{self, PairingLink};
use rift_remote::transport::{self, Channel, HandshakeError, Recv};
use rift_remote::{
    CompletePage, Config, DiffPage, Gateway, Input, InputError, Key, KeyBase, Menu, MenuKind,
    OutputError, OutputPage, Page, Screen, Source, SourceError, serve,
};
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
    /// Each pane's screen (`screen` answers it when `alt`, or forced).
    screens: BTreeMap<u64, Screen>,
    /// Panes read through `screen`, with `force`.
    screen_reads: Vec<(u64, bool)>,
    /// Output of (pane, block): lines already gone, then the held lines
    /// (each ending with `\n`).
    outputs: BTreeMap<(u64, u64), (u64, Vec<String>)>,
    /// What each pane waits for (topology `waiting`).
    waiting: BTreeMap<u64, Waiting>,
    /// What `complete` was asked: (pane, line, cursor).
    completes: Vec<(u64, String, usize)>,
    /// What `open` was asked: (pane, device name, kind).
    opens: Vec<(u64, String, OpenKind)>,
    /// What `close` was asked: (pane, device name, kind).
    closes: Vec<(u64, String, CloseKind)>,
    /// `start` allowed (the computer's `remote_when_closed`).
    can_start: bool,
    /// Devices that started a shell.
    starts: Vec<String>,
    /// What `fit` was told: (pane, size or release).
    fits: Vec<(u64, Option<(u16, u16)>)>,
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
    fn fits(&self) -> Vec<(u64, Option<(u16, u16)>)> {
        self.0.lock().unwrap().fits.clone()
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
            background: false,
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
                        git: None,
                        waiting: st.waiting.get(&id).cloned(),
                        running: None,
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

    fn can_start(&self) -> bool {
        self.0.lock().unwrap().can_start
    }

    fn fit(&self, pane: u64, size: Option<(u16, u16)>) -> Result<(), InputError> {
        let mut st = self.0.lock().unwrap();
        if !st.panes.contains_key(&pane) {
            return Err(InputError::NoPane);
        }
        st.fits.push((pane, size));
        Ok(())
    }

    /// A new empty pane, numbered after the highest.
    fn start(&self, device: &str) -> Result<u64, InputError> {
        let mut st = self.0.lock().unwrap();
        if !st.can_start {
            return Err(InputError::Denied);
        }
        let new = st.panes.keys().max().unwrap() + 1;
        st.panes.insert(new, Vec::new());
        st.starts.push(device.to_string());
        Ok(new)
    }

    /// `Pane` removes the pane; `Tab` every pane (the fake has one tab).
    fn close(&self, pane: u64, device: &str, kind: CloseKind) -> Result<(), InputError> {
        let mut st = self.0.lock().unwrap();
        if !st.panes.contains_key(&pane) {
            return Err(InputError::NoPane);
        }
        match kind {
            CloseKind::Pane => st.panes.remove(&pane),
            CloseKind::Tab => {
                st.panes.clear();
                None
            }
        };
        st.closes.push((pane, device.to_string(), kind));
        Ok(())
    }

    /// A new empty pane, numbered after the highest.
    fn open(&self, pane: u64, device: &str, kind: OpenKind) -> Result<u64, InputError> {
        let mut st = self.0.lock().unwrap();
        if !st.panes.contains_key(&pane) {
            return Err(InputError::NoPane);
        }
        let new = st.panes.keys().max().unwrap() + 1;
        st.panes.insert(new, Vec::new());
        st.opens.push((pane, device.to_string(), kind));
        Ok(new)
    }

    fn screen(&self, pane: u64, force: bool) -> Result<Option<Screen>, SourceError> {
        let mut st = self.0.lock().unwrap();
        if !st.panes.contains_key(&pane) {
            return Err(SourceError::NoPane);
        }
        st.screen_reads.push((pane, force));
        Ok(st.screens.get(&pane).filter(|s| s.alt || force).cloned())
    }

    /// Pane 12's chip menus, larger than the wire allows: the gateway
    /// bounds them.
    fn menu(&self, pane: u64, kind: MenuKind) -> Result<Menu, OutputError> {
        if !self.0.lock().unwrap().panes.contains_key(&pane) {
            return Err(OutputError::NoPane);
        }
        let mut names: Vec<String> = (0..600).map(|i| format!("d{i:03}")).collect();
        names.insert(1, "x".repeat(2000));
        Ok(match kind {
            MenuKind::Dirs => Menu::Dirs {
                cwd: Some("/w/rift".into()),
                parent: true,
                dirs: names,
            },
            MenuKind::Branches => Menu::Branches {
                current: Some("main".into()),
                branches: vec!["main".into(), "feat".into()],
            },
            MenuKind::Changes => Menu::Changes {
                branch: Some("main".into()),
                files: vec![ChangedFile {
                    path: "a.rs".into(),
                    status: "M".into(),
                    added: 3,
                    removed: 1,
                    staged: false,
                }],
            },
        })
    }

    /// More completions than the wire allows, one of them over-long.
    fn complete(&self, pane: u64, line: &str, cursor: usize) -> Result<CompletePage, OutputError> {
        let mut st = self.0.lock().unwrap();
        if !st.panes.contains_key(&pane) {
            return Err(OutputError::NoPane);
        }
        st.completes.push((pane, line.to_string(), cursor));
        let mut items: Vec<Completion> = (0..80)
            .map(|i| Completion {
                text: format!("{line}{i}"),
                kind: CompletionKind::Subcommand,
            })
            .collect();
        items.insert(
            1,
            Completion {
                text: "x".repeat(2000),
                kind: CompletionKind::Path,
            },
        );
        Ok(CompletePage {
            items,
            history: vec![HistoryEntry {
                text: format!("{line} --all"),
                at: Some(1_760_000_000),
            }],
        })
    }

    /// `a.rs` (unstaged) has a diff longer than the wire allows.
    fn diff(&self, pane: u64, path: &str, staged: bool) -> Result<DiffPage, OutputError> {
        if !self.0.lock().unwrap().panes.contains_key(&pane) {
            return Err(OutputError::NoPane);
        }
        match (path, staged) {
            ("a.rs", false) => {}
            // untracked: its whole content
            (".env", false) => {
                return Ok(DiffPage {
                    untracked: true,
                    lines: vec![
                        DiffLine("@".into(), "@@ -0,0 +1 @@".into()),
                        DiffLine("+".into(), "TOKEN=secret".into()),
                    ],
                    ..Default::default()
                });
            }
            // within the bounds, but each control character is six bytes
            // of JSON: the frame is over the gateway's message limit
            ("ctl.txt", false) => {
                return Ok(DiffPage {
                    lines: (0..250)
                        .map(|_| DiffLine("+".into(), "\u{1}".repeat(1000)))
                        .collect(),
                    ..Default::default()
                });
            }
            _ => return Err(OutputError::NoBlock),
        }
        let mut lines = vec![DiffLine("@".into(), "@@ -1 +1 @@".into())];
        lines.push(DiffLine("+".into(), "y".repeat(1500)));
        lines.extend((0..3000).map(|i| DiffLine(" ".into(), format!("line {i}"))));
        Ok(DiffPage {
            lines,
            ..Default::default()
        })
    }

    /// Every held line from `from` on, whatever `limit`: the gateway bounds
    /// the page.
    fn output(
        &self,
        pane: u64,
        block: u64,
        from: u64,
        _limit: usize,
    ) -> Result<OutputPage, OutputError> {
        let st = self.0.lock().unwrap();
        if !st.panes.contains_key(&pane) {
            return Err(OutputError::NoPane);
        }
        let (gone, lines) = st.outputs.get(&(pane, block)).ok_or(OutputError::NoBlock)?;
        let total = gone + lines.len() as u64;
        let from = from.max(*gone);
        let held = lines.get((from - gone) as usize..).unwrap_or_default();
        Ok(OutputPage {
            from,
            text: held.concat(),
            total,
        })
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
    // the upgrade blocks (a short timeout here breaks it as Interrupted on a
    // loaded CI runner); reads after it poll every 50 ms
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let ws = tungstenite::client(format!("ws://{addr}/"), stream)
        .unwrap()
        .0;
    ws.get_ref()
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    ws
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

/// Next JSON message within 10 s: a hang detector, not a speed check. The
/// slowest wait measured here is 0.2 s, but a loaded CI runner took over 3 s
/// for the ~1 MiB frames of `a_slow_client_gets_the_newest_screen_…`.
fn recv(ch: &mut Ch) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "no message within 10 s");
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
               "device":d.id,"permission":"operate","features":["screen","text","output","push_failures","push_ntfy","git","notify_off","chips","waiting","complete","running","open","close","fit"]})
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
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "forbidden"));
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
                    Key::ctrl('c'),
                    Key::plain(KeyBase::Esc),
                    Key::plain(KeyBase::Up),
                    Key::plain(KeyBase::Enter),
                    Key {
                        shift: true,
                        ..Key::plain(KeyBase::Tab)
                    }
                ])
            ),
        ]
    );
}

/// Panes of a `topology` message.
fn topology_panes(v: &Value) -> Vec<u64> {
    v["windows"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|w| w["tabs"].as_array().unwrap())
        .flat_map(|t| t["panes"].as_array().unwrap())
        .map(|p| p["id"].as_u64().unwrap())
        .collect()
}

#[test]
fn open_answers_the_new_pane_after_a_topology_with_it() {
    // no topology poll during the test: only `open` can tell the new pane
    let env = start(|c| c.topology_poll = Duration::from_secs(60));
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    for (rid, kind, want) in [("n1", "tab", 13), ("n2", "pane", 14)] {
        put(&mut ch, json!({"t":"open","id":rid,"pane":12,"kind":kind}));
        let mut told = false;
        let v = loop {
            let v = recv(&mut ch);
            match v["t"].as_str() {
                Some("topology") => told |= topology_panes(&v).contains(&want),
                Some("ack") => break v,
                _ => {}
            }
        };
        assert_eq!(v, json!({"t":"ack","id":rid,"ok":true,"pane":want}));
        assert!(
            told,
            "{kind}: the topology with pane {want} comes before the ack"
        );
    }
    assert_eq!(
        env.fake.0.lock().unwrap().opens,
        [
            (12, "Test Phone".to_string(), OpenKind::Tab),
            (12, "Test Phone".to_string(), OpenKind::Pane),
        ]
    );
    // a third within the second is too many
    put(
        &mut ch,
        json!({"t":"open","id":"n3","pane":12,"kind":"tab"}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "rate"));
    std::thread::sleep(Duration::from_millis(1050));
    put(
        &mut ch,
        json!({"t":"open","id":"n4","pane":99,"kind":"tab"}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "no_pane"));
    put(
        &mut ch,
        json!({"t":"open","id":"n5","pane":12,"kind":"window"}),
    );
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "bad_request"));
    assert_eq!(env.fake.0.lock().unwrap().opens.len(), 2);
}

#[test]
fn close_answers_after_a_topology_without_the_pane() {
    let env = start(|c| c.topology_poll = Duration::from_secs(60));
    let (_k, mut ch, welcome) = env.pair(Permission::Operate);
    assert!(
        welcome["features"]
            .as_array()
            .unwrap()
            .contains(&json!("close")),
        "{welcome}"
    );
    // open a second pane, then close it: the topology without it comes first
    put(
        &mut ch,
        json!({"t":"open","id":"n1","pane":12,"kind":"pane"}),
    );
    recv_t(&mut ch, "ack");
    put(
        &mut ch,
        json!({"t":"close","id":"c1","pane":13,"kind":"pane"}),
    );
    let mut told = false;
    let v = loop {
        let v = recv(&mut ch);
        match v["t"].as_str() {
            Some("topology") => told = !topology_panes(&v).contains(&13),
            Some("ack") => break v,
            _ => {}
        }
    };
    assert_eq!(v, json!({"t":"ack","id":"c1","ok":true}));
    assert!(told, "the topology without pane 13 comes before the ack");
    assert_eq!(
        env.fake.0.lock().unwrap().closes,
        [(13, "Test Phone".to_string(), CloseKind::Pane)]
    );
    // a pane that is gone
    std::thread::sleep(Duration::from_millis(1050));
    put(
        &mut ch,
        json!({"t":"close","id":"c2","pane":13,"kind":"pane"}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "no_pane"));
    // the whole tab
    put(
        &mut ch,
        json!({"t":"close","id":"c3","pane":12,"kind":"tab"}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    assert_eq!(
        env.fake.0.lock().unwrap().closes.last(),
        Some(&(12, "Test Phone".to_string(), CloseKind::Tab))
    );
    // an unknown kind is a bad request
    std::thread::sleep(Duration::from_millis(1050));
    put(
        &mut ch,
        json!({"t":"close","id":"c4","pane":12,"kind":"window"}),
    );
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "bad_request"));
}

#[test]
fn view_devices_are_forbidden_to_close() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(
        &mut ch,
        json!({"t":"close","id":"c1","pane":12,"kind":"pane"}),
    );
    let v = recv_t(&mut ch, "ack");
    assert_eq!(
        (ack(&v), v["id"].as_str()),
        ((false, "forbidden"), Some("c1"))
    );
    assert!(env.fake.0.lock().unwrap().closes.is_empty());
}

#[test]
fn start_is_offered_only_where_the_computer_allows_it() {
    let env = start(|c| c.topology_poll = Duration::from_secs(60));
    // not allowed: not in the welcome, and refused
    let (_k, mut ch, welcome) = env.pair(Permission::Operate);
    assert!(
        !welcome["features"]
            .as_array()
            .unwrap()
            .contains(&json!("start")),
        "{welcome}"
    );
    put(&mut ch, json!({"t":"start","id":"s1"}));
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "forbidden"));
    // allowed: in the welcome; the new pane's topology comes before the ack
    env.fake.0.lock().unwrap().can_start = true;
    let (_k, mut ch, welcome) = env.pair(Permission::Operate);
    assert!(
        welcome["features"]
            .as_array()
            .unwrap()
            .contains(&json!("start")),
        "{welcome}"
    );
    put(&mut ch, json!({"t":"start","id":"s2"}));
    let mut told = false;
    let v = loop {
        let v = recv(&mut ch);
        match v["t"].as_str() {
            Some("topology") => told |= topology_panes(&v).contains(&13),
            Some("ack") => break v,
            _ => {}
        }
    };
    assert_eq!(v, json!({"t":"ack","id":"s2","ok":true,"pane":13}));
    assert!(told, "the topology with the new pane comes before the ack");
    // a view device may not
    let (_k, mut view, _) = env.pair(Permission::View);
    put(&mut view, json!({"t":"start","id":"s3"}));
    assert_eq!(ack(&recv_t(&mut view, "ack")), (false, "forbidden"));
    assert_eq!(env.fake.0.lock().unwrap().starts, ["Test Phone"]);
}

#[test]
fn view_devices_are_forbidden_to_open() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(
        &mut ch,
        json!({"t":"open","id":"n1","pane":12,"kind":"pane"}),
    );
    let v = recv_t(&mut ch, "ack");
    assert_eq!(
        (ack(&v), v["id"].as_str()),
        ((false, "forbidden"), Some("n1"))
    );
    assert!(env.fake.0.lock().unwrap().opens.is_empty());
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
fn view_devices_are_forbidden_to_run_send_keys_or_type() {
    let env = start(|_| {});
    let (_k, mut ch, welcome) = env.pair(Permission::View);
    assert_eq!(welcome["permission"], "view");
    for (rid, msg) in [
        ("r1", json!({"t":"run","id":"r1","pane":12,"command":"ls"})),
        (
            "k1",
            json!({"t":"keys","id":"k1","pane":12,"keys":["ctrl+c"]}),
        ),
        ("x1", json!({"t":"text","id":"x1","pane":12,"text":"y"})),
    ] {
        put(&mut ch, msg);
        let v = recv_t(&mut ch, "ack");
        assert_eq!(
            (ack(&v), v["id"].as_str()),
            ((false, "forbidden"), Some(rid))
        );
    }
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
        json!(["f13"]),
        json!(["shift+a"]),
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
fn a_megabyte_command_is_cut_and_never_hides_its_pane() {
    let env = start(|_| {});
    let huge = "é".repeat(1024 * 1024);
    env.fake.edit(12, |b| b[1].command = huge.clone());
    let (_k, mut ws, _) = env.pair(Permission::View);
    put(&mut ws, json!({"t":"subscribe","pane":12,"after":null}));
    let s = recv_t(&mut ws, "snapshot");
    assert_eq!(ids(&s), [1, 2]);
    assert_eq!(s["has_older"], false);
    let cut = s["blocks"][1]["command"].as_str().unwrap();
    assert_eq!(cut.len(), rift_remote::MAX_BLOCK_COMMAND);
    assert!(huge.starts_with(cut));
    // its update fits too
    env.fake.edit(12, |b| {
        b[1].output = "two, more\n".into();
        b.push(block(3, "three\n"));
    });
    let u = recv_t(&mut ws, "update");
    assert_eq!(ids(&u), [2, 3]);
    // and an `older` page with it
    put(&mut ws, json!({"t":"older","pane":12,"before":3}));
    let o = recv_t(&mut ws, "older");
    assert_eq!(ids(&o), [1, 2]);
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

/// One request a [`PushSink`] received.
#[derive(Clone)]
struct Seen {
    /// `POST /path HTTP/1.1`
    line: String,
    /// `name: value`, as sent.
    headers: Vec<String>,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find_map(|h| {
            let (n, v) = h.split_once(':')?;
            n.eq_ignore_ascii_case(name).then(|| v.trim())
        })
    }
}

/// A loopback push service: every request, each answered with
/// `reply(body)` (the body as JSON; `null` when it is not JSON).
struct PushSink {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl PushSink {
    fn start(reply: impl Fn(&Value) -> String + Send + 'static) -> PushSink {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/--/api/v2/push/send",
            listener.local_addr().unwrap()
        );
        let all: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let seen = all.clone();
        std::thread::spawn(move || {
            for s in listener.incoming() {
                let Ok(mut s) = s else { continue };
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut len = 0usize;
                let mut request_line = String::new();
                r.read_line(&mut request_line).unwrap_or(0);
                let mut headers = Vec::new();
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    headers.push(line.trim_end().to_string());
                }
                let mut body = vec![0u8; len];
                r.read_exact(&mut body).unwrap();
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let answer = reply(&v);
                seen.lock().unwrap().push(Seen {
                    line: request_line.trim_end().to_string(),
                    headers,
                    body: String::from_utf8(body).unwrap(),
                });
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                );
            }
        });
        PushSink { url, seen: all }
    }

    /// The `n`th request (waits up to 3 s).
    fn raw(&self, n: usize) -> Seen {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(v) = self.seen.lock().unwrap().get(n) {
                return v.clone();
            }
            assert!(Instant::now() < deadline, "no push request {n} within 3 s");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The `n`th request's JSON body (waits up to 3 s).
    fn request(&self, n: usize) -> Value {
        serde_json::from_str(&self.raw(n).body).unwrap()
    }

    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
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
        .and_then(|d| d.alerts())
}

/// Post ntfy alerts for topics on ntfy.sh to `sink` instead.
fn to_sink(sink: &str) -> Vec<(String, String)> {
    vec![("https://ntfy.sh/".to_string(), format!("{sink}/"))]
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
            "title": "✕ 2 · A command",
            "subtitle": "Failed after 31s · test-host",
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
    assert_eq!(v[0]["title"], "✓ 0 · cargo fmt");
    assert_eq!(v[0]["subtitle"], "Finished in 40ms · test-host");
    assert!(v[0].get("body").is_none(), "never output");
    // used up: the next short command does not alert
    env.fake.edit(12, |b| b.push(finished(6, "ls", 0, 40)));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(sink.count(), 2);
    // asked, then switched off (the bell on a running block): no alert
    let mut ch = env.login(&key);
    put(&mut ch, json!({"t":"notify","pane":12}));
    put(&mut ch, json!({"t":"notify","pane":12,"off":true}));
    std::thread::sleep(Duration::from_millis(300));
    ch.close();
    env.fake.edit(12, |b| b.push(finished(7, "make", 0, 40)));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(sink.count(), 2, "a wait switched off does not alert");
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

#[test]
fn failures_only_alerts_long_failures_but_not_long_successes() {
    let ok = r#"{"data":[{"status":"ok","id":"r"}]}"#;
    let sink = PushSink::start(move |_| ok.to_string());
    let url = sink.url.clone();
    let env = start(move |c| c.push_url = url);
    let (_key, mut ch, welcome) = env.pair(Permission::View);
    let device = welcome["device"].as_str().unwrap().to_string();
    put(
        &mut ch,
        json!({"t":"push","token":TOKEN,"long_ms":1000,"show_command":false,"failures_only":true}),
    );
    put(&mut ch, json!({"t":"ping","n":1}));
    recv_t(&mut ch, "pong");
    assert!(push_of(&env, &device).is_some_and(|p| p.failures_only));
    std::thread::sleep(Duration::from_millis(300));
    ch.close();
    std::thread::sleep(Duration::from_millis(300));
    // long and successful: no alert
    env.fake.edit(12, |b| b.push(finished(3, "make", 0, 5_000)));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(sink.count(), 0);
    // long and failed: an alert
    env.fake.edit(12, |b| b.push(finished(4, "make", 1, 5_000)));
    assert_eq!(sink.request(0)[0]["title"], "✕ 1 · A command");
}

const NTFY: &str = "https://ntfy.sh/rift-x7Kq2mP9vT4wL8nR3sY6hZ";

/// An ntfy request's Title header, RFC 2047 decoded (the marks are not ASCII).
fn ntfy_title(r: &Seen) -> String {
    use base64::Engine;
    let t = r.header("Title").expect("a Title header");
    let Some(b64) = t
        .strip_prefix("=?UTF-8?B?")
        .and_then(|t| t.strip_suffix("?="))
    else {
        return t.to_string();
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap();
    String::from_utf8(bytes).unwrap()
}

/// Pair a device, register `push`, wait until it is stored, close the app.
fn register_then_close(env: &Env, push: Value) -> String {
    let (_key, mut ch, welcome) = env.pair(Permission::View);
    let device = welcome["device"].as_str().unwrap().to_string();
    put(&mut ch, push);
    put(&mut ch, json!({"t":"ping","n":1}));
    recv_t(&mut ch, "pong");
    assert!(
        push_of(env, &device).is_some(),
        "the registration was not stored"
    );
    std::thread::sleep(Duration::from_millis(300));
    ch.close();
    std::thread::sleep(Duration::from_millis(300));
    device
}

#[test]
fn a_device_with_an_ntfy_url_is_alerted_through_ntfy_while_its_app_is_closed() {
    let ntfy = PushSink::start(|_| "{}".to_string());
    let to = ntfy.url.clone();
    let env = start(move |c| c.ntfy_redirect = to_sink(&to));
    let device = register_then_close(
        &env,
        json!({"t":"push","token":null,"ntfy":NTFY,"long_ms":30000,"show_command":false,"failures_only":false}),
    );
    assert_eq!(push_of(&env, &device).unwrap().ntfy.as_deref(), Some(NTFY));
    // a short command: no alert
    env.fake.edit(12, |b| b.push(finished(3, "ls", 0, 20)));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(ntfy.count(), 0);
    // a long failed one: an alert without the command line
    env.fake
        .edit(12, |b| b.push(finished(4, "make secret-thing", 2, 31_000)));
    let host = registry::encode_key(&env.pinned);
    let r = ntfy.raw(0);
    assert!(r.line.starts_with("POST "), "{}", r.line);
    assert_eq!(r.body, "Failed after 31s · test-host");
    assert_eq!(ntfy_title(&r), "✕ 2 · A command");
    assert_eq!(r.header("Priority"), Some("high"));
    assert_eq!(r.header("Tags"), Some("x"));
    let link = format!("rift://host/{host}/pane/12");
    assert_eq!(r.header("Click"), Some(link.as_str()));
    assert!(!r.body.contains("secret") && !r.headers.iter().any(|h| h.contains("secret")));
    // a long success (once that post ended: one at a time per device)
    std::thread::sleep(Duration::from_millis(200));
    env.fake
        .edit(12, |b| b.push(finished(5, "make", 0, 31_000)));
    let r = ntfy.raw(1);
    assert_eq!(r.body, "Finished in 31s · test-host");
    assert_eq!(ntfy_title(&r), "✓ 0 · A command");
    assert_eq!(r.header("Priority"), Some("default"));
    assert_eq!(r.header("Tags"), Some("white_check_mark"));
}

#[test]
fn a_device_with_a_token_and_an_ntfy_url_gets_both_with_the_command_when_asked() {
    let ok = r#"{"data":[{"status":"ok","id":"r"}]}"#;
    let expo = PushSink::start(move |_| ok.to_string());
    let ntfy = PushSink::start(|_| "{}".to_string());
    let (url, to) = (expo.url.clone(), ntfy.url.clone());
    let env = start(move |c| {
        c.push_url = url;
        c.ntfy_redirect = to_sink(&to);
    });
    register_then_close(
        &env,
        json!({"t":"push","token":TOKEN,"ntfy":NTFY,"long_ms":1000,"show_command":true,"failures_only":true}),
    );
    // failures_only: a long success alerts neither
    env.fake.edit(12, |b| b.push(finished(3, "make", 0, 5_000)));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!((expo.count(), ntfy.count()), (0, 0));
    env.fake
        .edit(12, |b| b.push(finished(4, "cargo test", 101, 5_000)));
    assert_eq!(expo.request(0)[0]["title"], "✕ 101 · cargo test");
    assert_eq!(
        expo.request(0)[0]["subtitle"],
        "Failed after 5.0s · test-host"
    );
    let r = ntfy.raw(0);
    assert_eq!(r.body, "Failed after 5.0s · test-host");
    assert_eq!(ntfy_title(&r), "✕ 101 · cargo test");
}

#[test]
fn an_unregistered_token_is_dropped_but_the_ntfy_url_next_to_it_stays() {
    let gone =
        r#"{"data":[{"status":"error","message":"m","details":{"error":"DeviceNotRegistered"}}]}"#;
    let expo = PushSink::start(move |_| gone.to_string());
    let ntfy = PushSink::start(|_| "{}".to_string());
    let (url, to) = (expo.url.clone(), ntfy.url.clone());
    let env = start(move |c| {
        c.push_url = url;
        c.ntfy_redirect = to_sink(&to);
    });
    let device = register_then_close(
        &env,
        json!({"t":"push","token":TOKEN,"ntfy":NTFY,"long_ms":1,"show_command":false}),
    );
    env.fake.edit(12, |b| b.push(finished(3, "true", 0, 5)));
    expo.request(0);
    ntfy.raw(0);
    let deadline = Instant::now() + Duration::from_secs(3);
    while push_of(&env, &device).is_some_and(|p| p.token.is_some()) {
        assert!(Instant::now() < deadline, "the unregistered token stayed");
        std::thread::sleep(Duration::from_millis(10));
    }
    let reg = push_of(&env, &device).expect("the ntfy registration was removed too");
    assert_eq!(reg.ntfy.as_deref(), Some(NTFY));
    // still alerted through ntfy (once the gateway re-read devices.json: 100 ms here)
    std::thread::sleep(Duration::from_millis(400));
    env.fake.edit(12, |b| b.push(finished(4, "true", 0, 5)));
    ntfy.raw(1);
    assert_eq!(expo.count(), 1, "no Expo post without a token");
}

#[test]
fn ntfy_urls_are_validated_and_cleared() {
    let env = start(|_| {});
    let (_key, mut ch, welcome) = env.pair(Permission::Operate);
    let device = welcome["device"].as_str().unwrap().to_string();
    for bad in [
        "http://ntfy.sh/topic",
        "https://ntfy.sh/topic?x=1",
        "https://u:p@ntfy.sh/topic",
        "https://ntfy.sh/a/b",
        "https://localhost/topic",
        "https://127.0.0.1:8080/topic",
        "https://169.254.169.254/topic",
    ] {
        put(
            &mut ch,
            json!({"t":"push","token":null,"ntfy":bad,"long_ms":null,"show_command":false}),
        );
        assert_eq!(recv_t(&mut ch, "error")["code"], "bad_request", "{bad}");
    }
    // a valid URL with a bad token is refused whole
    put(
        &mut ch,
        json!({"t":"push","token":"fcm:x","ntfy":NTFY,"long_ms":null,"show_command":false}),
    );
    assert_eq!(recv_t(&mut ch, "error")["code"], "bad_request");
    assert!(push_of(&env, &device).is_none());
    put(
        &mut ch,
        json!({"t":"push","token":null,"ntfy":NTFY,"long_ms":null,"show_command":false}),
    );
    put(&mut ch, json!({"t":"ping","n":1}));
    recv_t(&mut ch, "pong");
    let reg = push_of(&env, &device).unwrap();
    assert_eq!((reg.token, reg.ntfy.as_deref()), (None, Some(NTFY)));
    // "Notify me" works with ntfy alone (no error)
    put(&mut ch, json!({"t":"notify","pane":12}));
    put(&mut ch, json!({"t":"ping","n":2}));
    assert_eq!(recv(&mut ch)["t"], "pong");
    // both null: off
    put(
        &mut ch,
        json!({"t":"push","token":null,"ntfy":null,"long_ms":null,"show_command":false}),
    );
    put(&mut ch, json!({"t":"ping","n":3}));
    recv_t(&mut ch, "pong");
    assert!(push_of(&env, &device).is_none());
}

#[test]
fn ntfy_alerts_are_rate_limited_per_device() {
    let ntfy = PushSink::start(|_| "{}".to_string());
    let to = ntfy.url.clone();
    let env = start(move |c| c.ntfy_redirect = to_sink(&to));
    register_then_close(
        &env,
        json!({"t":"push","token":null,"ntfy":NTFY,"long_ms":1,"show_command":false,"failures_only":false}),
    );
    for id in 3..3 + 13 {
        env.fake.edit(12, |b| b.push(finished(id, "true", 0, 5)));
        let want = (id as usize - 2).min(12);
        let deadline = Instant::now() + Duration::from_secs(3);
        while ntfy.count() < want {
            assert!(Instant::now() < deadline, "alert {want} did not come");
            std::thread::sleep(Duration::from_millis(10));
        }
        // let that post's curl end: one post per device is under way at a time
        std::thread::sleep(Duration::from_millis(200));
    }
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(ntfy.count(), 12, "at most 12 a minute");
}

#[test]
fn the_pane_a_device_follows_gets_no_ntfy_banner_unless_asked() {
    let ok = r#"{"data":[{"status":"ok","id":"r"}]}"#;
    let expo = PushSink::start(move |_| ok.to_string());
    let ntfy = PushSink::start(|_| "{}".to_string());
    let (url, to) = (expo.url.clone(), ntfy.url.clone());
    let env = start(move |c| {
        c.push_url = url;
        c.ntfy_redirect = to_sink(&to);
        c.topology_poll = Duration::from_millis(100);
    });
    env.fake.set(13, vec![block(1, "")]);
    // an ntfy-only device that stays connected, following pane 12
    let (_key, mut ch, _) = env.pair(Permission::View);
    put(
        &mut ch,
        json!({"t":"push","token":null,"ntfy":NTFY,"long_ms":1,"show_command":false}),
    );
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    // a device with both, following pane 12 too
    let (_key2, mut ch2, _) = env.pair(Permission::View);
    put(
        &mut ch2,
        json!({"t":"push","token":TOKEN,"ntfy":NTFY,"long_ms":1,"show_command":false}),
    );
    put(&mut ch2, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch2, "snapshot");
    std::thread::sleep(Duration::from_millis(300));
    // the followed pane: no ntfy post for either; the token still gets Expo
    env.fake.edit(12, |b| b.push(finished(3, "make", 1, 5_000)));
    expo.request(0);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(ntfy.count(), 0, "an ntfy banner about the pane on screen");
    assert_eq!(expo.count(), 1);
    // another pane: both devices get their ntfy post
    env.fake.edit(13, |b| b.push(finished(9, "make", 1, 5_000)));
    ntfy.raw(1);
    assert!(
        (0..2).all(|i| ntfy
            .raw(i)
            .header("Click")
            .is_some_and(|c| c.ends_with("/pane/13"))),
        "only pane 13 went to ntfy"
    );
    // asked ("Notify me") about the followed pane: posted all the same
    put(&mut ch, json!({"t":"notify","pane":12}));
    put(&mut ch, json!({"t":"ping","n":1}));
    while recv(&mut ch)["t"] != "pong" {}
    std::thread::sleep(Duration::from_millis(200));
    env.fake.edit(12, |b| b.push(finished(4, "make", 0, 5)));
    let r = ntfy.raw(2);
    assert!(r.header("Click").is_some_and(|c| c.ends_with("/pane/12")));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(ntfy.count(), 3, "only the asking device's ntfy post");
}

#[test]
fn a_followed_pane_gets_ntfy_alerts_while_the_app_is_not_on_screen() {
    let ntfy = PushSink::start(|_| "{}".to_string());
    let to = ntfy.url.clone();
    let env = start(move |c| c.ntfy_redirect = to_sink(&to));
    let (_key, mut ch, _) = env.pair(Permission::View);
    put(
        &mut ch,
        json!({"t":"push","token":null,"ntfy":NTFY,"long_ms":1,"show_command":false}),
    );
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    // backgrounded with the socket still up (iOS suspends the app): alerted
    put(&mut ch, json!({"t":"visible","on":false}));
    put(&mut ch, json!({"t":"ping","n":1}));
    recv_t(&mut ch, "pong");
    env.fake.edit(12, |b| b.push(finished(3, "make", 1, 5_000)));
    let r = ntfy.raw(0);
    assert!(r.header("Click").is_some_and(|c| c.ends_with("/pane/12")));
    // on screen again: the pane it follows gets no banner
    put(&mut ch, json!({"t":"visible","on":true}));
    put(&mut ch, json!({"t":"ping","n":2}));
    while recv(&mut ch)["t"] != "pong" {}
    env.fake.edit(12, |b| b.push(finished(4, "make", 1, 5_000)));
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(ntfy.count(), 1, "a banner about the pane on screen");
}

#[test]
fn a_hanging_ntfy_server_delays_no_other_device_and_takes_one_post_at_a_time() {
    use std::io::Read;
    // a server that accepts and never answers
    let hang = TcpListener::bind("127.0.0.1:0").unwrap();
    let hang_url = format!("http://{}", hang.local_addr().unwrap());
    let held: Arc<Mutex<Vec<std::net::TcpStream>>> = Arc::default();
    let keep = held.clone();
    std::thread::spawn(move || {
        for mut s in hang.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
            let _ = s.read(&mut buf);
            keep.lock().unwrap().push(s);
        }
    });
    let ok = r#"{"data":[{"status":"ok","id":"r"}]}"#;
    let expo = PushSink::start(move |_| ok.to_string());
    let ntfy = PushSink::start(|_| "{}".to_string());
    let (url, to) = (expo.url.clone(), ntfy.url.clone());
    let env = start(move |c| {
        c.push_url = url;
        c.ntfy_redirect = vec![
            ("https://hang.example/".into(), format!("{hang_url}/")),
            ("https://ntfy.sh/".into(), format!("{to}/")),
        ];
    });
    register_then_close(
        &env,
        json!({"t":"push","token":null,"ntfy":"https://hang.example/rift-a","long_ms":1,"show_command":false}),
    );
    register_then_close(
        &env,
        json!({"t":"push","token":null,"ntfy":NTFY,"long_ms":1,"show_command":false}),
    );
    register_then_close(
        &env,
        json!({"t":"push","token":TOKEN,"ntfy":null,"long_ms":1,"show_command":false}),
    );
    let t0 = Instant::now();
    env.fake.edit(12, |b| b.push(finished(3, "true", 0, 5)));
    // the others arrive while the first device's server hangs (curl waits 15 s)
    ntfy.raw(0);
    expo.request(0);
    assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
    // the hanging device's next alert is dropped, not queued behind the first
    env.fake.edit(12, |b| b.push(finished(4, "true", 0, 5)));
    ntfy.raw(1);
    expo.request(1);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        held.lock().unwrap().len(),
        1,
        "one post at a time per device"
    );
}

#[test]
fn a_silent_connection_stops_counting_as_following_its_pane() {
    let ntfy = PushSink::start(|_| "{}".to_string());
    let to = ntfy.url.clone();
    let env = start(move |c| {
        c.ntfy_redirect = to_sink(&to);
        c.follow_fresh = Duration::from_millis(500);
    });
    let (_key, mut ch, _) = env.pair(Permission::View);
    put(
        &mut ch,
        json!({"t":"push","token":null,"ntfy":NTFY,"long_ms":1,"show_command":false}),
    );
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    // longer than follow_fresh after connecting, then just heard from:
    // following, no banner
    std::thread::sleep(Duration::from_millis(700));
    put(&mut ch, json!({"t":"ping","n":1}));
    recv_t(&mut ch, "pong");
    env.fake.edit(12, |b| b.push(finished(3, "make", 1, 5_000)));
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(ntfy.count(), 0);
    // silent for longer than follow_fresh (a dropped network): alerted again
    std::thread::sleep(Duration::from_millis(500));
    env.fake.edit(12, |b| b.push(finished(4, "make", 1, 5_000)));
    let r = ntfy.raw(0);
    assert!(r.header("Click").is_some_and(|c| c.ends_with("/pane/12")));
}

// --- fit ------------------------------------------------------------------

/// Wait for `want` to be what the source was last told (or the end of 3 s).
fn last_fit(env: &Env, want: Option<(u16, u16)>) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(3) {
        if env.fake.fits().last().map(|f| f.1) == Some(want) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn fit_is_welcomed_acked_clamped_and_released() {
    let env = start(|_| {});
    let (_k, mut ch, welcome) = env.pair(Permission::Operate);
    assert!(
        welcome["features"]
            .as_array()
            .unwrap()
            .contains(&json!("fit"))
    );
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(
        &mut ch,
        json!({"t":"fit","id":"f1","pane":12,"cols":64,"rows":40}),
    );
    assert_eq!(
        recv_t(&mut ch, "ack"),
        json!({"t":"ack","id":"f1","ok":true})
    );
    assert_eq!(env.fake.fits(), [(12, Some((64, 40)))]);
    // out of range is clamped; a refresh reaches the source again (lease)
    put(
        &mut ch,
        json!({"t":"fit","id":"f2","pane":12,"cols":1000,"rows":1}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    assert_eq!(env.fake.fits().last(), Some(&(12, Some((300, 5)))));
    put(
        &mut ch,
        json!({"t":"fit","id":"f3","pane":12,"cols":64,"rows":40}),
    );
    recv_t(&mut ch, "ack");
    let n = env.fake.fits().len();
    put(
        &mut ch,
        json!({"t":"fit","id":"f4","pane":12,"cols":64,"rows":40}),
    );
    recv_t(&mut ch, "ack");
    assert_eq!(
        env.fake.fits().len(),
        n + 1,
        "an unchanged fit is refreshed"
    );
    // half a size, a pane it does not follow (the fit rate is per second)
    std::thread::sleep(Duration::from_millis(1050));
    put(
        &mut ch,
        json!({"t":"fit","id":"f5","pane":12,"cols":64,"rows":null}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "bad_request"));
    put(
        &mut ch,
        json!({"t":"fit","id":"f6","pane":99,"cols":64,"rows":40}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "bad_request"));
    // release
    put(
        &mut ch,
        json!({"t":"fit","id":"f7","pane":12,"cols":null,"rows":null}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    assert_eq!(env.fake.fits().last(), Some(&(12, None)));
}

#[test]
fn view_devices_are_forbidden_to_fit() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(
        &mut ch,
        json!({"t":"fit","id":"f1","pane":12,"cols":64,"rows":40}),
    );
    let v = recv_t(&mut ch, "ack");
    assert_eq!(
        (ack(&v), v["id"].as_str()),
        ((false, "forbidden"), Some("f1"))
    );
    assert!(env.fake.fits().is_empty());
}

#[test]
fn a_fit_is_released_when_the_client_unsubscribes_or_disconnects() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(
        &mut ch,
        json!({"t":"fit","id":"f1","pane":12,"cols":64,"rows":40}),
    );
    recv_t(&mut ch, "ack");
    put(&mut ch, json!({"t":"unsubscribe"}));
    assert!(last_fit(&env, None), "{:?}", env.fake.fits());
    // subscribing again does not bring it back
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(env.fake.fits().last(), Some(&(12, None)));
    // a fit, then the connection drops
    put(
        &mut ch,
        json!({"t":"fit","id":"f2","pane":12,"cols":50,"rows":30}),
    );
    recv_t(&mut ch, "ack");
    assert!(last_fit(&env, Some((50, 30))));
    drop(ch);
    assert!(last_fit(&env, None), "{:?}", env.fake.fits());
}

#[test]
fn several_phones_fit_to_the_smallest() {
    let env = start(|_| {});
    let (_k, mut a, _) = env.pair(Permission::Operate);
    let (_k2, mut b, _) = env.pair(Permission::Operate);
    for ch in [&mut a, &mut b] {
        put(ch, json!({"t":"subscribe","pane":12,"after":null}));
        recv_t(ch, "snapshot");
    }
    put(
        &mut a,
        json!({"t":"fit","id":"a","pane":12,"cols":80,"rows":24}),
    );
    recv_t(&mut a, "ack");
    put(
        &mut b,
        json!({"t":"fit","id":"b","pane":12,"cols":60,"rows":40}),
    );
    recv_t(&mut b, "ack");
    assert_eq!(env.fake.fits().last(), Some(&(12, Some((60, 24)))));
    // the smaller one leaves: the other's size holds
    drop(b);
    assert!(last_fit(&env, Some((80, 24))), "{:?}", env.fake.fits());
    put(
        &mut a,
        json!({"t":"fit","id":"a2","pane":12,"cols":null,"rows":null}),
    );
    recv_t(&mut a, "ack");
    assert_eq!(env.fake.fits().last(), Some(&(12, None)));
}

// --- Phase 3: text ------------------------------------------------------------

#[test]
fn text_reaches_the_source_or_is_busy_at_a_prompt() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    put(
        &mut ch,
        json!({"t":"text","id":"x1","pane":12,"text":":wq ünïcode"}),
    );
    assert_eq!(
        recv_t(&mut ch, "ack"),
        json!({"t":"ack","id":"x1","ok":true})
    );
    assert_eq!(
        env.fake.inputs(),
        [(
            12,
            "Test Phone".to_string(),
            Input::Text(":wq ünïcode".into())
        )]
    );
    // the window answers busy for a pane at its prompt: the message says so
    env.fake.fail_input(Some(InputError::Busy));
    put(&mut ch, json!({"t":"text","id":"x2","pane":12,"text":"ls"}));
    let v = recv_t(&mut ch, "ack");
    assert_eq!(ack(&v), (false, "busy"));
    assert!(
        v["message"].as_str().unwrap().contains("at its prompt"),
        "{v}"
    );
    env.fake.fail_input(None);
    put(&mut ch, json!({"t":"text","id":"x3","pane":99,"text":"ls"}));
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "no_pane"));
}

#[test]
fn text_with_control_characters_or_over_a_kib_is_rejected() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    for (text, want) in [
        ("a\nb".to_string(), "bad_request"),
        ("a\rb".into(), "bad_request"),
        ("tab\there".into(), "bad_request"),
        ("\u{1b}[2J".into(), "bad_request"),
        ("del\u{7f}".into(), "bad_request"),
        ("c1\u{85}".into(), "bad_request"),
        ("csi\u{9b}2J".into(), "bad_request"),
        ("nul\0".into(), "bad_request"),
        (String::new(), "bad_request"),
        ("x".repeat(1025), "too_large"),
    ] {
        put(&mut ch, json!({"t":"text","id":"x","pane":12,"text":text}));
        assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, want), "{text:?}");
    }
    assert!(env.fake.inputs().is_empty());
    // 1 KiB exactly, and spaces only, are text
    put(
        &mut ch,
        json!({"t":"text","id":"ok","pane":12,"text":"é".repeat(512)}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    put(&mut ch, json!({"t":"text","id":"sp","pane":12,"text":"  "}));
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    assert_eq!(env.fake.inputs().len(), 2);
}

#[test]
fn text_counts_against_the_keys_rate() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    let mut got = Vec::new();
    for i in 0..21 {
        let msg = if i % 2 == 0 {
            json!({"t":"keys","id":format!("{i}"),"pane":12,"keys":["up"]})
        } else {
            json!({"t":"text","id":format!("{i}"),"pane":12,"text":"j"})
        };
        put(&mut ch, msg);
        got.push(ack(&recv_t(&mut ch, "ack")).1.to_string());
    }
    assert_eq!(got[..20], vec![String::new(); 20]);
    assert_eq!(got[20], "rate");
}

// --- Phase 3: screen ------------------------------------------------------------

/// A 20-column screen of plain rows.
fn screen(rows: &[&str], alt: bool, cursor: Option<[u16; 2]>) -> Screen {
    Screen {
        cols: 20,
        rows: rows.len() as u16,
        alt,
        cursor,
        fg: None,
        bg: None,
        rules: Vec::new(),
        lines: rows
            .iter()
            .map(|r| {
                if r.is_empty() {
                    Vec::new()
                } else {
                    vec![Run::plain(*r)]
                }
            })
            .collect(),
    }
}

impl Fake {
    fn set_screen(&self, pane: u64, s: Screen) {
        self.0.lock().unwrap().screens.insert(pane, s);
    }
    fn edit_screen(&self, pane: u64, f: impl FnOnce(&mut Screen)) {
        f(self.0.lock().unwrap().screens.get_mut(&pane).unwrap());
    }
    fn screen_reads(&self) -> Vec<(u64, bool)> {
        self.0.lock().unwrap().screen_reads.clone()
    }
}

/// Every message within `ms`.
fn drain(ch: &mut Ch, ms: u64) -> Vec<Value> {
    let until = Instant::now() + Duration::from_millis(ms);
    let mut out = Vec::new();
    while Instant::now() < until {
        if let Recv::Msg(m) = ch.recv() {
            out.push(serde_json::from_slice(&m).unwrap());
        }
    }
    out
}

fn screens_in(msgs: &[Value]) -> Vec<&Value> {
    msgs.iter()
        .filter(|v| v["t"] == "screen" || v["t"] == "screen_end")
        .collect()
}

/// The next `screen` or `screen_end` (updates, topology, done skipped).
fn recv_screen(ch: &mut Ch) -> Value {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(Instant::now() < deadline, "no screen within 3 s");
        let v = recv(ch);
        if v["t"] == "screen" || v["t"] == "screen_end" {
            return v;
        }
    }
}

#[test]
fn screens_are_off_until_asked_for() {
    let env = start(|_| {});
    env.fake
        .set_screen(12, screen(&["vim", "~"], true, Some([0, 1])));
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    env.fake.edit_screen(12, |s| s.cursor = Some([1, 0]));
    assert!(screens_in(&drain(&mut ch, 300)).is_empty());
    assert!(env.fake.screen_reads().is_empty(), "read without a mode");
    // only the subscribed pane
    put(&mut ch, json!({"t":"screen","pane":13,"mode":"auto"}));
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "bad_request"));
    put(&mut ch, json!({"t":"unsubscribe"}));
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"on"}));
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "bad_request"));
    assert!(screens_in(&drain(&mut ch, 200)).is_empty());
    // a reconnecting app re-subscribes a pane that closed meanwhile and asks
    // for its screen at once: one `no_pane`, the screen request dropped
    put(&mut ch, json!({"t":"subscribe","pane":99,"after":7}));
    put(&mut ch, json!({"t":"screen","pane":99,"mode":"auto"}));
    put(&mut ch, json!({"t":"ping","n":5}));
    let after: Vec<Value> = std::iter::repeat_with(|| recv(&mut ch))
        .take_while(|v| v["t"] != "pong")
        .filter(|v| v["t"] != "topology")
        .collect();
    assert_eq!(after.len(), 1, "{after:?}");
    assert_eq!(code(&after[0]), ("error", "no_pane"));
    // another pane is still refused
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"auto"}));
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "bad_request"));
}

#[test]
fn auto_sends_a_full_screen_then_only_what_changed_and_ends_off_alt() {
    let env = start(|_| {});
    env.fake
        .set_screen(12, screen(&["top", "  PID", "", ":"], false, Some([3, 1])));
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"auto"}));
    // not on the alternate screen: nothing, and the read is not forced
    assert!(screens_in(&drain(&mut ch, 300)).is_empty());
    assert!(env.fake.screen_reads().iter().all(|&(p, f)| p == 12 && !f));
    env.fake.edit_screen(12, |s| s.alt = true);
    assert_eq!(
        recv_screen(&mut ch),
        json!({"t":"screen","pane":12,"cols":20,"rows":4,"alt":true,"cursor":[3,1],"full":true,
               "lines":[[0,[["top",null,null,0]]],[1,[["  PID",null,null,0]]],[2,[]],[3,[[":",null,null,0]]]]})
    );
    // unchanged: nothing more
    assert!(screens_in(&drain(&mut ch, 300)).is_empty());
    // one row changes (styled): only that row
    env.fake.edit_screen(12, |s| {
        s.lines[1] = vec![
            Run::plain("  PID"),
            Run {
                text: " CPU%".into(),
                fg: Some("1e1e1e".into()),
                bg: Some("e5c07b".into()),
                flags: 1,
            },
        ]
    });
    assert_eq!(
        recv_screen(&mut ch),
        json!({"t":"screen","pane":12,"cols":20,"rows":4,"alt":true,"cursor":[3,1],"full":false,
               "lines":[[1,[["  PID",null,null,0],[" CPU%","1e1e1e","e5c07b",1]]]]})
    );
    // the cursor alone moves: a screen without lines; hidden: null
    env.fake.edit_screen(12, |s| s.cursor = Some([0, 2]));
    let v = recv_screen(&mut ch);
    assert_eq!(
        (&v["full"], &v["cursor"], &v["lines"]),
        (&json!(false), &json!([0, 2]), &json!([]))
    );
    env.fake.edit_screen(12, |s| s.cursor = None);
    assert_eq!(recv_screen(&mut ch)["cursor"], Value::Null);
    // a resize: full again
    env.fake.edit_screen(12, |s| {
        s.rows = 2;
        s.lines.truncate(2);
    });
    let v = recv_screen(&mut ch);
    assert_eq!(
        (&v["full"], v["lines"].as_array().unwrap().len()),
        (&json!(true), 2)
    );
    // leaving the alternate screen ends them
    env.fake.edit_screen(12, |s| s.alt = false);
    assert_eq!(recv_screen(&mut ch), json!({"t":"screen_end","pane":12}));
    assert!(screens_in(&drain(&mut ch, 300)).is_empty());
    // and coming back starts over with a full screen
    env.fake.edit_screen(12, |s| s.alt = true);
    assert_eq!(recv_screen(&mut ch)["full"], true);
}

#[test]
fn on_sends_the_primary_screen_and_off_ends_it() {
    let env = start(|_| {});
    env.fake
        .set_screen(12, screen(&["$ ssh box", "box$ "], false, Some([1, 5])));
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"on"}));
    let v = recv_screen(&mut ch);
    assert_eq!((&v["alt"], &v["full"]), (&json!(false), &json!(true)));
    assert!(env.fake.screen_reads().contains(&(12, true)), "forced");
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"off"}));
    assert_eq!(recv_screen(&mut ch), json!({"t":"screen_end","pane":12}));
    env.fake
        .edit_screen(12, |s| s.lines[1] = vec![Run::plain("box$ ls")]);
    assert!(screens_in(&drain(&mut ch, 300)).is_empty());
    // `auto`/`on` again: a full screen, even when nothing changed since
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"on"}));
    assert_eq!(recv_screen(&mut ch)["full"], true);
    // another connection in `auto` on the same pane: the forced read of the
    // primary screen is not for it
    let (_k2, mut other, _) = env.pair(Permission::View);
    put(&mut other, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut other, "snapshot");
    put(&mut other, json!({"t":"screen","pane":12,"mode":"auto"}));
    env.fake
        .edit_screen(12, |s| s.lines[1] = vec![Run::plain("box$ ls -l")]);
    assert_eq!(recv_screen(&mut ch)["lines"][0][0], 1);
    assert!(screens_in(&drain(&mut other, 300)).is_empty());
}

#[test]
fn a_new_subscription_resets_the_screen_mode_silently() {
    let env = start(|_| {});
    env.fake.set(13, vec![block(1, "x\n")]);
    env.fake.set_screen(12, screen(&["htop"], true, None));
    env.fake.set_screen(13, screen(&["less"], true, None));
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"auto"}));
    assert_eq!(recv_screen(&mut ch)["full"], true);
    // re-subscribing (the same pane or another) stops them without screen_end
    for pane in [12, 13] {
        put(&mut ch, json!({"t":"subscribe","pane":pane,"after":null}));
        let msgs = drain(&mut ch, 400);
        assert!(screens_in(&msgs).is_empty(), "pane {pane}: {msgs:?}");
        assert!(msgs.iter().any(|v| v["t"] == "snapshot"));
    }
    env.fake
        .edit_screen(13, |s| s.lines[0] = vec![Run::plain("less 2")]);
    assert!(screens_in(&drain(&mut ch, 300)).is_empty());
    // the connections are independent: another client's mode is its own
    let (_k2, mut other, _) = env.pair(Permission::View);
    put(&mut other, json!({"t":"subscribe","pane":13,"after":null}));
    recv_t(&mut other, "snapshot");
    put(&mut other, json!({"t":"screen","pane":13,"mode":"auto"}));
    assert_eq!(recv_screen(&mut other)["full"], true);
    assert!(screens_in(&drain(&mut ch, 300)).is_empty());
}

#[test]
fn a_full_screen_over_a_mib_goes_out_without_styles() {
    let env = start(|_| {});
    // 300 × 120 runs, each styled on its own: over 1 MiB of JSON
    let row: Vec<Run> = (0..300)
        .map(|i| Run {
            text: "€€€".into(),
            fg: Some(format!("{:06x}", i)),
            bg: Some("000000".into()),
            flags: 31,
        })
        .collect();
    env.fake.set_screen(
        12,
        Screen {
            cols: 300,
            rows: 120,
            alt: true,
            cursor: Some([0, 0]),
            fg: None,
            bg: None,
            rules: Vec::new(),
            lines: vec![row; 120],
        },
    );
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"auto"}));
    let v = recv_screen(&mut ch);
    assert_eq!(v["full"], true);
    let lines = v["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 120);
    assert_eq!(lines[7], json!([7, [["€€€".repeat(300), null, null, 0]]]));
    assert!(v.to_string().len() <= rift_remote::MAX_GATEWAY_MESSAGE);
    // a screen larger than the protocol's is cut to its top-left 300 × 120
    env.fake.set_screen(
        12,
        Screen {
            cols: 400,
            rows: 200,
            alt: true,
            cursor: Some([150, 2]),
            fg: None,
            bg: None,
            rules: Vec::new(),
            lines: vec![vec![Run::plain("x")]; 200],
        },
    );
    let v = recv_screen(&mut ch);
    assert_eq!(
        (&v["cols"], &v["rows"], &v["cursor"]),
        (&json!(300), &json!(120), &Value::Null)
    );
    assert_eq!(v["lines"].as_array().unwrap().len(), 120);
}

#[test]
fn after_an_unstyled_screen_diffs_restyle_only_the_rows_that_differ() {
    let env = start(|_| {});
    let styled = |c: &str| Run {
        text: "€€".into(),
        fg: Some(c.into()),
        bg: Some("000000".into()),
        flags: 1,
    };
    // over 1 MiB styled: it goes out unstyled
    let row: Vec<Run> = (0..300).map(|i| styled(&format!("{i:06x}"))).collect();
    env.fake.set_screen(
        12,
        Screen {
            cols: 300,
            rows: 120,
            alt: true,
            cursor: None,
            fg: None,
            bg: None,
            rules: Vec::new(),
            lines: vec![row; 120],
        },
    );
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"auto"}));
    let v = recv_screen(&mut ch);
    assert_eq!(v["full"], true);
    assert_eq!(
        v["lines"][3],
        json!([3, [["€€".repeat(300), null, null, 0]]])
    );
    assert!(screens_in(&drain(&mut ch, 300)).is_empty());
    // the program redraws: one row styled, the others as the client has
    // them (unstyled): only that row goes out, with its style
    env.fake.edit_screen(12, |s| {
        for l in &mut s.lines {
            *l = vec![Run::plain("€€".repeat(300))];
        }
        s.lines[5] = vec![styled("e5c07b")];
    });
    let v = recv_screen(&mut ch);
    assert_eq!(v["full"], false);
    assert_eq!(v["lines"], json!([[5, [["€€", "e5c07b", "000000", 1]]]]));
}

#[test]
fn screens_carry_the_ground_and_the_block_rules_whole_in_every_frame() {
    let env = start(|_| {});
    let mut s = screen(&["~ ok", "hi", "~ bad"], true, None);
    s.fg = Some("d4d4d4".into());
    s.bg = Some("1e1e1e".into());
    s.rules = vec![
        (0, "3fb950".into()),
        (1, "3fb950".into()),
        (2, "f85149".into()),
    ];
    env.fake.set_screen(12, s);
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"auto"}));
    let v = recv_screen(&mut ch);
    assert_eq!(v["full"], true);
    assert_eq!((&v["fg"], &v["bg"]), (&json!("d4d4d4"), &json!("1e1e1e")));
    let rules = json!([[0, "3fb950"], [1, "3fb950"], [2, "f85149"]]);
    assert_eq!(v["rules"], rules);
    // a diff frame (one row changed) still lists every rule
    env.fake
        .edit_screen(12, |s| s.lines[1] = vec![Run::plain("yo")]);
    let v = recv_screen(&mut ch);
    assert_eq!(v["full"], false);
    assert_eq!(v["lines"], json!([[1, [["yo", null, null, 0]]]]));
    assert_eq!(v["rules"], rules);
    // only a rule changed (the command finished): that alone is a frame
    env.fake.edit_screen(12, |s| s.rules[2].1 = "d29922".into());
    let v = recv_screen(&mut ch);
    assert_eq!(v["lines"], json!([]));
    assert_eq!(
        v["rules"],
        json!([[0, "3fb950"], [1, "3fb950"], [2, "d29922"]])
    );
    // none of it for a source that sends none: the keys are absent
    env.fake.set_screen(12, screen(&["x"], true, None));
    let v = recv_screen(&mut ch);
    assert!(
        v.get("fg").is_none() && v.get("bg").is_none() && v.get("rules").is_none(),
        "{v}"
    );
}

#[test]
fn on_to_auto_on_the_primary_screen_ends_it() {
    let env = start(|_| {});
    env.fake.set_screen(12, screen(&["$ ls"], false, None));
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"on"}));
    assert_eq!(recv_screen(&mut ch)["alt"], false);
    // `auto` has no screen off the alternate one: the shown one ends
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"auto"}));
    assert_eq!(recv_screen(&mut ch), json!({"t":"screen_end","pane":12}));
    assert!(screens_in(&drain(&mut ch, 300)).is_empty());
}

#[test]
fn a_slow_client_gets_the_newest_screen_not_a_queue_of_them() {
    let env = start(|_| {});
    // about 0.8 MiB of styled runs, every one recoloured by each change
    let screen_n = |n: usize| Screen {
        cols: 300,
        rows: 120,
        alt: true,
        cursor: Some([0, 0]),
        fg: None,
        bg: None,
        rules: Vec::new(),
        lines: (0..120)
            .map(|r| {
                (0..300)
                    .map(|c| Run {
                        text: "a".into(),
                        fg: Some(format!("{:06x}", (n * 7919 + r * 300 + c) % 0xff_ffff)),
                        bg: None,
                        flags: 0,
                    })
                    .collect()
            })
            .collect(),
    };
    env.fake.set_screen(12, screen_n(0));
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"auto"}));
    let first = recv_screen(&mut ch);
    assert_eq!(first["full"], true);
    let mut rows: Vec<Value> = first["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l[1].clone())
        .collect();
    // 60 changes over 3 s while the client reads nothing
    const CHANGES: usize = 60;
    let fake = env.fake.clone();
    let writer = std::thread::spawn(move || {
        for n in 1..=CHANGES {
            fake.set_screen(12, screen_n(n));
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    std::thread::sleep(Duration::from_secs(3));
    writer.join().unwrap();
    // then it catches up: a few frames, each a diff against what it has,
    // ending at the newest screen
    let want: Vec<Value> = screen_n(CHANGES)
        .lines
        .iter()
        .map(|l| serde_json::to_value(l).unwrap())
        .collect();
    let mut frames = 0;
    while rows != want {
        let v = recv_screen(&mut ch);
        assert_eq!(v["t"], "screen", "{v}");
        frames += 1;
        for l in v["lines"].as_array().unwrap() {
            rows[l[0].as_u64().unwrap() as usize] = l[1].clone();
        }
    }
    assert!(
        frames < CHANGES / 2,
        "{frames} frames for {CHANGES} changes"
    );
    // still connected
    put(&mut ch, json!({"t":"ping","n":1}));
    assert_eq!(recv_t(&mut ch, "pong")["n"], 1);
}

// --- Phase 3: output paging -------------------------------------------------------

impl Fake {
    fn set_output(&self, pane: u64, block: u64, gone: u64, lines: Vec<String>) {
        self.0
            .lock()
            .unwrap()
            .outputs
            .insert((pane, block), (gone, lines));
    }
}

#[test]
fn output_pages_by_lines_and_reports_gone_ones() {
    let env = start(|_| {});
    env.fake
        .set_output(12, 2, 0, (0..40).map(|i| format!("line {i}\n")).collect());
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(
        &mut ch,
        json!({"t":"output","id":"o1","pane":12,"block":2,"from":0,"limit":3}),
    );
    assert_eq!(
        recv_t(&mut ch, "output"),
        json!({"t":"output","id":"o1","pane":12,"block":2,"from":0,
               "text":"line 0\nline 1\nline 2\n","total":40})
    );
    put(
        &mut ch,
        json!({"t":"output","id":"o2","pane":12,"block":2,"from":38,"limit":500}),
    );
    let v = recv_t(&mut ch, "output");
    assert_eq!(
        (&v["from"], &v["text"]),
        (&json!(38), &json!("line 38\nline 39\n"))
    );
    // at or past the end: empty
    put(
        &mut ch,
        json!({"t":"output","id":"o3","pane":12,"block":2,"from":40,"limit":10}),
    );
    let v = recv_t(&mut ch, "output");
    assert_eq!((&v["text"], &v["total"]), (&json!(""), &json!(40)));
    // lines gone from scrollback: the answer starts at the first held one
    env.fake
        .set_output(12, 2, 25, (25..40).map(|i| format!("line {i}\n")).collect());
    put(
        &mut ch,
        json!({"t":"output","id":"o4","pane":12,"block":2,"from":0,"limit":2}),
    );
    let v = recv_t(&mut ch, "output");
    assert_eq!(
        (&v["from"], &v["text"], &v["total"]),
        (&json!(25), &json!("line 25\nline 26\n"), &json!(40))
    );
    // unknown block, unknown pane, a limit out of 1 … 500
    for (msg, want) in [
        (
            json!({"t":"output","id":"b","pane":12,"block":99,"from":0,"limit":5}),
            "no_block",
        ),
        (
            json!({"t":"output","id":"p","pane":77,"block":2,"from":0,"limit":5}),
            "no_pane",
        ),
        (
            json!({"t":"output","id":"z","pane":12,"block":2,"from":0,"limit":0}),
            "bad_request",
        ),
        (
            json!({"t":"output","id":"l","pane":12,"block":2,"from":0,"limit":501}),
            "bad_request",
        ),
    ] {
        put(&mut ch, msg.clone());
        let v = recv_t(&mut ch, "ack");
        assert_eq!((ack(&v), &v["id"]), ((false, want), &msg["id"]), "{msg}");
    }
}

#[test]
fn output_pages_are_bounded_to_64_kib_at_a_line_end() {
    let env = start(|_| {});
    let line = format!("{}\n", "x".repeat(999));
    env.fake.set_output(12, 2, 0, vec![line.clone(); 200]);
    env.fake
        .set_output(12, 3, 0, vec!["é".repeat(40_000), "tail\n".into()]);
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(
        &mut ch,
        json!({"t":"output","id":"o","pane":12,"block":2,"from":0,"limit":500}),
    );
    let v = recv_t(&mut ch, "output");
    let text = v["text"].as_str().unwrap();
    assert_eq!(text.len(), 65 * 1000);
    assert!(text.ends_with('\n'));
    assert_eq!(v["total"], 200);
    // one line longer than 64 KiB: its first 64 KiB
    put(
        &mut ch,
        json!({"t":"output","id":"o","pane":12,"block":3,"from":0,"limit":500}),
    );
    let text = recv_t(&mut ch, "output")["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        text.len() <= rift_remote::MAX_OUTPUT_BYTES
            && text.len() > rift_remote::MAX_OUTPUT_BYTES - 2
    );
    assert!(text.chars().all(|c| c == 'é'));
}

#[test]
fn chip_menus_and_diffs_are_answered_within_bounds() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"dirs","id":"m1","pane":12}));
    let v = recv_t(&mut ch, "dirs");
    let dirs = v["dirs"].as_array().unwrap();
    assert_eq!(dirs.len(), rift_remote::MAX_MENU_ITEMS);
    assert_eq!(
        (&dirs[0], &dirs[1]),
        (&json!("d000"), &json!("d001")),
        "the over-long name is left out"
    );
    assert_eq!(
        (&v["id"], &v["cwd"], &v["parent"], &v["more"]),
        (&json!("m1"), &json!("/w/rift"), &json!(true), &json!(true))
    );
    put(&mut ch, json!({"t":"branches","id":"m2","pane":12}));
    assert_eq!(
        recv_t(&mut ch, "branches"),
        json!({"t":"branches","id":"m2","pane":12,"current":"main","branches":["main","feat"],"more":false})
    );
    put(&mut ch, json!({"t":"changes","id":"m3","pane":12}));
    assert_eq!(
        recv_t(&mut ch, "changes"),
        json!({"t":"changes","id":"m3","pane":12,"branch":"main","more":false,
               "files":[{"path":"a.rs","status":"M","added":3,"removed":1,"staged":false}]})
    );
    put(
        &mut ch,
        json!({"t":"diff","id":"m4","pane":12,"path":"a.rs","staged":false}),
    );
    let v = recv_t(&mut ch, "diff");
    let lines = v["lines"].as_array().unwrap();
    assert_eq!(lines.len(), rift_remote::MAX_DIFF_LINES);
    assert_eq!(
        lines[1][1].as_str().unwrap().len(),
        rift_remote::MAX_DIFF_LINE_CHARS,
        "a long line is cut"
    );
    assert_eq!(
        (&v["truncated"], &v["binary"]),
        (&json!(true), &json!(false))
    );
    // not a change, an unknown pane, a path too long
    for (msg, want) in [
        (
            json!({"t":"diff","id":"a","pane":12,"path":"b.rs","staged":false}),
            "no_block",
        ),
        (
            json!({"t":"diff","id":"b","pane":12,"path":"a.rs","staged":true}),
            "no_block",
        ),
        (json!({"t":"dirs","id":"c","pane":77}), "no_pane"),
        (
            json!({"t":"diff","id":"d","pane":77,"path":"a.rs","staged":false}),
            "no_pane",
        ),
        (
            json!({"t":"diff","id":"e","pane":12,"path":"p".repeat(1025),"staged":false}),
            "bad_request",
        ),
        (
            json!({"t":"diff","id":"f","pane":12,"path":"","staged":false}),
            "bad_request",
        ),
    ] {
        put(&mut ch, msg.clone());
        let v = recv_t(&mut ch, "ack");
        assert_eq!((ack(&v), &v["id"]), ((false, want), &msg["id"]), "{msg}");
    }
}

#[test]
fn an_untracked_files_content_needs_operate() {
    let env = start(|_| {});
    let untracked = json!({"t":"diff","id":"u","pane":12,"path":".env","staged":false});
    let (_k, mut view, _) = env.pair(Permission::View);
    put(&mut view, untracked.clone());
    let v = recv_t(&mut view, "ack");
    assert_eq!((ack(&v), &v["id"]), ((false, "forbidden"), &json!("u")));
    // a tracked file's diff is still the view device's
    put(
        &mut view,
        json!({"t":"diff","id":"t","pane":12,"path":"a.rs","staged":false}),
    );
    assert_eq!(recv_t(&mut view, "diff")["id"], "t");
    let (_k, mut operate, _) = env.pair(Permission::Operate);
    put(&mut operate, untracked);
    let v = recv_t(&mut operate, "diff");
    assert_eq!(v["lines"][1], json!(["+", "TOKEN=secret"]));
}

#[test]
fn an_answer_over_the_message_limit_is_a_too_large_ack() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(
        &mut ch,
        json!({"t":"diff","id":"big","pane":12,"path":"ctl.txt","staged":false}),
    );
    let v = recv_t(&mut ch, "ack");
    assert_eq!((ack(&v), &v["id"]), ((false, "too_large"), &json!("big")));
    // the connection goes on
    put(&mut ch, json!({"t":"ping","n":5}));
    assert_eq!(recv_t(&mut ch, "pong")["n"], 5);
}

#[test]
fn view_devices_get_no_completions() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(
        &mut ch,
        json!({"t":"complete","id":"k","pane":12,"line":"","cursor":0}),
    );
    let v = recv_t(&mut ch, "ack");
    assert_eq!((ack(&v), &v["id"]), ((false, "forbidden"), &json!("k")));
    assert!(env.fake.0.lock().unwrap().completes.is_empty());
}

#[test]
fn completions_are_answered_within_bounds() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    put(
        &mut ch,
        json!({"t":"complete","id":"k1","pane":12,"line":"cargo t","cursor":7}),
    );
    let v = recv_t(&mut ch, "complete");
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), rift_remote::MAX_COMPLETE_ITEMS);
    assert_eq!(
        (&items[0], &items[1]),
        (
            &json!({"text":"cargo t0","kind":"subcommand"}),
            &json!({"text":"cargo t1","kind":"subcommand"})
        ),
        "the over-long completion is left out"
    );
    assert_eq!(
        (&v["id"], &v["more"], &v["history"]),
        (
            &json!("k1"),
            &json!(true),
            &json!([{"text":"cargo t --all","at":1_760_000_000}])
        )
    );
    // a cursor inside `é`, past the end, a line too long, an unknown pane
    for (msg, want) in [
        (
            json!({"t":"complete","id":"a","pane":12,"line":"é","cursor":1}),
            "bad_request",
        ),
        (
            json!({"t":"complete","id":"b","pane":12,"line":"ls","cursor":3}),
            "bad_request",
        ),
        (
            json!({"t":"complete","id":"c","pane":12,"line":"x".repeat(1025),"cursor":0}),
            "bad_request",
        ),
        (
            json!({"t":"complete","id":"d","pane":77,"line":"ls","cursor":2}),
            "no_pane",
        ),
    ] {
        put(&mut ch, msg.clone());
        let v = recv_t(&mut ch, "ack");
        assert_eq!((ack(&v), &v["id"]), ((false, want), &msg["id"]), "{msg}");
    }
    assert_eq!(
        env.fake.0.lock().unwrap().completes,
        [(12, "cargo t".to_string(), 7)],
        "refused requests never reach the desktop"
    );
}

#[test]
fn a_panes_waiting_reaches_the_phone_bounded() {
    let env = start(|_| {});
    env.fake.set(13, vec![block(1, "")]);
    let (_k, mut ch, _) = env.pair(Permission::View);
    // the next topology poll sends the change
    {
        let mut st = env.fake.0.lock().unwrap();
        st.waiting.insert(
            12,
            Waiting {
                kind: WaitingKind::Choice,
                prompt: "q".repeat(400) + "?",
                choices: (1..=12).map(|i| format!("c{i}")).collect(),
                selected: 11,
                numbered: true,
                context: (0..20).map(|i| format!("l{i}")).collect(),
            },
        );
        // a password never carries what the screen showed around it
        st.waiting.insert(
            13,
            Waiting {
                kind: WaitingKind::Password,
                prompt: "Password:".into(),
                choices: vec!["secret".into()],
                selected: 1,
                numbered: true,
                context: vec!["secret".into()],
            },
        );
    }
    let v = recv_t(&mut ch, "topology");
    let panes = &v["windows"][0]["tabs"][0]["panes"];
    let pane = |id: u64| {
        panes
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .unwrap()["waiting"]
            .clone()
    };
    let w = pane(12);
    assert_eq!(w["prompt"].as_str().unwrap().chars().count(), 300);
    assert_eq!(w["choices"].as_array().unwrap().len(), 9);
    assert_eq!(w["selected"], 8);
    let context = w["context"].as_array().unwrap();
    assert_eq!((context.len(), &context[11]), (12, &json!("l19")));
    assert_eq!(
        pane(13),
        json!({"kind":"password","prompt":"Password:","choices":[],"selected":0,"numbered":false,"context":[]})
    );
}

#[test]
fn output_is_rate_limited_per_connection() {
    let env = start(|_| {});
    env.fake.set_output(12, 2, 0, vec!["a\n".into()]);
    let (_k, mut ch, _) = env.pair(Permission::View);
    let t = Instant::now();
    let mut got = Vec::new();
    for i in 0..11 {
        put(
            &mut ch,
            json!({"t":"output","id":format!("o{i}"),"pane":12,"block":2,"from":0,"limit":1}),
        );
        let v = loop {
            let v = recv(&mut ch);
            if v["t"] != "topology" {
                break v;
            }
        };
        got.push(v["t"].as_str().unwrap().to_string() + v["error"].as_str().unwrap_or(""));
    }
    assert!(
        t.elapsed() < Duration::from_millis(900),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(got[..10], vec!["output".to_string(); 10]);
    assert_eq!(got[10], "ackrate");
}
