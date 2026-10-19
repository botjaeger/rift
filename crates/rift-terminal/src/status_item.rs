//! The menu bar icon (`background_processes`, macOS). The app shows it while it
//! runs; once no window is left, the phone gateway (`remote --managed`, a
//! background helper) shows it for as long as it runs, so Rift can still be
//! opened, or quit entirely, after its window quit. Its menu is a muda menu:
//! the app receives the picks through its menu event handler like the menu
//! bar's (ids [`PREFIX`]…); the gateway reads muda's channel in
//! [`run_helper`].

use std::sync::atomic::Ordering;

use muda::{ContextMenu, MenuItem, PredefinedMenuItem, Submenu};
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSImage, NSMenu, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use objc2_foundation::{NSData, NSSize, NSString};

/// The app icon as a line drawing, black on transparent, 36 px for an
/// 18 pt menu bar slot (`packaging/gen-icon.py`); a template image, so
/// macOS tints it.
const ICON: &[u8] = include_bytes!("../../../assets/icon/rift-menubar.png");

/// Every item's id starts with this; the rest is a menu bar id
/// (`action:new_window`) or one of the items below.
pub const PREFIX: &str = "status:";
/// Quit Rift and stop its background helpers.
pub const QUIT_COMPLETELY: &str = "status:quit_completely";
/// The gateway's item: open Rift.
const OPEN: &str = "status:open";
const QUIT_LABEL: &str = "Quit Rift Completely";

/// An icon in the menu bar; removed when dropped.
pub struct StatusItem {
    item: Retained<NSStatusItem>,
    _menu: Submenu,
}

impl StatusItem {
    pub fn new(mtm: MainThreadMarker, menu: Submenu) -> StatusItem {
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = item.button(mtm) {
            match icon() {
                Some(image) => button.setImage(Some(&image)),
                None => button.setTitle(&NSString::from_str("Rift")),
            }
            button.setToolTip(Some(&NSString::from_str("Rift")));
        }
        // SAFETY: muda's submenu is an NSMenu that lives as long as `menu`,
        // which this item keeps
        let ns = unsafe { &*(menu.ns_menu() as *const NSMenu) };
        item.setMenu(Some(ns));
        StatusItem { item, _menu: menu }
    }
}

impl Drop for StatusItem {
    fn drop(&mut self) {
        NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
    }
}

fn icon() -> Option<Retained<NSImage>> {
    let data = NSData::with_bytes(ICON);
    let image = NSImage::initWithData(NSImage::alloc(), &data)?;
    image.setSize(NSSize::new(18.0, 18.0));
    image.setTemplate(true);
    // VoiceOver names the button by its image
    image.setAccessibilityDescription(Some(&NSString::from_str("Rift")));
    Some(image)
}

fn item(id: &str, label: &str) -> MenuItem {
    MenuItem::with_id(id, label, true, None)
}

/// The app's menu: its windows and settings, and the full quit.
pub fn app_menu() -> Submenu {
    let menu = Submenu::new("Rift", true);
    let _ = menu.append_items(&[
        &item(&format!("{PREFIX}action:new_window"), "New Window"),
        &item(&format!("{PREFIX}action:open_settings"), "Settings…"),
        &PredefinedMenuItem::separator(),
        &item(QUIT_COMPLETELY, QUIT_LABEL),
    ]);
    menu
}

/// The gateway's menu while no window is open.
fn helper_menu() -> Submenu {
    let menu = Submenu::new("Rift", true);
    let _ = menu.append_items(&[
        &item(OPEN, "Open Rift"),
        &PredefinedMenuItem::separator(),
        &item(QUIT_COMPLETELY, QUIT_LABEL),
    ]);
    menu
}

/// Bring Rift's windows to the front (a menu bar icon pick leaves the app
/// that was active in front).
pub fn activate() {
    if let Some(mtm) = MainThreadMarker::new() {
        #[allow(deprecated)]
        objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}

/// The gateway's main thread: an accessory app (no Dock icon, no menu bar)
/// that shows the icon while `shared.want` and answers its menu, until
/// `shared.done`.
pub fn run_helper(shared: &crate::remote::Tray) {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEventMask};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode};

    let Some(mtm) = MainThreadMarker::new() else {
        while !shared.done.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.finishLaunching();
    let events = muda::MenuEvent::receiver();
    let mut shown: Option<StatusItem> = None;
    while !shared.done.load(Ordering::Acquire) {
        let until = NSDate::dateWithTimeIntervalSinceNow(0.25);
        // SAFETY: the main thread's own event queue, default mode
        let event = unsafe {
            app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&until),
                NSDefaultRunLoopMode,
                true,
            )
        };
        if let Some(event) = event {
            app.sendEvent(&event);
        }
        let want = shared.want.load(Ordering::Acquire);
        if want != shown.is_some() {
            shown = want.then(|| StatusItem::new(mtm, helper_menu()));
        }
        while let Ok(e) = events.try_recv() {
            match e.id.0.as_str() {
                OPEN => open_rift(),
                QUIT_COMPLETELY => shared.quit.store(true, Ordering::Release),
                _ => {}
            }
        }
    }
}

/// Open the Rift.app this helper runs from (the binary itself outside a
/// bundle).
fn open_rift() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let bundle = exe
        .ancestors()
        .nth(3)
        .filter(|b| b.extension().is_some_and(|e| e == "app"));
    let spawned = match bundle {
        Some(b) => std::process::Command::new("/usr/bin/open").arg(b).spawn(),
        None => std::process::Command::new(&exe).spawn(),
    };
    match spawned {
        // reaped off this thread: the menu stays responsive
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => log::warn!("menu bar icon: could not open Rift: {e}"),
    }
}
