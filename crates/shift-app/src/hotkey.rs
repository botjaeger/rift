//! System-wide show/hide shortcut (`global_hotkey = "alt+space"`, Warp's
//! hotkey window). Pressing it from any app brings Shift forward; pressing
//! it while Shift is frontmost hides it again.

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

/// A registered hotkey (unregistered on drop by the manager).
pub struct GlobalHotkey {
    _manager: GlobalHotKeyManager,
    spec: String,
}

impl GlobalHotkey {
    pub fn spec(&self) -> &str {
        &self.spec
    }
}

/// Parse `spec` in `[keybindings]` chord syntax (`alt+space`, `ctrl+\``).
pub fn parse(spec: &str) -> Result<HotKey, String> {
    let b = shift_core::keybindings::Binding::parse(spec)
        .ok_or_else(|| format!("invalid global_hotkey {spec:?}"))?;
    let mut parts: Vec<&str> = Vec::new();
    if b.ctrl {
        parts.push("ctrl");
    }
    if b.alt {
        parts.push("alt");
    }
    if b.shift {
        parts.push("shift");
    }
    if b.cmd {
        parts.push("super");
    }
    let key = match b.key.as_str() {
        "`" => "Backquote".to_string(),
        "left" => "ArrowLeft".into(),
        "right" => "ArrowRight".into(),
        "up" => "ArrowUp".into(),
        "down" => "ArrowDown".into(),
        k => k.to_string(),
    };
    let s = format!("{}+{key}", parts.join("+"));
    let s = s.trim_start_matches('+');
    s.parse::<HotKey>()
        .map_err(|e| format!("invalid global_hotkey {spec:?}: {e}"))
}

/// Register `spec`; `on_press` runs (on a hotkey thread) for each press.
pub fn register(
    spec: &str,
    on_press: impl Fn() + Send + Sync + 'static,
) -> Result<GlobalHotkey, String> {
    let hotkey = parse(spec)?;
    let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
    manager
        .register(hotkey)
        .map_err(|e| format!("global_hotkey {spec:?}: {e}"))?;
    let id = hotkey.id();
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        if e.id() == id && e.state == HotKeyState::Pressed {
            on_press();
        }
    }));
    Ok(GlobalHotkey {
        _manager: manager,
        spec: spec.to_string(),
    })
}

/// Is Shift the frontmost app (macOS) — decides show vs hide.
#[cfg(target_os = "macos")]
pub fn app_active() -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    MainThreadMarker::new()
        .map(|mtm| NSApplication::sharedApplication(mtm).isActive())
        .unwrap_or(false)
}

/// Hide the whole app (macOS ⌘H semantics) or bring it back to the front.
#[cfg(target_os = "macos")]
pub fn set_app_shown(show: bool) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    if show {
        app.unhide(None);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    } else {
        app.hide(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chord_syntax() {
        assert!(parse("alt+space").is_ok());
        assert!(parse("ctrl+`").is_ok());
        assert!(parse("cmd+shift+t").is_ok());
        assert!(parse("f12").is_ok());
        assert!(parse("alt+").is_err());
        assert!(parse("").is_err());
    }
}
