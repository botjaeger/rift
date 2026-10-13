//! Remote gateway for the mobile companion (Phases 1–2 of
//! `docs/mobile-remote-plan.md`; wire format in `docs/remote-protocol.md`,
//! version 2).
//!
//! The gateway serves paired devices over WebSocket, every frame a Noise
//! (`Noise_XX_25519_ChaChaPoly_BLAKE2s`) message ([`transport`]). It polls a
//! [`Source`] for the window topology and the blocks of subscribed panes, and
//! hands validated `run` / `keys` requests of `operate` devices to
//! [`Source::input`]. Paired devices, pairing secrets, the gateway's static
//! key, its status and the connection history live under
//! `<data dir>/remote/` ([`registry`]). Outside the local network, phones
//! reach it through a [`relay`] that only splices ciphertext; the gateway
//! keeps a control connection there (`uplink`).

pub mod protocol;
pub mod push;
pub mod registry;
pub mod relay;
mod server;
pub mod transport;
mod uplink;

pub use server::{Config, Gateway, serve};

use protocol::{Block, Window};
use std::time::Duration;

/// Largest JSON message a client may send; a larger one closes the
/// connection.
pub const MAX_CLIENT_MESSAGE: usize = 4 * 1024;
/// Largest JSON message the gateway sends (in ≤ [`transport::MAX_CHUNK`]
/// byte chunks).
pub const MAX_GATEWAY_MESSAGE: usize = 1024 * 1024;
/// Concurrent connections; the next one gets `error` `busy` after the
/// handshake.
pub const MAX_CONNECTIONS: usize = 8;
/// The Noise handshake must finish this soon after the socket opens.
pub const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(5);
/// Failed handshakes per remote address within [`FAILURE_WINDOW`] before
/// that address is refused for [`REFUSAL`].
pub const MAX_FAILURES: usize = 10;
pub const FAILURE_WINDOW: Duration = Duration::from_secs(60);
pub const REFUSAL: Duration = Duration::from_secs(60);
/// The gateway pings every client this often.
pub const PING_INTERVAL: Duration = Duration::from_secs(15);
/// A client silent this long is dropped.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
/// Blocks per `snapshot` / `older`.
pub const MAX_BLOCKS: usize = 30;
/// Output per block: the last this many lines ...
pub const MAX_OUTPUT_LINES: usize = 500;
/// ... and at most this many bytes.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
/// `update`s kept per pane for resuming subscriptions.
pub const REPLAY_RING: usize = 256;
/// Poll interval of subscribed panes.
pub const PANE_POLL: Duration = Duration::from_millis(100);
/// Poll interval of the topology (and of the `done` probe of the panes no
/// client follows).
pub const TOPOLOGY_POLL: Duration = Duration::from_secs(1);
/// `devices.json` is re-read this often (revocation closes connections).
pub const DEVICES_POLL: Duration = Duration::from_millis(500);
/// `run` requests per second per connection.
pub const RUN_RATE: usize = 4;
/// Longest `run` command, in bytes.
pub const MAX_COMMAND: usize = 2 * 1024;
/// Keys per `keys` message, and `keys` messages per second per connection.
pub const MAX_KEYS: usize = 20;
pub const KEYS_RATE: usize = 20;
/// Longest `id` of `run` / `keys`.
pub const MAX_REQUEST_ID: usize = 64;
/// The gateway's relay connection is tried again after this long, doubling
/// up to [`RELAY_BACKOFF_MAX`] (each with up to half again of jitter),
/// and from the start again once it registered.
pub const RELAY_BACKOFF_MIN: Duration = Duration::from_secs(1);
pub const RELAY_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Why [`Source::blocks`] has no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// The pane does not exist (closed, or never did).
    NoPane,
    /// The window owning it did not answer this time (busy, restarting).
    Unavailable(String),
}

/// One read of a pane's blocks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    /// Newest last.
    pub blocks: Vec<Block>,
    /// Blocks older than the first of `blocks` exist.
    pub has_older: bool,
    /// The pane's oldest block that would be sent (any page), if any:
    /// every id below it is gone (scrollback trim, `clear`).
    pub oldest: Option<u64>,
}

/// A key a client may send (`keys`), by its protocol name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// `ctrl+a` … `ctrl+z` (the lowercase letter).
    Ctrl(char),
    Esc,
    Tab,
    ShiftTab,
    Enter,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

impl Key {
    const NAMED: [(&'static str, Key); 13] = [
        ("esc", Key::Esc),
        ("tab", Key::Tab),
        ("shift+tab", Key::ShiftTab),
        ("enter", Key::Enter),
        ("backspace", Key::Backspace),
        ("up", Key::Up),
        ("down", Key::Down),
        ("left", Key::Left),
        ("right", Key::Right),
        ("home", Key::Home),
        ("end", Key::End),
        ("pageup", Key::PageUp),
        ("pagedown", Key::PageDown),
    ];

    /// The key named exactly `name` (lowercase, as in the protocol).
    pub fn parse(name: &str) -> Option<Key> {
        if let Some(c) = name.strip_prefix("ctrl+") {
            let mut chars = c.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_lowercase() => Some(Key::Ctrl(c)),
                _ => None,
            };
        }
        Key::NAMED.iter().find(|(n, _)| *n == name).map(|&(_, k)| k)
    }

    /// The protocol name.
    pub fn name(self) -> String {
        match self {
            Key::Ctrl(c) => format!("ctrl+{c}"),
            k => Key::NAMED
                .iter()
                .find(|(_, x)| *x == k)
                .map(|(n, _)| (*n).to_string())
                .unwrap_or_default(),
        }
    }
}

/// What a client sends to a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// A command line for a pane at its prompt (validated: one line, no
    /// control characters, at most [`MAX_COMMAND`] bytes).
    Command(String),
    /// Keys as if pressed on the desktop (at most [`MAX_KEYS`]).
    Keys(Vec<Key>),
}

/// Why [`Source::input`] did not deliver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputError {
    /// The pane is running something (`run` only).
    Busy,
    /// The pane does not exist.
    NoPane,
    /// The window refused remote input (`remote_access` is off).
    Denied,
    /// The window did not answer, or cannot take input now.
    Unavailable(String),
}

/// What the gateway reads, and where validated input goes.
pub trait Source: Send + Sync + 'static {
    /// Every window with its tabs and panes.
    fn topology(&self) -> Vec<Window>;
    /// Up to `limit` blocks of `pane` older than block id `before` (the
    /// newest when `None`).
    fn blocks(&self, pane: u64, before: Option<u64>, limit: usize) -> Result<Page, SourceError>;
    /// The newest `limit` blocks of `pane` without their output (`output`
    /// may be empty and `dropped` 0): the `done` probe of panes nobody
    /// follows needs only ids, states and exits. Defaults to [`blocks`].
    ///
    /// [`blocks`]: Source::blocks
    fn blocks_meta(&self, pane: u64, limit: usize) -> Result<Page, SourceError> {
        self.blocks(pane, None, limit)
    }
    /// Deliver `input` from the device named `device` to `pane`.
    fn input(&self, pane: u64, device: &str, input: Input) -> Result<(), InputError>;
    /// `status.json` was just rewritten (the gateway started, a client
    /// connected or left): tell whoever shows it. Called on the gateway's
    /// own threads, so it must return at once. Defaults to nothing.
    fn status_changed(&self) {}
}

/// Block ids tracked per pane for removal reports (the newest window plus
/// what clients paged with `older`); past it the lowest ids are forgotten.
pub const MAX_TRACKED: usize = REPLAY_RING * MAX_BLOCKS;

/// The tail of `text`: at most [`MAX_OUTPUT_LINES`] lines and
/// [`MAX_OUTPUT_BYTES`] bytes. Returns the kept text and the number of whole
/// lines removed from the head. A single last line longer than the byte
/// bound keeps its tail, cut on a char boundary (not counted as dropped).
pub fn bound_output(text: &str) -> (String, u64) {
    bound_output_with(text, MAX_OUTPUT_LINES, MAX_OUTPUT_BYTES)
}

/// [`bound_output`] with explicit bounds.
pub fn bound_output_with(text: &str, max_lines: usize, max_bytes: usize) -> (String, u64) {
    if text.len() <= max_bytes && text.lines().count() <= max_lines {
        return (text.to_string(), 0);
    }
    // byte offset where each line starts
    let starts: Vec<usize> = std::iter::once(0)
        .chain(
            text.match_indices('\n')
                .map(|(i, _)| i + 1)
                .filter(|&i| i < text.len()),
        )
        .collect();
    let by_lines = starts.len().saturating_sub(max_lines);
    // first line from which the rest fits in `max_bytes`
    let by_bytes = starts.partition_point(|&s| text.len() - s > max_bytes);
    let first = by_lines.max(by_bytes).min(starts.len() - 1);
    let mut from = starts[first];
    if text.len() - from > max_bytes {
        from = text.len() - max_bytes;
        while !text.is_char_boundary(from) {
            from += 1;
        }
    }
    (text[from..].to_string(), first as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_output_is_kept_whole() {
        assert_eq!(bound_output("a\nb\n"), ("a\nb\n".to_string(), 0));
        assert_eq!(bound_output(""), (String::new(), 0));
    }

    #[test]
    fn long_output_keeps_the_last_500_lines() {
        let text: String = (1..=200_000).map(|i| format!("{i}\n")).collect();
        let (out, dropped) = bound_output(&text);
        assert_eq!(dropped, 199_500);
        assert_eq!(out.lines().count(), 500);
        assert!(out.starts_with("199501\n") && out.ends_with("200000\n"));
        assert!(out.len() <= MAX_OUTPUT_BYTES);
        // without a final newline the last partial line counts too
        let (out, dropped) = bound_output(text.trim_end());
        assert_eq!((out.lines().count(), dropped), (500, 199_500));
    }

    #[test]
    fn wide_lines_are_bounded_by_bytes() {
        let line = format!("{}\n", "x".repeat(999));
        let text = line.repeat(100); // 100 KB in 100 lines
        let (out, dropped) = bound_output(&text);
        assert!(out.len() <= MAX_OUTPUT_BYTES);
        assert_eq!(out.len(), 65 * 1000);
        assert_eq!(dropped, 35);
        assert!(out.starts_with('x'));
    }

    #[test]
    fn one_huge_line_is_cut_on_a_char_boundary() {
        // 3-byte chars: 64 KiB is not a multiple of 3
        let text = "é€".repeat(30_000);
        let (out, dropped) = bound_output(&text);
        assert_eq!(dropped, 0);
        assert!(out.len() <= MAX_OUTPUT_BYTES && out.len() > MAX_OUTPUT_BYTES - 4);
        assert!(text.ends_with(&out));
        let (out, _) = bound_output_with("aé€", 10, 4);
        assert_eq!(out, "€");
    }

    #[test]
    fn key_names_are_exactly_the_protocol_list() {
        let mut names: Vec<String> = ('a'..='z').map(|c| format!("ctrl+{c}")).collect();
        names.extend(
            [
                "esc",
                "tab",
                "shift+tab",
                "enter",
                "backspace",
                "up",
                "down",
                "left",
                "right",
                "home",
                "end",
                "pageup",
                "pagedown",
            ]
            .map(String::from),
        );
        for n in &names {
            let k = Key::parse(n).unwrap_or_else(|| panic!("{n}"));
            assert_eq!(&k.name(), n);
        }
        assert_eq!(names.len(), 39);
        for bad in [
            "",
            "ctrl+",
            "ctrl+A",
            "ctrl+ab",
            "ctrl+1",
            "ctrl+é",
            "Esc",
            "ENTER",
            " up",
            "up ",
            "alt+x",
            "f1",
            "space",
            "a",
            "ctrl+c\n",
            "page_up",
            "shift+enter",
        ] {
            assert_eq!(Key::parse(bad), None, "{bad:?}");
        }
    }
}
