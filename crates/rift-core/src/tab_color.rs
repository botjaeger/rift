//! Tab colours: one of six named colours (drawn from the theme's bright
//! ANSI palette, so they follow the theme) or any hex colour, set by hand,
//! by a profile or launch configuration, or by a `[[tab_color_rules]]`
//! directory rule.

use serde::{Deserialize, Serialize};

use crate::theme::{Color, Theme};

/// The named tab colours, in menu and cycle order.
pub const NAMES: [&str; 6] = ["red", "green", "yellow", "blue", "magenta", "cyan"];

#[derive(Clone, Copy, PartialEq)]
pub enum TabColor {
    /// Index into [`NAMES`].
    Named(usize),
    /// A fixed colour (`#rrggbb`), the same in every theme.
    Hex(Color),
}

/// Dumps (`script` `dump`) print a named colour as its index, as they did
/// before hex colours existed, and a hex colour as its hex.
impl std::fmt::Debug for TabColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TabColor::Named(i) => write!(f, "{i}"),
            TabColor::Hex(c) => write!(f, "{:?}", c.to_hex()),
        }
    }
}

impl TabColor {
    /// A colour name (`red` … `cyan`, any case) or `#rgb` / `#rrggbb`;
    /// anything else (Warp's `black` and `white`, a typo) is none.
    pub fn parse(s: &str) -> Option<TabColor> {
        let s = s.trim();
        if s.starts_with('#') {
            // a tab colour is opaque: `#rrggbbaa` is refused, not half-shown
            return (s.len() == 4 || s.len() == 7)
                .then(|| Color::hex(s).ok())
                .flatten()
                .map(TabColor::Hex);
        }
        NAMES
            .iter()
            .position(|n| n.eq_ignore_ascii_case(s))
            .map(TabColor::Named)
    }

    /// How config, session and launch files spell it.
    pub fn to_config(self) -> String {
        match self {
            TabColor::Named(i) => NAMES[i.min(NAMES.len() - 1)].to_string(),
            TabColor::Hex(c) => c.to_hex(),
        }
    }

    /// The colour drawn: a named one from the theme's bright ANSI palette
    /// (red … cyan are ANSI 9..=14), a hex one as given.
    pub fn resolve(self, theme: &Theme) -> Color {
        match self {
            TabColor::Named(i) => theme.ansi[9 + i.min(5)],
            TabColor::Hex(c) => c,
        }
    }

    /// The next colour of the cycle (`cycle_tab_color`): none, red … cyan,
    /// none again. A hex colour moves on to red.
    pub fn cycle(current: Option<TabColor>) -> Option<TabColor> {
        match current {
            None | Some(TabColor::Hex(_)) => Some(TabColor::Named(0)),
            Some(TabColor::Named(i)) if i + 1 < NAMES.len() => Some(TabColor::Named(i + 1)),
            Some(TabColor::Named(_)) => None,
        }
    }
}

/// `[[tab_color_rules]]`: tabs whose directory is `dir` or below it take
/// `color` unless the tab has a colour of its own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabColorRule {
    /// A directory; `~` is the home directory.
    pub dir: String,
    /// A colour name (`red` … `cyan`) or `#rrggbb`.
    pub color: String,
}

/// `rules` as matched: `~` expanded, and a directory that exists also by
/// its real path (`/tmp` is `/private/tmp` on macOS, a symlinked project
/// is where the shell reports it), so either spelling of the cwd matches.
/// Done once per config load: it reads the file system.
pub fn resolve_rules(rules: &[TabColorRule], home: Option<&str>) -> Vec<TabColorRule> {
    let mut out = Vec::new();
    for r in rules {
        let dir = r.dir.trim();
        let expanded = match (dir.strip_prefix('~'), home) {
            (Some(rest), Some(h)) if rest.is_empty() || rest.starts_with('/') => {
                format!("{}{rest}", h.trim_end_matches('/'))
            }
            _ => dir.to_string(),
        };
        if let Ok(real) = crate::sys::canonicalize(&expanded) {
            let real = real.to_string_lossy().into_owned();
            if real != expanded {
                out.push(TabColorRule {
                    dir: real,
                    color: r.color.clone(),
                });
            }
        }
        out.push(TabColorRule {
            dir: expanded,
            color: r.color.clone(),
        });
    }
    out
}

/// The rule colour for a tab in `cwd`: the rule with the deepest matching
/// `dir` wins (`~/work/prod` over `~/work`). `home` expands `~`.
pub fn for_dir(rules: &[TabColorRule], cwd: &str, home: Option<&str>) -> Option<TabColor> {
    let cwd = cwd.trim_end_matches('/');
    rules
        .iter()
        .filter_map(|r| {
            let dir = r.dir.trim();
            let dir = match (dir.strip_prefix('~'), home) {
                (Some(rest), Some(h)) if rest.is_empty() || rest.starts_with('/') => {
                    format!("{}{rest}", h.trim_end_matches('/'))
                }
                (Some(_), _) => return None,
                (None, _) => dir.to_string(),
            };
            let dir = dir.trim_end_matches('/');
            if dir.is_empty() && !r.dir.trim().starts_with('/') {
                return None;
            }
            let inside = cwd == dir
                || cwd
                    .strip_prefix(dir)
                    .is_some_and(|rest| rest.starts_with('/'));
            inside
                .then(|| TabColor::parse(&r.color))
                .flatten()
                .map(|c| (dir.len(), c))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_and_hex_only() {
        assert_eq!(TabColor::parse("Blue"), Some(TabColor::Named(3)));
        assert_eq!(
            TabColor::parse(" #ff8800 "),
            Some(TabColor::Hex(Color::hex("#ff8800").unwrap()))
        );
        assert_eq!(
            TabColor::parse("#f80"),
            Some(TabColor::Hex(Color::hex("#ff8800").unwrap()))
        );
        for bad in ["black", "white", "#ff880080", "#zzzzzz", "", "ff8800"] {
            assert_eq!(TabColor::parse(bad), None, "{bad:?}");
        }
        assert_eq!(TabColor::Named(4).to_config(), "magenta");
        assert_eq!(TabColor::parse("#FF8800").unwrap().to_config(), "#ff8800");
        assert_eq!(format!("{:?}", Some(TabColor::Named(4))), "Some(4)");
        assert_eq!(
            format!("{:?}", TabColor::parse("#ff8800")),
            "Some(\"#ff8800\")"
        );
    }

    #[test]
    fn resolves_named_from_the_theme_and_hex_as_given() {
        let t = Theme::default();
        assert_eq!(TabColor::Named(0).resolve(&t), t.ansi[9]);
        assert_eq!(TabColor::Named(5).resolve(&t), t.ansi[14]);
        let c = Color::hex("#123456").unwrap();
        assert_eq!(TabColor::Hex(c).resolve(&t), c);
    }

    #[test]
    fn cycle_walks_every_name_then_none() {
        let mut seen = Vec::new();
        let mut c = TabColor::cycle(None);
        while let Some(x) = c {
            seen.push(x);
            c = TabColor::cycle(c);
        }
        assert_eq!(seen.len(), NAMES.len());
        assert_eq!(seen[0], TabColor::Named(0));
        assert_eq!(
            TabColor::cycle(Some(TabColor::Hex(Color::BLACK))),
            Some(TabColor::Named(0))
        );
    }

    #[test]
    #[cfg(unix)]
    fn resolved_rules_match_the_real_path_too() {
        let base = std::env::temp_dir().join(format!("rift-tabrule-{}", std::process::id()));
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = base.join("link");
        let _ = std::fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let rules = vec![TabColorRule {
            dir: link.to_string_lossy().into_owned(),
            color: "cyan".into(),
        }];
        let cwd = crate::sys::canonicalize(&real).unwrap();
        let cwd = cwd.to_string_lossy();
        assert_eq!(
            for_dir(&rules, &cwd, None),
            None,
            "unresolved: the link only"
        );
        let resolved = resolve_rules(&rules, None);
        assert_eq!(for_dir(&resolved, &cwd, None), Some(TabColor::Named(5)));
        assert_eq!(
            for_dir(&resolved, &link.to_string_lossy(), None),
            Some(TabColor::Named(5))
        );
        // `~` expands; a missing directory keeps its spelling
        let home_rules = vec![TabColorRule {
            dir: "~/nowhere".into(),
            color: "red".into(),
        }];
        assert_eq!(resolve_rules(&home_rules, Some("/h"))[0].dir, "/h/nowhere");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn deepest_matching_dir_wins() {
        let rules = vec![
            TabColorRule {
                dir: "~/work".into(),
                color: "blue".into(),
            },
            TabColorRule {
                dir: "~/work/prod/".into(),
                color: "#ff0000".into(),
            },
            TabColorRule {
                dir: "/srv".into(),
                color: "green".into(),
            },
        ];
        let home = Some("/home/me");
        assert_eq!(
            for_dir(&rules, "/home/me/work/app", home),
            Some(TabColor::Named(3))
        );
        assert_eq!(
            for_dir(&rules, "/home/me/work/prod/api", home),
            TabColor::parse("#ff0000")
        );
        assert_eq!(
            for_dir(&rules, "/home/me/work", home),
            Some(TabColor::Named(3))
        );
        // a sibling that only shares the prefix is not inside
        assert_eq!(for_dir(&rules, "/home/me/workshop", home), None);
        assert_eq!(for_dir(&rules, "/srv/www", home), Some(TabColor::Named(1)));
        // `~` with no home matches nothing; a bad colour matches nothing
        assert_eq!(for_dir(&rules, "/home/me/work", None), None);
        let bad = vec![TabColorRule {
            dir: "/srv".into(),
            color: "black".into(),
        }];
        assert_eq!(for_dir(&bad, "/srv", home), None);
        // an empty dir is no rule (it would match everything)
        let empty = vec![TabColorRule {
            dir: " ".into(),
            color: "red".into(),
        }];
        assert_eq!(for_dir(&empty, "/srv", home), None);
    }
}
