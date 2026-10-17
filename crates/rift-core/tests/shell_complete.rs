//! Completions from the user's own shell (`rift_core::shell_complete`),
//! run against the real zsh, bash and fish with a scratch home holding a
//! user-defined completion for `mycmd` (a tool Rift has no spec for). A
//! shell that is not installed is reported and skipped.

use rift_core::shell_complete::{BUDGET, Completer, Query, query};
use rift_core::shell_env::ShellKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn find(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(program))
        .find(|p| p.is_file())
}

/// A scratch home with `mycmd` (its completion offers deploy/destroy/
/// status) and `hangcmd` (its completion sleeps) defined for all three
/// shells, and both commands on `bin/`.
struct Fixture {
    root: PathBuf,
    _serial: std::sync::MutexGuard<'static, ()>,
}

static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// One test at a time: they start real interactive shells (rc files,
/// compinit), and nine at once on a loaded runner outlast every timeout.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!(
            "rift-shell-complete-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for d in [
            "bin",
            "home/fn",
            "home/.config/fish/completions",
            "cwd/sub dir",
        ] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("cwd/file one"), "").unwrap();
        for cmd in ["mycmd", "hangcmd"] {
            let p = root.join("bin").join(cmd);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let home = root.join("home");
        let w = |name: &str, body: &str| std::fs::write(home.join(name), body).unwrap();
        // zsh: completion functions on fpath, compinit by the rc file
        w(
            ".zshrc",
            "fpath=($ZDOTDIR/fn $fpath)\nautoload -Uz compinit && compinit -u -d $ZDOTDIR/.zcompdump\n\
             echo 'rc output is skipped'\n",
        );
        w(
            "fn/_mycmd",
            "#compdef mycmd\nlocal -a subs\nsubs=('deploy:ship it' 'destroy:tear down' 'status:show state')\n\
             _describe 'command' subs\n",
        );
        w("fn/_hangcmd", "#compdef hangcmd\nsleep 30\ncompadd never\n");
        // bash: `complete -F` specs in the rc file; a login shell reads
        // .bash_profile, which sources it
        w(
            ".bashrc",
            "_mycmd() { COMPREPLY=($(compgen -W 'deploy destroy status' -- \"$2\")); }\n\
             complete -F _mycmd mycmd\n\
             _hangcmd() { sleep 30; COMPREPLY=(never); }\ncomplete -F _hangcmd hangcmd\n\
             echo 'rc output is skipped'\n",
        );
        w(".bash_profile", ". ~/.bashrc\n");
        // fish: autoloaded completion files
        w(
            ".config/fish/completions/mycmd.fish",
            "complete -c mycmd -f -a 'deploy\\t\"ship it\" destroy\\t\"tear down\" status'\n",
        );
        w(
            ".config/fish/completions/hangcmd.fish",
            "complete -c hangcmd -f -a '(sleep 30; echo never)'\n",
        );
        // an rc hook printing an OSC 7 without a newline on every `cd`
        // (as shell integrations do): it runs into the reply's markers
        std::fs::create_dir_all(home.join(".config/fish/conf.d")).unwrap();
        w(
            ".config/fish/conf.d/osc7.fish",
            "function __osc7 --on-variable PWD\n  printf '\\e]7;file://h%s\\a' $PWD\nend\n",
        );
        let zshrc = std::fs::read_to_string(home.join(".zshrc")).unwrap();
        w(
            ".zshrc",
            &format!("{zshrc}chpwd() {{ printf '\\e]7;file://h%s\\a' $PWD }}\n"),
        );
        Fixture {
            root,
            _serial: serial,
        }
    }

    fn completer(&self, program: &Path) -> Completer {
        let home = self.root.join("home");
        let path = format!(
            "{}:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin",
            self.root.join("bin").display()
        );
        let home_s = home.to_string_lossy().into_owned();
        let config = home.join(".config").to_string_lossy().into_owned();
        Completer::new(&program.to_string_lossy()).with_env(&[
            ("HOME", &home_s),
            ("ZDOTDIR", &home_s),
            ("XDG_CONFIG_HOME", &config),
            (
                "XDG_DATA_HOME",
                &home.join(".local/share").to_string_lossy(),
            ),
            ("PATH", &path),
        ])
    }

    fn cwd(&self) -> String {
        self.root.join("cwd").to_string_lossy().into_owned()
    }

    fn query(&self, kind: ShellKind, line: &str) -> Query {
        query(kind, line, line.len(), &self.cwd()).expect("a shell query")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The first answer may arrive while the shell still reads its rc files:
/// ask until it is ready (at most 10 s).
fn ask(c: &mut Completer, q: &Query) -> Vec<(String, String)> {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let items = c.complete(q, BUDGET);
        if !items.is_empty() || Instant::now() > until {
            return items;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Wait until the shell is up (it answers a `mycmd` request).
fn warm(c: &mut Completer, f: &Fixture, kind: ShellKind) {
    let got = ask(c, &f.query(kind, "mycmd "));
    assert!(!got.is_empty(), "{kind:?}: the shell never answered");
}

fn words(items: &[(String, String)]) -> Vec<&str> {
    items.iter().map(|(w, _)| w.as_str()).collect()
}

fn shells() -> Vec<(ShellKind, PathBuf)> {
    let mut out = Vec::new();
    for (kind, name) in [
        (ShellKind::Zsh, "zsh"),
        (ShellKind::Bash, "bash"),
        (ShellKind::Fish, "fish"),
    ] {
        match find(name) {
            Some(p) => out.push((kind, p)),
            None => {
                eprintln!("BLOCKED: {name} is not installed; its shell completions are untested")
            }
        }
    }
    // macOS ships bash 3.2 as /bin/bash: the server must run there too
    let system = PathBuf::from("/bin/bash");
    if system.is_file() && !out.iter().any(|(_, p)| *p == system) {
        out.push((ShellKind::Bash, system));
    }
    out
}

#[test]
fn user_completion_functions_answer_for_tools_rift_has_no_spec_for() {
    for (kind, program) in shells() {
        let f = Fixture::new(&format!("user-{kind:?}"));
        let mut c = f.completer(&program);
        let got = ask(&mut c, &f.query(kind, "mycmd de"));
        assert_eq!(words(&got), vec!["deploy", "destroy"], "{kind:?}: {got:?}");
        if kind != ShellKind::Bash {
            assert_eq!(got[0].1, "ship it", "{kind:?}: descriptions");
        }
        // the shell stays warm: the next answer is fast
        let t = Instant::now();
        let got = c.complete(&f.query(kind, "mycmd s"), BUDGET);
        let took = t.elapsed();
        assert_eq!(words(&got), vec!["status"], "{kind:?}");
        eprintln!("{kind:?}: warm request {took:?}");
        assert!(took < BUDGET, "{kind:?}: {took:?}");
        // the directory is the request's
        let other = std::env::temp_dir();
        let q = Query {
            cwd: f.cwd(),
            line: "cat ".into(),
            words: vec!["cat".into(), String::new()],
        };
        let got = c.complete(&q, BUDGET);
        if kind != ShellKind::Bash {
            // bash completes only commands with a `complete` spec
            assert!(
                words(&got).contains(&"file one"),
                "{kind:?}: files of the request's directory: {got:?}"
            );
        }
        let q2 = Query {
            cwd: other.to_string_lossy().into_owned(),
            ..q
        };
        let got = c.complete(&q2, BUDGET);
        assert!(!words(&got).contains(&"file one"), "{kind:?}: {got:?}");
    }
}

#[test]
fn without_the_user_function_the_shell_offers_nothing() {
    // negative control: the words above come from the user's function
    for (kind, program) in shells() {
        let f = Fixture::new(&format!("none-{kind:?}"));
        let home = f.root.join("home");
        let _ = std::fs::remove_file(home.join("fn/_mycmd"));
        let _ = std::fs::remove_file(home.join(".zcompdump"));
        let _ = std::fs::remove_file(home.join(".config/fish/completions/mycmd.fish"));
        std::fs::write(home.join(".bashrc"), "true\n").unwrap();
        let mut c = f.completer(&program);
        // a request the shell can answer proves it is up
        let up = Query {
            cwd: f.cwd(),
            line: "cat fi".into(),
            words: vec!["cat".into(), "fi".into()],
        };
        if kind != ShellKind::Bash {
            assert!(!ask(&mut c, &up).is_empty(), "{kind:?}");
        }
        let got = c.complete(&f.query(kind, "mycmd de"), BUDGET);
        assert!(
            !words(&got).contains(&"deploy"),
            "{kind:?}: no function, no words: {got:?}"
        );
    }
}

#[test]
fn bash_loads_completions_on_demand_and_reads_word_list_specs() {
    for (kind, program) in shells() {
        if kind != ShellKind::Bash {
            continue;
        }
        let f = Fixture::new("bash-loader");
        let home = f.root.join("home");
        // bash-completion's loader: defines a command's spec on first use
        let loader = "_completion_loader() {\n  [ \"$1\" = lazy ] || return 1\n  \
                      _lazy() { COMPREPLY=(lazy-one lazy-two); }\n  complete -F _lazy lazy\n  return 124\n}\n\
                      complete -D -F _completion_loader\ncomplete -W 'start stop restart' svc\n";
        let rc = std::fs::read_to_string(home.join(".bashrc")).unwrap();
        std::fs::write(home.join(".bashrc"), format!("{rc}{loader}")).unwrap();
        let mut c = f.completer(&program);
        warm(&mut c, &f, kind);
        let got = c.complete(&f.query(kind, "lazy "), BUDGET);
        assert_eq!(words(&got), vec!["lazy-one", "lazy-two"], "{program:?}");
        let got = c.complete(&f.query(kind, "svc st"), BUDGET);
        assert_eq!(words(&got), vec!["start", "stop"], "{program:?}");
        // control: without the loader, `lazy` has no spec and no words
        std::fs::write(home.join(".bashrc"), &rc).unwrap();
        let mut c = f.completer(&program);
        warm(&mut c, &f, kind);
        let got = c.complete(&f.query(kind, "lazy "), BUDGET);
        assert!(got.is_empty(), "{program:?}: {got:?}");
    }
}

#[test]
fn a_hanging_completion_function_times_out_and_the_shell_recovers() {
    for (kind, program) in shells() {
        let f = Fixture::new(&format!("hang-{kind:?}"));
        let mut c = f.completer(&program);
        warm(&mut c, &f, kind);
        // control: a normal request finishes inside the budget
        let t = Instant::now();
        assert!(!c.complete(&f.query(kind, "mycmd d"), BUDGET).is_empty());
        assert!(t.elapsed() < BUDGET);
        let pid = c.pid().unwrap();
        let t = Instant::now();
        let got = c.complete(&f.query(kind, "hangcmd "), BUDGET);
        let took = t.elapsed();
        assert!(got.is_empty(), "{kind:?}: {got:?}");
        assert!(
            took < BUDGET + Duration::from_millis(200),
            "{kind:?}: gave up after {took:?}"
        );
        assert!(!c.is_running(), "{kind:?}: the hung shell is stopped");
        // its process group (the sleeping completion function) is gone
        std::thread::sleep(Duration::from_millis(200));
        let alive = std::process::Command::new("kill")
            .args(["-0", "--", &format!("-{pid}")])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(!alive, "{kind:?}: process group {pid} still runs");
        // after the pause, a new shell answers again
        std::thread::sleep(Duration::from_millis(1100));
        let got = ask(&mut c, &f.query(kind, "mycmd de"));
        assert_eq!(words(&got), vec!["deploy", "destroy"], "{kind:?}");
    }
}

#[test]
fn the_command_line_is_data_never_run() {
    for (kind, program) in shells() {
        let f = Fixture::new(&format!("inject-{kind:?}"));
        let mut c = f.completer(&program);
        warm(&mut c, &f, kind);
        let marker = f.root.join("PWNED");
        let m = marker.to_string_lossy();
        // `query` refuses these lines; send them straight to the shell to
        // prove the server itself never evaluates the line or the words
        let payloads = [
            format!("mycmd $(touch {m})"),
            format!("mycmd `touch {m}`"),
            format!("mycmd ; touch {m}; mycmd "),
            format!("mycmd (touch {m}) "),
            format!("mycmd \"$(touch {m})"),
        ];
        for line in &payloads {
            let q = Query {
                cwd: f.cwd(),
                line: line.clone(),
                words: vec!["mycmd".into(), format!("$(touch {m})")],
            };
            let _ = c.complete(&q, BUDGET);
            // a cwd is data too
            let q = Query {
                cwd: format!("$(touch {m})"),
                line: "mycmd ".into(),
                words: vec!["mycmd".into(), String::new()],
            };
            let _ = c.complete(&q, BUDGET);
            assert!(!marker.exists(), "{kind:?}: {line:?} ran");
        }
        // control: the same payload in a line a shell runs does create it
        let ran = std::process::Command::new(&program)
            .arg("-c")
            .arg(format!("echo $(touch {m})"))
            .status()
            .unwrap();
        assert!(
            ran.success() && marker.exists(),
            "{kind:?}: the payload works"
        );
        std::fs::remove_file(&marker).unwrap();
    }
}

/// Append `rc` to the fixture's `.zshrc` (zsh only).
fn zshrc_append(f: &Fixture, rc: &str) {
    let path = f.root.join("home/.zshrc");
    let old = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{old}{rc}")).unwrap();
}

fn zsh() -> Option<PathBuf> {
    let found = find("zsh");
    if found.is_none() {
        eprintln!("BLOCKED: zsh is not installed");
    }
    found
}

#[test]
fn a_pasted_zsh_glob_qualifier_never_runs_with_expand_in_the_completer() {
    let Some(program) = zsh() else { return };
    let f = Fixture::new("expand");
    // `_expand` first: it evaluates the word (globs, qualifiers, `$( )`)
    zshrc_append(&f, "zstyle ':completion:*' completer _expand _complete\n");
    let mut c = f.completer(&program);
    warm(&mut c, &f, ShellKind::Zsh);
    let marker = f.root.join("cwd/PWNED");
    let line = "ls *(e:'touch PWNED':)";
    // Rift never sends it: `(` is refused for zsh
    assert_eq!(query(ShellKind::Zsh, line, line.len(), &f.cwd()), None);
    // nor does the server evaluate it when it gets it anyway
    for line in [
        line.to_string(),
        "ls *(e:'touch PWNED':) ".to_string(),
        "mycmd $(touch PWNED)".to_string(),
        "mycmd ${(e):-\\$(touch PWNED)}".to_string(),
    ] {
        let q = Query {
            cwd: f.cwd(),
            line: line.clone(),
            words: vec!["ls".into(), "x".into()],
        };
        let _ = c.complete(&q, BUDGET);
        std::thread::sleep(Duration::from_millis(100));
        assert!(!marker.exists(), "{line:?} ran");
    }
    // the completer still answers through `_complete`
    let got = ask(&mut c, &f.query(ShellKind::Zsh, "mycmd de"));
    assert_eq!(words(&got), vec!["deploy", "destroy"], "{got:?}");
    // control: `_expand` does run the qualifier in a shell's own Tab
    let ran = std::process::Command::new(&program)
        .current_dir(f.root.join("cwd"))
        .args(["-f", "-c", "echo *(e:'touch PWNED':) >/dev/null"])
        .status()
        .unwrap();
    assert!(ran.success() && marker.exists(), "the payload works");
}

#[test]
fn zsh_widgets_on_printable_keys_never_alter_the_line() {
    let Some(program) = zsh() else { return };
    let f = Fixture::new("widgets");
    // an abbreviation widget on space (zsh-abbr style: `mycmd` expands to
    // `othercmd`) and an autopair-like widget on `"`: typed as keystrokes,
    // the line would run them and complete another line
    zshrc_append(
        &f,
        "__abbr() { [[ $LBUFFER == mycmd ]] && LBUFFER=othercmd; zle .self-insert }\n\
         zle -N __abbr\nbindkey ' ' __abbr\n\
         __pair() { LBUFFER+='\"'; RBUFFER='\"'$RBUFFER }\nzle -N __pair\nbindkey '\"' __pair\n",
    );
    let mut c = f.completer(&program);
    let got = ask(&mut c, &f.query(ShellKind::Zsh, "mycmd de"));
    assert_eq!(words(&got), vec!["deploy", "destroy"], "{got:?}");
    let got = c.complete(&f.query(ShellKind::Zsh, "cat \"fi"), BUDGET);
    assert_eq!(words(&got), vec!["file one"], "{got:?}");
    // magic-space on space (history expansion) leaves the line alone too
    // (a Fixture holds the suite's serial lock: release the first before the
    // second is made, or this test waits on itself)
    drop(c);
    drop(f);
    let f = Fixture::new("magic-space");
    zshrc_append(&f, "bindkey ' ' magic-space\n");
    let mut c = f.completer(&program);
    let got = ask(&mut c, &f.query(ShellKind::Zsh, "mycmd de"));
    assert_eq!(words(&got), vec!["deploy", "destroy"], "{got:?}");
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[test]
fn the_zsh_server_ends_when_its_parent_is_gone_mid_request() {
    let Some(program) = zsh() else { return };
    let f = Fixture::new("orphan");
    let home = f.root.join("home");
    let pidfile = f.root.join("server.pid");
    // a parent that hands the server a hanging request (`hangcmd`'s
    // function sleeps 30 s), then exits as a crashed Rift would
    let request = format!("1\n{}\nhangcmd \n1\nhangcmd\n\n", f.cwd());
    let script = format!(
        "printf '%s' \"$REQ\" | (exec \"$ZSH\" -f -c \"$SRV\" rift-complete \"$ZSH\" >/dev/null) & p=$!; echo $p > '{}'; \
         i=0; while [ -z \"$(pgrep -P $p)\" ] && [ $i -lt 300 ]; do sleep 0.1; i=$((i+1)); done; sleep 1",
        pidfile.display()
    );
    let path = format!("{}:/usr/bin:/bin", f.root.join("bin").display());
    let status = std::process::Command::new("/bin/sh")
        .args(["-c", &script])
        .env("REQ", &request)
        .env("ZSH", &program)
        .env("SRV", rift_core::shell_complete::SERVER_ZSH)
        .env("HOME", &home)
        .env("ZDOTDIR", &home)
        .env("PATH", &path)
        .env("TERM", "xterm-256color")
        .status()
        .unwrap();
    assert!(status.success());
    let pid = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .to_string();
    // the inner shell (and its sleeping function) hang off the zpty child
    let children = std::process::Command::new("pgrep")
        .args(["-P", &pid])
        .output()
        .unwrap();
    let children: Vec<String> = String::from_utf8_lossy(&children.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    assert!(!children.is_empty(), "the server runs its inner shell");
    // the server notices its parent is gone within about a second
    let until = Instant::now() + Duration::from_secs(5);
    while alive(&pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(300));
    let server_left = alive(&pid);
    let state = std::process::Command::new("ps")
        .args(["-o", "pid=,ppid=,stat=", "-p", &pid])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let left: Vec<&String> = children.iter().filter(|c| alive(c)).collect();
    // never leave them running whatever the outcome
    for p in std::iter::once(&pid).chain(children.iter()) {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", p])
            .stderr(std::process::Stdio::null())
            .status();
    }
    assert!(!server_left, "the server outlived its parent: {state}");
    assert!(left.is_empty(), "its inner shell outlived it: {left:?}");
}

#[test]
fn bash_directory_specs_mark_directories() {
    for (kind, program) in shells() {
        if kind != ShellKind::Bash {
            continue;
        }
        let f = Fixture::new("bash-dirs");
        let home = f.root.join("home");
        let rc = std::fs::read_to_string(home.join(".bashrc")).unwrap();
        std::fs::write(
            home.join(".bashrc"),
            format!("{rc}complete -d cdish\ncomplete -W 'sub plain' wordy\n"),
        )
        .unwrap();
        let mut c = f.completer(&program);
        warm(&mut c, &f, kind);
        // `complete -d`: directories end in `/`, as readline shows them
        let got = c.complete(&f.query(kind, "cdish s"), BUDGET);
        assert_eq!(words(&got), vec!["sub dir/"], "{program:?}");
        // a word list is no file name, even where a directory has its name
        std::fs::create_dir_all(f.root.join("cwd/plain")).unwrap();
        let got = c.complete(&f.query(kind, "wordy pl"), BUDGET);
        assert_eq!(words(&got), vec!["plain"], "{program:?}");
    }
}
