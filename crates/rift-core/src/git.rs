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
    /// Set when the directory is inside a linked worktree (`git worktree
    /// add`); `None` in the main checkout.
    pub worktree: Option<Worktree>,
}

/// A linked worktree's identity: the repository's common directory plus
/// the worktree's own path. Never the branch: it changes with `git
/// switch`, and other repositories have branches of the same name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// Its directory's name (the tab subtitle's `<name> · <branch>`).
    pub name: String,
    /// Its working directory.
    pub path: PathBuf,
    /// The `.git` directory shared by every worktree of the repository.
    pub common_dir: PathBuf,
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
        worktree: linked_worktree(repo),
    })
}

/// `path` without a trailing separator (git2 ends directories with one).
fn clean(path: &std::path::Path) -> PathBuf {
    path.components().collect()
}

/// The linked worktree `repo` was opened in; `None` for the main checkout
/// (and a bare repository).
fn linked_worktree(repo: &git2::Repository) -> Option<Worktree> {
    if !repo.is_worktree() {
        return None;
    }
    let path = clean(repo.workdir()?);
    Some(Worktree {
        name: path.file_name()?.to_string_lossy().into_owned(),
        common_dir: clean(repo.commondir()),
        path,
    })
}

/// The main checkout's working directory: the parent of the common `.git`
/// directory (`None` for a bare repository).
fn main_workdir(repo: &git2::Repository) -> Option<PathBuf> {
    let common = clean(repo.commondir());
    if common.file_name()? != ".git" {
        return None;
    }
    common.parent().map(PathBuf::from)
}

/// The main checkout of the repository containing `cwd` (from any of its
/// worktrees); `None` outside a repository or for a bare one.
pub fn main_checkout(cwd: &std::path::Path) -> Option<PathBuf> {
    main_workdir(&git2::Repository::discover(cwd).ok()?)
}

/// Where a new worktree for `branch` goes: `<parent>/<repo>.worktrees/<branch>`
/// beside the main checkout `main`.
pub fn worktree_dir(main: &std::path::Path, branch: &str) -> Option<PathBuf> {
    let name = main.file_name()?.to_string_lossy();
    Some(
        main.parent()?
            .join(format!("{name}.worktrees"))
            .join(branch),
    )
}

/// The first `wt-<n>` that names neither a local branch nor an existing
/// worktree directory of the repository containing `cwd`.
pub fn suggest_worktree_branch(cwd: &std::path::Path) -> Option<String> {
    let repo = git2::Repository::discover(cwd).ok()?;
    let main = main_workdir(&repo)?;
    (1..1000).map(|n| format!("wt-{n}")).find(|b| {
        repo.find_branch(b, git2::BranchType::Local).is_err()
            && worktree_dir(&main, b).is_some_and(|d| !d.exists())
    })
}

/// `git worktree add -b <branch> <dir> HEAD` for the repository containing
/// `cwd` (HEAD is `cwd`'s own checkout), the directory from
/// [`worktree_dir`]. Runs the git CLI with an argument list (no shell) and
/// no terminal prompt. Returns the new worktree's directory, or git's
/// message; a failure that still left a branch or directory behind says so.
pub fn add_worktree(cwd: &std::path::Path, branch: &str) -> Result<PathBuf, String> {
    let branch = branch.trim();
    if branch.is_empty() {
        return Err("Type a name for the new branch".into());
    }
    if branch.starts_with('-') || !git2::Branch::name_is_valid(branch).unwrap_or(false) {
        return Err(format!("“{branch}” is not a valid branch name"));
    }
    let repo = git2::Repository::discover(cwd).map_err(|_| "Not inside a Git repository")?;
    let main = main_workdir(&repo).ok_or("The repository has no working directory")?;
    let dir = worktree_dir(&main, branch).ok_or("No directory for the worktree")?;
    let branch_existed = repo.find_branch(branch, git2::BranchType::Local).is_ok();
    let dir_existed = dir.exists();
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["worktree", "add", "-b", branch])
        .arg(&dir)
        .arg("HEAD")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("Could not run git: {e}"))?;
    if out.status.success() {
        return Ok(dir);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut msg = stderr
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("git worktree add failed")
        .to_string();
    let mut left = Vec::new();
    // git creates the branch before it checks the directory: a branch this
    // call made for a worktree that never came goes again
    if !branch_existed
        && let Ok(mut b) = repo.find_branch(branch, git2::BranchType::Local)
        && b.delete().is_err()
    {
        left.push(format!("branch {branch}"));
    }
    if !dir_existed && dir.exists() {
        left.push(format!("directory {}", dir.display()));
    }
    if !left.is_empty() {
        msg.push_str(&format!(" (left behind: {})", left.join(", ")));
    }
    Err(msg)
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
        let dir = std::env::temp_dir().join(format!("rift-branches-{}", std::process::id()));
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
        assert!(local_branches(&std::env::temp_dir().join("rift-no-repo-here")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A scratch repository `<tmp>/<tag>/repo` with one commit on `main`.
    fn scratch_repo(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("rift-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("repo");
        std::fs::create_dir_all(&dir).unwrap();
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("main");
        let repo = git2::Repository::init_opts(&dir, &opts).unwrap();
        std::fs::write(dir.join("f.txt"), "x\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("f.txt")).unwrap();
        index.write().unwrap();
        let sig = git2::Signature::now("t", "t@e.x").unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        (root, dir)
    }

    fn canon(p: &std::path::Path) -> PathBuf {
        p.canonicalize().unwrap()
    }

    #[test]
    fn linked_worktrees_have_their_own_identity_and_the_main_checkout_none() {
        let (root, main) = scratch_repo("wt-id");
        let main_info = query(&main).unwrap();
        assert_eq!(main_info.branch, "main");
        assert_eq!(
            main_info.worktree, None,
            "the main checkout is no linked worktree"
        );

        assert_eq!(suggest_worktree_branch(&main).as_deref(), Some("wt-1"));
        let a = add_worktree(&main, "wt-1").unwrap();
        assert_eq!(canon(&a), canon(&root.join("repo.worktrees/wt-1")));
        assert_eq!(suggest_worktree_branch(&main).as_deref(), Some("wt-2"));
        // from inside a linked worktree: the new one still goes beside the
        // main checkout
        assert_eq!(canon(&main_checkout(&a).unwrap()), canon(&main));
        let b = add_worktree(&a, "feature/x").unwrap();
        assert_eq!(canon(&b), canon(&root.join("repo.worktrees/feature/x")));

        let ia = query(&a.join(".")).unwrap();
        let ib = query(&b).unwrap();
        let (wa, wb) = (ia.worktree.clone().unwrap(), ib.worktree.clone().unwrap());
        assert_eq!((ia.branch.as_str(), wa.name.as_str()), ("wt-1", "wt-1"));
        assert_eq!((ib.branch.as_str(), wb.name.as_str()), ("feature/x", "x"));
        assert_eq!(canon(&wa.common_dir), canon(&main.join(".git")));
        assert_eq!(
            canon(&wa.common_dir),
            canon(&wb.common_dir),
            "one repository"
        );
        assert_ne!(wa.path, wb.path, "two worktrees");

        // a branch switch changes the branch, not the worktree's identity
        let switched = std::process::Command::new("git")
            .arg("-C")
            .arg(&a)
            .args(["switch", "-q", "-c", "other"])
            .status()
            .unwrap();
        assert!(switched.success());
        let again = query(&a).unwrap();
        assert_eq!(again.branch, "other");
        assert_eq!(again.worktree, Some(wa));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn adding_a_worktree_fails_cleanly_with_gits_message() {
        let (root, main) = scratch_repo("wt-err");
        // the branch exists: git refuses, nothing is created
        let err = add_worktree(&main, "main").unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert!(!err.contains("left behind"), "{err}");
        assert!(!root.join("repo.worktrees").exists());
        // the directory exists (not a worktree): refused, the branch is
        // not left behind
        std::fs::create_dir_all(root.join("repo.worktrees/busy")).unwrap();
        std::fs::write(root.join("repo.worktrees/busy/keep"), "").unwrap();
        let err = add_worktree(&main, "busy").unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        let repo = git2::Repository::open(&main).unwrap();
        assert!(!err.contains("left behind"), "{err}");
        // git made the branch before it refused: it is gone again
        assert!(
            repo.find_branch("busy", git2::BranchType::Local).is_err(),
            "{err}"
        );
        // not a branch name, not a repository
        assert!(
            add_worktree(&main, "-x")
                .unwrap_err()
                .contains("not a valid")
        );
        assert!(
            add_worktree(&main, "a..b")
                .unwrap_err()
                .contains("not a valid")
        );
        assert!(add_worktree(&main, "  ").is_err());
        assert_eq!(
            add_worktree(&root, "x").unwrap_err(),
            "Not inside a Git repository"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unborn_branch_reports_its_name_and_dirty_state() {
        let dir = std::env::temp_dir().join(format!("rift-git-{}", std::process::id()));
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
