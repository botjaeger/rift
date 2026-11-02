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

use protocol::{Block, CloseKind, OpenKind, Window};
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
/// After a device scrolls or types into a pane, its screen is read this
/// often ([`KICK_WINDOW`] long): the program's redraw reaches the phone in a
/// frame or two instead of at the next [`PANE_POLL`].
pub const KICK_POLL: Duration = Duration::from_millis(16);
/// How long a scroll or keystroke keeps its pane's screen on [`KICK_POLL`].
pub const KICK_WINDOW: Duration = Duration::from_millis(600);
/// Poll interval of the topology (and of the `done` probe of the panes no
/// client follows).
pub const TOPOLOGY_POLL: Duration = Duration::from_secs(1);
/// `devices.json` is re-read this often (revocation closes connections).
pub const DEVICES_POLL: Duration = Duration::from_millis(500);
/// `run` requests per second per connection.
pub const RUN_RATE: usize = 4;
/// `open` requests per second per connection.
pub const OPEN_RATE: usize = 2;
/// Longest `run` command, in bytes.
pub const MAX_COMMAND: usize = 2 * 1024;
/// Keys per `keys` message, and `keys` messages per second per connection.
pub const MAX_KEYS: usize = 20;
pub const KEYS_RATE: usize = 20;
/// Longest `id` of `run` / `keys` / `text` / `open` / `output`.
pub const MAX_REQUEST_ID: usize = 64;
/// Longest `text`, in bytes (it counts against [`KEYS_RATE`]).
pub const MAX_TEXT: usize = 1024;
/// How often the gateway asks whether a program copied something for phones.
pub const COPY_POLL: std::time::Duration = std::time::Duration::from_millis(500);
/// Longest `paste`, in bytes (it counts against [`KEYS_RATE`]). Its message
/// may reach [`MAX_UPLOAD_MESSAGE`], which leaves room for JSON escapes.
pub const MAX_PASTE: usize = 48 * 1024;
/// Most lines one `scroll` moves (a larger one is clamped).
pub const MAX_SCROLL_LINES: i32 = 50;
/// `scroll` messages per second per connection, counted apart from `keys`:
/// a finger's drag goes out in small steps (one per 30 ms) so the program
/// moves smoothly.
pub const SCROLL_RATE: usize = 40;
/// Largest image or file an `upload` carries, in bytes.
pub const MAX_UPLOAD_BYTES: u64 = 10 * 1024 * 1024;
/// Most bytes in one `upload` chunk (60 000 characters of base64, which a
/// chunk boundary never splits: a multiple of 3 bytes).
pub const MAX_UPLOAD_CHUNK: usize = 45_000;
/// Largest JSON message of an `upload` chunk; every other client message
/// stays within [`MAX_CLIENT_MESSAGE`].
pub const MAX_UPLOAD_MESSAGE: usize = 62 * 1024;
/// `upload`s started per second per connection.
pub const UPLOAD_RATE: usize = 1;
/// Images being received at once across the gateway (each holds up to
/// [`MAX_UPLOAD_BYTES`] until it is pasted); another one is `busy`.
pub const MAX_UPLOADS: usize = 2;
/// How long one `upload` may take from its first chunk to its last; past
/// it what arrived is dropped and the upload answered `busy`.
pub const UPLOAD_DEADLINE: Duration = Duration::from_secs(60);

/// Most characters in an `upload`'s `ext`.
pub const MAX_UPLOAD_EXT: usize = 8;

/// Whether `ext` may end a saved upload's name: 1 to [`MAX_UPLOAD_EXT`] of
/// `a-z0-9`, so nothing a phone sends can be a path or need quoting.
pub fn valid_upload_ext(ext: &str) -> bool {
    (1..=MAX_UPLOAD_EXT).contains(&ext.len())
        && ext
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// An image type an `upload` may carry, told by its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
}

impl ImageKind {
    /// The kind whose signature `head` starts with: PNG's 8 bytes or
    /// JPEG's SOI marker and the next marker's `FF`.
    pub fn sniff(head: &[u8]) -> Option<ImageKind> {
        const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
        if head.starts_with(PNG) {
            Some(ImageKind::Png)
        } else if head.starts_with(b"\xff\xd8\xff") {
            Some(ImageKind::Jpeg)
        } else {
            None
        }
    }

    /// The file name extension the desktop saves it with.
    pub fn extension(self) -> &'static str {
        match self {
            ImageKind::Png => "png",
            ImageKind::Jpeg => "jpg",
        }
    }
}
/// `output` requests per second per connection.
pub const OUTPUT_RATE: usize = 10;
/// `fit` requests per second per connection (the app re-sends its fit every
/// [`FIT_LEASE`] / 3).
pub const FIT_RATE: usize = 4;
/// How long the desktop keeps a `fit` that was not refreshed.
pub const FIT_LEASE: Duration = Duration::from_secs(30);
/// Bounds of a `fit` (columns, rows); a value outside is clamped.
pub const FIT_COLS: std::ops::RangeInclusive<u16> = 20..=300;
pub const FIT_ROWS: std::ops::RangeInclusive<u16> = 5..=120;
/// Most lines per `output` answer (also bounded by [`MAX_OUTPUT_BYTES`]).
pub const MAX_OUTPUT_PAGE_LINES: usize = 500;
/// Most `dirs`, `branches` or `files` in one chip menu answer.
pub const MAX_MENU_ITEMS: usize = 500;
/// Longest name or path in a chip menu answer, in bytes (longer ones are left out).
pub const MAX_MENU_NAME: usize = 1024;
/// Most lines of a `diff` answer, its bytes, and the characters kept per line.
pub const MAX_DIFF_LINES: usize = 2000;
pub const MAX_DIFF_BYTES: usize = 256 * 1024;
pub const MAX_DIFF_LINE_CHARS: usize = 1000;
/// Longest `path` of a `diff` request, in bytes.
pub const MAX_DIFF_PATH: usize = MAX_MENU_NAME;
/// Longest `complete` line, and longest completion or history `text` kept,
/// in bytes; most `items` and `history` entries in one answer.
pub const MAX_COMPLETE_LINE: usize = 1024;
pub const MAX_COMPLETE_TEXT: usize = 1024;
pub const MAX_COMPLETE_ITEMS: usize = 50;
/// A pane's `waiting`: characters of its prompt, choices kept and
/// characters per choice, context lines kept and characters per line (the
/// window's `rift_core::prompts` bounds; a rift-terminal test keeps both
/// equal).
pub const MAX_WAITING_PROMPT: usize = 300;
pub const MAX_WAITING_CHOICES: usize = 9;
pub const MAX_WAITING_CHOICE: usize = 200;
pub const MAX_WAITING_CONTEXT: usize = 12;
/// A pane's `chips`: chips kept, parts kept per chip text and characters
/// per part (a folder's path can be long).
pub const MAX_PANE_CHIPS: usize = 16;
pub const MAX_CHIP_PARTS: usize = 6;
pub const MAX_CHIP_TEXT: usize = 200;
/// Characters of a pane's `running.line`.
pub const MAX_RUNNING_LINE: usize = 200;
/// Characters of a tab's `title` and a pane's `job` and `last_command`
/// (a pasted megabyte run as one command would otherwise push the whole
/// `topology` past [`MAX_GATEWAY_MESSAGE`], and it would never be sent).
pub const MAX_PANE_TEXT: usize = 1024;
/// Bytes of a block's `command` sent (cut on a character boundary): a block
/// over [`MAX_GATEWAY_MESSAGE`] could go out in no `snapshot`, `update` or
/// `older`, and the blocks older than it would not either.
pub const MAX_BLOCK_COMMAND: usize = 16 * 1024;

/// `blocks` within the wire bounds: each `command` cut to
/// [`MAX_BLOCK_COMMAND`] bytes (the output is bounded where it is read).
pub fn bound_blocks(blocks: &mut [Block]) {
    for b in blocks {
        if b.command.len() > MAX_BLOCK_COMMAND {
            let mut end = MAX_BLOCK_COMMAND;
            while !b.command.is_char_boundary(end) {
                end -= 1;
            }
            b.command.truncate(end);
        }
    }
}
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
    /// The theme's default foreground / background, `"rrggbb"`.
    #[serde(default)]
    pub fg: Option<String>,
    #[serde(default)]
    pub bg: Option<String>,
    /// Rows carrying a block rule and its colour.
    #[serde(default)]
    pub rules: Vec<(u16, String)>,
    /// Row 0 first, one entry per row (trailing default blanks left out).
    pub lines: Vec<Vec<protocol::Run>>,
    /// A capture the pane's output has moved on from, this long ago in whole
    /// seconds ([`Screen`]s of a window in use; `docs/remote-protocol.md`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_ms: Option<u64>,
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

/// A prompt chip's menu (feature `chips`), as the window answers it
/// ([`Source::menu`]); the gateway bounds it ([`bound_menu`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "menu", rename_all = "snake_case")]
pub enum Menu {
    Dirs {
        cwd: Option<String>,
        parent: bool,
        dirs: Vec<String>,
    },
    Branches {
        current: Option<String>,
        branches: Vec<String>,
    },
    Changes {
        branch: Option<String>,
        files: Vec<protocol::ChangedFile>,
    },
    /// The pull request, interpreter, Docker and Time on tab chips'
    /// menus (feature `prompt_chips`).
    Chip { detail: protocol::ChipDetail },
}

/// Which chip menu [`Source::menu`] reads; the last five are the `chip`
/// request's kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MenuKind {
    Dirs,
    Branches,
    Changes,
    Pr,
    Python,
    Php,
    Docker,
    Elapsed,
}

/// A changed file's diff ([`Source::diff`]), unbounded ([`bound_diff`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffPage {
    pub binary: bool,
    /// The window already cut it (the review's own line limit).
    #[serde(default)]
    pub truncated: bool,
    /// A file git does not track yet (status `?`): its whole content, which
    /// only operate devices are sent.
    #[serde(default)]
    pub untracked: bool,
    pub lines: Vec<protocol::DiffLine>,
}

/// `menu` within the wire bounds: names over [`MAX_MENU_NAME`] bytes left
/// out, at most [`MAX_MENU_ITEMS`] entries; `true` when anything was left out.
pub fn bound_menu(menu: Menu) -> (Menu, bool) {
    fn keep<T>(items: Vec<T>, name: impl Fn(&T) -> &str) -> (Vec<T>, bool) {
        let n = items.len();
        let kept: Vec<T> = items
            .into_iter()
            .filter(|i| name(i).len() <= MAX_MENU_NAME)
            .take(MAX_MENU_ITEMS)
            .collect();
        let more = kept.len() < n;
        (kept, more)
    }
    let short = |s: Option<String>| s.filter(|s| s.len() <= MAX_MENU_NAME);
    match menu {
        Menu::Dirs { cwd, parent, dirs } => {
            let (dirs, more) = keep(dirs, |d| d);
            (
                Menu::Dirs {
                    cwd: short(cwd),
                    parent,
                    dirs,
                },
                more,
            )
        }
        Menu::Branches { current, branches } => {
            let (branches, more) = keep(branches, |b| b);
            (
                Menu::Branches {
                    current: short(current),
                    branches,
                },
                more,
            )
        }
        Menu::Changes { branch, files } => {
            let (files, more) = keep(files, |f| &f.path);
            (
                Menu::Changes {
                    branch: short(branch),
                    files,
                },
                more,
            )
        }
        Menu::Chip { detail } => {
            use protocol::ChipDetail as D;
            let (detail, more) = match detail {
                D::Python { current, versions } => {
                    let (versions, more) = keep(versions, |v| &v.command);
                    (
                        D::Python {
                            current: short(current),
                            versions,
                        },
                        more,
                    )
                }
                D::Php { current, versions } => {
                    let (versions, more) = keep(versions, |v| &v.command);
                    (
                        D::Php {
                            current: short(current),
                            versions,
                        },
                        more,
                    )
                }
                D::Docker {
                    project,
                    error,
                    containers,
                } => {
                    let (containers, more) = keep(containers, |c| &c.logs);
                    (
                        D::Docker {
                            project: short(project),
                            error: short(error),
                            containers,
                        },
                        more,
                    )
                }
                other => (other, false),
            };
            (Menu::Chip { detail }, more)
        }
    }
}

/// `page`'s lines within the wire bounds: each cut at
/// [`MAX_DIFF_LINE_CHARS`] characters, at most [`MAX_DIFF_LINES`] lines and
/// [`MAX_DIFF_BYTES`] bytes; `true` when anything was cut (or the window had
/// cut it already).
pub fn bound_diff(page: DiffPage) -> (Vec<protocol::DiffLine>, bool) {
    let mut cut = page.truncated;
    let mut bytes = 0usize;
    let mut out = Vec::new();
    for protocol::DiffLine(kind, text) in page.lines {
        if out.len() >= MAX_DIFF_LINES {
            cut = true;
            break;
        }
        let text: String = if text.chars().count() > MAX_DIFF_LINE_CHARS {
            cut = true;
            text.chars().take(MAX_DIFF_LINE_CHARS).collect()
        } else {
            text
        };
        if bytes + kind.len() + text.len() > MAX_DIFF_BYTES {
            cut = true;
            break;
        }
        bytes += kind.len() + text.len();
        out.push(protocol::DiffLine(kind, text));
    }
    (out, cut)
}

/// `complete`'s answer ([`Source::complete`]), unbounded ([`bound_complete`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletePage {
    pub items: Vec<protocol::Completion>,
    pub history: Vec<protocol::HistoryEntry>,
}

/// `page` within the wire bounds: entries over [`MAX_COMPLETE_TEXT`] bytes
/// left out, at most [`MAX_COMPLETE_ITEMS`] of each list; `true` when
/// anything was left out.
pub fn bound_complete(page: CompletePage) -> (CompletePage, bool) {
    fn keep<T>(items: Vec<T>, text: impl Fn(&T) -> &str) -> (Vec<T>, bool) {
        let n = items.len();
        let kept: Vec<T> = items
            .into_iter()
            .filter(|i| text(i).len() <= MAX_COMPLETE_TEXT)
            .take(MAX_COMPLETE_ITEMS)
            .collect();
        let more = kept.len() < n;
        (kept, more)
    }
    let (items, a) = keep(page.items, |c| &c.text);
    let (history, b) = keep(page.history, |h| &h.text);
    (CompletePage { items, history }, a || b)
}

/// Every pane's `waiting` within the wire bounds (see the `MAX_WAITING_`
/// constants; a `password` never carries choices or context), its
/// `running.line` and `running.title` within [`MAX_RUNNING_LINE`], and tab titles, jobs and last
/// commands within [`MAX_PANE_TEXT`].
pub fn bound_topology(mut windows: Vec<Window>) -> Vec<Window> {
    let cut = |s: &mut String, max: usize| {
        if let Some((i, _)) = s.char_indices().nth(max) {
            s.truncate(i);
        }
    };
    for tab in windows.iter_mut().flat_map(|w| &mut w.tabs) {
        cut(&mut tab.title, MAX_PANE_TEXT);
    }
    for pane in windows
        .iter_mut()
        .flat_map(|w| &mut w.tabs)
        .flat_map(|t| &mut t.panes)
    {
        for text in [pane.job.as_mut(), pane.last_command.as_mut()]
            .into_iter()
            .flatten()
        {
            cut(text, MAX_PANE_TEXT);
        }
        if let Some(chips) = pane.chips.as_mut() {
            chips.truncate(MAX_PANE_CHIPS);
            for chip in chips.iter_mut() {
                for list in [&mut chip.full, &mut chip.short] {
                    list.truncate(MAX_CHIP_PARTS);
                    list.iter_mut()
                        .for_each(|p| cut(&mut p.text, MAX_CHIP_TEXT));
                }
            }
        }
        if let Some(r) = pane.running.as_mut() {
            cut(&mut r.line, MAX_RUNNING_LINE);
            if let Some(t) = r.title.as_mut() {
                cut(t, MAX_RUNNING_LINE);
            }
        }
        let Some(w) = pane.waiting.as_mut() else {
            continue;
        };
        cut(&mut w.prompt, MAX_WAITING_PROMPT);
        if w.kind == protocol::WaitingKind::Choice {
            w.choices.truncate(MAX_WAITING_CHOICES);
            w.choices
                .iter_mut()
                .for_each(|c| cut(c, MAX_WAITING_CHOICE));
            let skip = w.context.len().saturating_sub(MAX_WAITING_CONTEXT);
            w.context.drain(..skip);
            w.context
                .iter_mut()
                .for_each(|c| cut(c, MAX_WAITING_CHOICE));
            w.selected = w.selected.min(w.choices.len().saturating_sub(1));
        } else {
            w.choices.clear();
            w.context.clear();
            w.selected = 0;
            w.numbered = false;
        }
    }
    windows
}

#[cfg(test)]
mod chip_bounds {
    use super::*;
    use protocol::DiffLine;

    #[test]
    fn a_running_line_is_cut_to_its_bound() {
        let pane = |line: String| protocol::Pane {
            id: 1,
            active: true,
            cwd: None,
            job: Some("cargo".into()),
            last_command: None,
            last_exit: None,
            git: None,
            chips: None,
            waiting: None,
            running: Some(protocol::PaneRunning {
                started: Some(1),
                line,
                title: None,
                idle: false,
                agent: false,
            }),
        };
        let windows = vec![Window {
            pid: 1,
            background: false,
            tabs: vec![protocol::Tab {
                index: 0,
                title: "t".into(),
                active: true,
                panes: vec![pane("é".repeat(500)), pane("short".into())],
            }],
        }];
        let out = bound_topology(windows);
        let lines: Vec<&str> = out[0].tabs[0]
            .panes
            .iter()
            .map(|p| p.running.as_ref().unwrap().line.as_str())
            .collect();
        assert_eq!(lines[0].chars().count(), MAX_RUNNING_LINE);
        assert_eq!(lines[1], "short");
    }

    #[test]
    fn a_pasted_megabyte_command_still_lets_the_topology_out() {
        let big = "é".repeat(1024 * 1024);
        let pane = protocol::Pane {
            id: 1,
            active: true,
            cwd: None,
            job: Some(big.clone()),
            last_command: Some(big.clone()),
            last_exit: None,
            git: None,
            chips: None,
            waiting: None,
            running: None,
        };
        let windows = vec![Window {
            pid: 1,
            background: false,
            tabs: vec![protocol::Tab {
                index: 0,
                title: big,
                active: true,
                panes: vec![pane],
            }],
        }];
        let out = bound_topology(windows);
        let tab = &out[0].tabs[0];
        assert_eq!(tab.title.chars().count(), MAX_PANE_TEXT);
        let pane = &tab.panes[0];
        assert_eq!(pane.job.as_ref().unwrap().chars().count(), MAX_PANE_TEXT);
        assert_eq!(
            pane.last_command.as_ref().unwrap().chars().count(),
            MAX_PANE_TEXT
        );
        let frame = serde_json::to_string(&protocol::ServerMsg::Topology { windows: out }).unwrap();
        assert!(frame.len() <= MAX_GATEWAY_MESSAGE, "{} bytes", frame.len());
    }

    #[test]
    fn menus_leave_out_long_names_and_cap_entries() {
        let mut dirs: Vec<String> = (0..MAX_MENU_ITEMS + 3).map(|i| i.to_string()).collect();
        dirs.insert(0, "n".repeat(MAX_MENU_NAME + 1));
        let (m, more) = bound_menu(Menu::Dirs {
            cwd: Some("c".repeat(MAX_MENU_NAME + 1)),
            parent: false,
            dirs,
        });
        let Menu::Dirs { cwd, dirs, .. } = m else {
            unreachable!()
        };
        assert!(more);
        assert_eq!(cwd, None);
        assert_eq!(dirs.len(), MAX_MENU_ITEMS);
        assert_eq!(dirs[0], "0");
        let (_, more) = bound_menu(Menu::Branches {
            current: Some("main".into()),
            branches: vec!["main".into()],
        });
        assert!(!more);
    }

    #[test]
    fn diffs_are_cut_by_lines_bytes_and_line_length() {
        let line = |t: String| DiffLine(" ".into(), t);
        let (l, cut) = bound_diff(DiffPage {
            lines: vec![line("a".into()), line("b".into())],
            ..Default::default()
        });
        assert_eq!((l.len(), cut), (2, false));
        let (l, cut) = bound_diff(DiffPage {
            lines: (0..MAX_DIFF_LINES + 1)
                .map(|i| line(i.to_string()))
                .collect(),
            ..Default::default()
        });
        assert_eq!((l.len(), cut), (MAX_DIFF_LINES, true));
        let (l, cut) = bound_diff(DiffPage {
            lines: vec![line("é".repeat(MAX_DIFF_LINE_CHARS + 5))],
            ..Default::default()
        });
        assert_eq!((l[0].1.chars().count(), cut), (MAX_DIFF_LINE_CHARS, true));
        // 300 lines of 1,000 bytes stop under 256 KiB
        let (l, cut) = bound_diff(DiffPage {
            lines: (0..300).map(|_| line("z".repeat(1000))).collect(),
            ..Default::default()
        });
        assert!(
            cut && l.len() < 300
                && l.iter().map(|d| d.0.len() + d.1.len()).sum::<usize>() <= MAX_DIFF_BYTES
        );
        // the window's own cut is kept
        let (_, cut) = bound_diff(DiffPage {
            truncated: true,
            ..Default::default()
        });
        assert!(cut);
    }
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
    /// Text pasted into the program as one paste (validated: 1 to
    /// [`MAX_PASTE`] bytes, line breaks and tabs the only control
    /// characters).
    Paste(String),
    /// A left click at a cell of the pane's screen, 0-based.
    Mouse { col: u16, row: u16 },
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
    /// `scroll`: what the wheel does there sends the program input (a
    /// wheel report, arrow keys), which a `view` device may not.
    ViewOnly,
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
    /// What a program copied (OSC 52) after copy number `since`, for phones
    /// (`remote_clipboard`): the copy's number and its text. Defaults to none.
    fn clipboard(&self, since: u64) -> Option<(u64, String)> {
        let _ = since;
        None
    }
    /// `pane`'s visible screen while it shows the alternate screen, or
    /// always when `force`; `Ok(None)` otherwise. Defaults to none.
    fn screen(&self, pane: u64, force: bool) -> Result<Option<Screen>, SourceError> {
        let _ = (pane, force);
        Ok(None)
    }
    /// Hold `pane` at most `size` (columns, rows) for the phones following
    /// it (the smallest of them), or release it (`None`); called again
    /// within [`FIT_LEASE`] / 3 while held, which keeps the lease. Defaults
    /// to accepting and ignoring.
    fn fit(&self, pane: u64, size: Option<(u16, u16)>) -> Result<(), InputError> {
        let _ = (pane, size);
        Ok(())
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
    /// A prompt chip's menu for `pane` (feature `chips`): what the
    /// desktop's chip menus and code review panel list. Defaults to `NoPane`.
    fn menu(&self, pane: u64, kind: MenuKind) -> Result<Menu, OutputError> {
        let _ = (pane, kind);
        Err(OutputError::NoPane)
    }
    /// Restart the Time on tab clock of the tab holding `pane` for the
    /// device named `device` (feature `prompt_chips`). Defaults to
    /// `Unavailable`.
    fn reset_elapsed(&self, pane: u64, device: &str) -> Result<(), InputError> {
        let _ = (pane, device);
        Err(InputError::Unavailable(
            "this computer cannot reset the clock".into(),
        ))
    }
    /// The diff of one changed file of `pane`'s repository; `NoBlock` when
    /// `path` is not among the changes. Defaults to `NoPane`.
    fn diff(&self, pane: u64, path: &str, staged: bool) -> Result<DiffPage, OutputError> {
        let _ = (pane, path, staged);
        Err(OutputError::NoPane)
    }
    /// Completions and history for `line` in `pane` (feature `complete`;
    /// `cursor` is a byte offset at a character boundary). Defaults to
    /// `NoPane`.
    fn complete(&self, pane: u64, line: &str, cursor: usize) -> Result<CompletePage, OutputError> {
        let _ = (pane, line, cursor);
        Err(OutputError::NoPane)
    }
    /// Open a new tab or split next to `pane` for the device named `device`
    /// (feature `open`); the new pane's id. Defaults to `Unavailable`.
    fn open(&self, pane: u64, device: &str, kind: OpenKind) -> Result<u64, InputError> {
        let _ = (pane, device, kind);
        Err(InputError::Unavailable(
            "this computer cannot open shells".into(),
        ))
    }
    /// Close `pane`, or its tab, for the device named `device` (feature
    /// `close`), without asking the desktop's user. Defaults to
    /// `Unavailable`.
    fn close(&self, pane: u64, device: &str, kind: CloseKind) -> Result<(), InputError> {
        let _ = (pane, device, kind);
        Err(InputError::Unavailable(
            "this computer cannot close shells".into(),
        ))
    }
    /// Whether [`start`] may be used: the welcome then lists `start`.
    /// Defaults to false.
    ///
    /// [`start`]: Source::start
    fn can_start(&self) -> bool {
        false
    }
    /// Start a shell while no Rift window is open, for the device named
    /// `device` (feature `start`); the new pane's id. Defaults to
    /// `Unavailable`: the computer does not allow it.
    fn start(&self, device: &str) -> Result<u64, InputError> {
        let _ = device;
        Err(InputError::Unavailable(
            "this computer does not open shells while Rift is closed".into(),
        ))
    }
    /// Scroll `pane` by `lines` (positive: up; already within
    /// ±[`MAX_SCROLL_LINES`], not 0) as the desktop's wheel over it would.
    /// `operate`: the device may send the program input; without it only
    /// the pane's own view moves, and a wheel that would reach the program
    /// is [`InputError::ViewOnly`]. Defaults to `Unavailable`.
    fn scroll(&self, pane: u64, lines: i32, operate: bool) -> Result<(), InputError> {
        let _ = (pane, lines, operate);
        Err(InputError::Unavailable(
            "this computer cannot scroll from the phone".into(),
        ))
    }
    /// Paste the image `bytes` (validated: [`ImageKind::sniff`] says
    /// `kind`, at most [`MAX_UPLOAD_BYTES`]) from the device named `device`
    /// into the program running in `pane`, as a file dropped on the desktop
    /// is: saved under a name the desktop makes, its path pasted.
    /// `Busy` when `pane` is at its prompt. Defaults to `Unavailable`.
    fn upload(
        &self,
        pane: u64,
        device: &str,
        kind: ImageKind,
        bytes: &[u8],
    ) -> Result<(), InputError> {
        let _ = (pane, device, kind, bytes);
        Err(InputError::Unavailable(
            "this computer cannot take images from the phone".into(),
        ))
    }
    /// [`Source::upload`] for a file of any bytes (at most
    /// [`MAX_UPLOAD_BYTES`]), saved with the extension `ext`
    /// ([`valid_upload_ext`]) under a name the desktop makes; nothing runs
    /// it. Defaults to `Unavailable`.
    fn upload_file(
        &self,
        pane: u64,
        device: &str,
        ext: &str,
        bytes: &[u8],
    ) -> Result<(), InputError> {
        let _ = (pane, device, ext, bytes);
        Err(InputError::Unavailable(
            "this computer cannot take files from the phone".into(),
        ))
    }
    /// Whether `pane` would take an image or file now ([`Source::upload`]'s
    /// errors, without one), asked at an `upload`'s first chunk so a pane
    /// at its prompt is refused before the rest arrives. Defaults to `Ok`
    /// (the last chunk's [`Source::upload`] still decides).
    fn upload_ready(&self, pane: u64) -> Result<(), InputError> {
        let _ = pane;
        Ok(())
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
    fn images_are_told_by_their_signature_only() {
        assert_eq!(
            ImageKind::sniff(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"),
            Some(ImageKind::Png)
        );
        assert_eq!(
            ImageKind::sniff(b"\xff\xd8\xff\xe0\0\x10JFIF"),
            Some(ImageKind::Jpeg)
        );
        for bad in [
            b"GIF89a\0\0".as_slice(),
            b"\x89PNG\r\n\x1a",
            b"\xff\xd8",
            b"<svg xmlns",
            b"#!/bin/sh\n",
            b"",
        ] {
            assert_eq!(ImageKind::sniff(bad), None, "{bad:?}");
        }
        // a chunk is whole base64 quanta: no chunk boundary splits one
        assert_eq!(MAX_UPLOAD_CHUNK % 3, 0);
        // and a chunk's message is one Noise chunk
        const { assert!(MAX_UPLOAD_MESSAGE <= transport::MAX_CHUNK) };
        const { assert!(MAX_UPLOAD_CHUNK / 3 * 4 + 1024 <= MAX_UPLOAD_MESSAGE) };
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
