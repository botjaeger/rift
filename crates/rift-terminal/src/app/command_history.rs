//! Rift's own command history in the app (`rift_core::command_history`
//! holds the store and the ranking): which run reads and writes it, what a
//! finished block records, and the ranked rows for the completion panel's
//! history lines, the ghost text and the ⌃R pane.

use super::*;
use crate::state::HistorySession;
use rift_core::command_history::{Store, repo_of, should_record, suggestions, tier};

/// The store for this run. A scripted run (`--script`) reads it only with
/// `RIFT_SCRIPT_HISTORY=1`, as it imports the shell history, and writes it
/// only with `RIFT_SCRIPT_RECORD_HISTORY=1` and its own `XDG_DATA_HOME` (a
/// global `XDG_DATA_HOME` alone would be the user's real file).
pub(super) fn open(scripted: bool) -> Store {
    let read = rift_core::history::import_allowed(
        scripted,
        std::env::var_os("RIFT_SCRIPT_HISTORY").as_deref(),
    );
    let records = scripted_store_allowed(
        std::env::var_os("RIFT_SCRIPT_RECORD_HISTORY").as_deref(),
        std::env::var_os("XDG_DATA_HOME").as_deref(),
    );
    let path = (!scripted || records)
        .then(rift_core::command_history::path)
        .flatten();
    Store::start(path, read)
}

/// The command lines the ghost text's ✕ ignored. A scripted run reads and
/// writes the file only with `RIFT_SCRIPT_RECORD_IGNORED=1` and its own
/// `XDG_DATA_HOME`; otherwise its ignores last for the run only.
pub(super) fn open_ignored(scripted: bool) -> rift_core::ignored_suggestions::Store {
    let opted = scripted_store_allowed(
        std::env::var_os("RIFT_SCRIPT_RECORD_IGNORED").as_deref(),
        std::env::var_os("XDG_DATA_HOME").as_deref(),
    );
    let on_disk = !scripted || opted;
    let path = on_disk.then(rift_core::ignored_suggestions::path).flatten();
    rift_core::ignored_suggestions::Store::start(path, on_disk)
}

/// A scripted run's opt-in to a store on disk: the variable is `1` and the
/// run has its own data directory (a global `XDG_DATA_HOME` alone would be
/// the user's real file).
fn scripted_store_allowed(
    opt_in: Option<&std::ffi::OsStr>,
    data_home: Option<&std::ffi::OsStr>,
) -> bool {
    opt_in.is_some_and(|v| v == "1") && data_home.is_some_and(|d| !d.is_empty())
}

/// The previous command of a pane: command line, directory, exit code.
pub(super) type LastCommand = (String, Option<String>, Option<i32>);

/// The history lines of `entries` for `buffer` in directory `cwd`, best
/// first: what usually follows `last` (the pane's previous command), then
/// by directory (`command_history::suggestions`). `None` (no local
/// directory: another machine) gets none.
pub(super) fn ranked<'a>(
    entries: &'a [rift_core::command_history::Entry],
    last: Option<&LastCommand>,
    buffer: &str,
    cwd: Option<&str>,
) -> Vec<&'a str> {
    let Some(cwd) = cwd.filter(|_| !buffer.trim().is_empty()) else {
        return Vec::new();
    };
    let repo = repo_of(cwd);
    let last = last.map(|(c, d, e)| (c.as_str(), d.as_deref(), *e));
    suggestions(entries, last, buffer, Some(cwd), Some(&repo))
}

/// What a finished block means for its pane's history session: one the
/// editor submitted that finished in this run (not restored or replayed)
/// becomes the pane's previous command, and is returned with its finish
/// time and place in the session when it is to be recorded
/// ([`should_record`]).
fn note_finished(
    history: &mut HistorySession,
    b: &rift_core::Block,
) -> Option<(u64, (String, u64))> {
    let rift_core::BlockState::Done { exit, .. } = b.state else {
        return None;
    };
    let finished = b.finished_at.and_then(unix_secs)?;
    if b.restored || b.replayed || !b.from_editor {
        return None;
    }
    history.last = Some((
        b.command.clone(),
        b.cwd.clone().filter(|_| !b.cwd_remote),
        exit,
    ));
    if !should_record(&b.command, b.cwd_remote) {
        return None;
    }
    let seq = history.seq;
    history.seq += 1;
    Some((finished, (history.id.clone(), seq)))
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
        if !self.ignored.flush(Duration::from_secs(1)) {
            log::warn!("ignored suggestions: the last change was not written in time");
        }
    }

    /// Quit: the queued history writes land first, since the quit watchdog
    /// may end the process before `main`'s flush runs.
    pub(super) fn exit_app(&self, event_loop: &ActiveEventLoop) {
        self.flush_command_history();
        crate::quit::exit(event_loop);
    }

    /// Record the finished commands among `blocks` in the pane's history
    /// session `history`: those the editor submitted (a program can forge
    /// the others with OSC 133) that finished in this session (not restored
    /// or replayed) and pass [`should_record`] ([`note_finished`]).
    pub(super) fn record_command_history<'a>(
        &self,
        history: &mut HistorySession,
        blocks: impl Iterator<Item = &'a rift_core::Block>,
    ) {
        for b in blocks {
            if let Some((finished, session)) = note_finished(history, b) {
                let exit = match b.state {
                    rift_core::BlockState::Done { exit, .. } => exit,
                    _ => None,
                };
                self.command_history.record(
                    b.command.clone(),
                    b.cwd.clone(),
                    finished,
                    exit,
                    Some(session),
                );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn done(id: u64, command: &str, exit: i32) -> rift_core::Block {
        rift_core::Block {
            id,
            command: command.into(),
            state: rift_core::BlockState::Done {
                exit: Some(exit),
                duration: Duration::from_millis(1),
            },
            prompt_start: 0,
            command_start: None,
            output_start: None,
            end: None,
            unterminated: false,
            collapsed: false,
            bookmarked: false,
            cwd: Some("/x".into()),
            cwd_remote: false,
            branch: None,
            host: None,
            restored: false,
            replayed: false,
            finished_at: Some(std::time::SystemTime::now()),
            from_editor: true,
            background: false,
            skip_cols: 0,
        }
    }

    #[test]
    fn only_commands_finished_in_this_run_become_the_previous_one() {
        let mut h = HistorySession::default();
        // restored, replayed or not from the editor: never the previous one
        let mut restored = done(1, "cd project1", 0);
        restored.restored = true;
        let mut replayed = done(2, "cd project1", 0);
        replayed.replayed = true;
        let mut forged = done(3, "cd project1", 0);
        forged.from_editor = false;
        for b in [&restored, &replayed, &forged] {
            assert_eq!(note_finished(&mut h, b), None);
        }
        assert_eq!(h.last, None);
        assert_eq!(h.seq, 0);
        // a finished one is, and is recorded at the session's next place
        let (_, (id, seq)) = note_finished(&mut h, &done(4, "cd project1", 0)).unwrap();
        assert_eq!((id.as_str(), seq), (h.id.as_str(), 0));
        assert_eq!(
            h.last,
            Some(("cd project1".into(), Some("/x".into()), Some(0)))
        );
        let (_, (_, seq)) = note_finished(&mut h, &done(5, "ls", 2)).unwrap();
        assert_eq!(seq, 1);
        assert_eq!(h.last.as_ref().unwrap().2, Some(2));
        // not recorded (leading space) yet still the previous command
        assert_eq!(note_finished(&mut h, &done(6, " secret", 0)), None);
        assert_eq!(h.last.as_ref().unwrap().0, " secret");
        assert_eq!(h.seq, 2);
        // another shell: another session
        assert_ne!(HistorySession::default().id, h.id);
    }

    #[test]
    fn a_pane_on_another_machine_gets_no_rift_history() {
        let entries = vec![rift_core::command_history::Entry {
            command: "cd project1".into(),
            cwd: Some("/x".into()),
            repo: Some("/x".into()),
            finished: 1,
            exit: Some(0),
            session: Some("s".into()),
            seq: Some(0),
        }];
        let last: LastCommand = ("ls".into(), Some("/x".into()), Some(0));
        assert_eq!(
            ranked(&entries, Some(&last), "cd", Some("/x")),
            ["cd project1"]
        );
        // an ssh pane has no local directory: none of it
        assert!(ranked(&entries, Some(&last), "cd", None).is_empty());
        // its block's directory is remote: no previous command directory
        let mut h = HistorySession::default();
        let mut remote = done(1, "cd project1", 0);
        remote.cwd_remote = true;
        assert_eq!(note_finished(&mut h, &remote), None);
        assert_eq!(h.last.unwrap().1, None);
    }

    #[test]
    fn an_ignored_next_command_gives_way_to_the_next_suggestion() {
        // `cd project1` was followed by the backup twice, by the restore once
        let run = |cmd: &str, seq: u64, s: &str| rift_core::command_history::Entry {
            command: cmd.into(),
            cwd: Some("/x".into()),
            repo: Some("/x".into()),
            finished: seq,
            exit: Some(0),
            session: Some(s.into()),
            seq: Some(seq),
        };
        let entries = vec![
            run("cd project1", 0, "a"),
            run("bash db.sh backup", 1, "a"),
            run("cd project1", 0, "b"),
            run("bash db.sh backup", 1, "b"),
            run("cd project1", 0, "c"),
            run("bash db.sh restore", 1, "c"),
        ];
        let last: LastCommand = ("cd project1".into(), Some("/x".into()), Some(0));
        let lines = ranked(&entries, Some(&last), "bash db.sh", Some("/x"));
        assert_eq!(lines[0], "bash db.sh backup");
        let ghost_line = |ignored: &[&str]| {
            rift_core::completion::gather("bash db.sh", 10, None, None)
                .finish_checked_except("bash db.sh", &lines, &[], &[], None, &|l| {
                    ignored.contains(&l)
                })
                .into_iter()
                .find(|c| c.source == "history")
                .map(|c| c.insert)
        };
        assert_eq!(ghost_line(&[]).as_deref(), Some("bash db.sh backup"));
        assert_eq!(
            ghost_line(&["bash db.sh backup"]).as_deref(),
            Some("bash db.sh restore")
        );
        assert_eq!(
            ghost_line(&["bash db.sh backup", "bash db.sh restore"]),
            None
        );
    }

    #[test]
    fn a_scripted_run_keeps_its_stores_on_disk_only_when_opted_in() {
        use std::ffi::OsStr;
        let yes = Some(OsStr::new("1"));
        let dir = Some(OsStr::new("/tmp/scratch"));
        assert!(scripted_store_allowed(yes, dir));
        assert!(!scripted_store_allowed(None, dir));
        assert!(!scripted_store_allowed(Some(OsStr::new("true")), dir));
        // no data directory of its own: the user's real file, never
        assert!(!scripted_store_allowed(yes, None));
        assert!(!scripted_store_allowed(yes, Some(OsStr::new(""))));
    }
}
