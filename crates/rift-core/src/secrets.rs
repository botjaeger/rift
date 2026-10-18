//! Secret masking (`mask_secrets`, `[[secret_patterns]]`): tokens, keys and
//! passwords that match a bounded built-in set of patterns, or the user's
//! own, are shown as `•` (one per character, so nothing reflows), and are
//! masked in copies, saved transcripts and what the phone gateway sends.
//!
//! Masking is a display aid, not a guarantee: a secret no pattern knows, one
//! split by the program's own output, or one that scrolled through a log
//! before masking was on is shown as is.

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::{Arc, RwLock};

/// What a masked character shows.
pub const MASK: char = '\u{2022}';
/// Most `[[secret_patterns]]` entries.
pub const MAX_PATTERNS: usize = 64;
/// Longest pattern, in bytes.
pub const MAX_PATTERN_LEN: usize = 1024;
/// Compiled-size bound of one pattern (`RegexBuilder::size_limit`).
const SIZE_LIMIT: usize = 1 << 20;

/// One `[[secret_patterns]]` entry: a name (for the user) and a regular
/// expression. With a capture group, only the first group is masked
/// (`token=(\S+)` keeps `token=` readable); else the whole match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretPattern {
    pub name: String,
    pub regex: String,
}

/// The built-in patterns: (name, regex). Every one is anchored on a prefix
/// or an assignment, so hashes, UUIDs and git SHAs stay readable.
pub const BUILTIN: &[(&str, &str)] = &[
    (
        "AWS access key id",
        r"\b(?:AKIA|ASIA|ABIA|ACCA)[0-9A-Z]{16}\b",
    ),
    (
        "AWS secret access key",
        r#"(?i)aws_?secret_?(?:access_?)?key["']?[ \t]*[=:][ \t]*["']?([A-Za-z0-9/+=]{40})\b"#,
    ),
    (
        "GitHub token",
        r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,255}\b",
    ),
    (
        "GitHub fine-grained token",
        r"\bgithub_pat_[A-Za-z0-9_]{22,255}\b",
    ),
    ("Slack token", r"\bxox[abposr]-[A-Za-z0-9-]{10,250}\b"),
    ("Stripe live key", r"\b(?:sk|rk)_live_[A-Za-z0-9]{16,247}\b"),
    (
        "Private key",
        r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----(?s:.*?-----END (?:[A-Z0-9]+ )*PRIVATE KEY-----)?",
    ),
    (
        "JSON Web Token",
        r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
    ),
    (
        "Password or token assignment",
        r#"(?i)\b(?:password|passwd|pwd|secret|token|api_?key|access_?token|auth_?token|client_?secret)["']?[ \t]*[=:][ \t]*["']?([^\s"'&;,]{4,})"#,
    ),
];

/// The compiled patterns.
#[derive(Debug, Clone)]
pub struct Masker {
    patterns: Vec<(String, Regex)>,
}

fn compile(name: &str, src: &str) -> Result<Regex, String> {
    if src.len() > MAX_PATTERN_LEN {
        return Err(format!(
            "secret pattern {name:?}: longer than {MAX_PATTERN_LEN} bytes"
        ));
    }
    RegexBuilder::new(src)
        .size_limit(SIZE_LIMIT)
        .build()
        .map_err(|e| {
            let first = e.to_string();
            let last = first.lines().last().unwrap_or("").trim().to_string();
            format!("secret pattern {name:?}: {last}")
        })
}

/// `[[secret_patterns]]` problems that reject config.toml like a parse
/// error: too many, an empty name or pattern, one that does not compile.
pub fn validate(user: &[SecretPattern]) -> Result<(), String> {
    Masker::new(user).map(|_| ())
}

impl Masker {
    /// The built-in patterns, then `user`'s.
    pub fn new(user: &[SecretPattern]) -> Result<Masker, String> {
        if user.len() > MAX_PATTERNS {
            return Err(format!("secret_patterns: more than {MAX_PATTERNS} entries"));
        }
        let mut patterns: Vec<(String, Regex)> = BUILTIN
            .iter()
            .map(|(n, s)| (n.to_string(), compile(n, s).expect("built-in pattern")))
            .collect();
        for (i, p) in user.iter().enumerate() {
            let name = if p.name.trim().is_empty() {
                format!("secret_patterns[{i}]")
            } else {
                p.name.trim().to_string()
            };
            if p.regex.is_empty() {
                return Err(format!("secret pattern {name:?}: regex is empty"));
            }
            patterns.push((name.clone(), compile(&name, &p.regex)?));
        }
        Ok(Masker { patterns })
    }

    /// Masked spans of `text` as char ranges `start..end`, sorted, merged.
    pub fn find(&self, text: &str) -> Vec<(usize, usize)> {
        let mut bytes: Vec<(usize, usize)> = Vec::new();
        for (_, re) in &self.patterns {
            let grouped = re.captures_len() > 1;
            for c in re.captures_iter(text) {
                let m = if grouped { c.get(1) } else { c.get(0) };
                if let Some(m) = m.filter(|m| !m.is_empty()) {
                    bytes.push((m.start(), m.end()));
                }
            }
        }
        if bytes.is_empty() {
            return bytes;
        }
        bytes.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (s, e) in bytes {
            match merged.last_mut() {
                Some((_, le)) if s <= *le => *le = (*le).max(e),
                _ => merged.push((s, e)),
            }
        }
        // byte offsets → char indices, in one pass
        let mut out = Vec::with_capacity(merged.len());
        let mut chars = 0usize;
        let mut at = 0usize;
        let count_to = |b: usize, chars: &mut usize, at: &mut usize| {
            *chars += text[*at..b].chars().count();
            *at = b;
            *chars
        };
        for (s, e) in merged {
            let cs = count_to(s, &mut chars, &mut at);
            let ce = count_to(e, &mut chars, &mut at);
            out.push((cs, ce));
        }
        out
    }

    /// `text` with every masked char (line breaks excepted) as [`MASK`].
    pub fn mask<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let spans = self.find(text);
        if spans.is_empty() {
            return Cow::Borrowed(text);
        }
        let mut out = String::with_capacity(text.len());
        let mut spans = spans.into_iter().peekable();
        for (i, c) in text.chars().enumerate() {
            while spans.peek().is_some_and(|(_, e)| *e <= i) {
                spans.next();
            }
            let masked = spans.peek().is_some_and(|(s, _)| *s <= i);
            out.push(if masked && c != '\n' && c != '\r' {
                MASK
            } else {
                c
            });
        }
        Cow::Owned(out)
    }

    /// The names of the patterns, built-in first.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.patterns.iter().map(|(n, _)| n.as_str())
    }
}

/// The masker in force (`mask_secrets` on), set by the app on every config
/// change; read by every place text leaves the screen.
static ACTIVE: RwLock<Option<Arc<Masker>>> = RwLock::new(None);

pub fn set_active(masker: Option<Arc<Masker>>) {
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = masker;
}

/// The masker in force; `None` while masking is off.
pub fn active() -> Option<Arc<Masker>> {
    ACTIVE.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `text` masked when masking is on.
pub fn mask(text: &str) -> Cow<'_, str> {
    match active() {
        Some(m) => match m.mask(text) {
            Cow::Borrowed(_) => Cow::Borrowed(text),
            Cow::Owned(s) => Cow::Owned(s),
        },
        None => Cow::Borrowed(text),
    }
}

/// [`mask`] of an owned string, without a copy when nothing matched.
pub fn mask_owned(text: String) -> String {
    match mask(&text) {
        Cow::Borrowed(_) => text,
        Cow::Owned(s) => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masker() -> Masker {
        Masker::new(&[]).unwrap()
    }

    /// The text of each masked span.
    fn found(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        masker()
            .find(text)
            .into_iter()
            .map(|(s, e)| chars[s..e].iter().collect())
            .collect()
    }

    // Fake tokens only: built from parts so no scanner mistakes them for
    // real credentials, and none of them was ever issued.
    fn fake(prefix: &str, body: &str, n: usize) -> String {
        format!("{prefix}{}", body.repeat(n))
    }

    #[test]
    fn each_builtin_pattern_masks_its_fixture() {
        let aws_id = fake("AKIA", "EXAMPLE0", 2);
        let aws_secret = fake("", "wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEYab", 1);
        let ghp = fake("ghp_", "FAKE0123", 5);
        let pat = fake("github_pat_", "FAKE_0123", 4);
        let slack = fake("xoxb-", "0000-fake", 3);
        let stripe = fake("sk_live_", "FAKEfake", 3);
        let jwt = format!(
            "eyJ{}.eyJ{}.{}",
            "hbGciOiJIUzI1NiJ9", "zdWIiOiJmYWtlIn0", "c2lnbmF0dXJlZmFrZQ"
        );
        let cases: Vec<(String, String)> = vec![
            (format!("key {aws_id} here"), aws_id.clone()),
            (
                format!("aws_secret_access_key = {aws_secret}"),
                aws_secret.clone(),
            ),
            (format!("export GH={ghp}"), ghp.clone()),
            (format!("token: {pat}"), pat.clone()),
            (format!("SLACK {slack} x"), slack.clone()),
            (stripe.clone(), stripe.clone()),
            (
                "-----BEGIN RSA PRIVATE KEY-----".into(),
                "-----BEGIN RSA PRIVATE KEY-----".into(),
            ),
            (format!("Authorization: Bearer {jwt}"), jwt.clone()),
            (
                "mysql --password=hunter2x -u root".into(),
                "hunter2x".into(),
            ),
            ("API_KEY: 'abcd1234efgh'".into(), "abcd1234efgh".into()),
        ];
        for (text, secret) in cases {
            let got = found(&text);
            assert!(got.iter().any(|g| g.contains(&secret)), "{text:?}: {got:?}");
            let masked = masker().mask(&text);
            assert!(!masked.contains(&secret), "{masked}");
            assert_eq!(masked.chars().count(), text.chars().count());
        }
    }

    #[test]
    fn hashes_ids_and_plain_words_stay_readable() {
        for text in [
            // a git SHA (40 hex), a sha256, a UUID
            "commit 3f2c9a1b7d4e5f60718293a4b5c6d7e8f9012345",
            "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            "id 123e4567-e89b-12d3-a456-426614174000",
            "Password:",
            "the token was rotated",
            "ghp_short",
            "sk_test_FAKEfakeFAKEfakeFAKEfake",
            "AKIA lowercase akiaexample0example",
        ] {
            assert!(found(text).is_empty(), "{text:?}: {:?}", found(text));
            assert!(matches!(masker().mask(text), Cow::Borrowed(_)));
        }
    }

    #[test]
    fn a_private_key_block_is_masked_whole_line_breaks_kept() {
        let text = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAA\nAAAABG5vbmU=\n-----END OPENSSH PRIVATE KEY-----\nafter";
        let masked = masker().mask(text);
        assert!(!masked.contains("b3Blbn") && !masked.contains("AAAABG"));
        assert_eq!(masked.lines().count(), text.lines().count());
        assert!(masked.ends_with("\nafter"), "{masked}");
    }

    #[test]
    fn user_patterns_add_to_the_builtins_and_bad_ones_are_rejected() {
        let user = [SecretPattern {
            name: "internal".into(),
            regex: r"\bcorp-[0-9]{6}\b".into(),
        }];
        let m = Masker::new(&user).unwrap();
        assert_eq!(m.find("id corp-123456 x"), [(3, 14)]);
        assert!(m.names().any(|n| n == "internal"));
        let bad = |regex: &str| {
            Masker::new(&[SecretPattern {
                name: "bad".into(),
                regex: regex.into(),
            }])
            .unwrap_err()
        };
        assert!(bad("(unclosed").contains("\"bad\""));
        assert!(bad("").contains("empty"));
        assert!(bad(&"a".repeat(MAX_PATTERN_LEN + 1)).contains("longer"));
        // a pattern whose program is huge is refused, not compiled
        assert!(!bad(r"\w{1000}\w{1000}\w{1000}").is_empty());
        let many = vec![user[0].clone(); MAX_PATTERNS + 1];
        assert!(validate(&many).is_err());
    }

    #[test]
    fn spans_count_chars_not_bytes() {
        let text = format!("é → {}", fake("ghp_", "FAKE0123", 5));
        assert_eq!(masker().find(&text), [(4, 4 + 44)]);
    }

    #[test]
    fn matching_cost_stays_linear() {
        // a long line of near-misses: well under a millisecond per KiB
        let line = "token ".repeat(20_000) + "a=b ";
        let t = std::time::Instant::now();
        let _ = masker().find(&line);
        assert!(
            t.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            t.elapsed()
        );
    }
}
