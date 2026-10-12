//! Language runtimes for the prompt chips: the PHP / Python version a shell
//! runs (from the binary its PATH resolves to, which the shell hooks report)
//! and the installed ones a chip switches the shell to (Homebrew, pyenv,
//! phpenv, asdf, mise). Switching is shell-local: `rift-use` (shell hooks)
//! puts a version's `bin` first on PATH, or sets `PYENV_VERSION`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Switch {
    /// Put this directory first on PATH.
    Path(PathBuf),
    /// `PYENV_VERSION` (a pyenv version name, or `system`).
    Pyenv(String),
}

/// One installed runtime a chip offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub version: String,
    /// Where it comes from: `php@8.3`, `pyenv`, `asdf`…
    pub source: String,
    pub switch: Switch,
}

impl Runtime {
    /// `rift-use` arguments selecting it (`php` / `python` = the tool).
    pub fn command(&self, tool: &str) -> String {
        let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
        match &self.switch {
            Switch::Path(dir) => format!("rift-use {tool} {}", q(&dir.to_string_lossy())),
            Switch::Pyenv(v) => format!("rift-use pyenv {}", q(v)),
        }
    }
}

/// Homebrew prefixes: `$HOMEBREW_PREFIX`, else the standard two.
pub fn brew_prefixes() -> Vec<PathBuf> {
    match std::env::var_os("HOMEBREW_PREFIX").filter(|v| !v.is_empty()) {
        Some(p) => vec![PathBuf::from(p)],
        None => vec!["/opt/homebrew".into(), "/usr/local".into()],
    }
}

/// The home directory version managers install under.
pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// pyenv's root: `$PYENV_ROOT`, else `~/.pyenv`.
pub fn pyenv_root() -> Option<PathBuf> {
    std::env::var_os("PYENV_ROOT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".pyenv")))
}

/// The version a runtime's install path names: `Cellar/php@8.3/8.3.35`,
/// `versions/3.12.2` (pyenv, phpenv, Python.framework `Versions`),
/// `installs/php/8.2.1` (asdf, mise).
pub fn version_from_path(path: &Path) -> Option<String> {
    let parts: Vec<&str> = path.iter().filter_map(|c| c.to_str()).collect();
    let at = |i: usize| parts.get(i).copied();
    let found = parts.iter().enumerate().find_map(|(i, c)| match *c {
        "Cellar" => at(i + 2),
        "versions" | "Versions" => at(i + 1),
        "installs" => at(i + 2),
        _ => None,
    })?;
    // Homebrew revisions (`8.3.35_1`) are the same version
    let v = found.split('_').next().unwrap_or(found);
    v.starts_with(|c: char| c.is_ascii_digit())
        .then(|| v.to_string())
}

/// Newest first: numeric runs compare as numbers.
fn version_key(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap_or(0))
        .collect()
}

fn sort_newest_first(list: &mut [Runtime]) {
    list.sort_by(|a, b| {
        version_key(&b.version)
            .cmp(&version_key(&a.version))
            .then(a.source.cmp(&b.source))
    });
}

fn subdirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().is_dir())
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Version-manager install roots under `home`: (root, source).
fn manager_roots(home: &Path, tool: &str) -> Vec<(PathBuf, &'static str)> {
    let mut roots = vec![
        (home.join(".asdf/installs").join(tool), "asdf"),
        (home.join(".local/share/mise/installs").join(tool), "mise"),
    ];
    if tool == "php" {
        roots.insert(0, (home.join(".phpenv/versions"), "phpenv"));
    }
    roots
}

/// Installed PHP versions: Homebrew `php` / `php@X.Y`, phpenv, asdf, mise.
pub fn php_installs(home: &Path, prefixes: &[PathBuf]) -> Vec<Runtime> {
    let mut list = Vec::new();
    let mut seen = Vec::new();
    let mut add = |list: &mut Vec<Runtime>, version: String, source: String, bin: PathBuf| {
        if !bin.join("php").is_file() {
            return;
        }
        let real = std::fs::canonicalize(bin.join("php")).unwrap_or(bin.join("php"));
        if seen.contains(&real) {
            return;
        }
        seen.push(real);
        list.push(Runtime {
            version,
            source,
            switch: Switch::Path(bin),
        });
    };
    for prefix in prefixes {
        for (name, dir) in subdirs(&prefix.join("opt")) {
            let formula = name == "php"
                || name
                    .strip_prefix("php@")
                    .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()));
            if !formula {
                continue;
            }
            let version = std::fs::canonicalize(&dir)
                .ok()
                .and_then(|real| version_from_path(&real))
                .unwrap_or_else(|| name.trim_start_matches("php@").to_string());
            add(&mut list, version, name, dir.join("bin"));
        }
    }
    for (root, source) in manager_roots(home, "php") {
        for (version, dir) in subdirs(&root) {
            add(&mut list, version, source.into(), dir.join("bin"));
        }
    }
    sort_newest_first(&mut list);
    list
}

/// Installed Python versions: pyenv's (and `system`) when pyenv is set up,
/// else Homebrew `python@X.Y`, asdf and mise.
pub fn python_installs(home: &Path, prefixes: &[PathBuf], pyenv: Option<&Path>) -> Vec<Runtime> {
    let mut list = Vec::new();
    if let Some(root) = pyenv.filter(|r| r.join("versions").is_dir()) {
        for (version, _) in subdirs(&root.join("versions")) {
            list.push(Runtime {
                switch: Switch::Pyenv(version.clone()),
                version,
                source: "pyenv".into(),
            });
        }
        sort_newest_first(&mut list);
        list.push(Runtime {
            version: "system".into(),
            source: "pyenv".into(),
            switch: Switch::Pyenv("system".into()),
        });
        return list;
    }
    for prefix in prefixes {
        for (name, dir) in subdirs(&prefix.join("opt")) {
            let Some(short) = name.strip_prefix("python@") else {
                continue;
            };
            if !dir.join("bin/python3").is_file() {
                continue;
            }
            let version = std::fs::canonicalize(&dir)
                .ok()
                .and_then(|real| version_from_path(&real))
                .unwrap_or_else(|| short.to_string());
            list.push(Runtime {
                version,
                source: name,
                switch: Switch::Path(dir.join("bin")),
            });
        }
    }
    for (root, source) in manager_roots(home, "python") {
        for (version, dir) in subdirs(&root) {
            if dir.join("bin/python3").is_file() {
                list.push(Runtime {
                    version,
                    source: source.into(),
                    switch: Switch::Path(dir.join("bin")),
                });
            }
        }
    }
    sort_newest_first(&mut list);
    list
}

/// pyenv's version for `cwd`: `PYENV_VERSION` is reported by the shell (the
/// Python chip shows it directly), so this reads `.python-version` up the
/// tree, then `<root>/version`, else `system`.
pub fn pyenv_version(root: &Path, cwd: &Path) -> String {
    let first_line = |p: &Path| {
        std::fs::read_to_string(p)
            .ok()?
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
    };
    cwd.ancestors()
        .find_map(|d| first_line(&d.join(".python-version")))
        .or_else(|| first_line(&root.join("version")))
        .unwrap_or_else(|| "system".into())
}

/// The Python version a shell's `python3` (`bin`, as reported) runs in
/// `cwd`: pyenv shims resolve through pyenv's files; anything else by its
/// install path. `None` when neither says (a system Python).
pub fn python_version(bin: &Path, cwd: &Path) -> Option<String> {
    if let Some(root) = bin
        .parent()
        .filter(|d| d.file_name().is_some_and(|n| n == "shims"))
        .and_then(Path::parent)
    {
        return Some(pyenv_version(root, cwd));
    }
    version_from_path(&std::fs::canonicalize(bin).ok()?)
}

/// The PHP version of `bin` (as the shell reports it): from its install
/// path, else by asking it once in the background (`None` until it
/// answers; the chip repaints with the next frame).
pub fn php_version(bin: &Path) -> Option<String> {
    let real = std::fs::canonicalize(bin).ok()?;
    if let Some(v) = version_from_path(&real) {
        return Some(v);
    }
    static ASKED: Mutex<Option<HashMap<PathBuf, Option<String>>>> = Mutex::new(None);
    let mut asked = ASKED.lock().unwrap();
    let asked = asked.get_or_insert_with(HashMap::new);
    if let Some(v) = asked.get(&real) {
        return v.clone();
    }
    asked.insert(real.clone(), None);
    let _ = std::thread::Builder::new()
        .name("php-version".into())
        .spawn(move || {
            let v = std::process::Command::new(&real)
                .args(["-r", "echo PHP_VERSION;"])
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|v| version_key(v).len() >= 2 && v.len() < 32);
            if let Some(map) = ASKED.lock().unwrap().as_mut() {
                map.insert(real, v);
            }
        });
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rift-rt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
    }

    #[test]
    fn versions_come_from_install_paths() {
        let v = |p: &str| version_from_path(Path::new(p));
        assert_eq!(
            v("/opt/homebrew/Cellar/php@8.3/8.3.35_1/bin/php").as_deref(),
            Some("8.3.35")
        );
        assert_eq!(
            v("/home/u/.pyenv/versions/3.12.2/bin/python3").as_deref(),
            Some("3.12.2")
        );
        assert_eq!(
            v("/home/u/.asdf/installs/php/8.2.1/bin/php").as_deref(),
            Some("8.2.1")
        );
        assert_eq!(
            v("/Library/Frameworks/Python.framework/Versions/3.12/bin/python3").as_deref(),
            Some("3.12")
        );
        assert_eq!(v("/usr/bin/php"), None);
        assert_eq!(v("/x/versions/system/bin/php"), None);
    }

    #[cfg(unix)]
    #[test]
    fn php_installs_list_homebrew_and_managers_newest_first() {
        let d = scratch("php");
        let brew = d.join("brew");
        for (formula, version) in [("php", "8.4.1"), ("php@8.2", "8.2.9")] {
            touch(&brew.join(format!("Cellar/{formula}/{version}/bin/php")));
            std::fs::create_dir_all(brew.join("opt")).unwrap();
            std::os::unix::fs::symlink(
                brew.join(format!("Cellar/{formula}/{version}")),
                brew.join(format!("opt/{formula}")),
            )
            .unwrap();
        }
        // not PHP, or not installed
        std::fs::create_dir_all(brew.join("opt/phpstan/bin")).unwrap();
        std::fs::create_dir_all(brew.join("opt/php@7.4/bin")).unwrap();
        touch(&d.join("home/.phpenv/versions/8.3.0/bin/php"));
        let list = php_installs(&d.join("home"), std::slice::from_ref(&brew));
        let shown: Vec<(&str, &str)> = list
            .iter()
            .map(|r| (r.version.as_str(), r.source.as_str()))
            .collect();
        assert_eq!(
            shown,
            [("8.4.1", "php"), ("8.3.0", "phpenv"), ("8.2.9", "php@8.2")]
        );
        assert_eq!(
            list[2].command("php"),
            format!("rift-use php '{}'", brew.join("opt/php@8.2/bin").display())
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn python_installs_prefer_pyenv() {
        let d = scratch("py");
        for v in ["3.11.1", "3.12.2"] {
            std::fs::create_dir_all(d.join(format!("pyenv/versions/{v}"))).unwrap();
        }
        touch(&d.join("brew/opt/python@3.13/bin/python3"));
        let list = python_installs(&d.join("home"), &[d.join("brew")], Some(&d.join("pyenv")));
        let shown: Vec<&str> = list.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(shown, ["3.12.2", "3.11.1", "system"]);
        assert_eq!(list[1].command("python"), "rift-use pyenv '3.11.1'");
        // without pyenv: Homebrew's
        let list = python_installs(&d.join("home"), &[d.join("brew")], Some(&d.join("none")));
        assert_eq!(list.len(), 1);
        assert_eq!(
            (list[0].version.as_str(), list[0].source.as_str()),
            ("3.13", "python@3.13")
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn pyenv_version_reads_local_then_global_files() {
        let d = scratch("pyver");
        let root = d.join("pyenv");
        let project = d.join("project/src");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(pyenv_version(&root, &project), "system");
        std::fs::write(root.join("version"), "3.12.2\n").unwrap();
        assert_eq!(pyenv_version(&root, &project), "3.12.2");
        std::fs::write(d.join("project/.python-version"), "# pinned\n3.11.1\n").unwrap();
        assert_eq!(pyenv_version(&root, &project), "3.11.1");
        // a shim resolves through pyenv
        assert_eq!(
            python_version(&root.join("shims/python3"), &project).as_deref(),
            Some("3.11.1")
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
