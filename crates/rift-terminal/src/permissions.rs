//! Settings › Permissions (macOS): what the page shows about the privacy
//! switches, and how it learns it.
//!
//! The switch that matters is Full Disk Access of the app that runs the
//! shells: with `durable_sessions` that is the session service (Rift Helper
//! on an installed bundle), else the window. Each answers for itself, and
//! the service over the IPC (`rift_pty::service::privacy`).

use rift_pty::service::ServicePrivacy;

use crate::notify::Permission;

/// System Settings › Privacy & Security › Full Disk Access.
pub const FULL_DISK_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

/// One app's grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Access {
    /// Not read yet.
    #[default]
    Checking,
    Granted,
    Missing,
    /// The app could not tell.
    Unknown,
    /// The running session service predates the question.
    Restart,
    /// No session service runs.
    NotRunning,
}

impl Access {
    /// The status pill's words.
    pub fn text(self) -> &'static str {
        match self {
            Access::Checking => "Checking…",
            Access::Granted => "Granted",
            Access::Missing => "Not granted",
            Access::Unknown => "Unknown",
            Access::Restart => "Unknown · restart Rift",
            Access::NotRunning => "Not running",
        }
    }

    /// Green for a grant, red for a missing one, neutral otherwise.
    pub fn tone(self) -> Option<bool> {
        match self {
            Access::Granted => Some(true),
            Access::Missing => Some(false),
            _ => None,
        }
    }

    fn of(grant: Option<bool>) -> Access {
        match grant {
            Some(true) => Access::Granted,
            Some(false) => Access::Missing,
            None => Access::Unknown,
        }
    }
}

/// Where new shells run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellHost {
    /// In the window's process (`durable_sessions` off, a scripted or
    /// isolated run).
    #[default]
    Window,
    /// In the session service; `helper`: as Rift Helper.app.
    Service { helper: bool },
}

impl ShellHost {
    pub fn current(durable: bool) -> ShellHost {
        if durable {
            ShellHost::Service {
                helper: rift_pty::launchd::runs_as_helper_app(),
            }
        } else {
            ShellHost::Window
        }
    }
}

/// What Settings › Permissions shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct View {
    pub host: ShellHost,
    /// The window's own Full Disk Access.
    pub window: Access,
    /// The session service's (only asked while `host` is the service).
    pub service: Access,
    /// A scripted run's notification permission (else the real one).
    pub notifications: Option<Permission>,
    /// The page is open (the app reads once as it opens).
    pub shown: bool,
}

/// The helper app's name in System Settings.
const HELPER: &str = "Rift Helper";

/// One app of the Full Disk Access list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRow {
    pub app: &'static str,
    /// What it does for the user, and, when its switch is off, what to do.
    pub role: String,
    pub access: Access,
    /// A Show in Finder button: Rift Helper while its switch is off (it is
    /// in no list until dragged in).
    pub reveal: bool,
}

impl AppRow {
    /// The row as a screen reader says it.
    pub fn name(&self) -> String {
        format!("Full Disk Access, {}: {}", self.app, self.access.text())
    }
}

/// The apps whose switch the user may turn on, shells' host first.
pub fn full_disk_rows(v: &View) -> Vec<AppRow> {
    let row = |app: &'static str, role: &str, access: Access| AppRow {
        reveal: app == HELPER && access == Access::Missing,
        app,
        role: if access == Access::Missing {
            format!("{role} · switch it on, or add it with +")
        } else {
            role.to_string()
        },
        access,
    };
    match v.host {
        ShellHost::Window => vec![row("Rift", "Runs your shells and this window", v.window)],
        ShellHost::Service { helper } => vec![
            row(
                if helper {
                    HELPER
                } else {
                    "Rift session service"
                },
                "Runs your shells",
                v.service,
            ),
            row(
                "Rift",
                "This window: file explorer, editor, drops",
                v.window,
            ),
        ],
    }
}

/// The notification row's status words and tone.
pub fn notification_status(p: Permission) -> (&'static str, Option<bool>) {
    match p {
        Permission::Allowed => ("Granted", Some(true)),
        Permission::Denied => ("Not granted", Some(false)),
        Permission::NotAsked => ("Not asked yet", None),
        Permission::Checking => ("Checking…", None),
        Permission::Script => ("Unknown · not running as Rift.app", None),
        Permission::NotifySend => ("Unknown", None),
    }
}

/// Read the grants (blocks on the session service: off the UI thread).
pub fn probe(host: ShellHost) -> (Access, Access) {
    combine(
        host,
        rift_pty::privacy::full_disk_access(),
        rift_pty::service::privacy,
    )
}

/// The window's probe result and, only while the service hosts the shells,
/// its answer (`ask` is not called otherwise: it would reach a service no
/// shell runs in).
fn combine(
    host: ShellHost,
    window: Option<bool>,
    ask: impl FnOnce() -> ServicePrivacy,
) -> (Access, Access) {
    let service = match host {
        ShellHost::Window => Access::Unknown,
        ShellHost::Service { .. } => from_service(ask()),
    };
    (Access::of(window), service)
}

fn from_service(p: ServicePrivacy) -> Access {
    match p {
        ServicePrivacy::Answer { full_disk } => Access::of(full_disk),
        ServicePrivacy::Outdated => Access::Restart,
        ServicePrivacy::Unreachable => Access::NotRunning,
    }
}

/// A scripted run's fixed answers: `RIFT_SCRIPT_PERMISSIONS`, words of
/// `key=value` (see [`parse_injected`]). Read only by scripted runs.
pub fn injected() -> Option<View> {
    parse_injected(&std::env::var("RIFT_SCRIPT_PERMISSIONS").ok()?)
}

/// `host=helper|service|window`, `window=` / `service=`
/// `granted|missing|unknown|restart|down`,
/// `notifications=allowed|off|unasked`. Unlisted keys keep their default.
pub fn parse_injected(text: &str) -> Option<View> {
    let mut v = View::default();
    for word in text.split([',', ' ']).filter(|w| !w.is_empty()) {
        let (key, value) = word.split_once('=')?;
        let access = || match value {
            "granted" => Some(Access::Granted),
            "missing" => Some(Access::Missing),
            "unknown" => Some(Access::Unknown),
            "restart" => Some(Access::Restart),
            "down" => Some(Access::NotRunning),
            _ => None,
        };
        match key {
            "host" => {
                v.host = match value {
                    "helper" => ShellHost::Service { helper: true },
                    "service" => ShellHost::Service { helper: false },
                    "window" => ShellHost::Window,
                    _ => return None,
                }
            }
            "window" => v.window = access()?,
            "service" => v.service = access()?,
            "notifications" => {
                v.notifications = Some(match value {
                    "allowed" => Permission::Allowed,
                    "off" => Permission::Denied,
                    "unasked" => Permission::NotAsked,
                    _ => return None,
                })
            }
            _ => return None,
        }
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(host: ShellHost, window: Access, service: Access) -> View {
        View {
            host,
            window,
            service,
            ..View::default()
        }
    }

    #[test]
    fn the_shells_host_is_listed_first_and_named() {
        let helper = ShellHost::Service { helper: true };
        let rows = full_disk_rows(&view(helper, Access::Granted, Access::Missing));
        assert_eq!(rows[0].app, "Rift Helper");
        assert_eq!(rows[0].access, Access::Missing);
        assert!(rows[0].role.ends_with("switch it on, or add it with +"));
        assert_eq!(rows[1].app, "Rift");
        assert_eq!(rows[1].access, Access::Granted);
        assert!(!rows[1].role.contains("turn it on"));
        // no helper bundle: the service is not "Rift Helper"
        let plain = full_disk_rows(&view(
            ShellHost::Service { helper: false },
            Access::Granted,
            Access::Granted,
        ));
        assert_eq!(plain[0].app, "Rift session service");
    }

    #[test]
    fn only_a_missing_helper_offers_show_in_finder() {
        let helper = ShellHost::Service { helper: true };
        let off = full_disk_rows(&view(helper, Access::Missing, Access::Missing));
        assert!(off[0].reveal, "the helper, switch off");
        assert!(!off[1].reveal, "Rift itself is in the list already");
        let on = full_disk_rows(&view(helper, Access::Missing, Access::Granted));
        assert!(!on[0].reveal);
        let unsure = full_disk_rows(&view(helper, Access::Missing, Access::NotRunning));
        assert!(!unsure[0].reveal);
        let plain = full_disk_rows(&view(
            ShellHost::Service { helper: false },
            Access::Missing,
            Access::Missing,
        ));
        assert!(!plain[0].reveal, "no helper app to show");
    }

    #[test]
    fn shells_in_the_window_list_only_the_window() {
        // the service's answer is not shown, whatever it is
        let rows = full_disk_rows(&view(ShellHost::Window, Access::Missing, Access::Granted));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].app, "Rift");
        assert_eq!(rows[0].access, Access::Missing);
    }

    #[test]
    fn the_status_words_follow_the_probe() {
        let granted = full_disk_rows(&view(ShellHost::Window, Access::Granted, Access::Checking));
        let missing = full_disk_rows(&view(ShellHost::Window, Access::Missing, Access::Checking));
        assert_eq!(granted[0].name(), "Full Disk Access, Rift: Granted");
        assert_eq!(missing[0].name(), "Full Disk Access, Rift: Not granted");
        assert_ne!(granted[0].role, missing[0].role);
        assert_eq!(Access::Restart.text(), "Unknown · restart Rift");
        assert_eq!(Access::Granted.tone(), Some(true));
        assert_eq!(Access::Missing.tone(), Some(false));
        assert_eq!(Access::Restart.tone(), None);
    }

    #[test]
    fn a_service_answer_maps_to_a_grant() {
        let a = |full_disk| ServicePrivacy::Answer { full_disk };
        assert_eq!(from_service(a(Some(true))), Access::Granted);
        assert_eq!(from_service(a(Some(false))), Access::Missing);
        assert_eq!(from_service(a(None)), Access::Unknown);
        assert_eq!(from_service(ServicePrivacy::Outdated), Access::Restart);
        assert_eq!(
            from_service(ServicePrivacy::Unreachable),
            Access::NotRunning
        );
    }

    #[test]
    fn the_probe_results_reach_the_view() {
        let helper = ShellHost::Service { helper: true };
        let answer = |full_disk| move || ServicePrivacy::Answer { full_disk };
        assert_eq!(
            combine(helper, Some(false), answer(Some(true))),
            (Access::Missing, Access::Granted)
        );
        assert_eq!(
            combine(helper, Some(true), answer(Some(false))),
            (Access::Granted, Access::Missing)
        );
        assert_eq!(
            combine(helper, None, || ServicePrivacy::Outdated),
            (Access::Unknown, Access::Restart)
        );
        // shells in the window: the service is never asked
        let (window, _) = combine(ShellHost::Window, Some(true), || {
            panic!("the service was asked")
        });
        assert_eq!(window, Access::Granted);
    }

    #[test]
    fn notification_words_follow_the_permission() {
        assert_eq!(
            notification_status(Permission::Allowed),
            ("Granted", Some(true))
        );
        assert_eq!(
            notification_status(Permission::Denied),
            ("Not granted", Some(false))
        );
        assert_eq!(notification_status(Permission::NotAsked).1, None);
    }

    #[test]
    fn injected_answers_parse_and_bad_ones_are_refused() {
        let v =
            parse_injected("host=helper window=granted service=missing notifications=off").unwrap();
        assert_eq!(v.host, ShellHost::Service { helper: true });
        assert_eq!(v.window, Access::Granted);
        assert_eq!(v.service, Access::Missing);
        assert_eq!(v.notifications, Some(Permission::Denied));
        assert_eq!(parse_injected("window=bogus"), None);
        assert_eq!(parse_injected("nonsense"), None);
        assert_eq!(parse_injected("").unwrap(), View::default());
    }
}
