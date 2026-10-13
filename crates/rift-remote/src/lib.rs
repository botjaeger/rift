//! Remote gateway for the mobile companion (Phases 1–3 of
//! `docs/mobile-remote-plan.md`; wire format in `docs/remote-protocol.md`,
//! version 2).
//!
//! The gateway serves paired devices over WebSocket, every frame a Noise
//! (`Noise_XX_25519_ChaChaPoly_BLAKE2s`) message ([`transport`]). It polls a
//! [`Source`] for the window topology, the blocks of subscribed panes and
//! (Phase 3) their screens, pages block output on request, and hands
//! validated `run` / `keys` / `text` requests of `operate` devices to
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
use serde::{Deserialize, Serialize};
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
/// Longest `id` of `run` / `keys` / `text` / `output`.
pub const MAX_REQUEST_ID: usize = 64;
/// Longest `text`, in bytes (it counts against [`KEYS_RATE`]).
pub const MAX_TEXT: usize = 1024;
/// `output` requests per second per connection.
pub const OUTPUT_RATE: usize = 10;
/// Most lines per `output` answer (also bounded by [`MAX_OUTPUT_BYTES`]).
pub const MAX_OUTPUT_PAGE_LINES: usize = 500;
/// Largest screen sent (`screen`): a larger one is cut to its top-left.
pub const MAX_SCREEN_COLS: u16 = 300;
pub const MAX_SCREEN_ROWS: u16 = 120;
/// Most zero-width characters (combining marks) a screen cell carries; the
/// rest are left out.
pub const MAX_CELL_MARKS: usize = 16;
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

/// A pane's visible terminal screen ([`Source::screen`]), at most
/// [`MAX_SCREEN_COLS`] × [`MAX_SCREEN_ROWS`] (the top-left part).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screen {
    pub cols: u16,
    pub rows: u16,
    /// The pane shows the alternate screen.
    pub alt: bool,
    /// `[row, column]` while the program shows its cursor.
    pub cursor: Option<[u16; 2]>,
    /// Row 0 first, one entry per row (trailing default blanks left out).
    pub lines: Vec<Vec<protocol::Run>>,
}

/// Lines of a block's output ([`Source::output`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputPage {
    /// Number of the first line of `text` (0 is the block's first output
    /// line, the way `Block.dropped` counts).
    pub from: u64,
    /// Whole lines, each ending with a line break except the block's
    /// unfinished last line ([`bound_page`]).
    pub text: String,
    /// The block's output lines so far.
    pub total: u64,
}

/// Why [`Source::output`] has no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputError {
    /// No such block in the pane (or one being typed at the prompt).
    NoBlock,
    NoPane,
    /// The window owning it did not answer this time.
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

/// A key a client may send (`keys`), by its protocol name: optional
/// modifiers (`ctrl+`, `alt+`, `shift+`, in that order) and a base key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub base: KeyBase,
}

/// The key under a [`Key`]'s modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyBase {
    Esc,
    Tab,
    Enter,
    Backspace,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// `f1` … `f12`.
    F(u8),
    /// `a` … `z`, only with `ctrl` and/or `alt` and without `shift`.
    Letter(char),
    /// Only with `ctrl` and/or `alt` and without `shift`.
    Space,
}

impl KeyBase {
    const NAMED: [(&'static str, KeyBase); 15] = [
        ("esc", KeyBase::Esc),
        ("tab", KeyBase::Tab),
        ("enter", KeyBase::Enter),
        ("backspace", KeyBase::Backspace),
        ("delete", KeyBase::Delete),
        ("insert", KeyBase::Insert),
        ("up", KeyBase::Up),
        ("down", KeyBase::Down),
        ("left", KeyBase::Left),
        ("right", KeyBase::Right),
        ("home", KeyBase::Home),
        ("end", KeyBase::End),
        ("pageup", KeyBase::PageUp),
        ("pagedown", KeyBase::PageDown),
        ("space", KeyBase::Space),
    ];

    fn parse(name: &str) -> Option<KeyBase> {
        if let Some(n) = KeyBase::NAMED.iter().find(|(n, _)| *n == name) {
            return Some(n.1);
        }
        // f1 … f12, no leading zero or sign
        if let Some(n) = name.strip_prefix('f')
            && let Ok(n) = n.parse::<u8>()
            && (1..=12).contains(&n)
            && name == format!("f{n}")
        {
            return Some(KeyBase::F(n));
        }
        let mut chars = name.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_lowercase() => Some(KeyBase::Letter(c)),
            _ => None,
        }
    }

    fn name(self) -> String {
        match self {
            KeyBase::F(n) => format!("f{n}"),
            KeyBase::Letter(c) => c.to_string(),
            b => KeyBase::NAMED
                .iter()
                .find(|(_, x)| *x == b)
                .map(|(n, _)| (*n).to_string())
                .unwrap_or_default(),
        }
    }
}

impl Key {
    /// `base` without modifiers.
    pub const fn plain(base: KeyBase) -> Key {
        Key {
            ctrl: false,
            alt: false,
            shift: false,
            base,
        }
    }

    /// `ctrl+` the letter `c`.
    pub const fn ctrl(c: char) -> Key {
        Key {
            ctrl: true,
            ..Key::plain(KeyBase::Letter(c))
        }
    }

    /// The key named exactly `name` (lowercase, canonical modifier order,
    /// as in the protocol).
    pub fn parse(name: &str) -> Option<Key> {
        let mut rest = name;
        let mut take = |m: &str| match rest.strip_prefix(m) {
            Some(r) => {
                rest = r;
                true
            }
            None => false,
        };
        let ctrl = take("ctrl+");
        let alt = take("alt+");
        let shift = take("shift+");
        let base = KeyBase::parse(rest)?;
        // a plain or shifted letter (or space) is text, not a key
        if matches!(base, KeyBase::Letter(_) | KeyBase::Space) && (shift || !(ctrl || alt)) {
            return None;
        }
        Some(Key {
            ctrl,
            alt,
            shift,
            base,
        })
    }

    /// The protocol name.
    pub fn name(self) -> String {
        let mut out = String::new();
        for (on, m) in [
            (self.ctrl, "ctrl+"),
            (self.alt, "alt+"),
            (self.shift, "shift+"),
        ] {
            if on {
                out.push_str(m);
            }
        }
        out.push_str(&self.base.name());
        out
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
    /// Text typed into the program running in the pane (validated: one
    /// line, no control characters, 1 to [`MAX_TEXT`] bytes).
    Text(String),
}

/// Why [`Source::input`] did not deliver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputError {
    /// `run`: the pane is running something; `text`: the pane is at its
    /// prompt.
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
    /// `pane`'s visible screen while it shows the alternate screen, or
    /// always when `force`; `Ok(None)` otherwise. Defaults to none.
    fn screen(&self, pane: u64, force: bool) -> Result<Option<Screen>, SourceError> {
        let _ = (pane, force);
        Ok(None)
    }
    /// Up to `limit` output lines of `block` in `pane` from line `from` on
    /// (see [`OutputPage`]). Defaults to `NoPane`.
    fn output(
        &self,
        pane: u64,
        block: u64,
        from: u64,
        limit: usize,
    ) -> Result<OutputPage, OutputError> {
        let _ = (pane, block, from, limit);
        Err(OutputError::NoPane)
    }
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

/// The head of `text` (lines, each ending with `\n` but maybe the last) for
/// an `output` answer: at most `limit` lines and [`MAX_OUTPUT_BYTES`],
/// cut at the end of a line; a first line longer than the byte bound keeps
/// its first [`MAX_OUTPUT_BYTES`], cut on a char boundary.
pub fn bound_page(text: &str, limit: usize) -> &str {
    let mut end = 0;
    for (n, line) in text.split_inclusive('\n').enumerate() {
        if n == limit {
            break;
        }
        if end + line.len() > MAX_OUTPUT_BYTES {
            if n == 0 {
                let mut cut = MAX_OUTPUT_BYTES;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                return &text[..cut];
            }
            break;
        }
        end += line.len();
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_bounded_by_lines_and_bytes_at_a_line_end() {
        assert_eq!(bound_page("a\nb\nc", 2), "a\nb\n");
        assert_eq!(bound_page("a\nb\nc", 3), "a\nb\nc");
        assert_eq!(bound_page("a\nb\n", 500), "a\nb\n");
        assert_eq!(bound_page("a\n", 0), "");
        assert_eq!(bound_page("", 5), "");
        // bytes: whole lines only
        let line = format!("{}\n", "x".repeat(999));
        let text = line.repeat(100);
        let page = bound_page(&text, 500);
        assert_eq!(page.len(), 65 * 1000);
        assert!(page.ends_with('\n'));
        // one line over the bound keeps its first 64 KiB, on a char boundary
        let huge = format!("{}\nnext\n", "é€".repeat(30_000));
        let page = bound_page(&huge, 500);
        assert!(page.len() <= MAX_OUTPUT_BYTES && page.len() > MAX_OUTPUT_BYTES - 4);
        assert!(huge.starts_with(page) && !page.contains('\n'));
    }

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

    /// Every canonical key name, built from the protocol's words.
    fn every_key_name() -> Vec<String> {
        let mut named: Vec<String> = [
            "esc",
            "tab",
            "enter",
            "backspace",
            "delete",
            "insert",
            "up",
            "down",
            "left",
            "right",
            "home",
            "end",
            "pageup",
            "pagedown",
        ]
        .map(String::from)
        .to_vec();
        named.extend((1..=12).map(|n| format!("f{n}")));
        let mut letters: Vec<String> = ('a'..='z').map(String::from).collect();
        letters.push("space".into());
        let mut names = Vec::new();
        for mods in [
            "",
            "ctrl+",
            "alt+",
            "shift+",
            "ctrl+alt+",
            "ctrl+shift+",
            "alt+shift+",
            "ctrl+alt+shift+",
        ] {
            names.extend(named.iter().map(|b| format!("{mods}{b}")));
        }
        for mods in ["ctrl+", "alt+", "ctrl+alt+"] {
            names.extend(letters.iter().map(|b| format!("{mods}{b}")));
        }
        names
    }

    #[test]
    fn key_names_are_exactly_the_protocol_list() {
        let names = every_key_name();
        // 8 modifier sets × 26 named keys, 3 × 27 letters and space
        assert_eq!(names.len(), 8 * 26 + 3 * 27);
        let mut seen = std::collections::HashSet::new();
        for n in &names {
            let k = Key::parse(n).unwrap_or_else(|| panic!("{n}"));
            assert_eq!(&k.name(), n);
            assert!(seen.insert(k), "{n} parsed to a key seen before");
        }
        assert_eq!(Key::parse("ctrl+c"), Some(Key::ctrl('c')));
        assert_eq!(
            Key::parse("shift+f5"),
            Some(Key {
                shift: true,
                ..Key::plain(KeyBase::F(5))
            })
        );
        for bad in [
            "",
            "ctrl+",
            "alt+",
            "ctrl+A",
            "ctrl+ab",
            "ctrl+1",
            "ctrl+é",
            "shift+ctrl+c",
            "alt+ctrl+x",
            "shift+alt+up",
            "shift+a",
            "ctrl+shift+a",
            "alt+shift+space",
            "shift+space",
            "a",
            "space",
            "alt+alt+x",
            "ctrl+ctrl+c",
            "ctrl+f13",
            "f0",
            "f01",
            "f+1",
            "f",
            "F1",
            "Esc",
            "ENTER",
            " up",
            "up ",
            "ctrl+ c",
            "ctrl-c",
            "ctrl+c\n",
            "page_up",
            "meta+x",
            "cmd+c",
            "super+c",
            "shift+",
            "+",
        ] {
            assert_eq!(Key::parse(bad), None, "{bad:?}");
        }
    }
}
