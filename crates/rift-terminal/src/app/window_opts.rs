//! Per-window options and window-level input: the launch mode of a new
//! window (`launch_mode`), always on top (`always_on_top`, toggled per
//! window), named windows ("Rename window…": the title, saved with the
//! session), resizes snapped to whole cells (`snap_to_cells`) with the
//! grid's columns × rows shown while they last (`resize_overlay`), opacity
//! steps (`opacity_*` actions) and ⌘ + wheel zoom (`wheel_zoom`).

use super::*;

/// How long the columns × rows overlay stays after the last resize.
pub(super) const RESIZE_OVERLAY_SHOWN: Duration = Duration::from_millis(1000);
/// A resize this recent makes a grid change show the overlay (a font zoom
/// or a split changes the grid too, without one).
const RESIZE_RECENT: Duration = Duration::from_millis(1000);
/// One opacity step (5 %).
const OPACITY_STEP: f32 = 0.05;
/// Trackpad pixels per wheel-zoom step.
const ZOOM_PX: f32 = 40.0;
/// Longest window name.
const MAX_WINDOW_NAME: usize = 100;

/// A typed window name: trimmed, control characters dropped, at most
/// [`MAX_WINDOW_NAME`] characters; `None` when nothing is left.
pub(super) fn clean_window_name(raw: &str) -> Option<String> {
    let name: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_WINDOW_NAME)
        .collect();
    let name = name.trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// `opacity` moved `steps` opacity steps, kept in 0.01–1.0 and rounded to
/// whole percent (repeated steps do not drift).
pub(super) fn step_opacity(opacity: f32, steps: i32) -> f32 {
    let v = (opacity + steps as f32 * OPACITY_STEP).clamp(0.01, 1.0);
    (v * 100.0).round() / 100.0
}

/// Whole wheel-zoom steps in a wheel `delta` (up = positive): a notch per
/// line, a trackpad's pixels gathered in `acc` until they make a step. A
/// sideways delta counts when there is no vertical one (⇧ turns a mouse
/// wheel sideways on macOS).
pub(super) fn zoom_steps(delta: MouseScrollDelta, acc: &mut f32) -> i32 {
    let pick = |x: f64, y: f64| if y != 0.0 { y } else { x };
    match delta {
        MouseScrollDelta::LineDelta(x, y) => {
            let v = pick(f64::from(x), f64::from(y));
            if v == 0.0 {
                0
            } else {
                (v.signum() * v.abs().round().max(1.0)) as i32
            }
        }
        MouseScrollDelta::PixelDelta(p) => {
            *acc += pick(p.x, p.y) as f32;
            let steps = (*acc / ZOOM_PX).trunc();
            *acc -= steps * ZOOM_PX;
            steps as i32
        }
    }
}

impl App {
    /// A new ordinary window's attributes for `launch_mode` (a headless
    /// run records the mode but never maximizes or fills the screen) and
    /// `always_on_top`.
    pub(super) fn window_options(&mut self, mut attrs: WindowAttributes) -> WindowAttributes {
        if self.win.dropdown.is_some() {
            return attrs;
        }
        self.win.launch = self.config.launch_mode;
        if !self.headless {
            match self.config.launch_mode {
                rift_core::LaunchMode::Default => {}
                rift_core::LaunchMode::Maximized => attrs = attrs.with_maximized(true),
                rift_core::LaunchMode::Fullscreen => {
                    attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)))
                }
            }
        }
        let on_top = self.config.always_on_top;
        self.win.on_top = on_top;
        self.win.on_top_applied = Some(on_top);
        if on_top && !self.headless {
            attrs = attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
        }
        attrs
    }

    /// The config's window-level options on the window being served: a
    /// changed `always_on_top` (a window toggled by hand keeps its state
    /// until the setting itself changes), the resize step for
    /// `snap_to_cells`, the title.
    pub(super) fn apply_window_options(&mut self) {
        if self.win.dropdown.is_some() {
            return;
        }
        let on_top = self.config.always_on_top;
        if self.win.on_top_applied != Some(on_top) {
            self.win.on_top_applied = Some(on_top);
            self.set_on_top(on_top);
        }
        let snap = self.config.snap_to_cells;
        let step = LogicalSize::new(self.cell_w, self.line_h);
        if let Some(w) = &self.win.window {
            w.set_resize_increments(snap.then_some(step));
        }
        self.apply_window_title();
    }

    /// ⌘ toggle: the window being served floats above other apps' windows,
    /// or stops.
    pub(super) fn toggle_always_on_top(&mut self) {
        if self.win.dropdown.is_some() {
            self.toast("The drop-down window always stays on top".into());
            return;
        }
        let on = !self.win.on_top;
        self.set_on_top(on);
        self.toast(if on {
            "Window stays on top".into()
        } else {
            "Window no longer stays on top".into()
        });
    }

    fn set_on_top(&mut self, on: bool) {
        self.win.on_top = on;
        if let Some(w) = self.shown_window() {
            w.set_window_level(if on {
                winit::window::WindowLevel::AlwaysOnTop
            } else {
                winit::window::WindowLevel::Normal
            });
        }
    }

    /// The OS title of the window being served: its name (secrets masked
    /// while `mask_secrets` is on: the title shows in the Window menu, the
    /// Dock, Mission Control and screen readers), else "Rift".
    pub(super) fn apply_window_title(&self) {
        if let Some(w) = &self.win.window {
            w.set_title(&self.window_title());
        }
    }

    /// The title [`Self::apply_window_title`] sets.
    pub(super) fn window_title(&self) -> String {
        match self.win.name.as_deref() {
            Some(name) => rift_core::secrets::mask(name).into_owned(),
            None => "Rift".into(),
        }
    }

    /// "Rename window…": the palette takes the name, prefilled with the
    /// current one.
    pub(super) fn start_rename_window(&mut self) {
        if self.win.dropdown.is_some() {
            self.toast("The drop-down window has no title to name".into());
            return;
        }
        let current = self.win.name.clone().unwrap_or_default();
        self.open_palette(crate::views::palette::PaletteMode::RenameWindow);
        // selected: the first key typed replaces it
        self.win.palette.query_selected = !current.is_empty();
        self.win.palette.query = current;
        self.refresh_palette();
    }

    /// The palette's typed name becomes the window's (an empty one clears
    /// it); the title and the session file follow.
    pub(super) fn name_window_from_palette(&mut self) {
        let name = clean_window_name(&self.win.palette.query);
        self.win.name = name;
        self.apply_window_title();
        self.save_session();
    }

    /// Step the window opacity (`window_opacity`, saved) by `steps`; 0
    /// resets it to the default.
    pub(super) fn step_window_opacity(&mut self, steps: i32) {
        self.config.window_opacity = if steps == 0 {
            rift_core::Config::default().window_opacity
        } else {
            step_opacity(self.config.opacity(), steps)
        };
        self.save_config();
        self.apply_config();
    }

    /// `wheel_zoom`: ⌘ + wheel (Ctrl + wheel off macOS) zooms the font, ⇧
    /// added steps the opacity. True when the wheel was taken.
    pub(super) fn wheel_zoom(&mut self, delta: MouseScrollDelta) -> bool {
        let m = self.win.modifiers;
        let primary = if cfg!(target_os = "macos") {
            m.super_key() && !m.control_key()
        } else {
            m.control_key() && !m.super_key()
        };
        if !self.config.wheel_zoom || !primary || m.alt_key() {
            return false;
        }
        let steps = zoom_steps(delta, &mut self.win.wheel_zoom_px);
        if steps == 0 {
            return true;
        }
        if m.shift_key() {
            self.step_window_opacity(steps);
        } else {
            self.config.font_size = (self.config.font_size() + steps as f32).clamp(8.0, 28.0);
            self.save_config();
            self.apply_config();
        }
        true
    }

    /// After a frame: the active pane's grid changed size during a window
    /// resize, so the columns × rows overlay shows (`resize_overlay`).
    pub(super) fn track_resize_overlay(&mut self) {
        let grid = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .and_then(|p| p.base_grid)
            .map(|(c, r, _)| (c, r));
        let was = std::mem::replace(&mut self.win.overlay_grid, grid);
        let resizing = self
            .win
            .resized_at
            .is_some_and(|t| t.elapsed() < RESIZE_RECENT);
        if let (true, true, Some(was), Some(now)) =
            (self.config.resize_overlay, resizing, was, grid)
            && was != now
        {
            self.win.resize_overlay = Some((now, Instant::now()));
        }
    }

    /// The overlay's text while it shows.
    pub(super) fn resize_overlay_text(&self) -> Option<String> {
        let ((cols, rows), at) = self.win.resize_overlay?;
        (at.elapsed() < RESIZE_OVERLAY_SHOWN).then(|| format!("{cols} × {rows}"))
    }

    /// Draw the columns × rows overlay in the window's centre (topmost
    /// layer, a tooltip's plate).
    pub(super) fn draw_resize_overlay(&mut self, w: f32, h: f32) {
        let Some(text) = self.resize_overlay_text() else {
            return;
        };
        let Some(engine) = self.text.as_mut() else {
            return;
        };
        // a tooltip sits 6 px under its control: this point puts the
        // 24 px plate on the centre line
        let tip = crate::views::tooltip::Tip::new([w / 2.0, h / 2.0 - 18.0, 0.0, 0.0], text, None);
        crate::views::tooltip::render(
            self.win.scene.layer(7),
            engine,
            &self.theme,
            &tip,
            (w, h),
            (-1.0, -1.0),
        );
    }

    /// When the overlay goes (a repaint then), or that it went now.
    pub(super) fn resize_overlay_deadline(&mut self, now: Instant) -> (bool, Option<Instant>) {
        let Some((_, at)) = self.win.resize_overlay else {
            return (false, None);
        };
        let until = at + RESIZE_OVERLAY_SHOWN;
        if now >= until {
            self.win.resize_overlay = None;
            (true, None)
        } else {
            (false, Some(until))
        }
    }

    /// The dump's line for these options.
    pub(super) fn window_opts_dump(&self) -> String {
        let increments = self.win.window.as_ref().and_then(|w| {
            w.resize_increments()
                .map(|s| s.to_logical::<f32>(w.scale_factor()))
                .map(|s| format!("{:.2}x{:.2}", s.width, s.height))
        });
        let size = self.win.window.as_ref().map(|w| {
            let s = w.inner_size().to_logical::<f64>(w.scale_factor());
            format!("{}x{}", s.width.round(), s.height.round())
        });
        format!(
            "window_name={:?} on_top={} launch={:?} size={:?} resize_increments={:?} resize_overlay={:?} title={:?} opacity={}",
            self.win.name,
            self.win.on_top,
            self.win.launch,
            size,
            increments,
            self.resize_overlay_text(),
            self.window_title(),
            self.config.opacity()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_names_are_trimmed_and_bounded() {
        assert_eq!(clean_window_name("  logs \n"), Some("logs".into()));
        assert_eq!(clean_window_name("a\u{1b}[31mb"), Some("a[31mb".into()));
        assert_eq!(clean_window_name("   "), None);
        assert_eq!(
            clean_window_name(&"x".repeat(300)).map(|n| n.len()),
            Some(MAX_WINDOW_NAME)
        );
    }

    #[test]
    fn opacity_steps_by_five_percent_within_bounds() {
        assert_eq!(step_opacity(0.5, 1), 0.55);
        assert_eq!(step_opacity(0.5, -2), 0.4);
        assert_eq!(step_opacity(0.98, 1), 1.0);
        assert_eq!(step_opacity(0.03, -1), 0.01);
        // repeated steps land on whole percent
        let mut v = 0.5;
        for _ in 0..7 {
            v = step_opacity(v, -1);
        }
        assert_eq!(v, 0.15);
    }

    #[test]
    fn wheel_notches_and_trackpad_pixels_make_steps() {
        let mut acc = 0.0;
        assert_eq!(
            zoom_steps(MouseScrollDelta::LineDelta(0.0, 1.0), &mut acc),
            1
        );
        assert_eq!(
            zoom_steps(MouseScrollDelta::LineDelta(0.0, -2.0), &mut acc),
            -2
        );
        // a fraction of a notch is still one
        assert_eq!(
            zoom_steps(MouseScrollDelta::LineDelta(0.0, 0.2), &mut acc),
            1
        );
        // ⇧ turned the wheel sideways
        assert_eq!(
            zoom_steps(MouseScrollDelta::LineDelta(-1.0, 0.0), &mut acc),
            -1
        );
        let px = |y| MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, y));
        assert_eq!(zoom_steps(px(30.0), &mut acc), 0);
        assert_eq!(zoom_steps(px(30.0), &mut acc), 1);
        assert_eq!(acc, 20.0);
        assert_eq!(zoom_steps(px(-100.0), &mut acc), -2);
    }
}
