//! Shell widgets (Warp): ⌃R, ⌃T and ⌥C in the command editor run the
//! shell's own widgets (fzf, atuin) where its config binds them
//! (`rift_core::shell_widget` has the hand-off), with `shell_widgets =
//! "auto"`. The widget's line comes back into the editor, not run.

use super::*;
use rift_core::shell_widget::{self, WidgetKey};

impl App {
    /// Hand the active pane's command line to its shell's widget for the
    /// key `event` is, when the shell binds one: true when handed over (the
    /// key is consumed and keys go raw to the shell until it answers).
    pub(super) fn shell_widget_key(&mut self, event: &input::KeyPress) -> bool {
        // the composer's draft is for the running program, not the shell
        if self.config.shell_widgets != rift_core::ShellWidgets::Auto
            || self.composing_pane().is_some()
        {
            return false;
        }
        let Some(key) = input::widget_key(event, self.win.modifiers, self.option_meta()) else {
            return false;
        };
        self.hand_to_shell_widget(key)
    }

    fn hand_to_shell_widget(&mut self, key: WidgetKey) -> bool {
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        else {
            return false;
        };
        {
            let widgets = pane.session.widgets.lock().unwrap();
            if !widgets.bound(key) || widgets.busy() {
                return false;
            }
        }
        let Some(bytes) = shell_widget::handoff_bytes(key, &pane.editor.buffer, pane.editor.cursor)
        else {
            return false;
        };
        pane.session.widgets.lock().unwrap().begin();
        pane.session.write(&bytes);
        pane.view_offset = 0;
        pane.selected_block = None;
        pane.correction = None;
        // the widget draws below the shell's prompt: show the terminal now
        self.win.editor_active = false;
        self.win.completion.close();
        true
    }

    /// A widget's line came back for pane `id`: it replaces the command
    /// line, cursor where the widget left it (an unchanged line keeps the
    /// editor's own cursor). Nothing runs.
    pub(super) fn take_shell_widget_result(&mut self, id: PaneId) {
        let Some(pane) = self.find_tab_of_pane(id).and_then(|(_, t)| t.pane_mut(id)) else {
            return;
        };
        let Some((before, after)) = pane.session.widgets.lock().unwrap().take_result() else {
            return;
        };
        let ed = &mut pane.editor;
        if ed.buffer.len() == before.len() + after.len()
            && ed.buffer.starts_with(&before)
            && ed.buffer.ends_with(&after)
        {
            return;
        }
        ed.collapse_carets();
        let all = 0..ed.buffer.len();
        ed.replace_range(all, &format!("{before}{after}"));
        ed.cursor = before.len();
        self.blink_epoch = Instant::now();
        if self.active_pane_id() == Some(id) {
            self.update_completions();
        }
    }
}
