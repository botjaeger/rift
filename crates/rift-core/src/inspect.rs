//! The command inspector (Warp "Command X-Ray"): what one part of a command
//! line is — the command, a subcommand, an option, an option's value, an
//! argument or an operator — and what it does, from the same command specs
//! completions use (`completion_specs`). A command without a description
//! here is described by the app from the system's manual (`whatis`).

use std::ops::Range;

use crate::completion_specs::{self, ArgKind, OptionSpec};
use crate::highlight::{self, TokenKind};

/// What a part of the command line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Command,
    Subcommand,
    Option,
    /// The value an option takes (`-m "msg"`).
    OptionValue,
    Argument,
    Operator,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Command => "Command",
            Role::Subcommand => "Subcommand",
            Role::Option => "Option",
            Role::OptionValue => "Option value",
            Role::Argument => "Argument",
            Role::Operator => "Operator",
        }
    }
}

/// One part of the command line, described.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    /// Byte range of the part in the buffer.
    pub range: Range<usize>,
    pub role: Role,
    /// The part as written (an option with both forms: `-m, --message`).
    pub title: String,
    /// What it does; `None` for a command Rift has no words for (the app
    /// asks the manual).
    pub text: Option<String>,
}

/// Commands that run the command after them (and their own options):
/// `sudo git commit` describes `commit` as git's.
const WRAPPERS: &[&str] = &[
    "sudo",
    "doas",
    "env",
    "nohup",
    "time",
    "nice",
    "exec",
    "command",
    "builtin",
    "caffeinate",
    "stdbuf",
    "setsid",
    "timeout",
    "xargs",
];

/// A redirection operator (`>`, `>>`, `<`, `<<`, `>&`, `&>`, `<>`): not a
/// boundary between commands.
fn is_redirect(op: &str) -> bool {
    op.starts_with('>') || op.starts_with('<') || op == "&>" || op == "&>>"
}

/// `NAME=value` before a command: an environment variable for it.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// Describe the part of `buffer` at byte `offset` (a cursor at a part's end
/// counts as on it); `None` over blanks.
pub fn describe(buffer: &str, offset: usize) -> Option<Inspection> {
    let tokens = highlight::tokenize(buffer);
    let at = tokens
        .iter()
        .position(|(r, _)| r.start <= offset && offset < r.end)
        .or_else(|| tokens.iter().position(|(r, _)| r.end == offset))?;
    let (range, kind) = tokens[at].clone();
    let raw = &buffer[range.clone()];
    let word = unquote(raw);
    let op_at = |i: usize| tokens[i].1 == TokenKind::Op;
    if kind == TokenKind::Op {
        return Some(Inspection {
            title: word.clone(),
            text: Some(operator(&word).unwrap_or("A shell operator").to_string()),
            role: Role::Operator,
            range,
        });
    }
    // a redirection's file (`> out.txt`, `2>&1`'s `1`), and the descriptor
    // number right before one (`2>`)
    if at > 0 && op_at(at - 1) && is_redirect(&buffer[tokens[at - 1].0.clone()]) {
        let op = &buffer[tokens[at - 1].0.clone()];
        let text = if op.ends_with('&') {
            format!("File descriptor {word}: where {op} sends it")
        } else if op == "<<" || op == "<<-" {
            "The word that ends the here-document".to_string()
        } else if op.starts_with('<') {
            "The file the command reads its input from".to_string()
        } else {
            format!("The file {op} writes to")
        };
        return Some(Inspection {
            range,
            role: Role::Argument,
            title: word,
            text: Some(text),
        });
    }
    if at + 1 < tokens.len()
        && op_at(at + 1)
        && tokens[at + 1].0.start == range.end
        && is_redirect(&buffer[tokens[at + 1].0.clone()])
        && word.chars().all(|c| c.is_ascii_digit())
    {
        let what = match word.as_str() {
            "0" => "standard input",
            "1" => "standard output",
            "2" => "standard error",
            _ => "a file descriptor",
        };
        return Some(Inspection {
            range,
            role: Role::Argument,
            text: Some(format!("File descriptor {word} ({what}), redirected")),
            title: word,
        });
    }
    // this command's words: since the last control operator or line break,
    // without redirections and their files
    let seg_start = (0..=at)
        .rev()
        .find(|&i| {
            (op_at(i) && !is_redirect(&buffer[tokens[i].0.clone()]))
                || (i > 0 && buffer[tokens[i - 1].0.end..tokens[i].0.start].contains('\n'))
        })
        .map_or(0, |i| if op_at(i) { i + 1 } else { i });
    let mut words: Vec<usize> = Vec::new();
    let mut skip_next = false;
    for i in seg_start..tokens.len() {
        if i > at && (op_at(i) && !is_redirect(&buffer[tokens[i].0.clone()])) {
            break;
        }
        if op_at(i) {
            skip_next = is_redirect(&buffer[tokens[i].0.clone()]);
            continue;
        }
        if std::mem::take(&mut skip_next) {
            continue;
        }
        words.push(i);
    }
    // the command: past `NAME=value` and wrappers (and their options)
    let mut cmd_at = None;
    let mut wrapper: Option<String> = None;
    let mut pos = 0;
    while pos < words.len() {
        let i = words[pos];
        let w = unquote(&buffer[tokens[i].0.clone()]);
        if is_assignment(&w) {
            if i == at {
                let name = w.split('=').next().unwrap_or_default().to_string();
                return Some(Inspection {
                    range,
                    role: Role::Argument,
                    text: Some(format!(
                        "Sets the environment variable {name} for this command"
                    )),
                    title: w,
                });
            }
            pos += 1;
            continue;
        }
        if wrapper.is_some() && w.starts_with('-') {
            if i == at {
                return Some(Inspection {
                    range,
                    role: Role::Option,
                    text: Some(format!(
                        "An option of {}",
                        wrapper.clone().unwrap_or_default()
                    )),
                    title: w,
                });
            }
            pos += 1;
            continue;
        }
        if WRAPPERS.contains(&w.as_str()) && pos + 1 < words.len() {
            if i == at {
                return Some(Inspection {
                    range,
                    role: Role::Command,
                    text: command_text(&w)
                        .map(str::to_string)
                        .or_else(|| Some(format!("Runs the command after it ({w})"))),
                    title: w,
                });
            }
            wrapper = Some(w);
            pos += 1;
            continue;
        }
        cmd_at = Some(pos);
        break;
    }
    let cmd_pos = cmd_at?;
    let cmd_i = words[cmd_pos];
    if cmd_i > at {
        return None;
    }
    let command = unquote(&buffer[tokens[cmd_i].0.clone()]);
    if at == cmd_i {
        return Some(Inspection {
            range,
            role: Role::Command,
            text: command_text(&command).map(str::to_string),
            title: command,
        });
    }
    let args: Vec<String> = words[cmd_pos + 1..]
        .iter()
        .take_while(|&&i| i < at)
        .map(|&i| unquote(&buffer[tokens[i].0.clone()]))
        .collect();
    let kind = if word.starts_with('-') && word.len() > 1 {
        TokenKind::Flag
    } else {
        kind
    };
    let spec = completion_specs::lookup(&command);
    let resolved = spec.map(|s| completion_specs::resolve(s, &args));
    let option_of = |w: &str| -> Option<OptionSpec> {
        let key = w.split('=').next().unwrap_or(w);
        resolved.as_ref()?.options.iter().copied().find(|o| {
            (!o.short.is_empty() && o.short == key) || (!o.long.is_empty() && o.long == key)
        })
    };
    if kind == TokenKind::Flag {
        return Some(match option_of(&word) {
            Some(o) => Inspection {
                range,
                role: Role::Option,
                title: option_title(&o),
                text: Some(o.desc.to_string()),
            },
            None => Inspection {
                range,
                role: Role::Option,
                text: Some(match spec {
                    Some(_) => format!("An option of {command} that Rift has no description for"),
                    None => format!("An option of {command}"),
                }),
                title: word,
            },
        });
    }
    // the value of the option right before it
    if let Some(prev) = args.last()
        && prev.starts_with('-')
        && !prev.contains('=')
        && let Some(o) = option_of(prev).filter(|o| o.takes_arg)
    {
        return Some(Inspection {
            range,
            role: Role::OptionValue,
            title: word,
            text: Some(format!("The value of {}: {}", option_title(&o), o.desc)),
        });
    }
    if let Some(r) = &resolved
        && r.offer_subcommands
        && let Some(sub) = r.subcommands.iter().find(|s| s.name == word)
    {
        return Some(Inspection {
            range,
            role: Role::Subcommand,
            title: word,
            text: Some(sub.desc.to_string()),
        });
    }
    let what = match resolved.as_ref().map(|r| r.args) {
        Some(ArgKind::Path) => "a file or folder",
        Some(ArgKind::Dir) => "a folder",
        Some(ArgKind::GitRef) => "a branch, tag or other git ref",
        Some(ArgKind::GitBranch) => "a local branch",
        Some(ArgKind::GitRemote) => "a git remote",
        Some(ArgKind::SshHost) => "a host from your ssh config",
        Some(ArgKind::MakeTarget) => "a Makefile target",
        Some(ArgKind::PkgScript) => "a package.json script",
        Some(ArgKind::None) | None => "an argument",
    };
    Some(Inspection {
        range,
        role: Role::Argument,
        title: word,
        text: Some(format!("{} of {command}", capitalize(what))),
    })
}

/// `-m, --message` (either form alone when the other is missing).
fn option_title(o: &OptionSpec) -> String {
    match (o.short.is_empty(), o.long.is_empty()) {
        (false, false) => format!("{}, {}", o.short, o.long),
        (false, true) => o.short.to_string(),
        _ => o.long.to_string(),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// A word without its surrounding quotes.
fn unquote(word: &str) -> String {
    completion_specs::tokenize(word)
        .into_iter()
        .next()
        .unwrap_or_else(|| word.to_string())
}

fn operator(op: &str) -> Option<&'static str> {
    Some(match op {
        "|" => "Pipe: the command on the left writes into the one on the right",
        "||" => "Or: runs the next command only if the one before failed",
        "&&" => "And: runs the next command only if the one before succeeded",
        ";" => "Runs the next command after this one, whatever its result",
        "&" => "Runs the command in the background",
        ">" => "Writes the output to a file, replacing it",
        ">>" => "Appends the output to a file",
        "<" => "Reads the input from a file",
        "2>" => "Writes the errors to a file",
        "&>" => "Writes the output and the errors to a file",
        "|&" => "Pipe with the errors too",
        "(" | ")" => "Runs the commands inside in a subshell",
        _ => return None,
    })
}

/// The description `whatis` prints for `command` (`git(1) - the stupid
/// content tracker`; Linux puts a space before the section, and a page can
/// name several commands, `gzip, gunzip(1) - …`), capitalized.
pub fn parse_whatis(command: &str, output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (names, desc) = line.split_once(" - ")?;
        let names = names.split('(').next()?;
        names
            .split(',')
            .any(|n| n.trim() == command)
            .then(|| capitalize(desc.trim()))
            .filter(|d| !d.is_empty())
    })
}

/// What Rift says about a command it has a spec for, or a common one; the
/// rest comes from the manual.
pub fn command_text(command: &str) -> Option<&'static str> {
    Some(match command {
        "git" => "Distributed version control",
        "cargo" => "The Rust package manager and build tool",
        "npm" => "The Node.js package manager",
        "yarn" => "A Node.js package manager",
        "pnpm" => "A fast, disk-efficient Node.js package manager",
        "docker" => "Build, run and manage containers",
        "kubectl" => "Control Kubernetes clusters",
        "brew" => "The Homebrew package manager",
        "gh" => "The GitHub command line",
        "go" => "Build and manage Go code",
        "rustup" => "Install and manage Rust toolchains",
        "systemctl" => "Control systemd services",
        "apt" | "apt-get" => "The Debian and Ubuntu package manager",
        "pip" | "pip3" => "The Python package installer",
        "make" => "Build targets from a Makefile",
        "ssh" => "Log in to and run commands on another machine",
        "scp" => "Copy files to and from another machine over ssh",
        "cd" => "Change the current directory",
        "ls" => "List directory contents",
        "grep" => "Print lines that match a pattern",
        "rg" => "Search files for a pattern (ripgrep)",
        "find" => "Search for files in a directory tree",
        "tar" => "Create and extract archives",
        "curl" => "Transfer data from or to a URL",
        "python" | "python3" => "The Python interpreter",
        "node" => "The Node.js JavaScript runtime",
        "cat" => "Print and concatenate files",
        "echo" => "Print its arguments",
        "printf" => "Print formatted text",
        "pwd" => "Print the current directory",
        "mkdir" => "Make directories",
        "rm" => "Remove files or directories",
        "cp" => "Copy files and directories",
        "mv" => "Move or rename files",
        "touch" => "Create a file or update its times",
        "chmod" => "Change file permissions",
        "chown" => "Change file owner and group",
        "ln" => "Make links between files",
        "head" => "Print the first lines of a file",
        "tail" => "Print the last lines of a file",
        "less" => "Page through text",
        "sort" => "Sort lines of text",
        "uniq" => "Drop or count repeated lines",
        "wc" => "Count lines, words and bytes",
        "sed" => "Stream editor: filter and transform text",
        "awk" => "Pattern scanning and text processing",
        "xargs" => "Run a command with arguments read from input",
        "ps" => "List running processes",
        "kill" => "Send a signal to a process",
        "top" => "Show running processes live",
        "sudo" => "Run a command as another user (root by default)",
        "env" => "Run a command in a changed environment, or print it",
        "export" => "Set environment variables for this shell and its children",
        "source" | "." => "Run a file's commands in this shell",
        "alias" => "Define or list command aliases",
        "history" => "Show the command history",
        "which" => "Show where a command is",
        "man" => "Show a command's manual",
        "vim" | "nvim" | "vi" => "The Vim text editor",
        "code" => "Open Visual Studio Code",
        "open" => "Open files, folders and URLs with their app",
        "exit" => "Leave the shell",
        "clear" => "Clear the screen",
        "true" => "Do nothing, successfully",
        "false" => "Do nothing, unsuccessfully",
        "test" | "[" => "Evaluate a condition",
        "time" => "Time a command",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(buffer: &str, needle: &str) -> Inspection {
        let offset = buffer.find(needle).expect(needle) + 1;
        describe(buffer, offset).expect(needle)
    }

    #[test]
    fn describes_each_part_of_a_git_command() {
        let line = r#"git commit -m "fix it" --amend src/main.rs"#;
        let c = at(line, "git");
        assert_eq!((c.role, c.title.as_str()), (Role::Command, "git"));
        assert_eq!(c.range, 0..3);
        assert!(c.text.is_some());
        let s = at(line, "commit");
        assert_eq!(s.role, Role::Subcommand);
        assert!(s.text.unwrap().to_lowercase().contains("record"));
        let m = at(line, "-m");
        assert_eq!((m.role, m.title.as_str()), (Role::Option, "-m, --message"));
        let v = at(line, "\"fix");
        assert_eq!((v.role, v.title.as_str()), (Role::OptionValue, "fix it"));
        assert!(v.text.unwrap().starts_with("The value of -m, --message"));
        let a = at(line, "--amend");
        assert_eq!(a.role, Role::Option);
        let p = at(line, "src/");
        assert_eq!(p.role, Role::Argument);
    }

    #[test]
    fn options_belong_to_their_level() {
        // `-p` before a subcommand is git's own, after `log` it is log's
        let before = at("git -p log", "-p");
        let after = at("git log -p", "-p");
        assert_ne!(before.text, after.text);
    }

    #[test]
    fn operators_and_segments() {
        let line = "ls -la | grep rs && cd src";
        assert_eq!(at(line, "|").role, Role::Operator);
        let g = at(line, "grep");
        assert_eq!((g.role, g.title.as_str()), (Role::Command, "grep"));
        let a = at(line, "&&");
        assert!(a.text.unwrap().contains("succeeded"));
        let d = at(line, "src");
        assert_eq!(d.role, Role::Argument);
        assert_eq!(d.text.as_deref(), Some("A folder of cd"));
    }

    #[test]
    fn wrappers_variables_lines_and_redirections() {
        let c = at("sudo git commit -m hi", "commit");
        assert_eq!(c.role, Role::Subcommand);
        assert_eq!(at("sudo git commit -m hi", "sudo").role, Role::Command);
        assert_eq!(
            at("sudo -E git status", "-E").text.as_deref(),
            Some("An option of sudo")
        );
        assert_eq!(at("env FOO=1 git log", "git").role, Role::Command);
        let v = at("FOO=1 git status", "FOO");
        assert!(v.text.unwrap().contains("FOO"));
        assert_eq!(at("FOO=1 git status", "git").role, Role::Command);
        assert_eq!(at("time git status", "status").role, Role::Subcommand);
        // a line break starts a command
        let g = at("ls\ngit status", "git");
        assert_eq!((g.role, g.title.as_str()), (Role::Command, "git"));
        // redirections: the descriptor, the operator, the file
        let line = "make 2>&1 > build.log";
        let fd = describe(line, line.find("2>").unwrap()).unwrap();
        assert!(fd.text.unwrap().contains("standard error"));
        assert_eq!(at(line, "2>").role, Role::Operator);
        assert!(
            at(line, "&1")
                .text
                .unwrap()
                .starts_with("File descriptor 1")
        );
        let f = at(line, "build.log");
        assert_eq!(f.role, Role::Argument);
        assert!(f.text.unwrap().contains("writes to"));
        assert_eq!(at("cat <<EOF", "EOF").role, Role::Argument);
        assert_eq!(at("ls > out.txt", "out").role, Role::Argument);
        // an operator Rift has no words for still says what it is
        assert!(at("a <> b", "<>").text.is_some());
    }

    #[test]
    fn reads_the_manuals_one_liner() {
        let mac = "git(1)                   - the stupid content tracker\n";
        assert_eq!(
            parse_whatis("git", mac).as_deref(),
            Some("The stupid content tracker")
        );
        let linux = "jq (1)               - Command-line JSON processor\n";
        assert_eq!(
            parse_whatis("jq", linux).as_deref(),
            Some("Command-line JSON processor")
        );
        let several = "gzip, gunzip, zcat(1)    - compression/decompression tool\n";
        assert!(parse_whatis("gunzip", several).is_some());
        // another page's line, or nothing found
        assert_eq!(
            parse_whatis("git", "git-shell(1) - restricted login shell\n"),
            None
        );
        assert_eq!(parse_whatis("frob", "frob: nothing appropriate\n"), None);
    }

    #[test]
    fn unknown_commands_ask_the_manual_and_blanks_say_nothing() {
        let x = at("frobnicate --fast now", "frob");
        assert_eq!(x.role, Role::Command);
        assert_eq!(x.text, None);
        assert_eq!(at("frobnicate --fast now", "--fast").role, Role::Option);
        assert_eq!(describe("ls  -l", 3), None);
        assert_eq!(describe("", 0), None);
        // the end of a word still counts as on it (a cursor after it)
        assert_eq!(describe("ls", 2).map(|i| i.role), Some(Role::Command));
    }
}
