//! Subshells (Warp "Warpify subshells"): a command that starts another
//! interactive shell Rift's hooks do not reach — a container's (`docker
//! exec -it c bash`), a virtualenv's (`poetry shell`) — gets an offer to
//! load them there. Accepted, Rift types a one-line bootstrap into that
//! shell: the hooks for bash or zsh, inline (base64, so the line has no
//! newline: between the lines of a multi-line one the shell's terminal is
//! in canonical mode, and macOS drops queued input past its limit), so
//! only `base64` has to exist on the other side. Plain `bash` and `zsh` started from a Rift shell load
//! the hooks by themselves (`shell/rift.bash`, `shell/rift.zsh`).

use regex::Regex;

/// Warp's subshell commands (`SUBSHELL_COMMAND_REGEXES`), less the plain
/// shells Rift's own hooks already follow into.
pub const DEFAULT_PATTERNS: &[&str] = &[
    r#"^(docker|podman)\s+(run|exec)\s+.*?['"]?(bash|zsh|fish)['"]?$"#,
    r"^poetry\s+shell",
    r"^pipenv\s+shell",
    r"^aws-vault\s+exec\b",
    r"^flox\s+(-\S+\s+)*activate\b",
];

/// Whether `command` starts a subshell worth the offer: it matches a
/// default or an `added` pattern (an invalid one is skipped) and is not in
/// `denied` (the commands "Don't ask again" was pressed for).
pub fn offers(command: &str, added: &[String], denied: &[String]) -> bool {
    let command = command.trim();
    if command.is_empty() || denied.iter().any(|d| d.trim() == command) {
        return false;
    }
    DEFAULT_PATTERNS
        .iter()
        .copied()
        .chain(added.iter().map(String::as_str))
        .any(|p| Regex::new(p).is_ok_and(|re| re.is_match(command)))
}

/// The line typed into the subshell started by `command`: fish's hooks when
/// a word of it names fish (fish parses no POSIX `if`), else bash's or zsh's,
/// whichever shell it is, evaluated once (`__rift_loaded`). A leading space
/// keeps it out of the history where the shell ignores such lines.
pub fn bootstrap(command: &str) -> String {
    use base64::Engine as _;
    // a word of the command is fish (`docker exec -it c fish`,
    // `/usr/bin/fish --no-config`)
    let fish = command.split_whitespace().any(|w| {
        let w = w.trim_matches(['\'', '"']);
        w.rsplit('/').next() == Some("fish")
    });
    if fish {
        let b64 = base64::engine::general_purpose::STANDARD
            .encode(strip(rift_core::shell_integration::RIFT_FISH));
        return format!(" printf %s {} | base64 -d | source\r", quote(&b64));
    }
    posix_bootstrap()
}

/// The bash/zsh line of [`bootstrap`].
fn posix_bootstrap() -> String {
    use base64::Engine as _;
    let encode = |src: &str| base64::engine::general_purpose::STANDARD.encode(strip(src));
    let bash = encode(rift_core::shell_integration::RIFT_BASH);
    let zsh = encode(rift_core::shell_integration::RIFT_ZSH);
    let run = |b64: &str| format!("eval \"$(printf %s {} | base64 -d)\"", quote(b64));
    format!(
        " if [ -z \"${{__rift_loaded-}}\" ]; then if [ -n \"${{ZSH_VERSION-}}\" ]; then {}; \
         elif [ -n \"${{BASH_VERSION-}}\" ]; then {}; fi; fi\r",
        run(&zsh),
        run(&bash)
    )
}

/// A hooks file without its comment-only and blank lines (what is typed
/// into the shell stays short).
fn strip(src: &str) -> String {
    src.lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.is_empty() && !t.starts_with('#')
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `s` in single quotes, its own single quotes closed and escaped.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_for_container_and_environment_shells() {
        for cmd in [
            "docker exec -it web bash",
            "docker run --rm -it ubuntu:24.04 'bash'",
            "podman exec -it db zsh",
            "poetry shell",
            "pipenv shell",
            "aws-vault exec prod",
            "flox -v activate",
        ] {
            assert!(offers(cmd, &[], &[]), "{cmd}");
        }
        for cmd in ["bash", "zsh", "docker ps", "docker exec web ls", "ls"] {
            assert!(!offers(cmd, &[], &[]), "{cmd}");
        }
        // added patterns, invalid ones skipped, and the denylist
        let added = vec![r"^kubectl\s+exec\b".to_string(), "(".to_string()];
        assert!(offers("kubectl exec -it pod -- bash", &added, &[]));
        let denied = vec!["poetry shell".to_string()];
        assert!(!offers(" poetry shell ", &[], &denied));
    }

    #[test]
    fn fish_gets_its_own_line() {
        for cmd in [
            "docker exec -it web fish",
            "podman run -it img '/usr/bin/fish'",
        ] {
            let line = bootstrap(cmd);
            assert!(line.ends_with("| base64 -d | source\r"), "{cmd}");
        }
        assert!(bootstrap("docker exec -it web bash").contains("BASH_VERSION"));
        assert!(bootstrap("poetry shell").contains("ZSH_VERSION"));
    }

    #[test]
    fn the_bootstrap_is_one_quoted_command() {
        let line = bootstrap("poetry shell");
        assert!(line.starts_with(" if [ -z \"${__rift_loaded-}\" ]"));
        assert!(line.ends_with("fi; fi\r"));
        // one line: no newline before the final Enter
        assert!(!line.trim_end_matches('\r').contains(['\n', '\r']));
        assert_eq!(quote("a'b"), r"'a'\''b'");
        // what it decodes to: the hooks, without their comment lines
        use base64::Engine as _;
        let payloads: Vec<String> = line
            .split("printf %s '")
            .skip(1)
            .map(|p| p.split('\'').next().unwrap())
            .map(|b| {
                String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b).unwrap())
                    .unwrap()
            })
            .collect();
        assert_eq!(payloads.len(), 2);
        assert!(
            payloads[0].contains("add-zsh-hook") && payloads[1].contains("__rift_prompt_command")
        );
        assert!(
            !payloads
                .iter()
                .any(|p| p.lines().any(|l| l.trim_start().starts_with('#')))
        );
        // a shell parses it: bash -n checks the syntax without running it
        let out = std::process::Command::new("bash")
            .arg("-n")
            .arg("-c")
            .arg(line.trim_end_matches('\r'))
            .output()
            .expect("bash");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
