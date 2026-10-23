//! One process, many windows. `App::win` is the window being served:
//! window events, drawing, keys and commands act on it. Every other window
//! waits in `App::parked`; routing an event swaps its window in
//! (`enter_window`, `enter_pane_window`), so the rest of the app reads one
//! window's state through `self.win` whichever window it is. Pane ids come
//! from one process-wide counter, so a pane id names one pane in one window.

use super::*;

/// Offset of a new window from the one it was opened from (cascade).
const CASCADE: f64 = 24.0;

impl App {
    /// The id of the window being served.
    pub(super) fn win_id(&self) -> Option<WindowId> {
        self.win.window.as_ref().map(|w| w.id())
    }

    /// How many windows are open.
    pub(super) fn window_count(&self) -> usize {
        usize::from(self.win.window.is_some()) + self.parked.len()
    }

    /// How many ordinary windows are open (the drop-down hotkey window
    /// left out: it alone does not keep the app open).
    pub(super) fn ordinary_window_count(&self) -> usize {
        std::iter::once(&*self.win)
            .chain(self.parked.iter().map(|w| &**w))
            .filter(|w| w.window.is_some() && w.dropdown.is_none())
            .count()
    }

    /// Serve window `id`. False when no window has that id.
    pub(super) fn enter_window(&mut self, id: WindowId) -> bool {
        if self.win_id() == Some(id) {
            return true;
        }
        let Some(i) = self
            .parked
            .iter()
            .position(|w| w.window.as_ref().is_some_and(|x| x.id() == id))
        else {
            return false;
        };
        std::mem::swap(&mut self.win, &mut self.parked[i]);
        true
    }

    /// Serve the window that holds `pane`. False when no window holds it.
    pub(super) fn enter_pane_window(&mut self, pane: PaneId) -> bool {
        let holds = |w: &WindowState| w.tabs.iter().any(|t| t.pane(pane).is_some());
        if holds(&self.win) {
            return true;
        }
        let Some(i) = self.parked.iter().position(|w| holds(w)) else {
            return false;
        };
        std::mem::swap(&mut self.win, &mut self.parked[i]);
        true
    }

    /// Serve the window that last had keyboard focus (menu commands, the
    /// control socket, the global hotkey act there).
    pub(super) fn enter_focused_window(&mut self) {
        if let Some(id) = self.focused_window {
            self.enter_window(id);
        }
    }

    /// Run `f` once with each window served, the current one first; the
    /// current window is served again afterwards. `f` must not open or
    /// close windows.
    pub(super) fn each_window(&mut self, mut f: impl FnMut(&mut App)) {
        f(self);
        for i in 0..self.parked.len() {
            std::mem::swap(&mut self.win, &mut self.parked[i]);
            f(self);
            std::mem::swap(&mut self.win, &mut self.parked[i]);
        }
    }

    /// Every window's state, in the order they were opened.
    pub(super) fn windows_in_order(&self) -> Vec<&WindowState> {
        let all: Vec<&WindowState> = std::iter::once(&*self.win)
            .chain(self.parked.iter().map(|w| &**w))
            .filter(|w| w.window.is_some())
            .collect();
        let mut ordered: Vec<&WindowState> = self
            .window_order
            .iter()
            .filter_map(|id| {
                all.iter()
                    .find(|w| w.window.as_ref().is_some_and(|x| x.id() == *id))
                    .copied()
            })
            .collect();
        // a window missing from the order (none should be) goes last
        for w in all {
            if !ordered.iter().any(|o| std::ptr::eq(*o, w)) {
                ordered.push(w);
            }
        }
        ordered
    }

    /// Position of the window being served in [`App::windows_in_order`].
    pub(super) fn window_index(&self) -> usize {
        let id = self.win_id();
        self.window_order
            .iter()
            .position(|w| Some(*w) == id)
            .unwrap_or(0)
    }

    /// Serve the `n`th window in opening order (0 = the first). False when
    /// there is none.
    pub(super) fn enter_window_index(&mut self, n: usize) -> bool {
        match self.window_order.get(n).copied() {
            Some(id) => self.enter_window(id),
            None => false,
        }
    }

    /// Every tab of every window.
    pub(super) fn all_tabs(&self) -> impl Iterator<Item = &TabState> {
        self.win
            .tabs
            .iter()
            .chain(self.parked.iter().flat_map(|w| w.tabs.iter()))
    }

    /// Every pane of every window.
    pub(super) fn all_panes(&self) -> impl Iterator<Item = &PaneState> {
        self.all_tabs().flat_map(|t| t.panes.iter())
    }

    /// Create the window being served (a fresh `WindowState`): its winit
    /// window (hidden in a headless run), accessibility adapter, GPU
    /// surface and pipelines. The first window also opens the GPU device and
    /// loads the fonts; later ones share both.
    pub(super) fn create_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        frame: Option<rift_core::session_state::SavedWindow>,
    ) {
        let mut attrs = WindowAttributes::default()
            .with_title("Rift")
            .with_window_icon(crate::icon::window_icon(self.config.app_icon))
            // always transparent so `window_opacity` can change live
            .with_transparent(true)
            .with_inner_size(match frame {
                Some(f) => LogicalSize::new(f.width, f.height),
                None => {
                    let (w, h) = self.config.window_size();
                    LogicalSize::new(w, h)
                }
            });
        if let Some(f) = frame {
            attrs = attrs
                .with_position(winit::dpi::LogicalPosition::new(f.x, f.y))
                .with_maximized(f.maximized);
            if f.fullscreen {
                attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
            }
        }

        // `launch_mode` and `always_on_top` (ordinary windows)
        attrs = self.window_options(attrs);

        // the drop-down hotkey window: no title bar or window buttons, its
        // height from the config, above other windows
        let dropdown = self.win.dropdown.is_some();
        if dropdown {
            attrs = attrs
                .with_decorations(false)
                .with_resizable(false)
                .with_window_level(winit::window::WindowLevel::AlwaysOnTop);
        }

        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::WindowAttributesExtMacOS;
            if !dropdown {
                attrs = attrs
                    .with_titlebar_transparent(true)
                    .with_fullsize_content_view(true)
                    .with_title_hidden(true);
            }
            if !self.headless {
                crate::icon::set_dock_icon(self.config.app_icon);
            }
        }
        // X11 WM_CLASS / Wayland app_id "rift" matches rift.desktop, so
        // docks, taskbars and switchers show its hicolor icon
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use winit::platform::wayland::WindowAttributesExtWayland;
            use winit::platform::x11::WindowAttributesExtX11;
            attrs = WindowAttributesExtX11::with_name(attrs, "rift", "rift");
            attrs = WindowAttributesExtWayland::with_name(attrs, "rift", "rift");
        }

        // hidden until the accessibility adapter is attached (AccessKit
        // must see the window before it is first shown)
        let window = Arc::new(
            event_loop
                .create_window(attrs.with_visible(false))
                .expect("create window"),
        );
        self.win.a11y = Some(accesskit_winit::Adapter::with_event_loop_proxy(
            event_loop,
            &window,
            self.proxy.clone(),
        ));
        // the drop-down window shows when it slides in
        if !self.headless && !dropdown {
            window.set_visible(true);
        }
        // input methods (CJK composition, dead keys, emoji picker)
        window.set_ime_allowed(true);
        self.os_light = window.theme() == Some(winit::window::Theme::Light);
        let size = window.inner_size();
        // every window draws with the first one's device
        let shared = self.parked.iter().find_map(|w| w.gpu.as_ref());
        let gpu = match shared.map(|g| g.for_window(window.clone(), size.width, size.height)) {
            Some(Ok(gpu)) => gpu,
            Some(Err(e)) => {
                log::warn!("new window: {e}");
                Gpu::new(window.clone(), size.width, size.height)
            }
            None => Gpu::new(window.clone(), size.width, size.height),
        };
        let quads = QuadPipeline::new(&gpu.device, gpu.config.format);
        self.win.bg_image = Some(rift_renderer::ImagePipeline::new(
            &gpu.device,
            gpu.config.format,
        ));
        self.win.sprites = Some(rift_renderer::SpritePipeline::new(
            &gpu.device,
            gpu.config.format,
        ));
        if self.text.is_none() {
            let mut text = TextEngine::new(&gpu.device, &gpu.queue, gpu.config.format);
            self.cell_w = text.mono_cell_width(self.config.font_size());
            self.text = Some(text);
        }
        if self.headless {
            // hidden windows have no real place: each gets its own stretch
            // of a pretend desktop, far from the others (`tab_move`)
            let last = self
                .parked
                .iter()
                .map(|w| w.headless_origin.0)
                .fold(0.0, f64::max);
            self.win.headless_origin = (last + super::tab_move::HEADLESS_SPACING, 0.0);
        }
        self.window_order.push(window.id());
        self.focused_window = Some(window.id());
        self.win.window = Some(window);
        self.win.gpu = Some(gpu);
        self.win.quads = Some(quads);
    }

    /// Open another window and serve it (the current one is parked).
    pub(super) fn open_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        frame: Option<rift_core::session_state::SavedWindow>,
    ) {
        let old = std::mem::replace(&mut self.win, Box::new(WindowState::new(false)));
        if old.window.is_some() {
            self.parked.push(old);
        }
        self.create_window(event_loop, frame);
        self.apply_window_config();
    }

    /// The frame of a window opened from the one being served: offset from
    /// it, the same size (none from the screen-wide drop-down window).
    pub(super) fn cascade_frame(&self) -> Option<rift_core::session_state::SavedWindow> {
        let w = self
            .win
            .window
            .as_ref()
            .filter(|_| self.win.dropdown.is_none())?;
        let scale = w.scale_factor();
        let pos = w.outer_position().ok()?.to_logical::<f64>(scale);
        let size = w.inner_size().to_logical::<f64>(scale);
        Some(rift_core::session_state::SavedWindow {
            x: pos.x + CASCADE,
            y: pos.y + CASCADE,
            width: size.width,
            height: size.height,
            maximized: false,
            fullscreen: false,
        })
    }

    /// ⌘N: a window beside this one, its first tab where a new tab of this
    /// window would start (`working_directory`).
    pub(super) fn open_new_window(&mut self, event_loop: &ActiveEventLoop) {
        self.open_new_window_at(event_loop, None);
    }

    /// A window beside this one, its first tab of the default profile in
    /// `cwd` (a `rift://` link's folder), else the profile's directory, else
    /// where ⌘N starts.
    pub(super) fn open_new_window_at(
        &mut self,
        event_loop: &ActiveEventLoop,
        cwd: Option<std::path::PathBuf>,
    ) {
        let frame = self.cascade_frame();
        let started = Instant::now();
        // where ⌘N starts follows this window's pane, not the new window
        let fallback = self.start_dir();
        self.open_window(event_loop, frame);
        self.spawn_profile_tab(cwd, fallback);
        log::info!(
            "new window in {} ms ({} open)",
            started.elapsed().as_millis(),
            self.window_count()
        );
    }

    /// Close the window being served when another is open (its shells end)
    /// and serve the most recently focused of the rest. False when it is
    /// the last window: closing it quits.
    pub(super) fn close_window(&mut self) -> bool {
        // the hidden drop-down window alone does not keep the app open
        let ordinary = |w: &WindowState| w.dropdown.is_none();
        if self.win.dropdown.is_none() && !self.parked.iter().any(|w| ordinary(w)) {
            return false;
        }
        if self.parked.is_empty() {
            return false;
        }
        // a theme picker open in it: the theme it previewed goes back
        self.revert_theme_preview();
        for p in self.win.tabs.iter().flat_map(|t| &t.panes) {
            p.session.terminate();
        }
        let closing = self.win_id();
        // a window closing in the background (its shell exited) leaves
        // keyboard focus where it is
        let had_focus = self.win.focused;
        // the ordinary window focused before this one, else the newest
        let next = self
            .focus_history
            .iter()
            .rev()
            .filter(|id| Some(**id) != closing)
            .find_map(|id| {
                self.parked
                    .iter()
                    .position(|w| ordinary(w) && w.window.as_ref().is_some_and(|x| x.id() == *id))
            })
            .or_else(|| self.parked.iter().rposition(|w| ordinary(w)))
            .unwrap_or(self.parked.len() - 1);
        let next = self.parked.remove(next);
        let old = std::mem::replace(&mut self.win, next);
        for p in old.tabs.iter().flat_map(|t| &t.panes) {
            self.pending_commands.remove(&p.session.id);
            self.attention_seen.remove(&p.session.id);
            self.transcript_saved.remove(&p.session.id);
        }
        // the surface and pipelines go before the window they draw to
        drop(old);
        self.window_order.retain(|id| Some(*id) != closing);
        self.focus_history.retain(|id| Some(*id) != closing);
        if self.focused_window == closing || self.focused_window.is_none() {
            self.focused_window = self.win_id();
        }
        if let Some(w) = self.win.window.as_ref() {
            if had_focus && !self.headless {
                w.focus_window();
            }
            w.request_redraw();
        }
        self.save_session();
        true
    }

    /// Window `id` took keyboard focus: menu commands and the control
    /// socket act on it from now on.
    pub(super) fn note_window_focus(&mut self, id: WindowId) {
        self.focused_window = Some(id);
        self.focus_history.retain(|w| *w != id);
        self.focus_history.push(id);
    }

    /// Show `pane` (a pane of the window being served): its tab, then the
    /// window, raised and focused (the session switcher, `ctl focus`).
    pub(super) fn focus_pane_here(&mut self, pane: PaneId) {
        let Some(ti) = self.win.tabs.iter().position(|t| t.pane(pane).is_some()) else {
            return;
        };
        // a pane of the hidden drop-down window: the window drops down
        self.reveal_hotkey_window();
        self.win.settings.open = false;
        self.win.active = ti;
        let tab = &mut self.win.tabs[ti];
        tab.clear_activity();
        tab.active_pane = pane;
        if tab.zoomed.is_some() {
            tab.zoomed = Some(pane);
        }
        if let Some(id) = self.win_id() {
            self.note_window_focus(id);
        }
        if let Some(w) = self.win.window.as_ref() {
            if !self.headless {
                w.set_minimized(false);
                w.focus_window();
            }
            w.request_redraw();
        }
    }

    /// Open the windows an update or a crashed one saved (queued by
    /// `confirm_start`, which runs without the event loop).
    pub(super) fn open_queued_windows(&mut self, event_loop: &ActiveEventLoop) {
        if self.queued_windows.is_empty() {
            return;
        }
        let back = self.win_id();
        for saved in std::mem::take(&mut self.queued_windows) {
            self.open_saved_window(event_loop, &saved);
        }
        if let Some(id) = back {
            self.enter_window(id);
        }
        self.save_session();
    }

    /// One saved window: its frame and tabs (an empty one gets a new tab).
    pub(super) fn open_saved_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        saved: &rift_core::session_state::SavedWindowTabs,
    ) {
        let frame = saved
            .window
            .filter(|f| f.width >= 320.0 && f.height >= 200.0);
        self.open_window(event_loop, frame);
        self.win.name = saved
            .name
            .as_deref()
            .and_then(window_opts::clean_window_name);
        self.apply_window_title();
        if saved.tabs.is_empty() {
            self.spawn_tab();
            return;
        }
        // restoring must not rewrite the file tab by tab
        let persist = std::mem::replace(&mut self.persist_session, false);
        for tab in &saved.tabs {
            self.restore_tab(tab);
        }
        self.persist_session = persist;
        self.win.active = saved.active.min(self.win.tabs.len() - 1);
        self.restore_tab_groups(&saved.groups, &saved.tabs);
    }

    /// Serve the window an event from another thread is about: the pane's
    /// window, the window a reply was asked for, else the focused one.
    pub(super) fn route_user_event(&mut self, event: &mut UserEvent) {
        use UserEvent as E;
        let pane = match event {
            E::PtyWakeup(p)
            | E::PtyTitle(p, _)
            | E::PtyExit(p)
            | E::PtyBell(p)
            | E::PtyClipboardLoad(p, _)
            | E::PtyColorRequest(p, ..)
            | E::PtyWindowOp(p, _)
            | E::PtyNotify(p, ..)
            | E::PtyServiceFailed(p, _)
            | E::PtyServiceReady(p)
            | E::GitStatus(p, _)
            | E::PrStatus(p, _)
            | E::DockerStatus(p, _) => Some(*p),
            E::NotificationResponse(r) => Some(r.pane),
            // the asking pane's window; a late shell or remote answer after
            // the request was settled goes to the pane its menu belongs to
            E::Completions(..) | E::CompletionsPartial(..) => self
                .completion_pending
                .as_ref()
                .map(|r| r.pane)
                .or(self.completion_pane),
            E::Access(e) => {
                self.enter_window(e.window_id);
                return;
            }
            // the window that asked (the handler drops a switcher or
            // commit/push answer for a window closed meanwhile; a new
            // worktree still opens its tab in the window served)
            E::WindowsLoaded(asker, ..)
            | E::DetachedLoaded(asker, _)
            | E::ResultsFound(asker, ..)
            | E::WorktreeAdded(asker, _)
            | E::ReviewGit(asker, ..) => {
                if let Some(id) = *asker {
                    self.enter_window(id);
                }
                return;
            }
            E::ReviewLoaded(generation, _) => {
                let generation = *generation;
                self.enter_where(|w| w.review.generation == generation);
                return;
            }
            E::ViewerLoaded(loaded) => {
                self.enter_where(|w| w.viewer.awaits(loaded));
                return;
            }
            E::CompareReady(generation, _) => {
                let generation = *generation;
                self.enter_where(|w| w.compare.generation == generation);
                return;
            }
            E::Control(p) => {
                self.route_control(&mut p.request);
                return;
            }
            _ => None,
        };
        match pane {
            Some(p) if self.enter_pane_window(p) => {}
            _ => self.enter_focused_window(),
        }
    }

    /// Serve the first window `pred` holds for (else stay).
    pub(super) fn enter_where(&mut self, pred: impl Fn(&WindowState) -> bool) {
        if pred(&self.win) {
            return;
        }
        if let Some(i) = self.parked.iter().position(|w| pred(w)) {
            std::mem::swap(&mut self.win, &mut self.parked[i]);
        }
    }

    /// A control request acts in the window of the pane it names; a tab
    /// index counts across windows (`tabs_info`) and becomes the index in
    /// its own window here; anything else acts in the focused window.
    fn route_control(&mut self, req: &mut crate::control::Request) {
        use crate::control::Request as R;
        let pane = match req {
            R::Blocks { pane, .. }
            | R::Screen { pane, .. }
            | R::Output { pane, .. }
            | R::Menu { pane, .. }
            | R::Diff { pane, .. }
            | R::Complete { pane, .. }
            | R::SendText { pane, .. }
            | R::RemoteInput { pane, .. }
            | R::RemoteScroll { pane, .. }
            | R::RemoteUpload { pane, .. }
            | R::RemoteOpen { pane, .. }
            | R::RemoteClose { pane, .. }
            | R::RemoteFit { pane, .. }
            | R::RemoteResetElapsed { pane, .. }
            | R::Focus {
                pane: Some(pane), ..
            } => Some(*pane),
            R::Focus {
                tab: Some(tab),
                pane: None,
            } => {
                let mut n = *tab;
                let found = self
                    .windows_in_order()
                    .iter()
                    .enumerate()
                    .find_map(|(wi, w)| {
                        if n < w.tabs.len() {
                            Some(wi)
                        } else {
                            n -= w.tabs.len();
                            None
                        }
                    });
                match found {
                    Some(wi) => {
                        self.enter_window_index(wi);
                        *tab = n;
                    }
                    // out of range in every window: the focused one says so
                    None => self.enter_focused_window(),
                }
                return;
            }
            _ => None,
        };
        match pane {
            Some(p) if self.enter_pane_window(p) => {}
            _ => self.enter_focused_window(),
        }
    }

    /// The saved form of every window: the first one opened first. The
    /// drop-down hotkey window is not saved (a relaunch does not restore it).
    pub(super) fn saved_windows(&self) -> Vec<rift_core::session_state::SavedWindowTabs> {
        self.windows_in_order()
            .into_iter()
            .filter(|w| w.dropdown.is_none())
            .map(|w| self.saved_window(w))
            .collect()
    }
}
