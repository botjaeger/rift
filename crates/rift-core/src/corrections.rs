//! Command corrections from a failed command's output (Warp, thefuck-style).
//!
//! [`correct`] looks at a command that failed, its exit code, the tail of
//! its output and the directory it ran in, and proposes the command to run
//! instead. Rules, in order:
//!
//! * **git push without an upstream** — the `git push --set-upstream
//!   <remote> <branch>` git prints.
//! * **git "did you mean"** — the first of git's "most similar" commands.
//! * **`cd` to a mistyped directory** — the closest existing directory,
//!   component by component (Damerau-Levenshtein distance ≤ 2).
//! * **a script without the execute bit** (`./x.sh`: permission denied,
//!   exit 126) — `chmod +x ./x.sh && ./x.sh`.
//! * **permission denied / EACCES** — the same command under `sudo`.
//!
//! A suggestion is only offered (accepted with → in the empty editor), never
//! run. Output is untrusted: a rule only takes tokens that match a tight
//! pattern (a git ref name, a git subcommand) from it, never free text;
//! everything else comes from the command the user typed or from the file
//! system (directory names are shell-escaped).

use std::path::{Path, PathBuf};

use crate::completion::escape_word;
use crate::shell_env::closest_command;

/// Output looked at: its last this-many bytes.
const MAX_OUTPUT: usize = 16 * 1024;

/// A command that finished with a non-zero exit code.
#[derive(Debug, Clone, Copy)]
pub struct Failed<'a> {
    /// The command line as run.
    pub command: &'a str,
    pub exit: i32,
    /// Its output (the tail is enough; longer text is cut to `MAX_OUTPUT`).
    pub output: &'a str,
    /// The local directory it ran in (`None`: unknown or on another host).
    pub cwd: Option<&'a Path>,
}

/// The command to run instead of `f`, or `None`.
pub fn correct(f: &Failed) -> Option<String> {
    if f.exit == 0 || f.command.contains('\n') {
        return None;
    }
    let output = tail(f.output, MAX_OUTPUT);
    let words: Vec<&str> = f.command.split_whitespace().collect();
    git_set_upstream(&words, output)
        .or_else(|| git_did_you_mean(f.command, &words, output))
        .or_else(|| cd_typo(&words, output, f.cwd))
        .or_else(|| chmod_script(f, &words, output))
        .or_else(|| sudo_retry(f, &words, output))
}

/// The last `max` bytes of `s` (on a char boundary).
fn tail(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut at = s.len() - max;
    while !s.is_char_boundary(at) {
        at += 1;
    }
    &s[at..]
}

/// A git ref or remote name as git prints it: no blanks, quotes or shell
/// characters, not an option.
fn ref_token(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 200
        && !t.starts_with('-')
        && !t.contains("..")
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/@+-".contains(c))
}

/// A git subcommand name: lowercase letters, digits and dashes.
fn subcommand_token(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 40
        && t.starts_with(|c: char| c.is_ascii_lowercase())
        && t.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Characters that make a command more than one simple command.
fn compound(command: &str) -> bool {
    command.contains(['|', ';', '&', '<', '>', '`', '(', ')', '$'])
}

/// `git push` on a branch without an upstream: the command git suggests,
/// rebuilt from its two tokens; the user's own `git push` options stay.
fn git_set_upstream(words: &[&str], output: &str) -> Option<String> {
    if words.len() < 2 || words[0] != "git" || words[1] != "push" {
        return None;
    }
    if !output.contains("has no upstream branch") {
        return None;
    }
    let (remote, branch) = output.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("git push --set-upstream ")?;
        let mut it = rest.split_whitespace();
        let (remote, branch) = (it.next()?, it.next()?);
        (it.next().is_none() && ref_token(remote) && ref_token(branch)).then_some((remote, branch))
    })?;
    let mut out = format!("git push --set-upstream {remote} {branch}");
    for w in &words[2..] {
        // the user's flags (`-f`); anything else named a remote or branch
        if !w.starts_with('-') || compound(w) {
            return None;
        }
        out.push(' ');
        out.push_str(w);
    }
    Some(out)
}

/// `git <typo>`: git's first "most similar" command in place of the typo.
fn git_did_you_mean(command: &str, words: &[&str], output: &str) -> Option<String> {
    if words.len() < 2 || words[0] != "git" {
        return None;
    }
    let typo = words[1];
    if !output.contains(&format!("git: '{typo}' is not a git command")) {
        return None;
    }
    let mut lines = output.lines();
    lines.find(|l| l.starts_with("The most similar command"))?;
    let fix = lines.next()?.trim();
    if !subcommand_token(fix) || fix == typo {
        return None;
    }
    // splice over the typo's own position, after `git`
    let git_end = command.find("git")? + 3;
    let at = git_end + command[git_end..].find(typo)?;
    Some(format!(
        "{}{fix}{}",
        &command[..at],
        &command[at + typo.len()..]
    ))
}

/// `cd <path>` that does not exist: each missing component replaced by the
/// closest directory in its parent.
fn cd_typo(words: &[&str], output: &str, cwd: Option<&Path>) -> Option<String> {
    if words.len() != 2 || words[0] != "cd" {
        return None;
    }
    if !output.to_lowercase().contains("no such file or directory") {
        return None;
    }
    let arg = words[1];
    // quoted or escaped arguments, variables, globs: not a plain path
    if arg.contains(['\'', '"', '\\', '$', '*', '?', '[', '`']) || arg.starts_with('-') {
        return None;
    }
    let (mut base, mut out, rest) = if let Some(rest) = arg.strip_prefix("~/") {
        (
            PathBuf::from(std::env::var_os("HOME")?),
            String::from("~/"),
            rest,
        )
    } else if let Some(rest) = arg.strip_prefix('/') {
        (PathBuf::from("/"), String::from("/"), rest)
    } else {
        (cwd?.to_path_buf(), String::new(), arg)
    };
    if base.join(rest).is_dir() {
        return None;
    }
    let mut changed = false;
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        let name = if matches!(*part, "." | "..") || base.join(part).is_dir() {
            part.to_string()
        } else {
            let items = crate::completion_cache::dir_items(&base);
            let dirs: Vec<&str> = items
                .iter()
                .filter(|d| d.is_dir && (part.starts_with('.') || !d.name.starts_with('.')))
                .map(|d| d.name.as_str())
                .collect();
            changed = true;
            closest_command(part, &dirs)?.to_string()
        };
        base.push(&name);
        out.push_str(&escape_word(&name)?);
        if i + 1 < parts.len() {
            out.push('/');
        }
    }
    (changed && base.is_dir()).then(|| format!("cd {out}"))
}

/// `./script` refused (exit 126) for want of the execute bit.
fn chmod_script(f: &Failed, words: &[&str], output: &str) -> Option<String> {
    let first = *words.first()?;
    if f.exit != 126 || !first.contains('/') || compound(f.command) {
        return None;
    }
    if !output.to_lowercase().contains("permission denied") {
        return None;
    }
    if first.contains(['\'', '"', '\\', '*', '?', '[', '~']) {
        return None;
    }
    let path = if first.starts_with('/') {
        PathBuf::from(first)
    } else {
        f.cwd?.join(first)
    };
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || executable(&meta) {
        return None;
    }
    Some(format!("chmod +x {first} && {}", f.command.trim()))
}

#[cfg(unix)]
fn executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_: &std::fs::Metadata) -> bool {
    true
}

/// Builtins and commands `sudo` cannot run or would not help.
const NO_SUDO: &[&str] = &[
    "sudo", "doas", "cd", "export", "source", ".", "alias", "unalias", "exit", "set", "unset",
    "pushd", "popd", "exec", "eval", "builtin", "command", "ssh", "su",
];

/// "Permission denied" or EACCES from a simple command: the same command
/// under `sudo`. A redirection's denial is the shell's (sudo would not
/// cover it), and a script refused for its mode is `chmod_script`'s.
fn sudo_retry(f: &Failed, words: &[&str], output: &str) -> Option<String> {
    let first = *words.first()?;
    if f.exit == 126 || NO_SUDO.contains(&first) || first.contains('=') || compound(f.command) {
        return None;
    }
    if !(output.to_lowercase().contains("permission denied") || output.contains("EACCES")) {
        return None;
    }
    Some(format!("sudo {}", f.command.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUSH: &str = include_str!("../testdata/corrections/git-push-no-upstream.txt");
    const TYPO_ONE: &str = include_str!("../testdata/corrections/git-typo-one.txt");
    const TYPO_MANY: &str = include_str!("../testdata/corrections/git-typo-many.txt");
    const BASH_CD: &str = include_str!("../testdata/corrections/bash-cd.txt");
    const ZSH_CD: &str = include_str!("../testdata/corrections/zsh-cd.txt");
    const BASH_EXEC: &str = include_str!("../testdata/corrections/bash-exec-denied.txt");
    const ZSH_EXEC: &str = include_str!("../testdata/corrections/zsh-exec-denied.txt");
    const MKDIR: &str = include_str!("../testdata/corrections/mkdir-denied.txt");
    const REDIRECT: &str = include_str!("../testdata/corrections/bash-redirect-denied.txt");
    const NODE: &str = include_str!("../testdata/corrections/node-eacces.txt");

    fn fix(command: &str, exit: i32, output: &str, cwd: Option<&Path>) -> Option<String> {
        correct(&Failed {
            command,
            exit,
            output,
            cwd,
        })
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rift-corr-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn git_push_sets_the_upstream_git_names() {
        assert_eq!(
            fix("git push", 128, PUSH, None).as_deref(),
            Some("git push --set-upstream origin feat/x-1")
        );
        // the user's own flags stay
        assert_eq!(
            fix("git push -f", 128, PUSH, None).as_deref(),
            Some("git push --set-upstream origin feat/x-1 -f")
        );
        // not a push, or another failure: nothing
        assert_eq!(fix("git pull", 128, PUSH, None), None);
        assert_eq!(
            fix("git push", 1, "error: failed to push some refs", None),
            None
        );
        // an explicit remote/branch: git's line would drop it
        assert_eq!(fix("git push origin main", 128, PUSH, None), None);
    }

    #[test]
    fn git_push_output_is_never_interpolated() {
        // a hostile "branch" with shell characters is not a ref token
        let evil = PUSH.replace("origin feat/x-1", "origin x;rm${IFS}-rf");
        assert_eq!(fix("git push", 128, &evil, None), None);
        let evil = PUSH.replace("origin feat/x-1", "origin $(id)");
        assert_eq!(fix("git push", 128, &evil, None), None);
        let evil = PUSH.replace("origin feat/x-1", "origin main extra");
        assert_eq!(fix("git push", 128, &evil, None), None);
    }

    #[test]
    fn git_did_you_mean_takes_gits_first_suggestion() {
        assert_eq!(
            fix("git stauts", 1, TYPO_ONE, None).as_deref(),
            Some("git status")
        );
        assert_eq!(
            fix("git st -s", 1, TYPO_MANY, None).as_deref(),
            Some("git status -s")
        );
        // output about another word does not apply
        assert_eq!(fix("git sttus", 1, TYPO_ONE, None), None);
        // a suggestion that is not a plain subcommand name
        let evil = TYPO_ONE.replace("\tstatus", "\tstatus; rm -rf ~");
        assert_eq!(fix("git stauts", 1, &evil, None), None);
    }

    #[test]
    fn cd_to_the_closest_directory() {
        let dir = scratch("cd");
        std::fs::create_dir_all(dir.join("projects/src")).unwrap();
        std::fs::create_dir_all(dir.join("my docs")).unwrap();
        std::fs::write(dir.join("projcts.txt"), "").unwrap();
        for out in [BASH_CD, ZSH_CD] {
            assert_eq!(
                fix("cd projcts", 1, out, Some(&dir)).as_deref(),
                Some("cd projects")
            );
        }
        // every component, kept where it exists
        assert_eq!(
            fix("cd projcts/scr", 1, BASH_CD, Some(&dir)).as_deref(),
            Some("cd projects/src")
        );
        assert_eq!(
            fix("cd projects/sr", 1, BASH_CD, Some(&dir)).as_deref(),
            Some("cd projects/src")
        );
        // a name with a blank comes back escaped
        assert_eq!(
            fix("cd my_docs", 1, BASH_CD, Some(&dir)).as_deref(),
            Some("cd my\\ docs")
        );
        // absolute paths too
        let abs = format!("cd {}/projets", dir.display());
        assert_eq!(
            fix(&abs, 1, BASH_CD, None),
            Some(format!("cd {}/projects", dir.display()))
        );
        // nothing close, an existing directory, unknown cwd, other output
        assert_eq!(fix("cd zzzzzz", 1, BASH_CD, Some(&dir)), None);
        assert_eq!(fix("cd projects", 1, BASH_CD, Some(&dir)), None);
        assert_eq!(fix("cd projcts", 1, BASH_CD, None), None);
        assert_eq!(
            fix(
                "cd projcts",
                1,
                "cd: permission denied: projcts",
                Some(&dir)
            ),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chmod_a_script_without_the_execute_bit() {
        let dir = scratch("chmod");
        std::fs::write(dir.join("run.sh"), "#!/bin/sh\necho ran\n").unwrap();
        std::fs::create_dir_all(dir.join("adir")).unwrap();
        for out in [BASH_EXEC, ZSH_EXEC] {
            assert_eq!(
                fix("./run.sh --fast", 126, out, Some(&dir)).as_deref(),
                Some("chmod +x ./run.sh && ./run.sh --fast")
            );
        }
        // executable already, a directory, missing, another exit code
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let exe = dir.join("ok.sh");
            std::fs::write(&exe, "").unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(fix("./ok.sh", 126, BASH_EXEC, Some(&dir)), None);
        }
        assert_eq!(fix("./adir", 126, BASH_EXEC, Some(&dir)), None);
        assert_eq!(fix("./gone.sh", 126, BASH_EXEC, Some(&dir)), None);
        assert_eq!(
            fix("./run.sh", 1, BASH_EXEC, Some(&dir)),
            Some("sudo ./run.sh".into())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sudo_for_a_denied_command() {
        assert_eq!(
            fix("mkdir /etc/riftx", 1, MKDIR, None).as_deref(),
            Some("sudo mkdir /etc/riftx")
        );
        assert_eq!(
            fix("node m.js", 1, NODE, None).as_deref(),
            Some("sudo node m.js")
        );
        // a redirection's denial: sudo would not help
        assert_eq!(fix("echo hi > /etc/riftx.conf", 1, REDIRECT, None), None);
        // already sudo, a builtin, a pipeline, unrelated output
        assert_eq!(fix("sudo mkdir /etc/riftx", 1, MKDIR, None), None);
        assert_eq!(fix("cd /private/var/root", 1, MKDIR, None), None);
        assert_eq!(fix("cat x | grep y", 1, MKDIR, None), None);
        assert_eq!(
            fix(
                "mkdir /etc/riftx",
                1,
                "mkdir: /etc/riftx: File exists",
                None
            ),
            None
        );
        // success never gets a correction
        assert_eq!(fix("mkdir /etc/riftx", 0, MKDIR, None), None);
    }

    #[test]
    fn only_the_output_tail_is_read() {
        let long = format!("{}{PUSH}", "x".repeat(MAX_OUTPUT * 2));
        assert_eq!(
            fix("git push", 128, &long, None).as_deref(),
            Some("git push --set-upstream origin feat/x-1")
        );
        let buried = format!("{PUSH}{}", "x\n".repeat(MAX_OUTPUT));
        assert_eq!(fix("git push", 128, &buried, None), None);
        assert_eq!(tail("héllo", 4), "llo");
    }
}
