//! OSC 9;4 progress reports (ConEmu / Windows Terminal) as the app shows
//! them: which pane's report a tab shows, the tab's accessible label, the
//! `dump` text and the macOS Dock tile badge. The reader thread keeps each
//! pane's report (`Session::progress`); the tab bar draws it
//! (`tabbar::draw_progress`). Linux has no taskbar progress here.

use rift_core::{Progress, ProgressState};
use rift_pty::ProgressReport;

use crate::state::PaneState;

/// A pane's standing report.
pub fn of_pane(p: &PaneState) -> Option<ProgressReport> {
    *p.session.progress.lock().unwrap()
}

/// Error beats paused, paused beats normal and indeterminate.
fn severity(s: ProgressState) -> u8 {
    match s {
        ProgressState::Error => 2,
        ProgressState::Paused => 1,
        ProgressState::Normal | ProgressState::Indeterminate => 0,
    }
}

/// The report a tab shows of its panes': the most severe, the newest of
/// equals.
pub fn pick(reports: impl IntoIterator<Item = ProgressReport>) -> Option<Progress> {
    reports
        .into_iter()
        .max_by_key(|r| (severity(r.progress.state), r.seq))
        .map(|r| r.progress)
}

/// The tab's report (`None` when `show_progress` is off).
pub fn of_panes<'a>(
    panes: impl IntoIterator<Item = &'a PaneState>,
    shown: bool,
) -> Option<Progress> {
    if !shown {
        return None;
    }
    pick(panes.into_iter().filter_map(of_pane))
}

/// What the tab's accessible label gains: ", 42% done", ", progress error"
/// (after an agent's own state, a bare "error" would read as the agent's).
pub fn a11y_suffix(p: Option<Progress>) -> String {
    let Some(p) = p else {
        return String::new();
    };
    match (p.state, p.pct) {
        (ProgressState::Normal, Some(n)) => format!(", {n}% done"),
        (ProgressState::Normal | ProgressState::Indeterminate, _) => ", in progress".into(),
        (ProgressState::Error, _) => ", progress error".into(),
        (ProgressState::Paused, Some(n)) => format!(", progress paused at {n}%"),
        (ProgressState::Paused, None) => ", progress paused".into(),
    }
}

/// `dump`'s `<state>:<pct>` (`none:-` without a report).
pub fn dump(p: Option<Progress>) -> String {
    match p {
        None => "none:-".into(),
        Some(p) => format!(
            "{}:{}",
            p.state.name(),
            p.pct.map_or("-".into(), |n| n.to_string())
        ),
    }
}

/// The Dock tile badge: the percentage, `!` for an error, none for an
/// indeterminate report or a paused one without a value.
pub fn badge(p: Option<Progress>) -> Option<String> {
    let p = p?;
    match (p.state, p.pct) {
        (ProgressState::Error, _) => Some("!".into()),
        (ProgressState::Indeterminate, _) | (_, None) => None,
        (_, Some(n)) => Some(format!("{n}%")),
    }
}

/// Set the Dock tile's badge label (`None` clears it). Linux: nothing.
pub fn set_dock_badge(label: Option<&str>) {
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        use objc2_foundation::NSString;
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let label = label.map(NSString::from_str);
        NSApplication::sharedApplication(mtm)
            .dockTile()
            .setBadgeLabel(label.as_deref());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = label;
}

/// System Settings › Accessibility › Display › Reduce motion (macOS): the
/// indeterminate sweep stands still. Elsewhere it moves.
pub fn reduce_motion() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::dropdown::reduce_motion()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProgressState::*;

    fn report(state: ProgressState, pct: Option<u8>, seq: u64) -> ProgressReport {
        ProgressReport {
            progress: Progress { state, pct },
            alt: false,
            seq,
        }
    }

    #[test]
    fn a_tab_shows_the_most_severe_then_the_newest_report() {
        assert_eq!(pick([]), None);
        // error beats a newer normal one
        let p = pick([report(Error, None, 1), report(Normal, Some(42), 2)]).unwrap();
        assert_eq!(p.state, Error);
        // paused beats normal and indeterminate
        let p = pick([report(Normal, Some(1), 5), report(Paused, Some(9), 3)]).unwrap();
        assert_eq!(p.state, Paused);
        // equals: the newest
        let p = pick([report(Normal, Some(10), 7), report(Indeterminate, None, 8)]).unwrap();
        assert_eq!(p.state, Indeterminate);
        let p = pick([report(Normal, Some(10), 9), report(Indeterminate, None, 8)]).unwrap();
        assert_eq!(p.pct, Some(10));
    }

    #[test]
    fn labels_badges_and_dump_text() {
        let p = |state, pct| Some(Progress { state, pct });
        assert_eq!(a11y_suffix(p(Normal, Some(42))), ", 42% done");
        assert_eq!(a11y_suffix(p(Error, Some(42))), ", progress error");
        assert_eq!(a11y_suffix(p(Indeterminate, None)), ", in progress");
        assert_eq!(a11y_suffix(p(Paused, None)), ", progress paused");
        assert_eq!(a11y_suffix(p(Paused, Some(60))), ", progress paused at 60%");
        assert_eq!(a11y_suffix(None), "");
        assert_eq!(badge(p(Normal, Some(42))).as_deref(), Some("42%"));
        assert_eq!(badge(p(Error, None)).as_deref(), Some("!"));
        assert_eq!(badge(p(Paused, Some(60))).as_deref(), Some("60%"));
        assert_eq!(badge(p(Paused, None)), None);
        assert_eq!(badge(p(Indeterminate, None)), None);
        assert_eq!(badge(None), None);
        assert_eq!(dump(p(Normal, Some(42))), "normal:42");
        assert_eq!(dump(p(Indeterminate, None)), "indeterminate:-");
        assert_eq!(dump(None), "none:-");
    }
}
