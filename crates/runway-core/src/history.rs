/// Shell history import for ⌃R search. Reads zsh's `$HISTFILE`
/// (`~/.zsh_history` default), handling all three zsh quirks:
/// EXTENDED_HISTORY timestamps, multiline entries, and metafied bytes.
use std::path::PathBuf;

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
            // continuation of a multiline entry
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

pub fn history_file() -> Option<PathBuf> {
    if let Some(f) = std::env::var_os("HISTFILE") {
        return Some(PathBuf::from(f));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".zsh_history"))
}

/// Load, dedupe (keep most recent occurrence), cap. Oldest first.
pub fn load_shell_history(cap: usize) -> Vec<String> {
    let Some(path) = history_file() else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read(&path) else {
        return Vec::new();
    };
    let entries = parse_zsh_history(&raw);
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
        let raw = b": 1719400000:0;for f in *; do\\\n  echo $f\ndone\n: 1719400001:0;pwd\n";
        let parsed = parse_zsh_history(raw);
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].starts_with("for f in *; do"));
        assert!(parsed[0].contains("done"));
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
}
