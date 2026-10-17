/// Shell history import for ⌃R search. Reads the history file of the
/// user's shell: zsh (`$HISTFILE`, `~/.zsh_history`; EXTENDED_HISTORY
/// timestamps, multiline entries, metafied bytes), bash (`$HISTFILE`,
/// `~/.bash_history`; `#<epoch>` timestamp lines) and fish
/// (`$XDG_DATA_HOME/fish/fish_history`, a YAML-like `- cmd:` list). Only
/// the last [`MAX_READ`] bytes are read.
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Bytes read from the end of a history file; older entries are skipped.
pub const MAX_READ: u64 = 4 * 1024 * 1024;

/// On-disk history format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zsh,
    Bash,
    Fish,
}

/// zsh "metafies" bytes ≥ 0x80: 0x83 followed by `byte ^ 0x20`.
fn unmetafy(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x83 && i + 1 < bytes.len() {
            out.push(bytes[i + 1] ^ 0x20);
            i += 2;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn is_extended_entry_start(line: &str) -> bool {
    // `: <ts>:<dur>;cmd`
    let Some(rest) = line.strip_prefix(": ") else {
        return false;
    };
    let Some((ts, tail)) = rest.split_once(':') else {
        return false;
    };
    if ts.is_empty() || !ts.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let Some((dur, _)) = tail.split_once(';') else {
        return false;
    };
    !dur.is_empty() && dur.bytes().all(|b| b.is_ascii_digit())
}

/// Parse zsh history file content into commands, oldest first.
pub fn parse_zsh_history(raw: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(&unmetafy(raw)).into_owned();
    let mut out: Vec<String> = Vec::new();
    let mut current: Option<String> = None;

    for line in text.lines() {
        if is_extended_entry_start(line) {
            if let Some(c) = current.take() {
                push_entry(&mut out, c);
            }
            // strip `: <ts>:<dur>;`
            let cmd = line
                .split_once(';')
                .map(|x| x.1)
                .unwrap_or_default()
                .to_string();
            current = Some(cmd);
        } else if let Some(c) = &mut current {
            // continuation of a multiline entry: zsh wrote `\` + newline
            if c.ends_with('\\') {
                c.pop();
            }
            c.push('\n');
            c.push_str(line);
        } else {
            // plain (non-EXTENDED_HISTORY) format: one command per line
            push_entry(&mut out, line.to_string());
        }
    }
    if let Some(c) = current.take() {
        push_entry(&mut out, c);
    }
    out
}

fn push_entry(out: &mut Vec<String>, mut cmd: String) {
    // zsh escapes embedded newlines in multiline entries with a backslash
    while cmd.ends_with('\\') {
        cmd.pop();
    }
    let trimmed = cmd.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
}

/// Parse bash history into commands, oldest first. `#<epoch>` lines
/// (HISTTIMEFORMAT) are timestamps, not commands: when the file has any,
/// an entry is every line up to the next timestamp (a multiline command is
/// one entry). Without timestamps bash cannot tell where an entry ends, so
/// each line is one.
pub fn parse_bash_history(raw: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(raw);
    let mut out: Vec<String> = Vec::new();
    let mut push = |cmd: &str| {
        let cmd = cmd.trim();
        if !cmd.is_empty() {
            out.push(cmd.to_string());
        }
    };
    let mut current: Option<Vec<&str>> = None;
    for line in text.lines() {
        if is_bash_timestamp(line) {
            if let Some(c) = current.take() {
                push(&c.join("\n"));
            }
            current = Some(Vec::new());
        } else if let Some(c) = &mut current {
            c.push(line);
        } else {
            // before the first timestamp, or no timestamps at all
            push(line);
        }
    }
    if let Some(c) = current {
        push(&c.join("\n"));
    }
    out
}

fn is_bash_timestamp(line: &str) -> bool {
    line.strip_prefix('#')
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// Parse fish history (`- cmd: <escaped>` entries with indented `when:` /
/// `paths:` fields) into commands, oldest first.
pub fn parse_fish_history(raw: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(raw)
        .lines()
        .filter_map(|l| l.strip_prefix("- cmd: "))
        .map(unescape_fish)
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

/// fish writes a newline as `\n` and a backslash as `\\`.
fn unescape_fish(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// History file and format for `shell` (a program path or name; a login
/// shell's leading `-` is ignored). `histfile` ($HISTFILE) applies to bash
/// and zsh only; `xdg_data` ($XDG_DATA_HOME) to fish. Other shells fall
/// back to zsh's file and parser, which also reads one command per line.
pub fn source_for(
    shell: &str,
    home: &Path,
    histfile: Option<&Path>,
    xdg_data: Option<&Path>,
) -> (PathBuf, Format) {
    let base = shell.rsplit('/').next().unwrap_or(shell);
    let base = base.trim_start_matches('-');
    let histfile_or = |default: &str| {
        histfile
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(default))
    };
    match base {
        "fish" => {
            let data = xdg_data
                .map(Path::to_path_buf)
                .unwrap_or_else(|| home.join(".local/share"));
            (data.join("fish/fish_history"), Format::Fish)
        }
        "bash" => (histfile_or(".bash_history"), Format::Bash),
        _ => (histfile_or(".zsh_history"), Format::Zsh),
    }
}

/// The last `max` bytes of `path`, starting at the first whole entry: the
/// cut line is dropped, then (fish, zsh extended format) the rest of the
/// cut entry up to the next entry start.
fn read_tail(path: &Path, max: u64, format: Format) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    if len <= max {
        let mut buf = Vec::with_capacity(len as usize);
        f.read_to_end(&mut buf)?;
        return Ok(buf);
    }
    f.seek(SeekFrom::Start(len - max))?;
    let mut buf = Vec::with_capacity(max as usize);
    f.take(max).read_to_end(&mut buf)?;
    let Some(nl) = buf.iter().position(|&b| b == b'\n') else {
        return Ok(Vec::new());
    };
    buf.drain(..=nl);
    let starts_entry = |line: &[u8]| match format {
        Format::Fish => line.starts_with(b"- cmd: "),
        Format::Zsh => is_extended_entry_start(&String::from_utf8_lossy(line)),
        Format::Bash => is_bash_timestamp(&String::from_utf8_lossy(line)),
    };
    // plain-format zsh and untimestamped bash have no entry markers: every
    // line is an entry
    if matches!(format, Format::Zsh | Format::Bash) && !buf.split(|&b| b == b'\n').any(starts_entry)
    {
        return Ok(buf);
    }
    let mut skip = 0;
    for line in buf.split(|&b| b == b'\n') {
        if starts_entry(line) {
            break;
        }
        skip += line.len() + 1;
    }
    buf.drain(..skip.min(buf.len()));
    Ok(buf)
}

/// Load `shell`'s history (see [`source_for`]), dedupe (keep most recent
/// occurrence), cap. Oldest first.
pub fn load_shell_history(shell: &str, cap: usize) -> Vec<String> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let env_path = |k: &str| {
        std::env::var_os(k)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let histfile = env_path("HISTFILE");
    let xdg_data = env_path("XDG_DATA_HOME");
    let (path, format) = source_for(shell, &home, histfile.as_deref(), xdg_data.as_deref());
    load_from(&path, format, MAX_READ, cap)
}

fn load_from(path: &Path, format: Format, max: u64, cap: usize) -> Vec<String> {
    let Ok(raw) = read_tail(path, max, format) else {
        return Vec::new();
    };
    let entries = match format {
        Format::Zsh => parse_zsh_history(&raw),
        Format::Bash => parse_bash_history(&raw),
        Format::Fish => parse_fish_history(&raw),
    };
    // dedupe keeping the LAST occurrence (most recent position)
    let mut seen = std::collections::HashSet::new();
    let mut deduped: Vec<String> = Vec::new();
    for cmd in entries.into_iter().rev() {
        if seen.insert(cmd.clone()) {
            deduped.push(cmd);
        }
        if deduped.len() >= cap {
            break;
        }
    }
    deduped.reverse();
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_format() {
        let raw = b": 1719400000:0;git status\n: 1719400001:2;cargo build --release\n";
        assert_eq!(
            parse_zsh_history(raw),
            vec!["git status".to_string(), "cargo build --release".into()]
        );
    }

    #[test]
    fn plain_format() {
        let raw = b"ls -la\necho hi\n";
        assert_eq!(
            parse_zsh_history(raw),
            vec!["ls -la".to_string(), "echo hi".into()]
        );
    }

    #[test]
    fn multiline_entry() {
        let raw = b": 1719400000:0;for f in *; do\\\n  echo $f\\\ndone\n: 1719400001:0;pwd\n";
        let parsed = parse_zsh_history(raw);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], "for f in *; do\n  echo $f\ndone");
        assert_eq!(parsed[1], "pwd");
    }

    #[test]
    fn metafied_utf8() {
        // "é" = 0xC3 0xA9 → metafied: 0x83, 0xC3^0x20=0xE3, 0x83, 0xA9^0x20=0x89
        let raw: &[u8] = &[
            b':', b' ', b'1', b':', b'0', b';', b'e', b'c', b'h', b'o', b' ', 0x83, 0xE3, 0x83,
            0x89, b'\n',
        ];
        assert_eq!(parse_zsh_history(raw), vec!["echo é".to_string()]);
    }

    #[test]
    fn not_extended_lookalike() {
        // starts with ": " but no valid ts/dur → plain line
        let raw = b": hello ; world\n";
        assert_eq!(parse_zsh_history(raw), vec![": hello ; world".to_string()]);
    }

    #[test]
    fn bash_timestamps_skipped() {
        let raw = b"#1719400000\ngit status\n#1719400001\nls -la\n#1719400002\n#tag\n";
        assert_eq!(
            parse_bash_history(raw),
            vec!["git status".to_string(), "ls -la".into(), "#tag".into()]
        );
    }

    #[test]
    fn bash_multiline_entry_with_timestamps_is_one_entry() {
        let raw =
            b"#1719400000\nls\n#1719400001\nfor i in 1 2; do\necho $i\ndone\n#1719400002\npwd\n";
        assert_eq!(
            parse_bash_history(raw),
            vec![
                "ls".to_string(),
                "for i in 1 2; do\necho $i\ndone".into(),
                "pwd".into()
            ]
        );
    }

    #[test]
    fn bash_without_timestamps_is_one_entry_per_line() {
        let raw = b"ls\nfor i in 1 2; do\necho $i\ndone\n";
        assert_eq!(parse_bash_history(raw).len(), 4);
    }

    #[test]
    fn fish_entries() {
        assert_eq!(
            parse_fish_history(b"- cmd: git log\n  when: 1\n"),
            vec!["git log".to_string()]
        );
        let raw = b"- cmd: echo a\\nb \\\\x\n  when: 2\n  paths:\n    - b\n";
        assert_eq!(parse_fish_history(raw), vec!["echo a\nb \\x".to_string()]);
    }

    #[test]
    fn sources_per_shell() {
        let home = Path::new("/h");
        let hf = Some(Path::new("/x/hist"));
        let xdg = Some(Path::new("/d"));
        let src = |shell, hf, xdg| source_for(shell, home, hf, xdg);
        let p = PathBuf::from;
        assert_eq!(
            src("/bin/zsh", None, None),
            (p("/h/.zsh_history"), Format::Zsh)
        );
        assert_eq!(src("-zsh", hf, None), (p("/x/hist"), Format::Zsh));
        assert_eq!(
            src("/usr/local/bin/bash", None, xdg),
            (p("/h/.bash_history"), Format::Bash)
        );
        assert_eq!(src("bash", hf, None), (p("/x/hist"), Format::Bash));
        // HISTFILE does not apply to fish
        assert_eq!(
            src("/opt/homebrew/bin/fish", hf, xdg),
            (p("/d/fish/fish_history"), Format::Fish)
        );
        assert_eq!(
            src("fish", None, None),
            (p("/h/.local/share/fish/fish_history"), Format::Fish)
        );
    }

    #[test]
    fn bounded_read_keeps_whole_trailing_entries() {
        let dir = std::env::temp_dir().join(format!("rift-hist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tail_from = |body: &str, at: &str| (body.len() - body.find(at).unwrap()) as u64;

        // fish: the cut lands inside the second entry's `when:` field
        let fish = dir.join("fish_history");
        let body =
            "- cmd: first\n  when: 1\n- cmd: second\n  when: 22222\n- cmd: third\n  when: 3\n";
        std::fs::write(&fish, body).unwrap();
        let max = tail_from(body, "2222");
        assert_eq!(load_from(&fish, Format::Fish, max, 100), vec!["third"]);
        assert_eq!(
            load_from(&fish, Format::Fish, MAX_READ, 100),
            vec!["first", "second", "third"]
        );

        // zsh: the cut lands in a multiline entry's continuation lines
        let zsh = dir.join("zsh_history");
        let body = ": 1:0;echo one\n: 2:0;for f in *; do\\\n  echo $f\\\ndone\n: 3:0;pwd\n";
        std::fs::write(&zsh, body).unwrap();
        let max = tail_from(body, "ho $f");
        assert_eq!(load_from(&zsh, Format::Zsh, max, 100), vec!["pwd"]);

        // bash: the cut line is dropped
        let bash = dir.join("bash_history");
        let body = "#1\nls -la\n#2\npwd\n";
        std::fs::write(&bash, body).unwrap();
        let max = tail_from(body, "la\n");
        assert_eq!(load_from(&bash, Format::Bash, max, 100), vec!["pwd"]);

        // bash, multiline: the cut lands inside an entry; its tail is dropped
        let body = "#1\nfor i in 1 2; do\necho $i\ndone\n#2\npwd\n";
        std::fs::write(&bash, body).unwrap();
        let max = tail_from(body, "ho $i");
        assert_eq!(load_from(&bash, Format::Bash, max, 100), vec!["pwd"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
