//! The sticky command header per pane (Warp: minimize it): with
//! `sticky_command_header` on, `toggle_sticky_header` (or the pane menu)
//! folds the active pane's header into a small tab at the top right of
//! its grid; a click on the tab, or the toggle again, brings it back.

use super::*;

/// Width and height of the minimized header's tab.
const TAB_W: f32 = 28.0;
const TAB_H: f32 = 16.0;

impl App {
    /// Minimize (or restore) pane `id`'s sticky command header.
    pub(super) fn toggle_sticky_header(&mut self, id: PaneId) {
        if !self.config.sticky_command_header {
            self.toast("Sticky command headers are off (Settings › Input)".into());
            return;
        }
        let Some(pane) = self.pane_mut_ref(id) else {
            return;
        };
        pane.sticky_minimized = !pane.sticky_minimized;
        pane.sticky_hit = None;
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }
}

/// Draw the minimized header's tab at the top right of the grid (`right`
/// edge, `top`); its rect for the click that restores the header.
pub(super) fn render_minimized(
    layer: &mut rift_renderer::Layer,
    theme: &Theme,
    right: f32,
    top: f32,
    mouse: (f32, f32),
) -> [f32; 4] {
    let rect = [right - TAB_W - 8.0, top, TAB_W, TAB_H];
    let hover = mouse.0 >= rect[0]
        && mouse.0 < rect[0] + rect[2]
        && mouse.1 >= rect[1]
        && mouse.1 < rect[1] + rect[3];
    layer.bordered(
        [rect[0], rect[1]],
        [rect[2], rect[3]],
        theme.surface,
        if hover { theme.accent } else { theme.border },
        4.0,
        1.0,
    );
    crate::views::icons::draw(
        layer,
        crate::views::icons::Icon::ChevronDown,
        [rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0],
        12.0,
        if hover {
            theme.text_bright
        } else {
            theme.text_dim
        },
    );
    rect
}
