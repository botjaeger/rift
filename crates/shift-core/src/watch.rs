use notify::{RecursiveMode, Watcher};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Watch the config dir (config.toml + themes/) and invoke `on_change` after
/// a short debounce — editors fire several events per save.
pub struct ConfigWatcher {
    _watcher: notify::RecommendedWatcher,
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
            while rx.recv().is_ok() {
                // coalesce bursts
                let deadline = Instant::now() + debounce;
                while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                    if rx.recv_timeout(left).is_err() {
                        break;
                    }
                }
                on_change();
            }
        })
        .ok()?;

    Some(ConfigWatcher { _watcher: watcher })
}
