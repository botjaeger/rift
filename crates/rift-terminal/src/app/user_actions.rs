//! `[[actions]]` (`rift_core::actions`) in the app: a run walks its steps
//! in order against one target pane, kept by id. A step that opens a pane
//! (a split, a tab, a launch configuration) makes that pane the target, and
//! a `send` to it waits for its shell's first prompt, so the text lands in
//! the new pane whatever the user focuses meanwhile. The first failing step
//! stops the run with a toast naming the action and the step; earlier steps
//! stay done.

use super::*;
use rift_core::actions::Step;

/// How long a `send` waits for a pane the run opened to show its first
/// prompt; a shell without integration never does, and gets the text raw.
const PROMPT_WAIT: Duration = Duration::from_secs(10);
/// How often a waiting run looks again.
const POLL: Duration = Duration::from_millis(50);

pub(crate) struct ActionRun {
    name: String,
    steps: Vec<Step>,
    next: usize,
    /// The pane the steps act on (the focused one when the run started).
    target: Option<PaneId>,
    /// A pane this run opened, and when: a `send` waits for its prompt.
    fresh: Option<(PaneId, Instant)>,
}

/// Actions that open a pane (the target moves to it).
fn opens_pane(action: AppAction) -> bool {
    matches!(
        action,
        AppAction::NewTab
            | AppAction::NewWindow
            | AppAction::SplitRight
            | AppAction::SplitDown
            | AppAction::NewTabProfile(_)
            | AppAction::ReopenClosedTab
    )
}

impl App {
    /// Run `[[actions]]` entry `i` (its chord or palette row).
    pub(super) fn run_user_action(&mut self, i: usize, event_loop: &ActiveEventLoop) {
        let Some(name) = self
            .config
            .actions
            .get(i)
            .map(|a| a.name.trim().to_string())
        else {
            self.toast_error("That action is no longer in config.toml".into());
            return;
        };
        let builtin = rift_core::keybindings::all_actions();
        let steps = match rift_core::actions::flatten(&self.config.actions, i, &builtin) {
            Ok(steps) => steps,
            Err(e) => {
                self.toast_error(e);
                return;
            }
        };
        self.win.action_runs.push(ActionRun {
            name,
            steps,
            next: 0,
            target: self.active_pane_id(),
            fresh: None,
        });
        self.tick_user_actions(event_loop);
    }

    /// Advance this window's runs as far as each goes; the next wake while
    /// one waits for a prompt.
    pub(super) fn tick_user_actions(&mut self, event_loop: &ActiveEventLoop) -> Option<Instant> {
        if self.win.action_runs.is_empty() {
            return None;
        }
        let runs = std::mem::take(&mut self.win.action_runs);
        let mut kept = Vec::new();
        let mut due = None;
        for mut run in runs {
            if let Some(at) = self.advance_run(&mut run, event_loop) {
                due = earliest(due, Some(at));
                kept.push(run);
            }
        }
        // runs started by a step (none today) keep their place after these
        kept.append(&mut self.win.action_runs);
        self.win.action_runs = kept;
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
        due
    }

    /// Run `run`'s steps until it ends (`None`) or waits (`Some(wake)`).
    fn advance_run(
        &mut self,
        run: &mut ActionRun,
        event_loop: &ActiveEventLoop,
    ) -> Option<Instant> {
        while let Some(step) = run.steps.get(run.next).cloned() {
            let failed = |app: &mut App, why: &str| {
                app.toast_error(format!(
                    "Action \u{201c}{}\u{201d} stopped at step {} ({}): {why}",
                    run.name,
                    run.next + 1,
                    step.describe()
                ));
            };
            match &step {
                Step::Action(name) => {
                    let Some(action) = input::action_from_name(name) else {
                        failed(self, "Rift has no such action");
                        return None;
                    };
                    if let Some(t) = run.target
                        && !self.focus_pane_by_id(t)
                    {
                        failed(self, "its pane is closed");
                        return None;
                    }
                    let before: Vec<PaneId> = self.window_pane_ids();
                    self.handle_action(action, event_loop);
                    if self.win.confirm.is_some() {
                        failed(self, "it waits for a confirmation");
                        return None;
                    }
                    let after = self.active_pane_id();
                    if opens_pane(action) {
                        match after.filter(|id| !before.contains(id)) {
                            Some(id) => {
                                run.target = Some(id);
                                run.fresh = Some((id, Instant::now()));
                            }
                            None => {
                                failed(self, "no pane opened");
                                return None;
                            }
                        }
                    } else {
                        run.target = after;
                    }
                }
                Step::Open(name) => {
                    let cfg = self
                        .launch_configs
                        .iter()
                        .find(|c| c.name.trim() == name)
                        .cloned();
                    let Some(cfg) = cfg else {
                        failed(self, "no launch configuration has that name");
                        return None;
                    };
                    let tabs = self.win.tabs.len();
                    self.apply_launch_config(&cfg);
                    if self.win.tabs.len() == tabs {
                        // apply_launch_config said why in a toast of its own
                        failed(self, "it did not open");
                        return None;
                    }
                    run.target = self.active_pane_id();
                    run.fresh = run.target.map(|id| (id, Instant::now()));
                }
                Step::Send(text) => {
                    let Some(t) = run.target.filter(|t| self.window_pane_ids().contains(t)) else {
                        failed(self, "its pane is closed");
                        return None;
                    };
                    if let Some((id, since)) = run.fresh
                        && id == t
                        && !self.pane_ready(t)
                        && since.elapsed() < PROMPT_WAIT
                    {
                        return Some(Instant::now() + POLL);
                    }
                    run.fresh = None;
                    // refused or lost text fails the step (and the run)
                    if self.pane_read_only(t) {
                        failed(self, "the pane is read-only");
                        return None;
                    }
                    if self.pane_exited(t) {
                        failed(self, "the pane's program has exited");
                        return None;
                    }
                    self.type_into_pane(t, text);
                }
            }
            run.next += 1;
        }
        None
    }

    /// Every shell pane of this window.
    fn window_pane_ids(&self) -> Vec<PaneId> {
        self.win
            .tabs
            .iter()
            .flat_map(|t| t.panes.iter().map(|p| p.session.id))
            .collect()
    }

    /// Focus pane `id` of this window (its tab, the pane); false when it is
    /// not here any more.
    fn focus_pane_by_id(&mut self, id: PaneId) -> bool {
        let Some(ti) = self.win.tabs.iter().position(|t| t.pane(id).is_some()) else {
            return false;
        };
        if ti != self.win.active {
            self.switch_tab(ti);
        }
        let tab = &mut self.win.tabs[ti];
        if tab.active_pane != id {
            tab.active_pane = id;
            tab.zoomed = None;
        }
        self.win.viewer_focus = false;
        true
    }

    /// Pane `id` is kept after its program exited (the Restart bar).
    fn pane_exited(&self, id: PaneId) -> bool {
        self.win
            .tabs
            .iter()
            .find_map(|t| t.pane(id))
            .is_some_and(|p| p.exited.is_some())
    }

    /// A pane this run opened has drawn its first prompt to its end
    /// (OSC 133;B): its shell reads the line now. Text sent at 133;A
    /// arrives before the line editor starts and the terminal echoes it.
    fn pane_ready(&self, id: PaneId) -> bool {
        self.pane_at_prompt(id)
            && self
                .win
                .tabs
                .iter()
                .find_map(|t| t.pane(id))
                .is_some_and(|p| {
                    let blocks = p.session.blocks.lock().unwrap();
                    blocks.last().is_some_and(|b| b.command_start.is_some())
                })
    }

    /// The pane's command line belongs to Rift's editor (a prompt, not a
    /// running program).
    pub(super) fn pane_at_prompt(&self, id: PaneId) -> bool {
        self.win
            .tabs
            .iter()
            .find_map(|t| t.pane(id))
            .is_some_and(|p| Self::editor_visible(p) && !p.composer.open)
    }

    /// Type `text` into pane `id` as keys would: at a prompt into its
    /// editor (`\n` submits), else to its program (`\n` is Enter).
    fn type_into_pane(&mut self, id: PaneId, text: &str) {
        if self.pane_at_prompt(id) {
            let active = self.active_pane_id() == Some(id);
            for (i, part) in text.split('\n').enumerate() {
                if i > 0 {
                    self.editor_key_on(id, EditorKey::Submit, active);
                }
                if !part.is_empty() {
                    self.editor_key_on(id, EditorKey::Insert(part.to_string()), active);
                }
            }
        } else {
            let bytes = text.replace('\n', "\r");
            self.write_synced(id, bytes.as_bytes(), false);
        }
    }
}
