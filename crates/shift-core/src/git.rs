use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

/// Prompt-line git context (design 01: branch · ↑ahead).
#[derive(Debug, Clone, PartialEq)]
pub struct GitInfo {
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    pub dirty: bool,
    /// Uncommitted changes against HEAD (untracked files included, their
    /// lines as added): changed files, added and removed lines.
    pub files: usize,
    pub added: usize,
    pub removed: usize,
}

pub type GitCallback = Arc<dyn Fn(u64, Option<GitInfo>) + Send + Sync>;

/// Background git2 worker — statuses can take 100ms+ on large repos, so this
/// never runs on the render thread. Latest request per pane wins.
pub struct GitWorker {
    tx: Sender<(u64, PathBuf)>,
}

impl GitWorker {
    pub fn new(callback: GitCallback) -> Self {
        let (tx, rx) = channel::<(u64, PathBuf)>();
        std::thread::Builder::new()
            .name("git-status".into())
            .spawn(move || {
                while let Ok((pane, path)) = rx.recv() {
                    // drain: keep only the newest request per pane
                    let mut latest: Vec<(u64, PathBuf)> = vec![(pane, path)];
                    while let Ok(next) = rx.try_recv() {
                        latest.retain(|(p, _)| *p != next.0);
                        latest.push(next);
                    }
                    for (pane, path) in latest {
                        callback(pane, query(&path));
                    }
                }
            })
            .expect("spawn git worker");
        GitWorker { tx }
    }

    pub fn request(&self, pane: u64, path: PathBuf) {
        let _ = self.tx.send((pane, path));
    }
}

fn query(path: &std::path::Path) -> Option<GitInfo> {
    let repo = git2::Repository::discover(path).ok()?;
    query_repo(&repo)
}

fn query_repo(repo: &git2::Repository) -> Option<GitInfo> {
    let head = repo.head().ok();
    let branch = match &head {
        Some(head) if head.is_branch() => head.shorthand().ok()?.to_string(),
        Some(head) => format!("{:.7}", head.target()?),
        // unborn branch (fresh `git init`, no commits yet): HEAD is a
        // symbolic ref to the branch that will be created
        None => repo
            .find_reference("HEAD")
            .ok()?
            .symbolic_target()
            .ok()??
            .strip_prefix("refs/heads/")?
            .to_string(),
    };

    let (ahead, behind) = (|| {
        let head = head.as_ref()?;
        let local = head.target()?;
        let local_branch = git2::Branch::wrap(repo.find_reference(head.name().ok()?).ok()?);
        let upstream = local_branch.upstream().ok()?.get().target()?;
        repo.graph_ahead_behind(local, upstream).ok()
    })()
    .unwrap_or((0, 0));

    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .exclude_submodules(true)
        .update_index(false);
    let dirty = repo
        .statuses(Some(&mut opts))
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    // `3 • +10 −2` (the diff stats chip); a clean tree skips the diff
    let (files, added, removed) = dirty
        .then(|| {
            let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
            let mut o = git2::DiffOptions::new();
            // an untracked file's lines count as added, as in the review
            // panel and the tab bar's total
            o.include_untracked(true)
                .recurse_untracked_dirs(true)
                .show_untracked_content(true);
            let diff = repo
                .diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut o))
                .ok()?;
            let stats = diff.stats().ok()?;
            Some((stats.files_changed(), stats.insertions(), stats.deletions()))
        })
        .flatten()
        .unwrap_or((0, 0, 0));

    Some(GitInfo {
        branch,
        ahead,
        behind,
        dirty,
        files,
        added,
        removed,
    })
}

/// Local branches of the repository containing `path`, sorted, with the
/// checked-out one first (an unborn one included). Empty outside a
/// repository.
pub fn local_branches(path: &std::path::Path) -> Vec<String> {
    let Ok(repo) = git2::Repository::discover(path) else {
        return Vec::new();
    };
    let mut names: Vec<String> = repo
        .branches(Some(git2::BranchType::Local))
        .map(|it| {
            it.filter_map(|b| b.ok()?.0.name().ok()?.map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    names.sort_by_key(|n| n.to_lowercase());
    if let Some(current) = query_repo(&repo).map(|i| i.branch) {
        names.retain(|n| *n != current);
        names.insert(0, current);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_branches_put_the_current_one_first() {
        let dir = std::env::temp_dir().join(format!("shift-branches-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("main");
        let repo = git2::Repository::init_opts(&dir, &opts).unwrap();
        // unborn: the branch-to-be is still listed
        assert_eq!(local_branches(&dir), ["main"]);
        let sig = git2::Signature::now("t", "t@e.x").unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        let commit = repo
            .commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let commit = repo.find_commit(commit).unwrap();
        for name in ["zeta", "Alpha"] {
            repo.branch(name, &commit, false).unwrap();
        }
        assert_eq!(local_branches(&dir), ["main", "Alpha", "zeta"]);
        assert!(local_branches(&std::env::temp_dir().join("shift-no-repo-here")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unborn_branch_reports_its_name_and_dirty_state() {
        let dir = std::env::temp_dir().join(format!("shift-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("main");
        let repo = git2::Repository::init_opts(&dir, &opts).unwrap();
        let info = query_repo(&repo).expect("unborn HEAD still yields info");
        assert_eq!(info.branch, "main");
        assert_eq!((info.ahead, info.behind), (0, 0));
        assert!(!info.dirty);
        std::fs::write(dir.join("f.txt"), "x").unwrap();
        let info = query_repo(&repo).unwrap();
        assert!(info.dirty);
        // an untracked file counts with its lines (`x`, no newline: one)
        assert_eq!((info.files, info.added, info.removed), (1, 1, 0));
        // staged, its line counts
        std::fs::write(dir.join("f.txt"), "x\ny\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("f.txt")).unwrap();
        index.write().unwrap();
        let info = query_repo(&repo).unwrap();
        assert_eq!((info.files, info.added, info.removed), (1, 2, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
