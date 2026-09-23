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

    Some(GitInfo {
        branch,
        ahead,
        behind,
        dirty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unborn_branch_reports_its_name_and_dirty_state() {
        let dir = std::env::temp_dir().join(format!("runway-git-{}", std::process::id()));
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
        assert!(query_repo(&repo).unwrap().dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
