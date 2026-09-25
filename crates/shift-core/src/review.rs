//! Code review model: the repository's staged and unstaged changes as files,
//! hunks and lines (git2), plus the writes the review panel offers — stage,
//! unstage and revert a whole file or one hunk.
//!
//! Hunk operations re-diff at the moment of the write and pick the hunk by
//! its line ranges and content, so a panel that is a refresh behind cannot
//! apply a different hunk than the one the user clicked. A changed hunk makes
//! the write fail and the panel reload.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use git2::{
    ApplyLocation, ApplyOptions, Delta, Diff, DiffOptions, Patch, Repository,
    build::CheckoutBuilder,
};

/// Lines kept per file; the rest of a huge diff is summarized.
pub const MAX_LINES_PER_FILE: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    /// Without the trailing newline.
    pub text: String,
}

/// A hunk's line ranges and content: its identity for stage/unstage/revert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HunkId {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub content_hash: u64,
}

impl HunkId {
    fn swapped(self) -> HunkId {
        HunkId {
            old_start: self.new_start,
            old_lines: self.new_lines,
            new_start: self.old_start,
            new_lines: self.old_lines,
            content_hash: self.content_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub id: HunkId,
    /// `@@ -a,b +c,d @@ context`
    pub header: String,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// Repository-relative, `/`-separated.
    pub path: String,
    /// Previous path of a rename.
    pub old_path: Option<String>,
    /// `M` modified, `A` added, `D` deleted, `R` renamed, `T` type change,
    /// `?` untracked, `C` conflicted.
    pub status: char,
    pub added: usize,
    pub removed: usize,
    pub binary: bool,
    /// Lines past `MAX_LINES_PER_FILE` were dropped.
    pub truncated: bool,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Review {
    /// Working directory of the repository.
    pub root: PathBuf,
    pub branch: String,
    /// HEAD → index.
    pub staged: Vec<FileDiff>,
    /// Index → working tree, untracked files included.
    pub unstaged: Vec<FileDiff>,
}

impl Review {
    /// Status letter per changed path (unstaged wins over staged).
    pub fn statuses(&self) -> HashMap<String, char> {
        let mut out = HashMap::new();
        for f in &self.staged {
            out.insert(f.path.clone(), f.status);
        }
        for f in &self.unstaged {
            out.insert(f.path.clone(), f.status);
        }
        out
    }

    /// Every directory (repository-relative) that contains a change.
    pub fn dirty_dirs(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for f in self.staged.iter().chain(&self.unstaged) {
            let mut path = f.path.as_str();
            while let Some((parent, _)) = path.rsplit_once('/') {
                if !out.insert(parent.to_string()) {
                    break;
                }
                path = parent;
            }
        }
        out
    }
}

fn err(e: git2::Error) -> String {
    e.message().to_string()
}

fn head_tree(repo: &Repository) -> Option<git2::Tree<'_>> {
    repo.head().ok()?.peel_to_tree().ok()
}

fn branch_name(repo: &Repository) -> String {
    match repo.head() {
        Ok(head) if head.is_branch() => head.shorthand().unwrap_or("HEAD").to_string(),
        Ok(head) => head
            .target()
            .map(|oid| format!("{:.7}", oid))
            .unwrap_or_else(|| "HEAD".into()),
        Err(_) => "(no commits)".into(),
    }
}

fn unstaged_options() -> DiffOptions {
    let mut opts = DiffOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true)
        .include_typechange(true);
    opts
}

/// Load the review of the repository containing `dir`. `None` outside a
/// repository or in a bare one.
pub fn load(dir: &Path) -> Option<Result<Review, String>> {
    let repo = Repository::discover(dir).ok()?;
    let root = repo.workdir()?.to_path_buf();
    Some(load_repo(&repo, root))
}

fn load_repo(repo: &Repository, root: PathBuf) -> Result<Review, String> {
    let index = repo.index().map_err(err)?;
    let tree = head_tree(repo);
    let staged = repo
        .diff_tree_to_index(tree.as_ref(), Some(&index), None)
        .map_err(err)?;
    let unstaged = repo
        .diff_index_to_workdir(Some(&index), Some(&mut unstaged_options()))
        .map_err(err)?;
    Ok(Review {
        root,
        branch: branch_name(repo),
        staged: collect(&staged)?,
        unstaged: collect(&unstaged)?,
    })
}

fn status_char(delta: Delta) -> char {
    match delta {
        Delta::Added => 'A',
        Delta::Deleted => 'D',
        Delta::Renamed => 'R',
        Delta::Copied => 'C',
        Delta::Typechange => 'T',
        Delta::Untracked => '?',
        Delta::Conflicted => 'U',
        _ => 'M',
    }
}

fn path_str(p: Option<&Path>) -> Option<String> {
    p.map(|p| p.to_string_lossy().replace('\\', "/"))
}

fn collect(diff: &Diff<'_>) -> Result<Vec<FileDiff>, String> {
    let mut files = Vec::new();
    for idx in 0..diff.deltas().len() {
        let Some(delta) = diff.get_delta(idx) else {
            continue;
        };
        let new_path = path_str(delta.new_file().path());
        let old_path = path_str(delta.old_file().path());
        let path = new_path.clone().or(old_path.clone()).unwrap_or_default();
        let mut file = FileDiff {
            old_path: (delta.status() == Delta::Renamed)
                .then_some(old_path)
                .flatten(),
            path,
            status: status_char(delta.status()),
            added: 0,
            removed: 0,
            binary: delta.flags().is_binary(),
            truncated: false,
            hunks: Vec::new(),
        };
        if let Some(patch) = Patch::from_diff(diff, idx).map_err(err)? {
            file.binary |= patch.delta().flags().is_binary();
            let (_, added, removed) = patch.line_stats().map_err(err)?;
            file.added = added;
            file.removed = removed;
            let mut kept = 0usize;
            for h in 0..patch.num_hunks() {
                let (hunk, n) = patch.hunk(h).map_err(err)?;
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                let mut out = Hunk {
                    id: HunkId {
                        old_start: hunk.old_start(),
                        old_lines: hunk.old_lines(),
                        new_start: hunk.new_start(),
                        new_lines: hunk.new_lines(),
                        content_hash: 0,
                    },
                    header: String::from_utf8_lossy(hunk.header())
                        .trim_end()
                        .to_string(),
                    lines: Vec::with_capacity(n),
                };
                for l in 0..n {
                    let line = patch.line_in_hunk(h, l).map_err(err)?;
                    line.origin().hash(&mut hasher);
                    line.content().hash(&mut hasher);
                    let kind = match line.origin() {
                        '+' => LineKind::Added,
                        '-' => LineKind::Removed,
                        ' ' => LineKind::Context,
                        _ => continue, // "\ No newline at end of file" markers
                    };
                    if kept >= MAX_LINES_PER_FILE {
                        file.truncated = true;
                        continue;
                    }
                    kept += 1;
                    let text = String::from_utf8_lossy(line.content());
                    out.lines.push(Line {
                        kind,
                        old_no: line.old_lineno(),
                        new_no: line.new_lineno(),
                        text: text.trim_end_matches(['\n', '\r']).to_string(),
                    });
                }
                out.id.content_hash = hasher.finish();
                file.hunks.push(out);
                if file.truncated {
                    break;
                }
            }
        }
        files.push(file);
    }
    Ok(files)
}

/// A review-panel write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Unstaged → index.
    Stage,
    /// Index → back to HEAD.
    Unstage,
    /// Discard unstaged changes in the working tree (destructive).
    Revert,
}

/// Apply `op` to `path` (repository-relative), to one hunk when `hunk` is
/// set, else to the whole file.
pub fn apply(root: &Path, op: Op, path: &str, hunk: Option<HunkId>) -> Result<(), String> {
    let repo = Repository::open(root).map_err(err)?;
    match hunk {
        None => apply_file(&repo, op, path),
        Some(id) => apply_hunk(&repo, op, path, id),
    }
}

fn apply_file(repo: &Repository, op: Op, path: &str) -> Result<(), String> {
    let root = repo.workdir().ok_or("bare repository")?.to_path_buf();
    let rel = Path::new(path);
    match op {
        Op::Stage => {
            let mut index = repo.index().map_err(err)?;
            if root.join(rel).symlink_metadata().is_ok() {
                index.add_path(rel).map_err(err)?;
            } else {
                index.remove_path(rel).map_err(err)?;
            }
            index.write().map_err(err)
        }
        Op::Unstage => match repo.head().ok().and_then(|h| h.peel_to_commit().ok()) {
            Some(commit) => repo
                .reset_default(Some(commit.as_object()), [path])
                .map_err(err),
            // unborn HEAD: unstaging drops the entry from the index
            None => {
                let mut index = repo.index().map_err(err)?;
                index.remove_path(rel).map_err(err)?;
                index.write().map_err(err)
            }
        },
        Op::Revert => {
            let tracked = repo.index().map_err(err)?.get_path(rel, 0).is_some();
            if tracked {
                let mut checkout = CheckoutBuilder::new();
                checkout.force().path(path);
                repo.checkout_index(None, Some(&mut checkout)).map_err(err)
            } else {
                // untracked: reverting means deleting it
                let full = root.join(rel);
                if full.is_dir() {
                    std::fs::remove_dir_all(&full)
                } else {
                    std::fs::remove_file(&full)
                }
                .map_err(|e| e.to_string())
            }
        }
    }
}

fn apply_hunk(repo: &Repository, op: Op, path: &str, id: HunkId) -> Result<(), String> {
    let index = repo.index().map_err(err)?;
    let fresh = match op {
        Op::Stage | Op::Revert => {
            repo.diff_index_to_workdir(Some(&index), Some(&mut unstaged_options()))
        }
        Op::Unstage => {
            let tree = head_tree(repo);
            repo.diff_tree_to_index(tree.as_ref(), Some(&index), None)
        }
    }
    .map_err(err)?;
    if !collect(&fresh)?
        .iter()
        .any(|file| file.path == path && file.hunks.iter().any(|hunk| hunk.id == id))
    {
        return Err("the change moved; refreshed".into());
    }
    // the diff whose application performs the op, the hunk id as it appears
    // in that diff, and where it applies
    let (diff, want, location) = match op {
        Op::Stage => (
            repo.diff_index_to_workdir(Some(&index), Some(&mut unstaged_options())),
            id,
            ApplyLocation::Index,
        ),
        Op::Unstage => {
            let tree = head_tree(repo);
            let mut opts = DiffOptions::new();
            opts.reverse(true);
            (
                repo.diff_tree_to_index(tree.as_ref(), Some(&index), Some(&mut opts)),
                id.swapped(),
                ApplyLocation::Index,
            )
        }
        Op::Revert => {
            let mut opts = unstaged_options();
            opts.reverse(true);
            (
                repo.diff_index_to_workdir(Some(&index), Some(&mut opts)),
                id.swapped(),
                ApplyLocation::WorkDir,
            )
        }
    };
    let diff = diff.map_err(err)?;
    let mut found = false;
    let mut opts = ApplyOptions::new();
    opts.delta_callback(|delta| {
        delta.is_some_and(|d| {
            path_str(d.new_file().path()).as_deref() == Some(path)
                || path_str(d.old_file().path()).as_deref() == Some(path)
        })
    });
    opts.hunk_callback(|hunk| {
        let hit = hunk.is_some_and(|h| {
            h.old_start() == want.old_start
                && h.old_lines() == want.old_lines
                && h.new_start() == want.new_start
                && h.new_lines() == want.new_lines
        });
        found |= hit;
        hit
    });
    repo.apply(&diff, location, Some(&mut opts)).map_err(err)?;
    drop(opts);
    if found {
        Ok(())
    } else {
        Err("the change moved; refreshed".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempRepo {
        dir: PathBuf,
        repo: Repository,
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn temp_repo(name: &str) -> TempRepo {
        let dir = std::env::temp_dir().join(format!(
            "shift-review-{name}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let repo = Repository::init(&dir).unwrap();
        TempRepo { dir, repo }
    }

    fn commit_all(t: &TempRepo, msg: &str) {
        let mut index = t.repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = t.repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("t", "t@example.com").unwrap();
        let parent = t.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
        t.repo
            .commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents)
            .unwrap();
    }

    /// 30 numbered lines: edits at 2 and 28 land in separate hunks.
    fn lines(edits: &[(usize, &str)]) -> String {
        (1..=30)
            .map(|n| {
                edits
                    .iter()
                    .find(|(at, _)| *at == n)
                    .map(|(_, s)| s.to_string())
                    .unwrap_or_else(|| format!("line {n}"))
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    fn review(t: &TempRepo) -> Review {
        load(&t.dir).expect("in a repo").expect("loads")
    }

    #[test]
    fn loads_staged_unstaged_and_untracked() {
        let t = temp_repo("load");
        fs::write(t.dir.join("a.txt"), lines(&[])).unwrap();
        commit_all(&t, "init");
        fs::write(
            t.dir.join("a.txt"),
            lines(&[(2, "two"), (28, "twenty-eight")]),
        )
        .unwrap();
        fs::create_dir_all(t.dir.join("src")).unwrap();
        fs::write(t.dir.join("src/new.rs"), "fn main() {}\n").unwrap();

        let r = review(&t);
        assert!(r.staged.is_empty());
        assert_eq!(r.unstaged.len(), 2);
        let a = r.unstaged.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!(a.status, 'M');
        assert_eq!((a.added, a.removed), (2, 2));
        assert_eq!(a.hunks.len(), 2, "edits 26 lines apart are two hunks");
        let removed = a.hunks[0]
            .lines
            .iter()
            .find(|l| l.kind == LineKind::Removed)
            .unwrap();
        assert_eq!((removed.text.as_str(), removed.old_no), ("line 2", Some(2)));
        let new = r.unstaged.iter().find(|f| f.path == "src/new.rs").unwrap();
        assert_eq!(new.status, '?');
        assert_eq!(new.hunks[0].lines[0].text, "fn main() {}");
        assert_eq!(r.statuses().get("src/new.rs"), Some(&'?'));
        assert!(r.dirty_dirs().contains("src"));
    }

    #[test]
    fn stages_and_unstages_one_hunk() {
        let t = temp_repo("hunk");
        fs::write(t.dir.join("a.txt"), lines(&[])).unwrap();
        commit_all(&t, "init");
        fs::write(
            t.dir.join("a.txt"),
            lines(&[(2, "two"), (28, "twenty-eight")]),
        )
        .unwrap();

        let second = review(&t).unstaged[0].hunks[1].id;
        apply(&t.dir, Op::Stage, "a.txt", Some(second)).unwrap();
        let r = review(&t);
        assert_eq!(r.staged[0].hunks.len(), 1);
        assert!(
            r.staged[0].hunks[0]
                .lines
                .iter()
                .any(|l| l.text == "twenty-eight")
        );
        assert_eq!(r.unstaged[0].hunks.len(), 1);
        assert!(r.unstaged[0].hunks[0].lines.iter().any(|l| l.text == "two"));

        let staged = r.staged[0].hunks[0].id;
        apply(&t.dir, Op::Unstage, "a.txt", Some(staged)).unwrap();
        let r = review(&t);
        assert!(r.staged.is_empty());
        assert_eq!(r.unstaged[0].hunks.len(), 2);
        // the working tree was never touched
        assert_eq!(
            fs::read_to_string(t.dir.join("a.txt")).unwrap(),
            lines(&[(2, "two"), (28, "twenty-eight")])
        );
    }

    #[test]
    fn reverts_one_hunk_and_keeps_the_other() {
        let t = temp_repo("revert");
        fs::write(t.dir.join("a.txt"), lines(&[])).unwrap();
        commit_all(&t, "init");
        fs::write(
            t.dir.join("a.txt"),
            lines(&[(2, "two"), (28, "twenty-eight")]),
        )
        .unwrap();

        let first = review(&t).unstaged[0].hunks[0].id;
        apply(&t.dir, Op::Revert, "a.txt", Some(first)).unwrap();
        assert_eq!(
            fs::read_to_string(t.dir.join("a.txt")).unwrap(),
            lines(&[(28, "twenty-eight")])
        );
    }

    #[test]
    fn a_stale_hunk_is_refused() {
        let t = temp_repo("stale");
        fs::write(t.dir.join("a.txt"), lines(&[])).unwrap();
        commit_all(&t, "init");
        fs::write(t.dir.join("a.txt"), lines(&[(2, "two")])).unwrap();
        let old = review(&t).unstaged[0].hunks[0].id;
        // the file changes again before the click lands
        fs::write(t.dir.join("a.txt"), lines(&[(15, "fifteen")])).unwrap();
        assert!(apply(&t.dir, Op::Revert, "a.txt", Some(old)).is_err());
        assert_eq!(
            fs::read_to_string(t.dir.join("a.txt")).unwrap(),
            lines(&[(15, "fifteen")]),
            "nothing was written"
        );
    }

    #[test]
    fn a_stale_hunk_with_the_same_line_range_is_refused() {
        let t = temp_repo("stale-content");
        fs::write(t.dir.join("a.txt"), lines(&[])).unwrap();
        commit_all(&t, "init");
        fs::write(t.dir.join("a.txt"), lines(&[(2, "first edit")])).unwrap();
        let old = review(&t).unstaged[0].hunks[0].id;

        // A second edit at the same line retains the hunk's line ranges.
        fs::write(t.dir.join("a.txt"), lines(&[(2, "different edit")])).unwrap();
        assert!(apply(&t.dir, Op::Revert, "a.txt", Some(old)).is_err());
        assert_eq!(
            fs::read_to_string(t.dir.join("a.txt")).unwrap(),
            lines(&[(2, "different edit")]),
        );
    }

    #[test]
    fn whole_file_stage_unstage_and_revert() {
        let t = temp_repo("file");
        fs::write(t.dir.join("a.txt"), lines(&[])).unwrap();
        commit_all(&t, "init");
        fs::write(t.dir.join("a.txt"), lines(&[(2, "two")])).unwrap();
        fs::write(t.dir.join("b.txt"), "new\n").unwrap();

        apply(&t.dir, Op::Stage, "b.txt", None).unwrap();
        let r = review(&t);
        assert_eq!(r.staged.len(), 1);
        assert_eq!(
            (r.staged[0].path.as_str(), r.staged[0].status),
            ("b.txt", 'A')
        );

        apply(&t.dir, Op::Unstage, "b.txt", None).unwrap();
        assert!(review(&t).staged.is_empty());

        apply(&t.dir, Op::Revert, "a.txt", None).unwrap();
        assert_eq!(fs::read_to_string(t.dir.join("a.txt")).unwrap(), lines(&[]));
        apply(&t.dir, Op::Revert, "b.txt", None).unwrap();
        assert!(!t.dir.join("b.txt").exists());
        assert!(review(&t).unstaged.is_empty());
    }

    #[test]
    fn unborn_head_stages_and_unstages() {
        let t = temp_repo("unborn");
        fs::write(t.dir.join("a.txt"), "a\n").unwrap();
        assert_eq!(review(&t).branch, "(no commits)");
        apply(&t.dir, Op::Stage, "a.txt", None).unwrap();
        assert_eq!(review(&t).staged.len(), 1);
        apply(&t.dir, Op::Unstage, "a.txt", None).unwrap();
        let r = review(&t);
        assert!(r.staged.is_empty());
        assert_eq!(r.unstaged[0].status, '?');
    }

    #[test]
    fn outside_a_repository_is_none() {
        let dir = std::env::temp_dir().join(format!("shift-review-none-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        // temp dirs can sit inside a repo on some machines; only assert when
        // discovery really finds nothing
        if Repository::discover(&dir).is_err() {
            assert!(load(&dir).is_none());
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
