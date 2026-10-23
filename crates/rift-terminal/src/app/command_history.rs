//! Rift's own command history in the app (`rift_core::command_history`
//! holds the store and the ranking): which run reads and writes it, what a
//! finished block records, and the ranked rows for the completion panel's
//! history lines, the ghost text and the ⌃R pane.

use super::*;
use rift_core::command_history::{Store, ranked_matches, repo_of, should_record, tier};

/// The store for this run. A scripted run (`--script`) reads it only with
/// `RIFT_SCRIPT_HISTORY=1`, as it imports the shell history, and writes it
/// only with `RIFT_SCRIPT_RECORD_HISTORY=1` and its own `XDG_DATA_HOME` (a
/// global `XDG_DATA_HOME` alone would be the user's real file).
pub(super) fn open(scripted: bool) -> Store {
    let read = rift_core::history::import_allowed(
        scripted,
        std::env::var_os("RIFT_SCRIPT_HISTORY").as_deref(),
    );
    let records = std::env::var_os("RIFT_SCRIPT_RECORD_HISTORY").is_some_and(|v| v == "1")
        && std::env::var_os("XDG_DATA_HOME").is_some_and(|d| !d.is_empty());
    let path = (!scripted || records)
        .then(rift_core::command_history::path)
        .flatten();
    Store::start(path, read)
}

/// The history lines of `entries` for `buffer` in directory `cwd`, best
/// first. `None` (no local directory: another machine) gets none.
pub(super) fn ranked<'a>(
    entries: &'a [rift_core::command_history::Entry],
    buffer: &str,
    cwd: Option<&str>,
) -> Vec<&'a str> {
    let Some(cwd) = cwd.filter(|_| !buffer.trim().is_empty()) else {
        return Vec::new();
    };
    let repo = repo_of(cwd);
    ranked_matches(entries, buffer, Some(cwd), Some(&repo))
}

fn unix_secs(t: std::time::SystemTime) -> Option<u64> {
    t.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

impl App {
    /// At quit: wait briefly for the history writes still queued.
    pub fn flush_command_history(&self) {
        if !self.command_history.flush(Duration::from_secs(1)) {
            log::warn!("command history: the last entries were not written in time");
        }
    }

    /// Quit: the queued history writes land first, since the quit watchdog
    /// may end the process before `main`'s flush runs.
    pub(super) fn exit_app(&self, event_loop: &ActiveEventLoop) {
        self.flush_command_history();
        crate::quit::exit(event_loop);
    }

    /// Record the finished commands among `blocks`: those the editor
    /// submitted (a program can forge the others with OSC 133) that
    /// finished in this session (not restored or replayed) and pass
    /// [`should_record`].
    pub(super) fn record_command_history<'a>(
        &self,
        blocks: impl Iterator<Item = &'a rift_core::Block>,
    ) {
        for b in blocks {
            let rift_core::BlockState::Done { exit, .. } = b.state else {
                continue;
            };
            let Some(finished) = b.finished_at.and_then(unix_secs) else {
                continue;
            };
            if b.restored || b.replayed || !b.from_editor {
                continue;
            }
            if should_record(&b.command, b.cwd_remote) {
                self.command_history
                    .record(b.command.clone(), b.cwd.clone(), finished, exit);
            }
        }
    }

    /// ⌃R's rows before the typed and imported history: this window's
    /// finished commands and Rift's saved history, ranked like the ghost
    /// text — run in the active pane's directory, then in its repository,
    /// then anywhere; newest first within each.
    pub(super) fn ranked_history_rows(
        &self,
        active: Option<PaneId>,
    ) -> Vec<crate::views::history::Entry> {
        use crate::views::history::Entry;
        let here = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .and_then(|p| p.session.local_cwd());
        let here_repo = here.as_deref().map(repo_of);
        // the blocks first, each pane's lock held only to copy them: the
        // repositories are looked up after (the reader thread waits on it)
        let mut done: Vec<(Option<String>, Entry)> = Vec::new();
        for tab in &self.win.tabs {
            for p in &tab.panes {
                let in_active = Some(p.session.id) == active;
                let blocks = p.session.blocks.lock().unwrap();
                for b in blocks.iter() {
                    if !matches!(b.state, rift_core::BlockState::Done { .. }) {
                        continue;
                    }
                    done.push((
                        b.cwd.clone().filter(|_| !b.cwd_remote),
                        Entry {
                            command: b.command.clone(),
                            cwd: b.cwd.as_deref().map(editor::shorten_home),
                            exit: b.exit_code(),
                            finished: b.finished_at,
                            here: in_active,
                        },
                    ));
                }
            }
        }
        let mut rows: Vec<(u8, Entry)> = done
            .into_iter()
            .map(|(local, e)| {
                let repo = local.as_deref().map(repo_of);
                let t = tier(
                    local.as_deref(),
                    repo.as_deref(),
                    here.as_deref(),
                    here_repo.as_deref(),
                );
                (t, e)
            })
            .collect();
        for e in self.command_history.snapshot().iter() {
            let t = tier(
                e.cwd.as_deref(),
                e.repo.as_deref(),
                here.as_deref(),
                here_repo.as_deref(),
            );
            rows.push((
                t,
                Entry {
                    command: e.command.clone(),
                    cwd: e.cwd.as_deref().map(editor::shorten_home),
                    exit: e.exit,
                    finished: std::time::UNIX_EPOCH.checked_add(Duration::from_secs(e.finished)),
                    here: false,
                },
            ));
        }
        // stable: at equal time a block of this window stays before its
        // saved copy; unknown times (replayed blocks) last in their tier
        rows.sort_by_key(|(t, e)| (*t, std::cmp::Reverse(e.finished)));
        rows.into_iter().map(|(_, e)| e).collect()
    }
}
