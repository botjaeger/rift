//! Wire types of the remote protocol, version 2 (`docs/remote-protocol.md`).
//!
//! Every message object carries `"t"`. Nullable fields are always present
//! (`null`), as in the fixtures under `crates/rift-remote/fixtures/`; fields
//! a message only sometimes has (`ack.error`, the client hello's `pair`) are
//! left out when absent.

use serde::{Deserialize, Serialize};

/// Protocol version carried by both handshake payloads and `welcome`.
pub const VERSION: u32 = 2;

/// Handshake message 2 payload (gateway → client).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayHello {
    pub v: u32,
    pub host: String,
    pub os: String,
    pub version: String,
}

/// Handshake message 3 payload (client → gateway).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientHello {
    pub v: u32,
    pub name: String,
    pub platform: String,
    /// A pairing secret (base64url) on the first connection of a device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair: Option<String>,
}

/// What a paired device may do: `view` gets `forbidden` for `run` / `keys`
/// / `text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Operate,
    View,
}

impl Permission {
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::Operate => "operate",
            Permission::View => "view",
        }
    }

    pub fn parse(s: &str) -> Option<Permission> {
        match s {
            "operate" => Some(Permission::Operate),
            "view" => Some(Permission::View),
            _ => None,
        }
    }
}

/// Client → gateway. An unknown `"t"` fails to decode (the gateway answers
/// `error` `bad_request`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Follow one pane; `after` is the last `seq` the client applied.
    Subscribe {
        pane: u64,
        #[serde(default)]
        after: Option<u64>,
    },
    /// Up to 30 blocks older than block id `before`.
    Older {
        pane: u64,
        before: u64,
    },
    Unsubscribe,
    /// Submit a command line to a pane at its prompt.
    Run {
        id: String,
        pane: u64,
        command: String,
    },
    /// Send keys (names as in [`crate::Key`]) to a pane.
    Keys {
        id: String,
        pane: u64,
        keys: Vec<String>,
    },
    /// Remove this device from the paired devices and close.
    Forget,
    Ping {
        n: u64,
    },
    /// Whether the app is on screen (`push_ntfy` gateways; a new connection
    /// starts visible). While it is not, the pane it follows gets ntfy
    /// alerts again: a backgrounded app keeps its socket for a while but
    /// cannot notify. No reply.
    Visible {
        on: bool,
    },
    /// Where to alert while the app is closed: an Expo push token, an ntfy
    /// topic URL, both, or neither (`null` both: stop). `long_ms`: alert
    /// for every command at least this long (`null`: only for `notify`).
    /// No reply.
    Push {
        token: Option<String>,
        /// `https://<host>[:port]/<topic>` (`push_ntfy` feature; older
        /// clients leave it out).
        #[serde(default)]
        ntfy: Option<String>,
        long_ms: Option<u64>,
        #[serde(default)]
        show_command: bool,
        /// The `long_ms` alerts only for commands that failed.
        #[serde(default)]
        failures_only: bool,
    },
    /// Alert when the next command in `pane` finishes, whatever its length
    /// ("Notify me"). Needs a `push` registration. No reply.
    Notify {
        pane: u64,
    },
    /// Type printable text (one line, no control characters) into the
    /// program running in `pane`.
    Text {
        id: String,
        pane: u64,
        text: String,
    },
    /// What to send of the subscribed pane's terminal screen.
    Screen {
        pane: u64,
        mode: ScreenMode,
    },
    /// Lines `from` … `from + limit - 1` of a block's output.
    Output {
        id: String,
        pane: u64,
        block: u64,
        from: u64,
        limit: usize,
    },
}

/// `screen` modes: never, while the pane shows the alternate screen, or
/// always.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenMode {
    #[default]
    Off,
    Auto,
    On,
}

/// A run of equally styled cells of a `screen` row; on the wire the array
/// `[text, fg, bg, flags]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RunArray", into = "RunArray")]
pub struct Run {
    pub text: String,
    /// `"rrggbb"`, or `None` for the default foreground / background.
    pub fg: Option<String>,
    pub bg: Option<String>,
    /// [`Run::BOLD`] | [`Run::ITALIC`] | …
    pub flags: u8,
}

type RunArray = (String, Option<String>, Option<String>, u8);

impl Run {
    pub const BOLD: u8 = 1;
    pub const ITALIC: u8 = 2;
    pub const UNDERLINE: u8 = 4;
    pub const DIM: u8 = 8;
    pub const STRIKE: u8 = 16;

    /// Text in the default style.
    pub fn plain(text: impl Into<String>) -> Run {
        Run {
            text: text.into(),
            fg: None,
            bg: None,
            flags: 0,
        }
    }
}

impl From<RunArray> for Run {
    fn from((text, fg, bg, flags): RunArray) -> Run {
        Run {
            text,
            fg,
            bg,
            flags,
        }
    }
}

impl From<Run> for RunArray {
    fn from(r: Run) -> RunArray {
        (r.text, r.fg, r.bg, r.flags)
    }
}

/// Gateway → client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome {
        v: u32,
        host: String,
        os: String,
        version: String,
        /// This device's id in the desktop's paired devices.
        device: String,
        permission: Permission,
        /// What the gateway serves beyond Phase 2 (`screen`, `text`,
        /// `output`); none when absent.
        #[serde(default)]
        features: Vec<String>,
    },
    Topology {
        windows: Vec<Window>,
    },
    Snapshot {
        pane: u64,
        seq: u64,
        blocks: Vec<Block>,
        has_older: bool,
    },
    Update {
        pane: u64,
        seq: u64,
        blocks: Vec<Block>,
        removed: Vec<u64>,
    },
    Older {
        pane: u64,
        blocks: Vec<Block>,
        has_older: bool,
    },
    /// The answer to `run` / `keys` / `text` (and a refused `output`) with
    /// the client's `id`.
    Ack {
        id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ErrorCode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// A pane's command finished (sent to every connection).
    Done {
        pane: u64,
        block: u64,
        command: String,
        exit: Option<i32>,
        duration_ms: u64,
    },
    Pong {
        n: u64,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
    /// The subscribed pane's screen: every row when `full`, else the rows
    /// that changed since the previous one (`[row, runs]`).
    Screen {
        pane: u64,
        cols: u16,
        rows: u16,
        alt: bool,
        /// `[row, column]` while the program shows its cursor.
        cursor: Option<[u16; 2]>,
        full: bool,
        lines: Vec<(u16, Vec<Run>)>,
    },
    /// Screens of `pane` stopped (left the alternate screen in `auto`, or
    /// `off`).
    ScreenEnd {
        pane: u64,
    },
    /// The answer to `output`: lines from `from` on.
    Output {
        id: String,
        pane: u64,
        block: u64,
        from: u64,
        text: String,
        /// The block's output lines so far.
        total: u64,
    },
}

impl ServerMsg {
    pub fn error(code: ErrorCode, message: impl Into<String>) -> ServerMsg {
        ServerMsg::Error {
            code,
            message: message.into(),
        }
    }

    pub fn ack_ok(id: impl Into<String>) -> ServerMsg {
        ServerMsg::Ack {
            id: id.into(),
            ok: true,
            error: None,
            message: None,
        }
    }

    pub fn ack_err(
        id: impl Into<String>,
        code: ErrorCode,
        message: impl Into<String>,
    ) -> ServerMsg {
        ServerMsg::Ack {
            id: id.into(),
            ok: false,
            error: Some(code),
            message: Some(message.into()),
        }
    }
}

/// A relay's control message (text frames of at most 1 KiB on the
/// gateway's `/gateway` connection; `docs/remote-protocol.md` § Relay).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum RelayMsg {
    /// Gateway → relay: claim `id` (routing id) for the static public key
    /// `key`, with `secret` for its `/accept` connections (32 bytes each,
    /// base64url).
    Register {
        id: String,
        key: String,
        secret: String,
    },
    /// Relay → gateway: prove you hold `key`'s private half: `m` is message
    /// 1 of a `Noise_NK_25519_ChaChaPoly_BLAKE2s` handshake (prologue
    /// `rift-relay/1`) to it, base64url.
    Prove { m: String },
    /// Gateway → relay: message 2 of that handshake, base64url.
    Proof { m: String },
    /// Relay → gateway: the registration holds.
    Registered,
    /// Relay → gateway: a phone waits on `conn`; dial `/accept`.
    Open { conn: String },
    /// Relay → gateway, then the relay closes.
    Error { code: RelayError },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayError {
    /// The relay holds its maximum of ids, none of them offline.
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Auth,
    Version,
    BadRequest,
    Busy,
    Forbidden,
    NoPane,
    NoBlock,
    TooLarge,
    Rate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub pid: u32,
    pub tabs: Vec<Tab>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tab {
    pub index: usize,
    pub title: String,
    pub active: bool,
    pub panes: Vec<Pane>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pane {
    pub id: u64,
    pub active: bool,
    pub cwd: Option<String>,
    /// Foreground job other than the shell.
    pub job: Option<String>,
    pub last_command: Option<String>,
    pub last_exit: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockState {
    Running,
    Done,
}

/// One command block (a prompt being typed is never sent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub id: u64,
    pub command: String,
    pub state: BlockState,
    /// Exit status once `done` (may stay `None`).
    pub exit: Option<i32>,
    /// Elapsed while running, final duration when done.
    pub duration_ms: u64,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    /// Plain text, soft-wrapped rows joined, bounded by
    /// [`crate::bound_output`].
    pub output: String,
    /// Lines cut from the head of `output`.
    pub dropped: u64,
    pub restored: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::Path;

    fn fixtures(side: &str) -> Vec<(String, Value)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(side);
        let mut out: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .map(|e| {
                let text = std::fs::read_to_string(e.path()).unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    serde_json::from_str(&text).unwrap(),
                )
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    #[test]
    fn every_client_fixture_round_trips() {
        let all = fixtures("client");
        assert_eq!(all.len(), 17, "client fixtures");
        for (name, value) in all {
            let msg: ClientMsg =
                serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(serde_json::to_value(&msg).unwrap(), value, "{name}");
        }
    }

    #[test]
    fn every_server_fixture_round_trips() {
        let all = fixtures("server");
        assert_eq!(all.len(), 15, "server fixtures");
        for (name, value) in all {
            let msg: ServerMsg =
                serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(serde_json::to_value(&msg).unwrap(), value, "{name}");
        }
    }

    #[test]
    fn every_handshake_fixture_round_trips() {
        let all = fixtures("handshake");
        assert_eq!(all.len(), 3, "handshake fixtures");
        for (name, value) in all {
            let back = if name.starts_with("gateway") {
                let h: GatewayHello =
                    serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(h.v, VERSION);
                serde_json::to_value(&h).unwrap()
            } else {
                let h: ClientHello =
                    serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(h.v, VERSION);
                assert_eq!(h.pair.is_some(), name.contains("pair"), "{name}");
                serde_json::to_value(&h).unwrap()
            };
            assert_eq!(back, value, "{name}");
        }
    }

    #[test]
    fn every_relay_fixture_round_trips() {
        let all = fixtures("relay");
        assert_eq!(all.len(), 6, "relay fixtures");
        for (name, value) in all {
            let msg: RelayMsg =
                serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(serde_json::to_value(&msg).unwrap(), value, "{name}");
            // control messages are text frames of at most 1 KiB
            assert!(
                value.to_string().len() <= crate::relay::MAX_CONTROL,
                "{name}"
            );
        }
    }

    #[test]
    fn unknown_or_malformed_client_messages_are_rejected() {
        for bad in [
            r#"{"t":"send_text","pane":1,"text":"rm -rf /\n"}"#,
            r#"{"t":"write","pane":1}"#,
            r#"{"pane":1}"#,
            r#"{"t":"subscribe"}"#,
            r#"{"t":"hello","v":1,"token":"x"}"#,
            r#"{"t":"run","pane":1,"command":"ls"}"#,
            r#"{"t":"keys","id":"k","pane":1}"#,
            r#"{"t":"text","pane":1,"text":"x"}"#,
            r#"{"t":"screen","pane":1,"mode":"always"}"#,
            r#"{"t":"screen","pane":1}"#,
            r#"{"t":"output","id":"o","pane":1,"block":2,"from":0}"#,
            r#"{"t":"output","id":"o","pane":1,"block":2,"from":-1,"limit":5}"#,
            "not json",
        ] {
            assert!(serde_json::from_str::<ClientMsg>(bad).is_err(), "{bad}");
        }
        // Phase 2 clients leave `failures_only` (and `ntfy`) out: every long
        // command alerts, through Expo only
        assert_eq!(
            serde_json::from_str::<ClientMsg>(
                r#"{"t":"push","token":null,"long_ms":null,"show_command":false}"#
            )
            .unwrap(),
            ClientMsg::Push {
                token: None,
                ntfy: None,
                long_ms: None,
                show_command: false,
                failures_only: false
            }
        );
        // `after` may be omitted: a fresh subscription
        assert_eq!(
            serde_json::from_str::<ClientMsg>(r#"{"t":"subscribe","pane":3}"#).unwrap(),
            ClientMsg::Subscribe {
                pane: 3,
                after: None
            }
        );
    }

    #[test]
    fn phase_two_gateways_have_no_features_and_runs_are_arrays() {
        // a welcome without `features` (an older gateway) serves none of them
        let w: ServerMsg = serde_json::from_str(
            r#"{"t":"welcome","v":2,"host":"h","os":"linux","version":"0","device":"d","permission":"view"}"#,
        )
        .unwrap();
        assert!(matches!(w, ServerMsg::Welcome { features, .. } if features.is_empty()));
        let run = Run {
            text: "ok".into(),
            fg: Some("e5c07b".into()),
            bg: None,
            flags: Run::BOLD | Run::UNDERLINE,
        };
        assert_eq!(
            serde_json::to_string(&run).unwrap(),
            r#"["ok","e5c07b",null,5]"#
        );
        assert!(serde_json::from_str::<Run>(r#"["ok",null,null]"#).is_err());
        assert!(serde_json::from_str::<Run>(r#"{"text":"ok"}"#).is_err());
    }
}
