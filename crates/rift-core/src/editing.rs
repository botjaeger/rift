//! Command-line editing helpers for the owned editor's Warp keys: the last
//! word of a command (Meta+.), subword motion (⌃⌥← / ⌃⌥→) and how much of
//! the ghost text a partial accept takes (⌥→, Tab).

use std::ops::Range;

/// The last shell word of `cmd`, as typed: quotes and backslash escapes
/// stay (`cp a "my file"` → `"my file"`, `cp a my\ file` → `my\ file`).
/// `None` for a blank command.
pub fn last_word(cmd: &str) -> Option<&str> {
    let mut last: Option<Range<usize>> = None;
    let mut start: Option<usize> = None;
    let mut quote: Option<char> = None;
    let mut chars = cmd.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match (quote, c) {
            (Some('\''), '\'') => quote = None,
            (Some('"'), '"') => quote = None,
            (Some('"'), '\\') => {
                chars.next();
            }
            (Some(_), _) => {}
            (None, c) if c.is_whitespace() => {
                if let Some(s) = start.take() {
                    last = Some(s..i);
                }
                continue;
            }
            (None, '\\') => {
                start.get_or_insert(i);
                chars.next();
                continue;
            }
            (None, '\'' | '"') => quote = Some(c),
            (None, _) => {}
        }
        start.get_or_insert(i);
    }
    if let Some(s) = start {
        last = Some(s..cmd.len());
    }
    last.map(|r| &cmd[r])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Lower,
    Upper,
    Digit,
    /// Whitespace, `_`, `-`, `.`, `/` and other punctuation.
    Sep,
}

fn class(c: char) -> Class {
    if c.is_lowercase() {
        Class::Lower
    } else if c.is_uppercase() {
        Class::Upper
    } else if c.is_numeric() {
        Class::Digit
    } else if c.is_alphabetic() {
        // letters without case (CJK, …) run like lower case
        Class::Lower
    } else {
        Class::Sep
    }
}

/// Byte offset of the subword start left of `at` (⌃⌥←): words split at
/// separators (`_ - . /`, spaces), at a lower-to-upper step (`camelCase`)
/// and before the last capital of a run followed by lower case
/// (`HTTPServer` → `HTTP`, `Server`).
pub fn subword_left(s: &str, at: usize) -> usize {
    let chars: Vec<(usize, char)> = s[..at.min(s.len())].char_indices().collect();
    let mut i = chars.len();
    while i > 0 && class(chars[i - 1].1) == Class::Sep {
        i -= 1;
    }
    if i == 0 {
        return 0;
    }
    match class(chars[i - 1].1) {
        Class::Lower => {
            while i > 0 && class(chars[i - 1].1) == Class::Lower {
                i -= 1;
            }
            // the capital a camel-case word starts with
            if i > 0 && class(chars[i - 1].1) == Class::Upper {
                i -= 1;
            }
        }
        k => {
            while i > 0 && class(chars[i - 1].1) == k {
                i -= 1;
            }
        }
    }
    chars.get(i).map_or(0, |&(b, _)| b)
}

/// Byte offset of the subword end right of `at` (⌃⌥→); boundaries as
/// [`subword_left`].
pub fn subword_right(s: &str, at: usize) -> usize {
    let at = at.min(s.len());
    let chars: Vec<(usize, char)> = s[at..].char_indices().map(|(b, c)| (at + b, c)).collect();
    let n = chars.len();
    let end = |i: usize| chars.get(i).map_or(s.len(), |&(b, _)| b);
    let mut i = 0;
    while i < n && class(chars[i].1) == Class::Sep {
        i += 1;
    }
    if i == n {
        return s.len();
    }
    match class(chars[i].1) {
        Class::Upper => {
            let run = i;
            while i < n && class(chars[i].1) == Class::Upper {
                i += 1;
            }
            if i - run == 1 {
                // `Server`: one capital, then its lower case
                while i < n && class(chars[i].1) == Class::Lower {
                    i += 1;
                }
            } else if i < n && class(chars[i].1) == Class::Lower {
                // `HTTPServer`: the run's last capital starts the next word
                i -= 1;
            }
        }
        k => {
            while i < n && class(chars[i].1) == k {
                i += 1;
            }
        }
    }
    end(i)
}

/// Byte length of the ghost text's next word: its leading whitespace and
/// the next run of non-space characters (⌥→, ⌃→).
pub fn ghost_word(ghost: &str) -> usize {
    let lead = ghost.len() - ghost.trim_start().len();
    ghost[lead..]
        .find(char::is_whitespace)
        .map_or(ghost.len(), |i| lead + i)
}

/// Byte length of the ghost text's next part (Tab): [`ghost_word`], cut
/// after the first run of `/` that follows a name, so a path goes a folder
/// at a time (`scripts/`, `/usr/`, `~/`, `https://`).
pub fn ghost_segment(ghost: &str) -> usize {
    let lead = ghost.len() - ghost.trim_start().len();
    let word = &ghost[lead..ghost_word(ghost)];
    // a leading `/` stays with the name after it
    let name = word.len() - word.trim_start_matches('/').len();
    match word[name..].find('/') {
        Some(i) => {
            let rest = &word[name + i..];
            lead + name + i + rest.len() - rest.trim_start_matches('/').len()
        }
        None => lead + word.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_word_keeps_quotes_and_escapes() {
        assert_eq!(last_word("git status"), Some("status"));
        assert_eq!(last_word("cp a \"my file\""), Some("\"my file\""));
        assert_eq!(last_word("cp a 'it''s here'"), Some("'it''s here'"));
        assert_eq!(last_word("cp a my\\ file"), Some("my\\ file"));
        assert_eq!(last_word("echo --name=\"a b\"  "), Some("--name=\"a b\""));
        assert_eq!(last_word("echo \"a \\\" b\""), Some("\"a \\\" b\""));
        assert_eq!(last_word("ls\n  src/main.rs"), Some("src/main.rs"));
        assert_eq!(last_word("vim"), Some("vim"));
        assert_eq!(last_word("echo ünïcødé"), Some("ünïcødé"));
        assert_eq!(last_word("   "), None);
        assert_eq!(last_word(""), None);
        // an unterminated quote runs to the end
        assert_eq!(last_word("echo \"a b"), Some("\"a b"));
    }

    fn walk_right(s: &str) -> Vec<usize> {
        let (mut at, mut out) = (0, Vec::new());
        while at < s.len() {
            at = subword_right(s, at);
            out.push(at);
        }
        out
    }

    fn walk_left(s: &str) -> Vec<usize> {
        let (mut at, mut out) = (s.len(), Vec::new());
        while at > 0 {
            at = subword_left(s, at);
            out.push(at);
        }
        out
    }

    #[test]
    fn subwords_split_case_and_separators() {
        // camelCase, snake_case, kebab-case, paths
        assert_eq!(walk_right("fooBarBaz"), [3, 6, 9]);
        assert_eq!(walk_left("fooBarBaz"), [6, 3, 0]);
        assert_eq!(walk_right("foo_bar_baz"), [3, 7, 11]);
        assert_eq!(walk_left("foo_bar_baz"), [8, 4, 0]);
        assert_eq!(walk_right("git-cherry-pick"), [3, 10, 15]);
        assert_eq!(walk_left("git-cherry-pick"), [11, 4, 0]);
        assert_eq!(walk_right("HTTPServer"), [4, 10]);
        assert_eq!(walk_left("HTTPServer"), [4, 0]);
        assert_eq!(walk_right("v2Api"), [1, 2, 5]);
        assert_eq!(walk_right("src/lib.rs"), [3, 7, 10]);
        assert_eq!(walk_left("cd  my_Dir"), [7, 4, 0]);
        // multi-byte letters
        assert_eq!(walk_right("çaVa"), [3, 5]);
        assert_eq!(subword_left("", 0), 0);
        assert_eq!(subword_right("a ", 1), 2);
    }

    /// The pieces successive accepts of `ghost` take with `take`.
    fn accepts(mut ghost: &str, take: fn(&str) -> usize) -> Vec<&str> {
        let mut out = Vec::new();
        while !ghost.is_empty() {
            let (part, rest) = ghost.split_at(take(ghost));
            out.push(part);
            ghost = rest;
        }
        out
    }

    #[test]
    fn ghost_words_and_segments() {
        let line = "bash scripts/prisma-admin.sh backup";
        assert_eq!(
            accepts(line, ghost_word),
            ["bash", " scripts/prisma-admin.sh", " backup"]
        );
        assert_eq!(
            accepts(line, ghost_segment),
            ["bash", " scripts/", "prisma-admin.sh", " backup"]
        );
        // a leading `/` stays with its name; a run of them stays together
        assert_eq!(accepts(" /usr/bin", ghost_segment), [" /usr/", "bin"]);
        assert_eq!(accepts("~/code/x", ghost_segment), ["~/", "code/", "x"]);
        assert_eq!(accepts("../src", ghost_segment), ["../", "src"]);
        assert_eq!(
            accepts("curl https://x.io/a", ghost_segment),
            ["curl", " https://", "x.io/", "a"]
        );
        assert_eq!(accepts("cd src/", ghost_segment), ["cd", " src/"]);
        assert_eq!(accepts("cat é/ü", ghost_segment), ["cat", " é/", "ü"]);
        // whitespace alone goes in one piece
        assert_eq!(ghost_segment("  "), 2);
        assert_eq!(ghost_word("  "), 2);
        assert_eq!(ghost_segment(""), 0);
    }
}
