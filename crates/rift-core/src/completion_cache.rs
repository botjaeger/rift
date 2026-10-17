//! Caches for the completion providers, which run on every keystroke.
//!
//! A cached value remembers every file and directory it was read from with
//! that path's modification time, size and inode ([`Stamp`]); the next
//! request compares them (one `stat` each) and reads again when any differs.
//! A value read within [`RACY`] of one of its paths' modification time is
//! never reused: a filesystem with coarse timestamps could otherwise hide a
//! second change made in the same tick (git's "racy" rule). Each cache keeps
//! a small fixed number of values and drops the least recently used.

use std::collections::HashMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

/// A path changed this recently, relative to when it was read, is read again
/// on the next request.
pub(crate) const RACY: Duration = Duration::from_secs(2);

/// Version of a path: modification time, size and inode; `None` when it does
/// not exist (its creation is a change too).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp(Option<(SystemTime, u64, u64)>);

impl Stamp {
    pub(crate) fn of(path: &Path) -> Stamp {
        use std::os::unix::fs::MetadataExt;
        Stamp(std::fs::metadata(path).ok().map(|m| {
            (
                m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                m.len(),
                m.ino(),
            )
        }))
    }

    /// Changed too close to `read_at` to tell a later change apart.
    pub(crate) fn racy(&self, read_at: SystemTime) -> bool {
        self.0.is_some_and(|(mtime, _, _)| mtime + RACY > read_at)
    }
}

struct Entry<V> {
    deps: Vec<(PathBuf, Stamp)>,
    /// No dependency was racy when it was read.
    trusted: bool,
    used: u64,
    value: Arc<V>,
}

/// At most `cap` values by key, each valid while its dependencies' stamps
/// are unchanged.
pub(crate) struct Lru<K, V> {
    cap: usize,
    tick: u64,
    map: HashMap<K, Entry<V>>,
}

impl<K: Hash + Eq + Clone, V> Lru<K, V> {
    pub(crate) fn new(cap: usize) -> Self {
        Lru {
            cap: cap.max(1),
            tick: 0,
            map: HashMap::new(),
        }
    }

    /// The value for `key`: the cached one while every path it was read from
    /// is unchanged, else `load()`'s (the value and the paths it read).
    pub(crate) fn get(&mut self, key: &K, load: impl FnOnce() -> (V, Vec<PathBuf>)) -> Arc<V> {
        self.tick += 1;
        if let Some(e) = self.map.get_mut(key)
            && e.trusted
            && e.deps.iter().all(|(p, s)| Stamp::of(p) == *s)
        {
            e.used = self.tick;
            return e.value.clone();
        }
        let read_at = SystemTime::now();
        let (value, paths) = load();
        // stamped after the read: a change during it is newer than `read_at`,
        // so it is racy and the value is read again next time
        let deps: Vec<(PathBuf, Stamp)> = paths
            .into_iter()
            .map(|p| {
                let s = Stamp::of(&p);
                (p, s)
            })
            .collect();
        let trusted = !deps.iter().any(|(_, s)| s.racy(read_at));
        if !self.map.contains_key(key)
            && self.map.len() >= self.cap
            && let Some(old) = self
                .map
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
        {
            self.map.remove(&old);
        }
        let value = Arc::new(value);
        self.map.insert(
            key.clone(),
            Entry {
                deps,
                trusted,
                used: self.tick,
                value: value.clone(),
            },
        );
        value
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }
}

/// [`Lru::get`] on a shared cache (a poisoned lock is still usable: entries
/// are only replaced whole).
pub(crate) fn cached<K: Hash + Eq + Clone, V>(
    cache: &Mutex<Lru<K, V>>,
    key: &K,
    load: impl FnOnce() -> (V, Vec<PathBuf>),
) -> Arc<V> {
    cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key, load)
}

/// One directory entry for path completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirItem {
    pub(crate) name: String,
    /// A directory, or a symlink to one.
    pub(crate) is_dir: bool,
}

/// Entries kept per directory listing.
const MAX_DIR_ITEMS: usize = 50_000;
/// Directory listings kept.
const DIR_CACHE: usize = 16;

static DIRS: std::sync::LazyLock<Mutex<Lru<PathBuf, Vec<DirItem>>>> =
    std::sync::LazyLock::new(|| Mutex::new(Lru::new(DIR_CACHE)));

/// `dir`'s entries sorted by name (empty when unreadable), cached until the
/// directory's own stamp changes (an entry added, removed or renamed).
/// Types come from the directory entry itself; only symlinks are `stat`ed,
/// to tell a link to a directory.
pub(crate) fn dir_items(dir: &Path) -> Arc<Vec<DirItem>> {
    cached(&DIRS, &dir.to_path_buf(), || {
        let mut items = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten().take(MAX_DIR_ITEMS) {
                let is_dir = match e.file_type() {
                    Ok(t) if t.is_symlink() => {
                        std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir())
                    }
                    Ok(t) => t.is_dir(),
                    Err(_) => false,
                };
                items.push(DirItem {
                    name: e.file_name().to_string_lossy().into_owned(),
                    is_dir,
                });
            }
        }
        items.sort_by(|a, b| a.name.cmp(&b.name));
        (items, vec![dir.to_path_buf()])
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Duration;

    /// Set `path`'s modification time `secs_ago` seconds in the past, out of
    /// the racy window, so the cache trusts what it read.
    pub(crate) fn age(path: &Path, secs_ago: u64) {
        let t = SystemTime::now() - Duration::from_secs(secs_ago);
        std::fs::File::open(path).unwrap().set_modified(t).unwrap();
    }

    pub(crate) fn scratch(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rift-cache-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(lru: &mut Lru<PathBuf, String>, f: &Path) -> String {
        let fc = f.to_path_buf();
        (*lru.get(&fc, || {
            (
                std::fs::read_to_string(f).unwrap_or_default(),
                vec![fc.clone()],
            )
        }))
        .clone()
    }

    #[test]
    fn a_changed_file_is_read_again_an_unchanged_one_is_not() {
        let dir = scratch("lru");
        let f = dir.join("a");
        std::fs::write(&f, "one").unwrap();
        age(&f, 100);
        let old = std::fs::metadata(&f).unwrap().modified().unwrap();
        let mut lru = Lru::new(4);
        assert_eq!(read(&mut lru, &f), "one");
        // same size and mtime restored: the stamp is unchanged, so the
        // cached value is served (it really is a cache)
        std::fs::write(&f, "two").unwrap();
        std::fs::File::open(&f).unwrap().set_modified(old).unwrap();
        assert_eq!(read(&mut lru, &f), "one");
        // a new mtime: read again
        age(&f, 50);
        assert_eq!(read(&mut lru, &f), "two");
        // a new size alone: read again
        std::fs::write(&f, "three").unwrap();
        age(&f, 50);
        assert_eq!(read(&mut lru, &f), "three");
        // removed: read again
        std::fs::remove_file(&f).unwrap();
        assert_eq!(read(&mut lru, &f), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_racy_read_is_never_reused() {
        let dir = scratch("racy");
        let f = dir.join("a");
        std::fs::write(&f, "one").unwrap();
        let mut lru = Lru::new(4);
        assert_eq!(read(&mut lru, &f), "one");
        // written again within the same timestamp tick: same size, and
        // the mtime may well be equal on a coarse filesystem
        let m = std::fs::metadata(&f).unwrap().modified().unwrap();
        std::fs::write(&f, "two").unwrap();
        std::fs::File::open(&f).unwrap().set_modified(m).unwrap();
        assert_eq!(read(&mut lru, &f), "two");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_least_recently_used_value_is_dropped() {
        let dir = scratch("cap");
        let mut lru = Lru::new(2);
        let files: Vec<PathBuf> = (0..3).map(|i| dir.join(format!("f{i}"))).collect();
        for f in &files {
            std::fs::write(f, "x").unwrap();
            age(f, 100);
        }
        read(&mut lru, &files[0]);
        read(&mut lru, &files[1]);
        read(&mut lru, &files[0]); // f1 is now the oldest
        read(&mut lru, &files[2]);
        assert_eq!(lru.len(), 2);
        assert!(lru.map.contains_key(&files[0]));
        assert!(!lru.map.contains_key(&files[1]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dir_listing_follows_entries_added_and_removed() {
        let dir = scratch("dir");
        std::fs::write(dir.join("a.txt"), "").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::os::unix::fs::symlink(dir.join("sub"), dir.join("link")).unwrap();
        age(&dir, 100);
        let names = |d: &Path| -> Vec<(String, bool)> {
            dir_items(d)
                .iter()
                .map(|i| (i.name.clone(), i.is_dir))
                .collect()
        };
        assert_eq!(
            names(&dir),
            [
                ("a.txt".into(), false),
                ("link".into(), true),
                ("sub".into(), true)
            ]
        );
        std::fs::write(dir.join("b.txt"), "").unwrap();
        age(&dir, 90);
        assert!(names(&dir).contains(&("b.txt".into(), false)));
        std::fs::remove_file(dir.join("a.txt")).unwrap();
        age(&dir, 80);
        assert!(!names(&dir).iter().any(|(n, _)| n == "a.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
