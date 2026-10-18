//! Secret masking in the app (`mask_secrets`; `rift_core::secrets` holds
//! the patterns, `crate::secret_mask` the rows): the masker in force, a
//! click that shows a masked span for a while, and copies, which are masked
//! unless "Copy output unmasked" is asked for.

use super::*;

impl App {
    /// Apply `mask_secrets` and `secret_patterns` (`apply_config`). A
    /// pattern list that does not compile never gets here (config.toml is
    /// refused); the built-in patterns stay in force if it ever does.
    pub(super) fn apply_secret_masking(&mut self) {
        let masker = self.config.mask_secrets.then(|| {
            rift_core::secrets::Masker::new(&self.config.secret_patterns).unwrap_or_else(|e| {
                log::warn!("{e}; masking with the built-in patterns");
                rift_core::secrets::Masker::new(&[]).expect("built-in patterns")
            })
        });
        rift_core::secrets::set_active(masker.map(Arc::new));
    }

    /// A click at (x, y) on a masked span of `pane`: shown for
    /// `secret_mask::REVEAL_FOR`. True when it hit one.
    pub(super) fn reveal_secret_at(&mut self, pane: PaneId, x: f32, y: f32) -> bool {
        let Some(p) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.pane_mut(pane))
        else {
            return false;
        };
        match p.secrets.hit_at(x, y) {
            Some(key) => {
                p.secrets.reveal(key, Instant::now());
                true
            }
            None => false,
        }
    }

    /// Revealed secrets whose time is up: a repaint, and when the next one's
    /// is.
    pub(super) fn secret_deadlines(&mut self, now: Instant) -> (bool, Option<Instant>) {
        let mut repaint = false;
        let mut next: Option<Instant> = None;
        for tab in &mut self.win.tabs {
            for p in &mut tab.panes {
                let (expired, due) = p.secrets.expire(now);
                repaint |= expired;
                if let Some(d) = due {
                    next = Some(next.map_or(d, |n| n.min(d)));
                }
            }
        }
        (repaint, next)
    }
}
