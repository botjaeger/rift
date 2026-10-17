//! Completion latency benchmark (docs/performance.md, "Completion latency").
//!
//! ```sh
//! cargo test --release -p rift-core --test completion_bench -- --ignored --nocapture
//! ```
//!
//! Each scenario builds its fixture in a scratch directory, warms up once,
//! then times `REQUESTS` calls of `completion::complete` (the work the
//! completion thread does per keystroke), rotating through a few buffers.
//! It prints one markdown row per scenario and fails when a p95 is over
//! `TARGET_P95`.

use rift_core::completion::complete;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const REQUESTS: usize = 300;
const TARGET_P95: Duration = Duration::from_millis(5);

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rift-bench-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Stats {
    p50: Duration,
    p95: Duration,
    max: Duration,
}

fn measure(buffers: &[&str], history: &[String], shell: &[String], cwd: &Path) -> Stats {
    let cwd = cwd.to_str();
    for b in buffers {
        complete(b, b.len(), history, shell, cwd); // warm-up
    }
    let mut times: Vec<Duration> = (0..REQUESTS)
        .map(|i| {
            let b = buffers[i % buffers.len()];
            let t = Instant::now();
            let out = complete(b, b.len(), history, shell, cwd);
            let dt = t.elapsed();
            std::hint::black_box(out);
            dt
        })
        .collect();
    times.sort();
    Stats {
        p50: times[times.len() / 2],
        p95: times[times.len() * 95 / 100],
        max: times[times.len() - 1],
    }
}

fn row(name: &str, s: &Stats) -> bool {
    println!(
        "| {name} | {:.2} ms | {:.2} ms | {:.2} ms |",
        s.p50.as_secs_f64() * 1e3,
        s.p95.as_secs_f64() * 1e3,
        s.max.as_secs_f64() * 1e3
    );
    s.p95 <= TARGET_P95
}

#[test]
#[ignore = "benchmark: run with --release -- --ignored --nocapture"]
fn completion_latency() {
    println!("| Scenario | p50 | p95 | max |\n|---|---|---|---|");
    let mut ok = true;

    // 10 000 files and 50 directories in one directory
    let dir = scratch("dir");
    for i in 0..10_000 {
        std::fs::write(dir.join(format!("file_{i:05}.txt")), "").unwrap();
    }
    for i in 0..50 {
        std::fs::create_dir(dir.join(format!("dir_{i:02}"))).unwrap();
    }

    // 5 000 loose branches (in 50 directories) and 20 000 packed tags
    let repo = scratch("git");
    let git = repo.join(".git");
    std::fs::create_dir_all(git.join("refs/tags")).unwrap();
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    for t in 0..50 {
        let team = git.join(format!("refs/heads/team-{t:02}"));
        std::fs::create_dir_all(&team).unwrap();
        for f in 0..100 {
            std::fs::write(team.join(format!("feature-{f:03}")), "0\n").unwrap();
        }
    }
    let mut packed = String::from("# pack-refs with: peeled fully-peeled sorted\n");
    for i in 0..20_000 {
        packed.push_str(&format!(
            "{:040x} refs/tags/v{}.{}.{}\n",
            i,
            i / 1000,
            (i / 10) % 100,
            i % 10
        ));
    }
    std::fs::write(git.join("packed-refs"), packed).unwrap();

    // Files written in the last 2 s are never served from a cache (a second
    // change in the same timestamp tick could hide): measure the steady
    // state of a directory and repository nobody is writing to right now.
    std::thread::sleep(Duration::from_millis(2100));

    ok &= row(
        "path, 10 000-file directory",
        &measure(
            &[
                "cat ./file_01",
                "cat ./",
                "cat ./zz",
                "cd ./dir_",
                "cat ./file_0999",
            ],
            &[],
            &[],
            &dir,
        ),
    );
    ok &= row(
        "git refs, 5 000 loose + 20 000 packed",
        &measure(
            &[
                "git checkout te",
                "git checkout ",
                "git checkout v1.2",
                "git checkout team-07/feature-0",
            ],
            &[],
            &[],
            &repo,
        ),
    );

    // 50 000 shell history lines + 200 pane entries
    let shell: Vec<String> = (0..50_000)
        .map(|i| {
            format!(
                "git commit -m 'change number {i}' --author someone{}",
                i % 97
            )
        })
        .collect();
    let pane: Vec<String> = (0..200)
        .map(|i| format!("cargo test -p crate{i}"))
        .collect();
    let home = scratch("hist");
    ok &= row(
        "history, 50 000 lines",
        &measure(
            &[
                "git commit -m 'change number 4999",
                "ls -la /nowhere",
                "cargo te",
                "git co",
            ],
            &pane,
            &shell,
            &home,
        ),
    );

    // first word: $PATH commands
    ok &= row(
        "command names ($PATH)",
        &measure(&["gi", "ca", "py", "zzq"], &[], &[], &home),
    );

    for d in [dir, repo, home] {
        let _ = std::fs::remove_dir_all(d);
    }
    assert!(ok, "a p95 is over {TARGET_P95:?}");
}
