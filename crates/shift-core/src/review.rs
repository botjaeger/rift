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
    /// The part that differs from its paired removed/added line (byte
    /// range into `text`): the panel highlights it, like Warp.
    pub emph: Option<std::ops::Range<usize>>,
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
    /// `MAX_LINES_PER_FILE` cut this hunk's `lines`: the panel does not show
    /// all of it, so hunk operations refuse it (whole-file ones still work).
    pub partial: bool,
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
    /// The new side's lines, for the panel's "N unmodified lines" rows
    /// between hunks. `None` for added, deleted, binary and large files.
    pub source: Option<Vec<String>>,
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
        staged: collect_as(&staged, false, Some((repo, false)))?,
        unstaged: collect_as(&unstaged, false, Some((repo, true)))?,
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
    p.map(|p| {
        let s = p.to_string_lossy();
        // a backslash separates path components only on Windows; on unix it
        // is a legal file-name character
        if cfg!(windows) {
            s.replace('\\', "/")
        } else {
            s.into_owned()
        }
    })
}

/// Files, hunks and lines of `diff`. `reversed` for a diff built with
/// `reverse(true)`: hunk content hashes are computed as the forward diff's,
/// so `HunkId::swapped` of a forward id matches the reversed diff's hunk.
/// `source` (repository, new side is the working tree) loads each modified
/// file's new side for the panel.
fn collect_as(
    diff: &Diff<'_>,
    reversed: bool,
    source: Option<(&Repository, bool)>,
) -> Result<Vec<FileDiff>, String> {
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
            source: None,
        };
        if let Some(patch) = Patch::from_diff(diff, idx).map_err(err)? {
            file.binary |= patch.delta().flags().is_binary();
            let (_, added, removed) = patch.line_stats().map_err(err)?;
            file.added = added;
            file.removed = removed;
            let mut kept = 0usize;
            for h in 0..patch.num_hunks() {
                let (hunk, n) = patch.hunk(h).map_err(err)?;
                // The hash covers each side's text separately (old side:
                // context + removed lines, new side: context + added lines),
                // so it does not depend on how the diff aligned the change
                // and is the same for the diff and its reverse.
                let mut old_side = std::collections::hash_map::DefaultHasher::new();
                let mut new_side = std::collections::hash_map::DefaultHasher::new();
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
                    partial: false,
                };
                for l in 0..n {
                    let line = patch.line_in_hunk(h, l).map_err(err)?;
                    // (on the old side, on the new side) in this diff's terms;
                    // EOF-newline markers count on the side they describe
                    let (old, new) = match line.origin() {
                        ' ' | '=' => (true, true),
                        '-' | '<' => (true, false),
                        '+' | '>' => (false, true),
                        _ => (false, false),
                    };
                    let (old, new) = if reversed { (new, old) } else { (old, new) };
                    let marker = matches!(line.origin(), '=' | '<' | '>');
                    for (side, hasher) in [(old, &mut old_side), (new, &mut new_side)] {
                        if side {
                            marker.hash(hasher);
                            line.content().hash(hasher);
                        }
                    }
                    let kind = match line.origin() {
                        '+' => LineKind::Added,
                        '-' => LineKind::Removed,
                        ' ' => LineKind::Context,
                        _ => continue, // "\ No newline at end of file" markers
                    };
                    if kept >= MAX_LINES_PER_FILE {
                        file.truncated = true;
                        out.partial = true;
                        continue;
                    }
                    kept += 1;
                    let text = String::from_utf8_lossy(line.content());
                    out.lines.push(Line {
                        kind,
                        old_no: line.old_lineno(),
                        new_no: line.new_lineno(),
                        text: text.trim_end_matches(['\n', '\r']).to_string(),
                        emph: None,
                    });
                }
                mark_changes(&mut out.lines);
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                old_side.finish().hash(&mut hasher);
                new_side.finish().hash(&mut hasher);
                out.id.content_hash = hasher.finish();
                file.hunks.push(out);
                if file.truncated {
                    break;
                }
            }
        }
        if let Some((repo, workdir)) = source
            && !file.binary
        {
            file.source = new_side(repo, &delta, workdir);
        }
        files.push(file);
    }
    Ok(files)
}

/// Largest file whose new side is loaded for the "unmodified lines" rows.
const MAX_SOURCE_BYTES: usize = 1 << 20;

/// A modified file's new side as lines: the index blob for a staged diff,
/// the working-tree file for an unstaged one.
fn new_side(repo: &Repository, delta: &git2::DiffDelta<'_>, workdir: bool) -> Option<Vec<String>> {
    if !matches!(
        delta.status(),
        Delta::Modified | Delta::Renamed | Delta::Copied
    ) {
        return None;
    }
    let file = delta.new_file();
    let bytes = if workdir {
        let full = repo.workdir()?.join(file.path()?);
        let meta = full.symlink_metadata().ok()?;
        if !meta.is_file() || meta.len() > MAX_SOURCE_BYTES as u64 {
            return None;
        }
        std::fs::read(full).ok()?
    } else {
        let blob = repo.find_blob(file.id()).ok()?;
        if blob.size() > MAX_SOURCE_BYTES {
            return None;
        }
        blob.content().to_vec()
    };
    if bytes.contains(&0) {
        return None;
    }
    Some(
        String::from_utf8_lossy(&bytes)
            .lines()
            .map(str::to_string)
            .collect(),
    )
}

/// Pair each run of removed lines with the added run right after it, line
/// by line, and mark the part of each pair that differs.
fn mark_changes(lines: &mut [Line]) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Removed {
            i += 1;
            continue;
        }
        let removed = i;
        while i < lines.len() && lines[i].kind == LineKind::Removed {
            i += 1;
        }
        let added = i;
        while i < lines.len() && lines[i].kind == LineKind::Added {
            i += 1;
        }
        // in order, each removed line with the next added line it resembles
        // (an inserted line between them is skipped)
        let mut next = added;
        for a in removed..added {
            let found = (next..i)
                .find_map(|b| changed_span(&lines[a].text, &lines[b].text).map(|spans| (b, spans)));
            if let Some((b, (ra, rb))) = found {
                lines[a].emph = Some(ra);
                lines[b].emph = Some(rb);
                next = b + 1;
            }
        }
    }
}

/// The differing middle of two lines (byte ranges between their common
/// prefix and suffix), or `None` when they share under a quarter of the
/// longer one: a rewritten line is highlighted whole by its row tint.
fn changed_span(a: &str, b: &str) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let prefix = a
        .char_indices()
        .zip(b.chars())
        .take_while(|((_, x), y)| x == y)
        .last()
        .map_or(0, |((i, x), _)| i + x.len_utf8());
    // the prefix is equal text, so it ends at the same byte in both
    let suffix: usize = a[prefix..]
        .chars()
        .rev()
        .zip(b[prefix..].chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum();
    let common = prefix + suffix;
    if common == 0 || common * 4 < a.len().max(b.len()) {
        return None;
    }
    Some((prefix..a.len() - suffix, prefix..b.len() - suffix))
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
            Some(commit) => {
                // literal path: reset_default would read `a[12].txt` as a glob
                let mut index = repo.index().map_err(err)?;
                match commit.tree().map_err(err)?.get_path(rel) {
                    Ok(entry) => {
                        let size = repo
                            .find_blob(entry.id())
                            .map(|b| b.size() as u32)
                            .unwrap_or(0);
                        index
                            .add(&git2::IndexEntry {
                                ctime: git2::IndexTime::new(0, 0),
                                mtime: git2::IndexTime::new(0, 0),
                                dev: 0,
                                ino: 0,
                                mode: entry.filemode() as u32,
                                uid: 0,
                                gid: 0,
                                file_size: size,
                                id: entry.id(),
                                flags: 0,
                                flags_extended: 0,
                                path: path.as_bytes().to_vec(),
                            })
                            .map_err(err)?;
                    }
                    // not in HEAD: unstaging a new file drops it from the index
                    Err(_) => {
                        let _ = index.remove_path(rel);
                    }
                }
                index.write().map_err(err)
            }
            // unborn HEAD: unstaging drops the entry from the index
            None => {
                let mut index = repo.index().map_err(err)?;
                index.remove_path(rel).map_err(err)?;
                index.write().map_err(err)
            }
        },
        Op::Revert => {
            let index = repo.index().map_err(err)?;
            if index.get_path(rel, 0).is_some() {
                let mut checkout = CheckoutBuilder::new();
                checkout.force().path(path).disable_pathspec_match(true);
                return repo.checkout_index(None, Some(&mut checkout)).map_err(err);
            }
            if (1..=3).any(|stage| index.get_path(rel, stage).is_some()) {
                return Err("merge conflict: resolve it instead of reverting".into());
            }
            // untracked: reverting means deleting the file. Never a directory:
            // it may be a nested repository or hold work git does not list.
            let full = root.join(rel);
            let meta = full.symlink_metadata().map_err(|e| e.to_string())?;
            if meta.is_dir() {
                return Err("not reverting an untracked directory; delete it yourself".into());
            }
            std::fs::remove_file(&full).map_err(|e| e.to_string())
        }
    }
}

/// An untracked file's only hunk is the whole file: Stage writes the checked
/// content to the index, Revert deletes the file if it still holds exactly
/// that content. `diff` is the checked diff (reversed for Revert).
fn apply_untracked_hunk(
    repo: &Repository,
    op: Op,
    path: &str,
    diff: &Diff<'_>,
) -> Result<(), String> {
    let moved = || Err::<(), String>("the change moved; refreshed".into());
    let Some(idx) = (0..diff.deltas().len()).find(|&i| {
        diff.get_delta(i).is_some_and(|d| {
            path_str(d.new_file().path()).as_deref() == Some(path)
                || path_str(d.old_file().path()).as_deref() == Some(path)
        })
    }) else {
        return moved();
    };
    let Some(patch) = Patch::from_diff(diff, idx).map_err(err)? else {
        return moved();
    };
    // the working-tree side: added lines forward, removed lines reversed
    let side = if op == Op::Stage { '+' } else { '-' };
    let mut content = Vec::new();
    for h in 0..patch.num_hunks() {
        for l in 0..patch.num_lines_in_hunk(h).map_err(err)? {
            let line = patch.line_in_hunk(h, l).map_err(err)?;
            if line.origin() == side {
                content.extend_from_slice(line.content());
            }
        }
    }
    let root = repo.workdir().ok_or("bare repository")?;
    let full = root.join(path);
    match op {
        Op::Stage => {
            let delta = patch.delta();
            let mut index = repo.index().map_err(err)?;
            let entry = git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode: u32::from(delta.new_file().mode()),
                uid: 0,
                gid: 0,
                file_size: 0,
                id: git2::Oid::ZERO_SHA1,
                flags: 0,
                flags_extended: 0,
                path: path.as_bytes().to_vec(),
            };
            index.add_frombuffer(&entry, &content).map_err(err)?;
            index.write().map_err(err)
        }
        Op::Revert => {
            let meta = full.symlink_metadata().map_err(|e| e.to_string())?;
            let current = if meta.file_type().is_symlink() {
                std::fs::read_link(&full).map(|t| t.to_string_lossy().into_owned().into_bytes())
            } else {
                std::fs::read(&full)
            }
            .map_err(|e| e.to_string())?;
            if current != content {
                return moved();
            }
            std::fs::remove_file(&full).map_err(|e| e.to_string())
        }
        Op::Unstage => moved(),
    }
}

/// Diff options for one file's hunk write: limited to `path` (literally, no
/// glob), reversed when the write undoes the change.
fn hunk_options(op: Op, path: &str) -> DiffOptions {
    let mut opts = match op {
        // the file may be untracked, inside an untracked directory
        Op::Stage | Op::Revert => unstaged_options(),
        Op::Unstage => DiffOptions::new(),
    };
    opts.pathspec(path)
        .disable_pathspec_match(true)
        .reverse(op != Op::Stage);
    opts
}

fn apply_hunk(repo: &Repository, op: Op, path: &str, id: HunkId) -> Result<(), String> {
    let index = repo.index().map_err(err)?;
    let mut opts = hunk_options(op, path);
    // One diff is both content-checked and applied, so an edit landing in
    // between cannot be written unseen: the patch carries the checked
    // content, and applying it over different content fails.
    // Stage applies the forward diff to the index; Unstage and Revert apply
    // the reversed diff (index → HEAD, working tree → index), whose hunk
    // carries the swapped line ranges and the same content hash.
    let (diff, want, location) = match op {
        Op::Stage => (
            repo.diff_index_to_workdir(Some(&index), Some(&mut opts)),
            id,
            ApplyLocation::Index,
        ),
        Op::Unstage => {
            let tree = head_tree(repo);
            (
                repo.diff_tree_to_index(tree.as_ref(), Some(&index), Some(&mut opts)),
                id.swapped(),
                ApplyLocation::Index,
            )
        }
        Op::Revert => (
            repo.diff_index_to_workdir(Some(&index), Some(&mut opts)),
            id.swapped(),
            ApplyLocation::WorkDir,
        ),
    };
    let diff = diff.map_err(err)?;
    let files = collect_as(&diff, op != Op::Stage, None)?;
    let Some((file, hunk)) = files
        .iter()
        .filter(|file| file.path == path || file.old_path.as_deref() == Some(path))
        .flat_map(|file| file.hunks.iter().map(move |hunk| (file, hunk)))
        .find(|(_, hunk)| hunk.id == want)
    else {
        return Err("the change moved; refreshed".into());
    };
    if hunk.partial {
        return Err(format!(
            "this hunk is cut at {MAX_LINES_PER_FILE} lines; stage, unstage or revert the whole file"
        ));
    }
    #[cfg(test)]
    tests::between_check_and_apply();
    if op != Op::Unstage && index.get_path(Path::new(path), 0).is_none() {
        // git apply cannot take an untracked file's hunk: it has nothing in
        // the index to patch (Stage errors, Revert is a silent no-op)
        return apply_untracked_hunk(repo, op, path, &diff);
    }
    let mut found = false;
    let mut apply = ApplyOptions::new();
    apply.delta_callback(|delta| {
        delta.is_some_and(|d| {
            path_str(d.new_file().path()).as_deref() == Some(path)
                || path_str(d.old_file().path()).as_deref() == Some(path)
        })
    });
    // ranges identify the hunk within this diff, whose content was checked
    apply.hunk_callback(|hunk| {
        let hit = hunk.is_some_and(|h| {
            h.old_start() == want.old_start
                && h.old_lines() == want.old_lines
                && h.new_start() == want.new_start
                && h.new_lines() == want.new_lines
        });
        found |= hit;
        hit
    });
    repo.apply(&diff, location, Some(&mut apply)).map_err(err)?;
    drop(apply);
    // libgit2 applies a whole-file add or delete (one hunk) without calling
    // the hunk callback
    let whole_file = matches!(file.status, 'A' | 'D') && file.hunks.len() == 1;
    if found || whole_file {
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
    fn marks_changed_words_and_loads_the_new_side() {
        let t = temp_repo("emph");
        fs::write(
            t.dir.join("a.txt"),
            lines(&[(5, "cargo run -p shift-app  # dev")]),
        )
        .unwrap();
        commit_all(&t, "init");
        fs::write(
            t.dir.join("a.txt"),
            lines(&[(5, "cargo run  # dev"), (28, "x")]),
        )
        .unwrap();

        let r = review(&t);
        let a = &r.unstaged[0];
        let src = a
            .source
            .as_ref()
            .expect("a modified file loads its new side");
        assert_eq!((src.len(), src[4].as_str()), (30, "cargo run  # dev"));
        let find = |hunk: usize, kind| {
            a.hunks[hunk]
                .lines
                .iter()
                .find(|l| l.kind == kind)
                .unwrap()
                .clone()
        };
        let (old, new) = (find(0, LineKind::Removed), find(0, LineKind::Added));
        assert_eq!(&old.text[old.emph.clone().unwrap()], "-p shift-app ");
        assert_eq!(new.emph, Some(10..10), "pure deletion: empty span");
        // "line 28" → "x" shares nothing: the row tint is the highlight
        assert_eq!(find(1, LineKind::Removed).emph, None);

        // staged: the new side is the index blob, not the working tree
        apply(&t.dir, Op::Stage, "a.txt", None).unwrap();
        fs::write(t.dir.join("a.txt"), lines(&[(5, "later")])).unwrap();
        let r = review(&t);
        assert_eq!(r.staged[0].source.as_ref().unwrap()[4], "cargo run  # dev");
        assert_eq!(r.unstaged[0].source.as_ref().unwrap()[4], "later");
    }

    #[test]
    fn changed_span_keeps_char_boundaries() {
        let (a, b) = changed_span("héllo wörld", "héllo world").unwrap();
        assert_eq!((&"héllo wörld"[a], &"héllo world"[b]), ("ö", "o"));
        assert_eq!(changed_span("abc", "xyz"), None);
    }

    #[test]
    fn pairs_a_removed_line_past_an_inserted_one() {
        let line = |kind, text: &str| Line {
            kind,
            old_no: None,
            new_no: None,
            text: text.into(),
            emph: None,
        };
        let mut lines = vec![
            line(LineKind::Removed, "let v = compute(40);"),
            line(LineKind::Added, "println!(\"inserted\");"),
            line(LineKind::Added, "let v = recompute(40);"),
        ];
        mark_changes(&mut lines);
        assert_eq!(lines[1].emph, None);
        assert_eq!(&lines[2].text[lines[2].emph.clone().unwrap()], "re");
        assert_eq!(lines[0].emph, Some(8..8));
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

    thread_local! {
        static BETWEEN: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Test seam: runs once, after `apply_hunk` content-checked the hunk and
    /// before it writes.
    pub(super) fn between_check_and_apply() {
        if let Some(f) = BETWEEN.with(|b| b.borrow_mut().take()) {
            f();
        }
    }

    fn index_blob(t: &TempRepo, path: &str) -> String {
        let mut index = t.repo.index().unwrap();
        index.read(true).unwrap();
        let entry = index.get_path(Path::new(path), 0).unwrap();
        let blob = t.repo.find_blob(entry.id).unwrap();
        String::from_utf8(blob.content().to_vec()).unwrap()
    }

    #[test]
    fn an_edit_between_check_and_apply_is_never_reverted_unseen() {
        let t = temp_repo("race-revert");
        fs::write(t.dir.join("a.txt"), lines(&[])).unwrap();
        commit_all(&t, "init");
        fs::write(t.dir.join("a.txt"), lines(&[(2, "first edit")])).unwrap();
        let id = review(&t).unstaged[0].hunks[0].id;

        let file = t.dir.join("a.txt");
        BETWEEN.with(|b| {
            *b.borrow_mut() = Some(Box::new(move || {
                fs::write(&file, lines(&[(2, "different edit")])).unwrap();
            }))
        });
        assert!(apply(&t.dir, Op::Revert, "a.txt", Some(id)).is_err());
        assert_eq!(
            fs::read_to_string(t.dir.join("a.txt")).unwrap(),
            lines(&[(2, "different edit")]),
            "the unseen edit survives"
        );
        assert_eq!(index_blob(&t, "a.txt"), lines(&[]), "index untouched");
    }

    #[test]
    fn an_edit_between_check_and_apply_is_never_staged_unseen() {
        let t = temp_repo("race-stage");
        fs::write(t.dir.join("a.txt"), lines(&[])).unwrap();
        commit_all(&t, "init");
        fs::write(t.dir.join("a.txt"), lines(&[(2, "first edit")])).unwrap();
        let id = review(&t).unstaged[0].hunks[0].id;

        let file = t.dir.join("a.txt");
        BETWEEN.with(|b| {
            *b.borrow_mut() = Some(Box::new(move || {
                fs::write(&file, lines(&[(2, "different edit")])).unwrap();
            }))
        });
        assert!(apply(&t.dir, Op::Stage, "a.txt", Some(id)).is_err());
        assert_eq!(index_blob(&t, "a.txt"), lines(&[]), "index untouched");
        assert_eq!(
            fs::read_to_string(t.dir.join("a.txt")).unwrap(),
            lines(&[(2, "different edit")]),
            "the working tree is never written by a stage"
        );
    }

    #[test]
    fn reverts_the_hunk_of_an_untracked_file_unless_it_changed() {
        let t = temp_repo("untracked-revert");
        fs::write(t.dir.join("a.txt"), lines(&[])).unwrap();
        commit_all(&t, "init");
        fs::write(t.dir.join("b.txt"), "b\nno newline").unwrap();
        let id = review(&t).unstaged[0].hunks[0].id;

        let file = t.dir.join("b.txt");
        BETWEEN.with(|b| {
            *b.borrow_mut() = Some(Box::new(move || fs::write(&file, "edited\n").unwrap()))
        });
        assert!(apply(&t.dir, Op::Revert, "b.txt", Some(id)).is_err());
        assert_eq!(fs::read_to_string(t.dir.join("b.txt")).unwrap(), "edited\n");

        fs::write(t.dir.join("b.txt"), "b\nno newline").unwrap();
        apply(&t.dir, Op::Revert, "b.txt", Some(id)).unwrap();
        assert!(!t.dir.join("b.txt").exists());
        assert!(review(&t).unstaged.is_empty());
    }

    #[test]
    fn stages_a_hunk_of_a_file_in_an_untracked_directory() {
        let t = temp_repo("untracked-dir");
        fs::write(t.dir.join("a.txt"), lines(&[])).unwrap();
        commit_all(&t, "init");
        fs::write(t.dir.join("a.txt"), lines(&[(2, "two")])).unwrap();
        fs::create_dir_all(t.dir.join("src/deep")).unwrap();
        fs::write(t.dir.join("src/deep/new.rs"), "fn main() {}\n").unwrap();
        fs::write(t.dir.join("src/deep/other.rs"), "other\n").unwrap();
        let r = review(&t);
        let new = r
            .unstaged
            .iter()
            .find(|f| f.path == "src/deep/new.rs")
            .unwrap();
        apply(&t.dir, Op::Stage, "src/deep/new.rs", Some(new.hunks[0].id)).unwrap();
        let r = review(&t);
        assert_eq!(r.staged.len(), 1, "only the one file");
        assert_eq!(index_blob(&t, "src/deep/new.rs"), "fn main() {}\n");

        let staged = r.staged[0].hunks[0].id;
        apply(&t.dir, Op::Unstage, "src/deep/new.rs", Some(staged)).unwrap();
        let r = review(&t);
        assert!(r.staged.is_empty(), "{:?}", r.staged);
        assert!(r.unstaged.iter().any(|f| f.path == "src/deep/new.rs"));
    }

    #[test]
    fn hunk_ops_on_a_deleted_file() {
        let t = temp_repo("deleted");
        fs::write(t.dir.join("a.txt"), lines(&[])).unwrap();
        fs::write(t.dir.join("b.txt"), "b\n").unwrap();
        commit_all(&t, "init");
        fs::remove_file(t.dir.join("a.txt")).unwrap();
        fs::remove_file(t.dir.join("b.txt")).unwrap();

        let r = review(&t);
        let hunk = |f: &str| r.unstaged.iter().find(|x| x.path == f).unwrap().hunks[0].id;
        apply(&t.dir, Op::Revert, "a.txt", Some(hunk("a.txt"))).unwrap();
        assert_eq!(fs::read_to_string(t.dir.join("a.txt")).unwrap(), lines(&[]));
        apply(&t.dir, Op::Stage, "b.txt", Some(hunk("b.txt"))).unwrap();
        let r = review(&t);
        assert_eq!((r.staged.len(), r.staged[0].status), (1, 'D'));
        assert!(r.unstaged.is_empty());
    }

    #[test]
    fn a_hunk_cut_by_the_line_cap_is_partial_and_refused() {
        let t = temp_repo("partial");
        let text = |changed: bool| {
            (1..=5100)
                .map(|n| {
                    if changed && (n == 2 || n >= 100) {
                        format!("changed {n}\n")
                    } else {
                        format!("line {n}\n")
                    }
                })
                .collect::<String>()
        };
        fs::write(t.dir.join("a.txt"), text(false)).unwrap();
        commit_all(&t, "init");
        fs::write(t.dir.join("a.txt"), text(true)).unwrap();

        let r = review(&t);
        let file = &r.unstaged[0];
        assert!(file.truncated);
        assert_eq!(file.hunks.len(), 2);
        assert!(!file.hunks[0].partial);
        let last = file.hunks.last().unwrap();
        assert!(last.partial, "5,000 changed lines are cut at the cap");

        for op in [Op::Stage, Op::Revert] {
            let e = apply(&t.dir, op, "a.txt", Some(last.id)).unwrap_err();
            assert!(e.contains("whole file"), "{e}");
        }
        assert_eq!(fs::read_to_string(t.dir.join("a.txt")).unwrap(), text(true));
        assert!(review(&t).staged.is_empty());
        // the complete hunk above it still applies
        apply(&t.dir, Op::Stage, "a.txt", Some(file.hunks[0].id)).unwrap();
        assert_eq!(review(&t).staged[0].hunks.len(), 1);
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
    fn revert_never_deletes_an_untracked_directory_or_nested_repo() {
        let t = temp_repo("revert-dir");
        fs::write(t.dir.join("a.txt"), "a\n").unwrap();
        commit_all(&t, "init");
        let sub = t.dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        Repository::init(&sub).unwrap();
        fs::write(sub.join("work.txt"), "precious\n").unwrap();
        fs::create_dir_all(t.dir.join("plain")).unwrap();
        fs::write(t.dir.join("plain/x.txt"), "x\n").unwrap();
        for p in ["sub", "sub/", "plain"] {
            assert!(apply(&t.dir, Op::Revert, p, None).is_err(), "{p}");
        }
        assert!(sub.join(".git").exists());
        assert!(sub.join("work.txt").exists());
        assert!(t.dir.join("plain/x.txt").exists());
    }

    #[test]
    fn revert_refuses_a_conflicted_file() {
        let t = temp_repo("revert-conflict");
        fs::write(t.dir.join("a.txt"), "base\n").unwrap();
        commit_all(&t, "init");
        let blob = |s: &str| t.repo.blob(s.as_bytes()).unwrap();
        let entry = |id: git2::Oid, stage: u16| git2::IndexEntry {
            ctime: git2::IndexTime::new(0, 0),
            mtime: git2::IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: 0o100644,
            uid: 0,
            gid: 0,
            file_size: 0,
            id,
            flags: stage << 12,
            flags_extended: 0,
            path: b"a.txt".to_vec(),
        };
        let mut index = t.repo.index().unwrap();
        index.remove_path(Path::new("a.txt")).unwrap();
        for (content, stage) in [("base\n", 1), ("ours\n", 2), ("theirs\n", 3)] {
            index.add(&entry(blob(content), stage)).unwrap();
        }
        index.write().unwrap();
        assert!(index.has_conflicts());
        fs::write(t.dir.join("a.txt"), "my resolution\n").unwrap();
        assert!(apply(&t.dir, Op::Revert, "a.txt", None).is_err());
        assert_eq!(
            fs::read_to_string(t.dir.join("a.txt")).unwrap(),
            "my resolution\n"
        );
    }

    #[test]
    fn whole_file_ops_treat_the_path_literally() {
        let t = temp_repo("literal");
        for f in ["a1.txt", "a2.txt", "a[12].txt"] {
            fs::write(t.dir.join(f), "old\n").unwrap();
        }
        commit_all(&t, "init");
        for f in ["a1.txt", "a2.txt", "a[12].txt"] {
            fs::write(t.dir.join(f), "new\n").unwrap();
        }
        apply(&t.dir, Op::Revert, "a[12].txt", None).unwrap();
        assert_eq!(
            fs::read_to_string(t.dir.join("a[12].txt")).unwrap(),
            "old\n"
        );
        assert_eq!(fs::read_to_string(t.dir.join("a1.txt")).unwrap(), "new\n");
        assert_eq!(fs::read_to_string(t.dir.join("a2.txt")).unwrap(), "new\n");

        for f in ["a1.txt", "a2.txt", "a[12].txt"] {
            fs::write(t.dir.join(f), "newer\n").unwrap();
            apply(&t.dir, Op::Stage, f, None).unwrap();
        }
        apply(&t.dir, Op::Unstage, "a[12].txt", None).unwrap();
        let r = review(&t);
        let staged: Vec<_> = r.staged.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(staged, ["a1.txt", "a2.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn backslash_in_a_file_name_survives() {
        let t = temp_repo("backslash");
        fs::write(t.dir.join("a.txt"), "a\n").unwrap();
        commit_all(&t, "init");
        fs::write(t.dir.join("a\\b.txt"), "x\n").unwrap();
        let r = review(&t);
        assert!(r.unstaged.iter().any(|f| f.path == "a\\b.txt"));
        apply(&t.dir, Op::Stage, "a\\b.txt", None).unwrap();
        assert_eq!(review(&t).staged[0].path, "a\\b.txt");
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
