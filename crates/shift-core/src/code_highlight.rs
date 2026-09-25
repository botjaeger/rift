//! Lightweight source highlighting for the file viewer: keywords, strings,
//! comments and numbers for C-like and `#`-comment languages, picked by file
//! extension. Line by line, carrying only block-comment state, so a viewer
//! can highlight just the lines on screen.

use std::ops::Range;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Keyword,
    String,
    Comment,
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// `//` and `/* */` comments (Rust, C family, JS/TS, Go, Java, Swift, …).
    CLike,
    /// `/* */` only (CSS).
    Css,
    /// `#` comments (shell, Python, Ruby, TOML, YAML, …).
    Hash,
    /// No highlighting (Markdown, text, unknown).
    Plain,
}

const C_KEYWORDS: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "chan",
    "class",
    "const",
    "continue",
    "crate",
    "default",
    "defer",
    "do",
    "dyn",
    "else",
    "enum",
    "export",
    "extends",
    "extern",
    "false",
    "final",
    "finally",
    "fn",
    "for",
    "from",
    "fun",
    "func",
    "function",
    "go",
    "if",
    "impl",
    "implements",
    "import",
    "in",
    "interface",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "new",
    "nil",
    "null",
    "object",
    "override",
    "package",
    "private",
    "protected",
    "pub",
    "public",
    "ref",
    "return",
    "select",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "switch",
    "this",
    "throw",
    "trait",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "unsafe",
    "use",
    "val",
    "var",
    "void",
    "when",
    "where",
    "while",
    "yield",
];

const HASH_KEYWORDS: &[&str] = &[
    "and", "as", "async", "await", "begin", "break", "case", "class", "continue", "def", "do",
    "done", "elif", "else", "end", "esac", "except", "export", "false", "False", "fi", "finally",
    "for", "from", "function", "if", "import", "in", "is", "lambda", "local", "module", "nil",
    "None", "not", "or", "pass", "raise", "require", "rescue", "return", "then", "true", "True",
    "try", "unless", "while", "with", "yield",
];

/// Highlighting family for a file name.
pub fn lang_for(path: &Path) -> Lang {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if matches!(
        name.as_str(),
        "makefile" | "dockerfile" | ".bashrc" | ".zshrc" | ".profile" | ".gitignore"
    ) {
        return Lang::Hash;
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rs" | "c" | "h" | "cc" | "cpp" | "hpp" | "m" | "mm" | "js" | "mjs" | "cjs" | "jsx"
        | "ts" | "tsx" | "go" | "java" | "kt" | "kts" | "swift" | "cs" | "scala" | "dart"
        | "zig" | "wgsl" | "glsl" | "proto" | "json" | "jsonc" | "php" | "sol" => Lang::CLike,
        "css" | "scss" | "less" => Lang::Css,
        "sh" | "bash" | "zsh" | "fish" | "py" | "rb" | "toml" | "yaml" | "yml" | "pl" | "r"
        | "conf" | "ini" | "cfg" | "mk" | "nix" | "ex" | "exs" | "tf" => Lang::Hash,
        _ => Lang::Plain,
    }
}

/// Carried between lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    pub in_block_comment: bool,
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Spans of one line (byte ranges into `line`); unlisted text is plain.
pub fn line(lang: Lang, state: &mut State, line: &str) -> Vec<(Range<usize>, Kind)> {
    let mut out = Vec::new();
    if lang == Lang::Plain {
        return out;
    }
    let b = line.as_bytes();
    let block = matches!(lang, Lang::CLike | Lang::Css);
    let keywords = match lang {
        Lang::Hash => HASH_KEYWORDS,
        _ => C_KEYWORDS,
    };
    let mut i = 0;
    if state.in_block_comment {
        match line.find("*/") {
            Some(end) => {
                out.push((0..end + 2, Kind::Comment));
                state.in_block_comment = false;
                i = end + 2;
            }
            None => {
                out.push((0..b.len(), Kind::Comment));
                return out;
            }
        }
    }
    while i < b.len() {
        let c = b[i];
        // line comments
        if (lang == Lang::CLike && b[i..].starts_with(b"//"))
            || (lang == Lang::Hash && c == b'#' && (i == 0 || !is_ident(b[i - 1])))
        {
            out.push((i..b.len(), Kind::Comment));
            break;
        }
        if block && b[i..].starts_with(b"/*") {
            match line[i + 2..].find("*/") {
                Some(end) => {
                    let stop = i + 2 + end + 2;
                    out.push((i..stop, Kind::Comment));
                    i = stop;
                }
                None => {
                    out.push((i..b.len(), Kind::Comment));
                    state.in_block_comment = true;
                    break;
                }
            }
            continue;
        }
        if c == b'"' || c == b'\'' || c == b'`' {
            // a Rust lifetime / label ('a, 'static) is not a char literal
            let closing = b[i + 1..].iter().position(|&x| x == c);
            let lifetime = c == b'\'' && lang == Lang::CLike && closing.is_none_or(|p| p > 2);
            if !lifetime {
                let mut j = i + 1;
                while j < b.len() && b[j] != c {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                let stop = (j + 1).min(b.len());
                out.push((i..stop, Kind::String));
                i = stop;
                continue;
            }
        }
        if c.is_ascii_digit() && (i == 0 || !is_ident(b[i - 1])) {
            let mut j = i;
            while j < b.len() && (is_ident(b[j]) || b[j] == b'.') {
                j += 1;
            }
            out.push((i..j, Kind::Number));
            i = j;
            continue;
        }
        if is_ident(c) {
            let mut j = i;
            while j < b.len() && is_ident(b[j]) {
                j += 1;
            }
            if keywords.contains(&&line[i..j]) {
                out.push((i..j, Kind::Keyword));
            }
            i = j;
            continue;
        }
        // skip one UTF-8 character
        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(lang: Lang, state: &mut State, src: &str) -> Vec<(String, Kind)> {
        line(lang, state, src)
            .into_iter()
            .map(|(r, k)| (src[r].to_string(), k))
            .collect()
    }

    #[test]
    fn rust_line() {
        let mut s = State::default();
        assert_eq!(
            spans(
                Lang::CLike,
                &mut s,
                r#"pub fn f<'a>(x: &'a str) -> u8 { "a\"b" ; 42 } // done"#
            ),
            vec![
                ("pub".into(), Kind::Keyword),
                ("fn".into(), Kind::Keyword),
                (r#""a\"b""#.into(), Kind::String),
                ("42".into(), Kind::Number),
                ("// done".into(), Kind::Comment),
            ]
        );
    }

    #[test]
    fn block_comment_spans_lines() {
        let mut s = State::default();
        assert_eq!(
            spans(Lang::CLike, &mut s, "let a = 1; /* start"),
            vec![
                ("let".into(), Kind::Keyword),
                ("1".into(), Kind::Number),
                ("/* start".into(), Kind::Comment)
            ]
        );
        assert!(s.in_block_comment);
        assert_eq!(
            spans(Lang::CLike, &mut s, "middle"),
            vec![("middle".into(), Kind::Comment)]
        );
        assert_eq!(
            spans(Lang::CLike, &mut s, "end */ return"),
            vec![
                ("end */".into(), Kind::Comment),
                ("return".into(), Kind::Keyword)
            ]
        );
        assert!(!s.in_block_comment);
    }

    #[test]
    fn hash_languages_and_plain() {
        let mut s = State::default();
        assert_eq!(
            spans(Lang::Hash, &mut s, "if [ -f x ]; then echo 'hi' # note"),
            vec![
                ("if".into(), Kind::Keyword),
                ("then".into(), Kind::Keyword),
                ("'hi'".into(), Kind::String),
                ("# note".into(), Kind::Comment),
            ]
        );
        // `#` inside a word is not a comment (a URL fragment, $#)
        assert!(spans(Lang::Hash, &mut s, "a#b").is_empty());
        assert!(line(Lang::Plain, &mut s, "fn main() {}").is_empty());
        assert_eq!(lang_for(Path::new("src/main.rs")), Lang::CLike);
        assert_eq!(lang_for(Path::new("Cargo.toml")), Lang::Hash);
        assert_eq!(lang_for(Path::new("Makefile")), Lang::Hash);
        assert_eq!(lang_for(Path::new("README.md")), Lang::Plain);
    }

    #[test]
    fn non_ascii_text_does_not_split_characters() {
        let mut s = State::default();
        assert_eq!(
            spans(Lang::CLike, &mut s, "let é = \"ü\"; // ✓"),
            vec![
                ("let".into(), Kind::Keyword),
                ("\"ü\"".into(), Kind::String),
                ("// ✓".into(), Kind::Comment),
            ]
        );
    }
}
