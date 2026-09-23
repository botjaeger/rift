//! Desktop notifications and the audible bell, via the platform's own
//! tools (no extra dependencies): `osascript` on macOS, `notify-send` /
//! PulseAudio on Linux. Every call is fire-and-forget.

use std::process::{Command, Stdio};

fn spawn(cmd: &str, args: &[&str]) {
    let r = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(e) = r {
        log::debug!("notify: {cmd}: {e}");
    }
}

/// AppleScript string literal: escape backslashes and double quotes.
fn applescript_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Desktop notification (title + body).
pub fn send(title: &str, body: &str) {
    if std::env::var_os("SHIFT_NO_NOTIFY").is_some() {
        log::info!("notify (suppressed): {title} — {body}");
        return;
    }
    if cfg!(target_os = "macos") {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            applescript_str(body),
            applescript_str(title)
        );
        spawn("osascript", &["-e", &script]);
    } else {
        spawn("notify-send", &["--app-name=Shift", title, body]);
    }
}

/// Audible bell.
pub fn beep() {
    if std::env::var_os("SHIFT_NO_NOTIFY").is_some() {
        return;
    }
    if cfg!(target_os = "macos") {
        spawn("afplay", &["/System/Library/Sounds/Tink.aiff"]);
    } else if std::path::Path::new("/usr/share/sounds/freedesktop/stereo/bell.oga").exists() {
        spawn("paplay", &["/usr/share/sounds/freedesktop/stereo/bell.oga"]);
    } else {
        spawn("canberra-gtk-play", &["-i", "bell"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applescript_escaping() {
        assert_eq!(applescript_str(r#"say "hi" \ bye"#), r#"say \"hi\" \\ bye"#);
    }
}
