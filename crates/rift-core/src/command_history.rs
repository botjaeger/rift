//! Rift's own command history: every command a local pane finished, with
//! the directory it ran in, that directory's repository, when it finished
//! and its exit code. Kept in `~/.local/share/rift/history.json` so history
//! suggestions can rank by directory across restarts (the imported shell
//! history has no directory).
//!
//! Ranking ([`tier`]): commands run in the same directory first, then in the
//! same repository, then everywhere else; newest first within each. Each
//! entry also names its shell session and place in it, so [`next_after`]
//! can tell what usually follows a command (Warp's next-command history).
//!
//! Bounded like transcripts: the newest [`MAX_ENTRIES`] entries and
//! [`MAX_BYTES`] of text; a field over [`MAX_FIELD_BYTES`] is not kept. The
//! file is written atomically (0600) under a lock every Rift process takes,
//! and is untrusted on load: one over the bounds, unparsable or of another
//! version is not read, and the next write moves it aside to
//! `history.json.bad` instead of writing over it.
//!
//! Not recorded ([`should_record`]): a command with a leading space (the
//! shells' `HIST_IGNORE_SPACE` / `ignorespace` convention), one run on
//! another host (its directory is not on this disk), and one the secret
//! patterns match (`rift_core::secrets`: the user's when masking is on,
//! else the built-in ones).
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

pub const VERSION: u32 = 1;
pub const MAX_ENTRIES: usize = 10_000;
/// Commands and directories together.
pub const MAX_BYTES: usize = 2 << 20;
/// One command or directory.
pub const MAX_FIELD_BYTES: usize = 4 << 10;
/// Worst-case JSON escaping of the text plus per-entry metadata.
pub const MAX_FILE_BYTES: usize = MAX_BYTES * 6 + MAX_ENTRIES * 128;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub command: String,
    /// The directory it ran in.
    pub cwd: Option<String>,
    /// The working tree of the repository `cwd` is in, when it is in one
    /// (`fs_tree::root_for`); outside one, `cwd` itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// When it finished (seconds since the epoch).
    pub finished: u64,
    pub exit: Option<i32>,
    /// The shell session it ran in: a new id for every shell a pane starts
    /// (a restarted or restored pane gets a new one). Entries written
    /// before sessions were kept have none and give no next command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Its place in `session`: commands of one session in the order they
    /// finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct File {
    version: u32,
    entries: Vec<Entry>,
}

fn fits(e: &Entry) -> bool {
    e.command.len() <= MAX_FIELD_BYTES
        && [&e.cwd, &e.repo, &e.session]
            .into_iter()
            .flatten()
            .all(|s| s.len() <= MAX_FIELD_BYTES)
}

fn size(e: &Entry) -> usize {
    e.command.len()
        + [&e.cwd, &e.repo, &e.session]
            .into_iter()
            .flatten()
            .map(String::len)
            .sum::<usize>()
}

/// The newest entries within the bounds, oldest first; an entry with a field
/// over [`MAX_FIELD_BYTES`] is dropped (a cut command is not one to suggest).
pub fn bounded(mut entries: Vec<Entry>) -> Vec<Entry> {
    entries.retain(fits);
    let mut total = 0usize;
    let mut keep = 0;
    for e in entries.iter().rev() {
        let n = size(e);
        if keep == MAX_ENTRIES || total + n > MAX_BYTES {
            break;
        }
        total += n;
        keep += 1;
    }
    entries.split_off(entries.len() - keep)
}

fn valid(f: &File) -> bool {
    f.version == VERSION
        && f.entries.len() <= MAX_ENTRIES
        && f.entries.iter().all(fits)
        && f.entries.iter().map(size).sum::<usize>() <= MAX_BYTES
}

/// Whether a finished command goes into the history (see the module doc).
/// `remote`: its directory is on another host.
pub fn should_record(command: &str, remote: bool) -> bool {
    if remote || command.trim().is_empty() || command.starts_with([' ', '\t']) {
        return false;
    }
    let found = match crate::secrets::active() {
        Some(m) => !m.find(command).is_empty(),
        None => !builtin_masker().find(command).is_empty(),
    };
    !found
}

fn builtin_masker() -> &'static crate::secrets::Masker {
    static M: std::sync::OnceLock<crate::secrets::Masker> = std::sync::OnceLock::new();
    M.get_or_init(|| crate::secrets::Masker::new(&[]).expect("built-in patterns"))
}

/// The repository root used for ranking (`fs_tree::root_for`, as a string),
/// remembered per directory for the run: ↑, ⌃R and the completion worker
/// ask again and again, and a slow (network) directory must not stall each.
pub fn repo_of(cwd: &str) -> String {
    use std::collections::HashMap;
    static SEEN: std::sync::OnceLock<Mutex<HashMap<String, String>>> = std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Some(repo) = seen.lock().unwrap_or_else(|e| e.into_inner()).get(cwd) {
        return repo.clone();
    }
    // resolved outside the lock: another directory's walk never waits on it
    let repo = crate::fs_tree::root_for(Path::new(cwd))
        .to_string_lossy()
        .into_owned();
    let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
    if seen.len() >= 1024 {
        seen.clear();
    }
    seen.insert(cwd.to_string(), repo.clone());
    repo
}

/// How close an entry that ran in `cwd` (repository `repo`) is to the
/// current directory `here` (repository `here_repo`): 0 the same directory,
/// 1 the same repository, 2 anything else.
pub fn tier(
    cwd: Option<&str>,
    repo: Option<&str>,
    here: Option<&str>,
    here_repo: Option<&str>,
) -> u8 {
    let same = |a: Option<&str>, b: Option<&str>| a.is_some() && a == b;
    if same(cwd, here) {
        0
    } else if same(repo, here_repo) {
        1
    } else {
        2
    }
}

/// The distinct commands of `entries` starting with `prefix` (not equal to
/// it, single-line), ranked by [`tier`] against `here` / `here_repo`, newest
/// first within a tier.
pub fn ranked_matches<'a>(
    entries: &'a [Entry],
    prefix: &str,
    here: Option<&str>,
    here_repo: Option<&str>,
) -> Vec<&'a str> {
    let mut found: Vec<(u8, u64, usize, &str)> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.command.starts_with(prefix) && e.command != prefix && !e.command.contains('\n')
        })
        .map(|(i, e)| {
            let t = tier(e.cwd.as_deref(), e.repo.as_deref(), here, here_repo);
            (t, e.finished, i, e.command.as_str())
        })
        .collect();
    // newest: the later finish, then the later entry
    found.sort_by_key(|&(t, f, i, _)| (t, std::cmp::Reverse((f, i))));
    let mut seen = std::collections::HashSet::new();
    found
        .into_iter()
        .map(|(.., c)| c)
        .filter(|c| seen.insert(*c))
        .collect()
}

/// How many earlier runs of the previous command [`next_after`] looks at.
pub const NEXT_CONTEXTS: usize = 25;
/// How many next commands [`next_after`] gives at most.
pub const NEXT_MAX: usize = 5;

/// What usually follows the command a pane just finished (`last`: its
/// command line, directory and exit code), for a line starting with
/// `prefix`: the [`NEXT_CONTEXTS`] newest entries of that same command, in
/// that same directory with that same exit code, each followed by the next
/// non-empty command of its own session; those starting with `prefix`
/// (case-sensitive, single-line, not equal to it), most frequent first
/// (ties: the newer), at most [`NEXT_MAX`]. No directory: nothing.
pub fn next_after<'a>(
    entries: &'a [Entry],
    last: (&str, Option<&str>, Option<i32>),
    prefix: &str,
) -> Vec<&'a str> {
    let (command, cwd, exit) = last;
    if cwd.is_none() {
        return Vec::new();
    }
    // (command, count, newest position)
    let mut found: Vec<(&str, usize, usize)> = Vec::new();
    let runs = entries
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, e)| e.command == command && e.cwd.as_deref() == cwd && e.exit == exit)
        .take(NEXT_CONTEXTS);
    for (i, run) in runs {
        let Some(session) = run.session.as_deref() else {
            continue;
        };
        let next = entries[i + 1..].iter().enumerate().find(|(_, e)| {
            e.session.as_deref() == Some(session)
                && !e.command.trim().is_empty()
                && e.seq.zip(run.seq).is_none_or(|(a, b)| a > b)
        });
        let Some((j, next)) = next else { continue };
        let c = next.command.as_str();
        if !c.starts_with(prefix) || c == prefix || c.contains('\n') {
            continue;
        }
        let at = i + 1 + j;
        match found.iter_mut().find(|f| f.0 == c) {
            Some(f) => {
                f.1 += 1;
                f.2 = f.2.max(at);
            }
            None => found.push((c, 1, at)),
        }
    }
    found.sort_by_key(|&(_, n, at)| std::cmp::Reverse((n, at)));
    found.into_iter().take(NEXT_MAX).map(|f| f.0).collect()
}

/// The history lines for a line starting with `prefix`, best first: what
/// usually follows `last` ([`next_after`]), then [`ranked_matches`].
pub fn suggestions<'a>(
    entries: &'a [Entry],
    last: Option<(&str, Option<&str>, Option<i32>)>,
    prefix: &str,
    here: Option<&str>,
    here_repo: Option<&str>,
) -> Vec<&'a str> {
    let mut out = last.map_or_else(Vec::new, |l| next_after(entries, l, prefix));
    // at most NEXT_MAX: a scan per ranked line stays cheap
    let next = out.len();
    for c in ranked_matches(entries, prefix, here, here_repo) {
        if !out[..next].contains(&c) {
            out.push(c);
        }
    }
    out
}

/// `$XDG_DATA_HOME/rift/history.json` (default `~/.local/share/rift`).
pub fn path() -> Option<PathBuf> {
    crate::shell_integration::data_dir().map(|d| d.join("history.json"))
}

/// What a history file holds.
enum Loaded {
    Missing,
    /// Over the bounds, unparsable, of another version or not readable.
    Unreadable,
    Entries(Vec<Entry>),
}

fn read_file(path: &Path) -> Loaded {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::Missing,
        Err(_) => return Loaded::Unreadable,
    };
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES as u64 {
        return Loaded::Unreadable;
    }
    match std::fs::File::open(path).ok().and_then(load_reader) {
        Some(entries) => Loaded::Entries(entries),
        None => Loaded::Unreadable,
    }
}

/// The entries in `path`, oldest first; nothing when it is missing, over
/// the bounds or corrupt.
pub fn load_from(path: &Path) -> Vec<Entry> {
    match read_file(path) {
        Loaded::Entries(entries) => entries,
        Loaded::Missing | Loaded::Unreadable => Vec::new(),
    }
}

/// Add `batch` to the file as it is on disk now, under the lock every Rift
/// process takes, so their entries survive. A file that cannot be read is
/// moved aside to `history.json.bad` first, never written over.
pub fn append_to(path: &Path, batch: Vec<Entry>) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let _lock = lock(path);
    let mut all = match read_file(path) {
        Loaded::Missing => Vec::new(),
        Loaded::Entries(entries) => entries,
        Loaded::Unreadable => {
            if std::fs::rename(path, path.with_extension("json.bad")).is_err() {
                return false;
            }
            log::warn!(
                "command history: {} could not be read; moved aside to history.json.bad",
                path.display()
            );
            Vec::new()
        }
    };
    all.extend(batch);
    save_to(path, all)
}

/// The writers' lock (`history.json.lock`, or beside another `.json` file
/// `path`), held while the returned file is open. `None` when it cannot be
/// taken: the write goes ahead unlocked.
#[cfg(unix)]
pub(crate) fn lock(path: &Path) -> Option<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .open(path.with_extension("json.lock"))
        .ok()?;
    (unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } == 0).then_some(f)
}

#[cfg(not(unix))]
pub(crate) fn lock(_: &Path) -> Option<()> {
    None
}

fn load_reader(reader: impl Read) -> Option<Vec<Entry>> {
    let mut body = Vec::new();
    reader
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .ok()?;
    if body.len() > MAX_FILE_BYTES {
        return None;
    }
    let f: File = serde_json::from_slice(&body).ok()?;
    valid(&f).then_some(f.entries)
}

/// Write `entries` (bounded) to `path`: atomic, private (0600).
pub fn save_to(path: &Path, entries: Vec<Entry>) -> bool {
    let f = File {
        version: VERSION,
        entries: bounded(entries),
    };
    let Some(dir) = path.parent() else {
        return false;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let Ok(body) = serde_json::to_vec(&f) else {
        return false;
    };
    write_private(path, &body)
}

/// Write `body` to the `.json` file `path` (its directory exists):
/// atomic (a per-process temporary file renamed over it) and private (0600).
pub(crate) fn write_private(path: &Path, body: &[u8]) -> bool {
    // one per process: two writers never share a half-written file
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let written = opts
        .open(&tmp)
        .and_then(|mut f| {
            std::io::Write::write_all(&mut f, body)?;
            // on disk before the rename publishes it (a power loss leaves
            // the old file, not an empty one)
            f.sync_all()
        })
        .and_then(|_| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.is_ok()
}

/// The history a process uses: what it read at start plus what it recorded
/// since. Cheap to clone; a background thread loads, resolves repositories
/// and writes, so the caller never touches the disk.
#[derive(Clone, Default)]
pub struct Store {
    entries: Arc<Mutex<Arc<Vec<Entry>>>>,
    tx: Option<Sender<Msg>>,
}

enum Msg {
    Record(Entry),
    /// Answer once everything sent before is written.
    Flush(Sender<()>),
}

impl Store {
    /// Start the store. `read`: the file's entries are suggested (else only
    /// this run's). `path`: where entries are written (`None`: kept in
    /// memory only). Each write merges into the file as it is on disk then,
    /// so other Rift processes' entries survive.
    pub fn start(path: Option<PathBuf>, read: bool) -> Store {
        let entries: Arc<Mutex<Arc<Vec<Entry>>>> = Arc::default();
        let (tx, rx) = channel::<Msg>();
        let shared = entries.clone();
        let spawned = std::thread::Builder::new()
            .name("command-history".into())
            .spawn(move || {
                if read && let Some(p) = &path {
                    let loaded = load_from(p);
                    let mut cur = shared.lock().unwrap_or_else(|e| e.into_inner());
                    // nothing recorded yet can be in the file: it stays newest
                    let mut all = loaded;
                    all.extend(cur.iter().cloned());
                    *cur = Arc::new(bounded(all));
                }
                while let Ok(first) = rx.recv() {
                    let mut batch = Vec::new();
                    let mut waiting = Vec::new();
                    for msg in std::iter::once(first).chain(rx.try_iter()) {
                        match msg {
                            Msg::Record(e) => batch.push(e),
                            Msg::Flush(done) => waiting.push(done),
                        }
                    }
                    for e in &mut batch {
                        e.repo = e.cwd.as_deref().map(repo_of);
                    }
                    if !batch.is_empty() {
                        {
                            let mut cur = shared.lock().unwrap_or_else(|e| e.into_inner());
                            let v = Arc::make_mut(&mut *cur);
                            v.extend(batch.iter().cloned());
                            *v = bounded(std::mem::take(v));
                        }
                        if let Some(p) = &path
                            && !append_to(p, batch)
                        {
                            log::warn!("command history: could not write {}", p.display());
                        }
                    }
                    for done in waiting {
                        let _ = done.send(());
                    }
                }
            });
        Store {
            entries,
            tx: spawned.is_ok().then_some(tx),
        }
    }

    /// Record a finished command (repository resolved and written off this
    /// thread); `session`: its shell session and place in it. Callers check
    /// [`should_record`] first.
    pub fn record(
        &self,
        command: String,
        cwd: Option<String>,
        finished: u64,
        exit: Option<i32>,
        session: Option<(String, u64)>,
    ) {
        if let Some(tx) = &self.tx {
            let (session, seq) = session.unzip();
            let _ = tx.send(Msg::Record(Entry {
                command,
                cwd,
                repo: None,
                finished,
                exit,
                session,
                seq,
            }));
        }
    }

    /// Wait, at most `timeout`, until everything recorded so far is written
    /// (at quit). False when it was not.
    pub fn flush(&self, timeout: std::time::Duration) -> bool {
        let Some(tx) = &self.tx else { return true };
        let (done, wait) = channel();
        tx.send(Msg::Flush(done)).is_ok() && wait.recv_timeout(timeout).is_ok()
    }

    /// The entries now, oldest first.
    pub fn snapshot(&self) -> Arc<Vec<Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(cmd: &str, cwd: &str, repo: &str, finished: u64) -> Entry {
        Entry {
            command: cmd.into(),
            cwd: Some(cwd.into()),
            repo: Some(repo.into()),
            finished,
            exit: Some(0),
            session: None,
            seq: None,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rift-cmdhist-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// An entry of shell session `session`, at `seq` in it.
    fn run(cmd: &str, cwd: &str, session: &str, seq: u64, exit: i32) -> Entry {
        Entry {
            command: cmd.into(),
            cwd: Some(cwd.into()),
            repo: Some(cwd.into()),
            finished: seq,
            exit: Some(exit),
            session: Some(session.into()),
            seq: Some(seq),
        }
    }

    const PREV: &str = "cd project1";
    const BACKUP: &str = "bash scripts/db.sh backup";
    const RESTORE: &str = "bash scripts/db.sh restore --local --force";
    const PREFIX: &str = "bash scripts/db.sh";

    /// Three shell sessions, each: `cd project1` in /x, then backup, then
    /// restore in /x/project1 (the user's daily routine).
    fn routine() -> Vec<Entry> {
        (0..3)
            .flat_map(|s| {
                let id = format!("s{s}");
                let t = s * 10;
                [
                    run(PREV, "/x", &id, t, 0),
                    run(BACKUP, "/x/project1", &id, t + 1, 0),
                    run(RESTORE, "/x/project1", &id, t + 2, 0),
                ]
            })
            .collect()
    }

    #[test]
    fn next_command_follows_what_usually_comes_after_the_previous_one() {
        let entries = routine();
        let here = Some("/x/project1");
        let last = (PREV, Some("/x"), Some(0));
        assert_eq!(next_after(&entries, last, PREFIX), [BACKUP]);
        assert_eq!(
            suggestions(&entries, Some(last), PREFIX, here, here),
            [BACKUP, RESTORE]
        );
        // no previous command: the newest in the directory
        assert_eq!(
            suggestions(&entries, None, PREFIX, here, here),
            [RESTORE, BACKUP]
        );
        // after backup, restore
        let after = (BACKUP, Some("/x/project1"), Some(0));
        assert_eq!(
            suggestions(&entries, Some(after), PREFIX, here, here)[0],
            RESTORE
        );
        // a prefix the next command does not start with: the ranked lines
        assert_eq!(
            suggestions(&entries, Some(last), "bash scripts/db.sh r", here, here),
            [RESTORE]
        );
    }

    #[test]
    fn next_command_needs_the_same_directory_and_exit_code() {
        let entries = routine();
        let here = Some("/x/project1");
        for last in [
            (PREV, Some("/x"), Some(1)),
            (PREV, Some("/y"), Some(0)),
            (PREV, None, Some(0)),
            ("cd other", Some("/x"), Some(0)),
        ] {
            assert!(next_after(&entries, last, PREFIX).is_empty(), "{last:?}");
            assert_eq!(
                suggestions(&entries, Some(last), PREFIX, here, here)[0],
                RESTORE,
                "{last:?}"
            );
        }
    }

    #[test]
    fn next_command_comes_from_the_same_session_only() {
        // two panes writing alternately: b's restore lands between a's
        // `cd project1` and a's backup
        let entries = vec![
            run(PREV, "/x", "a", 1, 0),
            run(RESTORE, "/x/project1", "b", 1, 0),
            run(PREV, "/x", "b", 2, 0),
            run(BACKUP, "/x/project1", "a", 2, 0),
            run("ls", "/x/project1", "a", 3, 0),
            run(BACKUP, "/x/project1", "b", 3, 0),
        ];
        let last = (PREV, Some("/x"), Some(0));
        assert_eq!(next_after(&entries, last, PREFIX), [BACKUP]);
        // the most frequent first, then the newer
        let mut entries = entries;
        entries.extend([
            run(PREV, "/x", "c", 1, 0),
            run(RESTORE, "/x/project1", "c", 2, 0),
        ]);
        assert_eq!(next_after(&entries, last, PREFIX), [BACKUP, RESTORE]);
        // an entry without a session (an older Rift's) gives no next command
        let old: Vec<Entry> = routine()
            .into_iter()
            .map(|mut e| {
                e.session = None;
                e.seq = None;
                e
            })
            .collect();
        assert!(next_after(&old, last, PREFIX).is_empty());
    }

    #[test]
    fn files_with_and_without_sessions_read_both_ways() {
        // a file an older Rift wrote: no session or seq; still ranked by
        // directory
        let dir = scratch("compat");
        let p = dir.join("history.json");
        let old = r#"{"version":1,"entries":[
            {"command":"pnpm dev","cwd":"/p/a","repo":"/p/a","finished":1,"exit":0},
            {"command":"pnpm dev:web","cwd":"/p/b","repo":"/p/b","finished":2,"exit":0}]}"#;
        std::fs::write(&p, old).unwrap();
        let entries = load_from(&p);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].session, None);
        let here = Some("/p/a");
        assert_eq!(
            suggestions(&entries, Some(("ls", here, Some(0))), "pnpm de", here, here),
            ["pnpm dev", "pnpm dev:web"]
        );
        // a file this one writes: the same version, and the shape an older
        // Rift (0.21.0) reads — its Entry ignores unknown fields
        assert!(save_to(&p, routine()));
        assert_eq!(load_from(&p), routine());
        #[derive(Deserialize)]
        struct OldEntry {
            command: String,
            #[allow(dead_code)]
            cwd: Option<String>,
            #[serde(default)]
            #[allow(dead_code)]
            repo: Option<String>,
            #[allow(dead_code)]
            finished: u64,
            #[allow(dead_code)]
            exit: Option<i32>,
        }
        #[derive(Deserialize)]
        struct OldFile {
            version: u32,
            entries: Vec<OldEntry>,
        }
        let body = std::fs::read(&p).unwrap();
        assert!(body.len() <= MAX_FILE_BYTES);
        let back: OldFile = serde_json::from_slice(&body).unwrap();
        assert_eq!(back.version, 1);
        assert_eq!(back.entries.len(), 9);
        assert_eq!(back.entries[1].command, BACKUP);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn same_directory_wins_over_newer_global_history() {
        let entries = vec![
            e("pnpm dev", "/p/a", "/p/a", 100),
            e("pnpm dev:web", "/p/b", "/p/b", 200),
        ];
        let here = Some("/p/a");
        assert_eq!(
            ranked_matches(&entries, "pnpm de", here, here),
            ["pnpm dev", "pnpm dev:web"]
        );
        // from b, its own command first
        let b = Some("/p/b");
        assert_eq!(
            ranked_matches(&entries, "pnpm de", b, b),
            ["pnpm dev:web", "pnpm dev"]
        );
        // elsewhere: newest first
        let c = Some("/p/c");
        assert_eq!(
            ranked_matches(&entries, "pnpm de", c, c),
            ["pnpm dev:web", "pnpm dev"]
        );
    }

    #[test]
    fn tiers_directory_then_repository_then_everything_else() {
        let entries = vec![
            e("make here", "/r/src", "/r", 1),
            e("make repo", "/r/docs", "/r", 2),
            e("make other", "/x", "/x", 3),
            e("make repo2", "/r", "/r", 4),
            e("make here2", "/r/src", "/r", 5),
        ];
        assert_eq!(
            ranked_matches(&entries, "make", Some("/r/src"), Some("/r")),
            [
                "make here2",
                "make here",
                "make repo2",
                "make repo",
                "make other"
            ]
        );
        // a duplicate keeps its best place; equal prefix and multiline are out
        let entries = vec![
            e("ls -la", "/x", "/x", 9),
            e("ls -la", "/a", "/a", 1),
            e("ls", "/a", "/a", 2),
            e("ls\nx", "/a", "/a", 3),
        ];
        assert_eq!(
            ranked_matches(&entries, "ls", Some("/a"), Some("/a")),
            ["ls -la"]
        );
        assert_eq!(tier(None, None, None, None), 2);
    }

    #[test]
    fn persistence_round_trip() {
        let dir = scratch("rt");
        let p = dir.join("rift/history.json");
        let entries = vec![e("pnpm dev", "/p/a", "/p/a", 1), e("ls", "/p/b", "/p/b", 2)];
        assert!(save_to(&p, entries.clone()));
        assert_eq!(load_from(&p), entries);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let left: Vec<_> = std::fs::read_dir(dir.join("rift"))
            .unwrap()
            .flatten()
            .map(|d| d.file_name())
            .collect();
        assert_eq!(left, ["history.json"], "no temporary file left");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_file_is_moved_aside_never_written_over() {
        let dir = scratch("aside");
        let p = dir.join("history.json");
        // a newer Rift's file: this one cannot read it
        let future = br#"{"version":2,"entries":[{"command":"keep me"}]}"#;
        std::fs::write(&p, future).unwrap();
        assert!(append_to(&p, vec![e("ls", "/a", "/a", 1)]));
        assert_eq!(std::fs::read(p.with_extension("json.bad")).unwrap(), future);
        assert_eq!(load_from(&p), [e("ls", "/a", "/a", 1)]);
        // a readable one is merged into
        assert!(append_to(&p, vec![e("pwd", "/a", "/a", 2)]));
        assert_eq!(load_from(&p).len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_writers_lose_nothing() {
        let dir = scratch("race");
        let p = dir.join("history.json");
        let writers: Vec<_> = (0..4)
            .map(|w| {
                let p = p.clone();
                std::thread::spawn(move || {
                    for i in 0..50 {
                        assert!(append_to(&p, vec![e(&format!("w{w}-{i}"), "/", "/", i)]));
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        assert_eq!(load_from(&p).len(), 200);
        assert!(!p.with_extension("json.bad").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn store_records_and_a_new_store_reads_them_back() {
        let dir = scratch("store");
        let p = dir.join("history.json");
        let wait = std::time::Duration::from_secs(10);
        let s = Store::start(Some(p.clone()), true);
        s.record(
            "pnpm dev".into(),
            Some(dir.display().to_string()),
            5,
            Some(0),
            None,
        );
        assert!(s.flush(wait));
        assert_eq!(s.snapshot().len(), 1);
        let back = load_from(&p);
        assert_eq!(back[0].command, "pnpm dev");
        assert!(back[0].repo.is_some(), "repository resolved");
        // a second process reads it; one that may not read sees only its own
        let reader = Store::start(Some(p.clone()), true);
        assert!(reader.flush(wait));
        assert_eq!(reader.snapshot()[0].command, "pnpm dev");
        let blind = Store::start(Some(p.clone()), false);
        blind.record("ls".into(), None, 6, None, None);
        assert!(blind.flush(wait));
        let commands = |v: &[Entry]| v.iter().map(|e| e.command.clone()).collect::<Vec<_>>();
        assert_eq!(commands(&blind.snapshot()), ["ls"]);
        // the write merged into the file instead of replacing it
        assert_eq!(commands(&load_from(&p)), ["pnpm dev", "ls"]);
        // nowhere to write: kept in memory only
        let memory = Store::start(None, true);
        memory.record("pwd".into(), None, 7, None, None);
        assert!(memory.flush(wait));
        assert_eq!(commands(&memory.snapshot()), ["pwd"]);
        assert_eq!(load_from(&p).len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bounds_keep_the_newest() {
        let many: Vec<Entry> = (0..MAX_ENTRIES + 5)
            .map(|i| e(&format!("c{i}"), "/", "/", i as u64))
            .collect();
        let b = bounded(many);
        assert_eq!(b.len(), MAX_ENTRIES);
        assert_eq!(b.last().unwrap().command, format!("c{}", MAX_ENTRIES + 4));
        assert_eq!(b[0].command, "c5");
        // bytes: entries of ~4 KiB, about 512 fit in 2 MiB
        let big = "x".repeat(MAX_FIELD_BYTES - 10);
        let b = bounded(
            (0..1000)
                .map(|i| e(&format!("{big}{i}"), "/", "/", i))
                .collect(),
        );
        assert!(b.iter().map(size).sum::<usize>() <= MAX_BYTES);
        assert!(b.len() < 1000 && b.len() > 400, "{}", b.len());
        assert!(b.last().unwrap().command.ends_with("999"));
        // an oversized field is dropped, not cut
        let b = bounded(vec![e(&"y".repeat(MAX_FIELD_BYTES + 1), "/", "/", 1)]);
        assert!(b.is_empty());
    }

    #[test]
    fn a_corrupt_or_oversized_file_is_ignored() {
        let dir = scratch("bad");
        let p = dir.join("history.json");
        for body in [
            &b"{broken"[..],
            b"",
            b"[]",
            br#"{"version":2,"entries":[]}"#,
            br#"{"version":1,"entries":[{"command":1}]}"#,
            b"\xff\xfe\x00",
        ] {
            std::fs::write(&p, body).unwrap();
            assert!(load_from(&p).is_empty(), "{body:?}");
        }
        let long = format!(
            r#"{{"version":1,"entries":[{{"command":"{}","cwd":null,"finished":1,"exit":null}}]}}"#,
            "z".repeat(MAX_FIELD_BYTES + 1)
        );
        std::fs::write(&p, long).unwrap();
        assert!(load_from(&p).is_empty());
        let ok =
            r#"{"version":1,"entries":[{"command":"ls","cwd":null,"finished":1,"exit":null}]}"#;
        std::fs::write(&p, ok).unwrap();
        assert_eq!(load_from(&p).len(), 1);
        // a directory, a missing file, an endless stream: nothing
        assert!(load_from(&dir).is_empty());
        assert!(load_from(&dir.join("none.json")).is_empty());
        assert!(load_reader(std::io::repeat(b' ')).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn skips_leading_space_remote_and_secrets() {
        assert!(should_record("pnpm dev", false));
        assert!(!should_record(" pnpm dev", false));
        assert!(!should_record("\tls", false));
        assert!(!should_record("   ", false));
        assert!(!should_record("pnpm dev", true));
        assert!(!should_record("export TOKEN=abcd1234efgh", false));
        assert!(!should_record(
            "curl -H 'Authorization: token ghp_abcdefghijklmnopqrstuvwxyz0123456789'",
            false
        ));
    }
}
