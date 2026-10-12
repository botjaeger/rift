//! The pull request of a repository's checked-out branch, for the prompt's
//! GitHub PR chip: `gh pr view` on a background thread, cached per
//! repository and branch for a while (it costs a network round trip).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    pub number: u64,
    pub url: String,
    pub state: PrState,
}

/// `gh pr view --json number,url,state,isDraft` output.
pub fn parse(json: &str) -> Option<PrInfo> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let url = v.get("url")?.as_str()?;
    if !url.starts_with("https://") {
        return None;
    }
    let state = match (v.get("state")?.as_str()?, v.get("isDraft")?.as_bool()) {
        ("MERGED", _) => PrState::Merged,
        ("CLOSED", _) => PrState::Closed,
        (_, Some(true)) => PrState::Draft,
        _ => PrState::Open,
    };
    Some(PrInfo {
        number: v.get("number")?.as_u64()?,
        url: url.to_string(),
        state,
    })
}

/// How long a looked-up branch's answer is reused.
const FRESH: Duration = Duration::from_secs(120);
/// A `gh` that has not answered by then is killed.
const TIMEOUT: Duration = Duration::from_secs(15);

/// `gh` on PATH, else where Homebrew puts it (an app started from the
/// Finder has a short PATH).
fn find_gh() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin".into(), "/usr/local/bin".into()])
        .map(|d| d.join("gh"))
        .find(|p| p.is_file())
}

fn query(dir: &Path) -> Option<PrInfo> {
    let mut child = std::process::Command::new(find_gh()?)
        .args(["pr", "view", "--json", "number,url,state,isDraft"])
        .current_dir(dir)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let mut out = String::new();
    use std::io::Read as _;
    child
        .stdout
        .take()?
        .take(64 * 1024)
        .read_to_string(&mut out)
        .ok()?;
    parse(&out)
}

pub type PrCallback = Arc<dyn Fn(u64, Option<PrInfo>) + Send + Sync>;

/// Background `gh` worker: one lookup at a time, answers per pane.
pub struct PrWorker {
    tx: Sender<(u64, PathBuf, String)>,
}

impl PrWorker {
    pub fn new(callback: PrCallback) -> Self {
        let (tx, rx) = channel::<(u64, PathBuf, String)>();
        std::thread::Builder::new()
            .name("github-pr".into())
            .spawn(move || {
                let mut cache: HashMap<(PathBuf, String), (Instant, Option<PrInfo>)> =
                    HashMap::new();
                while let Ok((pane, dir, branch)) = rx.recv() {
                    let key = (dir, branch);
                    let info = match cache.get(&key) {
                        Some((at, info)) if at.elapsed() < FRESH => info.clone(),
                        _ => {
                            let info = query(&key.0);
                            cache.insert(key, (Instant::now(), info.clone()));
                            info
                        }
                    };
                    callback(pane, info);
                }
            })
            .expect("spawn github worker");
        PrWorker { tx }
    }

    /// Look up the PR of `branch` for the repository at `dir`.
    pub fn request(&self, pane: u64, dir: PathBuf, branch: String) {
        let _ = self.tx.send((pane, dir, branch));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gh_output() {
        let pr = parse(
            r#"{"isDraft":false,"number":42,"state":"OPEN","url":"https://github.com/o/r/pull/42"}"#,
        )
        .unwrap();
        assert_eq!(
            pr,
            PrInfo {
                number: 42,
                url: "https://github.com/o/r/pull/42".into(),
                state: PrState::Open
            }
        );
        let draft = r#"{"isDraft":true,"number":1,"state":"OPEN","url":"https://x/1"}"#;
        assert_eq!(parse(draft).unwrap().state, PrState::Draft);
        let merged = r#"{"isDraft":false,"number":1,"state":"MERGED","url":"https://x/1"}"#;
        assert_eq!(parse(merged).unwrap().state, PrState::Merged);
        // only a web link is ever opened
        let odd = r#"{"isDraft":false,"number":1,"state":"OPEN","url":"file:///etc/passwd"}"#;
        assert_eq!(parse(odd), None);
        assert_eq!(parse("no pull requests found"), None);
    }
}
