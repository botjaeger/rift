use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// Watch the config dir (config.toml + themes/) and invoke `on_change` after
/// a short debounce — editors fire several events per save.
pub struct ConfigWatcher {
    _watcher: notify::RecommendedWatcher,
}

/// How often the files are stat'ed as a backstop: FSEvents can deliver an
/// event many seconds late, or not at all, while fseventsd is backlogged.
const POLL: Duration = Duration::from_secs(1);

/// Size and modification time of config.toml and every file in themes/.
fn fingerprint(dir: &Path) -> Vec<(PathBuf, u64, Option<SystemTime>)> {
    let stat = |p: PathBuf| {
        let m = std::fs::metadata(&p).ok()?;
        Some((p, m.len(), m.modified().ok()))
    };
    let mut out: Vec<_> = stat(dir.join("config.toml")).into_iter().collect();
    if let Ok(entries) = std::fs::read_dir(dir.join("themes")) {
        out.extend(entries.flatten().filter_map(|e| stat(e.path())));
    }
    out.sort();
    out
}

pub fn watch(on_change: Arc<dyn Fn() + Send + Sync>) -> Option<ConfigWatcher> {
    let dir = crate::config::config_dir()?;
    std::fs::create_dir_all(&dir).ok()?;

    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            use notify::EventKind::*;
            if matches!(event.kind, Create(_) | Modify(_) | Remove(_)) {
                let _ = tx.send(());
            }
        }
    })
    .ok()?;
    watcher
        .watch(Path::new(&dir), RecursiveMode::Recursive)
        .ok()?;

    std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            let debounce = Duration::from_millis(150);
            let mut seen = fingerprint(&dir);
            loop {
                match rx.recv_timeout(POLL) {
                    Ok(()) => {
                        // coalesce bursts
                        let deadline = Instant::now() + debounce;
                        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                            if rx.recv_timeout(left).is_err() {
                                break;
                            }
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if fingerprint(&dir) == seen {
                            continue;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
                seen = fingerprint(&dir);
                on_change();
            }
        })
        .ok()?;

    Some(ConfigWatcher { _watcher: watcher })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_sees_config_and_theme_edits() {
        let dir = std::env::temp_dir().join(format!("shift-watch-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("themes")).unwrap();
        std::fs::write(dir.join("config.toml"), "a = 1\n").unwrap();
        let before = fingerprint(&dir);
        assert_eq!(before.len(), 1);
        std::fs::write(dir.join("config.toml"), "a = 12\n").unwrap();
        let config = fingerprint(&dir);
        assert_ne!(config, before);
        std::fs::write(dir.join("themes/t.toml"), "bg = 1\n").unwrap();
        assert_ne!(fingerprint(&dir), config);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
