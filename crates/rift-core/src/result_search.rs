//! Searching past results: finished blocks' commands *and* output, with
//! where and when they ran (the ⌃R history pane's Results scope).
//!
//! There is no index of its own. Each search reads what exists at that
//! moment: the blocks of open panes (restored ones included) and the saved
//! transcripts no open pane shows — and the latter only while
//! `save_transcripts` is on. Deleting transcripts or turning the option off
//! therefore removes them from search at once; output that was never saved
//! (or left the scrollback) is reported as unavailable, not guessed.
//!
//! A run is identified by where it happened, not by its text ([`RunId`]):
//! the same command run twice stays two results, and a pane that was
//! restored or reattached is one source, never counted again from its file.
//!
//! The work is bounded ([`Limits`]): bytes scanned and results returned.

use std::collections::HashSet;
use std::ops::Range;

use crate::transcript::Transcript;

/// One run's identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RunId {
    /// A block of an open pane: the pane (its `RIFT_PANE_ID`) and the
    /// block's first absolute line (unique and stable within the pane).
    Live { pane: String, line: u64 },
    /// Block `index` of the saved transcript `transcript`.
    Saved { transcript: String, index: usize },
}

/// Where a run happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    /// This computer.
    Local,
    /// Another machine (ssh), by its name.
    Remote(String),
    /// Another machine whose name was not recorded.
    RemoteUnnamed,
    /// Not recorded (no directory report from the shell).
    Unknown,
}

impl Host {
    /// From a block's record: its directory, whether that is remote, the
    /// remote host's name.
    pub fn of(cwd: Option<&str>, remote: bool, name: Option<&str>) -> Host {
        match (cwd, remote, name.filter(|n| !n.is_empty())) {
            (None, _, _) => Host::Unknown,
            (Some(_), false, _) => Host::Local,
            (Some(_), true, Some(n)) => Host::Remote(n.to_string()),
            (Some(_), true, None) => Host::RemoteUnnamed,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Host::Local => "local".into(),
            Host::Remote(n) => n.clone(),
            Host::RemoteUnnamed => "remote host".into(),
            Host::Unknown => "unknown host".into(),
        }
    }
}

/// How much of a run's output there is to search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// All of it.
    Full,
    /// Its start left the scrollback, or was cut to the transcript bound.
    Partial,
    /// None: every row left the scrollback.
    Unavailable,
}

/// Where a result was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// An open pane.
    Pane,
    /// A pane's block restored from a saved transcript.
    Restored,
    /// A saved transcript no open pane shows.
    Saved,
}

/// One searchable run.
#[derive(Debug, Clone)]
pub struct Doc {
    pub id: RunId,
    pub command: String,
    pub output: String,
    pub output_state: Output,
    pub exit: Option<i32>,
    /// Seconds since the epoch; `None`: not recorded.
    pub finished: Option<u64>,
    pub cwd: Option<String>,
    pub host: Host,
    pub branch: Option<String>,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Any,
    Ok,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Since {
    #[default]
    All,
    Today,
    Week,
}

/// The query's `key:value` words (`is:failed since:7d dir:api host:prod
/// branch:main`); the rest of the query is the text searched for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    pub status: Status,
    pub since: Since,
    pub dir: Option<String>,
    pub host: Option<String>,
    pub branch: Option<String>,
}

impl Filters {
    /// Short labels of the filters in force (the pane's chips).
    pub fn labels(&self) -> Vec<String> {
        let mut out = Vec::new();
        match self.status {
            Status::Any => {}
            Status::Ok => out.push("Succeeded".into()),
            Status::Failed => out.push("Failed".into()),
        }
        match self.since {
            Since::All => {}
            Since::Today => out.push("Today".into()),
            Since::Week => out.push("Last 7 days".into()),
        }
        if let Some(d) = &self.dir {
            out.push(format!("Dir {d}"));
        }
        if let Some(h) = &self.host {
            out.push(format!("Host {h}"));
        }
        if let Some(b) = &self.branch {
            out.push(format!("Branch {b}"));
        }
        out
    }
}

/// Split the query into the text to find and its filters. A word that is
/// not a known `key:value` stays text.
pub fn parse_query(query: &str) -> (String, Filters) {
    let mut f = Filters::default();
    let mut text = Vec::new();
    for word in query.split_whitespace() {
        let Some((key, value)) = word.split_once(':') else {
            text.push(word);
            continue;
        };
        let v = value.to_lowercase();
        let known = match (key.to_lowercase().as_str(), v.as_str()) {
            ("is" | "status", "ok" | "success" | "succeeded" | "passed") => {
                f.status = Status::Ok;
                true
            }
            ("is" | "status", "failed" | "fail" | "error") => {
                f.status = Status::Failed;
                true
            }
            ("since" | "date", "today") => {
                f.since = Since::Today;
                true
            }
            ("since" | "date", "7d" | "week") => {
                f.since = Since::Week;
                true
            }
            ("since" | "date", "all") => true,
            ("dir" | "in", v) if !v.is_empty() => {
                f.dir = Some(value.to_string());
                true
            }
            ("host", v) if !v.is_empty() => {
                f.host = Some(v.to_string());
                true
            }
            ("branch", v) if !v.is_empty() => {
                f.branch = Some(value.to_string());
                true
            }
            _ => false,
        };
        if !known {
            text.push(word);
        }
    }
    (text.join(" "), f)
}

/// Bounds of one search.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Command and output bytes read before the search stops.
    pub max_bytes: usize,
    pub max_results: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_bytes: 32 << 20,
            max_results: 200,
        }
    }
}

/// Lines of output a preview shows, and their width.
pub const PREVIEW_LINES: usize = 10;
pub const PREVIEW_COLS: usize = 400;

/// A found run, small enough to hand to the UI.
#[derive(Debug, Clone)]
pub struct Found {
    pub id: RunId,
    pub command: String,
    pub exit: Option<i32>,
    pub finished: Option<u64>,
    pub cwd: Option<String>,
    pub host: Host,
    pub branch: Option<String>,
    pub source: Source,
    pub output_state: Output,
    /// The match in the command (byte range), when it is there.
    pub in_command: Option<Range<usize>>,
    /// Output lines around the match (or the output's last lines).
    pub preview: Vec<String>,
    /// The matching preview line and its byte range.
    pub hit: Option<(usize, Range<usize>)>,
    /// 1-based line number of the match in the output.
    pub hit_line: Option<usize>,
}

/// What a search returned.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub found: Vec<Found>,
    pub scanned_bytes: usize,
    /// It stopped at a bound: older runs were not searched.
    pub truncated: bool,
}

/// Case-insensitive find of `needle` (already lowercase) in `hay`: the byte
/// range in `hay`, only where lowercasing kept the offsets.
fn find_in(hay: &str, needle: &str) -> Option<Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    let lower = hay.to_lowercase();
    let at = lower.find(needle)?;
    if lower.len() == hay.len()
        && hay.is_char_boundary(at)
        && hay.is_char_boundary(at + needle.len())
    {
        Some(at..at + needle.len())
    } else {
        // found, but the offsets moved: report the line without a span
        Some(0..0)
    }
}

fn clip_line(line: &str) -> String {
    match line.char_indices().nth(PREVIEW_COLS) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None => line.to_string(),
    }
}

fn passes(d: &Doc, f: &Filters, now: u64, today_start: u64) -> bool {
    let status = match f.status {
        Status::Any => true,
        Status::Ok => d.exit == Some(0),
        Status::Failed => d.exit.is_some_and(|c| c != 0),
    };
    let since = match f.since {
        Since::All => true,
        Since::Today => d.finished.is_some_and(|t| t >= today_start),
        Since::Week => d
            .finished
            .is_some_and(|t| t >= now.saturating_sub(7 * 86_400)),
    };
    let dir = f.dir.as_ref().is_none_or(|want| {
        d.cwd
            .as_ref()
            .is_some_and(|c| c.to_lowercase().contains(&want.to_lowercase()))
    });
    let host = f
        .host
        .as_ref()
        .is_none_or(|want| match (&d.host, want.as_str()) {
            (Host::Local, "local" | "localhost") => true,
            (Host::Unknown | Host::RemoteUnnamed, "unknown") => true,
            (Host::RemoteUnnamed, "remote") => true,
            (Host::Remote(_), "remote") => true,
            (Host::Remote(n), w) => n.to_lowercase().contains(w),
            _ => false,
        });
    let branch = f.branch.as_ref().is_none_or(|want| {
        d.branch
            .as_ref()
            .is_some_and(|b| b.to_lowercase().contains(&want.to_lowercase()))
    });
    status && since && dir && host && branch
}

/// Search `docs` (any order: newest first is applied here) for `text` in
/// commands and output, within `filters` and `limits`. `now` and
/// `today_start` are seconds since the epoch (local midnight for the
/// latter).
pub fn search(
    docs: &[Doc],
    text: &str,
    filters: &Filters,
    now: u64,
    today_start: u64,
    limits: Limits,
) -> Outcome {
    let needle = text.trim().to_lowercase();
    let mut order: Vec<&Doc> = docs.iter().collect();
    // newest first; runs with no recorded time after the dated ones
    order.sort_by_key(|d| std::cmp::Reverse(d.finished));
    let mut out = Outcome::default();
    for d in order {
        if out.found.len() >= limits.max_results || out.scanned_bytes >= limits.max_bytes {
            out.truncated = true;
            break;
        }
        if !passes(d, filters, now, today_start) {
            continue;
        }
        out.scanned_bytes += d.command.len();
        let in_command = find_in(&d.command, &needle);
        // the output's first match (also when the command matched: the
        // preview shows it)
        let mut hit_line = None;
        if !needle.is_empty() {
            let room = limits.max_bytes.saturating_sub(out.scanned_bytes);
            let mut read = 0usize;
            for (i, line) in d.output.lines().enumerate() {
                if read >= room {
                    out.truncated = true;
                    break;
                }
                read += line.len() + 1;
                if let Some(r) = find_in(line, &needle) {
                    hit_line = Some((i, r));
                    break;
                }
            }
            out.scanned_bytes += read;
            if hit_line.is_none() && in_command.is_none() {
                continue;
            }
        }
        let lines: Vec<&str> = d.output.lines().collect();
        let (preview, hit) = match &hit_line {
            Some((i, r)) => {
                let start = i.saturating_sub(2);
                let end = (start + PREVIEW_LINES).min(lines.len());
                let preview: Vec<String> = lines[start..end].iter().map(|l| clip_line(l)).collect();
                let r = if r.end <= PREVIEW_COLS {
                    r.clone()
                } else {
                    0..0
                };
                (preview, Some((i - start, r)))
            }
            None => {
                let start = lines.len().saturating_sub(PREVIEW_LINES);
                (lines[start..].iter().map(|l| clip_line(l)).collect(), None)
            }
        };
        out.found.push(Found {
            id: d.id.clone(),
            command: d.command.clone(),
            exit: d.exit,
            finished: d.finished,
            cwd: d.cwd.clone(),
            host: d.host.clone(),
            branch: d.branch.clone(),
            source: d.source,
            output_state: d.output_state,
            in_command: in_command.filter(|r| !r.is_empty()),
            preview,
            hit,
            hit_line: hit_line.map(|(i, _)| i + 1),
        });
    }
    out
}

/// The runs of saved transcripts, skipping `shown` (ids an open pane
/// shows: its blocks are searched there, restored or reattached, and must
/// not count twice). At most `budget` bytes of output are taken.
pub fn saved_docs(
    transcripts: impl IntoIterator<Item = (String, Transcript)>,
    shown: &HashSet<String>,
    budget: usize,
) -> Vec<Doc> {
    let mut left = budget;
    let mut docs = Vec::new();
    for (id, t) in transcripts {
        if shown.contains(&id) {
            continue;
        }
        let meta = t.meta();
        for (index, (b, m)) in t.blocks.iter().zip(meta).enumerate() {
            if left == 0 {
                return docs;
            }
            let take = b.output.len().min(left);
            let mut cut = take;
            while !b.output.is_char_boundary(cut) {
                cut -= 1;
            }
            left -= cut;
            docs.push(Doc {
                id: RunId::Saved {
                    transcript: id.clone(),
                    index,
                },
                command: m.command,
                output: b.output[..cut].to_string(),
                output_state: if cut < b.output.len() {
                    Output::Partial
                } else {
                    Output::Full
                },
                exit: b.exit,
                finished: b.finished,
                host: Host::of(m.cwd.as_deref(), m.remote, m.host.as_deref()),
                cwd: m.cwd,
                branch: m.branch,
                source: Source::Saved,
            });
        }
    }
    docs
}

/// Saved transcripts on disk (for [`saved_docs`]), each read within the
/// transcript bounds.
pub fn load_transcripts(skip: &HashSet<String>) -> Vec<(String, Transcript)> {
    crate::transcript::ids()
        .into_iter()
        .filter(|id| !skip.contains(id))
        .filter_map(|id| crate::transcript::load(&id).map(|t| (id, t)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::SavedBlock;

    fn doc(pane: &str, line: u64, cmd: &str, out: &str) -> Doc {
        Doc {
            id: RunId::Live {
                pane: pane.into(),
                line,
            },
            command: cmd.into(),
            output: out.into(),
            output_state: Output::Full,
            exit: Some(0),
            finished: Some(1000 + line),
            cwd: Some("~/api".into()),
            host: Host::Local,
            branch: None,
            source: Source::Pane,
        }
    }

    fn all(docs: &[Doc], q: &str) -> Outcome {
        let (text, f) = parse_query(q);
        search(docs, &text, &f, 2000, 1500, Limits::default())
    }

    #[test]
    fn timestamps_sort_without_overflow_and_unknown_is_last() {
        let docs: Vec<_> = [None, Some(0), Some(u64::MAX), Some(u64::MAX - 1)]
            .into_iter()
            .enumerate()
            .map(|(i, finished)| Doc {
                finished,
                ..doc("p.1", i as u64, "echo", "")
            })
            .collect();
        let found = all(&docs, "echo").found;
        assert_eq!(
            found.iter().map(|f| f.finished).collect::<Vec<_>>(),
            [Some(u64::MAX), Some(u64::MAX - 1), Some(0), None]
        );
    }

    #[test]
    fn query_words_become_filters() {
        let (text, f) = parse_query("migrate is:failed since:7d host:Prod dir:api branch:main x:y");
        assert_eq!(text, "migrate x:y");
        assert_eq!(f.status, Status::Failed);
        assert_eq!(f.since, Since::Week);
        assert_eq!(f.host.as_deref(), Some("prod"));
        assert_eq!(f.dir.as_deref(), Some("api"));
        assert_eq!(f.branch.as_deref(), Some("main"));
        assert_eq!(
            f.labels(),
            [
                "Failed",
                "Last 7 days",
                "Dir api",
                "Host prod",
                "Branch main"
            ]
        );
        assert_eq!(parse_query("is:ok since:today").1.status, Status::Ok);
        assert_eq!(parse_query("since:today").1.since, Since::Today);
        assert_eq!(parse_query("host:").0, "host:", "an empty value is text");
    }

    #[test]
    fn finds_output_and_commands_and_keeps_repeated_runs_apart() {
        let docs = vec![
            doc(
                "p.1",
                10,
                "make migrate",
                "applying 0003\nERROR: relation exists\n",
            ),
            doc("p.1", 20, "make migrate", "applying 0003\nok\n"),
            doc("p.2", 5, "ls", "README\n"),
        ];
        let o = all(&docs, "relation exists");
        assert_eq!(o.found.len(), 1);
        let f = &o.found[0];
        assert_eq!(
            f.id,
            RunId::Live {
                pane: "p.1".into(),
                line: 10
            }
        );
        assert_eq!(f.hit_line, Some(2));
        let (i, r) = f.hit.clone().unwrap();
        assert_eq!(&f.preview[i][r], "relation exists");
        // the same command twice: two results, newest first
        let o = all(&docs, "migrate");
        let ids: Vec<_> = o.found.iter().map(|f| f.id.clone()).collect();
        assert_eq!(
            ids,
            [
                RunId::Live {
                    pane: "p.1".into(),
                    line: 20
                },
                RunId::Live {
                    pane: "p.1".into(),
                    line: 10
                }
            ]
        );
        assert_eq!(o.found[0].in_command, Some(5..12));
        // nothing typed: everything, each with the output's last lines
        let o = all(&docs, "");
        assert_eq!(o.found.len(), 3);
        assert_eq!(o.found[2].preview, ["README"]);
    }

    #[test]
    fn hosts_and_branches_tell_runs_apart_and_unknown_stays_unknown() {
        let mut prod = doc("p.1", 1, "deploy", "done");
        prod.host = Host::of(Some("/srv"), true, Some("prod-1"));
        prod.branch = Some("main".into());
        let mut staging = doc("p.2", 2, "deploy", "done");
        staging.host = Host::of(Some("/srv"), true, Some("staging"));
        staging.branch = Some("release".into());
        let mut unknown = doc("p.3", 3, "deploy", "done");
        unknown.host = Host::of(None, false, None);
        unknown.exit = None;
        unknown.finished = None;
        let docs = vec![prod, staging, unknown];
        assert_eq!(docs[0].host.label(), "prod-1");
        assert_eq!(docs[1].host.label(), "staging");
        assert_eq!(docs[2].host.label(), "unknown host");
        assert_eq!(Host::of(Some("/x"), true, None).label(), "remote host");
        let one = |q| {
            all(&docs, q)
                .found
                .iter()
                .map(|f| f.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(one("deploy host:prod"), [docs[0].id.clone()]);
        assert_eq!(one("deploy branch:release"), [docs[1].id.clone()]);
        assert_eq!(one("deploy host:unknown"), [docs[2].id.clone()]);
        // no status or time recorded: neither succeeded, failed nor recent
        assert!(!one("deploy is:ok").contains(&docs[2].id));
        assert!(!one("deploy is:failed").contains(&docs[2].id));
        assert!(!one("deploy since:today").contains(&docs[2].id));
        assert!(!one("deploy host:local").contains(&docs[2].id));
    }

    #[test]
    fn status_and_date_filters() {
        let mut failed = doc("p.1", 900, "cargo test", "boom");
        failed.exit = Some(101);
        let old = doc("p.1", 1, "cargo test", "ok");
        let docs = vec![failed, old];
        // now 2000, today from 1500, 7 days back from 0
        assert_eq!(all(&docs, "cargo is:failed").found.len(), 1);
        assert_eq!(all(&docs, "cargo is:ok").found.len(), 1);
        assert_eq!(all(&docs, "cargo since:today").found.len(), 1);
        assert_eq!(all(&docs, "cargo since:7d").found.len(), 2);
    }

    #[test]
    fn searches_are_bounded() {
        let docs: Vec<Doc> = (0..50)
            .map(|i| doc("p.1", i, &format!("echo {i}"), &"x".repeat(1000)))
            .collect();
        let o = search(
            &docs,
            "zzz",
            &Filters::default(),
            0,
            0,
            Limits {
                max_bytes: 10_000,
                max_results: 200,
            },
        );
        assert!(o.truncated);
        assert!(o.scanned_bytes <= 10_000 + 1100, "{}", o.scanned_bytes);
        let o = search(
            &docs,
            "echo",
            &Filters::default(),
            0,
            0,
            Limits {
                max_bytes: 1 << 20,
                max_results: 7,
            },
        );
        assert_eq!(o.found.len(), 7);
        assert!(o.truncated);
        // a preview is a few lines, each clipped
        let long = doc(
            "p.1",
            1,
            "cat big",
            &format!("{}\n", "y".repeat(5000)).repeat(100),
        );
        let o = all(&[long], "cat");
        assert_eq!(o.found[0].preview.len(), PREVIEW_LINES);
        assert!(o.found[0].preview[0].chars().count() <= PREVIEW_COLS + 1);
    }

    fn saved(cmd: &str, out: &str) -> SavedBlock {
        SavedBlock {
            command: cmd.into(),
            exit: Some(1),
            duration_ms: 5,
            cwd: Some("/srv".into()),
            cwd_remote: true,
            output: out.into(),
            finished: Some(1_800),
            branch: Some("main".into()),
            host: Some("db-1".into()),
        }
    }

    #[test]
    fn saved_transcripts_count_once_and_within_the_budget() {
        let t = Transcript::bounded(
            vec![saved("psql -f m.sql", "ERROR here"), saved("ls", "a")],
            1,
        );
        let files = vec![
            ("open-pane".to_string(), t.clone()),
            ("closed".to_string(), t),
        ];
        // the open pane's transcript is searched through the pane itself
        let shown: HashSet<String> = ["open-pane".to_string()].into();
        let docs = saved_docs(files.clone(), &shown, 1 << 20);
        assert_eq!(docs.len(), 2);
        assert!(
            docs.iter().all(
                |d| matches!(&d.id, RunId::Saved { transcript, .. } if transcript == "closed")
            )
        );
        assert_eq!(docs[0].host, Host::Remote("db-1".into()));
        assert_eq!(docs[0].branch.as_deref(), Some("main"));
        assert_eq!(docs[0].source, Source::Saved);
        let o = all(&docs, "error is:failed host:db");
        assert_eq!(o.found.len(), 1);
        assert_eq!(
            o.found[0].id,
            RunId::Saved {
                transcript: "closed".into(),
                index: 0
            }
        );
        // the output budget: what did not fit is partial, then nothing
        let docs = saved_docs(files, &HashSet::new(), 4);
        assert_eq!(docs[0].output, "ERRO");
        assert_eq!(docs[0].output_state, Output::Partial);
        assert_eq!(docs.len(), 1, "the budget ran out");
        // no transcripts (cleared, or saving off): no saved results
        assert!(saved_docs(Vec::new(), &HashSet::new(), 1 << 20).is_empty());
    }
}
