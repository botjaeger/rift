//! The "Time on this tab" clock in `elapsed_counts = "running"` mode: the
//! time a tab had a command running, for time logging. The tab counts while
//! ANY of its panes runs a command, so the time of overlapping panes is a
//! union (two panes busy the same hour count one hour, not two). Live time is
//! monotonic (`Instant`); the total is what a session file keeps.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct TabClock {
    /// Time counted before `since`.
    total: Duration,
    /// When the current stretch began; `None` while the tab is idle.
    since: Option<Instant>,
}

impl TabClock {
    /// A clock restored from a saved total (idle until the next `sync`).
    pub fn from_secs(secs: u64) -> Self {
        TabClock {
            total: Duration::from_secs(secs),
            since: None,
        }
    }

    /// Follow the tab: `panes` holds one running flag per pane. Counts from
    /// the moment any pane runs until none does.
    pub fn sync(&mut self, panes: impl IntoIterator<Item = bool>, now: Instant) {
        let running = panes.into_iter().any(|r| r);
        match (self.since, running) {
            (None, true) => self.since = Some(now),
            (Some(since), false) => {
                self.total += now.saturating_duration_since(since);
                self.since = None;
            }
            _ => {}
        }
    }

    /// Whether a stretch is open (some pane runs).
    pub fn counting(&self) -> bool {
        self.since.is_some()
    }

    /// Everything counted so far, the open stretch included.
    pub fn total(&self, now: Instant) -> Duration {
        self.total
            + self
                .since
                .map_or(Duration::ZERO, |s| now.saturating_duration_since(s))
    }

    /// Zero the total. A stretch in progress restarts at `now`.
    pub fn reset(&mut self, now: Instant) {
        self.total = Duration::ZERO;
        if self.since.is_some() {
            self.since = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn one_pane_counts_only_while_it_runs() {
        let t0 = Instant::now();
        let mut c = TabClock::default();
        c.sync([false], t0);
        assert_eq!(c.total(t0 + secs(30)), Duration::ZERO);
        c.sync([true], t0 + secs(30));
        assert!(c.counting());
        assert_eq!(c.total(t0 + secs(50)), secs(20));
        c.sync([false], t0 + secs(90));
        assert!(!c.counting());
        // idle time does not count
        assert_eq!(c.total(t0 + secs(500)), secs(60));
    }

    #[test]
    fn overlapping_panes_count_the_union() {
        let t0 = Instant::now();
        let mut c = TabClock::default();
        c.sync([true, false], t0); // A starts at 0
        c.sync([true, true], t0 + secs(30)); // B joins
        c.sync([false, true], t0 + secs(60)); // A ends
        c.sync([false, false], t0 + secs(90)); // B ends
        // 0..90 once, not 60 + 60
        assert_eq!(c.total(t0 + secs(1000)), secs(90));
    }

    #[test]
    fn a_gap_between_commands_is_not_counted() {
        let t0 = Instant::now();
        let mut c = TabClock::default();
        c.sync([true], t0);
        c.sync([false], t0 + secs(10));
        c.sync([true], t0 + secs(100));
        c.sync([false], t0 + secs(125));
        assert_eq!(c.total(t0 + secs(300)), secs(35));
    }

    #[test]
    fn reset_mid_run_zeroes_and_keeps_counting() {
        let t0 = Instant::now();
        let mut c = TabClock::default();
        c.sync([true], t0);
        c.reset(t0 + secs(40));
        assert_eq!(c.total(t0 + secs(40)), Duration::ZERO);
        assert!(c.counting());
        assert_eq!(c.total(t0 + secs(55)), secs(15));
        c.sync([false], t0 + secs(60));
        assert_eq!(c.total(t0 + secs(100)), secs(20));
        // reset while idle stays idle
        c.reset(t0 + secs(100));
        assert!(!c.counting());
        assert_eq!(c.total(t0 + secs(200)), Duration::ZERO);
    }

    #[test]
    fn a_saved_total_resumes_from_reattach() {
        let t0 = Instant::now();
        let mut c = TabClock::from_secs(7200);
        // restored idle: time while Rift was closed does not count
        assert_eq!(c.total(t0 + secs(5000)), secs(7200));
        c.sync([true], t0 + secs(5000));
        assert_eq!(c.total(t0 + secs(5030)), secs(7230));
    }

    #[test]
    fn closing_the_last_running_pane_stops_the_clock() {
        let t0 = Instant::now();
        let mut c = TabClock::default();
        c.sync([true, false], t0);
        // the running pane closes or moves to another tab: none runs now
        c.sync([false], t0 + secs(20));
        assert!(!c.counting());
        assert_eq!(c.total(t0 + secs(900)), secs(20));
        // a tab left with no panes at all stops too
        c.sync([true], t0 + secs(1000));
        c.sync([], t0 + secs(1010));
        assert!(!c.counting());
        assert_eq!(c.total(t0 + secs(2000)), secs(30));
    }
}
