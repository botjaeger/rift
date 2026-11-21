//! The command inspector (Warp "Command X-Ray"): resting the pointer on a
//! part of the command line, or ⇧⌘I at the cursor, shows a hover card with
//! what that part is and does (`rift_core::inspect`). A command Rift has no
//! words for is looked up in the system manual (`whatis`, off the UI
//! thread, once per command).

use super::*;
use crate::views::tooltip::Tip;

/// The longest the manual lookup may take.
const MANUAL_TIMEOUT: Duration = Duration::from_secs(10);

impl App {
    /// The inspector's card for the part of the active command line under
    /// the pointer, or (`at_cursor`, ⇧⌘I) at the cursor.
    pub(super) fn inspector_tip(&mut self, at_cursor: bool) -> Option<Tip> {
        if !self.win.editor_active {
            return None;
        }
        let (mx, my) = self.win.mouse;
        let tab = self.win.tabs.get(self.win.active)?;
        let pane_id = tab.active_pane;
        if !at_cursor
            && !self
                .win
                .hits
                .iter()
                .any(|h| h.target == ClickTarget::Editor(pane_id) && h.contains(mx, my))
        {
            return None;
        }
        let (buffer, cursor) = {
            let p = tab.pane(pane_id)?;
            (p.editor.buffer.clone(), p.editor.cursor)
        };
        let offset = if at_cursor {
            cursor
        } else {
            self.editor_offset_at_mouse(pane_id)?
        };
        let found = rift_core::inspect::describe(&buffer, offset)?;
        let rect = self.editor_span_rect(&buffer, found.range.clone())?;
        // the pointer must be on the part itself, not past the line's end
        if !at_cursor
            && !(mx >= rect[0] && mx < rect[0] + rect[2] && my >= rect[1] && my < rect[1] + rect[3])
        {
            return None;
        }
        let text = match found.text {
            Some(t) => t,
            None => self.manual_text(&found.title),
        };
        // the card opens above the whole input zone (the prompt's chips
        // included), never over the chips themselves
        let top = self
            .win
            .hits
            .iter()
            .filter(|h| h.target == ClickTarget::Editor(pane_id))
            .map(|h| h.rect)
            .find(|r| r[0] <= rect[0] && r[1] <= rect[1] && rect[1] < r[1] + r[3])
            .map_or(rect[1], |r| r[1]);
        let anchor = [rect[0], top, rect[2], rect[1] + rect[3] - top];
        Some(Tip::card(
            anchor,
            found.title,
            format!("{} · {text}", found.role.label()),
        ))
    }

    /// Where `range` of the active editor's buffer is drawn (its first
    /// wrapped row).
    fn editor_span_rect(
        &mut self,
        buffer: &str,
        range: std::ops::Range<usize>,
    ) -> Option<[f32; 4]> {
        let (text_x, input_y) = self.win.editor_anchor;
        let rows = editor::wrap_rows(buffer, self.win.editor_cols);
        let row = editor::row_of(&rows, range.start);
        let span = rows.get(row)?.clone();
        let engine = self.text.as_mut()?;
        let mut width = |s: &str| {
            engine.measure(
                s,
                editor::FONT_SIZE,
                rift_renderer::mono_family(),
                rift_renderer::glyphon::Weight::NORMAL,
            )
        };
        let end = range.end.min(span.end).max(range.start);
        let x0 = width(&buffer[span.start..range.start]);
        let x1 = width(&buffer[span.start..end]);
        Some([
            text_x + x0,
            input_y + row as f32 * editor::LINE_H,
            (x1 - x0).max(4.0),
            editor::LINE_H,
        ])
    }

    /// What the manual says about `command`: asked once, off the UI thread;
    /// until it answers the card says so.
    fn manual_text(&mut self, command: &str) -> String {
        match self.manual.get(command) {
            Some(Some(text)) => return text.clone(),
            Some(None) => return "No description in the manual".into(),
            None => {}
        }
        if self.manual_pending.insert(command.to_string()) {
            let proxy = self.proxy.clone();
            let command = command.to_string();
            std::thread::spawn(move || {
                let text = whatis(&command);
                let _ = proxy.send_event(UserEvent::Manual(command, text));
            });
        }
        "Looking it up in the manual…".into()
    }

    /// ⇧⌘I: the card at the cursor, or away again.
    pub(super) fn toggle_inspector(&mut self) {
        self.win.inspect_pinned = !self.win.inspect_pinned && self.win.editor_active;
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }
}

/// `whatis <command>`'s one-line description, `None` when the manual has
/// none (or the lookup fails or takes longer than [`MANUAL_TIMEOUT`]).
fn whatis(command: &str) -> Option<String> {
    // only a plain command name reaches the manual
    if command.is_empty()
        || !command
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
    {
        return None;
    }
    let mut child = std::process::Command::new("whatis")
        .arg(command)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < MANUAL_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    use std::io::Read as _;
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    rift_core::inspect::parse_whatis(command, &out)
}
