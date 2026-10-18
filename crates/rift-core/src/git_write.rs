//! The review panel's repository writes that go through the `git` program
//! rather than git2: commit (so the user's hooks run) and push (so their
//! credential helpers, ssh config and remotes behave as in a terminal).
//!
//! Every call passes its arguments as an array (never a shell string), with
//! no terminal to prompt on: `GIT_TERMINAL_PROMPT=0`, ssh in batch mode
//! unless the user configured their own ssh command, stdin closed, its own
//! process group. A push never forces.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Longest a commit (hooks included) or a push may take.
pub const TIMEOUT: Duration = Duration::from_secs(120);

/// What a push needs to know about the repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushState {
    /// The checked-out branch; `None` on a detached HEAD.
    pub branch: Option<String>,
    /// HEAD names a branch that has no commit yet.
    pub unborn: bool,
    /// The branch's upstream: (remote, merge ref such as `refs/heads/main`).
    pub upstream: Option<(String, String)>,
    /// Every configured remote.
    pub remotes: Vec<String>,
}

/// The `git` arguments that push `state`'s branch: to its upstream when it
/// has one of the same name, else `push -u <remote> <branch>` to the only
/// remote or to `origin`. An upstream with another name is refused (like
/// `push.default=simple`): the branch never lands on a different remote
/// branch. Never a force push.
pub fn push_args(state: &PushState) -> Result<Vec<String>, String> {
    if state.unborn {
        return Err("No commits yet: commit first".into());
    }
    let Some(branch) = &state.branch else {
        return Err("Not on a branch (detached HEAD): nothing to push".into());
    };
    // `--` keeps a remote or branch that looks like an option an operand
    if let Some((remote, merge)) = &state.upstream {
        if *merge != format!("refs/heads/{branch}") {
            let theirs = merge.strip_prefix("refs/heads/").unwrap_or(merge);
            return Err(format!(
                "Upstream is {remote}/{theirs}, not {branch}: push from a terminal or set the upstream"
            ));
        }
        return Ok(vec![
            "push".into(),
            "--".into(),
            remote.clone(),
            format!("HEAD:{merge}"),
        ]);
    }
    let remote = match state.remotes.as_slice() {
        [] => return Err("No remote to push to: add one with `git remote add`".into()),
        [only] => only.as_str(),
        many if many.iter().any(|r| r == "origin") => "origin",
        _ => {
            return Err(
                "Several remotes and none named origin: set this branch's upstream first".into(),
            );
        }
    };
    Ok(["push", "--set-upstream", "--", remote, branch.as_str()]
        .map(str::to_string)
        .to_vec())
}

/// The push state of the repository at `root`.
pub fn push_state(root: &Path) -> Result<PushState, String> {
    let repo = git2::Repository::open(root).map_err(|e| e.message().to_string())?;
    let branch = repo
        .head()
        .ok()
        .filter(|h| h.is_branch())
        .and_then(|h| h.shorthand().ok().map(str::to_string));
    // an unborn branch has no HEAD commit, but HEAD still names it
    let unborn = repo
        .find_reference("HEAD")
        .ok()
        .and_then(|h| h.symbolic_target().ok().flatten().map(str::to_string))
        .is_some_and(|target| repo.find_reference(&target).is_err());
    let upstream = branch.as_ref().and_then(|b| {
        let config = repo.config().ok()?;
        let remote = config.get_string(&format!("branch.{b}.remote")).ok()?;
        let merge = config.get_string(&format!("branch.{b}.merge")).ok()?;
        Some((remote, merge))
    });
    let remotes = repo
        .remotes()
        .map(|r| {
            r.iter()
                .filter_map(|n| n.ok().flatten())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok(PushState {
        branch,
        unborn,
        upstream,
        remotes,
    })
}

/// Commit the staged changes of the repository at `root` with `message`
/// (hooks run). Returns git's summary line (`[main 1a2b3c4] message`); on
/// failure, what git printed (a hook's output included).
pub fn commit(root: &Path, message: &str) -> Result<String, String> {
    if message.trim().is_empty() {
        return Err("Write a commit message".into());
    }
    commit_within(root, message, TIMEOUT)
}

/// [`commit`] that gives up after `limit`.
fn commit_within(root: &Path, message: &str, limit: Duration) -> Result<String, String> {
    let args = ["commit", "--file=-"].map(str::to_string);
    let out = git(root, &args, Some(message), limit)?;
    Ok(out.lines().next().unwrap_or("Committed").trim().to_string())
}

/// Push the current branch (see [`push_args`]). Returns a summary.
pub fn push(root: &Path) -> Result<String, String> {
    let state = push_state(root)?;
    let args = push_args(&state)?;
    git(root, &args, None, TIMEOUT).map_err(|e| explain_auth(&e))?;
    let branch = state.branch.unwrap_or_default();
    Ok(match state.upstream {
        Some((remote, _)) => format!("Pushed {branch} to {remote}"),
        None => format!("Pushed {branch} to {} and set it as upstream", args[3]),
    })
}

/// A failure that needs credentials Rift cannot ask for, said plainly.
fn explain_auth(stderr: &str) -> String {
    const AUTH: [&str; 7] = [
        "terminal prompts disabled",
        "could not read Username",
        "could not read Password",
        "Permission denied (publickey",
        "Host key verification failed",
        "Authentication failed",
        "Permission denied, please try again",
    ];
    if AUTH.iter().any(|a| stderr.contains(a)) {
        let last = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        format!(
            "Push needs credentials and Rift cannot prompt for them: push once in a terminal, \
             or set up a credential helper or ssh-agent ({})",
            last.trim()
        )
    } else {
        // the refusal first: no "To <remote>" line, no hints
        let kept: Vec<&str> = stderr
            .lines()
            .filter(|l| !l.starts_with("To ") && !l.starts_with("hint:"))
            .collect();
        if kept.is_empty() {
            stderr.to_string()
        } else {
            kept.join("\n").trim().to_string()
        }
    }
}

/// Run `git args` in `root` without a terminal, `stdin` (if any) as its
/// input, stopped (with everything it started) after `limit`. Ok: its
/// stdout. Err: its stderr (else stdout), trimmed.
fn git(
    root: &Path,
    args: &[String],
    stdin: Option<&str>,
    limit: Duration,
) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // its own group: nothing it starts can read the terminal Rift was
        // launched from
        .process_group(0);
    let own_ssh = std::env::var_os("GIT_SSH_COMMAND").is_some()
        || std::env::var_os("GIT_SSH").is_some()
        || git2::Repository::open(root)
            .and_then(|r| r.config())
            .is_ok_and(|c| c.get_string("core.sshCommand").is_ok());
    if !own_ssh {
        // ssh asks for passphrases and host keys on the terminal: fail instead
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not run git: {e}"))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let _ = pipe.write_all(text.as_bytes());
    }
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_string(&mut s);
            }
            s
        })
    };
    let out = read(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = read(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > limit => {
                // the whole group (hooks, ssh, credential helpers), not only
                // git: a hook left running keeps the output pipes open
                if let Ok(pid) = libc::pid_t::try_from(child.id()) {
                    // SAFETY: plain syscall; the group is the child's own
                    // (process_group(0) above), never Rift's
                    unsafe { libc::killpg(pid, libc::SIGKILL) };
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "git {} took over {}s; stopped",
                    args[0],
                    limit.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else {
        let text = if err.trim().is_empty() { &out } else { &err };
        Err(text.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn state(branch: Option<&str>, upstream: Option<(&str, &str)>, remotes: &[&str]) -> PushState {
        PushState {
            branch: branch.map(str::to_string),
            unborn: false,
            upstream: upstream.map(|(r, m)| (r.to_string(), m.to_string())),
            remotes: remotes.iter().map(|r| r.to_string()).collect(),
        }
    }

    #[test]
    fn push_arguments_follow_the_upstream_else_set_one_and_never_force() {
        let args = |s: PushState| push_args(&s).map(|a| a.join(" "));
        assert_eq!(
            args(state(
                Some("feat"),
                Some(("up", "refs/heads/feat")),
                &["origin", "up"]
            )),
            Ok("push -- up HEAD:refs/heads/feat".into())
        );
        // an upstream with another name (`checkout -b feat origin/main`):
        // refused, never `HEAD:refs/heads/main`
        assert_eq!(
            args(state(
                Some("feat"),
                Some(("up", "refs/heads/trunk")),
                &["origin", "up"]
            )),
            Err("Upstream is up/trunk, not feat: push from a terminal or set the upstream".into())
        );
        let mut unborn = state(Some("main"), None, &["origin"]);
        unborn.unborn = true;
        assert_eq!(args(unborn), Err("No commits yet: commit first".into()));
        assert_eq!(
            args(state(Some("feat"), None, &["fork"])),
            Ok("push --set-upstream -- fork feat".into())
        );
        assert_eq!(
            args(state(Some("feat"), None, &["fork", "origin"])),
            Ok("push --set-upstream -- origin feat".into())
        );
        assert!(
            args(state(Some("feat"), None, &["a", "b"]))
                .unwrap_err()
                .contains("upstream")
        );
        assert!(
            args(state(Some("feat"), None, &[]))
                .unwrap_err()
                .contains("No remote")
        );
        assert!(
            args(state(None, None, &["origin"]))
                .unwrap_err()
                .contains("detached")
        );
        // a branch or remote that looks like an option stays an operand
        let odd = push_args(&state(Some("feat"), None, &["-f"])).unwrap();
        assert_eq!(odd, ["push", "--set-upstream", "--", "-f", "feat"]);
        for s in [
            state(
                Some("feat"),
                Some(("origin", "refs/heads/feat")),
                &["origin"],
            ),
            state(Some("feat"), None, &["origin"]),
        ] {
            let a = push_args(&s).unwrap();
            assert!(
                !a.iter()
                    .any(|w| w.starts_with("--force") || w == "-f" || w.starts_with('+')),
                "{a:?}"
            );
        }
    }

    #[test]
    fn auth_failures_say_prompts_are_not_supported() {
        let e = explain_auth(
            "fatal: could not read Username for 'https://x': terminal prompts disabled",
        );
        assert!(e.contains("cannot prompt"), "{e}");
        assert_eq!(
            explain_auth("To /r.git\n ! [rejected] main (fetch first)\nhint: pull first"),
            "! [rejected] main (fetch first)"
        );
    }

    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "rift-gitw-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn run(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@e.x",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn repo(s: &Scratch) -> PathBuf {
        let work = s.0.join("work");
        fs::create_dir_all(&work).unwrap();
        run(&work, &["init", "-q"]);
        run(&work, &["config", "user.name", "t"]);
        run(&work, &["config", "user.email", "t@e.x"]);
        // the machine's own config must not sign or hook these commits
        run(&work, &["config", "commit.gpgsign", "false"]);
        run(&work, &["config", "core.hooksPath", ".git/hooks"]);
        fs::write(work.join("a.txt"), "a\n").unwrap();
        run(&work, &["add", "a.txt"]);
        run(&work, &["commit", "-qm", "init"]);
        work
    }

    #[test]
    fn commits_the_staged_files_and_a_failing_hook_reports_its_output() {
        let s = scratch("commit");
        let work = repo(&s);
        fs::write(work.join("b.txt"), "b\n").unwrap();
        fs::write(work.join("c.txt"), "unstaged\n").unwrap();
        run(&work, &["add", "b.txt"]);
        assert!(commit(&work, "  \n").unwrap_err().contains("message"));
        let line = commit(&work, "add b\n\nbody").unwrap();
        assert!(line.contains("add b"), "{line}");
        assert_eq!(run(&work, &["log", "-1", "--format=%s"]), "add b");
        assert_eq!(run(&work, &["show", "--name-only", "--format="]), "b.txt");
        // nothing staged: git refuses
        assert!(commit(&work, "again").is_err());

        let hook = work.join(".git/hooks/pre-commit");
        fs::write(
            &hook,
            "#!/bin/sh\necho 'lint: trailing space in b.txt' >&2\nexit 1\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        run(&work, &["add", "c.txt"]);
        let e = commit(&work, "add c").unwrap_err();
        assert!(e.contains("lint: trailing space in b.txt"), "{e}");
        assert_eq!(run(&work, &["log", "-1", "--format=%s"]), "add b");
    }

    #[test]
    fn pushes_set_an_upstream_then_update_it_and_refuse_a_non_fast_forward() {
        let s = scratch("push");
        let bare = s.0.join("remote.git");
        fs::create_dir_all(&bare).unwrap();
        run(&bare, &["init", "-q", "--bare"]);
        let work = repo(&s);
        run(&work, &["checkout", "-qb", "feat"]);
        run(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);

        let first = push(&work).unwrap();
        assert!(first.contains("set it as upstream"), "{first}");
        assert_eq!(run(&work, &["config", "branch.feat.remote"]), "origin");
        fs::write(work.join("a.txt"), "a2\n").unwrap();
        run(&work, &["commit", "-qam", "two"]);
        assert_eq!(push(&work).unwrap(), "Pushed feat to origin");
        assert_eq!(run(&bare, &["log", "-1", "--format=%s", "feat"]), "two");

        // someone else pushed: ours is behind and diverged
        let other = s.0.join("other");
        run(
            &s.0,
            &[
                "clone",
                "-q",
                "-b",
                "feat",
                bare.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        fs::write(other.join("o.txt"), "o\n").unwrap();
        run(&other, &["add", "o.txt"]);
        run(&other, &["commit", "-qm", "theirs"]);
        run(&other, &["push", "-q"]);
        fs::write(work.join("a.txt"), "a3\n").unwrap();
        run(&work, &["commit", "-qam", "ours"]);
        let e = push(&work).unwrap_err();
        assert!(e.contains("rejected"), "{e}");
        assert_eq!(
            run(&bare, &["log", "-1", "--format=%s", "feat"]),
            "theirs",
            "nothing was forced"
        );
    }

    #[test]
    fn a_branch_tracking_another_name_is_not_pushed_onto_it() {
        let s = scratch("track");
        let bare = s.0.join("remote.git");
        fs::create_dir_all(&bare).unwrap();
        run(&bare, &["init", "-q", "--bare"]);
        let work = repo(&s);
        run(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        run(&work, &["push", "-q", "-u", "origin", "main"]);
        let main_before = run(&bare, &["rev-parse", "main"]);
        run(&work, &["checkout", "-qb", "feat", "origin/main"]);
        fs::write(work.join("f.txt"), "f\n").unwrap();
        run(&work, &["add", "f.txt"]);
        run(&work, &["commit", "-qm", "feat work"]);
        let e = push(&work).unwrap_err();
        assert!(e.contains("Upstream is origin/main, not feat"), "{e}");
        assert_eq!(run(&bare, &["rev-parse", "main"]), main_before);
    }

    #[test]
    fn push_on_an_unborn_branch_says_commit_first() {
        let s = scratch("unborn");
        let work = s.0.join("work");
        fs::create_dir_all(&work).unwrap();
        run(&work, &["init", "-q"]);
        run(&work, &["remote", "add", "origin", "/nonexistent"]);
        assert_eq!(push(&work).unwrap_err(), "No commits yet: commit first");
    }

    #[test]
    fn a_hook_that_outlives_the_timeout_is_stopped_with_git() {
        let s = scratch("hang");
        let work = repo(&s);
        let done = s.0.join("hook-finished");
        let hook = work.join(".git/hooks/pre-commit");
        fs::write(
            &hook,
            format!("#!/bin/sh\nsleep 3\ntouch '{}'\n", done.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(work.join("b.txt"), "b\n").unwrap();
        run(&work, &["add", "b.txt"]);
        let start = Instant::now();
        let e = commit_within(&work, "hang", Duration::from_millis(300)).unwrap_err();
        assert!(e.contains("stopped"), "{e}");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        std::thread::sleep(Duration::from_secs(4));
        assert!(
            !done.exists(),
            "the hook kept running after git was stopped"
        );
    }
}
