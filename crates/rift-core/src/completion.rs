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

/// (start_byte, token) being completed — the shell word containing
/// `cursor`, as typed. Whitespace or an operator (`|;&<>()`) ends the word
/// (as in [`token_end`]) unless escaped with `\` or inside quotes.
pub fn current_token(buffer: &str, cursor: usize) -> (usize, &str) {
    let before = &buffer[..cursor];
    let mut start = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in before.char_indices() {
        if escaped {
            escaped = false;
        } else if quote == Some(c) {
            quote = None;
        } else if quote.is_some() {
            escaped = c == '\\' && quote == Some('"');
        } else if c == '\\' {
            escaped = true;
        } else if c == '\'' || c == '"' {
            quote = Some(c);
        } else if c.is_whitespace() || "|;&<>()".contains(c) {
            start = i + c.len_utf8();
        }
    }
    (start, &before[start..])
}

/// End byte of the shell word containing `cursor` (see [`current_token`]):
/// the first unquoted, unescaped whitespace or `|;&<>()` at or after
/// `cursor`. Completion replaces `start..end`, so a closing quote or the
/// rest of a word the cursor sits inside is replaced, not kept.
pub fn token_end(buffer: &str, cursor: usize) -> usize {
    let (start, _) = current_token(buffer, cursor);
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in buffer[start..].char_indices() {
        let at = start + i;
        if escaped {
            escaped = false;
        } else if quote == Some(c) {
            quote = None;
        } else if quote.is_some() {
            escaped = c == '\\' && quote == Some('"');
        } else if c == '\\' {
            escaped = true;
        } else if c == '\'' || c == '"' {
            quote = Some(c);
        } else if at >= cursor && (c.is_whitespace() || "|;&<>()".contains(c)) {
            return at;
        }
    }
    buffer.len()
}

/// The literal text of a typed shell word: quotes removed, `\x` → `x`.
fn unescape_word(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    let mut quote: Option<char> = None;
    let mut chars = token.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '\\') => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            // inside "…" a backslash escapes only `$ ` " \` and newline
            // (bash, zsh and fish alike); before anything else it is literal
            (Some('"'), '\\') => match chars.clone().next() {
                Some(n) if "$`\"\\\n".contains(n) => {
                    chars.next();
                    out.push(n);
                }
                _ => out.push('\\'),
            },
            _ => out.push(c),
        }
    }
    out
}

/// Backslash-escape `s` as one bash/zsh/fish word. A leading `~/` stays
/// unescaped so it still expands. `None` for control characters, which no
/// form portable across the three shells can carry.
pub fn escape_word(s: &str) -> Option<String> {
    if s.chars().any(char::is_control) {
        return None;
    }
    let (lead, rest) = match s.strip_prefix("~/") {
        Some(rest) => ("~/", rest),
        None => ("", s),
    };
    let mut out = String::from(lead);
    for (i, c) in rest.char_indices() {
        let at_start = lead.is_empty() && i == 0;
        let safe =
            c.is_alphanumeric() || "/._-+,:@%".contains(c) || (!at_start && "=~".contains(c));
        if !safe {
            out.push('\\');
        }
        out.push(c);
    }
    Some(out)
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
    // a spec path/dir argument is completed by its generator alone
    let mut generic_paths = true;

    // Which command are we completing inside? A pipe / `;` / `&&` / `||`
    // starts a fresh command, so only look at the current segment.
    let seg_start = completion_specs::segment_start(&buffer[..token_start]);
    let seg_prefix = &buffer[seg_start..token_start];
    let first_word = seg_prefix.trim().is_empty();

    // command names: first word of the segment, from $PATH (Warp-style).
    // Names are filenames: insert them escaped so `;`, quotes or `$` in a
    // basename stay one word; names with control characters are skipped.
    if first_word {
        out.extend(command_completions(path_commands(), token));
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
                generic_paths = !matches!(res.args, ArgKind::Path | ArgKind::Dir);
                out.extend(generator_completions(res.args, token, cwd));
            }
        }
    }

    // paths: token that looks like one
    let word = unescape_word(token);
    if generic_paths && (word.contains('/') || word.starts_with('.') || word.starts_with('~')) {
        out.extend(complete_path(&word, cwd));
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

    let mut seen = std::collections::HashSet::new();
    out.retain(|c| seen.insert(c.insert.clone()));
    out.truncate(50);
    out
}

/// Command-name completions for `token` (as typed) from the sorted `cmds`,
/// at most 12. Inserts are [`escape_word`]-escaped.
fn command_completions(cmds: &[String], token: &str) -> Vec<Completion> {
    let typed = unescape_word(token);
    if typed.chars().count() < 2
        || typed.contains('/')
        || typed.starts_with('.')
        || typed.starts_with('~')
    {
        return Vec::new();
    }
    let start = cmds.partition_point(|c| c.as_str() < typed.as_str());
    cmds[start..]
        .iter()
        .take_while(|c| c.starts_with(&typed))
        .filter(|c| **c != typed)
        .filter_map(|name| escape_word(name))
        .take(12)
        .map(|insert| Completion {
            insert,
            description: String::new(),
            source: "command",
        })
        .collect()
}

/// Option / flag completions for a token beginning with `-`.
fn option_completions(res: &completion_specs::Resolved, token: &str) -> Vec<Completion> {
    // `--opt=va…`: the value is not an option name, and an option insert
    // would replace the whole token and drop `=va…`
    if token.contains('=') {
        return Vec::new();
    }
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
    // generator values are literal text: match against the unescaped token
    // and insert escaped, so a ref named `x$(cmd)` cannot run on Enter
    let want = unescape_word(token);
    let vals_to_completions =
        |items: Vec<(String, String)>, src: &'static str| -> Vec<Completion> {
            items
                .into_iter()
                .filter(|(v, _)| v.starts_with(&want))
                .filter_map(|(v, desc)| {
                    Some(Completion {
                        insert: escape_word(&v)?,
                        description: desc,
                        source: src,
                    })
                })
                .take(50)
                .collect()
        };
    // git refs carry a per-ref kind ("branch"/"remote"/"tag") in the desc.
    let refs = |items: Vec<(String, String)>| -> Vec<Completion> {
        items
            .into_iter()
            .filter(|(v, _)| v.starts_with(&want))
            .filter_map(|(v, desc)| {
                let src = match desc.as_str() {
                    "branch" => "branch",
                    "tag" => "tag",
                    _ => "remote-branch",
                };
                Some(Completion {
                    insert: escape_word(&v)?,
                    description: desc,
                    source: src,
                })
            })
            .take(50)
            .collect()
    };
    match kind {
        ArgKind::None => Vec::new(),
        ArgKind::Path => complete_path(&unescape_word(token), cwd),
        ArgKind::Dir => complete_path(&unescape_word(token), cwd)
            .into_iter()
            .filter(|c| c.insert.ends_with('/'))
            .collect(),
        ArgKind::GitRef => refs(completion_specs::git_refs(cwd, &want)),
        ArgKind::GitBranch => refs(
            completion_specs::git_refs(cwd, &want)
                .into_iter()
                .filter(|(_, d)| d == "branch")
                .collect(),
        ),
        ArgKind::GitRemote => vals_to_completions(completion_specs::git_remotes(cwd), "remote"),
        ArgKind::SshHost => vals_to_completions(completion_specs::ssh_hosts(&want), "ssh-host"),
        ArgKind::MakeTarget => {
            vals_to_completions(completion_specs::make_targets(cwd, &want), "make-target")
        }
        ArgKind::PkgScript => {
            vals_to_completions(completion_specs::package_scripts(cwd, &want), "script")
        }
    }
}

/// Entries matching the literal (unescaped) path `token`; each `insert` is
/// escaped as one shell word.
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
            // follow symlinks: a link to a directory is a directory
            let is_dir = std::fs::metadata(e.path())
                .map(|m| m.is_dir())
                .or_else(|_| e.file_type().map(|t| t.is_dir()))
                .unwrap_or(false);
            // keep the user's original dir prefix (incl. ~) in the insert
            let orig_dir = &token[..token.len().saturating_sub(file_part.len())];
            Some(Completion {
                insert: escape_word(&format!(
                    "{orig_dir}{name}{}",
                    if is_dir { "/" } else { "" }
                ))?,
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
    fn command_names_are_escaped() {
        let mut cmds: Vec<String> = [
            "zzprobe;echo X",
            "zzsafe",
            "zz q'\"$`\\",
            "zz\tctl",
            "zz sp",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        cmds.sort();
        let inserts = |token: &str| -> Vec<String> {
            command_completions(&cmds, token)
                .into_iter()
                .map(|c| c.insert)
                .collect()
        };
        let all = inserts("zz");
        assert!(all.contains(&"zzprobe\\;echo\\ X".to_string()));
        assert!(all.contains(&"zzsafe".to_string()));
        assert!(all.contains(&"zz\\ q\\'\\\"\\$\\`\\\\".to_string()));
        assert!(all.contains(&"zz\\ sp".to_string()));
        // control characters cannot be carried portably: not offered
        assert!(!all.iter().any(|s| s.contains('\t')));
        // a typed escaped prefix matches the real name
        assert_eq!(inserts("zz\\ s"), vec!["zz\\ sp".to_string()]);
        // round trip: the shell word unescapes to exactly the filename
        for name in &cmds {
            if let Some(w) = escape_word(name) {
                assert_eq!(&unescape_word(&w), name);
            }
        }
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
    fn token_respects_escapes_and_quotes() {
        assert_eq!(current_token("cat my\\ di", 10), (4, "my\\ di"));
        assert_eq!(current_token("cat 'my di", 10), (4, "'my di"));
        assert_eq!(current_token("cat \"a b\" c", 11), (10, "c"));
        assert_eq!(current_token("echo é f", 9), (8, "f"));
    }

    #[test]
    fn double_quotes_keep_backslashes_that_escape_nothing() {
        assert_eq!(unescape_word(r#""./a\b""#), r"./a\b");
        assert_eq!(unescape_word(r#""a\$b\`c\"d\\e""#), r#"a$b`c"d\e"#);
        assert_eq!(unescape_word(r"./a\b"), "./ab"); // unquoted: escapes any char
        assert_eq!(unescape_word(r"'./a\b'"), r"./a\b"); // single quotes: literal
        assert_eq!(unescape_word(r#""a\"#), r"a\"); // trailing backslash stays
    }

    #[test]
    fn quoted_backslash_prefix_picks_the_backslash_file() {
        let dir = std::env::temp_dir().join(format!("rift-complete-bs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(r"d\x")).unwrap();
        std::fs::write(dir.join(r"a\b.txt"), "").unwrap();
        std::fs::write(dir.join("ab.txt"), "").unwrap();
        std::fs::write(dir.join(r"d\x/in.txt"), "").unwrap();
        let cwd = dir.to_str();
        let paths = |buf: &str| -> Vec<String> {
            complete(buf, buf.len(), &[], &[], cwd)
                .into_iter()
                .filter(|c| c.source == "path")
                .map(|c| c.insert)
                .collect()
        };
        assert_eq!(paths(r#"cat "./a\b""#), [r"./a\\b.txt"]);
        assert_eq!(paths(r#"cat "./a\b"#), [r"./a\\b.txt"]);
        assert_eq!(paths(r"cat ./a\\b"), [r"./a\\b.txt"]);
        assert_eq!(paths(r"cat ./ab"), ["./ab.txt"]);
        assert_eq!(paths(r#"cat "./d\x/i"#), [r"./d\\x/in.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn token_end_spans_the_whole_word() {
        // closing quote right of the cursor belongs to the word
        let b = "cat \"./hel\"";
        assert_eq!(token_end(b, b.len() - 1), b.len());
        // mid-word: the suffix up to the next argument is part of the word
        let b = "cat ./hello.txt --other";
        assert_eq!(token_end(b, 9), 15);
        // end of buffer and at a space
        assert_eq!(token_end("cat ./hel", 9), 9);
        assert_eq!(token_end("cat  x", 4), 4);
        // quoted / escaped whitespace does not end the word
        let b = "cat 'my di' x";
        assert_eq!(token_end(b, 7), 11);
        let b = "cat my\\ di x";
        assert_eq!(token_end(b, 5), 10);
        // operators end it; whitespace before the cursor never counts
        assert_eq!(token_end("ls ./a|grep x", 5), 6);
        assert_eq!(token_end("echo é\u{3000}f", 7), 7);
    }

    #[test]
    fn escaped_words_round_trip() {
        for name in [
            "hello world.txt",
            "it's",
            "say \"hi\"",
            "$HOME;rm (x)",
            "back\\slash",
            "a&b|c<d>e*f?g[h]{i}!j#k`l^",
            "café",
            "=eq",
            "~tilde",
        ] {
            let escaped = escape_word(name).unwrap();
            assert!(
                !escaped.contains(' ') || escaped.contains("\\ "),
                "{escaped}"
            );
            assert_eq!(
                current_token(&escaped, escaped.len()),
                (0, escaped.as_str())
            );
            assert_eq!(unescape_word(&escaped), name, "{escaped}");
        }
        assert_eq!(escape_word("~/a b").unwrap(), "~/a\\ b");
        assert_eq!(escape_word("café/x-1.txt").unwrap(), "café/x-1.txt");
        assert_eq!(escape_word("line\nbreak"), None);
    }

    #[test]
    fn path_completion_inserts_one_shell_word() {
        let dir = std::env::temp_dir().join(format!("rift-complete-quote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("my dir")).unwrap();
        std::fs::write(dir.join("hello world.txt"), "").unwrap();
        std::fs::write(dir.join("it's;$x.txt"), "").unwrap();
        std::fs::write(dir.join("my dir/inner.txt"), "").unwrap();
        let cwd = dir.to_str();
        let path_inserts = |buf: &str| -> Vec<String> {
            complete(buf, buf.len(), &[], &[], cwd)
                .into_iter()
                .filter(|c| c.source == "path")
                .map(|c| c.insert)
                .collect()
        };
        assert_eq!(path_inserts("cat ./hel"), ["./hello\\ world.txt"]);
        assert_eq!(path_inserts("cat ./it"), ["./it\\'s\\;\\$x.txt"]);
        // directory continuation: the escaped prefix completes further
        assert_eq!(path_inserts("cat ./my"), ["./my\\ dir/"]);
        assert_eq!(path_inserts("cat ./my\\ dir/in"), ["./my\\ dir/inner.txt"]);
        // a quoted token resolves too, and is replaced by the escaped form
        assert_eq!(path_inserts("cat './my dir/in"), ["./my\\ dir/inner.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
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
            "rift-complete-{}-{}",
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

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rift-complete-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn generator_values_are_escaped_and_matched_unescaped() {
        let dir = scratch("inject");
        let heads = dir.join(".git/refs/heads");
        std::fs::create_dir_all(&heads).unwrap();
        let evil = "x$(touch${IFS}pwn);id";
        std::fs::write(heads.join(evil), "0\n").unwrap();
        let cwd = dir.to_str();
        let want = escape_word(evil).unwrap();
        assert!(!want.contains("$(") || want.contains("\\$\\("));
        for buf in [
            "git checkout x",
            "git checkout x\\$\\(to",
            "git checkout 'x$(to",
        ] {
            let out = complete(buf, buf.len(), &[], &[], cwd);
            let got: Vec<&str> = out
                .iter()
                .filter(|c| c.source == "branch")
                .map(|c| c.insert.as_str())
                .collect();
            assert_eq!(got, [want.as_str()], "{buf}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn script_and_remote_generators_escape_too() {
        let dir = scratch("inject2");
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts": {"x;touch pwn": "echo", "ok": "echo"}}"#,
        )
        .unwrap();
        let cwd = dir.to_str();
        let out = complete("npm run x", 9, &[], &[], cwd);
        let got: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "script")
            .map(|c| c.insert.as_str())
            .collect();
        assert_eq!(got, ["x\\;touch\\ pwn"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dir_completion_has_no_files_and_no_duplicates() {
        let dir = scratch("dirs");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("file.txt"), "").unwrap();
        let cwd = dir.to_str();
        let inserts = |buf: &str| -> Vec<String> {
            complete(buf, buf.len(), &[], &[], cwd)
                .into_iter()
                .map(|c| c.insert)
                .collect()
        };
        assert_eq!(inserts("cd ./"), ["./sub/"]);
        assert_eq!(inserts("cat ./"), ["./file.txt", "./sub/"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directories_complete_as_directories() {
        let dir = scratch("symlink");
        std::fs::create_dir_all(dir.join("real")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
        let cwd = dir.to_str();
        let out = complete("cd ./li", 7, &[], &[], cwd);
        assert_eq!(
            out.iter().map(|c| c.insert.as_str()).collect::<Vec<_>>(),
            ["./link/"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn option_with_a_value_is_not_completed_as_an_option() {
        let out = complete("git commit --mess", 17, &[], &[], None);
        assert!(out.iter().any(|c| c.insert == "--message"));
        let out = complete("git commit --message=al", 23, &[], &[], None);
        assert!(out.iter().all(|c| c.source != "option"), "{out:?}");
    }

    #[test]
    fn redirects_and_newlines_bound_the_token_and_the_command() {
        assert_eq!(current_token("cat a >fi", 9), (7, "fi"));
        assert_eq!(current_token("a|ls", 4), (2, "ls"));
        assert_eq!(current_token("cat 'a>b", 8), (4, "'a>b"));
        let out = complete("ls\ngit ch", 9, &[], &[], None);
        assert!(
            out.iter()
                .any(|c| c.source == "shell" && c.insert == "checkout")
        );
    }
}
