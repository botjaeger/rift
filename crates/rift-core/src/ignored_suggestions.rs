//! Suggestions the user ignored (the ghost text's ✕, Warp's "Ignore this
//! suggestion"): whole command lines the ghost text never offers again,
//! whichever source suggested them (next-command history, Rift's history,
//! the pane's and the shell's history, the first completion candidate).
//! Matching is exact (the whole line, as Warp's ignored suggestions).
//! Running a command takes it off the list (Warp).
//!
//! Kept in `~/.local/share/rift/ignored-suggestions.json`, beside
//! `history.json` and like it: bounded (the newest [`MAX_ENTRIES`] lines
//! and [`MAX_BYTES`] of text; a line over [`MAX_COMMAND_BYTES`] is not
//! kept, though it stays ignored for the run), written atomically (0600)
//! under a lock every Rift process takes, each write merging its changes
//! into the file as it is on disk then; and untrusted on load: one over the
//! bounds, unparsable or of another version is not read, and the next write
//! moves it aside to `ignored-suggestions.json.bad` instead of writing over
//! it.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

pub const VERSION: u32 = 1;
pub const MAX_ENTRIES: usize = 1000;
/// Every line together.
pub const MAX_BYTES: usize = 256 << 10;
/// One line.
pub const MAX_COMMAND_BYTES: usize = 4 << 10;
/// Worst-case JSON escaping of the text plus per-line punctuation.
pub const MAX_FILE_BYTES: usize = MAX_BYTES * 6 + MAX_ENTRIES * 8 + 64;

#[derive(Debug, Serialize, Deserialize)]
struct File {
    version: u32,
    /// Oldest first.
    commands: Vec<String>,
}

/// `ignored-suggestions.json` in Rift's data directory.
pub fn path() -> Option<PathBuf> {
    crate::shell_integration::data_dir().map(|d| d.join("ignored-suggestions.json"))
}

/// The newest lines within the bounds, oldest first, each once (a line
/// ignored again counts as new); a line over [`MAX_COMMAND_BYTES`] is
/// dropped.
pub fn bounded(commands: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut total = 0usize;
    let mut keep = Vec::new();
    for c in commands.into_iter().rev() {
        if c.len() > MAX_COMMAND_BYTES || seen.contains(&c) {
            continue;
        }
        if keep.len() == MAX_ENTRIES || total + c.len() > MAX_BYTES {
            break;
        }
        total += c.len();
        seen.insert(c.clone());
        keep.push(c);
    }
    keep.reverse();
    keep
}

fn valid(f: &File) -> bool {
    f.version == VERSION
        && f.commands.len() <= MAX_ENTRIES
        && f.commands.iter().all(|c| c.len() <= MAX_COMMAND_BYTES)
        && f.commands.iter().map(String::len).sum::<usize>() <= MAX_BYTES
}

enum Loaded {
    Missing,
    /// Over the bounds, unparsable, of another version or not readable.
    Unreadable,
    Commands(Vec<String>),
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
        Some(c) => Loaded::Commands(c),
        None => Loaded::Unreadable,
    }
}

fn load_reader(reader: impl Read) -> Option<Vec<String>> {
    let mut body = Vec::new();
    reader
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .ok()?;
    if body.len() > MAX_FILE_BYTES {
        return None;
    }
    let f: File = serde_json::from_slice(&body).ok()?;
    valid(&f).then_some(f.commands)
}

/// The lines in `path`, oldest first; none when it is missing, over the
/// bounds or corrupt.
pub fn load_from(path: &Path) -> Vec<String> {
    match read_file(path) {
        Loaded::Commands(c) => c,
        Loaded::Missing | Loaded::Unreadable => Vec::new(),
    }
}

/// One edit of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Add(String),
    Remove(String),
    Clear,
}

fn apply(list: &mut Vec<String>, change: &Change) {
    match change {
        Change::Add(c) => {
            list.retain(|x| x != c);
            list.push(c.clone());
        }
        Change::Remove(c) => list.retain(|x| x != c),
        Change::Clear => list.clear(),
    }
}

/// Write `commands` (bounded) to `path`: atomic, private (0600).
pub fn save_to(path: &Path, commands: Vec<String>) -> bool {
    let f = File {
        version: VERSION,
        commands: bounded(commands),
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
    crate::command_history::write_private(path, &body)
}

/// Apply `changes` to the file as it is on disk now, under the lock every
/// Rift process takes, so their lines survive. A file that cannot be read
/// is moved aside to `ignored-suggestions.json.bad` first, never written
/// over.
pub fn update(path: &Path, changes: &[Change]) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let Some(_lock) = crate::command_history::lock(path) else {
        return false;
    };
    let mut all = match read_file(path) {
        Loaded::Missing => Vec::new(),
        Loaded::Commands(c) => c,
        Loaded::Unreadable => {
            if std::fs::rename(path, path.with_extension("json.bad")).is_err() {
                return false;
            }
            log::warn!(
                "ignored suggestions: {} could not be read; moved aside to {}",
                path.display(),
                "ignored-suggestions.json.bad"
            );
            Vec::new()
        }
    };
    for c in changes {
        apply(&mut all, c);
    }
    save_to(path, all)
}

/// The list a process uses: what it read plus its own changes since.
#[derive(Debug, Clone, Default)]
pub struct Ignored {
    /// Oldest first, each once.
    order: Vec<String>,
    set: HashSet<String>,
}

impl Ignored {
    pub fn from_lines(lines: Vec<String>) -> Ignored {
        let mut out = Ignored::default();
        for l in lines {
            out.apply(&Change::Add(l));
        }
        out
    }

    /// Whether `line` (a whole suggested command line) is ignored.
    pub fn contains(&self, line: &str) -> bool {
        self.set.contains(line)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Oldest first.
    pub fn lines(&self) -> &[String] {
        &self.order
    }

    /// In memory a line over [`MAX_COMMAND_BYTES`] stays (for this run);
    /// at most [`MAX_ENTRIES`] lines, the oldest go first.
    fn apply(&mut self, change: &Change) {
        apply(&mut self.order, change);
        if self.order.len() > MAX_ENTRIES {
            self.order.drain(..self.order.len() - MAX_ENTRIES);
        }
        self.set = self.order.iter().cloned().collect();
    }
}

/// The ignored lines a process uses. Cheap to clone; a background thread
/// reads and writes the file, so the caller never touches the disk.
#[derive(Clone, Default)]
pub struct Store {
    list: Arc<Mutex<Arc<Ignored>>>,
    tx: Option<Sender<Msg>>,
}

enum Msg {
    Change(Change),
    /// Answer once everything sent before is written.
    Flush(Sender<()>),
}

impl Store {
    /// Start the store. `read`: the file's lines are ignored too (else
    /// only this run's). `path`: where changes are written (`None`: kept in
    /// memory only).
    pub fn start(path: Option<PathBuf>, read: bool) -> Store {
        let list: Arc<Mutex<Arc<Ignored>>> = Arc::default();
        let (tx, rx) = channel::<Msg>();
        let shared = list.clone();
        let spawned = std::thread::Builder::new()
            .name("ignored-suggestions".into())
            .spawn(move || {
                let mut early = Vec::new();
                if read && let Some(p) = &path {
                    let loaded = load_from(p);
                    // changes are sent while the list is locked: those made
                    // before the file was read are all queued now, and go
                    // over what it holds
                    let mut cur = shared.lock().unwrap_or_else(|e| e.into_inner());
                    early.extend(rx.try_iter());
                    let mut fresh = Ignored::from_lines(loaded);
                    for m in &early {
                        if let Msg::Change(c) = m {
                            fresh.apply(c);
                        }
                    }
                    *cur = Arc::new(fresh);
                }
                let mut pending = early.into_iter();
                loop {
                    let first = match pending.next() {
                        Some(m) => m,
                        None => match rx.recv() {
                            Ok(m) => m,
                            Err(_) => break,
                        },
                    };
                    let mut changes = Vec::new();
                    let mut waiting = Vec::new();
                    for msg in std::iter::once(first)
                        .chain(pending.by_ref())
                        .chain(rx.try_iter())
                    {
                        match msg {
                            Msg::Change(c) => changes.push(c),
                            Msg::Flush(done) => waiting.push(done),
                        }
                    }
                    if !changes.is_empty()
                        && let Some(p) = &path
                        && !update(p, &changes)
                    {
                        log::warn!("ignored suggestions: could not write {}", p.display());
                    }
                    for done in waiting {
                        let _ = done.send(());
                    }
                }
            });
        Store {
            list,
            tx: spawned.is_ok().then_some(tx),
        }
    }

    /// Change the list now (the file follows off this thread).
    pub fn change(&self, change: Change) {
        let mut cur = self.list.lock().unwrap_or_else(|e| e.into_inner());
        Arc::make_mut(&mut *cur).apply(&change);
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Change(change));
        }
    }

    /// Never suggest `line` again.
    pub fn add(&self, line: &str) {
        self.change(Change::Add(line.to_string()));
    }

    /// Suggest `line` again (a no-op, and no write, when it is not
    /// ignored).
    pub fn remove(&self, line: &str) {
        if self.snapshot().contains(line) {
            self.change(Change::Remove(line.to_string()));
        }
    }

    /// Suggest every line again; the number that were ignored.
    pub fn clear(&self) -> usize {
        let n = self.snapshot().len();
        self.change(Change::Clear);
        n
    }

    /// The list now.
    pub fn snapshot(&self) -> Arc<Ignored> {
        self.list.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Wait, at most `timeout`, until every change so far is written (at
    /// quit). False when it was not.
    pub fn flush(&self, timeout: std::time::Duration) -> bool {
        let Some(tx) = &self.tx else { return true };
        let (done, wait) = channel();
        tx.send(Msg::Flush(done)).is_ok() && wait.recv_timeout(timeout).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rift-ignored-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn round_trip_is_private_and_atomic() {
        let dir = scratch("rt");
        let p = dir.join("rift/ignored-suggestions.json");
        assert!(save_to(&p, lines(&["git push --force", "rm -rf build"])));
        assert_eq!(load_from(&p), lines(&["git push --force", "rm -rf build"]));
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
        assert_eq!(left, ["ignored-suggestions.json"], "no temporary file left");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bounds_keep_the_newest_once() {
        let many: Vec<String> = (0..MAX_ENTRIES + 5).map(|i| format!("c{i}")).collect();
        let b = bounded(many);
        assert_eq!(b.len(), MAX_ENTRIES);
        assert_eq!(b[0], "c5");
        assert_eq!(b.last().unwrap(), &format!("c{}", MAX_ENTRIES + 4));
        // bytes: lines of ~4 KiB, about 64 fit in 256 KiB
        let big = "x".repeat(MAX_COMMAND_BYTES - 10);
        let b = bounded((0..200).map(|i| format!("{big}{i}")).collect());
        assert!(b.iter().map(String::len).sum::<usize>() <= MAX_BYTES);
        assert!(b.len() < 200 && b.len() > 50, "{}", b.len());
        assert!(b.last().unwrap().ends_with("199"));
        // a line over 4 KiB is not kept; a repeated one counts as new
        let b = bounded(vec!["y".repeat(MAX_COMMAND_BYTES + 1)]);
        assert!(b.is_empty());
        assert_eq!(bounded(lines(&["a", "b", "a"])), lines(&["b", "a"]));
    }

    #[test]
    fn a_corrupt_oversized_or_other_version_file_is_not_read_and_moved_aside() {
        let dir = scratch("bad");
        let p = dir.join("ignored-suggestions.json");
        for body in [
            &b"{broken"[..],
            b"",
            b"[]",
            br#"{"version":2,"commands":["keep me"]}"#,
            br#"{"version":1,"commands":[1]}"#,
            b"\xff\xfe\x00",
        ] {
            std::fs::write(&p, body).unwrap();
            assert!(load_from(&p).is_empty(), "{body:?}");
        }
        let long = format!(
            r#"{{"version":1,"commands":["{}"]}}"#,
            "z".repeat(MAX_COMMAND_BYTES + 1)
        );
        std::fs::write(&p, &long).unwrap();
        assert!(load_from(&p).is_empty());
        let many = format!(
            r#"{{"version":1,"commands":[{}]}}"#,
            (0..=MAX_ENTRIES)
                .map(|i| format!("\"c{i}\""))
                .collect::<Vec<_>>()
                .join(",")
        );
        std::fs::write(&p, &many).unwrap();
        assert!(load_from(&p).is_empty(), "too many lines");
        assert!(load_reader(std::io::repeat(b' ')).is_none(), "endless");
        // the next write moves it aside instead of writing over it
        let future = br#"{"version":2,"commands":["keep me"]}"#;
        std::fs::write(&p, future).unwrap();
        assert!(update(&p, &[Change::Add("ls".into())]));
        assert_eq!(std::fs::read(p.with_extension("json.bad")).unwrap(), future);
        assert_eq!(load_from(&p), lines(&["ls"]));
        // a readable one is merged into
        assert!(update(&p, &[Change::Add("pwd".into())]));
        assert_eq!(load_from(&p), lines(&["ls", "pwd"]));
        assert!(load_from(&dir).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_writers_lose_nothing() {
        let dir = scratch("race");
        let p = dir.join("ignored-suggestions.json");
        let writers: Vec<_> = (0..4)
            .map(|w| {
                let p = p.clone();
                std::thread::spawn(move || {
                    for i in 0..50 {
                        assert!(update(&p, &[Change::Add(format!("w{w}-{i}"))]));
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

    #[cfg(unix)]
    #[test]
    fn a_failed_writer_lock_leaves_ignored_suggestions_untouched() {
        let dir = scratch("lock-unavailable");
        let p = dir.join("ignored-suggestions.json");
        assert!(save_to(&p, lines(&["keep me"])));
        std::fs::create_dir(p.with_extension("json.lock")).unwrap();
        assert!(!update(&p, &[Change::Add("new".into())]));
        assert_eq!(load_from(&p), lines(&["keep me"]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn store_changes_merge_into_the_file_and_a_new_store_reads_them() {
        let dir = scratch("store");
        let p = dir.join("ignored-suggestions.json");
        let wait = std::time::Duration::from_secs(10);
        let a = Store::start(Some(p.clone()), true);
        a.add("git push --force");
        a.add("make deploy");
        // in memory at once
        assert!(a.snapshot().contains("make deploy"));
        assert!(!a.snapshot().contains("make"), "exact lines only");
        assert!(a.flush(wait));
        // another process: its changes merge into the file
        let b = Store::start(Some(p.clone()), true);
        assert!(b.flush(wait));
        assert!(b.snapshot().contains("git push --force"));
        b.add("ls -la");
        b.remove("make deploy");
        assert!(b.flush(wait));
        assert_eq!(load_from(&p), lines(&["git push --force", "ls -la"]));
        // a's next change keeps b's lines on disk
        a.add("pwd");
        assert!(a.flush(wait));
        assert_eq!(load_from(&p), lines(&["git push --force", "ls -la", "pwd"]));
        // removing a line that is not there writes nothing
        let before = std::fs::metadata(&p).unwrap().modified().unwrap();
        b.remove("never ignored");
        assert!(b.flush(wait));
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), before);
        // clear empties the file
        assert_eq!(a.clear(), 3);
        assert!(a.snapshot().is_empty());
        assert!(a.flush(wait));
        assert!(load_from(&p).is_empty());
        // one that may not read, or has nowhere to write, keeps its own
        save_to(&p, lines(&["old"]));
        let blind = Store::start(Some(p.clone()), false);
        assert!(blind.flush(wait));
        assert!(!blind.snapshot().contains("old"));
        let memory = Store::start(None, true);
        memory.add("only here");
        assert!(memory.flush(wait));
        assert!(memory.snapshot().contains("only here"));
        assert_eq!(load_from(&p), lines(&["old"]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_long_line_is_ignored_for_the_run_but_not_written() {
        let dir = scratch("long");
        let p = dir.join("ignored-suggestions.json");
        let s = Store::start(Some(p.clone()), true);
        let long = "x".repeat(MAX_COMMAND_BYTES + 1);
        s.add(&long);
        s.add("ls");
        assert!(s.flush(std::time::Duration::from_secs(10)));
        assert!(s.snapshot().contains(&long));
        assert_eq!(load_from(&p), lines(&["ls"]));
        let _ = std::fs::remove_dir_all(dir);
    }
}
