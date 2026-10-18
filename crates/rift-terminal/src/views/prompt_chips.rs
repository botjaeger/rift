//! The Rift prompt's context chips (Warp "Edit prompt"): what each chip in
//! `prompt_chips` shows for a pane, or nothing when it has nothing to show
//! (no repository, no virtualenv). Pure apart from the clock and the host
//! name, which the caller passes in.

use std::collections::HashMap;

use rift_core::docker::{ComposeInfo, Health};
use rift_core::git::GitInfo;
use rift_core::github::{PrInfo, PrState};
use rift_core::{Color, PromptChip, Theme};
use rift_remote::protocol::Tone;

use crate::views::glyphs;

/// One chip as drawn: icon, its color, and colored text runs. The same
/// values, as theme-independent tones plus a shortened text, are what a
/// paired phone is sent ([`ChipView::wire`]).
#[derive(Debug, Clone, PartialEq)]
pub struct ChipView {
    pub chip: PromptChip,
    /// Its Nerd Font mark (`glyphs`).
    pub glyph: char,
    pub icon_color: Color,
    pub texts: Vec<(String, Color)>,
    /// `icon_color` and `texts` as the theme's colour roles.
    pub icon_tone: Tone,
    pub parts: Vec<(String, Tone)>,
    /// The phone's narrow row shows this instead of `parts`.
    pub short: Vec<(String, Tone)>,
    /// A container down or unhealthy: the phone marks the chip when it
    /// folds it away.
    pub attention: bool,
}

impl ChipView {
    /// The chip for a paired phone's topology.
    pub fn wire(&self) -> rift_remote::protocol::PaneChip {
        use rift_remote::protocol::{ChipKind, Part};
        let parts = |list: &[(String, Tone)]| {
            list.iter()
                .map(|(text, tone)| Part {
                    text: text.clone(),
                    tone: *tone,
                })
                .collect()
        };
        rift_remote::protocol::PaneChip {
            kind: match self.chip {
                PromptChip::Cwd => ChipKind::Cwd,
                PromptChip::GitBranch => ChipKind::GitBranch,
                PromptChip::GitDiff => ChipKind::GitDiff,
                PromptChip::User => ChipKind::User,
                PromptChip::Host => ChipKind::Host,
                PromptChip::Python => ChipKind::Python,
                PromptChip::Php => ChipKind::Php,
                PromptChip::GithubPr => ChipKind::GithubPr,
                PromptChip::Docker => ChipKind::Docker,
                PromptChip::Date => ChipKind::Date,
                PromptChip::Time12 => ChipKind::Time12,
                PromptChip::Time24 => ChipKind::Time24,
                PromptChip::Elapsed => ChipKind::Elapsed,
            },
            icon: self.icon_tone,
            full: parts(&self.parts),
            short: parts(&self.short),
            attention: self.attention,
        }
    }
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

/// The Elapsed card's line (desktop hover card and phone): when the clock
/// started in `Always` mode, else what a command-running count says. None
/// until there is something to show.
pub fn elapsed_line(
    mode: rift_core::ElapsedCounts,
    started: Option<std::time::SystemTime>,
    shown: Option<Elapsed>,
) -> Option<String> {
    match (mode, started, shown) {
        (rift_core::ElapsedCounts::Always, Some(at), Some(e)) => {
            Some(started_text(LocalTime::at(at), e.time))
        }
        (rift_core::ElapsedCounts::Running, _, Some(e)) => Some(format!(
            "Counted {} \u{b7} {}",
            elapsed_text(e.time),
            if e.counting { "running now" } else { "paused" }
        )),
        _ => None,
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

/// A tone's colour in `theme`.
pub fn tone_color(theme: &Theme, tone: Tone) -> Color {
    match tone {
        Tone::Bright => theme.text_bright,
        Tone::Ui => theme.text_ui,
        Tone::Dim => theme.text_dim,
        Tone::Muted => theme.text_muted,
        Tone::Success => theme.success,
        Tone::Error => theme.error,
        Tone::Warning => theme.warning,
        Tone::Accent => theme.accent,
        Tone::Magenta => theme.ansi[5],
        // PHP's indigo, from the theme's blue and magenta
        Tone::Indigo => theme.accent.mix(theme.ansi[5], 0.5),
    }
}

/// `n` for a short chip: `970`, `2.1k`, `12k`.
fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1000..=9999 => format!("{:.1}k", n as f64 / 1000.0),
        _ => format!("{}k", n / 1000),
    }
}

/// `3.12.4` → `3.12` for a short chip; anything else (a virtualenv's name)
/// stays.
fn short_version(v: &str) -> String {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() > 2
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        parts[..2].join(".")
    } else {
        v.to_string()
    }
}

/// The last component of a (home-shortened) folder: `~/xxx/flow` → `flow`.
fn folder_name(cwd: &str) -> &str {
    cwd.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or(cwd)
}

/// A compose project that needs a look: a container down or unhealthy.
fn docker_attention(d: &ComposeInfo) -> bool {
    d.error.is_none()
        && d.containers
            .iter()
            .any(|c| !c.running() || c.health == Some(Health::Unhealthy))
}

/// The chips `chips` lists that have something to show, in order.
pub fn views(chips: &[PromptChip], theme: &Theme, i: &Inputs) -> Vec<ChipView> {
    chips
        .iter()
        .filter_map(|&chip| {
            // `parts` is what the desktop draws; `short` (the phone's narrow
            // row) defaults to the same
            let build = |glyph, icon: Tone, parts: Vec<(String, Tone)>| ChipView {
                chip,
                glyph,
                icon_color: tone_color(theme, icon),
                texts: parts
                    .iter()
                    .map(|(t, tone)| (t.clone(), tone_color(theme, *tone)))
                    .collect(),
                icon_tone: icon,
                short: parts.clone(),
                parts,
                attention: false,
            };
            let one = |glyph, icon, text: String, tone| {
                (!text.is_empty()).then(|| build(glyph, icon, vec![(text, tone)]))
            };
            let var = |name: &str| i.vars.get(name).cloned().unwrap_or_default();
            match chip {
                PromptChip::Cwd => {
                    let mut v = build(
                        glyphs::FOLDER,
                        Tone::Dim,
                        vec![(i.cwd.to_string(), Tone::Bright)],
                    );
                    v.short = vec![(folder_name(i.cwd).to_string(), Tone::Bright)];
                    Some(v)
                }
                PromptChip::GitBranch => {
                    let g = i.git?;
                    let mut parts = vec![(g.branch.clone(), Tone::Ui)];
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
                        parts.push((marks.join(" "), Tone::Muted));
                    }
                    let mut v = build(glyphs::BRANCH, Tone::Success, parts);
                    // the arrows go; the dirty dot stays
                    v.short = vec![(g.branch.clone(), Tone::Ui)];
                    if g.dirty {
                        v.short.push(("•".into(), Tone::Muted));
                    }
                    Some(v)
                }
                PromptChip::GitDiff => {
                    let g = i.git.filter(|g| g.files > 0)?;
                    let mut v = build(
                        glyphs::DIFF,
                        Tone::Dim,
                        vec![
                            (g.files.to_string(), Tone::Ui),
                            ("•".into(), Tone::Muted),
                            (format!("+{}", g.added), Tone::Success),
                            (format!("−{}", g.removed), Tone::Error),
                        ],
                    );
                    v.short = vec![
                        (g.files.to_string(), Tone::Ui),
                        (format!("+{}", compact(g.added as u64)), Tone::Success),
                        (format!("−{}", compact(g.removed as u64)), Tone::Error),
                    ];
                    Some(v)
                }
                PromptChip::User => {
                    one(glyphs::USER, Tone::Accent, i.user.to_string(), Tone::Bright)
                }
                PromptChip::Host => one(
                    glyphs::LAPTOP,
                    Tone::Accent,
                    i.host.to_string(),
                    Tone::Bright,
                ),
                PromptChip::Python => {
                    let text = Some(var("RiftPython"))
                        .filter(|v| !v.is_empty())
                        .or_else(|| i.python.clone())
                        .unwrap_or_default();
                    let mut v = one(glyphs::PYTHON, Tone::Warning, text.clone(), Tone::Bright)?;
                    v.short = vec![(short_version(&text), Tone::Bright)];
                    Some(v)
                }
                PromptChip::Php => {
                    let text = i.php.clone().unwrap_or_default();
                    let mut v = one(glyphs::PHP, Tone::Indigo, text.clone(), Tone::Bright)?;
                    v.short = vec![(short_version(&text), Tone::Bright)];
                    Some(v)
                }
                PromptChip::GithubPr => {
                    let pr = i.pr?;
                    // the GitHub mark and `PR #n` in one colour: the open
                    // PR's yellow, else its state's
                    let (tone, state) = match pr.state {
                        PrState::Open => (Tone::Warning, None),
                        PrState::Draft => (Tone::Dim, Some("draft")),
                        PrState::Merged => (Tone::Magenta, Some("merged")),
                        PrState::Closed => (Tone::Error, Some("closed")),
                    };
                    let mut parts = vec![(format!("PR #{}", pr.number), tone)];
                    if let Some(s) = state {
                        parts.push((s.into(), Tone::Muted));
                    }
                    let mut v = build(glyphs::GITHUB, tone, parts);
                    v.short = vec![(format!("#{}", pr.number), tone)];
                    Some(v)
                }
                PromptChip::Docker => {
                    let d = i.docker?;
                    let (count, tone) = match (d.error.is_some(), d.running()) {
                        (true, _) => ("off".to_string(), Tone::Muted),
                        (false, 0) => ("down".to_string(), Tone::Muted),
                        (false, n) => (format!("{n}/{}", d.containers.len()), Tone::Success),
                    };
                    let mut v = build(
                        glyphs::DOCKER,
                        Tone::Accent,
                        vec![(d.project.clone(), Tone::Bright), (count.clone(), tone)],
                    );
                    v.short = vec![(count, tone)];
                    v.attention = docker_attention(d);
                    Some(v)
                }
                PromptChip::Date => one(glyphs::CALENDAR, Tone::Dim, i.now.date(), Tone::Ui),
                PromptChip::Time12 => one(glyphs::CLOCK, Tone::Dim, i.now.time12(), Tone::Ui),
                PromptChip::Time24 => one(glyphs::CLOCK, Tone::Dim, i.now.time24(), Tone::Ui),
                PromptChip::Elapsed => {
                    let e = i.elapsed?;
                    one(
                        glyphs::HOURGLASS,
                        if e.counting { Tone::Dim } else { Tone::Muted },
                        elapsed_text(e.time),
                        Tone::Ui,
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
        worktree: None,
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
            worktree: None,
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

    /// What a pane with Docker, Python, a PR, a dirty tree and a running
    /// tab clock sends a phone: the configured order, full and short values
    /// as tones, and the attention flag of a container that is down.
    #[test]
    fn a_phone_gets_the_chips_the_desktop_draws() {
        use rift_core::docker::Container;
        let theme = Theme::default();
        let git = GitInfo {
            branch: "release/3.16.0".into(),
            ahead: 2,
            behind: 0,
            dirty: true,
            files: 102,
            added: 2086,
            removed: 970,
            worktree: None,
        };
        let pr = PrInfo {
            number: 42,
            url: "https://github.com/me/flow/pull/42".into(),
            state: PrState::Open,
        };
        let web = Container {
            service: "api".into(),
            name: "flow-api-1".into(),
            state: "running".into(),
            status: "Up 2 hours".into(),
            image: "nginx".into(),
            ports: Vec::new(),
            health: None,
        };
        let worker = Container {
            service: "worker".into(),
            state: "exited".into(),
            status: "Exited (1) 3 minutes ago".into(),
            ..web.clone()
        };
        let compose = |containers: Vec<Container>| ComposeInfo {
            project: "flow".into(),
            dir: "/Users/me/flow".into(),
            containers,
            error: None,
        };
        let down = compose(vec![web.clone(), worker]);
        let vars: HashMap<String, String> = HashMap::new();
        // the pane's configured order, not the palette's
        let order = [
            PromptChip::Cwd,
            PromptChip::GitBranch,
            PromptChip::GitDiff,
            PromptChip::GithubPr,
            PromptChip::Python,
            PromptChip::Docker,
            PromptChip::Php,
            PromptChip::Elapsed,
        ];
        let wire = |c: &ComposeInfo| -> Vec<rift_remote::protocol::PaneChip> {
            let inputs = Inputs {
                cwd: "~/xxx/flow",
                git: Some(&git),
                user: "me",
                host: "mac",
                vars: &vars,
                python: Some("3.12.4".into()),
                php: None,
                pr: Some(&pr),
                docker: Some(c),
                now: LocalTime::default(),
                elapsed: Some(Elapsed {
                    time: std::time::Duration::from_secs(6 * 3600 + 33 * 60),
                    counting: true,
                }),
            };
            views(&order, &theme, &inputs)
                .iter()
                .map(ChipView::wire)
                .collect()
        };
        let chips = wire(&down);
        // no PHP: it is left out, the rest keep their order
        let kinds: Vec<_> = chips
            .iter()
            .map(|c| serde_json::to_value(c.kind).unwrap())
            .collect();
        assert_eq!(
            kinds,
            [
                "cwd",
                "git_branch",
                "git_diff",
                "github_pr",
                "python",
                "docker",
                "elapsed"
            ]
        );
        let json = serde_json::to_value(&chips).unwrap();
        // short values: the folder's name, branch with its dot, compact
        // lines, PR number, a version without its patch, the count only
        let short = |i: usize| -> String {
            json[i]["short"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["text"].as_str().unwrap())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let shorts: Vec<String> = (0..7).map(short).collect();
        assert_eq!(
            shorts,
            [
                "flow",
                "release/3.16.0 •",
                "102 +2.1k −970",
                "#42",
                "3.12",
                "1/2",
                "6h 33m"
            ]
        );
        // full values are what the desktop draws
        assert_eq!(
            json[0]["full"][0],
            serde_json::json!({"text":"~/xxx/flow","tone":"bright"})
        );
        assert_eq!(
            json[1]["full"][1],
            serde_json::json!({"text":"↑2 •","tone":"muted"})
        );
        assert_eq!(json[4]["full"][0]["text"], "3.12.4");
        assert_eq!(
            json[5]["full"][1],
            serde_json::json!({"text":"1/2","tone":"success"})
        );
        assert_eq!(json[3]["icon"], "warning");
        // a container down needs a look; everything running does not
        let attention: Vec<bool> = chips.iter().map(|c| c.attention).collect();
        assert_eq!(attention, [false, false, false, false, false, true, false]);
        assert!(!wire(&compose(vec![web.clone()]))[5].attention);
        // an unhealthy one needs a look too
        let mut sick = web.clone();
        sick.health = Some(Health::Unhealthy);
        assert!(wire(&compose(vec![sick]))[5].attention);
        // Docker not running: "off", muted, no attention (nothing to look at)
        let mut off = compose(Vec::new());
        off.error = Some("Docker is not running".into());
        assert!(!wire(&off)[5].attention);
        // the flag is left out of the JSON unless set
        assert!(json[0].get("attention").is_none());
        assert_eq!(json[5]["attention"], true);
    }

    #[test]
    fn short_values_keep_what_is_not_a_version() {
        assert_eq!(short_version("3.12.4"), "3.12");
        assert_eq!(short_version("8.3"), "8.3");
        assert_eq!(short_version("my-venv"), "my-venv");
        assert_eq!(short_version("3.12.4rc1"), "3.12.4rc1");
        assert_eq!(compact(970), "970");
        assert_eq!(compact(2086), "2.1k");
        assert_eq!(compact(12400), "12k");
        assert_eq!(folder_name("~/xxx/flow"), "flow");
        assert_eq!(folder_name("~"), "~");
        assert_eq!(folder_name("/"), "/");
        assert_eq!(folder_name("/usr/"), "usr");
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
