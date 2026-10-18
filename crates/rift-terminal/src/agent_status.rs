//! Coding-agent identity and state per pane, for the tab strip and the
//! vertical rail (Warp's vertical-tab agent badges).
//!
//! A pane runs an agent when its running block's command names one
//! (`Agent::from_command`: env assignments, `npx`/`bunx`, wrappers), or when
//! the agent's own notification arrives (`AgentRun` set by the notification
//! handler). The state:
//!
//! - **waiting**: the agent asked for input or permission (an open request
//!   notification: nothing typed into the pane since);
//! - **working**: its block runs and it is not waiting (an agent whose title
//!   says it sits at its own prompt counts as done instead);
//! - **done** / **error**: its block finished with exit 0 / non-zero, until
//!   the pane is viewed.
//!
//! A tab with several agent panes shows the most urgent one: waiting, then
//! error, then working, then done.

use rift_core::BlockId;
use rift_core::notifications::Agent;

/// What an agent in a pane is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Working,
    Waiting,
    Done,
    Error,
}

impl AgentState {
    /// Words for tooltips and screen readers ("Claude Code — waiting for
    /// you").
    pub fn label(self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::Waiting => "waiting for you",
            AgentState::Done => "done",
            AgentState::Error => "error",
        }
    }

    /// The `dump` value.
    pub fn key(self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::Waiting => "waiting",
            AgentState::Done => "done",
            AgentState::Error => "error",
        }
    }

    /// Which state a tab of several agent panes shows (higher wins).
    fn rank(self) -> u8 {
        match self {
            AgentState::Waiting => 3,
            AgentState::Error => 2,
            AgentState::Working => 1,
            AgentState::Done => 0,
        }
    }
}

/// An agent and what it is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentStatus {
    pub agent: Agent,
    pub state: AgentState,
}

impl AgentStatus {
    /// "Claude Code — waiting for you".
    pub fn describe(self) -> String {
        format!("{} — {}", self.agent.name(), self.state.label())
    }
}

/// The agent a pane runs, or ran until its finish is seen: the block it
/// runs in (`None`: known only from its notification, with no block).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentRun {
    pub agent: Agent,
    pub block: Option<BlockId>,
}

/// What one block of the pane says, for [`step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockView {
    pub id: BlockId,
    pub running: bool,
    /// Exit status once finished (`None`: unknown or still running).
    pub exit: Option<i32>,
    /// The agent its command names.
    pub agent: Option<Agent>,
}

/// What the pane shows this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observed {
    /// The pane's newest block.
    pub last: Option<BlockView>,
    /// The block `run.block` names, when it still exists.
    pub tracked: Option<BlockView>,
    /// An agent request (needs input / permission) is open in the pane.
    pub open_request: bool,
    /// The agent's title says it sits at its own prompt.
    pub idle_title: bool,
    /// The pane is on screen in the focused window.
    pub viewed: bool,
}

/// Advance a pane's agent run by one frame: the run to keep and the status
/// to show.
pub fn step(run: Option<AgentRun>, seen: Observed) -> (Option<AgentRun>, Option<AgentStatus>) {
    let mut run = run;
    let mut tracked = seen.tracked;
    // a new agent command took over the pane
    if let Some(last) = seen.last
        && last.running
        && let Some(agent) = last.agent
        && run.is_none_or(|r| r.block != Some(last.id))
    {
        run = Some(AgentRun {
            agent,
            block: Some(last.id),
        });
        tracked = Some(last);
    }
    let Some(r) = run else {
        return (None, None);
    };
    let state = match (r.block, tracked) {
        (Some(_), Some(b)) if b.running => {
            if seen.open_request {
                AgentState::Waiting
            } else if seen.idle_title {
                AgentState::Done
            } else {
                AgentState::Working
            }
        }
        (Some(_), Some(b)) => {
            // finished: shown until the pane is looked at
            if seen.viewed {
                return (None, None);
            }
            match b.exit {
                Some(code) if code != 0 => AgentState::Error,
                _ => AgentState::Done,
            }
        }
        // its block was cleared away (⌘K)
        (Some(_), None) => return (None, None),
        // known from a notification alone: waiting while that request is
        // open, gone once it is answered and seen
        (None, _) => {
            if seen.open_request {
                AgentState::Waiting
            } else if seen.viewed {
                return (None, None);
            } else {
                AgentState::Done
            }
        }
    };
    (
        run,
        Some(AgentStatus {
            agent: r.agent,
            state,
        }),
    )
}

/// The status a tab shows for its panes' agents: the most urgent one
/// (waiting > error > working > done; the first pane wins a tie).
pub fn tab_status(panes: impl IntoIterator<Item = Option<AgentStatus>>) -> Option<AgentStatus> {
    panes
        .into_iter()
        .flatten()
        .fold(None, |best: Option<AgentStatus>, s| match best {
            Some(b) if b.state.rank() >= s.state.rank() => Some(b),
            _ => Some(s),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(id: BlockId, running: bool, exit: Option<i32>, agent: Option<Agent>) -> BlockView {
        BlockView {
            id,
            running,
            exit,
            agent,
        }
    }

    fn seen(last: Option<BlockView>, tracked: Option<BlockView>) -> Observed {
        Observed {
            last,
            tracked,
            open_request: false,
            idle_title: false,
            viewed: false,
        }
    }

    const CLAUDE: Option<Agent> = Some(Agent::ClaudeCode);

    #[test]
    fn an_agent_command_works_waits_and_finishes() {
        let running = block(4, true, None, CLAUDE);
        let (run, status) = step(None, seen(Some(running), None));
        let run = run.expect("the agent's block is tracked");
        assert_eq!(run.block, Some(4));
        assert_eq!(status.unwrap().state, AgentState::Working);
        assert_eq!(status.unwrap().describe(), "Claude Code — working");

        // its permission request is open
        let mut s = seen(Some(running), Some(running));
        s.open_request = true;
        let (run2, status) = step(Some(run), s);
        assert_eq!(run2, Some(run));
        assert_eq!(status.unwrap().state, AgentState::Waiting);
        assert_eq!(status.unwrap().describe(), "Claude Code — waiting for you");

        // exit 0 out of view: done; exit 1: error
        let done = block(4, false, Some(0), CLAUDE);
        let (kept, status) = step(Some(run), seen(Some(done), Some(done)));
        assert_eq!(kept, Some(run));
        assert_eq!(status.unwrap().state, AgentState::Done);
        let failed = block(4, false, Some(1), CLAUDE);
        let (_, status) = step(Some(run), seen(Some(failed), Some(failed)));
        assert_eq!(status.unwrap().state, AgentState::Error);

        // viewed once finished: cleared
        let mut s = seen(Some(done), Some(done));
        s.viewed = true;
        assert_eq!(step(Some(run), s), (None, None));
    }

    #[test]
    fn a_running_agent_stays_while_viewed_and_a_new_command_replaces_a_finished_one() {
        let running = block(2, true, None, CLAUDE);
        let mut s = seen(Some(running), Some(running));
        s.viewed = true;
        let run = AgentRun {
            agent: Agent::ClaudeCode,
            block: Some(2),
        };
        assert_eq!(step(Some(run), s).1.unwrap().state, AgentState::Working);
        // an idle title (✳) at its own prompt: its turn is done
        s.idle_title = true;
        assert_eq!(step(Some(run), s).1.unwrap().state, AgentState::Done);
        // a later codex run takes over from the finished claude block
        let codex = block(3, true, None, Some(Agent::Codex));
        let finished = block(2, false, Some(0), CLAUDE);
        let (run, status) = step(Some(run), seen(Some(codex), Some(finished)));
        assert_eq!(run.unwrap().block, Some(3));
        assert_eq!(status.unwrap().agent, Agent::Codex);
    }

    #[test]
    fn plain_commands_and_cleared_blocks_show_no_agent() {
        let sleep = block(1, true, None, None);
        assert_eq!(step(None, seen(Some(sleep), None)), (None, None));
        let run = AgentRun {
            agent: Agent::ClaudeCode,
            block: Some(1),
        };
        // ⌘K cleared the agent's block
        assert_eq!(step(Some(run), seen(None, None)), (None, None));
    }

    #[test]
    fn a_notification_alone_waits_until_answered_and_seen() {
        let run = AgentRun {
            agent: Agent::Gemini,
            block: None,
        };
        let mut s = seen(None, None);
        s.open_request = true;
        s.viewed = true;
        assert_eq!(step(Some(run), s).1.unwrap().state, AgentState::Waiting);
        s.open_request = false;
        assert_eq!(step(Some(run), s), (None, None));
    }

    #[test]
    fn a_tab_shows_its_most_urgent_agent() {
        let st = |agent, state| Some(AgentStatus { agent, state });
        use AgentState::*;
        let (c, x) = (Agent::ClaudeCode, Agent::Codex);
        assert_eq!(tab_status([None, None]), None);
        assert_eq!(tab_status([st(c, Done), st(x, Working)]), st(x, Working));
        assert_eq!(tab_status([st(c, Working), st(x, Error)]), st(x, Error));
        assert_eq!(
            tab_status([st(c, Error), None, st(x, Waiting)]),
            st(x, Waiting)
        );
        assert_eq!(tab_status([st(c, Waiting), st(x, Waiting)]), st(c, Waiting));
        assert_eq!(tab_status([st(c, Done), st(x, Done)]), st(c, Done));
        // the full order: waiting > error > working > done
        let mut all = vec![st(c, Done), st(c, Working), st(c, Error), st(c, Waiting)];
        for _ in 0..4 {
            assert_eq!(tab_status(all.clone()), st(c, Waiting));
            all.rotate_left(1);
        }
    }
}
