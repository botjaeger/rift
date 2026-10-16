//! The Rift prompt's context chips (Warp "Edit prompt"): what each chip in
//! `prompt_chips` shows for a pane, or nothing when it has nothing to show
//! (no repository, no virtualenv). Pure apart from the clock and the host
//! name, which the caller passes in.

use std::collections::HashMap;

use rift_core::docker::ComposeInfo;
use rift_core::git::GitInfo;
use rift_core::github::{PrInfo, PrState};
use rift_core::{Color, PromptChip, Theme};

use crate::views::glyphs;

/// One chip as drawn: icon, its color, and colored text runs.
#[derive(Debug, Clone, PartialEq)]
pub struct ChipView {
    pub chip: PromptChip,
    /// Its Nerd Font mark (`glyphs`).
    pub glyph: char,
    pub icon_color: Color,
    pub texts: Vec<(String, Color)>,
}

/// Local wall-clock time, minute resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalTime {
    pub year: i32,
    /// 1–12
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

impl LocalTime {
    #[cfg(unix)]
    pub fn now() -> Self {
        Self::at(std::time::SystemTime::now())
    }

    /// The local time of `when`.
    #[cfg(unix)]
    pub fn at(when: std::time::SystemTime) -> Self {
        let t = when
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()) as libc::time_t;
        // SAFETY: localtime_r fills the zeroed tm from a valid time_t
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&t, &mut tm).is_null() {
                return Self::default();
            }
            LocalTime {
                year: tm.tm_year + 1900,
                month: tm.tm_mon as u32 + 1,
                day: tm.tm_mday as u32,
                hour: tm.tm_hour as u32,
                minute: tm.tm_min as u32,
            }
        }
    }

    /// `October 11, 2026`
    pub fn date(&self) -> String {
        const MONTHS: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        let m = MONTHS[(self.month.clamp(1, 12) - 1) as usize];
        format!("{m} {}, {}", self.day, self.year)
    }

    /// `03:48 pm`
    pub fn time12(&self) -> String {
        let h = match self.hour % 12 {
            0 => 12,
            h => h,
        };
        let ampm = if self.hour < 12 { "am" } else { "pm" };
        format!("{h:02}:{:02} {ampm}", self.minute)
    }

    /// `15:48`
    pub fn time24(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }
}

/// The login name and the host name (without `.local`), read once.
pub fn user_and_host() -> &'static (String, String) {
    static ONCE: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_default();
        let mut buf = [0u8; 256];
        // SAFETY: gethostname writes at most buf.len() bytes into buf
        let host = if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..end]).into_owned()
        } else {
            String::new()
        };
        let host = host.strip_suffix(".local").unwrap_or(&host).to_string();
        (user, host)
    })
}

/// Everything the chips read for one pane.
pub struct Inputs<'a> {
    pub cwd: &'a str,
    pub git: Option<&'a GitInfo>,
    pub user: &'a str,
    pub host: &'a str,
    /// Shell variables (OSC 1337 SetUserVar).
    pub vars: &'a HashMap<String, String>,
    /// The Python version `python3` runs, when no virtualenv or
    /// `PYENV_VERSION` is reported.
    pub python: Option<String>,
    /// The PHP version `php` runs.
    pub php: Option<String>,
    /// The checked-out branch's pull request.
    pub pr: Option<&'a PrInfo>,
    /// The Docker Compose project around the cwd.
    pub docker: Option<&'a ComposeInfo>,
    pub now: LocalTime,
    /// The tab's Elapsed chip (None: nothing to show yet).
    pub elapsed: Option<Elapsed>,
}

/// What the Elapsed chip shows: the time, and whether it is counting now
/// (the hourglass is lit) or paused (dimmed).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Elapsed {
    pub time: std::time::Duration,
    pub counting: bool,
}

/// `2h 05m`, `12m`, `0m` (minute resolution, like the clock chips).
pub fn elapsed_text(d: std::time::Duration) -> String {
    let m = d.as_secs() / 60;
    match m / 60 {
        0 => format!("{m}m"),
        h => format!("{h}h {:02}m", m % 60),
    }
}

/// The Elapsed chip's hover line: `Started 03:48 pm`, with the date once
/// the clock has run 24 hours (`Started October 13, 2026, 03:48 pm`).
pub fn started_text(started: LocalTime, elapsed: std::time::Duration) -> String {
    if elapsed.as_secs() >= 24 * 3600 {
        format!("Started {}, {}", started.date(), started.time12())
    } else {
        format!("Started {}", started.time12())
    }
}

/// The chips `chips` lists that have something to show, in order.
pub fn views(chips: &[PromptChip], theme: &Theme, i: &Inputs) -> Vec<ChipView> {
    chips
        .iter()
        .filter_map(|&chip| {
            let one = |glyph, icon_color, text: String, color| {
                (!text.is_empty()).then(|| ChipView {
                    chip,
                    glyph,
                    icon_color,
                    texts: vec![(text, color)],
                })
            };
            let var = |name: &str| i.vars.get(name).cloned().unwrap_or_default();
            match chip {
                PromptChip::Cwd => Some(ChipView {
                    chip,
                    glyph: glyphs::FOLDER,
                    icon_color: theme.text_dim,
                    texts: vec![(i.cwd.to_string(), theme.text_bright)],
                }),
                PromptChip::GitBranch => {
                    let g = i.git?;
                    let mut texts = vec![(g.branch.clone(), theme.text_ui)];
                    let mut marks = Vec::new();
                    if g.ahead > 0 {
                        marks.push(format!("↑{}", g.ahead));
                    }
                    if g.behind > 0 {
                        marks.push(format!("↓{}", g.behind));
                    }
                    if g.dirty {
                        marks.push("•".into());
                    }
                    if !marks.is_empty() {
                        texts.push((marks.join(" "), theme.text_muted));
                    }
                    Some(ChipView {
                        chip,
                        glyph: glyphs::BRANCH,
                        icon_color: theme.success,
                        texts,
                    })
                }
                PromptChip::GitDiff => {
                    let g = i.git.filter(|g| g.files > 0)?;
                    Some(ChipView {
                        chip,
                        glyph: glyphs::DIFF,
                        icon_color: theme.text_dim,
                        texts: vec![
                            (g.files.to_string(), theme.text_ui),
                            ("•".into(), theme.text_muted),
                            (format!("+{}", g.added), theme.success),
                            (format!("−{}", g.removed), theme.error),
                        ],
                    })
                }
                PromptChip::User => one(
                    glyphs::USER,
                    theme.accent,
                    i.user.to_string(),
                    theme.text_bright,
                ),
                PromptChip::Host => one(
                    glyphs::LAPTOP,
                    theme.accent,
                    i.host.to_string(),
                    theme.text_bright,
                ),
                PromptChip::Python => one(
                    glyphs::PYTHON,
                    theme.warning,
                    Some(var("RiftPython"))
                        .filter(|v| !v.is_empty())
                        .or_else(|| i.python.clone())
                        .unwrap_or_default(),
                    theme.text_bright,
                ),
                PromptChip::Php => one(
                    glyphs::PHP,
                    // PHP's indigo, from the theme's blue and magenta
                    theme.accent.mix(theme.ansi[5], 0.5),
                    i.php.clone().unwrap_or_default(),
                    theme.text_bright,
                ),
                PromptChip::GithubPr => {
                    let pr = i.pr?;
                    // the GitHub mark and `PR #n` in one colour: the open
                    // PR's yellow, else its state's
                    let (color, state) = match pr.state {
                        PrState::Open => (theme.warning, None),
                        PrState::Draft => (theme.text_dim, Some("draft")),
                        PrState::Merged => (theme.ansi[5], Some("merged")),
                        PrState::Closed => (theme.error, Some("closed")),
                    };
                    let mut texts = vec![(format!("PR #{}", pr.number), color)];
                    if let Some(s) = state {
                        texts.push((s.into(), theme.text_muted));
                    }
                    Some(ChipView {
                        chip,
                        glyph: glyphs::GITHUB,
                        icon_color: color,
                        texts,
                    })
                }
                PromptChip::Docker => {
                    let d = i.docker?;
                    let (count, color) = match (d.error.is_some(), d.running()) {
                        (true, _) => ("off".to_string(), theme.text_muted),
                        (false, 0) => ("down".to_string(), theme.text_muted),
                        (false, n) => (format!("{n}/{}", d.containers.len()), theme.success),
                    };
                    Some(ChipView {
                        chip,
                        glyph: glyphs::DOCKER,
                        icon_color: theme.accent,
                        texts: vec![(d.project.clone(), theme.text_bright), (count, color)],
                    })
                }
                PromptChip::Date => one(
                    glyphs::CALENDAR,
                    theme.text_dim,
                    i.now.date(),
                    theme.text_ui,
                ),
                PromptChip::Time12 => {
                    one(glyphs::CLOCK, theme.text_dim, i.now.time12(), theme.text_ui)
                }
                PromptChip::Time24 => {
                    one(glyphs::CLOCK, theme.text_dim, i.now.time24(), theme.text_ui)
                }
                PromptChip::Elapsed => {
                    let e = i.elapsed?;
                    one(
                        glyphs::HOURGLASS,
                        if e.counting {
                            theme.text_dim
                        } else {
                            theme.text_muted
                        },
                        elapsed_text(e.time),
                        theme.text_ui,
                    )
                }
            }
        })
        .collect()
}

/// A chip with sample values, for the prompt editor's palette (Warp shows
/// what each chip looks like even where it would be hidden now).
pub fn sample(chip: PromptChip, theme: &Theme) -> ChipView {
    let git = GitInfo {
        branch: "main".into(),
        ahead: 1,
        behind: 0,
        dirty: true,
        files: 3,
        added: 10,
        removed: 2,
    };
    let vars: HashMap<String, String> = [("RiftPython".to_string(), "3.12.2".to_string())].into();
    let pr = PrInfo {
        number: 42,
        url: String::new(),
        state: PrState::Open,
    };
    let docker = ComposeInfo {
        project: "project".into(),
        dir: Default::default(),
        containers: vec![rift_core::docker::Container {
            service: "web".into(),
            name: "project-web-1".into(),
            state: "running".into(),
            status: "Up".into(),
            image: "nginx".into(),
            ports: Vec::new(),
            health: None,
        }],
        error: None,
    };
    let inputs = Inputs {
        cwd: "~/project",
        git: Some(&git),
        user: "user",
        host: "host",
        vars: &vars,
        python: None,
        php: Some("8.3.12".into()),
        pr: Some(&pr),
        docker: Some(&docker),
        now: LocalTime {
            year: 2026,
            month: 10,
            day: 11,
            hour: 15,
            minute: 48,
        },
        elapsed: Some(Elapsed {
            time: std::time::Duration::from_secs(2 * 3600 + 5 * 60),
            counting: true,
        }),
    };
    let mut v = views(&[chip], theme, &inputs)
        .pop()
        .expect("every chip shows with the sample inputs");
    // tool chips go by the tool's name: a sample version or project says
    // less than what the chip is
    let name = match chip {
        PromptChip::Docker => Some("docker"),
        PromptChip::Python => Some("python"),
        PromptChip::Php => Some("php"),
        _ => None,
    };
    if let Some(name) = name {
        v.texts = vec![(name.into(), theme.text_bright)];
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(v: &ChipView) -> String {
        v.texts
            .iter()
            .map(|(t, _)| t.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn chips_show_only_what_they_have() {
        let theme = Theme::default();
        let vars: HashMap<String, String> =
            [("RiftPython".to_string(), "my-venv".to_string())].into();
        let clean = GitInfo {
            branch: "main".into(),
            ahead: 2,
            behind: 0,
            dirty: false,
            files: 0,
            added: 0,
            removed: 0,
        };
        let now = LocalTime {
            year: 2023,
            month: 7,
            day: 12,
            hour: 0,
            minute: 5,
        };
        let i = Inputs {
            cwd: "~/zzz",
            git: Some(&clean),
            user: "alice",
            host: "",
            vars: &vars,
            python: Some("3.12.2".into()),
            php: None,
            pr: None,
            docker: None,
            now,
            elapsed: None,
        };
        let shown: Vec<String> = views(&PromptChip::ALL, &theme, &i)
            .iter()
            .map(texts)
            .collect();
        // no host name, clean tree, no PR, PHP, compose project or command
        // run yet: those drop out; a virtualenv wins over the version
        assert_eq!(
            shown,
            [
                "alice",
                "~/zzz",
                "main ↑2",
                "my-venv",
                "July 12, 2023",
                "12:05 am",
                "00:05"
            ]
        );
        // without a virtualenv, the version python3 runs
        let none = HashMap::new();
        let plain = Inputs {
            vars: &none,
            python: Some("3.12.2".into()),
            php: Some("8.3.12".into()),
            pr: None,
            docker: None,
            ..i
        };
        let v = views(&[PromptChip::Python, PromptChip::Php], &theme, &plain);
        assert_eq!(
            v.iter().map(texts).collect::<Vec<_>>(),
            ["3.12.2", "8.3.12"]
        );
        // a merged PR says so; a compose project counts its running containers
        let merged = PrInfo {
            number: 7,
            url: String::new(),
            state: PrState::Merged,
        };
        let mut compose = ComposeInfo {
            project: "app".into(),
            dir: Default::default(),
            containers: Vec::new(),
            error: None,
        };
        let docker_text = |c: &ComposeInfo, pr: Option<&PrInfo>| {
            let i = Inputs {
                vars: &none,
                python: None,
                php: None,
                pr,
                docker: Some(c),
                ..i
            };
            views(&[PromptChip::GithubPr, PromptChip::Docker], &theme, &i)
                .iter()
                .map(texts)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            docker_text(&compose, Some(&merged)),
            ["PR #7 merged", "app down"]
        );
        let web = rift_core::docker::Container {
            service: "web".into(),
            name: "app-web-1".into(),
            state: "running".into(),
            status: "Up".into(),
            image: "nginx".into(),
            ports: Vec::new(),
            health: None,
        };
        let db = rift_core::docker::Container {
            service: "db".into(),
            state: "exited".into(),
            ..web.clone()
        };
        compose.containers = vec![web, db];
        assert_eq!(docker_text(&compose, None), ["app 1/2"]);
        compose.error = Some("Docker is not running".into());
        assert_eq!(docker_text(&compose, None), ["app off"]);
        // outside a repository both git chips go
        let i = Inputs { git: None, ..i };
        let v = views(&[PromptChip::GitBranch, PromptChip::GitDiff], &theme, &i);
        assert!(v.is_empty());
        // every chip has a sample for the editor's palette
        for chip in PromptChip::ALL {
            assert_eq!(sample(chip, &theme).chip, chip);
        }
        assert_eq!(texts(&sample(PromptChip::GitDiff, &theme)), "3 • +10 −2");
    }

    #[test]
    fn times_format_like_warp() {
        let t = LocalTime {
            year: 2023,
            month: 7,
            day: 12,
            hour: 15,
            minute: 48,
        };
        assert_eq!(
            (t.time12(), t.time24()),
            ("03:48 pm".into(), "15:48".into())
        );
        let noon = LocalTime { hour: 12, ..t };
        assert_eq!(noon.time12(), "12:48 pm");
    }

    #[test]
    fn elapsed_counts_hours_and_minutes() {
        let s = std::time::Duration::from_secs;
        assert_eq!(elapsed_text(s(59)), "0m");
        assert_eq!(elapsed_text(s(12 * 60 + 30)), "12m");
        assert_eq!(elapsed_text(s(3600)), "1h 00m");
        assert_eq!(elapsed_text(s(26 * 3600 + 5 * 60)), "26h 05m");
        let theme = Theme::default();
        assert_eq!(texts(&sample(PromptChip::Elapsed, &theme)), "2h 05m");
    }

    #[test]
    fn started_names_the_date_after_a_day() {
        let s = std::time::Duration::from_secs;
        let at = LocalTime {
            year: 2026,
            month: 10,
            day: 13,
            hour: 15,
            minute: 48,
        };
        assert_eq!(started_text(at, s(16 * 3600)), "Started 03:48 pm");
        assert_eq!(started_text(at, s(24 * 3600 - 1)), "Started 03:48 pm");
        assert_eq!(
            started_text(at, s(24 * 3600)),
            "Started October 13, 2026, 03:48 pm"
        );
    }
}
