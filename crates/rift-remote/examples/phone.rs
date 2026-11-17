//! A scripted phone for checking a live gateway by hand: pairs with a
//! `rift-remote://pair?…` link (or reconnects with a saved key), prints every
//! message as a transcript, follows the first pane, sends one `run`, and
//! waits for `done` events.
//!
//! ```sh
//! cargo run -p rift-remote --example phone -- [--relay-only] KEYFILE LINK COMMAND [SECONDS] [TOUCH_FILE]
//! ```
//!
//! KEYFILE holds the device's private key (created on first use). The
//! phone tries the link's `a` addresses (1.5 s each), then its relay `r`
//! (as the app does); `--relay-only` skips the addresses. After the `run`
//! ack, TOUCH_FILE (if given) is created — a scripted Rift window can
//! `wait_file` on it to type a command itself — and the phone reads for
//! SECONDS (default 10). Exits 3 when the `run` was refused (`forbidden`,
//! `busy`, …), 1 when the connection could not be made or ended.
//!
//! The environment variable `PHONE_SEND` holds more messages, one JSON
//! object per line, sent (each waiting for its `ack`) after the `run` and
//! before TOUCH_FILE; `LAST` in a line stands for the last pane of the
//! first window's topology (`{"t":"close","id":"c1","pane":LAST,"kind":"pane"}`)
//! and `FIRST` for its first pane (the one the phone follows and runs in).
//! A refused one exits 3 as well.

use rift_remote::protocol::ClientHello;
use rift_remote::registry;
use rift_remote::relay::{self, RelayUrl};
use rift_remote::transport::{self, Recv, Socket};
use serde_json::{Value, json};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::stream::MaybeTlsStream;

type Stream = MaybeTlsStream<TcpStream>;

/// `%XX` escapes decoded (links percent-encode `n` and `r`).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let relay_only = args.first().is_some_and(|a| a == "--relay-only");
    if relay_only {
        args.remove(0);
    }
    let [keyfile, link, command, rest @ ..] = args.as_slice() else {
        eprintln!("usage: phone [--relay-only] KEYFILE LINK COMMAND [SECONDS] [TOUCH_FILE]");
        std::process::exit(2);
    };
    let seconds: u64 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(10);
    let touch = rest.get(1);
    let query = link
        .strip_prefix("rift-remote://pair?")
        .expect("a rift-remote://pair link");
    let param = |k: &str| {
        query
            .split('&')
            .find_map(|kv| kv.strip_prefix(&format!("{k}=")))
            .map(percent_decode)
            .unwrap_or_default()
    };
    let pinned = registry::decode_key(&param("k")).expect("k is a 32-byte key");
    let (key, pair) = match std::fs::read(keyfile) {
        Ok(k) => (k.try_into().expect("a 32-byte key file"), None),
        Err(_) => {
            let k = transport::generate_private_key().unwrap();
            std::fs::write(keyfile, k).unwrap();
            (k, Some(param("s")))
        }
    };
    let t0 = Instant::now();
    let log = |dir: &str, v: &Value| println!("{:>7.3}s {dir} {v}", t0.elapsed().as_secs_f64());

    let ws = dial(&param("a"), &param("r"), &pinned, relay_only, t0);
    let hello = ClientHello {
        v: 2,
        name: "Live Check Phone".into(),
        platform: "test".into(),
        pair,
    };
    log("hs→", &serde_json::to_value(&hello).unwrap());
    let (mut ch, gateway) =
        transport::connect(ws, &key, &pinned, &hello, 4 << 20).expect("handshake");
    log("hs←", &serde_json::to_value(&gateway).unwrap());

    let recv = |ch: &mut transport::Channel<Stream>, until: Instant| -> Option<Value> {
        while Instant::now() < until {
            match ch.recv() {
                Recv::Msg(m) => {
                    let v: Value = serde_json::from_slice(&m).unwrap();
                    log("←", &v);
                    return Some(v);
                }
                Recv::Idle | Recv::Alive | Recv::Partial => {}
                other => {
                    println!("connection ended: {other:?}");
                    std::process::exit(1);
                }
            }
        }
        None
    };
    let soon = || Instant::now() + Duration::from_secs(5);
    let welcome = recv(&mut ch, soon()).expect("welcome");
    assert_eq!(welcome["t"], "welcome", "{welcome}");
    let topo = recv(&mut ch, soon()).expect("topology");
    let pane = topo["windows"][0]["tabs"][0]["panes"][0]["id"]
        .as_u64()
        .expect("a pane in the topology");
    let send = |ch: &mut transport::Channel<Stream>, v: Value| {
        log("→", &v);
        assert!(ch.send(&v.to_string()));
    };
    send(&mut ch, json!({"t":"subscribe","pane":pane,"after":null}));
    send(
        &mut ch,
        json!({"t":"run","id":"live1","pane":pane,"command":command}),
    );
    let until = soon();
    let mut refused = false;
    while let Some(v) = recv(&mut ch, until) {
        if v["t"] == "ack" {
            refused = v["ok"] != true;
            break;
        }
        if v["t"] == "error" {
            refused = true;
            break;
        }
    }
    if let Ok(extra) = std::env::var("PHONE_SEND") {
        let last = topo["windows"][0]["tabs"]
            .as_array()
            .and_then(|t| t.last())
            .and_then(|t| t["panes"].as_array()?.last()?["id"].as_u64())
            .expect("a last pane in the topology");
        for line in extra.lines().filter(|l| !l.trim().is_empty()) {
            let line = line
                .replace("LAST", &last.to_string())
                .replace("FIRST", &pane.to_string());
            let msg: Value = serde_json::from_str(&line).expect("JSON");
            send(&mut ch, msg);
            let until = soon();
            while let Some(v) = recv(&mut ch, until) {
                if v["t"] == "ack" || v["t"] == "error" {
                    refused |= v["ok"] != true;
                    break;
                }
            }
        }
    }
    // stitched pages: `up`, `down` (a page_ahead of 120 rows), `seek:AT` or `fit:COLSxROWS` (the
    // pane at the phone's size), comma-separated, each answered before the next; the last page's
    // epoch carries on
    if let Ok(spec) = std::env::var("PHONE_PAGES") {
        let mut epoch = 0u64;
        for (i, step) in spec.split(',').enumerate() {
            if let Some(size) = step.strip_prefix("fit:") {
                let (c, r) = size.split_once('x').expect("fit:COLSxROWS");
                send(
                    &mut ch,
                    json!({"t":"fit","id":format!("pg{i}"),"pane":pane,"cols":c.parse::<u16>().unwrap(),"rows":r.parse::<u16>().unwrap()}),
                );
                // the program redraws at the new size
                let until = Instant::now() + Duration::from_secs(3);
                while recv(&mut ch, until).is_some() {}
                continue;
            }
            let msg = match step.strip_prefix("seek:") {
                Some(at) => {
                    let at: i64 = at.parse().expect("seek:AT");
                    json!({"t":"page_seek","id":format!("pg{i}"),"pane":pane,"epoch":epoch,"at":at})
                }
                None => {
                    json!({"t":"page_ahead","id":format!("pg{i}"),"pane":pane,"epoch":epoch,"dir":step,"rows":120})
                }
            };
            send(&mut ch, msg);
            let mut until = Instant::now() + Duration::from_secs(10);
            while let Some(v) = recv(&mut ch, until) {
                if v["t"] == "ack" && v["ok"] != true {
                    break;
                }
                if v["t"] == "page" {
                    epoch = v["epoch"].as_u64().unwrap_or(0);
                    // the job's other chunks follow at once
                    until = Instant::now() + Duration::from_millis(400);
                }
            }
        }
    }
    if refused {
        println!("run refused");
    }
    if let Some(f) = touch {
        std::fs::write(f, b"go").unwrap();
        println!("touched {f}");
    }
    let until = Instant::now() + Duration::from_secs(seconds);
    while recv(&mut ch, until).is_some() {}
    ch.close();
    if refused {
        std::process::exit(3);
    }
}

/// The first of the link's addresses that answers within 1.5 s, else its
/// relay (the app's order).
fn dial(
    addresses: &str,
    relay: &str,
    pinned: &[u8; 32],
    relay_only: bool,
    t0: Instant,
) -> tungstenite::WebSocket<Stream> {
    let say = |what: String| println!("{:>7.3}s -- {what}", t0.elapsed().as_secs_f64());
    if !relay_only {
        for a in addresses.split(',').filter(|a| !a.is_empty()) {
            let Ok(addr) = a.parse::<SocketAddr>() else {
                continue;
            };
            let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(1500)) else {
                say(format!("{a}: no answer"));
                continue;
            };
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            let Ok((ws, _)) =
                tungstenite::client(format!("ws://{a}/"), MaybeTlsStream::Plain(stream))
            else {
                continue;
            };
            say(format!("via lan {a}"));
            return ws;
        }
    }
    let url = RelayUrl::parse(relay).unwrap_or_else(|e| {
        println!("no address answered and no usable relay: {e}");
        std::process::exit(1)
    });
    let id = registry::routing_id(pinned);
    match relay::dial(
        &url,
        &format!("/connect?id={id}"),
        None,
        WebSocketConfig::default(),
        Duration::from_secs(10),
    ) {
        Ok(ws) => {
            ws.get_ref()
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            say(format!("via relay {}", url.endpoint("")));
            ws
        }
        Err(e) => {
            println!("relay {relay}: {e}");
            std::process::exit(1)
        }
    }
}
