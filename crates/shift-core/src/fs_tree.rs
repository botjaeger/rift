//! Directory listing for the file explorer: one level at a time (the tree
//! expands lazily), directories first, case-insensitive order, `.git` and
//! git-ignored entries left out.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// Cap per directory: a listing past this is cut (node_modules-sized dirs).
pub const MAX_ENTRIES: usize = 2000;

/// The explorer root for `cwd`: the enclosing repository's working tree, or
/// `cwd` itself outside a repository.
pub fn root_for(cwd: &Path) -> PathBuf {
    git2::Repository::discover(cwd)
        .ok()
        .and_then(|r| r.workdir().map(Path::to_path_buf))
        .map(|p| p.canonicalize().unwrap_or(p))
        .unwrap_or_else(|| cwd.to_path_buf())
}

/// List `dir`. `repo` filters ignored entries. Unreadable directories list
/// as empty.
pub fn list(dir: &Path, repo: Option<&git2::Repository>) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    // both sides canonical: on macOS /var is a link to /private/var, and a
    // mismatch would skip the ignore check
    let workdir = repo
        .and_then(|r| r.workdir())
        .map(|w| w.canonicalize().unwrap_or_else(|_| w.to_path_buf()));
    let base = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut out: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == ".DS_Store" {
                return None;
            }
            let path = e.path();
            // follows symlinks: a link to a directory expands like one
            let is_dir = path.is_dir();
            if let (Some(repo), Some(workdir)) = (repo, workdir.as_deref())
                && let Ok(rel) = base.join(&name).strip_prefix(workdir)
            {
                // a `dir/` pattern only matches with the trailing slash
                let rel = rel.to_string_lossy().replace('\\', "/");
                let probe = if is_dir { format!("{rel}/") } else { rel };
                if repo.is_path_ignored(&probe).unwrap_or(false) {
                    return None;
                }
            }
            Some(Entry { name, path, is_dir })
        })
        .take(MAX_ENTRIES)
        .collect();
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// `path` relative to `root`, `/`-separated (the review model's key).
pub fn rel_key(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn dirs_first_sorted_ignored_and_git_hidden() {
        let dir = std::env::temp_dir().join(format!(
            "shift-fstree-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("target/debug")).unwrap();
        fs::write(dir.join("b.txt"), "").unwrap();
        fs::write(dir.join("A.md"), "").unwrap();
        fs::write(dir.join(".gitignore"), "target/\n*.log\n").unwrap();
        fs::write(dir.join("x.log"), "").unwrap();
        let repo = git2::Repository::init(&dir).unwrap();

        let names: Vec<String> = list(&dir, Some(&repo))
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["src", ".gitignore", "A.md", "b.txt"]);
        // without the repo nothing is filtered but .git
        let all: Vec<String> = list(&dir, None).into_iter().map(|e| e.name).collect();
        assert_eq!(
            all,
            ["src", "target", ".gitignore", "A.md", "b.txt", "x.log"]
        );
        assert_eq!(root_for(&dir.join("src")), dir.canonicalize().unwrap());
        assert_eq!(
            rel_key(&dir, &dir.join("src/main.rs")).as_deref(),
            Some("src/main.rs")
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
