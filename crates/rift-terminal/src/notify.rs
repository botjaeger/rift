//! Desktop notifications and the audible bell. On macOS Rift.app posts its
//! notifications itself (UserNotifications: Rift's icon, and a click or an
//! action comes back to the pane); a build that is not an app bundle (`cargo
//! run`) uses `osascript` instead. Linux: `notify-send` and PulseAudio. Every
//! call is fire-and-forget.

use std::process::{Command, Stdio};
use std::sync::OnceLock;

fn spawn(cmd: &str, args: &[&str]) {
    let r = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match r {
        // reap it off the main thread, or every sound leaves a zombie
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => log::debug!("notify: {cmd}: {e}"),
    }
}

/// AppleScript string literal: escape backslashes and double quotes.
fn applescript_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A desktop notification: what it says, and where a click leads.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Alert {
    /// Who and what: `Claude Code · needs your approval`, `✕ Failed · exit 1 after 8s`.
    pub title: String,
    /// Where: `tab — ~/folder · branch`.
    pub subtitle: String,
    /// The command or the message (a failed command adds its last line).
    pub body: String,
    /// The pane it came from (0: none) and its attention entry (0: none).
    pub pane: u64,
    pub attention: u64,
    /// Offer Rerun beside Show (a failed command).
    pub rerun: bool,
}

/// What a finished or running command did, for its notification.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommandFacts {
    pub command: String,
    /// `None` while it runs (or no exit status).
    pub exit: Option<i32>,
    pub duration: Option<std::time::Duration>,
    /// Its last non-empty output line (a failure's reason, usually).
    pub last_line: Option<String>,
}

/// Everything an alert's wording comes from.
#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    pub kind: crate::views::notifications::Kind,
    pub agent: Option<rift_core::notifications::Agent>,
    /// The entry's title and detail (a command and its status, or a
    /// program's notification).
    pub title: &'a str,
    pub detail: &'a str,
    pub command: Option<&'a CommandFacts>,
    /// Where it happened: the tab's title, its folder, its branch.
    pub tab: &'a str,
    pub cwd: Option<&'a str>,
    pub branch: Option<&'a str>,
}

/// Title (who and what), subtitle (tab — folder · branch) and body (the
/// command or the message) of a desktop notification.
pub fn compose(f: &Facts) -> Alert {
    use crate::views::notifications::Kind;
    let dur = |c: &CommandFacts| c.duration.map(crate::views::terminal::format_duration);
    let agent = f.agent.map(|a| a.name());
    let joined = |a: &str, b: &str| match (a.trim(), b.trim()) {
        ("", b) => b.to_string(),
        (a, "") => a.to_string(),
        (a, b) if a == b => a.to_string(),
        (a, b) => format!("{a} · {b}"),
    };
    // a program's own title is often just the agent's name again
    let message = || match agent {
        Some(name) if f.title.trim() == name => f.detail.trim().to_string(),
        _ => joined(f.title, f.detail),
    };
    let text = format!("{} {}", f.title, f.detail).to_lowercase();
    let mut rerun = false;
    let (title, body) = match (f.kind, f.command) {
        (Kind::Finished, Some(c)) if c.exit.unwrap_or(0) == 0 => {
            let took = dur(c).map(|d| format!(" in {d}")).unwrap_or_default();
            let title = match agent {
                Some(name) => format!("{name} · done{took}"),
                None => format!("✓ Finished{took}"),
            };
            (title, c.command.clone())
        }
        (Kind::Failed, Some(c)) if c.exit.is_some() => {
            let code = c.exit.unwrap_or(1);
            let after = dur(c).map(|d| format!(" after {d}")).unwrap_or_default();
            let title = match agent {
                Some(name) => format!("{name} · exited {code}{after}"),
                None => format!("✕ Failed · exit {code}{after}"),
            };
            rerun = !c.command.trim().is_empty();
            let body = match &c.last_line {
                Some(line) => format!("{}\n{line}", c.command),
                None => c.command.clone(),
            };
            (title, body)
        }
        (Kind::Finished, _) => (
            match agent {
                Some(name) => format!("{name} · task done"),
                None => "Task done".into(),
            },
            message(),
        ),
        (Kind::Failed, _) => (
            match agent {
                Some(name) => format!("{name} · error"),
                None => "Error".into(),
            },
            message(),
        ),
        // a password prompt in a running command
        (Kind::Request, Some(c)) if agent.is_none() => (
            "Password requested".into(),
            if c.command.trim().is_empty() {
                f.title.to_string()
            } else {
                c.command.clone()
            },
        ),
        (Kind::Request, _) => {
            let what = if text.contains("approv") || text.contains("permission") {
                "needs your approval"
            } else {
                "needs your input"
            };
            (
                match agent {
                    Some(name) => format!("{name} · {what}"),
                    None => format!("Rift · {what}"),
                },
                message(),
            )
        }
        (_, _) => match agent {
            Some(name) => (name.to_string(), message()),
            None => (f.title.to_string(), f.detail.to_string()),
        },
    };
    let place = [f.cwd, f.branch]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let tab = f.tab.trim();
    let subtitle = match (tab, place.as_str()) {
        ("", p) => p.to_string(),
        (t, "") => t.to_string(),
        // a tab named after its folder says it once
        (t, p) if p == t || p.starts_with(&format!("{t} ")) => p.to_string(),
        (t, p) => format!("{t} — {p}"),
    };
    Alert {
        title,
        subtitle,
        body,
        rerun,
        ..Alert::default()
    }
}

/// What was done with a notification of this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Response {
    pub pane: u64,
    pub attention: u64,
    /// Rerun was pressed (else: the banner or Show).
    pub rerun: bool,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
static RESPONSE: OnceLock<Box<dyn Fn(Response) + Send + Sync>> = OnceLock::new();

/// Where clicks and actions on this process's notifications go. On macOS it
/// also takes the notification center's delegate, so set it at launch:
/// a click that brings Rift forward is delivered then.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn set_response_handler(f: impl Fn(Response) + Send + Sync + 'static) {
    let _ = RESPONSE.set(Box::new(f));
    #[cfg(target_os = "macos")]
    if !suppressed() && native::available() {
        native::init();
    }
}

/// Request identifier: the process, the pane and the entry. A click on a
/// notification an earlier run of Rift posted only brings Rift forward.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn request_id(a: &Alert) -> String {
    format!("rift.{}.{}.{}", std::process::id(), a.pane, a.attention)
}

/// The response a click on `id` (with `action`) means, if it is this
/// process's.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_response(id: &str, action: &str) -> Option<Response> {
    let mut parts = id.strip_prefix("rift.")?.split('.');
    let pid: u32 = parts.next()?.parse().ok()?;
    let pane: u64 = parts.next()?.parse().ok()?;
    let attention: u64 = parts.next()?.parse().ok()?;
    (pid == std::process::id() && parts.next().is_none()).then_some(Response {
        pane,
        attention,
        rerun: action == "rerun",
    })
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn respond(id: &str, action: &str) {
    match (parse_response(id, action), RESPONSE.get()) {
        (Some(r), Some(f)) => f(r),
        _ => log::info!("notify: response to {id} ({action}) is not this window's"),
    }
}

fn suppressed() -> bool {
    std::env::var_os("RIFT_NO_NOTIFY").is_some()
}

/// Whether Rift's desktop notifications can show (Settings › Notifications).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum Permission {
    /// Not read yet.
    Checking,
    /// macOS has not asked yet: the first notification (or Allow…) asks.
    NotAsked,
    Allowed,
    /// Off in System Settings › Notifications › Rift.
    Denied,
    /// Not an app bundle (`cargo run`): banners come from `osascript`.
    Script,
    /// Linux: `notify-send`; the desktop's own settings decide.
    NotifySend,
}

static PERMISSION: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn store_permission(p: Permission) {
    let v = match p {
        Permission::Checking => 0,
        Permission::NotAsked => 1,
        Permission::Allowed => 2,
        Permission::Denied => 3,
        Permission::Script => 4,
        Permission::NotifySend => 5,
    };
    PERMISSION.store(v, std::sync::atomic::Ordering::Release);
}

/// The last known permission ([`refresh_permission`] reads it again).
pub fn permission() -> Permission {
    if !cfg!(target_os = "macos") {
        return Permission::NotifySend;
    }
    #[cfg(target_os = "macos")]
    if !native::available() {
        return Permission::Script;
    }
    match PERMISSION.load(std::sync::atomic::Ordering::Acquire) {
        1 => Permission::NotAsked,
        2 => Permission::Allowed,
        3 => Permission::Denied,
        _ => Permission::Checking,
    }
}

/// Read the permission again (it changes in System Settings while Rift
/// runs); `changed` runs once it is known.
pub fn refresh_permission(changed: impl Fn() + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    if native::available() && !suppressed() {
        native::refresh(Box::new(changed));
        return;
    }
    let _ = changed;
}

/// Ask macOS for permission now (it asks only while not asked yet).
pub fn request_permission(changed: impl Fn() + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    if native::available() && !suppressed() {
        native::request(Box::new(changed));
        return;
    }
    let _ = changed;
}

/// System Settings › Notifications, on Rift's page.
pub fn open_system_settings() {
    if suppressed() {
        log::info!("notify (suppressed): open System Settings › Notifications");
        return;
    }
    if cfg!(target_os = "macos") {
        let id = bundle_id().unwrap_or_else(|| "dev.rift.terminal".into());
        let url =
            format!("x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={id}");
        spawn("open", &[&url]);
    }
}

fn bundle_id() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        objc2_foundation::NSBundle::mainBundle()
            .bundleIdentifier()
            .map(|s| s.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// Desktop notification with only a title and a body (Rift's own notices).
pub fn send(title: &str, body: &str) {
    send_alert(&Alert {
        title: title.into(),
        body: body.into(),
        ..Alert::default()
    });
}

/// Post `a` as a desktop notification.
pub fn send_alert(a: &Alert) {
    if suppressed() {
        log::info!(
            "notify (suppressed): {} — {} — {}",
            a.title,
            a.subtitle,
            a.body
        );
        return;
    }
    #[cfg(target_os = "macos")]
    if native::available() {
        native::post(a, request_id(a));
        return;
    }
    fallback(a);
}

/// Without the native center: `osascript` (macOS; Script Editor's icon, a
/// click opens Script Editor) or `notify-send`.
fn fallback(a: &Alert) {
    if cfg!(target_os = "macos") {
        let mut script = format!(
            "display notification \"{}\" with title \"{}\"",
            applescript_str(&a.body),
            applescript_str(&a.title)
        );
        if !a.subtitle.is_empty() {
            script.push_str(&format!(" subtitle \"{}\"", applescript_str(&a.subtitle)));
        }
        spawn("osascript", &["-e", &script]);
    } else {
        let body = if a.subtitle.is_empty() {
            a.body.clone()
        } else {
            format!("{}\n{}", a.subtitle, a.body)
        };
        spawn(
            "notify-send",
            &["--app-name=Rift", "--icon=rift", &a.title, &body],
        );
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::Alert;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AllocAnyThread, define_class, msg_send};
    use objc2_foundation::{NSArray, NSBundle, NSError, NSSet, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
        UNNotification, UNNotificationAction, UNNotificationActionOptions, UNNotificationCategory,
        UNNotificationCategoryOptions, UNNotificationPresentationOptions, UNNotificationRequest,
        UNNotificationResponse, UNNotificationSettings, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };
    use std::sync::Once;
    use std::sync::OnceLock;

    /// Category of a failed command's notification (Show, Rerun); every
    /// other one has Show only.
    const FAILED: &str = "rift.failed";
    const ALERT: &str = "rift.alert";

    /// The native center needs an app bundle: outside one (`cargo run`,
    /// tests) `currentNotificationCenter` throws.
    pub fn available() -> bool {
        static OK: OnceLock<bool> = OnceLock::new();
        *OK.get_or_init(|| NSBundle::mainBundle().bundleIdentifier().is_some())
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "RiftNotificationDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            /// Rift in front: banners still show (the app decides when to
            /// post one: only for panes out of view).
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                handler.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
            }

            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                handler: &block2::DynBlock<dyn Fn()>,
            ) {
                let id = response.notification().request().identifier().to_string();
                let action = response.actionIdentifier().to_string();
                super::respond(&id, &action);
                handler.call(());
            }
        }
    );

    fn ns(s: &str) -> Retained<NSString> {
        NSString::from_str(s)
    }

    fn center() -> Retained<UNUserNotificationCenter> {
        UNUserNotificationCenter::currentNotificationCenter()
    }

    /// The delegate (clicks and actions) and the categories (their buttons).
    pub fn init() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let center = center();
            // SAFETY: a plain NSObject subclass with no ivars
            let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(), init] };
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            // the center holds it weakly: it lives as long as the process
            std::mem::forget(delegate);
            let action = |id: &str, title: &str| {
                UNNotificationAction::actionWithIdentifier_title_options(
                    &ns(id),
                    &ns(title),
                    UNNotificationActionOptions::Foreground,
                )
            };
            let category = |id: &str, actions: &[Retained<UNNotificationAction>]| {
                UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
                    &ns(id),
                    &NSArray::from_retained_slice(actions),
                    &NSArray::new(),
                    UNNotificationCategoryOptions::empty(),
                )
            };
            let failed = category(FAILED, &[action("show", "Show"), action("rerun", "Rerun")]);
            let alert = category(ALERT, &[action("show", "Show")]);
            center.setNotificationCategories(&NSSet::from_retained_slice(&[failed, alert]));
        });
    }

    /// Ask once for permission (macOS asks the person the first time).
    fn authorize() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| request(Box::new(|| {})));
    }

    /// Ask for permission; `changed` runs once the answer is stored.
    pub fn request(changed: Box<dyn Fn() + Send + Sync>) {
        init();
        let done = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            if granted.as_bool() {
                log::info!("notify: notifications allowed");
            } else if error.is_null() {
                log::warn!(
                    "notify: notifications are off for Rift (System Settings › Notifications)"
                );
            } else {
                // SAFETY: a non-null NSError from the framework
                let e = unsafe { &*error };
                log::warn!("notify: authorization failed: {}", e.localizedDescription());
            }
            super::store_permission(if granted.as_bool() {
                super::Permission::Allowed
            } else {
                super::Permission::Denied
            });
            changed();
        });
        center().requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &done,
        );
    }

    /// Read the permission; `changed` runs once it is stored.
    pub fn refresh(changed: Box<dyn Fn() + Send + Sync>) {
        let done = RcBlock::new(move |settings: std::ptr::NonNull<UNNotificationSettings>| {
            // SAFETY: the framework passes a valid settings object
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            super::store_permission(match status {
                UNAuthorizationStatus::NotDetermined => super::Permission::NotAsked,
                UNAuthorizationStatus::Denied => super::Permission::Denied,
                _ => super::Permission::Allowed,
            });
            changed();
        });
        center().getNotificationSettingsWithCompletionHandler(&done);
    }

    pub fn post(a: &Alert, id: String) {
        init();
        authorize();
        let content = UNMutableNotificationContent::new();
        content.setTitle(&ns(&a.title));
        content.setSubtitle(&ns(&a.subtitle));
        content.setBody(&ns(&a.body));
        // banners from one pane stack together in Notification Center
        if a.pane != 0 {
            content.setThreadIdentifier(&ns(&format!("rift.pane.{}", a.pane)));
        }
        content.setCategoryIdentifier(&ns(if a.rerun { FAILED } else { ALERT }));
        // no sound here: Rift plays the event's own (Settings › Sounds)
        let request =
            UNNotificationRequest::requestWithIdentifier_content_trigger(&ns(&id), &content, None);
        let fallback = a.clone();
        let done = RcBlock::new(move |error: *mut NSError| {
            if error.is_null() {
                return;
            }
            // SAFETY: a non-null NSError from the framework
            let e = unsafe { &*error };
            log::warn!(
                "notify: not delivered ({}); using osascript",
                e.localizedDescription()
            );
            super::fallback(&fallback);
        });
        log::info!("notify: {} — {} — {}", a.title, a.subtitle, a.body);
        center().addNotificationRequest_withCompletionHandler(&request, Some(&done));
    }
}

/// Play an audio file (notification sounds, Settings preview).
pub fn play(path: &std::path::Path) {
    let p = path.to_string_lossy();
    if suppressed() {
        log::info!("sound (suppressed): {p}");
        return;
    }
    log::info!("sound: {p}");
    if cfg!(target_os = "macos") {
        spawn("afplay", &[&p]);
    } else if which("paplay") {
        spawn("paplay", &[&p]);
    } else {
        spawn("canberra-gtk-play", &["-f", &p]);
    }
}

fn which(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(cmd).is_file()))
}

/// Audible bell.
pub fn beep() {
    if suppressed() {
        // suppressed, but logged like notifications and sounds are
        log::info!("bell (suppressed by RIFT_NO_NOTIFY)");
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

    fn facts<'a>(
        kind: crate::views::notifications::Kind,
        title: &'a str,
        detail: &'a str,
        command: Option<&'a CommandFacts>,
    ) -> Facts<'a> {
        Facts {
            kind,
            agent: None,
            title,
            detail,
            command,
            tab: "rift",
            cwd: Some("~/zzz/rift"),
            branch: Some("main"),
        }
    }

    /// Each kind says who and what, where, and the command or message (the
    /// mockup's wording).
    #[test]
    fn alerts_say_who_what_and_where() {
        use crate::views::notifications::Kind;
        use rift_core::notifications::Agent;
        use std::time::Duration;
        let ok = CommandFacts {
            command: "cargo build --release".into(),
            exit: Some(0),
            duration: Some(Duration::from_secs(72)),
            last_line: None,
        };
        let a = compose(&facts(
            Kind::Finished,
            "cargo build --release",
            "",
            Some(&ok),
        ));
        assert_eq!(
            a.title,
            format!(
                "✓ Finished in {}",
                crate::views::terminal::format_duration(Duration::from_secs(72))
            )
        );
        assert_eq!(a.subtitle, "rift — ~/zzz/rift · main");
        assert_eq!(a.body, "cargo build --release");
        assert!(!a.rerun);

        let failed = CommandFacts {
            command: "cargo test".into(),
            exit: Some(101),
            duration: Some(Duration::from_secs(124)),
            last_line: Some("test session::tests::resize ... FAILED".into()),
        };
        let a = compose(&facts(Kind::Failed, "cargo test", "", Some(&failed)));
        assert_eq!(
            a.title,
            format!(
                "✕ Failed · exit 101 after {}",
                crate::views::terminal::format_duration(Duration::from_secs(124))
            )
        );
        assert_eq!(a.body, "cargo test\ntest session::tests::resize ... FAILED");
        assert!(a.rerun, "a failed command offers Rerun");

        let mut f = facts(
            Kind::Request,
            "Claude Code",
            "Allow Bash: scripts/gates.sh all? (needs your permission)",
            None,
        );
        f.agent = Some(Agent::ClaudeCode);
        let a = compose(&f);
        assert_eq!(a.title, "Claude Code · needs your approval");
        assert_eq!(
            a.body,
            "Allow Bash: scripts/gates.sh all? (needs your permission)"
        );

        let mut f = facts(Kind::Finished, "Codex", "Task completed.", None);
        f.agent = Some(Agent::Codex);
        assert_eq!(compose(&f).title, "Codex · task done");

        let running = CommandFacts {
            command: "sudo systemctl restart nginx".into(),
            ..CommandFacts::default()
        };
        let a = compose(&facts(
            Kind::Request,
            "Password:",
            "Waiting for a password.",
            Some(&running),
        ));
        assert_eq!(a.title, "Password requested");
        assert_eq!(a.body, "sudo systemctl restart nginx");

        let a = compose(&facts(
            Kind::Notify,
            "Deploy finished",
            "staging is live",
            None,
        ));
        assert_eq!(
            (a.title.as_str(), a.body.as_str()),
            ("Deploy finished", "staging is live")
        );

        // a tab named after its folder is said once; no context, no subtitle
        let mut f = facts(Kind::Notify, "t", "b", None);
        f.tab = "~/zzz/rift";
        assert_eq!(compose(&f).subtitle, "~/zzz/rift · main");
        f.tab = "";
        f.cwd = None;
        f.branch = None;
        assert_eq!(compose(&f).subtitle, "");
    }

    #[test]
    fn a_click_comes_back_to_its_pane_only_in_this_process() {
        let a = Alert {
            pane: 7,
            attention: 42,
            ..Alert::default()
        };
        let id = request_id(&a);
        assert_eq!(
            parse_response(&id, "com.apple.UNNotificationDefaultActionIdentifier"),
            Some(Response {
                pane: 7,
                attention: 42,
                rerun: false
            })
        );
        assert!(parse_response(&id, "rerun").unwrap().rerun);
        // an earlier run's notification: nothing to jump to
        let other = format!("rift.{}.7.42", std::process::id() + 1);
        assert_eq!(parse_response(&other, "show"), None);
        assert_eq!(parse_response("rift.x.7.42", "show"), None);
        assert_eq!(parse_response(&format!("{id}.9"), "show"), None);
    }
}
