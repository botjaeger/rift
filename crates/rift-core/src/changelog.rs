//! Release notes for the What's New page: the repository's `CHANGELOG.md`,
//! embedded at build time and parsed into releases, plus the last version
//! the user has seen (so the page opens once after an update).

use std::path::PathBuf;

/// The changelog shipped in the binary.
pub const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

/// The running version (workspace `version`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// `YYYY-MM-DD`, empty when the heading has none.
    pub date: String,
    /// Paragraph between the heading and the first section.
    pub summary: String,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// `New`, `Improved`, `Fixed`, …
    pub title: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Bold lead (`**Title**`), empty when the bullet has none.
    pub title: String,
    pub text: String,
}

/// Parse `## <version> — <date>` releases. Text above the first release
/// (the file's own preamble) is ignored; markdown beyond the documented
/// format is kept as plain text.
pub fn parse(src: &str) -> Vec<Release> {
    let mut releases: Vec<Release> = Vec::new();
    for raw in src.lines() {
        let line = raw.trim_end();
        if let Some(head) = line.strip_prefix("## ") {
            let (version, date) = match head.split_once(" — ").or_else(|| head.split_once(" - "))
            {
                Some((v, d)) => (v.trim(), d.trim()),
                None => (head.trim(), ""),
            };
            releases.push(Release {
                version: version.trim_start_matches('v').to_string(),
                date: date.to_string(),
                summary: String::new(),
                sections: Vec::new(),
            });
            continue;
        }
        let Some(release) = releases.last_mut() else {
            continue;
        };
        if let Some(title) = line.strip_prefix("### ") {
            release.sections.push(Section {
                title: title.trim().to_string(),
                items: Vec::new(),
            });
        } else if let Some(bullet) = line.strip_prefix("- ") {
            let item = item(bullet.trim());
            match release.sections.last_mut() {
                Some(section) => section.items.push(item),
                // a bullet before any section gets an untitled one
                None => release.sections.push(Section {
                    title: String::new(),
                    items: vec![item],
                }),
            }
        } else if !line.trim().is_empty() {
            let text = line.trim();
            // an indented line continues the last bullet; loose text before
            // the first section is the summary
            let target = if release.sections.is_empty() {
                &mut release.summary
            } else if raw.starts_with(' ')
                && let Some(item) = release.sections.last_mut().and_then(|s| s.items.last_mut())
            {
                &mut item.text
            } else {
                continue;
            };
            if !target.is_empty() {
                target.push(' ');
            }
            target.push_str(text);
        }
    }
    for release in &mut releases {
        release.summary = plain(&release.summary);
        for item in release.sections.iter_mut().flat_map(|s| &mut s.items) {
            item.text = plain(&item.text);
        }
    }
    releases
}

/// `*Relay*` → `Relay`: the file's italics (the names the UI shows) read as
/// plain text. A lone `*` (`rm *`) or one next to a space stays.
fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('*') {
        let after = &rest[i + 1..];
        match after.find('*') {
            Some(j) if j > 0 && !after[..j].starts_with(' ') && !after[..j].ends_with(' ') => {
                out.push_str(&rest[..i]);
                out.push_str(&after[..j]);
                rest = &after[j + 1..];
            }
            _ => {
                out.push_str(&rest[..=i]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `**Title** — text` → (Title, text); anything else is all text.
fn item(bullet: &str) -> Item {
    if let Some(rest) = bullet.strip_prefix("**")
        && let Some((title, text)) = rest.split_once("**")
    {
        let text = text.trim_start();
        let text = text
            .strip_prefix('—')
            .or_else(|| text.strip_prefix(':'))
            .or_else(|| text.strip_prefix('-'))
            .unwrap_or(text);
        return Item {
            title: title.trim().to_string(),
            text: text.trim().to_string(),
        };
    }
    Item {
        title: String::new(),
        text: bullet.to_string(),
    }
}

/// The embedded changelog, parsed.
pub fn releases() -> Vec<Release> {
    parse(CHANGELOG)
}

fn seen_path() -> Option<PathBuf> {
    crate::shell_integration::data_dir().map(|d| d.join("last-seen-version"))
}

/// True when this launch follows an update: a version was seen before and
/// it differs from the running one. A first install records the version
/// without opening the page.
pub fn opens_after_update(last_seen: Option<&str>, current: &str) -> bool {
    last_seen.is_some_and(|v| v.trim() != current)
}

/// The version recorded by the last `mark_seen`.
pub fn last_seen() -> Option<String> {
    std::fs::read_to_string(seen_path()?).ok()
}

/// Record the running version as seen.
pub fn mark_seen() {
    let Some(path) = seen_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, VERSION) {
        log::warn!("changelog: write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Changelog\n\nPreamble, ignored.\n\n## 1.2.0 — 2026-10-01\n\nA summary\nover two lines.\n\n### New\n\n- **Blocks** — every command\n  is a block.\n- plain bullet\n\n### Fixed\n\n- **Paste**: asks first.\n\n## v1.1.0\n\n- **Loose** — bullet with no section.\n";

    #[test]
    fn parses_releases_sections_and_items() {
        let r = parse(SAMPLE);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].version, "1.2.0");
        assert_eq!(r[0].date, "2026-10-01");
        assert_eq!(r[0].summary, "A summary over two lines.");
        assert_eq!(r[0].sections.len(), 2);
        assert_eq!(r[0].sections[0].title, "New");
        assert_eq!(
            r[0].sections[0].items[0],
            Item {
                title: "Blocks".into(),
                text: "every command is a block.".into()
            }
        );
        assert_eq!(r[0].sections[0].items[1].title, "");
        assert_eq!(r[0].sections[0].items[1].text, "plain bullet");
        assert_eq!(r[0].sections[1].items[0].title, "Paste");
        assert_eq!(r[0].sections[1].items[0].text, "asks first.");
        assert_eq!(r[1].version, "1.1.0");
        assert_eq!(r[1].date, "");
        assert_eq!(r[1].sections[0].title, "");
        assert_eq!(r[1].sections[0].items[0].title, "Loose");
    }

    #[test]
    fn embedded_changelog_lists_the_running_version_first() {
        let r = releases();
        assert!(!r.is_empty(), "CHANGELOG.md has no releases");
        assert_eq!(
            r[0].version, VERSION,
            "add a CHANGELOG.md entry for {VERSION}"
        );
        for release in &r {
            assert!(!release.sections.is_empty(), "{} is empty", release.version);
            for section in &release.sections {
                for item in &section.items {
                    assert!(!item.title.is_empty() && !item.text.is_empty());
                }
            }
        }
    }

    #[test]
    fn italics_read_as_plain_text() {
        assert_eq!(plain("Point *Relay* at it"), "Point Relay at it");
        assert_eq!(
            plain("now *Use a tunnel instead*, below"),
            "now Use a tunnel instead, below"
        );
        // not emphasis: a lone star, or one next to a space
        assert_eq!(plain("rm * here"), "rm * here");
        assert_eq!(plain("a * b * c"), "a * b * c");
        let r =
            parse("## 1.0.0 — 2026-01-01\n\nSee *About*.\n\n### New\n\n- **X** — open *Relay*.\n");
        assert_eq!(r[0].summary, "See About.");
        assert_eq!(r[0].sections[0].items[0].text, "open Relay.");
        // the shipped notes show no stray stars
        for r in releases() {
            for text in std::iter::once(&r.summary).chain(
                r.sections
                    .iter()
                    .flat_map(|s| s.items.iter().map(|i| &i.text)),
            ) {
                assert!(!text.contains("*Relay*"), "{text}");
            }
        }
    }

    #[test]
    fn opens_only_after_an_update() {
        assert!(!opens_after_update(None, "0.2.0"), "first install");
        assert!(!opens_after_update(Some("0.2.0"), "0.2.0"));
        assert!(!opens_after_update(Some("0.2.0\n"), "0.2.0"));
        assert!(opens_after_update(Some("0.1.0"), "0.2.0"));
    }
}
