/// Live shell-command tokenizer for the input editor. Four visual classes
/// (spec §3 syntax map): command, flag, string, plain — plus muted operators.
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// First word of the line / of each pipeline segment.
    Command,
    /// `-f` / `--flag`.
    Flag,
    /// Quoted string (including unterminated).
    Str,
    /// `&&`, `||`, `|`, `;`, redirections.
    Op,
    Plain,
}

pub fn tokenize(line: &str) -> Vec<(Range<usize>, TokenKind)> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut expect_command = true;

    while i < bytes.len() {
        let b = bytes[i];

        if b.is_ascii_whitespace() {
            // a newline (⇧⏎ multi-line command) starts a fresh command
            if b == b'\n' {
                expect_command = true;
            }
            i += 1;
            continue;
        }

        // quoted string — runs to the matching quote or end of line
        if b == b'\'' || b == b'"' {
            let quote = b;
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && quote == b'"' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push((start..i, TokenKind::Str));
            expect_command = false;
            continue;
        }

        // operators
        if matches!(b, b'&' | b'|' | b';' | b'>' | b'<' | b'(' | b')') {
            let start = i;
            while i < bytes.len()
                && matches!(bytes[i], b'&' | b'|' | b';' | b'>' | b'<' | b'(' | b')')
            {
                i += 1;
            }
            out.push((start..i, TokenKind::Op));
            expect_command = true;
            continue;
        }

        // bare word
        let start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(
                bytes[i],
                b'&' | b'|' | b';' | b'>' | b'<' | b'\'' | b'"' | b'(' | b')'
            )
        {
            i += 1;
        }
        let word = &line[start..i];
        let kind = if word.starts_with('-') && word.len() > 1 {
            TokenKind::Flag
        } else if expect_command {
            TokenKind::Command
        } else {
            TokenKind::Plain
        };
        if kind == TokenKind::Command && !word.is_empty() {
            expect_command = false;
        }
        out.push((start..i, kind));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<(&str, TokenKind)> {
        tokenize(line)
            .into_iter()
            .map(|(r, k)| (&line[r], k))
            .collect()
    }

    #[test]
    fn command_flag_string() {
        assert_eq!(
            kinds(r#"git commit -m "fix rgb rounding""#),
            vec![
                ("git", TokenKind::Command),
                ("commit", TokenKind::Plain),
                ("-m", TokenKind::Flag),
                (r#""fix rgb rounding""#, TokenKind::Str),
            ]
        );
    }

    #[test]
    fn pipeline_recolors_commands() {
        assert_eq!(
            kinds("ls | grep foo && cargo test"),
            vec![
                ("ls", TokenKind::Command),
                ("|", TokenKind::Op),
                ("grep", TokenKind::Command),
                ("foo", TokenKind::Plain),
                ("&&", TokenKind::Op),
                ("cargo", TokenKind::Command),
                ("test", TokenKind::Plain),
            ]
        );
    }

    #[test]
    fn unterminated_string() {
        assert_eq!(
            kinds(r#"echo "unfinish"#),
            vec![
                ("echo", TokenKind::Command),
                (r#""unfinish"#, TokenKind::Str)
            ]
        );
    }

    #[test]
    fn double_dash_flag() {
        assert_eq!(
            kinds("cargo build --release"),
            vec![
                ("cargo", TokenKind::Command),
                ("build", TokenKind::Plain),
                ("--release", TokenKind::Flag),
            ]
        );
    }

    #[test]
    fn newline_starts_a_new_command() {
        let toks = tokenize("echo a\ngit status");
        let cmds: Vec<&str> = toks
            .iter()
            .filter(|(_, k)| *k == TokenKind::Command)
            .map(|(r, _)| &"echo a\ngit status"[r.clone()])
            .collect();
        assert_eq!(cmds, vec!["echo", "git"]);
    }
}
