use std::time::{Duration, Instant};

pub type BlockId = u64;
/// Monotone absolute line index (never reused, survives scrollback trim).
pub type AbsLine = u64;
/// A transcript block's (duration, directory, exact command), for
/// [`BlockStore::mark_restored`].
pub type Restored = (Duration, Option<String>, String);

#[derive(Debug, Clone, PartialEq)]
pub enum BlockState {
    /// OSC 133;A seen — prompt is on screen, user may be typing.
    Prompt,
    /// OSC 133;C seen — command accepted, output streaming.
    Running { started: Instant },
    /// OSC 133;D seen.
    Done {
        exit: Option<i32>,
        duration: Duration,
    },
}

#[derive(Debug, Clone)]
pub struct Block {
    pub id: BlockId,
    pub command: String,
    pub state: BlockState,
    /// First line of the prompt (OSC 133;A).
    pub prompt_start: AbsLine,
    /// Point where user input begins (OSC 133;B): (line, column).
    pub command_start: Option<(AbsLine, usize)>,
    /// First line of output (OSC 133;C).
    pub output_start: Option<AbsLine>,
    /// One past the last output line (OSC 133;D).
    pub end: Option<AbsLine>,
    /// The output's last row has no newline of its own (`printf x`): the
    /// terminal moved to a fresh row for the prompt, so a copy must not
    /// end in one.
    pub unterminated: bool,
    pub collapsed: bool,
    /// Marked for quick navigation (Warp-style block bookmarks).
    pub bookmarked: bool,
    /// Working directory when the command ran (header context line).
    pub cwd: Option<String>,
    /// Git branch when the command ran (header context line).
    pub branch: Option<String>,
    /// Replayed from a saved transcript: the command is not running and
    /// the output is from a previous session.
    pub restored: bool,
    /// Finished while the session service replayed what it kept for a
    /// window that (re)started: it finished before this window saw it, so it
    /// raises no notification.
    pub replayed: bool,
    /// Wall-clock time the command finished (history ages); `None` while
    /// it runs and for replayed blocks.
    pub finished_at: Option<std::time::SystemTime>,
}

impl Block {
    /// Rows replaced by the synthesized header: [prompt_start, output_start).
    /// Only once the command is running — while typing, raw rows show.
    pub fn suppressed_range(&self) -> Option<(AbsLine, AbsLine)> {
        match self.state {
            BlockState::Prompt => None,
            _ => self.output_start.map(|o| (self.prompt_start, o)),
        }
    }

    /// Output rows hidden when collapsed: [output_start, end).
    pub fn output_range(&self) -> Option<(AbsLine, AbsLine)> {
        match (self.output_start, self.end) {
            (Some(o), Some(e)) if e > o => Some((o, e)),
            _ => None,
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self.state {
            BlockState::Done { exit, .. } => exit,
            _ => None,
        }
    }
}

/// All blocks of one pane, oldest first.
#[derive(Debug, Default)]
pub struct BlockStore {
    blocks: std::collections::VecDeque<Block>,
    next_id: BlockId,
    /// Soft `clear` marker: the first post-clear row. The display list
    /// pads the viewport below it so older content sits above the fold while
    /// staying reachable by scrolling (Warp behavior).
    clear_pad: Option<AbsLine>,
    /// When set, markers are timestamped with this instant instead of now
    /// (output replayed from the session service keeps its original time).
    clock: Option<Instant>,
    /// The first attach's replay runs: blocks finishing now are
    /// [`Block::replayed`].
    replaying: bool,
    /// Exact text the app submitted from its editor, waiting for 133;C. It
    /// wins over the text read back from the screen, which loses combining
    /// marks and turns continuation prompts (PS2) into command text.
    submitted: Option<String>,
    /// The grid was resized while the prompt was up: the shell's redraw of
    /// the prompt that follows (zsh on SIGWINCH) can land below a reflowed
    /// copy of the old one, which must stay hidden with it.
    resized_at_prompt: bool,
}

impl BlockStore {
    /// The grid changed size; a prompt redraw may follow.
    pub fn note_resize(&mut self) {
        self.resized_at_prompt = self
            .blocks
            .back()
            .is_some_and(|b| b.state == BlockState::Prompt);
    }

    pub fn set_clock(&mut self, at: Option<Instant>) {
        self.clock = at;
    }

    /// The first attach's replay starts (true) or ends (false).
    pub fn set_replaying(&mut self, on: bool) {
        self.replaying = on;
    }

    fn now(&self) -> Instant {
        self.clock.unwrap_or_else(Instant::now)
    }

    /// Wall-clock time of [`Self::now`] (a replayed marker's own time).
    fn wall_now(&self) -> std::time::SystemTime {
        let now = std::time::SystemTime::now();
        match self.clock {
            Some(at) => now
                .checked_sub(Instant::now().saturating_duration_since(at))
                .unwrap_or(now),
            None => now,
        }
    }

    pub fn on_prompt_start(&mut self, abs: AbsLine) {
        let now = self.now();
        let (wall, replaying) = (self.wall_now(), self.replaying);
        // a new prompt without 133;C: the submission ran nothing
        self.submitted = None;
        let after_resize = std::mem::take(&mut self.resized_at_prompt);
        // an unfinished Running block means we missed D — close it here
        if let Some(last) = self.blocks.back_mut() {
            match last.state {
                BlockState::Prompt => {
                    // empty enter / redrawn prompt: move the anchor. A redraw
                    // after a resize keeps the earlier anchor, so the
                    // reflowed old prompt above it stays in the hidden range
                    if !(after_resize && abs > last.prompt_start) {
                        last.prompt_start = abs;
                    }
                    last.command_start = None;
                    return;
                }
                BlockState::Running { started } => {
                    last.end = Some(abs);
                    last.state = BlockState::Done {
                        exit: None,
                        duration: now.saturating_duration_since(started),
                    };
                    last.finished_at = Some(wall);
                    last.replayed = replaying;
                }
                BlockState::Done { .. } => {}
            }
        }
        self.blocks.push_back(Block {
            id: self.next_id,
            command: String::new(),
            state: BlockState::Prompt,
            prompt_start: abs,
            command_start: None,
            output_start: None,
            end: None,
            unterminated: false,
            collapsed: false,
            bookmarked: false,
            cwd: None,
            branch: None,
            restored: false,
            replayed: false,
            finished_at: None,
        });
        self.next_id += 1;
    }

    /// Mark every finished block as replayed from a transcript, giving each
    /// its saved duration, directory and exact command (`meta` oldest
    /// first) — the replayed screen text flattens newlines. Matched from the
    /// newest: the oldest replayed blocks may already have scrolled out of
    /// the history.
    pub fn mark_restored(&mut self, meta: &[Restored]) {
        let done = self
            .blocks
            .iter_mut()
            .rev()
            .filter(|b| matches!(b.state, BlockState::Done { .. }));
        for (b, (d, cwd, command)) in done.zip(meta.iter().rev()) {
            if let BlockState::Done { duration, .. } = &mut b.state {
                *duration = *d;
            }
            b.cwd = cwd.clone();
            b.command = command.clone();
            b.restored = true;
        }
    }

    /// Attach prompt-time context to the newest block; existing values win
    /// (the app stamps richer info at submit, the reader thread only fills
    /// gaps for commands typed in raw passthrough).
    pub fn stamp_context(&mut self, cwd: Option<String>, branch: Option<String>) {
        if let Some(b) = self.blocks.back_mut() {
            if b.cwd.is_none() {
                b.cwd = cwd;
            }
            if b.branch.is_none() {
                b.branch = branch;
            }
        }
    }

    /// The app submitted `command` from its editor: stamp prompt-time
    /// context and keep the exact text for the block 133;C starts.
    pub fn stamp_submit(&mut self, cwd: Option<String>, branch: Option<String>, command: &str) {
        self.stamp_context(cwd, branch);
        if self
            .blocks
            .back()
            .is_some_and(|b| b.state == BlockState::Prompt)
        {
            self.submitted = Some(command.to_string());
        }
    }

    /// A command was submitted at the prompt and the shell has not started
    /// it yet (no 133;C): the prompt block is no longer free for another.
    pub fn submit_pending(&self) -> bool {
        self.submitted.is_some()
    }

    pub fn on_command_start(&mut self, abs: AbsLine, col: usize) {
        if let Some(last) = self.blocks.back_mut()
            && last.state == BlockState::Prompt
        {
            last.command_start = Some((abs, col));
        }
    }

    /// `command` is the text read back from the screen; a submission from
    /// the app's editor ([`BlockStore::stamp_submit`]) replaces it.
    pub fn on_output_start(&mut self, abs: AbsLine, command: String) {
        let now = self.now();
        let submitted = self.submitted.take();
        if let Some(last) = self.blocks.back_mut()
            && last.state == BlockState::Prompt
        {
            last.command = submitted.unwrap_or(command);
            last.output_start = Some(abs);
            last.state = BlockState::Running { started: now };
        }
    }

    pub fn on_done(&mut self, abs: AbsLine, exit: Option<i32>) {
        let now = self.now();
        let (wall, replaying) = (self.wall_now(), self.replaying);
        if let Some(last) = self.blocks.back_mut()
            && let BlockState::Running { started } = last.state
        {
            last.end = Some(abs);
            last.state = BlockState::Done {
                exit,
                duration: now.saturating_duration_since(started),
            };
            last.finished_at = Some(wall);
            last.replayed = replaying;
        }
    }

    /// The running command's output ended mid-row (see
    /// [`Block::unterminated`]); call before its 133;D.
    pub fn mark_unterminated(&mut self) {
        if let Some(last) = self.blocks.back_mut()
            && matches!(last.state, BlockState::Running { .. })
        {
            last.unterminated = true;
        }
    }

    /// Drop blocks that scrolled entirely out of the kept history.
    pub fn prune(&mut self, oldest_kept: AbsLine) {
        while let Some(front) = self.blocks.front() {
            let gone = match (front.end, front.state.clone()) {
                (Some(end), _) => end <= oldest_kept,
                (None, BlockState::Prompt) => false,
                _ => false,
            };
            if gone {
                self.blocks.pop_front();
            } else {
                break;
            }
        }
    }

    /// Move every anchor through `f(line, column) -> (line, column)`: a
    /// column resize reflows wrapped lines, so rows shift under the anchors.
    pub fn remap(&mut self, mut f: impl FnMut(AbsLine, usize) -> (AbsLine, usize)) {
        for b in &mut self.blocks {
            b.prompt_start = f(b.prompt_start, 0).0;
            b.command_start = b.command_start.map(|(l, c)| f(l, c));
            b.output_start = b.output_start.map(|o| f(o, 0).0);
            b.end = b.end.map(|e| f(e, 0).0);
        }
        if let Some(line) = self.clear_pad {
            self.clear_pad = Some(f(line, 0).0);
        }
    }

    pub fn clear(&mut self) {
        self.blocks.clear();
        self.clear_pad = None;
    }

    /// Record a soft `clear` whose first post-clear row is `line`.
    pub fn set_clear_pad(&mut self, line: AbsLine) {
        self.clear_pad = Some(line);
    }

    pub fn clear_pad(&self) -> Option<AbsLine> {
        self.clear_pad
    }

    pub fn toggle_bookmark(&mut self, id: BlockId) {
        if let Some(b) = self.blocks.iter_mut().find(|b| b.id == id)
            && b.suppressed_range().is_some()
        {
            b.bookmarked = !b.bookmarked;
        }
    }

    /// Bookmarked block ids, oldest first.
    pub fn bookmarks(&self) -> Vec<BlockId> {
        self.blocks
            .iter()
            .filter(|b| b.bookmarked)
            .map(|b| b.id)
            .collect()
    }

    pub fn toggle_collapsed(&mut self, id: BlockId) {
        if let Some(b) = self.blocks.iter_mut().find(|b| b.id == id)
            && b.output_range().is_some()
        {
            b.collapsed = !b.collapsed;
        }
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Block> {
        self.blocks.iter()
    }

    pub fn last(&self) -> Option<&Block> {
        self.blocks.back()
    }

    pub fn any_running(&self) -> bool {
        self.blocks
            .iter()
            .any(|b| matches!(b.state, BlockState::Running { .. }))
    }

    pub fn snapshot(&self) -> Vec<Block> {
        self.blocks.iter().cloned().collect()
    }

    /// Working directory of the command whose output holds line `abs`
    /// (`None` for prompt rows, unattributed output, or no recorded cwd).
    pub fn cwd_at(&self, abs: AbsLine) -> Option<&str> {
        self.block_at(abs)?.cwd.as_deref()
    }

    /// The command whose output holds line `abs` (`None` for prompt rows and
    /// unattributed output).
    pub fn block_at(&self, abs: AbsLine) -> Option<&Block> {
        self.blocks
            .iter()
            .rev()
            .find(|b| b.output_start.is_some_and(|o| o <= abs) && b.end.is_none_or(|e| abs < e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle() {
        let mut s = BlockStore::default();
        s.on_prompt_start(10);
        s.on_command_start(11, 24);
        s.on_output_start(12, "cargo build".into());
        assert!(s.any_running());
        s.on_done(20, Some(0));
        let b = s.last().unwrap();
        assert_eq!(b.command, "cargo build");
        assert_eq!(b.exit_code(), Some(0));
        assert_eq!(b.suppressed_range(), Some((10, 12)));
        assert_eq!(b.output_range(), Some((12, 20)));
    }

    #[test]
    fn replayed_commands_keep_their_own_finish_time() {
        let hour = std::time::Duration::from_secs(3600);
        let mut s = BlockStore::default();
        // the first attach's replay: an hour-old command
        s.set_replaying(true);
        s.set_clock(Instant::now().checked_sub(hour));
        s.on_prompt_start(0);
        s.on_command_start(0, 2);
        s.on_output_start(1, "make".into());
        s.on_done(3, Some(0));
        s.on_prompt_start(4);
        s.set_clock(None);
        s.set_replaying(false);
        let old = s.iter().find(|b| b.command == "make").unwrap();
        assert!(old.replayed);
        let age = old.finished_at.unwrap().elapsed().unwrap();
        assert!(age >= hour - std::time::Duration::from_secs(5), "{age:?}");
        // a command after the replay is live
        s.on_command_start(4, 2);
        s.on_output_start(5, "ls".into());
        s.on_done(6, Some(0));
        let live = s.last().unwrap();
        assert!(!live.replayed);
        assert!(live.finished_at.unwrap().elapsed().unwrap() < hour);
    }

    #[test]
    fn prompt_redrawn_after_resize_keeps_its_anchor() {
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        // a narrower grid reflows the prompt; zsh redraws it a row lower
        s.note_resize();
        s.on_prompt_start(1);
        assert_eq!(s.last().unwrap().prompt_start, 0);
        // without a resize, a redrawn prompt (empty Enter) moves the anchor
        s.on_prompt_start(3);
        assert_eq!(s.last().unwrap().prompt_start, 3);
        // a resize while a command runs is not a prompt redraw
        s.on_command_start(3, 2);
        s.on_output_start(4, "ls".into());
        s.note_resize();
        s.on_done(6, Some(0));
        s.on_prompt_start(6);
        assert_eq!(s.last().unwrap().prompt_start, 6);
    }

    #[test]
    fn submitted_text_wins_over_screen_text() {
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        assert!(!s.submit_pending());
        let exact = "printf 'first\nsecond\n' caf\u{65}\u{301}";
        s.stamp_submit(None, None, exact);
        // submitted, not started: the prompt is taken
        assert!(s.submit_pending());
        s.on_output_start(2, "printf 'first cont> second' cafe".into());
        assert!(!s.submit_pending());
        assert_eq!(s.last().unwrap().command, exact);
        s.on_done(3, Some(0));
        // raw passthrough (no submission): the screen text is all there is
        s.on_prompt_start(3);
        s.on_output_start(4, "ls".into());
        assert_eq!(s.last().unwrap().command, "ls");
        s.on_done(5, Some(0));
        // a submission that ran nothing does not label the next command
        s.on_prompt_start(5);
        s.stamp_submit(None, None, "# comment");
        assert!(s.submit_pending());
        s.on_prompt_start(6);
        assert!(!s.submit_pending(), "a new prompt frees it");
        s.on_output_start(7, "pwd".into());
        assert_eq!(s.last().unwrap().command, "pwd");
        // nor does one made while a command is running
        s.stamp_submit(None, None, "typed ahead");
        s.on_done(8, Some(0));
        s.on_prompt_start(8);
        s.on_output_start(9, "date".into());
        assert_eq!(s.last().unwrap().command, "date");
    }

    #[test]
    fn cwd_at_follows_the_owning_command() {
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        s.stamp_context(Some("/a".into()), None);
        s.on_output_start(1, "ls".into());
        s.on_done(3, Some(0));
        s.on_prompt_start(3);
        s.stamp_context(Some("/b".into()), None);
        s.on_output_start(4, "ls".into());
        assert_eq!(s.cwd_at(2), Some("/a"));
        assert_eq!(s.cwd_at(9), Some("/b")); // still running
        assert_eq!(s.cwd_at(0), None); // prompt row
    }

    #[test]
    fn empty_enter_moves_prompt() {
        let mut s = BlockStore::default();
        s.on_prompt_start(5);
        s.on_prompt_start(6);
        assert_eq!(s.snapshot().len(), 1);
        assert_eq!(s.last().unwrap().prompt_start, 6);
    }

    #[test]
    fn missed_done_closes_on_next_prompt() {
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        s.on_output_start(1, "ssh box".into());
        s.on_prompt_start(9);
        let blocks = s.snapshot();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].end, Some(9));
        assert!(matches!(
            blocks[0].state,
            BlockState::Done { exit: None, .. }
        ));
    }

    #[test]
    fn prune_drops_trimmed_blocks() {
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        s.on_output_start(1, "a".into());
        s.on_done(5, Some(0));
        s.on_prompt_start(5);
        s.on_output_start(6, "b".into());
        s.on_done(9, Some(1));
        s.prune(5);
        let blocks = s.snapshot();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].command, "b");
    }

    #[test]
    fn bookmark_lifecycle() {
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        let id = s.last().unwrap().id;
        s.toggle_bookmark(id);
        assert!(s.bookmarks().is_empty(), "prompt blocks unbookmarkable");
        s.on_output_start(1, "x".into());
        s.on_done(4, Some(0));
        s.toggle_bookmark(id);
        assert_eq!(s.bookmarks(), vec![id]);
        s.toggle_bookmark(id);
        assert!(s.bookmarks().is_empty());
    }

    #[test]
    fn clear_then_reanchor_prompt() {
        // mirrors ⌘K Clear Blocks: clear() then re-anchor a Prompt block at
        // the post-clear cursor; the next command must extract correctly
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        s.on_command_start(0, 2);
        s.on_output_start(1, "old".into());
        s.on_done(4, Some(0));
        s.on_prompt_start(4);

        s.clear();
        assert!(s.snapshot().is_empty());
        s.on_prompt_start(30);
        s.on_command_start(30, 2);
        let b = s.last().unwrap();
        assert_eq!(b.state, BlockState::Prompt);
        assert_eq!(b.prompt_start, 30);
        assert_eq!(b.command_start, Some((30, 2)));
        // next submitted command flows through the normal lifecycle
        s.on_output_start(31, "echo hi".into());
        s.on_done(33, Some(0));
        let b = s.last().unwrap();
        assert_eq!(b.command, "echo hi");
        assert_eq!(b.suppressed_range(), Some((30, 31)));
        assert_eq!(b.output_range(), Some((31, 33)));
    }

    #[test]
    fn collapse_only_with_output() {
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        let id = s.last().unwrap().id;
        s.toggle_collapsed(id);
        assert!(!s.last().unwrap().collapsed);
        s.on_output_start(1, "x".into());
        s.on_done(4, Some(0));
        s.toggle_collapsed(id);
        assert!(s.last().unwrap().collapsed);
    }
}
