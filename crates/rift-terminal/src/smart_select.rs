//! Smart double-click: the unit under the pointer — a URL, a file path
//! (with its `:line:col`), an email address, an IPv4/IPv6 address, a
//! number (decimal, float or hex), a UUID or a git SHA — selected whole,
//! where word selection would stop at its `:`, `/` or `@`. Anything else
//! falls back to the terminal's word selection, whose boundaries
//! [`word_boundaries`] derives from `word_chars`.
//!
//! Text is one grid row, one char per cell, so char indices are columns.

use std::ops::Range;

/// Chars that never belong to a unit: blanks, quotes, brackets, separators.
fn stop(c: char) -> bool {
    c.is_whitespace() || "\"'`<>()[]{}|,;".contains(c)
}

/// The smart unit covering column `col` of `text` (a char range), or
/// `None` when the click is not inside one. Trailing sentence punctuation
/// (`.` `:` `!` `?`) and surrounding quotes and brackets are left out.
pub fn unit_at(text: &str, col: usize) -> Option<Range<usize>> {
    let chars: Vec<char> = text.chars().collect();
    if col >= chars.len() || stop(chars[col]) {
        return None;
    }
    let mut s = col;
    while s > 0 && !stop(chars[s - 1]) {
        s -= 1;
    }
    let mut e = col + 1;
    while e < chars.len() && !stop(chars[e]) {
        e += 1;
    }
    while e > s && ".:!?".contains(chars[e - 1]) {
        e -= 1;
    }
    if !(s..e).contains(&col) {
        return None;
    }
    let token: String = chars[s..e].iter().collect();
    if is_unit(&token) {
        return Some(s..e);
    }
    // `--port=8080`, `url=https://…`: the side of the `=` that was clicked
    let mut start = s;
    for part in token.split('=') {
        let len = part.chars().count();
        if (start..start + len).contains(&col) {
            return is_unit(part).then_some(start..start + len);
        }
        start += len + 1;
    }
    None
}

fn is_unit(t: &str) -> bool {
    !t.is_empty()
        && (is_url(t)
            || is_email(t)
            || is_uuid(t)
            || is_ipv4(t)
            || is_ipv6(t)
            || is_number(t)
            || is_sha(t)
            || is_path(t))
}

/// `scheme://rest` or `mailto:address`.
fn is_url(t: &str) -> bool {
    if let Some(addr) = t.strip_prefix("mailto:") {
        return is_email(addr);
    }
    let Some((scheme, rest)) = t.split_once("://") else {
        return false;
    };
    !rest.is_empty()
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
}

/// `local@domain.tld`.
fn is_email(t: &str) -> bool {
    let Some((local, domain)) = t.split_once('@') else {
        return false;
    };
    let labels: Vec<&str> = domain.split('.').collect();
    !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c))
        && labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && labels
            .last()
            .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()))
}

fn hex(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// `8-4-4-4-12` hex digits.
fn is_uuid(t: &str) -> bool {
    let parts: Vec<&str> = t.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(p, n)| p.len() == n && hex(p))
}

/// Four decimal octets, with an optional `/prefix` or `:port`.
fn is_ipv4(t: &str) -> bool {
    let (addr, suffix) = match t.find(['/', ':']) {
        Some(i) => (&t[..i], &t[i + 1..]),
        None => (t, "0"),
    };
    let octets: Vec<&str> = addr.split('.').collect();
    octets.len() == 4
        && octets
            .iter()
            .all(|o| !o.is_empty() && o.len() <= 3 && o.parse::<u8>().is_ok())
        && !suffix.is_empty()
        && suffix.len() <= 5
        && suffix.chars().all(|c| c.is_ascii_digit())
}

/// Hex groups split by `:`, at most one `::`, at most eight groups (the
/// last may be a dotted IPv4).
fn is_ipv6(t: &str) -> bool {
    if t.matches(':').count() < 2 || t.matches("::").count() > 1 || t.contains(":::") {
        return false;
    }
    let groups: Vec<&str> = t.split(':').collect();
    if groups.len() > 8 {
        return false;
    }
    let last = groups.len() - 1;
    groups.iter().enumerate().all(|(i, g)| {
        g.is_empty() && (i == 0 || i == last || t.contains("::"))
            || g.len() <= 4 && hex(g)
            || i == last && is_ipv4(g)
    }) && groups.iter().any(|g| !g.is_empty())
}

/// `42`, `-3.5`, `1e9`, `1_000`, `0x1F`.
fn is_number(t: &str) -> bool {
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return hex(&h.replace('_', ""));
    }
    let t = t.strip_prefix(['-', '+']).unwrap_or(t);
    let (mantissa, exp) = match t.find(['e', 'E']) {
        Some(i) => (&t[..i], Some(&t[i + 1..])),
        None => (t, None),
    };
    let digits = |s: &str| {
        !s.is_empty() && !s.starts_with('_') && s.chars().all(|c| c.is_ascii_digit() || c == '_')
    };
    let mantissa_ok = match mantissa.split_once('.') {
        Some((int, frac)) => digits(int) && digits(frac),
        None => digits(mantissa),
    };
    mantissa_ok && exp.is_none_or(|e| digits(e.strip_prefix(['-', '+']).unwrap_or(e)))
}

/// A git object id: 7 to 40 hex digits, a letter among them (all digits
/// is a number).
fn is_sha(t: &str) -> bool {
    (7..=40).contains(&t.len()) && hex(t) && t.chars().any(|c| c.is_ascii_alphabetic())
}

/// A path (`/`, `./`, `~/` or a `name.ext`, the extension starting
/// with a letter), with an optional
/// `:line[:col]`.
fn is_path(t: &str) -> bool {
    let mut path = t;
    for _ in 0..2 {
        if let Some((head, n)) = path.rsplit_once(':')
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
        {
            path = head;
        }
    }
    if path.is_empty() || path.contains(':') || path.contains("://") {
        return false;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    path.contains('/')
        || name.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty() && ext.starts_with(|c: char| c.is_ascii_alphabetic())
        })
}

/// The terminal's default word boundaries (alacritty's
/// `SEMANTIC_ESCAPE_CHARS`) less the `word_chars` config's characters,
/// plus the `word_delimiters` config's (which wins over `word_chars`).
pub fn word_boundaries(word_chars: &str, word_delimiters: &str) -> String {
    let mut out: String = rift_pty::alacritty_terminal::term::SEMANTIC_ESCAPE_CHARS
        .chars()
        .filter(|c| !word_chars.contains(*c) || word_delimiters.contains(*c))
        .collect();
    for c in word_delimiters.chars() {
        if !out.contains(c) {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The unit double-clicked at the first char of `needle` in `text`.
    fn at(text: &str, needle: &str) -> Option<String> {
        let byte = text.find(needle).unwrap();
        let col = text[..byte].chars().count();
        unit_at(text, col).map(|r| text.chars().skip(r.start).take(r.len()).collect())
    }

    #[test]
    fn urls_whole_without_trailing_punctuation() {
        let t = "see https://example.com/a/b?x=1#frag. Done";
        assert_eq!(
            at(t, "example").as_deref(),
            Some("https://example.com/a/b?x=1#frag")
        );
        assert_eq!(
            at("(ftp://host/file)", "host").as_deref(),
            Some("ftp://host/file")
        );
        assert_eq!(
            at("'ssh://git@host:22/r'", "git@").as_deref(),
            Some("ssh://git@host:22/r")
        );
        assert_eq!(
            at("x mailto:me@ex.org,", "me@").as_deref(),
            Some("mailto:me@ex.org")
        );
    }

    #[test]
    fn paths_with_line_and_column() {
        assert_eq!(
            at("error at src/main.rs:42:7: oops", "main").as_deref(),
            Some("src/main.rs:42:7")
        );
        assert_eq!(at("open ./x.py.", "x.py").as_deref(), Some("./x.py"));
        assert_eq!(
            at("\"~/notes/todo.md\"", "todo").as_deref(),
            Some("~/notes/todo.md")
        );
        assert_eq!(at("Cargo.toml:3", "toml").as_deref(), Some("Cargo.toml:3"));
    }

    #[test]
    fn emails_ips_numbers_uuids_shas() {
        assert_eq!(
            at("<dev.ops+x@mail.example.com>", "ops").as_deref(),
            Some("dev.ops+x@mail.example.com")
        );
        assert_eq!(
            at("from 192.168.0.12: up", "168").as_deref(),
            Some("192.168.0.12")
        );
        assert_eq!(
            at("bind 10.0.0.1:8080", "0.0").as_deref(),
            Some("10.0.0.1:8080")
        );
        assert_eq!(
            at("net 10.0.0.0/24,", "10.").as_deref(),
            Some("10.0.0.0/24")
        );
        assert_eq!(
            at("addr [fe80::1ff:fe23:4567:890a]", "1ff").as_deref(),
            Some("fe80::1ff:fe23:4567:890a")
        );
        assert_eq!(at("lo ::1 up", "::1").as_deref(), Some("::1"));
        assert_eq!(at("took 3.1415s", "3.").as_deref(), None);
        assert_eq!(at("took 3.1415 s", "3.").as_deref(), Some("3.1415"));
        assert_eq!(at("val=-2.5e-3;", "2.5").as_deref(), Some("-2.5e-3"));
        assert_eq!(at("mask 0xFF00.", "FF").as_deref(), Some("0xFF00"));
        assert_eq!(at("n=1_000_000", "000").as_deref(), Some("1_000_000"));
        assert_eq!(
            at("id 123e4567-e89b-12d3-a456-426614174000 ok", "e89b").as_deref(),
            Some("123e4567-e89b-12d3-a456-426614174000")
        );
        assert_eq!(at("commit 4ff62eb feat", "ff6").as_deref(), Some("4ff62eb"));
        assert_eq!(
            at("(3a45dce9b2c1f00d3a45dce9b2c1f00d3a45dce9)", "dce9b").as_deref(),
            Some("3a45dce9b2c1f00d3a45dce9b2c1f00d3a45dce9")
        );
        // the clicked side of an `=`
        assert_eq!(at("--port=8080", "80").as_deref(), Some("8080"));
    }

    #[test]
    fn plain_words_and_blanks_fall_back() {
        assert_eq!(at("hello world", "world"), None);
        assert_eq!(at("foo:bar baz", "bar"), None);
        assert_eq!(at("abcdef", "abc"), None); // 6 hex digits: not a SHA
        assert_eq!(at("deadbeefx", "dead"), None);
        assert_eq!(unit_at("a b", 1), None);
        assert_eq!(unit_at("ab", 5), None);
        assert_eq!(at("999.999.1.1", "999"), None); // not an IPv4
        assert_eq!(at("1.2.3", "1.2"), None); // a version, not a path
    }

    #[test]
    fn word_chars_drop_boundaries() {
        assert_eq!(word_boundaries("", ""), ",│`|:\"' ()[]{}<>\t");
        assert_eq!(word_boundaries(":|", ""), ",│`\"' ()[]{}<>\t");
    }

    #[test]
    fn word_delimiters_add_boundaries() {
        assert_eq!(word_boundaries("", "/."), ",│`|:\"' ()[]{}<>\t/.");
        // a delimiter wins over the same character in word_chars
        assert_eq!(word_boundaries(":|", ":"), ",│`:\"' ()[]{}<>\t");
    }
}
