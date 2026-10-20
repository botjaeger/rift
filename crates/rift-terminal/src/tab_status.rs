//! What a tab says about itself in every tab style (`tab_style`): its
//! title, its one status and its ⌘-number hint.
//!
//! - **Title**: the name it was given (rename), else the folder of its
//!   focused pane (`~` for home). Two tabs on folders of the same name both
//!   show `parent/name`. The program's own title (OSC 0/2, an agent's
//!   session name) is the tooltip's, never the tab's: an agent's status
//!   glyph (`◑`) would repeat the status mark.
//! - **Status**: the most urgent state of any of its panes — needs you ›
//!   failed › working › done › new output — and whether the focused pane
//!   holds it (`own`; a tie goes to the focused pane). A viewed pane's
//!   done, failed and new output are seen; needs-you stays until answered.
//! - **Hint**: while ⌘ (the modifiers of `tab_select_1`) is held, the shown
//!   tabs 1–8 carry their `tab_select_N` chord and the last one
//!   `tab_select_9`'s.

use crate::agent_status::AgentState;

/// One state a tab (or a pane) reports, least urgent first (`Ord` is the
/// rank).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Output arrived since the tab was last viewed.
    Activity,
    /// A command or an agent finished, not yet viewed.
    Done,
    /// A command runs, or an agent works.
    Working,
    /// A command or an agent failed, not yet viewed.
    Failed,
    /// An agent (or a password prompt) waits on you.
    NeedsYou,
}

impl Status {
    /// Words for the detailed style's second line, tooltips and screen
    /// readers.
    pub fn words(self) -> &'static str {
        match self {
            Status::Activity => "New output",
            Status::Done => "Done",
            Status::Working => "Working",
            Status::Failed => "Failed",
            Status::NeedsYou => "Needs you",
        }
    }

    /// The `dump` value.
    pub fn key(self) -> &'static str {
        match self {
            Status::Activity => "activity",
            Status::Done => "done",
            Status::Working => "working",
            Status::Failed => "failed",
            Status::NeedsYou => "needs_you",
        }
    }
}

/// What one pane reports this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaneSignals {
    /// Its coding agent's state (`agent_status`).
    pub agent: Option<AgentState>,
    /// A command runs in it (an agent's own state wins over this).
    pub running: bool,
    /// Unread notifications of the pane: a request, a failure, a finish,
    /// anything else (a bell, an OSC notification).
    pub request: bool,
    pub failed: bool,
    pub finished: bool,
    pub notified: bool,
    /// Output since the tab was last viewed.
    pub activity: bool,
    /// The pane is on screen in the focused window.
    pub viewed: bool,
}

/// The most urgent thing pane `s` reports, if any.
pub fn pane_status(s: &PaneSignals) -> Option<Status> {
    let agent = |state| s.agent == Some(state);
    let mut found = Vec::new();
    if agent(AgentState::Waiting) || s.request {
        found.push(Status::NeedsYou);
    }
    if agent(AgentState::Error) || s.failed {
        found.push(Status::Failed);
    }
    if agent(AgentState::Working) || (s.agent.is_none() && s.running) {
        found.push(Status::Working);
    }
    if agent(AgentState::Done) || s.finished {
        found.push(Status::Done);
    }
    if s.activity || s.notified {
        found.push(Status::Activity);
    }
    found
        .into_iter()
        // looked at: what it finished, failed or printed is seen
        .filter(|st| !s.viewed || matches!(st, Status::NeedsYou | Status::Working))
        .max()
}

/// A tab's status: the state, and whether its focused pane holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabStatus {
    pub status: Status,
    pub own: bool,
    /// Index of the pane it comes from (the focused one on a tie).
    pub pane: usize,
}

/// The tab's status from its panes' (`focused` indexes the focused pane):
/// the most urgent, the focused pane winning a tie.
pub fn tab_status(panes: &[Option<Status>], focused: usize) -> Option<TabStatus> {
    let best = panes.iter().flatten().max().copied()?;
    let pane = if panes.get(focused).copied().flatten() == Some(best) {
        focused
    } else {
        panes.iter().position(|p| *p == Some(best))?
    };
    Some(TabStatus {
        status: best,
        own: pane == focused,
        pane,
    })
}

/// The last component of a `~`-shortened path (`~` itself for home).
pub fn folder(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/";
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// The last two components (`rift/src`), or the whole path when it has no
/// parent to add.
fn with_parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    let mut parts = trimmed.rsplitn(3, '/');
    let name = parts.next().unwrap_or(trimmed);
    match parts.next() {
        Some(parent) if !parent.is_empty() => format!("{parent}/{name}"),
        Some(_) => format!("/{name}"),
        None => name.to_string(),
    }
}

/// One tab's title inputs: the name it was given, else the focused pane's
/// `~`-shortened directory, else `fallback` (no directory known yet).
pub struct TitleInput<'a> {
    pub custom: Option<&'a str>,
    pub cwd: Option<&'a str>,
    pub fallback: &'a str,
}

/// The tabs' shown titles: a given name as is; a folder name that two
/// folder-titled tabs share shows `parent/name` on both.
pub fn titles(tabs: &[TitleInput]) -> Vec<String> {
    fn folder_of<'a>(t: &TitleInput<'a>) -> Option<&'a str> {
        t.custom.is_none().then_some(t.cwd).flatten()
    }
    tabs.iter()
        .map(|t| {
            if let Some(name) = t.custom {
                return name.to_string();
            }
            let Some(cwd) = t.cwd else {
                return t.fallback.to_string();
            };
            let name = folder(cwd);
            let shared = tabs
                .iter()
                .filter_map(folder_of)
                .filter(|other| folder(other) == name && *other != cwd)
                .count()
                > 0;
            if shared {
                with_parent(cwd)
            } else {
                name.to_string()
            }
        })
        .collect()
}

/// The part of `cwd` the shown `title` leaves out (`~/zzz` under `rift`):
/// the detailed style's idle second line. Empty for home itself.
pub fn rest_of_path(cwd: &str, title: &str) -> String {
    let cwd = cwd.trim_end_matches('/');
    match cwd.strip_suffix(title) {
        Some(rest) if cwd != title => rest.trim_end_matches('/').to_string(),
        _ => String::new(),
    }
}

/// Which `tab_select_N` names each of `shown` tabs: 1–8 by place, the last
/// one 9 (`nth_shown_tab`); tabs 9 … n−1 have none.
pub fn hint_numbers(shown: usize) -> Vec<Option<u8>> {
    (0..shown)
        .map(|k| {
            if k < 8 {
                Some(k as u8 + 1)
            } else if k + 1 == shown {
                Some(9)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Status::*;

    fn sig() -> PaneSignals {
        PaneSignals::default()
    }

    #[test]
    fn a_pane_reports_its_most_urgent_state() {
        assert_eq!(pane_status(&sig()), None);
        let all = PaneSignals {
            running: true,
            request: true,
            failed: true,
            finished: true,
            activity: true,
            ..sig()
        };
        assert_eq!(pane_status(&all), Some(NeedsYou));
        assert_eq!(
            pane_status(&PaneSignals {
                request: false,
                ..all
            }),
            Some(Failed)
        );
        assert_eq!(
            pane_status(&PaneSignals {
                running: true,
                finished: true,
                activity: true,
                ..sig()
            }),
            Some(Working)
        );
        assert_eq!(
            pane_status(&PaneSignals {
                finished: true,
                activity: true,
                ..sig()
            }),
            Some(Done)
        );
        assert_eq!(
            pane_status(&PaneSignals {
                notified: true,
                ..sig()
            }),
            Some(Activity)
        );
    }

    #[test]
    fn an_agent_state_maps_and_wins_over_its_running_block() {
        let agent = |state| PaneSignals {
            agent: Some(state),
            running: true,
            ..sig()
        };
        assert_eq!(pane_status(&agent(AgentState::Waiting)), Some(NeedsYou));
        assert_eq!(pane_status(&agent(AgentState::Error)), Some(Failed));
        assert_eq!(pane_status(&agent(AgentState::Working)), Some(Working));
        // at its own prompt (✳) the agent's block still runs: done, not working
        assert_eq!(pane_status(&agent(AgentState::Done)), Some(Done));
    }

    #[test]
    fn viewing_clears_done_failed_and_output_but_not_needs_you() {
        let viewed = |s: PaneSignals| pane_status(&PaneSignals { viewed: true, ..s });
        assert_eq!(
            viewed(PaneSignals {
                finished: true,
                ..sig()
            }),
            None
        );
        assert_eq!(
            viewed(PaneSignals {
                failed: true,
                ..sig()
            }),
            None
        );
        assert_eq!(
            viewed(PaneSignals {
                activity: true,
                ..sig()
            }),
            None
        );
        assert_eq!(
            viewed(PaneSignals {
                agent: Some(AgentState::Error),
                ..sig()
            }),
            None
        );
        assert_eq!(
            viewed(PaneSignals {
                request: true,
                ..sig()
            }),
            Some(NeedsYou)
        );
        assert_eq!(
            viewed(PaneSignals {
                agent: Some(AgentState::Waiting),
                failed: true,
                ..sig()
            }),
            Some(NeedsYou)
        );
        assert_eq!(
            viewed(PaneSignals {
                running: true,
                ..sig()
            }),
            Some(Working)
        );
    }

    #[test]
    fn a_tab_shows_its_most_urgent_pane_and_whose_it_is() {
        assert_eq!(tab_status(&[None, None], 0), None);
        // the other pane needs you while the focused one works
        assert_eq!(
            tab_status(&[Some(Working), Some(NeedsYou)], 0),
            Some(TabStatus {
                status: NeedsYou,
                own: false,
                pane: 1
            })
        );
        // focus it: the same status, now its own
        assert_eq!(
            tab_status(&[Some(Working), Some(NeedsYou)], 1),
            Some(TabStatus {
                status: NeedsYou,
                own: true,
                pane: 1
            })
        );
        // a tie goes to the focused pane
        assert_eq!(
            tab_status(&[Some(Done), None, Some(Done)], 2),
            Some(TabStatus {
                status: Done,
                own: true,
                pane: 2
            })
        );
        // the full order, wherever each state sits
        let mut all = vec![
            Some(Activity),
            Some(Done),
            Some(Working),
            Some(Failed),
            Some(NeedsYou),
        ];
        for _ in 0..5 {
            assert_eq!(tab_status(&all, 0).unwrap().status, NeedsYou);
            all.rotate_left(1);
        }
        assert_eq!(
            tab_status(&[Some(Activity), Some(Failed), Some(Done)], 0)
                .unwrap()
                .status,
            Failed
        );
    }

    fn t<'a>(custom: Option<&'a str>, cwd: Option<&'a str>) -> TitleInput<'a> {
        TitleInput {
            custom,
            cwd,
            fallback: "shell",
        }
    }

    #[test]
    fn titles_are_folders_and_shared_names_add_the_parent() {
        assert_eq!(
            titles(&[
                t(None, Some("~/zzz/rift")),
                t(None, Some("~")),
                t(None, None),
                t(Some("build"), Some("~/zzz/flow")),
            ]),
            ["rift", "~", "shell", "build"]
        );
        assert_eq!(
            titles(&[
                t(None, Some("~/zzz/rift/src")),
                t(None, Some("~/zzz/flow/src")),
                t(None, Some("~/zzz/flow")),
            ]),
            ["rift/src", "flow/src", "flow"]
        );
        // two tabs on the same folder: no parent needed, it is one folder
        assert_eq!(
            titles(&[t(None, Some("~/a/src")), t(None, Some("~/a/src"))]),
            ["src", "src"]
        );
        // a renamed tab does not make its folder ambiguous
        assert_eq!(
            titles(&[t(None, Some("~/a/src")), t(Some("x"), Some("~/b/src"))]),
            ["src", "x"]
        );
        // a root-level folder keeps its slash
        assert_eq!(
            titles(&[t(None, Some("/src")), t(None, Some("~/src"))]),
            ["/src", "~/src"]
        );
        assert_eq!(folder("~"), "~");
        assert_eq!(folder("/"), "/");
    }

    #[test]
    fn the_rest_of_the_path_sits_under_the_title() {
        assert_eq!(rest_of_path("~/zzz/rift", "rift"), "~/zzz");
        assert_eq!(rest_of_path("~/zzz/rift/src", "rift/src"), "~/zzz");
        assert_eq!(rest_of_path("~", "~"), "");
        assert_eq!(rest_of_path("/tmp", "tmp"), "");
        // a renamed tab: its title is no part of the path
        assert_eq!(rest_of_path("~/zzz/rift", "build"), "");
    }

    #[test]
    fn hints_number_the_first_eight_and_the_last() {
        assert_eq!(hint_numbers(0), Vec::<Option<u8>>::new());
        assert_eq!(hint_numbers(3), [Some(1), Some(2), Some(3)]);
        let ten = hint_numbers(10);
        assert_eq!(ten[7], Some(8));
        assert_eq!(ten[8], None);
        assert_eq!(ten[9], Some(9));
        // exactly nine: the ninth is both the ninth and the last
        assert_eq!(hint_numbers(9)[8], Some(9));
    }
}
