/// Completion providers for the input editor (design 04): PATH commands,
/// spec-driven subcommands / flags / argument generators (see
/// [`crate::completion_specs`]), filesystem paths, and shell history.
/// Prefix-based; the UI layer ranks.
use crate::completion_specs::{self, ArgKind};

#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// Full replacement for the token being completed.
    pub insert: String,
    pub description: String,
    pub source: &'static str,
}

/// (start_byte, token) being completed — the word containing `cursor`.
pub fn current_token(buffer: &str, cursor: usize) -> (usize, &str) {
    let before = &buffer[..cursor];
    let start = before
        .rfind(char::is_whitespace)
        .map(|i| i + 1)
        .unwrap_or(0);
    (start, &before[start..])
}

/// Executable names on `$PATH`, scanned once per process (sorted, deduped).
pub fn path_commands() -> &'static [String] {
    static CMDS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    CMDS.get_or_init(|| {
        let mut names: Vec<String> = Vec::new();
        for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in entries.flatten() {
                let is_exec = e
                    .metadata()
                    .map(|m| {
                        use std::os::unix::fs::PermissionsExt;
                        !m.is_dir() && m.permissions().mode() & 0o111 != 0
                    })
                    .unwrap_or(false);
                if is_exec && let Some(n) = e.file_name().to_str() {
                    names.push(n.to_string());
                }
            }
        }
        names.sort();
        names.dedup();
        names
    })
}

/// Gather completions for the token at `cursor`. `history` is the pane's
/// own (newest last); `shell_history` the imported shell history.
pub fn complete(
    buffer: &str,
    cursor: usize,
    history: &[String],
    shell_history: &[String],
    cwd: Option<&str>,
) -> Vec<Completion> {
    let (token_start, token) = current_token(buffer, cursor);
    let mut out = Vec::new();

    // Which command are we completing inside? A pipe / `;` / `&&` / `||`
    // starts a fresh command, so only look at the current segment.
    let seg_start = completion_specs::segment_start(&buffer[..token_start]);
    let seg_prefix = &buffer[seg_start..token_start];
    let first_word = seg_prefix.trim().is_empty();

    // command names: first word of the segment, from $PATH (Warp-style)
    if first_word
        && token.len() >= 2
        && !token.contains('/')
        && !token.starts_with('.')
        && !token.starts_with('~')
    {
        let cmds = path_commands();
        let start = cmds.partition_point(|c| c.as_str() < token);
        for name in cmds[start..].iter().take_while(|c| c.starts_with(token)) {
            if name != token {
                out.push(Completion {
                    insert: name.clone(),
                    description: String::new(),
                    source: "command",
                });
            }
            if out.len() >= 12 {
                break;
            }
        }
    }

    // spec-driven: subcommands, options, and argument generators
    if !first_word {
        let words = completion_specs::tokenize(seg_prefix);
        if let Some((cmd, args)) = words.split_first()
            && let Some(spec) = completion_specs::lookup(cmd)
        {
            let res = completion_specs::resolve(spec, args);
            if token.starts_with('-') {
                out.extend(option_completions(&res, token));
            } else {
                if res.offer_subcommands {
                    for s in res.subcommands {
                        if s.name.starts_with(token) && s.name != token {
                            out.push(Completion {
                                insert: s.name.into(),
                                description: s.desc.into(),
                                source: "shell",
                            });
                        }
                    }
                }
                out.extend(generator_completions(res.args, token, cwd));
            }
        }
    }

    // paths: token that looks like one
    if token.contains('/') || token.starts_with('.') || token.starts_with('~') {
        out.extend(complete_path(token, cwd));
    }

    // history: whole-buffer prefix matches (drives ghost text); the pane's
    // own entries first, then the imported shell history (newest first)
    if !buffer.trim().is_empty() {
        let mut n = 0;
        for h in history
            .iter()
            .rev()
            .take(200)
            .chain(shell_history.iter().rev().take(3000))
        {
            if h.starts_with(buffer) && h != buffer && !h.contains('\n') {
                out.push(Completion {
                    insert: h.clone(),
                    description: String::new(),
                    source: "history",
                });
                n += 1;
                if n >= 5 {
                    break;
                }
            }
        }
    }

    out.dedup_by(|a, b| a.insert == b.insert);
    out.truncate(50);
    out
}

/// Option / flag completions for a token beginning with `-`.
fn option_completions(res: &completion_specs::Resolved, token: &str) -> Vec<Completion> {
    let key = token.split('=').next().unwrap_or(token);
    let mut out = Vec::new();
    for o in &res.options {
        // Choose the form to offer: bare "-" lists every option (long form
        // preferred); "--…" matches long forms; "-x" matches short forms.
        let form: Option<&str> = if token == "-" {
            if !o.long.is_empty() {
                Some(o.long)
            } else if !o.short.is_empty() {
                Some(o.short)
            } else {
                None
            }
        } else if token.starts_with("--") {
            (!o.long.is_empty() && o.long.starts_with(key)).then_some(o.long)
        } else {
            (!o.short.is_empty() && o.short.starts_with(key)).then_some(o.short)
        };
        let Some(form) = form else { continue };
        if form == token {
            continue;
        }
        // skip a non-repeatable flag already present in the command
        if !o.repeatable
            && res.used_options.iter().any(|u| {
                let uk = u.split('=').next().unwrap_or(u);
                (!o.short.is_empty() && uk == o.short) || (!o.long.is_empty() && uk == o.long)
            })
        {
            continue;
        }
        out.push(Completion {
            insert: form.to_string(),
            description: o.desc.to_string(),
            source: "option",
        });
    }
    out
}

/// Values produced by a spec argument generator, prefix-filtered by `token`.
fn generator_completions(kind: ArgKind, token: &str, cwd: Option<&str>) -> Vec<Completion> {
    let vals_to_completions =
        |items: Vec<(String, String)>, src: &'static str| -> Vec<Completion> {
            items
                .into_iter()
                .filter(|(v, _)| v.starts_with(token))
                .map(|(v, desc)| Completion {
                    insert: v,
                    description: desc,
                    source: src,
                })
                .take(50)
                .collect()
        };
    // git refs carry a per-ref kind ("branch"/"remote"/"tag") in the desc.
    let refs = |items: Vec<(String, String)>| -> Vec<Completion> {
        items
            .into_iter()
            .filter(|(v, _)| v.starts_with(token))
            .map(|(v, desc)| {
                let src = match desc.as_str() {
                    "branch" => "branch",
                    "tag" => "tag",
                    _ => "remote-branch",
                };
                Completion {
                    insert: v,
                    description: desc,
                    source: src,
                }
            })
            .take(50)
            .collect()
    };
    match kind {
        ArgKind::None => Vec::new(),
        ArgKind::Path => complete_path(token, cwd),
        ArgKind::Dir => complete_path(token, cwd)
            .into_iter()
            .filter(|c| c.insert.ends_with('/'))
            .collect(),
        ArgKind::GitRef => refs(completion_specs::git_refs(cwd)),
        ArgKind::GitBranch => refs(
            completion_specs::git_refs(cwd)
                .into_iter()
                .filter(|(_, d)| d == "branch")
                .collect(),
        ),
        ArgKind::GitRemote => vals_to_completions(completion_specs::git_remotes(cwd), "remote"),
        ArgKind::SshHost => vals_to_completions(completion_specs::ssh_hosts(), "ssh-host"),
        ArgKind::MakeTarget => {
            vals_to_completions(completion_specs::make_targets(cwd), "make-target")
        }
        ArgKind::PkgScript => vals_to_completions(completion_specs::package_scripts(cwd), "script"),
    }
}

fn complete_path(token: &str, cwd: Option<&str>) -> Vec<Completion> {
    // Expand only "~/…": expansion must not change the basename, or the
    // orig_dir arithmetic below breaks (bare "~" expanding to $HOME made
    // file_part longer than the token itself → slice-index underflow panic).
    let expanded = if let Some(rest) = token.strip_prefix("~/") {
        match std::env::var("HOME") {
            Ok(home) => format!("{home}/{rest}"),
            Err(_) => token.to_string(),
        }
    } else {
        token.to_string()
    };

    let (dir_part, file_part) = match expanded.rfind('/') {
        Some(i) => (&expanded[..=i], &expanded[i + 1..]),
        None => ("", expanded.as_str()),
    };

    let base = if dir_part.starts_with('/') {
        std::path::PathBuf::from(dir_part)
    } else {
        std::path::PathBuf::from(cwd.unwrap_or(".")).join(dir_part)
    };

    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };

    let mut out: Vec<Completion> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with(file_part) || (file_part.is_empty() && name.starts_with('.')) {
                return None;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            // keep the user's original dir prefix (incl. ~) in the insert
            let orig_dir = &token[..token.len().saturating_sub(file_part.len())];
            Some(Completion {
                insert: format!("{orig_dir}{name}{}", if is_dir { "/" } else { "" }),
                description: if is_dir {
                    "directory".into()
                } else {
                    String::new()
                },
                source: "path",
            })
        })
        .collect();
    out.sort_by(|a, b| a.insert.cmp(&b.insert));
    out.truncate(30);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_subcommands() {
        let out = complete("git ch", 6, &[], &[], None);
        let names: Vec<&str> = out.iter().map(|c| c.insert.as_str()).collect();
        assert!(names.contains(&"checkout"));
        assert!(names.contains(&"cherry-pick"));
        assert!(names.contains(&"check-ignore"));
        assert!(!names.contains(&"commit"));
    }

    #[test]
    fn history_prefix() {
        let history = vec!["git push origin main".to_string(), "ls -la".to_string()];
        let out = complete("git pu", 6, &history, &[], None);
        assert!(out.iter().any(|c| c.insert == "git push origin main"));
    }

    #[test]
    fn token_extraction() {
        assert_eq!(current_token("git che", 7), (4, "che"));
        assert_eq!(current_token("ls", 2), (0, "ls"));
        assert_eq!(current_token("cargo build ", 12), (12, ""));
    }

    #[test]
    fn bare_tilde_does_not_panic() {
        // regression: "~" expanded to $HOME made file_part longer than the
        // token → slice-index underflow panic on every '~' keystroke
        complete("~", 1, &[], &[], None);
        complete("ls ~", 4, &[], &[], None);
        complete("ls ~ab", 6, &[], &[], None);
        complete("cat ~/", 6, &[], &[], None);
    }

    #[test]
    fn tilde_slash_expands_and_keeps_prefix() {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        if std::fs::read_dir(&home).is_err() {
            return;
        }
        let out = complete("ls ~/", 5, &[], &[], None);
        // every path completion must re-use the user's "~/" prefix verbatim
        assert!(
            out.iter()
                .filter(|c| c.source == "path")
                .all(|c| c.insert.starts_with("~/"))
        );
    }

    #[test]
    fn path_commands_complete_first_word_only() {
        // `ls` exists on every unix PATH; completing "l" needs 2 chars
        assert!(complete("l", 1, &[], &[], None).is_empty());
        let out = complete("ls", 2, &[], &[], None);
        assert!(
            out.iter()
                .all(|c| c.source == "command" && c.insert.starts_with("ls"))
        );
        assert!(out.iter().all(|c| c.insert != "ls"));
        // not the first word: no command completion
        let out = complete("echo ls", 7, &[], &[], None);
        assert!(out.iter().all(|c| c.source != "command"));
    }

    #[test]
    fn shell_history_feeds_ghost_after_pane_history() {
        let pane = vec!["git push origin main".to_string()];
        let shell = vec!["git pull".to_string(), "git push --force".to_string()];
        let out = complete("git pu", 6, &pane, &shell, None);
        let hist: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "history")
            .map(|c| c.insert.as_str())
            .collect();
        assert_eq!(
            hist,
            vec!["git push origin main", "git push --force", "git pull"]
        );
        // multi-line entries never become ghost text
        let shell = vec!["git pu\nll".to_string()];
        let out = complete("git pu", 6, &[], &shell, None);
        assert!(out.iter().all(|c| c.source != "history"));
    }

    #[test]
    fn empty_token_after_space_lists_subcommands() {
        let out = complete("git ", 4, &[], &[], None);
        let subs: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "shell")
            .map(|c| c.insert.as_str())
            .collect();
        assert!(subs.contains(&"commit"));
        assert!(subs.contains(&"status"));
    }

    #[test]
    fn nested_subcommand_completion() {
        let out = complete("docker compose ", 15, &[], &[], None);
        let subs: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "shell")
            .map(|c| c.insert.as_str())
            .collect();
        assert!(subs.contains(&"up"), "docker compose <sub>: {subs:?}");
        assert!(subs.contains(&"down"));
    }

    #[test]
    fn option_completion() {
        // "-" lists every option (long form preferred)
        let out = complete("git commit -", 12, &[], &[], None);
        assert!(
            out.iter().any(|c| c.source == "option"
                && c.insert == "--message"
                && !c.description.is_empty())
        );
        // "--a" matches long forms by prefix
        let out = complete("git commit --a", 14, &[], &[], None);
        assert!(
            out.iter()
                .any(|c| c.source == "option" && c.insert == "--amend")
        );
        assert!(
            out.iter()
                .all(|c| c.source != "option" || c.insert.starts_with("--a"))
        );
        // a non-repeatable flag already present is not offered again
        let out = complete("git commit --amend --a", 22, &[], &[], None);
        assert!(out.iter().all(|c| c.insert != "--amend"));
    }

    #[test]
    fn pipe_starts_a_new_command() {
        // subcommand completion resolves against the command after the pipe
        let out = complete("cat foo | git ch", 16, &[], &[], None);
        let names: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "shell")
            .map(|c| c.insert.as_str())
            .collect();
        assert!(names.contains(&"checkout"), "after pipe: {names:?}");
        assert!(!names.contains(&"commit"));
        // and `;` / `&&` behave the same
        let out = complete("x=1; git ch", 11, &[], &[], None);
        assert!(
            out.iter()
                .any(|c| c.source == "shell" && c.insert == "checkout")
        );
    }

    #[test]
    fn git_checkout_completes_refs_from_repo() {
        // a temp dir with fake git ref files; no real git invoked
        let dir = std::env::temp_dir().join(format!(
            "runway-complete-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(dir.join(".git/refs/heads")).unwrap();
        std::fs::write(dir.join(".git/refs/heads/main"), "0\n").unwrap();
        std::fs::write(dir.join(".git/refs/heads/develop"), "0\n").unwrap();

        let cwd = dir.to_str().unwrap();
        let out = complete("git checkout ", 13, &[], &[], Some(cwd));
        let branches: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "branch")
            .map(|c| c.insert.as_str())
            .collect();
        assert!(branches.contains(&"main"), "refs: {branches:?}");
        assert!(branches.contains(&"develop"));
        // prefix filtering still applies
        let out = complete("git checkout de", 15, &[], &[], Some(cwd));
        assert!(out.iter().any(|c| c.insert == "develop"));
        assert!(
            out.iter()
                .all(|c| c.source != "branch" || c.insert.starts_with("de"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
