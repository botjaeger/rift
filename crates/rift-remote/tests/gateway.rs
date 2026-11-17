//! The gateway over real localhost sockets, fed by a fake source; the test
//! acts as the phone with `snow` through `rift_remote::transport::connect`.

use rift_remote::protocol::{
    Block, BlockState, ClientHello, CloseKind, MoveKind, OpenKind, Pane, Permission, Run, Tab,
    Window,
};
use rift_remote::protocol::{
    ChangedFile, Completion, CompletionKind, DiffLine, HistoryEntry, Waiting, WaitingKind,
};
use rift_remote::protocol::{
    ChipDetail, ChipKind, ClosingReason, ComposeContainer, PaneChip, Part, Scrollback, Theme, Tone,
};
use rift_remote::registry::{self, PairingLink};
use rift_remote::transport::{self, Channel, HandshakeError, Recv};
use rift_remote::{
    CompletePage, Config, DiffPage, Gateway, HistoryPage, ImageKind, Input, InputError, Key,
    KeyBase, Menu, MenuKind, OutputError, OutputPage, Page, Screen, Source, SourceError, serve,
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
    moves: Vec<(u64, String, MoveKind, usize)>,
    /// `start` allowed (the computer's `remote_access` with durable shells).
    can_start: bool,
    /// Devices that started a shell.
    starts: Vec<String>,
    /// What `fit` was told: (pane, size or release).
    fits: Vec<(u64, Option<(u16, u16)>)>,
    /// Panes whose clock was reset: (pane, device name).
    resets: Vec<(u64, String)>,
    /// Panes whose program tracks the mouse: a wheel there is input.
    tracking: Vec<u64>,
    /// What `scroll` was told: (pane, lines, operate).
    scrolls: Vec<(u64, i32, bool)>,
    /// What `upload` received: (pane, device name, kind, bytes).
    uploads: Vec<(u64, String, ImageKind, Vec<u8>)>,
    /// The newest copy a program made: (number, text).
    copy: Option<(u64, String)>,
    /// What `upload_file` received: (pane, device name, ext, bytes).
    files: Vec<(u64, String, String, Vec<u8>)>,
    /// What `upload_ready` answers (a pane at its prompt: `Busy`).
    ready_error: Option<InputError>,
    /// The desktop's theme (`None`: an older computer, no feature).
    theme: Option<Theme>,
    /// Each pane's numbered lines: (epoch, number of the first, lines).
    histories: BTreeMap<u64, (u64, u64, Vec<String>)>,
    /// Panes running a full-screen document viewer: its lines and the first
    /// one in view; the wheel scrolls it (`screen` draws it, see [`doc_screen`]).
    docs: BTreeMap<u64, (Vec<String>, i64)>,
    /// The doc viewer speeds up under a repeated wheel, as Claude Code does: each
    /// step of more than one line moves that many lines times its place in the
    /// run (at most 6); a single line moves one and ends the run.
    accelerate: bool,
    streak: i64,
    /// Every this many reads of a doc viewer's screen fails with "read again" (the
    /// window's answer while another reader holds its cache); 0: none.
    flaky_reads: usize,
    /// The doc viewer's status row ticks every 25 reads, about 0.1 s (Claude Code's
    /// clock): between two wheel steps, even ones that moved nothing.
    ticking: bool,
    /// Once the view's first line goes above `.0` scrolling up, it jumps `.1`
    /// lines further (Claude Code laying a long message out anew at its height).
    jump: Option<(i64, i64)>,
}

/// Rows of a [`State::docs`] viewer's screen: a sticky header, the document,
/// a status row that ticks with every read; a side panel at column 30.
const DOC_ROWS: usize = 14;

fn doc_screen(lines: &[String], top: i64, tick: usize) -> Screen {
    let row = |left: String, r: usize| {
        let mut l: String = left.chars().take(29).collect();
        while l.chars().count() < 30 {
            l.push(' ');
        }
        vec![
            Run {
                text: l,
                fg: None,
                bg: None,
                flags: 0,
            },
            Run {
                text: format!("│panel {r}"),
                fg: None,
                bg: None,
                flags: 0,
            },
        ]
    };
    let mut out = vec![row(format!("❯ section {}", top / 10), 0)];
    for r in 1..DOC_ROWS - 1 {
        let n = top + r as i64 - 1;
        out.push(row(lines.get(n as usize).cloned().unwrap_or_default(), r));
    }
    out.push(row(format!("status {tick}"), DOC_ROWS - 1));
    Screen {
        cols: 40,
        rows: DOC_ROWS as u16,
        alt: true,
        cursor: None,
        fg: None,
        bg: None,
        rules: Vec::new(),
        lines: out,
        age_ms: None,
        scrollback: None,
    }
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
                pinned: false,
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
                        chips: (id == 12).then(|| {
                            vec![PaneChip {
                                kind: ChipKind::Docker,
                                icon: Tone::Accent,
                                full: vec![
                                    Part {
                                        text: "flow".into(),
                                        tone: Tone::Bright,
                                    },
                                    Part {
                                        text: "3/4".into(),
                                        tone: Tone::Success,
                                    },
                                ],
                                // longer than the wire keeps
                                short: vec![Part {
                                    text: "é".repeat(500),
                                    tone: Tone::Success,
                                }],
                                attention: true,
                            }]
                        }),
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

    fn clipboard(&self, since: u64) -> Option<(u64, String)> {
        self.0
            .lock()
            .unwrap()
            .copy
            .clone()
            .filter(|(seq, _)| *seq > since)
    }

    fn theme(&self) -> Option<Theme> {
        self.0.lock().unwrap().theme.clone()
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

    fn move_to(
        &self,
        pane: u64,
        device: &str,
        kind: MoveKind,
        to: usize,
    ) -> Result<(), InputError> {
        let mut st = self.0.lock().unwrap();
        if !st.panes.contains_key(&pane) {
            return Err(InputError::NoPane);
        }
        st.moves.push((pane, device.to_string(), kind, to));
        Ok(())
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
        let reads = st.screen_reads.len();
        if st.docs.contains_key(&pane) && st.flaky_reads > 0 && reads.is_multiple_of(st.flaky_reads)
        {
            return Err(SourceError::Unavailable(
                "the screen moved; read again".into(),
            ));
        }
        if let Some((lines, top)) = st.docs.get(&pane) {
            // the status ticks only now and then: a screen at rest stays equal
            let tick = reads / if st.ticking { 25 } else { 1000 };
            return Ok(Some(doc_screen(lines, *top, tick)));
        }
        Ok(st.screens.get(&pane).filter(|s| s.alt || force).cloned())
    }

    /// A wheel over a mouse-tracking pane is input (operate only); any
    /// other pane's view moves for every device.
    fn scroll(&self, pane: u64, lines: i32, operate: bool) -> Result<(), InputError> {
        let mut st = self.0.lock().unwrap();
        if !st.panes.contains_key(&pane) {
            return Err(InputError::NoPane);
        }
        if st.tracking.contains(&pane) && !operate {
            return Err(InputError::ViewOnly);
        }
        st.scrolls.push((pane, lines, operate));
        let mut moved = lines as i64;
        if st.accelerate {
            if lines.abs() == 1 {
                st.streak = 0;
            } else {
                st.streak += 1;
                moved *= st.streak.min(6);
            }
        }
        let jump = st.jump;
        if let Some((doc, top)) = st.docs.get_mut(&pane) {
            let last = doc.len() as i64 - (DOC_ROWS as i64 - 2);
            *top = (*top - moved).clamp(0, last.max(0));
            if let Some((at, extra)) = jump
                && moved > 0
                && *top < at
            {
                *top = (*top - extra).max(0);
                st.jump = None;
            }
        }
        Ok(())
    }

    fn upload(
        &self,
        pane: u64,
        device: &str,
        kind: ImageKind,
        bytes: &[u8],
    ) -> Result<(), InputError> {
        let mut st = self.0.lock().unwrap();
        if let Some(e) = st.input_error.clone() {
            return Err(e);
        }
        if !st.panes.contains_key(&pane) {
            return Err(InputError::NoPane);
        }
        st.uploads
            .push((pane, device.to_string(), kind, bytes.to_vec()));
        Ok(())
    }

    fn upload_file(
        &self,
        pane: u64,
        device: &str,
        ext: &str,
        bytes: &[u8],
    ) -> Result<(), InputError> {
        let mut st = self.0.lock().unwrap();
        if let Some(e) = st.input_error.clone() {
            return Err(e);
        }
        if !st.panes.contains_key(&pane) {
            return Err(InputError::NoPane);
        }
        st.files
            .push((pane, device.to_string(), ext.to_string(), bytes.to_vec()));
        Ok(())
    }

    fn upload_ready(&self, pane: u64) -> Result<(), InputError> {
        let st = self.0.lock().unwrap();
        if !st.panes.contains_key(&pane) {
            return Err(InputError::NoPane);
        }
        st.ready_error.clone().map_or(Ok(()), Err)
    }

    fn reset_elapsed(&self, pane: u64, device: &str) -> Result<(), InputError> {
        let mut st = self.0.lock().unwrap();
        if !st.panes.contains_key(&pane) {
            return Err(InputError::NoPane);
        }
        st.resets.push((pane, device.to_string()));
        Ok(())
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
            MenuKind::Pr => Menu::Chip {
                detail: ChipDetail::Pr {
                    number: 42,
                    state: "open".into(),
                    url: "https://github.com/me/flow/pull/42".into(),
                },
            },
            MenuKind::Python => Menu::Chip {
                detail: ChipDetail::Python {
                    current: Some("3.12.4".into()),
                    versions: Vec::new(),
                },
            },
            // the wrong kind of answer, which the gateway must not forward
            MenuKind::Php => Menu::Chip {
                detail: ChipDetail::Python {
                    current: None,
                    versions: Vec::new(),
                },
            },
            MenuKind::Docker => Menu::Chip {
                detail: ChipDetail::Docker {
                    project: Some("flow".into()),
                    error: None,
                    containers: (0..600)
                        .map(|i| ComposeContainer {
                            service: format!("s{i:03}"),
                            state: "running".into(),
                            running: true,
                            ports: Vec::new(),
                            uptime: "2 hours".into(),
                            health: None,
                            logs: format!("docker compose logs s{i:03}"),
                        })
                        .collect(),
                },
            },
            MenuKind::Elapsed => Menu::Chip {
                detail: ChipDetail::Elapsed {
                    seconds: 120,
                    counting: true,
                    line: "Counted 2m \u{b7} running now".into(),
                },
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

    /// Every held line in `from` … `to - 1`, whatever its count: the
    /// gateway bounds the page. Another epoch: no lines.
    fn history(
        &self,
        pane: u64,
        epoch: u64,
        from: u64,
        to: u64,
    ) -> Result<HistoryPage, OutputError> {
        let st = self.0.lock().unwrap();
        let (now, first, lines) = st.histories.get(&pane).ok_or(OutputError::NoPane)?;
        if epoch != *now {
            return Ok(HistoryPage {
                epoch: *now,
                first: *first,
                from,
                ..Default::default()
            });
        }
        let end = first + lines.len() as u64;
        let (from, to) = (from.clamp(*first, end), to.clamp(*first, end));
        let held = &lines[(from - first) as usize..(to - first) as usize];
        Ok(HistoryPage {
            epoch: *now,
            first: *first,
            from,
            lines: held.iter().map(|l| vec![Run::plain(l)]).collect(),
            // a rule on every tenth line
            rules: (from..to)
                .filter(|n| n % 10 == 0)
                .map(|n| ((n - from) as u16, "3fb950".to_string()))
                .collect(),
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
    // readable sequences; `a_resume_from_an_earlier_gateway_run_…` uses the real base
    cfg.seq_base = Some(0);
    // a stitch job on a loaded machine (the gates run beside each other): what it stitches, not how fast
    cfg.page_job_time = Duration::from_secs(30);
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
    recv_within(ch, Duration::from_secs(10))
}

fn recv_within(ch: &mut Ch, wait: Duration) -> Value {
    let deadline = Instant::now() + wait;
    loop {
        assert!(Instant::now() < deadline, "no message within {wait:?}");
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
fn the_welcome_lists_the_configured_addresses_at_most_sixteen() {
    let many: Vec<SocketAddr> = (1..=20)
        .map(|i| format!("10.0.0.{i}:7878").parse().unwrap())
        .collect();
    let env = start(|c| c.addresses = Some(many.clone()));
    let (_, _, welcome) = env.pair(Permission::Operate);
    let want: Vec<String> = many.iter().take(16).map(|a| a.to_string()).collect();
    assert_eq!(welcome["addresses"], json!(want));
}

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
               "device":d.id,"permission":"operate","features":["screen","text","output","push_failures","push_ntfy","git","notify_off","chips","waiting","complete","running","open","close","move","fit","prompt_chips","closing","scroll","upload","upload_files","paste","mouse","clipboard","history","stitch"],
               // where it listens now (a specific address: that one)
               "addresses":[env.addr.to_string()]})
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
fn a_device_connecting_again_with_every_slot_taken_ends_only_its_silent_connections() {
    let env = start(|c| c.idle_timeout = Duration::from_secs(8));
    let (key, first, _) = env.pair(Permission::Operate);
    let mut stale: Vec<Ch> = vec![first];
    stale.extend((0..7).map(|_| env.login(&key)));
    // still heard (a phone paired twice): nothing ends, and the ninth waits
    let v = env.refused(&key, &hello(None));
    assert_eq!(code(&v), ("error", "busy"));
    // what a dropped network leaves: past half the idle timeout, unheard
    std::thread::sleep(Duration::from_millis(4200));
    let again = env.login(&key);
    for ch in &mut stale {
        assert_closed(ch);
    }
    drop(again);
}

#[test]
fn the_ninth_connection_is_busy() {
    let env = start(|_| {});
    let (key, first, _) = env.pair(Permission::Operate);
    let mut open: Vec<Ch> = vec![first];
    open.extend((0..7).map(|_| env.login(&key)));
    let v = env.refused(&new_key(), &hello(None));
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
    // the permission is read (and kept) while it operates ...
    put(
        &mut ch,
        json!({"t":"keys","id":"k0","pane":12,"keys":["esc"]}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    assert!(registry::set_permission(&env.dir, &id, Permission::View).unwrap());
    // ... and the open connection may not run any more once it changed
    put(
        &mut ch,
        json!({"t":"run","id":"r1","pane":12,"command":"echo late"}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "forbidden"));
    assert_eq!(env.fake.inputs().len(), 1, "a command ran after the change");
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

/// The next message that is not a topology refresh or a `done`.
fn next_real(ch: &mut Ch) -> Value {
    loop {
        let v = recv(ch);
        if v["t"] != "topology" && v["t"] != "done" {
            return v;
        }
    }
}

#[test]
fn closing_reaches_every_device_and_flush_waits_for_it() {
    let env = start(|_| {});
    let gw = env.gw.as_ref().unwrap();
    // nobody connected: nothing to wait for
    assert!(gw.flush(Duration::ZERO));
    let (_k1, mut one, _) = env.pair(Permission::Operate);
    let (_k2, mut two, _) = env.pair(Permission::View);
    gw.announce_closing(ClosingReason::Update, Some(60));
    assert!(gw.flush(Duration::from_secs(5)), "closing not written");
    let update = json!({"t":"closing","reason":"update","back_in_s":60});
    assert_eq!(next_real(&mut one), update);
    assert_eq!(next_real(&mut two), update);
    // a device connecting after the announcement gets it too
    let (_k3, mut three, _) = env.pair(Permission::Operate);
    assert_eq!(next_real(&mut three), update);
    // written once: a ping is answered next, not by another `closing`
    put(&mut one, json!({"t":"ping","n":7}));
    assert_eq!(next_real(&mut one), json!({"t":"pong","n":7}));
}

#[test]
fn a_closing_announced_right_before_the_stop_is_written_before_the_close() {
    let mut env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    let gw = env.gw.take().unwrap();
    gw.announce_closing(ClosingReason::Quit, None);
    // stopped at once, without waiting for the connection's next turn
    drop(gw);
    assert_eq!(next_real(&mut ch), json!({"t":"closing","reason":"quit"}));
    assert_closed(&mut ch);
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
fn move_reaches_the_source_and_is_listed_in_the_welcome() {
    let env = start(|c| c.topology_poll = Duration::from_secs(60));
    let (_k, mut ch, welcome) = env.pair(Permission::Operate);
    assert!(
        welcome["features"]
            .as_array()
            .unwrap()
            .contains(&json!("move")),
        "{welcome}"
    );
    put(
        &mut ch,
        json!({"t":"move","id":"v1","pane":12,"kind":"tab","to":0}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    put(
        &mut ch,
        json!({"t":"move","id":"v2","pane":12,"kind":"pane","to":1}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    put(
        &mut ch,
        json!({"t":"move","id":"v3","pane":99,"kind":"pane","to":1}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "no_pane"));
    put(
        &mut ch,
        json!({"t":"move","id":"v4","pane":12,"kind":"window","to":1}),
    );
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "bad_request"));
    assert_eq!(
        env.fake.0.lock().unwrap().moves,
        [
            (12, "Test Phone".to_string(), MoveKind::Tab, 0),
            (12, "Test Phone".to_string(), MoveKind::Pane, 1)
        ]
    );
}

#[test]
fn view_devices_are_forbidden_to_move() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(
        &mut ch,
        json!({"t":"move","id":"v1","pane":12,"kind":"tab","to":0}),
    );
    let v = recv_t(&mut ch, "ack");
    assert_eq!(ack(&v), (false, "forbidden"));
    assert!(env.fake.0.lock().unwrap().moves.is_empty());
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

/// A phone resuming after the gateway restarted holds the seq of the earlier
/// run. The new run's ring starts at 1 and is contiguous, so without a
/// per-run base a small `after` was answered with updates on top of the
/// phone's stale blocks instead of a snapshot.
#[test]
fn a_resume_from_an_earlier_gateway_run_gets_a_snapshot_not_a_replay() {
    let env = start(|c| c.seq_base = None);
    let fake = &env.fake;
    let (key, mut watcher, _) = env.pair(Permission::View);
    put(
        &mut watcher,
        json!({"t":"subscribe","pane":12,"after":null}),
    );
    let base = recv_t(&mut watcher, "snapshot")["seq"].as_u64().unwrap();
    // the start time in milliseconds: far above any seq an earlier run reached
    assert!(base > 1_000_000_000_000, "{base}");
    for n in 1..=5 {
        fake.edit(12, |b| b[1].output = format!("v{n}\n"));
        await_seq(&mut watcher, base + n);
    }
    // the seq the phone applied in a run that is gone
    let mut phone = env.login(&key);
    put(&mut phone, json!({"t":"subscribe","pane":12,"after":3}));
    let s = recv_t(&mut phone, "snapshot");
    assert_eq!(s["seq"], base + 5);
    assert_eq!(s["blocks"][1]["output"], "v5\n");
    // a resume from this run still replays
    let mut again = env.login(&key);
    put(
        &mut again,
        json!({"t":"subscribe","pane":12,"after":base + 3}),
    );
    assert_eq!(recv_t(&mut again, "update")["seq"], base + 4);
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

/// What a program copies goes to the phones that may operate, only while
/// they are here: nothing older reaches one that connects later.
#[test]
fn copies_reach_operating_phones_and_never_an_older_one() {
    let settle = std::time::Duration::from_millis(1200);
    let env = start(|_| {});
    let copy = |seq: u64, text: &str| {
        env.fake.0.lock().unwrap().copy = Some((seq, text.to_string()));
    };
    copy(1, "before anyone came");
    let (_k, mut phone, _) = env.pair(Permission::Operate);
    let (_v, mut viewer, _) = env.pair(Permission::View);
    std::thread::sleep(settle);
    copy(2, "seen");
    assert_eq!(recv_t(&mut phone, "clipboard")["text"], "seen");
    // the view device hears nothing of it before the answer to its ping
    put(&mut viewer, json!({"t":"ping","n":7}));
    loop {
        let m = recv(&mut viewer);
        assert_ne!(m["t"], "clipboard");
        if m["t"] == "pong" {
            break;
        }
    }
    drop((phone, viewer));
    std::thread::sleep(settle);
    copy(3, "while nobody was here");
    std::thread::sleep(settle);
    let (_k2, mut back, _) = env.pair(Permission::Operate);
    std::thread::sleep(settle);
    copy(4, "after coming back");
    assert_eq!(recv_t(&mut back, "clipboard")["text"], "after coming back");
}

fn theme(name: &str, bg: &str) -> Theme {
    let c = |s: &str| s.to_string();
    Theme {
        name: c(name),
        dark: true,
        bg: c(bg),
        bg_chrome: c("000000"),
        surface: c("1e1e1e"),
        border: c("ffffff1a"),
        fg: c("ffffff"),
        text_bright: c("ffffff"),
        text_ui: c("e6e6e6"),
        text_dim: c("9b9b9b"),
        text_muted: c("828282"),
        accent: c("19aad8"),
        success: c("1ca05a"),
        error: c("ff8272"),
        warning: c("e5a01a"),
        selection: c("76a7fa66"),
        ansi: vec![c("616161"); 16],
    }
}

/// The welcome carries the desktop's theme and says `theme`; a change on
/// the computer reaches every phone, a computer without one sends neither.
#[test]
fn the_desktops_theme_comes_with_the_welcome_and_follows_changes() {
    let env = start(|_| {});
    // an older computer: no theme, no feature
    let (_k, _old, welcome) = env.pair(Permission::Operate);
    assert!(welcome.get("theme").is_none(), "{welcome}");
    assert!(
        !welcome["features"]
            .as_array()
            .unwrap()
            .contains(&json!("theme"))
    );
    drop(_old);
    env.fake.0.lock().unwrap().theme = Some(theme("Dark Default", "050505"));
    let (_k, mut phone, welcome) = env.pair(Permission::Operate);
    let (_v, mut viewer, _) = env.pair(Permission::View);
    assert_eq!(welcome["theme"]["name"], "Dark Default");
    assert_eq!(welcome["theme"]["bg"], "050505");
    assert_eq!(welcome["theme"]["ansi"].as_array().unwrap().len(), 16);
    assert!(
        welcome["features"]
            .as_array()
            .unwrap()
            .contains(&json!("theme"))
    );
    // the theme changes on the computer: both phones hear it, whatever
    // they may do (the first look may repeat the welcome's theme)
    env.fake.0.lock().unwrap().theme = Some(theme("Dracula", "282a36"));
    for ch in [&mut phone, &mut viewer] {
        let t = loop {
            let t = recv_t(ch, "theme");
            if t["theme"]["name"] != "Dark Default" {
                break t;
            }
        };
        assert_eq!(t["theme"]["name"], "Dracula");
        assert_eq!(t["theme"]["bg"], "282a36");
    }
}

#[test]
fn paste_keeps_its_lines_and_refuses_escapes() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    let lines = "one\ntwo\r\n\tthree";
    put(
        &mut ch,
        json!({"t":"paste","id":"p1","pane":12,"text":lines}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    assert_eq!(
        env.fake.inputs(),
        [(
            12,
            "Test Phone".to_string(),
            Input::Paste(lines.to_string())
        )]
    );
    for (text, want) in [
        ("\u{1b}[201~rm".to_string(), "bad_request"),
        ("del\u{7f}".into(), "bad_request"),
        ("csi\u{9b}2J".into(), "bad_request"),
        (String::new(), "bad_request"),
        ("x".repeat(48 * 1024 + 1), "too_large"),
    ] {
        put(&mut ch, json!({"t":"paste","id":"p","pane":12,"text":text}));
        assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, want));
    }
    assert_eq!(env.fake.inputs().len(), 1);
    // past the usual message limit: a whole paste, not a closed connection
    put(
        &mut ch,
        json!({"t":"paste","id":"big","pane":12,"text":"x".repeat(48 * 1024)}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
}

#[test]
fn a_tap_reaches_the_source_as_a_click() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    put(
        &mut ch,
        json!({"t":"mouse","id":"m1","pane":12,"col":4,"row":7}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    assert_eq!(
        env.fake.inputs(),
        [(
            12,
            "Test Phone".to_string(),
            Input::Mouse { col: 4, row: 7 }
        )]
    );
    let (_k, mut view, _) = env.pair(Permission::View);
    put(
        &mut view,
        json!({"t":"mouse","id":"m2","pane":12,"col":0,"row":0}),
    );
    assert_eq!(ack(&recv_t(&mut view, "ack")), (false, "forbidden"));
}

#[test]
fn a_view_device_may_not_paste() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"paste","id":"p","pane":12,"text":"a"}));
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "forbidden"));
    assert!(env.fake.inputs().is_empty());
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
        age_ms: None,
        scrollback: None,
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
    // a capture falling behind says by how much, with no row in it; caught up: gone
    env.fake.edit_screen(12, |s| s.age_ms = Some(4000));
    assert_eq!(
        recv_screen(&mut ch),
        json!({"t":"screen","pane":12,"cols":20,"rows":4,"alt":true,"cursor":[3,1],"full":false,
               "age_ms":4000,"lines":[]})
    );
    env.fake.edit_screen(12, |s| s.age_ms = None);
    assert_eq!(
        recv_screen(&mut ch),
        json!({"t":"screen","pane":12,"cols":20,"rows":4,"alt":true,"cursor":[3,1],"full":false,
               "lines":[]})
    );
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
            age_ms: None,
            scrollback: None,
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
            age_ms: None,
            scrollback: None,
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
            age_ms: None,
            scrollback: None,
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
        age_ms: None,
        scrollback: None,
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
    // 60 changes over 3 s while the client reads nothing. The screens are
    // built first: building them on the way stretched the stall past the
    // gateway's write timeout on a loaded runner, which then (rightly)
    // dropped the client
    const CHANGES: usize = 60;
    let screens: Vec<Screen> = (1..=CHANGES).map(screen_n).collect();
    let fake = env.fake.clone();
    let writer = std::thread::spawn(move || {
        for screen in screens {
            fake.set_screen(12, screen);
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
fn prompt_chips_ride_the_topology_bounded_and_open_their_menus() {
    let env = start(|_| {});
    // the pane's chips come with the topology, an over-long text cut
    let key = new_key();
    let secret = link_secret(&env.link(Permission::View));
    let mut ch = env.handshake(&key, &hello(Some(&secret))).unwrap();
    assert_eq!(recv(&mut ch)["t"], "welcome");
    let topology = recv_t(&mut ch, "topology");
    let chips = &topology["windows"][0]["tabs"][0]["panes"][0]["chips"];
    assert_eq!(chips[0]["kind"], "docker");
    assert_eq!(chips[0]["attention"], true);
    assert_eq!(chips[0]["full"][1], json!({"text":"3/4","tone":"success"}));
    assert_eq!(
        chips[0]["short"][0]["text"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        rift_remote::MAX_CHIP_TEXT
    );
    // a pane without chips has no `chips` at all
    assert!(
        topology["windows"][0]["tabs"][0]["panes"][1]
            .get("chips")
            .is_none()
    );
    // the menus answer view devices too
    put(&mut ch, json!({"t":"chip","id":"c1","pane":12,"kind":"pr"}));
    assert_eq!(
        recv_t(&mut ch, "chip"),
        json!({"t":"chip","id":"c1","pane":12,"kind":"pr","number":42,"state":"open",
               "url":"https://github.com/me/flow/pull/42","more":false})
    );
    put(
        &mut ch,
        json!({"t":"chip","id":"c2","pane":12,"kind":"docker"}),
    );
    let v = recv_t(&mut ch, "chip");
    assert_eq!(
        v["containers"].as_array().unwrap().len(),
        rift_remote::MAX_MENU_ITEMS
    );
    assert_eq!((&v["more"], &v["project"]), (&json!(true), &json!("flow")));
    put(
        &mut ch,
        json!({"t":"chip","id":"c3","pane":12,"kind":"elapsed"}),
    );
    assert_eq!(recv_t(&mut ch, "chip")["seconds"], 120);
    // the source answered another chip's menu: not forwarded
    put(
        &mut ch,
        json!({"t":"chip","id":"c4","pane":12,"kind":"php"}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "no_pane"));
    // the folder, branch and changes chips have requests of their own
    put(
        &mut ch,
        json!({"t":"chip","id":"c5","pane":12,"kind":"dirs"}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "bad_request"));
    put(&mut ch, json!({"t":"chip","id":"c6","pane":77,"kind":"pr"}));
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "no_pane"));
}

#[test]
fn only_operate_devices_reset_the_time_on_tab() {
    let env = start(|_| {});
    let (_k, mut view, _) = env.pair(Permission::View);
    put(&mut view, json!({"t":"reset_elapsed","id":"r1","pane":12}));
    assert_eq!(ack(&recv_t(&mut view, "ack")), (false, "forbidden"));
    assert!(env.fake.0.lock().unwrap().resets.is_empty());
    let (_k, mut op, _) = env.pair(Permission::Operate);
    put(&mut op, json!({"t":"reset_elapsed","id":"r2","pane":12}));
    assert_eq!(ack(&recv_t(&mut op, "ack")), (true, ""));
    assert_eq!(
        env.fake.0.lock().unwrap().resets,
        [(12, "Test Phone".to_string())]
    );
    put(&mut op, json!({"t":"reset_elapsed","id":"r3","pane":77}));
    assert_eq!(ack(&recv_t(&mut op, "ack")), (false, "no_pane"));
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

// --- history ----------------------------------------------------------------

impl Fake {
    fn set_history(&self, pane: u64, epoch: u64, first: u64, lines: Vec<String>) {
        self.0
            .lock()
            .unwrap()
            .histories
            .insert(pane, (epoch, first, lines));
    }
}

/// The texts of a `history` answer's lines.
fn history_texts(v: &Value) -> Vec<String> {
    v["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l[0][0].as_str().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn history_pages_numbered_lines_keeping_the_end_of_a_long_range() {
    let env = start(|_| {});
    env.fake.set_history(
        12,
        7,
        100,
        (100..1000).map(|n| format!("line {n}")).collect(),
    );
    let (_k, mut ch, welcome) = env.pair(Permission::View);
    let features = welcome["features"].as_array().unwrap();
    assert!(features.contains(&json!("history")), "{features:?}");
    put(
        &mut ch,
        json!({"t":"history","id":"h1","pane":12,"epoch":7,"from":118,"to":121}),
    );
    let v = recv_t(&mut ch, "history");
    assert_eq!(
        v,
        json!({"t":"history","id":"h1","pane":12,"epoch":7,"first":100,"from":118,
               "lines":[[["line 118",null,null,0]],[["line 119",null,null,0]],[["line 120",null,null,0]]],
               "rules":[[2,"3fb950"]]})
    );
    // longer than a page: the lines nearest `to`
    put(
        &mut ch,
        json!({"t":"history","id":"h2","pane":12,"epoch":7,"from":0,"to":900}),
    );
    let v = recv_t(&mut ch, "history");
    assert_eq!(v["from"], 700);
    let texts = history_texts(&v);
    assert_eq!(texts.len(), 200);
    assert_eq!(texts.last().unwrap(), "line 899");
    // lines gone from the scrollback: from the first held one
    put(
        &mut ch,
        json!({"t":"history","id":"h3","pane":12,"epoch":7,"from":90,"to":102}),
    );
    let v = recv_t(&mut ch, "history");
    assert_eq!((&v["from"], history_texts(&v).len()), (&json!(100), 2));
    // another numbering: no lines, the current epoch
    put(
        &mut ch,
        json!({"t":"history","id":"h4","pane":12,"epoch":6,"from":200,"to":210}),
    );
    let v = recv_t(&mut ch, "history");
    assert_eq!(
        (&v["epoch"], &v["first"], history_texts(&v).len()),
        (&json!(7), &json!(100), 0)
    );
    // an empty range, an unknown pane
    for (msg, want) in [
        (
            json!({"t":"history","id":"e","pane":12,"epoch":7,"from":300,"to":300}),
            "bad_request",
        ),
        (
            json!({"t":"history","id":"p","pane":77,"epoch":7,"from":0,"to":5}),
            "no_pane",
        ),
    ] {
        put(&mut ch, msg);
        assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, want));
    }
}

#[test]
fn history_over_a_gateway_frame_drops_lines_from_its_start() {
    let env = start(|_| {});
    // 200 lines of 10 000 characters: two megabytes
    env.fake.set_history(
        12,
        1,
        0,
        (0..200)
            .map(|n| format!("{n:05}{}", "x".repeat(9_995)))
            .collect(),
    );
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(
        &mut ch,
        json!({"t":"history","id":"h","pane":12,"epoch":1,"from":0,"to":200}),
    );
    let v = recv_t(&mut ch, "history");
    let texts = history_texts(&v);
    assert!(v.to_string().len() <= rift_remote::MAX_GATEWAY_MESSAGE);
    assert!((50..200).contains(&texts.len()), "{}", texts.len());
    assert_eq!(v["from"], 200 - texts.len());
    assert!(texts.last().unwrap().starts_with("00199"));
    // the rules follow their lines: line 190 carries one
    let rules = v["rules"].as_array().unwrap();
    assert!(
        rules.contains(&json!([texts.len() - 10, "3fb950"])),
        "{rules:?}"
    );
}

#[test]
fn history_is_rate_limited_per_connection() {
    let env = start(|_| {});
    env.fake.set_history(12, 1, 0, vec!["a".into()]);
    let (_k, mut ch, _) = env.pair(Permission::View);
    let t = Instant::now();
    let mut got = Vec::new();
    for i in 0..11 {
        put(
            &mut ch,
            json!({"t":"history","id":format!("h{i}"),"pane":12,"epoch":1,"from":0,"to":1}),
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
    assert_eq!(got[..10], vec!["history".to_string(); 10]);
    assert_eq!(got[10], "ackrate");
}

#[test]
fn a_change_of_scrollback_alone_sends_a_screen() {
    let env = start(|_| {});
    let at = |top| Scrollback {
        epoch: 3,
        first: 0,
        top,
    };
    let mut s = screen(&["a", "b"], false, None);
    s.scrollback = Some(at(40));
    env.fake.set_screen(12, s);
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"on"}));
    let v = recv_screen(&mut ch);
    assert_eq!(v["scrollback"], json!({"epoch":3,"first":0,"top":40}));
    // same rows, the lines numbered anew: a frame with no row
    env.fake.edit_screen(12, |s| s.scrollback = Some(at(41)));
    let v = recv_screen(&mut ch);
    assert_eq!(
        (&v["full"], &v["lines"], &v["scrollback"]["top"]),
        (&json!(false), &json!([]), &json!(41))
    );
}

// --- scroll -----------------------------------------------------------------

#[test]
fn scroll_reaches_the_source_clamped() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    for (rid, lines) in [("s1", 3), ("s2", -500), ("s3", 0)] {
        put(
            &mut ch,
            json!({"t":"scroll","id":rid,"pane":12,"lines":lines}),
        );
        let v = recv_t(&mut ch, "ack");
        assert_eq!((ack(&v), v["id"].as_str()), ((true, ""), Some(rid)));
    }
    // 0 moves nothing and never reaches the source
    assert_eq!(
        env.fake.0.lock().unwrap().scrolls,
        [(12, 3, true), (12, -rift_remote::MAX_SCROLL_LINES, true)]
    );
    put(&mut ch, json!({"t":"scroll","id":"s4","pane":99,"lines":1}));
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "no_pane"));
}

#[test]
fn view_devices_scroll_a_view_but_never_send_the_wheel_to_a_program() {
    let env = start(|_| {});
    env.fake.set(13, vec![block(3, "vim\n")]);
    env.fake.0.lock().unwrap().tracking.push(13);
    let (_k, mut ch, _) = env.pair(Permission::View);
    put(
        &mut ch,
        json!({"t":"scroll","id":"s1","pane":12,"lines":-4}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    put(&mut ch, json!({"t":"scroll","id":"s2","pane":13,"lines":2}));
    let v = recv_t(&mut ch, "ack");
    assert_eq!(
        (ack(&v), v["id"].as_str()),
        ((false, "forbidden"), Some("s2"))
    );
    // the source was told the device may not operate
    assert_eq!(env.fake.0.lock().unwrap().scrolls, [(12, -4, false)]);
    // an operate device reaches the program
    let (_k2, mut op, _) = env.pair(Permission::Operate);
    put(&mut op, json!({"t":"scroll","id":"s3","pane":13,"lines":2}));
    assert_eq!(ack(&recv_t(&mut op, "ack")), (true, ""));
    assert_eq!(env.fake.0.lock().unwrap().scrolls[1], (13, 2, true));
}

#[test]
fn scroll_has_its_own_rate_apart_from_keys() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    let mut got = Vec::new();
    for i in 0..=rift_remote::SCROLL_RATE {
        put(
            &mut ch,
            json!({"t":"scroll","id":format!("s{i}"),"pane":12,"lines":1}),
        );
        got.push(ack(&recv_t(&mut ch, "ack")).1.to_string());
    }
    assert_eq!(got[..rift_remote::SCROLL_RATE], vec![String::new(); 40]);
    assert_eq!(got[rift_remote::SCROLL_RATE], "rate");
    // keys still go: scrolling never spends typing's budget
    put(
        &mut ch,
        json!({"t":"keys","id":"k1","pane":12,"keys":["ctrl+c"]}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
}

// --- upload -----------------------------------------------------------------

/// A PNG-signed blob of `len` bytes.
fn png(len: usize) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    b.extend((0..len - 8).map(|i| (i % 251) as u8));
    b
}

/// `bytes` as `upload` chunks of at most `MAX_UPLOAD_CHUNK` bytes.
fn upload_msgs(rid: &str, pane: u64, bytes: &[u8]) -> Vec<Value> {
    use base64::Engine as _;
    bytes
        .chunks(rift_remote::MAX_UPLOAD_CHUNK)
        .enumerate()
        .map(|(i, c)| {
            json!({"t":"upload","id":rid,"pane":pane,"size":bytes.len(),
                   "offset":i * rift_remote::MAX_UPLOAD_CHUNK,
                   "data":base64::engine::general_purpose::STANDARD.encode(c)})
        })
        .collect()
}

#[test]
fn an_upload_in_chunks_reaches_the_source_once_with_one_ack() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    let image = png(100_000);
    let msgs = upload_msgs("u1", 12, &image);
    assert_eq!(msgs.len(), 3);
    for m in &msgs {
        // every chunk fits the upload limit, over the ordinary one
        let n = m.to_string().len();
        assert!(n <= rift_remote::MAX_UPLOAD_MESSAGE, "{n}");
    }
    assert!(msgs[0].to_string().len() > rift_remote::MAX_CLIENT_MESSAGE);
    for m in &msgs[..2] {
        put(&mut ch, m.clone());
    }
    // nothing answers a chunk in between (a pong comes first)
    put(&mut ch, json!({"t":"ping","n":5}));
    assert_eq!(recv_t(&mut ch, "pong")["n"], 5);
    assert!(env.fake.0.lock().unwrap().uploads.is_empty());
    put(&mut ch, msgs[2].clone());
    assert_eq!(
        recv_t(&mut ch, "ack"),
        json!({"t":"ack","id":"u1","ok":true})
    );
    let uploads = env.fake.0.lock().unwrap().uploads.clone();
    assert_eq!(uploads.len(), 1);
    assert_eq!(
        (uploads[0].0, uploads[0].1.as_str(), uploads[0].2),
        (12, "Test Phone", ImageKind::Png)
    );
    assert_eq!(uploads[0].3, image);
    // a JPEG in one chunk (past the rate of one upload a second)
    std::thread::sleep(Duration::from_millis(1050));
    let jpeg = [b"\xff\xd8\xff\xe0".as_slice(), &[7u8; 300]].concat();
    put(&mut ch, upload_msgs("u2", 12, &jpeg)[0].clone());
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    assert_eq!(env.fake.0.lock().unwrap().uploads[1].2, ImageKind::Jpeg);
}

#[test]
fn view_devices_are_forbidden_to_upload_and_never_send_an_uploads_size() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::View);
    // small chunks (within the ordinary message limit)
    let image = png(5_000);
    use base64::Engine as _;
    let msgs: Vec<Value> = image
        .chunks(1_500)
        .enumerate()
        .map(|(i, c)| {
            json!({"t":"upload","id":"u1","pane":12,"size":image.len(),"offset":i * 1_500,
                   "data":base64::engine::general_purpose::STANDARD.encode(c)})
        })
        .collect();
    for m in &msgs {
        put(&mut ch, m.clone());
    }
    let v = recv_t(&mut ch, "ack");
    assert_eq!(
        (ack(&v), v["id"].as_str()),
        ((false, "forbidden"), Some("u1"))
    );
    // one answer for the whole upload: the next message is the pong
    put(&mut ch, json!({"t":"ping","n":6}));
    assert_eq!(recv_t(&mut ch, "pong")["n"], 6);
    assert!(env.fake.0.lock().unwrap().uploads.is_empty());
    // an upload-sized chunk passes a view device's frame limit: closed
    // before anything decrypts or parses it
    let big = upload_msgs("u2", 12, &png(60_000))[0].clone();
    assert!(ch.send(&big.to_string()));
    assert_closed(&mut ch);
}

#[test]
fn a_frame_before_authentication_is_held_to_the_ordinary_limit() {
    let env = start(|_| {});
    let mut w = ws(env.addr);
    // a handshake message never comes near MAX_CLIENT_MESSAGE
    w.send(tungstenite::Message::binary(vec![
        0u8;
        rift_remote::MAX_CLIENT_MESSAGE
            + 512
    ]))
    .unwrap();
    let t = Instant::now();
    loop {
        match w.read() {
            Ok(tungstenite::Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) => break,
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                assert!(t.elapsed() < Duration::from_secs(5), "still open");
            }
            Err(_) => break,
            Ok(m) => panic!("answered a too-large handshake frame: {m:?}"),
        }
    }
}

/// `bytes` as `upload` chunks carrying the extension `ext`.
fn file_msgs(rid: &str, pane: u64, ext: &str, bytes: &[u8]) -> Vec<Value> {
    upload_msgs(rid, pane, bytes)
        .into_iter()
        .map(|mut m| {
            m["ext"] = json!(ext);
            m
        })
        .collect()
}

#[test]
fn a_file_with_an_extension_reaches_the_source_once_as_it_is() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    // no image signature, several chunks: any bytes go
    let bytes: Vec<u8> = (0..100_000).map(|i| (i % 253) as u8).collect();
    let msgs = file_msgs("f1", 12, "txt", &bytes);
    assert!(msgs.len() > 1);
    for m in &msgs {
        put(&mut ch, m.clone());
    }
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    put(&mut ch, json!({"t":"ping","n":1}));
    assert_eq!(recv_t(&mut ch, "pong")["n"], 1);
    let st = env.fake.0.lock().unwrap();
    assert!(st.uploads.is_empty(), "a file is not an image");
    assert_eq!(st.files.len(), 1, "once");
    assert_eq!((st.files[0].0, st.files[0].2.as_str()), (12, "txt"));
    assert_eq!(st.files[0].3, bytes);
}

#[test]
fn a_bad_extension_is_refused_and_a_gif_without_one_stays_refused() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    for bad in ["PNG", "../x", "abcdefghi", "", "a b", "a.b", "é"] {
        put(&mut ch, file_msgs("f", 12, bad, b"hello")[0].clone());
        let v = recv_t(&mut ch, "ack");
        assert_eq!(ack(&v), (false, "bad_request"), "{bad}: {v}");
        assert!(v["message"].as_str().unwrap().starts_with("ext "), "{v}");
        std::thread::sleep(Duration::from_millis(1050));
    }
    // the longest extension goes, and the same bytes without one do not
    put(
        &mut ch,
        file_msgs("ok", 12, "abcdefg8", b"GIF89a-whatever")[0].clone(),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    std::thread::sleep(Duration::from_millis(1050));
    put(
        &mut ch,
        upload_msgs("gif", 12, b"GIF89a-whatever")[0].clone(),
    );
    let v = recv_t(&mut ch, "ack");
    assert_eq!(ack(&v), (false, "bad_request"), "{v}");
    assert!(
        v["message"].as_str().unwrap().contains("PNG or JPEG"),
        "{v}"
    );
    let st = env.fake.0.lock().unwrap();
    assert!(st.uploads.is_empty());
    assert_eq!(st.files.len(), 1);
    assert_eq!(st.files[0].2, "abcdefg8");
}

#[test]
fn a_file_keeps_its_extension_its_cap_and_its_own_refusals() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    // a later chunk naming another extension (or none) is out of order
    let bytes = vec![7u8; 100_000];
    let msgs = file_msgs("f1", 12, "txt", &bytes);
    put(&mut ch, msgs[0].clone());
    let mut other = msgs[1].clone();
    other["ext"] = json!("sh");
    put(&mut ch, other);
    let v = recv_t(&mut ch, "ack");
    assert_eq!(ack(&v), (false, "bad_request"), "{v}");
    assert!(
        v["message"].as_str().unwrap().contains("out of order"),
        "{v}"
    );
    std::thread::sleep(Duration::from_millis(1050));
    // over the cap, whatever it says it is
    put(
        &mut ch,
        json!({"t":"upload","id":"f2","pane":12,"size":rift_remote::MAX_UPLOAD_BYTES + 1,
               "offset":0,"data":"AAAA","ext":"txt"}),
    );
    let v = recv_t(&mut ch, "ack");
    assert_eq!(ack(&v), (false, "too_large"), "{v}");
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .contains("the file is larger"),
        "{v}"
    );
    std::thread::sleep(Duration::from_millis(1050));
    // at a prompt: refused at the first chunk, naming a file
    env.fake.0.lock().unwrap().ready_error = Some(InputError::Busy);
    put(&mut ch, msgs[0].clone());
    let v = recv_t(&mut ch, "ack");
    assert_eq!(ack(&v), (false, "busy"), "{v}");
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .contains("at its prompt; a file goes"),
        "{v}"
    );
    assert!(env.fake.0.lock().unwrap().files.is_empty());
}

#[test]
fn a_view_device_may_not_upload_a_file() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::View);
    // the first chunk of more, within a view device's 4 KiB frames: refused
    // at once, not when the last chunk would arrive
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD.encode([7u8; 1_500]);
    put(
        &mut ch,
        json!({"t":"upload","id":"f1","pane":12,"size":3_000,"offset":0,"data":data,"ext":"txt"}),
    );
    let v = recv_t(&mut ch, "ack");
    assert_eq!(ack(&v), (false, "forbidden"), "{v}");
    assert!(env.fake.0.lock().unwrap().files.is_empty());
}

#[test]
fn an_upload_to_a_pane_at_its_prompt_is_refused_at_its_first_chunk() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    env.fake.0.lock().unwrap().ready_error = Some(InputError::Busy);
    let msgs = upload_msgs("u1", 12, &png(100_000));
    put(&mut ch, msgs[0].clone());
    let v = recv_t(&mut ch, "ack");
    assert_eq!(ack(&v), (false, "busy"), "{v}");
    assert!(
        v["message"]
            .as_str()
            .unwrap_or_default()
            .contains("at its prompt"),
        "{v}"
    );
    // the rest goes unanswered and never reaches the source
    for m in &msgs[1..] {
        put(&mut ch, m.clone());
    }
    put(&mut ch, json!({"t":"ping","n":3}));
    assert_eq!(recv_t(&mut ch, "pong")["n"], 3);
    assert!(env.fake.0.lock().unwrap().uploads.is_empty());
}

#[test]
fn an_upload_past_its_deadline_is_dropped_and_answered_busy() {
    let env = start(|c| c.upload_deadline = Duration::from_millis(400));
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    let msgs = upload_msgs("u1", 12, &png(100_000));
    put(&mut ch, msgs[0].clone());
    // nothing more arrives: the gateway answers once the deadline passes
    let t = Instant::now();
    let v = recv_t(&mut ch, "ack");
    assert!(
        t.elapsed() >= Duration::from_millis(300),
        "{:?}",
        t.elapsed()
    );
    assert_eq!((ack(&v), v["id"].as_str()), ((false, "busy"), Some("u1")));
    // its late chunks are dropped unanswered
    for m in &msgs[1..] {
        put(&mut ch, m.clone());
    }
    put(&mut ch, json!({"t":"ping","n":4}));
    assert_eq!(recv_t(&mut ch, "pong")["n"], 4);
    assert!(env.fake.0.lock().unwrap().uploads.is_empty());
}

#[test]
fn at_most_two_images_arrive_at_once_across_the_gateway() {
    let env = start(|_| {});
    let image = png(100_000);
    let mut phones: Vec<Ch> = (0..3).map(|_| env.pair(Permission::Operate).1).collect();
    // two phones start an image each and stop part-way
    for (i, ch) in phones.iter_mut().take(2).enumerate() {
        put(ch, upload_msgs(&format!("u{i}"), 12, &image)[0].clone());
        put(ch, json!({"t":"ping","n":i}));
        assert_eq!(recv_t(ch, "pong")["n"], i);
    }
    // a third is busy meanwhile
    let third = &mut phones[2];
    put(third, upload_msgs("u2", 12, &image)[0].clone());
    assert_eq!(ack(&recv_t(third, "ack")), (false, "busy"));
    // one finishes: its place is free again
    for m in &upload_msgs("u0", 12, &image)[1..] {
        put(&mut phones[0], m.clone());
    }
    assert_eq!(ack(&recv_t(&mut phones[0], "ack")), (true, ""));
    std::thread::sleep(Duration::from_millis(1050));
    let third = &mut phones[2];
    for m in upload_msgs("u3", 12, &image) {
        put(third, m);
    }
    assert_eq!(ack(&recv_t(third, "ack")), (true, ""));
}

#[test]
fn uploads_over_the_cap_not_images_or_out_of_order_are_refused() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    use base64::Engine as _;
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let cases = [
        // claims more than the cap
        (
            json!({"t":"upload","id":"a","pane":12,"size":rift_remote::MAX_UPLOAD_BYTES + 1,
                   "offset":0,"data":b64(&png(100))}),
            "too_large",
        ),
        // a GIF, a text file: the signature decides, never a name
        (
            json!({"t":"upload","id":"b","pane":12,"size":100,"offset":0,
                   "data":b64(&[b"GIF89a".as_slice(), &[0u8; 94]].concat())}),
            "bad_request",
        ),
        (
            json!({"t":"upload","id":"c","pane":12,"size":12,"offset":0,
                   "data":b64(b"#!/bin/sh\nrm")}),
            "bad_request",
        ),
        // a chunk with no start
        (
            json!({"t":"upload","id":"d","pane":12,"size":200,"offset":100,
                   "data":b64(&[0u8; 100])}),
            "bad_request",
        ),
        // not base64
        (
            json!({"t":"upload","id":"e","pane":12,"size":100,"offset":0,"data":"%%%%"}),
            "bad_request",
        ),
        // more bytes than the size
        (
            json!({"t":"upload","id":"f","pane":12,"size":20,"offset":0,"data":b64(&png(40))}),
            "bad_request",
        ),
        (
            json!({"t":"upload","id":"g","pane":12,"size":0,"offset":0,"data":""}),
            "bad_request",
        ),
    ];
    for (m, want) in cases {
        put(&mut ch, m.clone());
        let v = recv_t(&mut ch, "ack");
        assert_eq!(ack(&v), (false, want), "{m}");
        // past the rate of one upload a second
        std::thread::sleep(Duration::from_millis(1050));
    }
    // a gap after a good start: refused, its later chunks dropped
    let image = png(140_000);
    let msgs = upload_msgs("h", 12, &image);
    put(&mut ch, msgs[0].clone());
    put(&mut ch, msgs[2].clone());
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "bad_request"));
    put(&mut ch, msgs[3].clone());
    put(&mut ch, json!({"t":"ping","n":9}));
    assert_eq!(recv_t(&mut ch, "pong")["n"], 9);
    assert!(env.fake.0.lock().unwrap().uploads.is_empty());
}

#[test]
fn an_upload_to_a_pane_at_its_prompt_is_busy_and_starts_are_rate_limited() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    env.fake.fail_input(Some(InputError::Busy));
    put(&mut ch, upload_msgs("u1", 12, &png(500))[0].clone());
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "busy"));
    env.fake.fail_input(None);
    // a second start within the second
    put(&mut ch, upload_msgs("u2", 12, &png(500))[0].clone());
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (false, "rate"));
}

#[test]
fn an_upload_chunk_over_its_limit_and_a_large_non_upload_close_the_connection() {
    let env = start(|_| {});
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    // an `upload` padded past MAX_UPLOAD_MESSAGE
    let big = json!({"t":"upload","id":"u","pane":12,"size":10,"offset":0,
                     "data":"A".repeat(rift_remote::MAX_UPLOAD_MESSAGE)});
    assert!(ch.send(&big.to_string()));
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "too_large"));
    assert_closed(&mut ch);
    // any other type past MAX_CLIENT_MESSAGE, though under the upload limit
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    let text = json!({"t":"text","id":"x","pane":12,"text":"y".repeat(10_000)});
    assert!(ch.send(&text.to_string()));
    assert_eq!(code(&recv_t(&mut ch, "error")), ("error", "too_large"));
    assert_closed(&mut ch);
    assert!(env.fake.inputs().is_empty());
}

#[test]
fn a_scroll_brings_the_redrawn_screen_before_the_next_poll() {
    // the regular poll is far off: only the kick can deliver in time
    let env = start(|c| {
        c.pane_poll = Duration::from_secs(2);
        c.devices_poll = Duration::from_secs(2);
    });
    env.fake.set_screen(12, screen(&["less", "1"], true, None));
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"auto"}));
    recv_screen(&mut ch);
    // a redraw nobody asked for waits for the poll
    env.fake
        .edit_screen(12, |s| s.lines[1] = vec![Run::plain("2")]);
    assert!(screens_in(&drain(&mut ch, 300)).is_empty());
    // a scroll reads the screen at once, and again while the program redraws
    let at = Instant::now();
    put(&mut ch, json!({"t":"scroll","id":"s","pane":12,"lines":-3}));
    let first = recv_screen(&mut ch);
    assert_eq!(first["lines"][0][0], 1, "{first}");
    env.fake
        .edit_screen(12, |s| s.lines[1] = vec![Run::plain("5")]);
    let second = recv_screen(&mut ch);
    assert!(
        at.elapsed() < Duration::from_millis(400),
        "{:?}",
        at.elapsed()
    );
    assert_eq!(second["lines"][0][1][0][0], "5", "{second}");
}

#[test]
fn output_after_typing_arrives_before_the_next_poll() {
    // a program on the primary screen (a shell's echo, an agent redrawing
    // in place): the kick reads the pane's blocks too, not only a screen
    let env = start(|c| {
        c.pane_poll = Duration::from_secs(2);
        c.devices_poll = Duration::from_secs(2);
    });
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    // output nobody typed for waits for the poll
    env.fake.edit(12, |b| b[1].output = "two, more\n".into());
    assert!(
        drain(&mut ch, 300).iter().all(|m| m["t"] != "update"),
        "an update before the poll"
    );
    let at = Instant::now();
    put(&mut ch, json!({"t":"text","id":"x","pane":12,"text":"y"}));
    let update = |ch: &mut Ch| loop {
        let v = recv(ch);
        if v["t"] == "update" {
            return v;
        }
        assert!(
            ["ack", "topology", "done"].contains(&v["t"].as_str().unwrap_or("")),
            "{v}"
        );
    };
    let first = update(&mut ch);
    assert_eq!(first["blocks"][0]["output"], "two, more\n", "{first}");
    // and again while the program answers
    env.fake.edit(12, |b| b[1].output = "two, more, y\n".into());
    let second = update(&mut ch);
    assert!(
        at.elapsed() < Duration::from_millis(400),
        "{:?}",
        at.elapsed()
    );
    assert_eq!(second["blocks"][0]["output"], "two, more, y\n", "{second}");
}

#[test]
fn a_redraw_after_a_scroll_goes_out_within_a_screen_poll() {
    // a frame ready while the client loop waits on its socket goes out at once (HOT_READ_SLICE),
    // not after the rest of its read. The read is widened to 500 ms so a loaded machine (the
    // gates run beside a docker build) cannot close the gap between the two
    let env = start(|c| {
        c.pane_poll = Duration::from_secs(2);
        c.devices_poll = Duration::from_secs(2);
        c.read_slice = Duration::from_millis(500);
    });
    env.fake.set_screen(12, screen(&["less", "1"], true, None));
    let (_k, mut ch, _) = env.pair(Permission::Operate);
    put(&mut ch, json!({"t":"subscribe","pane":12,"after":null}));
    recv_t(&mut ch, "snapshot");
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"auto"}));
    recv_screen(&mut ch);
    let mut best = Duration::MAX;
    for round in 0..5 {
        // the scroll moved the program: its frame comes at once
        env.fake
            .edit_screen(12, |s| s.lines[1] = vec![Run::plain(format!("s{round}"))]);
        put(
            &mut ch,
            json!({"t":"scroll","id":format!("s{round}"),"pane":12,"lines":-1}),
        );
        recv_screen(&mut ch);
        // the program redraws once the scroll's frame went: nothing else wakes the loop
        let at = Instant::now();
        env.fake
            .edit_screen(12, |s| s.lines[1] = vec![Run::plain(format!("r{round}"))]);
        recv_screen(&mut ch);
        best = best.min(at.elapsed());
    }
    assert!(best < Duration::from_millis(250), "{best:?}");
}

/// Lines of a document whose lengths vary, as real text does.
fn doc_lines(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| format!("line {i} {}", "x".repeat(i % 9 + 4)))
        .collect()
}

/// The `page` answers to one request, until one carries all its rows.
/// The `page` messages that answer a job. A job here runs until it has every
/// row it was asked for (`start` sets a 30 s job time), and under the test
/// gate's load (background QoS, 32 threads) one took 13.8 s: the wait is the
/// job time, not [`recv`]'s 10 s.
fn pages(ch: &mut Ch) -> Vec<Value> {
    let mut out = Vec::new();
    loop {
        let p = loop {
            let v = recv_within(ch, Duration::from_secs(30));
            if v["t"] == "page" {
                break v;
            }
        };
        let last = p["from"].as_i64().unwrap() + p["lines"].as_array().unwrap().len() as i64;
        let done =
            last >= p["bottom"].as_i64().unwrap() || p["lines"].as_array().unwrap().is_empty();
        out.push(p);
        if done {
            return out;
        }
    }
}

fn page_text(p: &Value) -> Vec<String> {
    p["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_array()
                .unwrap()
                .iter()
                .map(|run| run[0].as_str().unwrap().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

#[test]
fn page_ahead_stitches_a_full_screen_document_and_page_seek_moves_the_program() {
    let env = start(|_| {});
    let doc = doc_lines(400);
    env.fake
        .0
        .lock()
        .unwrap()
        .docs
        .insert(12, (doc.clone(), 200));
    let (_, mut ch, welcome) = env.pair(Permission::Operate);
    assert!(
        welcome["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "stitch")
    );

    // up 40 rows from line 200: a new page (epoch 0)
    put(
        &mut ch,
        json!({"t":"page_ahead","id":"g1","pane":12,"epoch":0,"dir":"up","rows":40}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    let got = pages(&mut ch);
    let p = &got[0];
    assert_eq!(p["state"], "ok");
    // the document rows only: not the header, the status row or the panel
    assert_eq!(p["area"], json!([1, 13, 0, 30]));
    let epoch = p["epoch"].as_u64().unwrap();
    assert!(epoch > 0);
    let top = p["top"].as_i64().unwrap();
    assert!(top <= -40, "{p}");
    let text: Vec<String> = got.iter().flat_map(page_text).collect();
    let first = (200 + top) as usize;
    assert_eq!(text, doc[first..212].to_vec());
    // the program shows the page's first row
    assert_eq!(p["at"].as_i64(), Some(top));
    assert_eq!(env.fake.0.lock().unwrap().docs[&12].1, first as i64);

    // down past the start: the same page grows at its bottom, only the new rows sent
    put(
        &mut ch,
        json!({"t":"page_ahead","id":"g2","pane":12,"epoch":epoch,"dir":"down","rows":60}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    let more = pages(&mut ch);
    assert_eq!(more[0]["epoch"].as_u64(), Some(epoch));
    let from = more[0]["from"].as_i64().unwrap();
    assert_eq!(from, 12, "{}", more[0]);
    let added: Vec<String> = more.iter().flat_map(page_text).collect();
    let bottom = more[0]["bottom"].as_i64().unwrap();
    assert!(bottom >= 12 + 60);
    assert_eq!(added, doc[212..(200 + bottom) as usize].to_vec());

    // where the phone stopped: the program goes there
    put(
        &mut ch,
        json!({"t":"page_seek","id":"g3","pane":12,"epoch":epoch,"at":-10}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    let seek = recv_t(&mut ch, "page");
    assert_eq!(seek["at"].as_i64(), Some(-10));
    assert!(seek["lines"].as_array().unwrap().is_empty());
    assert_eq!(env.fake.0.lock().unwrap().docs[&12].1, 190);
}

#[test]
fn page_ahead_reports_the_documents_start_and_a_screen_that_does_not_scroll() {
    let env = start(|_| {});
    let doc = doc_lines(60);
    env.fake
        .0
        .lock()
        .unwrap()
        .docs
        .insert(12, (doc.clone(), 20));
    let (_, mut ch, _) = env.pair(Permission::Operate);
    put(
        &mut ch,
        json!({"t":"page_ahead","id":"g1","pane":12,"epoch":0,"dir":"up","rows":200}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    let got = pages(&mut ch);
    assert_eq!(got[0]["ends"], json!([true, false]));
    let text: Vec<String> = got.iter().flat_map(page_text).collect();
    assert_eq!(text, doc[0..32].to_vec());

    // a full-screen program that does not move under the wheel (htop): no page
    env.fake.0.lock().unwrap().docs.remove(&12);
    let mut s = doc_screen(&doc, 0, 0);
    s.alt = true;
    env.fake.0.lock().unwrap().screens.insert(12, s);
    put(
        &mut ch,
        json!({"t":"page_ahead","id":"g2","pane":12,"epoch":0,"dir":"up","rows":40}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    let flat = recv_t(&mut ch, "page");
    assert_eq!(flat["state"], "flat");
}

#[test]
fn page_requests_are_refused_to_a_view_device_and_one_at_a_time() {
    let env = start(|_| {});
    env.fake
        .0
        .lock()
        .unwrap()
        .docs
        .insert(12, (doc_lines(400), 200));
    let (_, mut view, _) = env.pair(Permission::View);
    put(
        &mut view,
        json!({"t":"page_ahead","id":"g1","pane":12,"epoch":0,"dir":"up","rows":40}),
    );
    let a = recv_t(&mut view, "ack");
    assert_eq!(ack(&a), (false, "forbidden"));
    let (_, mut ch, _) = env.pair(Permission::Operate);
    put(
        &mut ch,
        json!({"t":"page_ahead","id":"g2","pane":12,"epoch":0,"dir":"up","rows":200}),
    );
    put(
        &mut ch,
        json!({"t":"page_ahead","id":"g3","pane":12,"epoch":0,"dir":"down","rows":40}),
    );
    let first = recv_t(&mut ch, "ack");
    let second = recv_t(&mut ch, "ack");
    assert_eq!(ack(&first), (true, ""));
    assert_eq!(ack(&second), (false, "busy"));
}

#[test]
fn screens_wait_while_the_stitcher_scrolls_the_program() {
    let env = start(|_| {});
    env.fake
        .0
        .lock()
        .unwrap()
        .docs
        .insert(12, (doc_lines(400), 200));
    let (_, mut ch, _) = env.pair(Permission::Operate);
    put(&mut ch, json!({"t":"subscribe","pane":12}));
    put(&mut ch, json!({"t":"screen","pane":12,"mode":"auto"}));
    // the screen as it is, before any page
    loop {
        if recv(&mut ch)["t"] == "screen" {
            break;
        }
    }
    put(
        &mut ch,
        json!({"t":"page_ahead","id":"g1","pane":12,"epoch":0,"dir":"up","rows":60}),
    );
    // from the ack to the page, nothing of the program scrolling reaches the phone
    let mut acked = false;
    loop {
        let v = recv(&mut ch);
        match v["t"].as_str().unwrap() {
            "ack" if v["id"] == "g1" => acked = true,
            "page" => break,
            "screen" => assert!(!acked, "a screen while stitching: {v}"),
            _ => {}
        }
    }
    // then the screens come again, showing where the program is now
    loop {
        let v = recv(&mut ch);
        if v["t"] == "screen" {
            break;
        }
    }
}

#[test]
fn a_program_that_speeds_up_under_the_wheel_still_stitches_whole() {
    let env = start(|_| {});
    let doc = doc_lines(400);
    {
        let mut st = env.fake.0.lock().unwrap();
        st.docs.insert(12, (doc.clone(), 300));
        st.accelerate = true;
    }
    let (_, mut ch, _) = env.pair(Permission::Operate);
    put(
        &mut ch,
        json!({"t":"page_ahead","id":"g1","pane":12,"epoch":0,"dir":"up","rows":60}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    let got = pages(&mut ch);
    assert_eq!(got[0]["state"], "ok", "{}", got[0]);
    let top = got[0]["top"].as_i64().unwrap();
    assert!(top <= -60, "{}", got[0]);
    let text: Vec<String> = got.iter().flat_map(page_text).collect();
    // every line from where it reached down to the screen it started from, none missing
    assert_eq!(text, doc[(300 + top) as usize..312].to_vec());
}

#[test]
fn a_screen_read_that_fails_now_and_then_does_not_stop_the_page() {
    let env = start(|_| {});
    let doc = doc_lines(400);
    {
        let mut st = env.fake.0.lock().unwrap();
        st.docs.insert(12, (doc.clone(), 300));
        st.flaky_reads = 3;
    }
    let (_, mut ch, _) = env.pair(Permission::Operate);
    put(
        &mut ch,
        json!({"t":"page_ahead","id":"g1","pane":12,"epoch":0,"dir":"up","rows":60}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    let got = pages(&mut ch);
    assert_eq!(got[0]["state"], "ok", "{}", got[0]);
    let top = got[0]["top"].as_i64().unwrap();
    assert!(top <= -60, "{}", got[0]);
    let text: Vec<String> = got.iter().flat_map(page_text).collect();
    assert_eq!(text, doc[(300 + top) as usize..312].to_vec());
}

#[test]
fn a_program_that_jumps_far_once_still_stitches_whole() {
    let env = start(|_| {});
    let doc = doc_lines(400);
    {
        let mut st = env.fake.0.lock().unwrap();
        st.docs.insert(12, (doc.clone(), 300));
        // past line 285 the view lands 60 lines further up: no overlap with the page
        st.jump = Some((285, 60));
    }
    let (_, mut ch, _) = env.pair(Permission::Operate);
    put(
        &mut ch,
        json!({"t":"page_ahead","id":"g1","pane":12,"epoch":0,"dir":"up","rows":40}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    let got = pages(&mut ch);
    assert_eq!(got[0]["state"], "ok", "{}", got[0]);
    let top = got[0]["top"].as_i64().unwrap();
    // past the jump (line 285), every line on the way
    assert!(top <= -40, "{}", got[0]);
    let text: Vec<String> = got.iter().flat_map(page_text).collect();
    assert_eq!(text, doc[(300 + top) as usize..312].to_vec());
}

#[test]
fn the_documents_start_is_found_while_a_status_line_ticks() {
    let env = start(|_| {});
    let doc = doc_lines(60);
    {
        let mut st = env.fake.0.lock().unwrap();
        st.docs.insert(12, (doc.clone(), 20));
        st.ticking = true;
    }
    let (_, mut ch, _) = env.pair(Permission::Operate);
    put(
        &mut ch,
        json!({"t":"page_ahead","id":"g1","pane":12,"epoch":0,"dir":"up","rows":200}),
    );
    assert_eq!(ack(&recv_t(&mut ch, "ack")), (true, ""));
    let got = pages(&mut ch);
    assert_eq!(got[0]["ends"], json!([true, false]), "{}", got[0]);
    let text: Vec<String> = got.iter().flat_map(page_text).collect();
    assert_eq!(text, doc[0..32].to_vec());
}
