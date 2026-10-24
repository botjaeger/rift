use std::time::{Duration, Instant};

pub type BlockId = u64;
/// Monotone absolute line index (never reused, survives scrollback trim).
pub type AbsLine = u64;
/// What a transcript kept of a block, for [`BlockStore::mark_restored`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Restored {
    pub duration: Duration,
    pub cwd: Option<String>,
    /// The exact command.
    pub command: String,
    /// `cwd` is on another host.
    pub remote: bool,
    pub branch: Option<String>,
    /// The other host's name, when it was recorded.
    pub host: Option<String>,
    /// When it finished, when that was recorded.
    pub finished_at: Option<std::time::SystemTime>,
}

/// Where a block's command ran ([`BlockStore::output_dir`]).
#[derive(Debug, Clone, PartialEq)]
pub enum OutputDir {
    Local(String),
    Remote,
    Unattributed,
}

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
    /// `cwd` is a path on another host (ssh): never read on this disk.
    pub cwd_remote: bool,
    /// Git branch when the command ran (header context line).
    pub branch: Option<String>,
    /// The host `cwd` is on, when it is another one (its OSC 7 host name).
    pub host: Option<String>,
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
    /// `command` is what the app's editor submitted
    /// ([`BlockStore::stamp_submit`]), not text read back from the screen,
    /// which a program can forge with OSC 133.
    pub from_editor: bool,
    /// A background block (Warp): output a background job printed while
    /// the prompt waited ([`BlockStore::on_background_output`]). It has no
    /// command and no header; its first row starts at column `skip_cols`
    /// (the prompt the output landed after).
    pub background: bool,
    pub skip_cols: usize,
}

impl Block {
    /// A background block's hidden rows: the prompt its output landed
    /// after, [prompt_start, output_start) (it has no header).
    pub fn hidden_prompt(&self) -> Option<(AbsLine, AbsLine)> {
        self.output_start
            .filter(|_| self.background)
            .map(|o| (self.prompt_start, o))
    }

    /// Rows replaced by the synthesized header: [prompt_start, output_start).
    /// Only once the command is running — while typing, raw rows show. A
    /// background block has no header.
    pub fn suppressed_range(&self) -> Option<(AbsLine, AbsLine)> {
        match self.state {
            BlockState::Prompt => None,
            _ if self.background => None,
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
    /// Lifetime activity, retained when blocks are cleared or pruned.
    has_activity: bool,
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
    /// A background block is open before the waiting prompt: more output
    /// before the next prompt extends it.
    background_open: bool,
    /// The app sent the waiting shell a key it answers with output of its
    /// own (^C echoes `^C` and a new prompt): until that prompt, output is
    /// not a background job's.
    answer_pending: bool,
    /// Keys were typed into a running command: the shell reads them at its
    /// next prompt and echoes or runs them there, so until a command starts
    /// (133;C) output at the prompt is that, not a background job's.
    typed_ahead: bool,
    /// The grid was resized while the prompt was up: the shell's redraw of
    /// the prompt that follows (zsh on SIGWINCH) can land below a reflowed
    /// copy of the old one, which must stay hidden with it.
    resized_at_prompt: bool,
    /// [`BlockStore::ssh_target`]'s answer and the block list it was read
    /// from (length, newest id, newest command's length).
    ssh_memo: Option<SshMemo>,
}

type SshMemo = (
    (usize, BlockId, usize),
    Option<crate::remote_complete::Target>,
);

/// Blocks searched for the pane's `ssh` command, newest first.
const SSH_SCAN: usize = 200;

impl BlockStore {
    /// The `ssh` destination this pane runs (remote completions): the
    /// newest `ssh` command submitted from the app's editor, else the
    /// newest read back from the screen (a session replayed after a
    /// restart), whose options are cut to the destination
    /// ([`crate::remote_complete::parse_screen_ssh_command`]): screen text
    /// is what the remote side printed. Cached until the block list
    /// changes.
    pub fn ssh_target(&mut self) -> Option<crate::remote_complete::Target> {
        use crate::remote_complete::{parse_screen_ssh_command, parse_ssh_command};
        let key = self
            .blocks
            .back()
            .map(|b| (self.blocks.len(), b.id, b.command.len()));
        if let Some((at, target)) = &self.ssh_memo
            && Some(*at) == key
        {
            return target.clone();
        }
        let recent = || self.blocks.iter().rev().take(SSH_SCAN);
        let target = recent()
            .filter(|b| b.from_editor)
            .find_map(|b| parse_ssh_command(&b.command))
            .or_else(|| {
                recent()
                    .filter(|b| !b.from_editor)
                    .find_map(|b| parse_screen_ssh_command(&b.command))
            });
        self.ssh_memo = key.map(|k| (k, target.clone()));
        target
    }

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
        // the prompt redrawn on its own row: a PS1 that carries 133;A (the
        // user's iTerm2 integration's) is drawn again when the line is
        // accepted; the submission still waits for its 133;C, and the
        // echo's line break is not a background job's output
        let redrawn = self.blocks.back().is_some_and(|b| {
            b.state == BlockState::Prompt && b.command_start.is_some_and(|(line, _)| abs <= line)
        });
        // a new prompt without 133;C after a submission: the shell never
        // said it started (bash 3.2's DEBUG trap misses `cmd &`; a syntax
        // error): it is a block of what it printed, with no exit status
        let submitted = if redrawn { None } else { self.submitted.take() };
        self.background_open = false;
        self.answer_pending = false;
        let after_resize = std::mem::take(&mut self.resized_at_prompt);
        if let Some(last) = self.blocks.back_mut()
            && last.state == BlockState::Prompt
            && let (Some(command), Some((line, _))) = (submitted, last.command_start)
            && abs > line
        {
            // its rows: the echoed command (one per line it has), then output
            let out = (line + 1 + command.matches('\n').count() as AbsLine).min(abs);
            last.command = command;
            last.from_editor = true;
            last.output_start = Some(out);
            last.end = Some(abs);
            last.state = BlockState::Done {
                exit: None,
                duration: Duration::ZERO,
            };
            last.finished_at = Some(wall);
            last.replayed = replaying;
        }
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
            cwd_remote: false,
            branch: None,
            host: None,
            restored: false,
            replayed: false,
            finished_at: None,
            from_editor: false,
            background: false,
            skip_cols: 0,
        });
        self.next_id += 1;
    }

    /// Input reached the shell's terminal: typed into a running command, it
    /// is type-ahead the next prompt reads ([`BlockStore::on_background_output`]).
    pub fn note_input(&mut self) {
        if self
            .blocks
            .back()
            .is_some_and(|b| b.state != BlockState::Prompt)
        {
            self.typed_ahead = true;
        }
    }

    /// The app wrote a key the waiting shell answers itself (^C, ^D, ^L):
    /// what it prints before its next prompt is not background output.
    pub fn expect_answer(&mut self) {
        if self
            .blocks
            .back()
            .is_some_and(|b| b.state == BlockState::Prompt)
        {
            self.answer_pending = true;
        }
    }

    /// Output arrived while the prompt waited for input (nothing submitted)
    /// and left the cursor at (`line`, `col`), on a row below where input
    /// starts (133;B): a background job printed it (a prompt's own drawing,
    /// a right prompt or a redraw, stays on or above that row). It goes into a background
    /// block before the prompt's, from the input point on, and the prompt
    /// block moves to the cursor (where the next command will echo). More
    /// output extends the same block.
    pub fn on_background_output(&mut self, line: AbsLine, col: usize, input_row_used: bool) {
        let Some(prompt) = self.blocks.back() else {
            return;
        };
        let Some((input_line, input_col)) = prompt.command_start else {
            return;
        };
        if prompt.state != BlockState::Prompt
            || self.submitted.is_some()
            || self.answer_pending
            || self.typed_ahead
            || line <= input_line
        {
            return;
        }
        // output that began with a line break leaves the prompt's row as it
        // was: the block starts on the next row (`input_row_used`: the job
        // printed after the prompt on its row)
        let (first, skip) = if input_row_used {
            (input_line, input_col)
        } else {
            (input_line + 1, 0)
        };
        // a cursor mid-row: that row is the block's too
        let end = line + AbsLine::from(col > 0);
        self.has_activity = true;
        let n = self.blocks.len();
        let open = self.background_open && n >= 2 && self.blocks[n - 2].background;
        if open {
            self.blocks[n - 2].end = Some(end.max(self.blocks[n - 2].end.unwrap_or(end)));
        } else {
            let wall = self.wall_now();
            let prompt = self.blocks.back_mut().expect("checked above");
            let prompt_rows = prompt.prompt_start;
            // the background block takes the prompt's id (nothing points at
            // a waiting prompt); the prompt gets a new one
            let id = prompt.id;
            prompt.id = self.next_id;
            self.next_id += 1;
            let block = Block {
                id,
                command: String::new(),
                state: BlockState::Done {
                    exit: None,
                    duration: Duration::ZERO,
                },
                // the prompt the output landed after is hidden with it
                prompt_start: prompt_rows,
                command_start: None,
                output_start: Some(first),
                end: Some(end.max(first)),
                unterminated: col > 0,
                collapsed: false,
                bookmarked: false,
                cwd: None,
                cwd_remote: false,
                branch: None,
                host: None,
                restored: false,
                replayed: self.replaying,
                finished_at: Some(wall),
                from_editor: false,
                background: true,
                skip_cols: skip,
            };
            self.blocks.insert(n - 1, block);
            self.background_open = true;
        }
        let prompt = self.blocks.back_mut().expect("checked above");
        prompt.prompt_start = line;
        prompt.command_start = Some((line, col));
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
        for (b, m) in done.zip(meta.iter().rev()) {
            if let BlockState::Done { duration, .. } = &mut b.state {
                *duration = m.duration;
            }
            b.cwd = m.cwd.clone();
            b.cwd_remote = m.remote;
            b.command = m.command.clone();
            b.branch = m.branch.clone();
            b.host = m.host.clone().filter(|_| m.remote);
            // the replay's own time is not when it ran: unknown unless kept
            b.finished_at = m.finished_at;
            b.restored = true;
        }
    }

    /// Attach prompt-time context to the newest block; existing values win
    /// (the app stamps richer info at submit, the reader thread only fills
    /// gaps for commands typed in raw passthrough).
    pub fn stamp_context(&mut self, cwd: Option<String>, remote: bool, branch: Option<String>) {
        if let Some(b) = self.blocks.back_mut() {
            if b.cwd.is_none() {
                b.cwd_remote = remote && cwd.is_some();
                b.cwd = cwd;
            }
            if b.branch.is_none() {
                b.branch = branch;
            }
        }
    }

    /// The other host's name for the newest block, when its directory is
    /// remote and no name is set yet.
    pub fn stamp_host(&mut self, host: Option<String>) {
        if let Some(b) = self.blocks.back_mut()
            && b.cwd_remote
            && b.host.is_none()
        {
            b.host = host.filter(|h| !h.is_empty());
        }
    }

    /// The app submitted `command` from its editor: stamp prompt-time
    /// context and keep the exact text for the block 133;C starts.
    pub fn stamp_submit(
        &mut self,
        cwd: Option<String>,
        remote: bool,
        branch: Option<String>,
        command: &str,
    ) {
        self.has_activity |= !command.trim().is_empty();
        self.stamp_context(cwd, remote, branch);
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
        // the shell drew its prompt again: later output starts a new block
        self.background_open = false;
        if let Some(last) = self.blocks.back_mut()
            && last.state == BlockState::Prompt
        {
            last.command_start = Some((abs, col));
        }
    }

    /// `command` is the text read back from the screen; a submission from
    /// the app's editor ([`BlockStore::stamp_submit`]) replaces it.
    pub fn on_output_start(&mut self, abs: AbsLine, command: String) {
        self.has_activity = true;
        let now = self.now();
        self.typed_ahead = false;
        let submitted = self.submitted.take();
        if let Some(last) = self.blocks.back_mut()
            && last.state == BlockState::Prompt
        {
            last.from_editor = submitted.is_some();
            last.command = submitted.unwrap_or(command);
            last.output_start = Some(abs);
            last.state = BlockState::Running { started: now };
        }
    }

    /// The shell's copy of the running command's line, from a 133;C that
    /// came after another (the user's own integration sent a bare one
    /// first): it replaces the screen's text, never the app's submitted one.
    pub fn on_command_line(&mut self, command: &str) {
        if let Some(last) = self.blocks.back_mut()
            && matches!(last.state, BlockState::Running { .. })
            && !last.from_editor
        {
            last.command = command.to_string();
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

    /// Whether this pane has never submitted a command or received output.
    pub fn is_fresh(&self) -> bool {
        !self.has_activity
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

    /// Fold block `id` (also one still running: its output folds once it
    /// ends, as the display list shows a running block's output).
    pub fn fold(&mut self, id: BlockId) {
        if let Some(b) = self.blocks.iter_mut().find(|b| b.id == id) {
            b.collapsed = true;
        }
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

    /// Where the command owning line `abs` ran: a directory on this machine,
    /// another host (ssh), or unknown (prompt rows, unattributed output, no
    /// recorded cwd), which callers settle from the pane's current state.
    pub fn output_dir(&self, abs: AbsLine) -> OutputDir {
        match self.block_at(abs) {
            Some(Block {
                cwd: Some(_),
                cwd_remote: true,
                ..
            }) => OutputDir::Remote,
            Some(Block { cwd: Some(dir), .. }) => OutputDir::Local(dir.clone()),
            _ => OutputDir::Unattributed,
        }
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
    fn freshness_survives_clear_and_unlabelled_output() {
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        s.on_command_start(0, 2);
        assert!(s.is_fresh());
        s.on_output_start(1, String::new());
        s.on_done(3, Some(0));
        s.clear();
        s.on_prompt_start(3);
        assert!(!s.is_fresh());
    }

    #[test]
    fn submission_and_background_output_end_freshness() {
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        s.on_command_start(0, 2);
        s.stamp_submit(None, false, None, "echo hi");
        assert!(!s.is_fresh());
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        s.on_command_start(0, 2);
        s.on_background_output(2, 0, false);
        assert!(!s.is_fresh());
    }

    #[test]
    fn restored_blocks_keep_their_branch_host_and_time_or_none() {
        let mut s = BlockStore::default();
        for (i, cmd) in ["deploy", "ls"].iter().enumerate() {
            let at = i as AbsLine * 3;
            s.on_prompt_start(at);
            s.on_command_start(at, 2);
            s.on_output_start(at + 1, cmd.to_string());
            s.on_done(at + 2, Some(0));
        }
        s.on_prompt_start(6);
        let when = std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        s.mark_restored(&[
            Restored {
                command: "deploy".into(),
                cwd: Some("/srv".into()),
                remote: true,
                branch: Some("main".into()),
                host: Some("prod-1".into()),
                finished_at: Some(when),
                ..Default::default()
            },
            Restored {
                command: "ls".into(),
                // a host name without a remote directory means nothing
                host: Some("ghost".into()),
                ..Default::default()
            },
        ]);
        let done: Vec<&Block> = s.iter().filter(|b| b.restored).collect();
        assert_eq!(done[0].host.as_deref(), Some("prod-1"));
        assert_eq!(done[0].branch.as_deref(), Some("main"));
        assert_eq!(done[0].finished_at, Some(when));
        assert_eq!(done[1].host, None);
        // the replay's own time is not when it ran
        assert_eq!(done[1].finished_at, None);
        // stamping: only a remote block takes a host, once
        s.on_command_start(6, 2);
        s.stamp_context(Some("/srv".into()), true, None);
        s.stamp_host(Some("db-1".into()));
        s.stamp_host(Some("other".into()));
        assert_eq!(s.last().unwrap().host.as_deref(), Some("db-1"));
    }

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
    fn the_ssh_target_comes_from_the_editor_not_from_forged_screen_text() {
        use crate::remote_complete::Target;
        let mut s = BlockStore::default();
        let mut line = 0;
        let mut run = |s: &mut BlockStore, typed: Option<&str>, screen: &str| {
            s.on_prompt_start(line);
            if let Some(cmd) = typed {
                s.stamp_submit(None, false, None, cmd);
            }
            s.on_output_start(line + 1, screen.into());
            line += 2;
        };
        run(&mut s, Some("ssh -p 2222 box"), "ssh -p 2222 box");
        let typed = Some(Target {
            args: vec!["-p".into(), "2222".into()],
            host: "box".into(),
        });
        assert_eq!(s.ssh_target(), typed);
        // the remote side prints its own 133 markers around a command
        run(&mut s, None, "ssh -F /tmp/evil.conf -S /tmp/sock other");
        assert!(!s.last().unwrap().from_editor);
        assert_eq!(s.ssh_target(), typed, "the editor's command wins");
        // with no editor command (a replayed session), the screen's is
        // cut to its destination
        let mut s = BlockStore::default();
        run(
            &mut s,
            None,
            "ssh -F /tmp/evil.conf -S /tmp/sock -p 22 other",
        );
        assert_eq!(
            s.ssh_target(),
            Some(Target {
                args: vec!["-p".into(), "22".into()],
                host: "other".into(),
            })
        );
        // cached until the block list changes: the commands are not read
        // again (this one is no ssh command any more)
        let n = s.blocks.back().unwrap().command.len();
        s.blocks.back_mut().unwrap().command = "x".repeat(n);
        assert_eq!(s.ssh_target().map(|t| t.host), Some("other".into()));
        run(&mut s, Some("ssh new"), "ssh new");
        assert_eq!(s.ssh_target().map(|t| t.host), Some("new".into()));
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
        s.stamp_submit(None, false, None, exact);
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
        s.stamp_submit(None, false, None, "# comment");
        assert!(s.submit_pending());
        s.on_prompt_start(6);
        assert!(!s.submit_pending(), "a new prompt frees it");
        s.on_output_start(7, "pwd".into());
        assert_eq!(s.last().unwrap().command, "pwd");
        // nor does one made while a command is running
        s.stamp_submit(None, false, None, "typed ahead");
        s.on_done(8, Some(0));
        s.on_prompt_start(8);
        s.on_output_start(9, "date".into());
        assert_eq!(s.last().unwrap().command, "date");
    }

    #[test]
    fn a_prompt_redrawn_on_its_row_keeps_the_submission() {
        // iTerm2's integration puts 133;A in PS1: accepting the line redraws
        // the prompt (A, B again on its row) before the echo's line break
        // and 133;C. Neither the redraw nor that break may take the
        // submission or make a background block of the echo.
        let mut s = BlockStore::default();
        s.on_prompt_start(4);
        s.on_command_start(4, 20);
        s.stamp_submit(None, false, None, "cargo clean");
        s.on_prompt_start(4);
        s.on_command_start(4, 20);
        assert!(s.submit_pending(), "the redraw keeps it");
        s.on_background_output(5, 0, true);
        s.on_output_start(5, String::new());
        s.on_done(6, Some(0));
        assert_eq!(s.blocks.len(), 1, "no background block");
        let b = &s.blocks[0];
        assert_eq!(b.command, "cargo clean");
        assert!(b.from_editor);
        assert_eq!(
            (b.prompt_start, b.output_start, b.end),
            (4, Some(5), Some(6))
        );
    }

    #[test]
    fn a_submission_the_shell_never_started_is_still_a_block() {
        // bash 3.2: `sleep 1 &` gets no 133;C (its DEBUG trap misses
        // asynchronous commands), only the next prompt
        let mut s = BlockStore::default();
        s.on_prompt_start(4);
        s.on_command_start(4, 20);
        s.stamp_submit(None, false, None, "sleep 1 &");
        s.on_prompt_start(6);
        let b = &s.blocks[0];
        assert_eq!(b.command, "sleep 1 &");
        assert_eq!((b.output_start, b.end), (Some(5), Some(6)));
        assert_eq!(b.exit_code(), None);
        assert!(matches!(b.state, BlockState::Done { .. }));
        assert_eq!(s.last().unwrap().state, BlockState::Prompt);
        // a multi-line command echoes one row per line before its output
        s.on_command_start(6, 2);
        s.stamp_submit(None, false, None, "echo a\necho b");
        s.on_prompt_start(10);
        assert_eq!(s.blocks[1].output_start, Some(8));
        // the same prompt again with nothing submitted moves it, as before
        s.on_prompt_start(11);
        assert_eq!(s.blocks.len(), 3);
        assert_eq!(s.last().unwrap().prompt_start, 11);
    }

    #[test]
    fn background_output_gets_its_own_block_before_the_prompt() {
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        s.on_command_start(0, 2);
        s.on_output_start(1, "sleep 1 &".into());
        s.on_done(2, Some(0));
        // the prompt on rows 2–3, input after column 4 of row 3
        s.on_prompt_start(2);
        s.on_command_start(3, 4);
        // the prompt's own drawing (a right prompt) stays on its row
        s.on_background_output(3, 70, true);
        assert_eq!(s.blocks.len(), 2);
        // a job prints two lines: cursor at the start of row 5
        s.on_background_output(5, 0, true);
        assert_eq!(s.blocks.len(), 3);
        let bg = &s.blocks[1];
        assert!(bg.background && bg.command.is_empty());
        assert_eq!(bg.suppressed_range(), None, "no header");
        assert_eq!(bg.hidden_prompt(), Some((2, 3)));
        assert_eq!(
            (bg.output_start, bg.end, bg.skip_cols),
            (Some(3), Some(5), 4)
        );
        let prompt = s.last().unwrap();
        assert_eq!(prompt.state, BlockState::Prompt);
        assert_eq!(
            (prompt.prompt_start, prompt.command_start),
            (5, Some((5, 0)))
        );
        assert_ne!(prompt.id, bg.id);
        // more output extends it; a partial row is the block's too
        s.on_background_output(6, 3, true);
        assert_eq!(s.blocks.len(), 3);
        assert_eq!(s.blocks[1].end, Some(7));
        // the next command runs after it as usual
        s.stamp_submit(None, false, None, "ls");
        s.on_background_output(8, 0, true);
        assert_eq!(s.blocks.len(), 3, "not while a submission waits");
        s.on_output_start(7, "ls".into());
        assert_eq!(s.last().unwrap().command, "ls");
        // a redrawn prompt ends it: later output starts another
        s.on_done(8, Some(0));
        s.on_prompt_start(8);
        s.on_command_start(8, 2);
        s.on_background_output(9, 0, true);
        s.on_command_start(9, 2);
        s.on_background_output(10, 0, true);
        assert_eq!(s.blocks.iter().filter(|b| b.background).count(), 3);
        // output that starts with a line break (zsh's `[1] + done` notice)
        // starts the block on the row after the prompt's
        s.on_done(11, Some(0));
        s.on_prompt_start(11);
        s.on_command_start(11, 2);
        s.on_background_output(13, 0, false);
        let bg = s.blocks.iter().rev().find(|b| b.background).unwrap();
        assert_eq!(
            (bg.output_start, bg.skip_cols, bg.hidden_prompt()),
            (Some(12), 0, Some((11, 12)))
        );
        // ^C at the prompt: the shell's echo and new prompt are its own
        s.on_prompt_start(13);
        s.on_command_start(13, 2);
        s.expect_answer();
        s.on_background_output(14, 0, false);
        assert_eq!(s.blocks.iter().filter(|b| b.background).count(), 4);
        s.on_prompt_start(14);
        s.on_command_start(14, 2);
        s.on_background_output(15, 0, true);
        assert_eq!(
            s.blocks.iter().filter(|b| b.background).count(),
            5,
            "armed again"
        );
        // keys typed into a running command run at the next prompt
        let bg = |s: &BlockStore| s.blocks.iter().filter(|b| b.background).count();
        s.on_prompt_start(16);
        s.on_command_start(16, 2);
        s.on_output_start(17, "sleep 2".into());
        s.note_input();
        s.on_done(18, Some(0));
        s.on_prompt_start(18);
        s.on_command_start(18, 2);
        s.on_background_output(19, 0, true);
        assert_eq!(bg(&s), 5, "type-ahead");
        s.on_output_start(19, "echo typed".into());
        s.on_done(20, Some(0));
        s.on_prompt_start(20);
        s.on_command_start(20, 2);
        s.on_background_output(21, 0, true);
        assert_eq!(bg(&s), 6, "armed again after it ran");
    }

    #[test]
    fn cwd_at_follows_the_owning_command() {
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        s.stamp_context(Some("/a".into()), false, None);
        s.on_output_start(1, "ls".into());
        s.on_done(3, Some(0));
        s.on_prompt_start(3);
        s.stamp_context(Some("/b".into()), false, None);
        s.on_output_start(4, "ls".into());
        assert_eq!(s.cwd_at(2), Some("/a"));
        assert_eq!(s.cwd_at(9), Some("/b")); // still running
        assert_eq!(s.cwd_at(0), None); // prompt row
    }

    #[test]
    fn output_dir_knows_which_host_each_block_ran_on() {
        let mut s = BlockStore::default();
        s.on_prompt_start(0);
        s.stamp_context(Some("/a".into()), false, None);
        s.on_output_start(1, "ls".into());
        s.on_done(3, Some(0));
        s.on_prompt_start(3);
        s.stamp_context(Some("/srv".into()), true, None);
        s.on_output_start(4, "ls".into());
        // an old local block stays local after the pane went remote
        assert_eq!(s.output_dir(2), OutputDir::Local("/a".into()));
        assert_eq!(s.output_dir(9), OutputDir::Remote);
        assert_eq!(s.output_dir(0), OutputDir::Unattributed);
        // a remote flag without a cwd names nothing
        s.on_done(10, Some(0));
        s.on_prompt_start(10);
        s.stamp_context(None, true, None);
        s.on_output_start(11, "ls".into());
        assert_eq!(s.output_dir(12), OutputDir::Unattributed);
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
