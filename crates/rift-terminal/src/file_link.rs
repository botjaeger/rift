//! File links in output: the path under the pointer and the line (and
//! column) printed with it, in the forms compilers, linters and tracebacks
//! use:
//!
//! * `src/main.rs:42` / `src/main.rs:42:7` (rustc, gcc, grep -n, eslint)
//! * `src/App.cs(42,7)` / `src/App.cs(42, 7)` (MSBuild, tsc --pretty false)
//! * `src/x.ts[42, 7]`
//! * `src/x.py, line 42, column 7`
//! * `File "src/x.py", line 42` (Python tracebacks)
//! * `src/x.pl line 42` (Perl's `die`)
//!
//! Text is one grid row, one char per cell, so char indices are columns.

/// A 1-based line and column to open a file at (`App::open_file_at`).
pub type At = Option<(u32, Option<u32>)>;

/// A path printed in output, with the position printed beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLink {
    pub path: String,
    /// 1-based line.
    pub line: Option<u32>,
    /// 1-based column.
    pub col: Option<u32>,
}

impl FileLink {
    /// The line and column as `App::open_file_at` takes them.
    pub fn at(&self) -> At {
        self.line.map(|l| (l, self.col))
    }
}

fn stop(c: char) -> bool {
    c.is_whitespace() || "\"'`<>()[]{}|,;".contains(c)
}

/// The file link whose path covers column `col` of `text`.
pub fn link_at(text: &str, col: usize) -> Option<FileLink> {
    let chars: Vec<char> = text.chars().collect();
    if col >= chars.len() || stop(chars[col]) {
        return None;
    }
    let mut start = col;
    while start > 0 && !stop(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col + 1;
    while end < chars.len() && !stop(chars[end]) {
        end += 1;
    }
    let raw: String = chars[start..end].iter().collect();
    let token = raw.trim_end_matches(['.', ':']);
    // `path:line[:col]`
    let mut parts: Vec<&str> = token.split(':').collect();
    let mut nums: Vec<u32> = Vec::new();
    while parts.len() > 1 && nums.len() < 2 {
        match parts.last().and_then(|p| p.parse::<u32>().ok()) {
            Some(n) => {
                nums.insert(0, n);
                parts.pop();
            }
            None => break,
        }
    }
    let path = parts.join(":");
    if path.is_empty() || !(path.contains('/') || path.contains('.')) || path.contains("://") {
        return None;
    }
    let (line, col) = match nums[..] {
        [l, c] => (Some(l), Some(c)),
        [l] => (Some(l), None),
        _ => suffix_position(&chars[end..]).map_or((None, None), |(l, c)| (Some(l), c)),
    };
    Some(FileLink { path, line, col })
}

/// A position printed right after a path: `(l,c)`, `[l, c]`, `, line l,
/// column c`, `", line l` or ` line l`.
fn suffix_position(rest: &[char]) -> Option<(u32, Option<u32>)> {
    let s: String = rest.iter().take(40).collect();
    // `(42,7)` `(42, 7)` `[42, 7]` `(42)`
    if let Some(close) = match s.chars().next() {
        Some('(') => Some(')'),
        Some('[') => Some(']'),
        _ => None,
    } {
        let inner = &s[1..s.find(close)?];
        let mut it = inner.split(',').map(str::trim);
        let line = it.next()?.parse().ok()?;
        let col = match it.next() {
            Some(c) => Some(c.parse().ok()?),
            None => None,
        };
        return it.next().is_none().then_some((line, col));
    }
    // a closing quote around the path (`File "x.py", line 3`)
    let s = s.strip_prefix(['"', '\'']).unwrap_or(&s);
    let rest = s
        .strip_prefix(", line ")
        .or_else(|| s.strip_prefix(" line "))?;
    let (line, rest) = leading_number(rest)?;
    let col = rest
        .strip_prefix(", column ")
        .or_else(|| rest.strip_prefix(", col "))
        .and_then(leading_number)
        .map(|(c, _)| c);
    Some((line, col))
}

fn leading_number(s: &str) -> Option<(u32, &str)> {
    let n = s.chars().take_while(char::is_ascii_digit).count();
    Some((s[..n].parse().ok()?, &s[n..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(text: &str, needle: &str) -> Option<(String, Option<u32>, Option<u32>)> {
        let col = text[..text.find(needle).unwrap()].chars().count();
        link_at(text, col).map(|l| (l.path, l.line, l.col))
    }

    fn some(
        path: &str,
        line: Option<u32>,
        col: Option<u32>,
    ) -> Option<(String, Option<u32>, Option<u32>)> {
        Some((path.to_string(), line, col))
    }

    #[test]
    fn colon_suffixes() {
        assert_eq!(
            link("src/main.rs:42:7: error", "main"),
            some("src/main.rs", Some(42), Some(7))
        );
        assert_eq!(link("at ./a.py:3.", "a.py"), some("./a.py", Some(3), None));
        assert_eq!(
            link("see README.md", "README"),
            some("README.md", None, None)
        );
    }

    #[test]
    fn parenthesised_and_bracketed() {
        assert_eq!(
            link("src/App.cs(42,7): error CS1002", "App"),
            some("src/App.cs", Some(42), Some(7))
        );
        assert_eq!(
            link("src/App.cs(42, 7): warning", "App"),
            some("src/App.cs", Some(42), Some(7))
        );
        assert_eq!(
            link("lib/x.ts[12, 3] oops", "x.ts"),
            some("lib/x.ts", Some(12), Some(3))
        );
        assert_eq!(
            link("lib/x.ts(12)", "x.ts"),
            some("lib/x.ts", Some(12), None)
        );
        // not a position: no line
        assert_eq!(
            link("lib/x.ts(see docs)", "x.ts"),
            some("lib/x.ts", None, None)
        );
        assert_eq!(
            link("lib/x.ts[1,2,3]", "x.ts"),
            some("lib/x.ts", None, None)
        );
    }

    #[test]
    fn worded_positions() {
        assert_eq!(
            link("  File \"/srv/app/main.py\", line 12, in <module>", "main"),
            some("/srv/app/main.py", Some(12), None)
        );
        assert_eq!(
            link("app/x.py, line 4, column 9: E501", "x.py"),
            some("app/x.py", Some(4), Some(9))
        );
        assert_eq!(
            link("died at lib/Foo.pm line 31.", "Foo"),
            some("lib/Foo.pm", Some(31), None)
        );
        assert_eq!(
            link("lib/Foo.pm line x", "Foo"),
            some("lib/Foo.pm", None, None)
        );
    }

    #[test]
    fn not_links() {
        assert_eq!(link("hello world", "world"), None);
        assert_eq!(link("https://x.com/a.html", "x.com"), None);
        assert_eq!(link_at("a b", 1), None);
        assert_eq!(link_at("", 0), None);
    }
}
