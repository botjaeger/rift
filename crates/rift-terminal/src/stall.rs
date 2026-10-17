//! Main-thread stall log: when the event loop spends longer than
//! [`THRESHOLD`] in one pass (from `new_events` to the end of
//! `about_to_wait`), or the process takes that long to end after the loop
//! returned, a report goes to `<data dir>/stalls/`: what was under way, and
//! on macOS every thread's stack (`/usr/bin/sample`). A window that never
//! draws and a Quit that does nothing are both a blocked main thread; the
//! report names the call that blocked it. Its end ("lasted …") is added to
//! the same file. The newest [`KEEP`] reports are kept.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A pass longer than this is a stall.
pub const THRESHOLD: Duration = Duration::from_secs(2);
/// Reports kept in `stalls/`.
const KEEP: usize = 10;
/// How often the watcher looks.
const TICK: Duration = Duration::from_millis(250);

static START: OnceLock<Instant> = OnceLock::new();
/// Milliseconds since [`START`] when the current pass began.
static SINCE: AtomicU64 = AtomicU64::new(0);
static BUSY: AtomicBool = AtomicBool::new(false);
static PHASE: AtomicU8 = AtomicU8::new(Phase::Running as u8);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Phase {
    /// Inside a pass of the event loop.
    Running = 0,
    /// The event loop returned; the process is ending.
    Quitting = 1,
}

impl Phase {
    fn name(v: u8) -> &'static str {
        if v == Phase::Quitting as u8 {
            "ending the process after the event loop returned"
        } else {
            "one pass of the event loop (startup included)"
        }
    }
}

fn now_ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Start the watcher (once, first thing in `main`).
pub fn start() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        now_ms();
        let spawned = std::thread::Builder::new()
            .name("stall-watch".into())
            .spawn(watch);
        if spawned.is_err() {
            log::warn!("stall: could not start the watcher");
        }
    });
}

/// A pass of the event loop begins (`new_events`), or `phase` begins.
pub fn busy(phase: Phase) {
    SINCE.store(now_ms(), Ordering::Release);
    PHASE.store(phase as u8, Ordering::Release);
    BUSY.store(true, Ordering::Release);
}

/// Held for `about_to_wait`: the pass ends when it returns, on any path.
pub struct PassEnd;

impl Drop for PassEnd {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

/// The pass that began at `since` (ms), seen at `now`, has stalled.
fn stalled(busy: bool, since: u64, now: u64, threshold: Duration) -> bool {
    busy && now.saturating_sub(since) > threshold.as_millis() as u64
}

fn watch() {
    // the pass a report was written for, and its file
    let mut reported: Option<(u64, PathBuf)> = None;
    loop {
        std::thread::sleep(TICK);
        let (busy, since, now) = (
            BUSY.load(Ordering::Acquire),
            SINCE.load(Ordering::Acquire),
            now_ms(),
        );
        if let Some((pass, file)) = &reported
            && (!busy || since != *pass)
        {
            // a pass that began since ended the blocked one by then
            let end = if busy { since } else { now };
            let lasted = end.saturating_sub(*pass);
            append(file, &format!("lasted about {lasted} ms\n"));
            log::warn!("stall: the main thread was blocked for about {lasted} ms");
            reported = None;
        }
        if reported.is_none() && stalled(busy, since, now, THRESHOLD) {
            let phase = PHASE.load(Ordering::Acquire);
            reported = report(now - since, phase).map(|f| (since, f));
        }
    }
}

/// Write a report; its path.
fn report(blocked_ms: u64, phase: u8) -> Option<PathBuf> {
    let dir = rift_core::shell_integration::data_dir()?.join("stalls");
    std::fs::create_dir_all(&dir).ok()?;
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let pid = std::process::id();
    let file = dir.join(format!("stall-{at}-{pid}.txt"));
    let head = format!(
        "rift-terminal {} pid {pid} at {at} (unix)\nthe main thread is blocked: {}\nblocked for {blocked_ms} ms so far\n\n",
        env!("CARGO_PKG_VERSION"),
        Phase::name(phase),
    );
    std::fs::write(&file, head).ok()?;
    log::warn!(
        "stall: main thread blocked {blocked_ms} ms; report {}",
        file.display()
    );
    #[cfg(target_os = "macos")]
    {
        // on its own thread: `sample` can take many seconds on a loaded
        // machine, and the watcher must still see the stall end. If the
        // process ends first, `sample` (its own process) still writes the
        // stacks to the file the report names.
        let stacks = dir.join(format!("stall-{at}-{pid}.sample.txt"));
        append(
            &file,
            &format!(
                "stacks: below when `sample` finished, else in {}\n\n",
                stacks.display()
            ),
        );
        let report = file.clone();
        let spawned = std::thread::Builder::new()
            .name("stall-sample".into())
            .spawn(move || {
                let sampled = std::process::Command::new("/usr/bin/sample")
                    .args([&pid.to_string(), "1", "-mayDie", "-file"])
                    .arg(&stacks)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
                if sampled && let Ok(text) = std::fs::read_to_string(&stacks) {
                    append(&report, &text);
                    let _ = std::fs::remove_file(&stacks);
                }
            });
        if spawned.is_err() {
            log::warn!("stall: could not start sampling");
        }
    }
    prune(&dir, KEEP);
    Some(file)
}

fn append(file: &Path, text: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(file) {
        let _ = f.write_all(text.as_bytes());
    }
}

/// Keep the newest `keep` reports (their names sort by time).
fn prune(dir: &Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut reports: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with("stall-") && n.ends_with(".txt") && !n.ends_with(".sample.txt")
            })
        })
        .collect();
    reports.sort();
    let extra = reports.len().saturating_sub(keep);
    for p in &reports[..extra] {
        let _ = std::fs::remove_file(p);
        // and its stacks, when `sample` outlived the process
        let _ = std::fs::remove_file(p.with_extension("sample.txt"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_busy_pass_past_the_threshold_is_a_stall() {
        let t = Duration::from_secs(2);
        assert!(!stalled(false, 0, 10_000, t), "idle between passes");
        assert!(!stalled(true, 1_000, 3_000, t), "exactly the threshold");
        assert!(stalled(true, 1_000, 3_001, t));
        // a pass that began after the watcher read the clock
        assert!(!stalled(true, 5_000, 4_000, t));
    }

    #[test]
    fn prune_keeps_the_newest_reports_only() {
        let dir = std::env::temp_dir().join(format!("rift-stall-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for at in 1..=4 {
            std::fs::write(dir.join(format!("stall-{at}-1.txt")), "").unwrap();
        }
        std::fs::write(dir.join("other.txt"), "").unwrap();
        // stacks `sample` wrote after its process ended go with their report
        std::fs::write(dir.join("stall-1-1.sample.txt"), "").unwrap();
        std::fs::write(dir.join("stall-4-1.sample.txt"), "").unwrap();
        prune(&dir, 2);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "other.txt",
                "stall-3-1.txt",
                "stall-4-1.sample.txt",
                "stall-4-1.txt"
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
