//! The history pane's Results scope, off the UI thread: one worker reads
//! the finished blocks of every open pane (command and output, through the
//! panes' shared terminal state) and, while `save_transcripts` is on, the
//! saved transcripts no open pane shows, then searches them
//! (`rift_core::result_search`). Queries queue to the worker; it runs only
//! the newest one waiting, and the window drops an answer whose sequence
//! number is not its latest (a slow, stale search never replaces the
//! current query's results).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use rift_core::BlockStore;
use rift_core::result_search::{self, Doc, Host, Limits, Outcome, RunId, Source};
use rift_pty::EventProxy;
use rift_pty::alacritty_terminal::sync::FairMutex;
use rift_pty::alacritty_terminal::term::Term;
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use crate::app::UserEvent;

/// Output bytes read per search (open panes and transcripts together).
const READ_BUDGET: usize = 32 << 20;
/// Output bytes read per block.
const BLOCK_BYTES: usize = 1 << 20;

/// What the worker reads of one open pane.
pub struct PaneFeed {
    /// The pane's run identity (`RIFT_PANE_ID`: its session-service session
    /// when durable, so a reattached pane is the same source).
    pub pane: String,
    /// Its transcript id: that saved file is this pane, never a second copy.
    pub transcript: String,
    /// Its blocks restored from a saved transcript are searched (saving is
    /// on and was not cleared since).
    pub restored: bool,
    pub term: Arc<FairMutex<Term<EventProxy>>>,
    pub blocks: Arc<Mutex<BlockStore>>,
    pub scrolled: Arc<AtomicU64>,
}

/// One search request.
pub struct Job {
    pub asker: Option<WindowId>,
    pub seq: u64,
    pub query: String,
    pub panes: Vec<PaneFeed>,
    /// `save_transcripts` is on: saved transcripts are searched too.
    pub saved: bool,
    /// Seconds since the epoch: now, and the local day's start.
    pub now: u64,
    pub today_start: u64,
}

/// The search worker (started on first use).
#[derive(Default)]
pub struct Worker {
    tx: Option<Sender<Job>>,
}

impl Worker {
    pub fn submit(&mut self, job: Job, proxy: &EventLoopProxy<UserEvent>) {
        if self.tx.is_none() {
            let (tx, rx) = channel::<Job>();
            let proxy = proxy.clone();
            let started = std::thread::Builder::new()
                .name("result-search".into())
                .spawn(move || serve(rx, proxy));
            if started.is_err() {
                log::warn!("result search: no worker thread");
                return;
            }
            self.tx = Some(tx);
        }
        if let Some(tx) = &self.tx
            && tx.send(job).is_err()
        {
            self.tx = None;
        }
    }
}

fn serve(rx: Receiver<Job>, proxy: EventLoopProxy<UserEvent>) {
    while let Ok(mut job) = rx.recv() {
        // only the newest query waiting is worth answering
        while let Ok(newer) = rx.try_recv() {
            job = newer;
        }
        let outcome = run(&job);
        if proxy
            .send_event(UserEvent::ResultsFound(
                job.asker,
                job.seq,
                Box::new(outcome),
            ))
            .is_err()
        {
            return;
        }
    }
}

/// The searchable runs of one open pane: its finished blocks (not a
/// background job's output), newest last, each with the output lines gone
/// from scrollback before its text (a match's line number counts them).
pub fn pane_docs(feed: &PaneFeed, budget: &mut usize) -> Vec<(Doc, u64)> {
    // a snapshot first: reading output takes the blocks lock itself
    let blocks: Vec<rift_core::Block> = feed
        .blocks
        .lock()
        .unwrap()
        .iter()
        .filter(|b| !b.background && matches!(b.state, rift_core::BlockState::Done { .. }))
        .filter(|b| feed.restored || !b.restored)
        .cloned()
        .collect();
    let mut docs = Vec::with_capacity(blocks.len());
    for b in blocks {
        let (output, output_state, trimmed) = crate::views::terminal::output_text(
            &feed.blocks,
            &feed.scrolled,
            &feed.term,
            b.id,
            BLOCK_BYTES.min(*budget),
        )
        .unwrap_or((String::new(), result_search::Output::Full, 0));
        *budget = budget.saturating_sub(output.len());
        let doc = Doc {
            id: RunId::Live {
                pane: feed.pane.clone(),
                line: b.prompt_start,
            },
            exit: b.exit_code(),
            finished: b
                .finished_at
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            host: Host::of(b.cwd.as_deref(), b.cwd_remote, b.host.as_deref()),
            // a remote path is not under this computer's home
            cwd: if b.cwd_remote {
                b.cwd.clone()
            } else {
                b.cwd.as_deref().map(crate::views::editor::shorten_home)
            },
            branch: b.branch.clone(),
            source: if b.restored {
                Source::Restored
            } else {
                Source::Pane
            },
            command: b.command,
            output,
            output_state,
        };
        docs.push((doc, trimmed));
    }
    docs
}

pub fn run(job: &Job) -> Outcome {
    let (text, filters) = result_search::parse_query(&job.query);
    let mut budget = READ_BUDGET;
    let mut docs: Vec<Doc> = Vec::new();
    let mut trimmed: HashMap<RunId, u64> = HashMap::new();
    for feed in &job.panes {
        for (doc, t) in pane_docs(feed, &mut budget) {
            if t > 0 {
                trimmed.insert(doc.id.clone(), t);
            }
            docs.push(doc);
        }
    }
    // never read while saving is off: search is no way around that setting
    if job.saved {
        let shown: HashSet<String> = job.panes.iter().map(|p| p.transcript.clone()).collect();
        let files = result_search::load_transcripts(&shown);
        docs.extend(result_search::saved_docs(files, &shown, budget));
    }
    mask_docs(&mut docs, rift_core::secrets::active().as_deref());
    let mut outcome = result_search::search(
        &docs,
        &text,
        &filters,
        job.now,
        job.today_start,
        Limits::default(),
    );
    shift_hit_lines(&mut outcome, &trimmed);
    outcome
}

/// "match on line N" counts from the output's start, not the first line
/// scrollback still holds: `trimmed` = lines gone before each run's text.
fn shift_hit_lines(outcome: &mut Outcome, trimmed: &HashMap<RunId, u64>) {
    for f in &mut outcome.found {
        if let (Some(n), Some(t)) = (f.hit_line.as_mut(), trimmed.get(&f.id)) {
            *n += *t as usize;
        }
    }
}

/// Secrets masked (`mask_secrets`) before searching: the preview, the
/// command and the hit show them masked, and typing a secret's value finds
/// nothing.
fn mask_docs(docs: &mut [Doc], m: Option<&rift_core::secrets::Masker>) {
    for d in docs {
        d.command = crate::block_export::masked(std::mem::take(&mut d.command), m);
        d.output = crate::block_export::masked(std::mem::take(&mut d.output), m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block whose first 40 output lines left the scrollback: a match on
    /// its 3rd kept line is on line 43 of the output.
    #[test]
    fn match_line_counts_lines_gone_from_scrollback() {
        let doc = Doc {
            id: RunId::Live {
                pane: "p".into(),
                line: 7,
            },
            command: "seq".into(),
            output: "41\n42\nneedle\n".into(),
            output_state: result_search::Output::Partial,
            exit: Some(0),
            finished: None,
            cwd: None,
            host: Host::Local,
            branch: None,
            source: Source::Pane,
        };
        let filters = result_search::Filters::default();
        let mut out = result_search::search(&[doc], "needle", &filters, 0, 0, Limits::default());
        assert_eq!(out.found[0].hit_line, Some(3));
        let trimmed = HashMap::from([(out.found[0].id.clone(), 40)]);
        shift_hit_lines(&mut out, &trimmed);
        assert_eq!(out.found[0].hit_line, Some(43));
    }

    #[test]
    fn a_masked_secret_is_not_found() {
        let token = format!("ghp_{}", "FAKE0123".repeat(5));
        let doc = Doc {
            id: RunId::Saved {
                transcript: "t".into(),
                index: 0,
            },
            command: format!("echo {token}"),
            output: format!("key {token}\n"),
            output_state: result_search::Output::Full,
            exit: Some(0),
            finished: None,
            cwd: None,
            host: Host::Local,
            branch: None,
            source: Source::Saved,
        };
        let m = rift_core::secrets::Masker::new(&[]).unwrap();
        let mut docs = vec![doc];
        mask_docs(&mut docs, Some(&m));
        let filters = result_search::Filters::default();
        let found = |q: &str| result_search::search(&docs, q, &filters, 0, 0, Limits::default());
        assert!(found("FAKE0123").found.is_empty());
        let f = &found("key").found[0];
        assert!(
            !f.command.contains("FAKE") && f.command.contains('•'),
            "{f:?}"
        );
        assert!(f.preview.iter().all(|l| !l.contains("FAKE")), "{f:?}");
    }
}
