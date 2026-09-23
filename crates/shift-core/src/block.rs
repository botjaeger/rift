use std::time::{Duration, Instant};

pub type BlockId = u64;
/// Monotone absolute line index (never reused, survives scrollback trim).
pub type AbsLine = u64;

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
    /// Soft `clear` marker: (first post-clear row, viewport rows). The
    /// display list pads here so older content sits above the fold while
    /// staying reachable by scrolling (Warp behavior).
    clear_pad: Option<(AbsLine, usize)>,
    /// When set, markers are timestamped with this instant instead of now
    /// (output replayed from the session service keeps its original time).
    clock: Option<Instant>,
}

impl BlockStore {
    pub fn set_clock(&mut self, at: Option<Instant>) {
        self.clock = at;
    }

    fn now(&self) -> Instant {
        self.clock.unwrap_or_else(Instant::now)
    }

    pub fn on_prompt_start(&mut self, abs: AbsLine) {
        let now = self.now();
        // an unfinished Running block means we missed D — close it here
        if let Some(last) = self.blocks.back_mut() {
            match last.state {
                BlockState::Prompt => {
                    // empty enter / redrawn prompt: move the anchor
                    last.prompt_start = abs;
                    last.command_start = None;
                    return;
                }
                BlockState::Running { started } => {
                    last.end = Some(abs);
                    last.state = BlockState::Done {
                        exit: None,
                        duration: now.saturating_duration_since(started),
                    };
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
            collapsed: false,
            bookmarked: false,
            cwd: None,
            branch: None,
            restored: false,
        });
        self.next_id += 1;
    }

    /// Mark every finished block as replayed from a transcript, giving each
    /// its saved duration and directory (in order, oldest first).
    pub fn mark_restored(&mut self, meta: &[(Duration, Option<String>)]) {
        let done = self
            .blocks
            .iter_mut()
            .filter(|b| matches!(b.state, BlockState::Done { .. }));
        for (b, (d, cwd)) in done.zip(meta) {
            if let BlockState::Done { duration, .. } = &mut b.state {
                *duration = *d;
            }
            b.cwd = cwd.clone();
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

    pub fn on_command_start(&mut self, abs: AbsLine, col: usize) {
        if let Some(last) = self.blocks.back_mut()
            && last.state == BlockState::Prompt
        {
            last.command_start = Some((abs, col));
        }
    }

    pub fn on_output_start(&mut self, abs: AbsLine, command: String) {
        let now = self.now();
        if let Some(last) = self.blocks.back_mut()
            && last.state == BlockState::Prompt
        {
            last.command = command;
            last.output_start = Some(abs);
            last.state = BlockState::Running { started: now };
        }
    }

    pub fn on_done(&mut self, abs: AbsLine, exit: Option<i32>) {
        let now = self.now();
        if let Some(last) = self.blocks.back_mut()
            && let BlockState::Running { started } = last.state
        {
            last.end = Some(abs);
            last.state = BlockState::Done {
                exit,
                duration: now.saturating_duration_since(started),
            };
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
        if let Some((line, rows)) = self.clear_pad {
            self.clear_pad = Some((f(line, 0).0, rows));
        }
    }

    pub fn clear(&mut self) {
        self.blocks.clear();
        self.clear_pad = None;
    }

    /// Record a soft `clear` at `line` padding the viewport with `rows`.
    pub fn set_clear_pad(&mut self, line: AbsLine, rows: usize) {
        self.clear_pad = Some((line, rows));
    }

    pub fn clear_pad(&self) -> Option<(AbsLine, usize)> {
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
